//! # engine.rs
//!
//! The QuiltEngine — the reactive runtime.
//!
//! ## Role in the system
//!
//! This is the heart of Quilt. The engine holds the cell graph, tracks
//! dependencies, propagates changes, and exposes the universal verbs
//! `get` / `set` / `call` / `push` / `subscribe`.
//!
//! Everything else (CLI, MCP, future TUI/Web) is a view onto this
//! engine. If you understand this file, you understand the system.
//!
//! ## Depends on
//!
//! - `crate::types` — `Cell`, `CellDef`, `CellId`, `CellValue`, etc.
//! - `crate::context` — `extend_context`, `context_key`, `eval_when`.
//! - `crate::error` — `Error`, `Result`.
//! - `crate::cells` — the eight cell evaluators.
//! - `indexmap` — `IndexMap` for deterministic iteration.
//! - `parking_lot` — fast `Mutex` / `RwLock`.
//! - `crossbeam-channel` — bounded MPMC channels for subscriptions.
//!
//! ## Used by
//!
//! - `quilt-mcp` — wraps the engine in an MCP server (async via Tokio).
//! - `quilt-cli` — wraps the engine in a command-line interface.
//! - User code that wants to embed Quilt.
//!
//! ## Key design decisions
//!
//! - The engine is **synchronous** at its core. The public API
//!   (`get`, `set`, `call`, `push`, `subscribe`) is sync. This is
//!   the simplest design and matches the cell evaluators, which are
//!   mostly sync (only `api`, `program`, and `router` are async).
//! - The async boundary lives at the MCP server (Tokio runtime
//!   wraps a sync engine via `spawn_blocking` / `block_on`). This
//!   is the right place for async: the I/O adapters that need to
//!   stream data into the engine push via `push`, which is sync
//!   from the perspective of the consumer.
//! - Cells live behind an `RwLock` so reads (the common case) are
//!   cheap. Writes (set, push) take a write lock briefly to update
//!   the cell, then release before propagating.
//! - The caller context is an owned value. We never share mutable
//!   state inside a context. As the call descends, we build fresh
//!   contexts via `extend_context`.
//! - Per-context memoization is keyed by `context_key(ctx)`.
//! - The graph is an `IndexMap<CellId, Cell>`. Reverse edges
//!   (`dependents`) live alongside forward edges (`dependencies`).
//! - Subscriptions use `crossbeam-channel` for sync MPMC. The
//!   engine pushes `(cellId, newValue, prevValue)` tuples; consumers
//!   iterate. We don't have async backpressure here; if a
//!   consumer falls behind, the channel fills and the engine
//!   blocks briefly on `send`. This is intentional — the engine
//!   never drops events.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use once_cell::sync::OnceCell;

use crossbeam_channel::{unbounded, Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use serde_json::Value;

use crate::cells::{
    evaluate_api, evaluate_formula, evaluate_program, evaluate_router, evaluate_value,
    make_io_value, make_sensor_value, ApiExecutorRef, ProgramRuntime,
};
use crate::context::{context_key, empty_context, extend_context};
use crate::error::{Error, Result};
use crate::types::{
    now_millis, CallerContext, Cell, CellDef, CellId, CellKind, CellStatus, CellValue,
    EvaluationTrace, SheetDef, SubscriptionId,
};

// =============================================================================
// Subscription traits (referenced by types::Subscription)
// =============================================================================

/// A callback invoked when a subscribed cell changes.
pub trait SubscriptionCallback: Send + Sync {
    /// Called with `(cell_id, new_value, prev_value)`.
    fn on_change(
        &self,
        cell_id: &str,
        new_value: &CellValue,
        prev_value: &CellValue,
    );
}

/// A filter applied to subscription events. If the filter returns
/// `false`, the callback is not invoked.
pub trait SubscriptionFilter: Send + Sync {
    /// Return true to allow the event through.
    fn allow(&self, cell_id: &str, new_value: &CellValue, prev_value: &CellValue) -> bool;
}

// =============================================================================
// Engine options
// =============================================================================

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// Whether to record evaluation traces. Off by default (memory cost).
    pub tracing: bool,
    /// Maximum number of recent traces to keep. Default 1000.
    pub trace_capacity: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            tracing: false,
            trace_capacity: 1000,
        }
    }
}

// =============================================================================
// The engine
// =============================================================================

/// The reactive cell runtime. One instance per "session" / "agent" /
/// "deployment". Holds the cell graph and provides the universal API.
///
/// The engine is `Send + Sync` and can be wrapped in `Arc` for sharing
/// across threads.
pub struct QuiltEngine {
    /// Engine id, mostly for logging.
    id: String,
    /// Options.
    options: EngineOptions,
    /// The cell graph.
    cells: RwLock<IndexMap<CellId, Cell>>,
    /// Active subscriptions.
    subscriptions: RwLock<HashMap<SubscriptionId, Sender<SubscriptionEvent>>>,
    /// Recent evaluation traces.
    traces: Mutex<Vec<EvaluationTrace>>,
    /// Counter for unique subscription ids.
    sub_counter: Mutex<u64>,
    /// Self-reference for the runtime. Set via `set_self_arc` after the
    /// engine is wrapped in `Arc<QuiltEngine>`. Used by `EngineRuntime`
    /// (the handle program cells get) to keep a `'static` reference.
    self_ref: OnceCell<Weak<QuiltEngine>>,
}

impl QuiltEngine {
    /// Create a new engine with default options.
    pub fn new(id: impl Into<String>) -> Self {
        Self::with_options(id, EngineOptions::default())
    }

    /// Create a new engine with the given options.
    pub fn with_options(id: impl Into<String>, options: EngineOptions) -> Self {
        Self {
            id: id.into(),
            options,
            cells: RwLock::new(IndexMap::new()),
            subscriptions: RwLock::new(HashMap::new()),
            traces: Mutex::new(Vec::new()),
            sub_counter: Mutex::new(0),
            self_ref: OnceCell::new(),
        }
    }

    /// Wrap the engine in an `Arc<QuiltEngine>` and register the
    /// self-reference so that async tasks (program cells, etc.) can
    /// capture it without lifetime issues.
    ///
    /// This is the **only** supported way to construct an `Arc<QuiltEngine>`.
    /// Use it like:
    /// ```ignore
    /// let engine = QuiltEngine::new("foo").into_arc();
    /// ```
    pub fn into_arc(self) -> Arc<QuiltEngine> {
        let arc = Arc::new(self);
        let weak = Arc::downgrade(&arc);
        // `set` is infallible because the cell is freshly created.
        let engine_ref: &QuiltEngine = &arc;
        engine_ref.self_ref.set(weak).expect("self_ref just created");
        arc
    }

    /// Get an `Arc<QuiltEngine>` if the engine was created via
    /// `into_arc`. Returns `None` otherwise.
    pub fn arc_self(&self) -> Option<Arc<QuiltEngine>> {
        self.self_ref.get().and_then(|w| w.upgrade())
    }
    /// The engine's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    // =========================================================================
    // Sheet lifecycle
    // =========================================================================

    /// Load a sheet definition. Resets all cell state.
    ///
    /// Steps:
    ///   1. Acquire write lock; clear existing cells.
    ///   2. For each `CellDef`, instantiate a `Cell`.
    ///   3. Build dependency edges: declared deps first, then
    ///      auto-detect for formulas (by scanning the expression).
    pub fn load_sheet(&self, sheet: SheetDef) -> Result<()> {
        let mut cells = self.cells.write();

        cells.clear();

        // 1. Instantiate cells.
        for def in sheet.cells {
            let id = def.id.clone();
            let cell = Cell::new(def);
            cells.insert(id, cell);
        }

        // 2. Build dependency edges.
        let ids: Vec<CellId> = cells.keys().cloned().collect();
        for id in &ids {
            let deps = cells[id].def.deps.clone();
            for dep in deps {
                self.add_dep_locked(&mut cells, id.as_str(), dep.as_str());
            }
        }
        // Auto-detect for formulas.
        // Collect (id, expr) first to avoid borrow conflicts.
        let formula_deps: Vec<(String, String)> = ids
            .iter()
            .filter_map(|id| {
                let cell = cells.get(id)?;
                if cell.def.kind == CellKind::Formula {
                    let expr = cell.def.expr.clone()?;
                    Some((id.clone(), expr))
                } else {
                    None
                }
            })
            .collect();
        for (id, expr) in formula_deps {
            for known_id in &ids {
                if known_id == &id {
                    continue;
                }
                if expr_contains_token(&expr, known_id) {
                    self.add_dep_locked(&mut cells, &id, known_id);
                }
            }
        }

        Ok(())
    }

    /// Register a new cell at runtime. Used for dynamic registration
    /// (e.g. by an agent). Returns the new cell.
    pub fn register(&self, def: CellDef) -> Result<Arc<Cell>> {
        let id = def.id.clone();
        let mut cells = self.cells.write();
        if cells.contains_key(&id) {
            return Err(Error::CellAlreadyDefined(id));
        }
        let cell = Cell::new(def);
        cells.insert(id.clone(), cell);

        let deps = cells[&id].def.deps.clone();
        for dep in deps {
            self.add_dep_locked(&mut cells, id.as_str(), dep.as_str());
        }

        Ok(Arc::new(cells[&id].clone()))
    }

    fn add_dep_locked(
        &self,
        cells: &mut IndexMap<CellId, Cell>,
        from: &str,
        to: &str,
    ) {
        if !cells.contains_key(from) || !cells.contains_key(to) {
            return;
        }
        if let Some(from_cell) = cells.get_mut(from) {
            from_cell.dependencies.insert(to.to_string());
        }
        if let Some(to_cell) = cells.get_mut(to) {
            to_cell.dependents.insert(from.to_string());
        }
    }

    // =========================================================================
    // The universal API: get, set, call, push
    // =========================================================================

    /// Get a cell's value. Evaluates if needed.
    pub fn get(&self, id: &str, ctx: CallerContext) -> Result<CellValue> {
        let id_norm = self.normalize_id(id)?;
        let kind = {
            let cells = self.cells.read();
            match cells.get(&id_norm) {
                Some(c) => c.def.kind,
                None => return Err(Error::CellNotFound(id.to_string())),
            }
        };

        let full_ctx = extend_context(&ctx, &id_norm, None);

        match kind {
            CellKind::Value => {
                let cells = self.cells.read();
                let cell = cells.get(&id_norm).expect("checked above");
                Ok(evaluate_value(cell, &full_ctx))
            }
            CellKind::Formula => {
                // Build a snapshot of dep values.
                let snapshot = self.build_formula_snapshot(&id_norm, &full_ctx);
                let cells = self.cells.read();
                let cell = cells.get(&id_norm).expect("checked above");
                let result = evaluate_formula(cell, &snapshot, &full_ctx);
                // Cache and store.
                drop(cells);
                self.cache_result(&id_norm, &full_ctx, &result);
                Ok(result)
            }
            CellKind::Api | CellKind::Program | CellKind::Router => {
                self.evaluate_effectful(&id_norm, &full_ctx, None)
            }
            CellKind::Sensor | CellKind::Io | CellKind::Listener => {
                let cells = self.cells.read();
                let cell = cells.get(&id_norm).expect("checked above");
                Ok(cell.value.clone())
            }
        }
    }

    /// Set a cell's value. Triggers downstream recomputation.
    pub fn set(&self, id: &str, value: Value, ctx: CallerContext) -> Result<()> {
        let id_norm = self.normalize_id(id)?;
        let full_ctx = extend_context(&ctx, &id_norm, None);

        // 1. Update the cell and invalidate its cache.
        {
            let mut cells = self.cells.write();
            let cell = cells
                .get_mut(&id_norm)
                .ok_or_else(|| Error::CellNotFound(id.to_string()))?;
            cell.value = CellValue {
                data: value,
                status: CellStatus::Ready,
                computed_at: Some(now_millis()),
                error: None,
                effects: Vec::new(),
            };
            cell.context_cache.clear();
        }

        // 2. Notify subscribers.
        self.notify_change(&id_norm);

        // 3. Propagate.
        self.propagate(&id_norm, &full_ctx);

        Ok(())
    }

    /// Call a cell as a capability. For pure cells, same as `get`
    /// (input is ignored). For effectful cells, input is passed.
    pub fn call(
        &self,
        id: &str,
        input: Option<Value>,
        ctx: CallerContext,
    ) -> Result<CellValue> {
        let id_norm = self.normalize_id(id)?;
        let kind = {
            let cells = self.cells.read();
            match cells.get(&id_norm) {
                Some(c) => c.def.kind,
                None => return Err(Error::CellNotFound(id.to_string())),
            }
        };
        let full_ctx = extend_context(&ctx, &id_norm, None);

        // Caller-aware cache.
        let key = context_key(&full_ctx);
        {
            let cells = self.cells.read();
            if let Some(cell) = cells.get(&id_norm) {
                if let Some(cached) = cell.context_cache.get(&key) {
                    if cached.status == CellStatus::Ready && cached.error.is_none() {
                        return Ok(cached.clone());
                    }
                }
            }
        }

        let result = match kind {
            CellKind::Value | CellKind::Formula => Ok(self.get(id, ctx.clone())?),
            CellKind::Api => {
                let cell = {
                    let cells = self.cells.read();
                    cells.get(&id_norm).cloned().expect("checked above")
                };
                Ok(drive_async(evaluate_api(cell, full_ctx.clone(), input, None)))
            }
            CellKind::Program => {
                let arc_engine = self.arc_self().expect("engine must be created via into_arc");
                let runtime = Arc::new(EngineRuntime { engine: arc_engine });
                let cell = {
                    let cells = self.cells.read();
                    cells.get(&id_norm).cloned().expect("checked above")
                };
                Ok(drive_async(evaluate_program(cell, full_ctx.clone(), input, runtime)))
            }
            CellKind::Router => {
                let arc_engine = self.arc_self().expect("engine must be created via into_arc");
                let runtime = Arc::new(EngineRuntime { engine: arc_engine });
                let cell = {
                    let cells = self.cells.read();
                    cells.get(&id_norm).cloned().expect("checked above")
                };
                drive_async(evaluate_router(cell, full_ctx.clone(), input, runtime))
            }
            CellKind::Sensor | CellKind::Io | CellKind::Listener => {
                let cells = self.cells.read();
                let cell = cells.get(&id_norm).expect("checked above");
                Ok(cell.value.clone())
            }
        }?;

        self.cache_result(&id_norm, &full_ctx, &result);
        Ok(result)
    }

    /// Push a value into a sensor or IO cell. Triggers downstream.
    pub fn push(&self, id: &str, data: Value) -> Result<()> {
        let id_norm = self.normalize_id(id)?;
        let ctx = extend_context(&empty_context(), &id_norm, None);

        let kind = {
            let cells = self.cells.read();
            match cells.get(&id_norm) {
                Some(c) => c.def.kind,
                None => return Err(Error::CellNotFound(id.to_string())),
            }
        };
        if kind != CellKind::Sensor && kind != CellKind::Io {
            return Err(Error::InvalidCellDef {
                id: id.to_string(),
                message: format!("cannot push to {} cell (only sensor/io)", kind.as_str()),
            });
        }

        {
            let mut cells = self.cells.write();
            let cell = cells.get_mut(&id_norm).expect("checked above");
            cell.value = if kind == CellKind::Sensor {
                make_sensor_value(data)
            } else {
                make_io_value(data)
            };
        }

        self.notify_change(&id_norm);
        self.propagate(&id_norm, &ctx);

        Ok(())
    }

    /// Subscribe to a single cell's changes. Returns a receiver that
    /// yields `SubscriptionEvent` values.
    pub fn subscribe(&self, cell_id: &str) -> Result<SubscriptionHandle> {
        {
            let cells = self.cells.read();
            if !cells.contains_key(cell_id) {
                return Err(Error::CellNotFound(cell_id.to_string()));
            }
        }

        let sub_id = {
            let mut counter = self.sub_counter.lock();
            *counter += 1;
            format!("sub-{}", *counter)
        };

        let (tx, rx) = unbounded();
        self.subscriptions.write().insert(sub_id.clone(), tx);

        Ok(SubscriptionHandle { id: sub_id, rx })
    }

    /// Subscribe to all cells. Returns a receiver that yields every
    /// change.
    pub fn subscribe_all(&self) -> SubscriptionHandle {
        let sub_id = {
            let mut counter = self.sub_counter.lock();
            *counter += 1;
            format!("sub-all-{}", *counter)
        };

        let (tx, rx) = unbounded();
        // Add a wildcard entry.
        self.subscriptions.write().insert(sub_id.clone(), tx);

        SubscriptionHandle { id: sub_id, rx }
    }

    /// Cancel a subscription.
    pub fn unsubscribe(&self, sub_id: &str) {
        self.subscriptions.write().remove(sub_id);
    }

    // =========================================================================
    // Introspection
    // =========================================================================

    /// Get a cell by id. Returns None if no such cell.
    pub fn get_cell(&self, id: &str) -> Option<Arc<Cell>> {
        let cells = self.cells.read();
        cells.get(id).map(|c| Arc::new(c.clone()))
    }

    /// List all cells.
    pub fn list_cells(&self) -> Vec<Arc<Cell>> {
        let cells = self.cells.read();
        cells.values().map(|c| Arc::new(c.clone())).collect()
    }

    /// Get recent evaluation traces. Most recent first.
    pub fn traces(&self) -> Vec<EvaluationTrace> {
        let traces = self.traces.lock();
        traces.iter().rev().cloned().collect()
    }

    /// Record a trace entry. Used by cell evaluators.
    pub(crate) fn record_trace(&self, trace: EvaluationTrace) {
        if !self.options.tracing {
            return;
        }
        let mut traces = self.traces.lock();
        traces.push(trace);
        if traces.len() > self.options.trace_capacity {
            let drop = traces.len() - self.options.trace_capacity;
            traces.drain(0..drop);
        }
    }

    // =========================================================================
    // Internal
    // =========================================================================

    /// Build a snapshot of dependency values for a formula.
    fn build_formula_snapshot(
        &self,
        formula_id: &str,
        caller_ctx: &CallerContext,
    ) -> HashMap<CellId, Value> {
        // Collect the dependency list and kinds under a single
        // read lock, then drop the lock before recursive calls.
        let (deps, dep_kinds): (Vec<CellId>, Vec<(CellId, crate::types::CellKind)>) = {
            let cells = self.cells.read();
            let cell = match cells.get(formula_id) {
                Some(c) => c,
                None => return HashMap::new(),
            };
            let dep_kinds: Vec<(CellId, crate::types::CellKind)> = cell
                .dependencies
                .iter()
                .filter_map(|d| cells.get(d).map(|c| (d.clone(), c.def.kind)))
                .collect();
            (cell.dependencies.iter().cloned().collect(), dep_kinds)
        };

        // Pre-evaluate formula dependencies. For each dep, we
        // extend the caller's context with the dep's id (which
        // is what `get` would do internally) so the result lands
        // in the right cache slot for the snapshot lookup.
        for (dep_id, kind) in &dep_kinds {
            if *kind == crate::types::CellKind::Formula {
                // The dep's evaluation will extend `caller_ctx`
                // with `dep_id` as the caller. We pass the
                // original context (not yet extended) and let
                // the engine do the extension.
                let _ = self.get(dep_id, caller_ctx.clone());
            }
        }

        // Build the snapshot. For formula deps, look up the
        // value in `context_cache` for the DEP's own extended
        // context (i.e., the parent's context with `caller =
        // dep_id`). For other deps, use the cell's most recent
        // value (non-formula cells don't have per-context
        // caches in the same way).
        let cells = self.cells.read();
        deps.iter()
            .filter_map(|dep_id| {
                let dep = cells.get(dep_id)?;
                let value = if dep.def.kind == crate::types::CellKind::Formula {
                    // The dep was evaluated with the extended
                    // context where caller = dep_id. Look it up
                    // under that key.
                    let dep_ctx = extend_context(caller_ctx, dep_id.clone(), None);
                    let dep_key = context_key(&dep_ctx);
                    dep.context_cache
                        .get(&dep_key)
                        .map(|v| v.data.clone())
                        .unwrap_or(Value::Null)
                } else {
                    dep.value.data.clone()
                };
                Some((dep_id.clone(), value))
            })
            .collect()
    }

    /// Cache a result by context key.
    fn cache_result(&self, id: &str, ctx: &CallerContext, value: &CellValue) {
        let key = context_key(ctx);
        let mut cells = self.cells.write();
        if let Some(cell) = cells.get_mut(id) {
            cell.context_cache.insert(key, value.clone());
        }
    }

    /// Evaluate an effectful cell.
    fn evaluate_effectful(
        &self,
        id: &str,
        full_ctx: &CallerContext,
        input: Option<Value>,
    ) -> Result<CellValue> {
        // Clone the cell out of the lock and drop the lock BEFORE
        // creating the async future. This is critical: the future
        // holds owned data (Cell, CallerContext, Value) and is
        // therefore `Send`, so `drive_async` can move it across
        // thread boundaries even when we're inside a tokio runtime.
        let cell = {
            let cells = self.cells.read();
            match cells.get(id) {
                Some(c) => c.clone(),
                None => {
                    return Ok(CellValue {
                        data: Value::Null,
                        status: CellStatus::Error,
                        computed_at: Some(now_millis()),
                        error: Some(crate::types::CellError {
                            message: format!("no such cell: {}", id),
                            stack: None,
                        }),
                        effects: Vec::new(),
                    });
                }
            }
        };
        let ctx = full_ctx.clone();
        let kind = cell.def.kind;
        match kind {
            CellKind::Api => Ok(drive_async(evaluate_api(cell, ctx, input, None))),
            CellKind::Program => {
                let arc_engine = self
                    .arc_self()
                    .expect("engine must be created via into_arc");
                let runtime = Arc::new(EngineRuntime { engine: arc_engine });
                Ok(drive_async(evaluate_program(cell, ctx, input, runtime)))
            }
            CellKind::Router => {
                let arc_engine = self
                    .arc_self()
                    .expect("engine must be created via into_arc");
                let runtime = Arc::new(EngineRuntime { engine: arc_engine });
                drive_async(evaluate_router(cell, ctx, input, runtime))
            }
            _ => unreachable!("not effectful"),
        }
    }

    /// Propagate a change to all dependents.
    fn propagate(&self, changed_id: &str, ctx: &CallerContext) {
        // Collect dependents under a read lock.
        let dependents: Vec<CellId> = {
            let cells = self.cells.read();
            cells
                .get(changed_id)
                .map(|c| c.dependents.iter().cloned().collect())
                .unwrap_or_default()
        };

        // Mark formula/value dependents as stale.
        for dep_id in &dependents {
            let mut cells = self.cells.write();
            if let Some(dep) = cells.get_mut(dep_id) {
                if dep.def.kind == CellKind::Formula || dep.def.kind == CellKind::Value {
                    let mut stale = dep.value.clone();
                    stale.status = CellStatus::Stale;
                    dep.value = stale;
                    dep.context_cache.clear();
                }
            }
        }

        // Fire listeners.
        for dep_id in &dependents {
            let listener_data = {
                let cells = self.cells.read();
                cells.get(dep_id).map(|c| {
                    (
                        c.def.kind == CellKind::Listener,
                        c.def.watch.clone(),
                        c.value.clone(),
                        c.def.action.clone(),
                    )
                })
            };
            if let Some((true, watch, _current, _action)) = listener_data {
                if watch.iter().any(|w| w == changed_id) {
                    // The actual action firing happens via the
                    // listener cell evaluator in a future iteration.
                    // For MVP we just record the trace.
                    if self.options.tracing {
                        let mut trace_ctx = ctx.clone();
                        trace_ctx.caller = Some(dep_id.clone());
                        self.record_trace(EvaluationTrace {
                            cell_id: dep_id.clone(),
                            started_at: now_millis(),
                            completed_at: Some(now_millis()),
                            duration_ms: Some(0),
                            context: trace_ctx,
                            effects: vec![],
                            error: None,
                        });
                    }
                }
            }
        }

        // Recurse.
        for dep_id in &dependents {
            self.propagate(dep_id, ctx);
        }
    }

    /// Notify all subscribers of a cell change.
    fn notify_change(&self, cell_id: &str) {
        // Build the event outside the lock.
        let event = {
            let cells = self.cells.read();
            let cell = match cells.get(cell_id) {
                Some(c) => c,
                None => return,
            };
            SubscriptionEvent {
                cell_id: cell_id.to_string(),
                new_value: cell.value.clone(),
                prev_value: cell.value.clone(), // MVP: we don't track prev in notify
            }
        };

        // Send to all matching subscribers.
        let subscriptions = self.subscriptions.read();
        for (sub_id, tx) in subscriptions.iter() {
            if sub_id.starts_with("sub-all-") || sub_id.contains(cell_id) {
                let _ = tx.send(event.clone());
            }
        }
    }

    /// Normalize a cell id.
    fn normalize_id(&self, id: &str) -> Result<CellId> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidCellDef {
                id: id.to_string(),
                message: "cell id cannot be empty".to_string(),
            });
        }
        Ok(trimmed.to_string())
    }
}

// =============================================================================
// Subscription API
// =============================================================================

/// A subscription event.
#[derive(Debug, Clone)]
pub struct SubscriptionEvent {
    /// The cell that changed.
    pub cell_id: CellId,
    /// The new value.
    pub new_value: CellValue,
    /// The previous value (best-effort; the MVP doesn't track this precisely).
    pub prev_value: CellValue,
}

/// A handle to a subscription.
pub struct SubscriptionHandle {
    /// The subscription id, for unsubscribing.
    pub id: SubscriptionId,
    /// The channel receiver.
    pub rx: Receiver<SubscriptionEvent>,
}

// =============================================================================
// EngineRuntime — what program cells get when they call runtime.get/set/call
// =============================================================================

/// The runtime handle exposed to `program` and `router` cells.
///
/// Holds an `Arc<QuiltEngine>` (not a borrow) so it's `'static` —
/// it can be captured by async tasks without lifetime issues.
struct EngineRuntime {
    engine: Arc<QuiltEngine>,
}

impl ProgramRuntime for EngineRuntime {
    fn get(&self, id: &str, ctx: &CallerContext) -> Result<CellValue> {
        self.engine.get(id, ctx.clone())
    }

    fn set(&self, id: &str, value: Value, ctx: &CallerContext) -> Result<()> {
        self.engine.set(id, value, ctx.clone())
    }

    fn call(&self, id: &str, input: Option<Value>, ctx: &CallerContext) -> Result<CellValue> {
        self.engine.call(id, input, ctx.clone())
    }

    fn list(&self) -> Vec<String> {
        self.engine.list_cells().into_iter().map(|c| c.def.id.clone()).collect()
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Naive token-containment check. The TypeScript version uses
/// `RegExp` with word boundaries; we approximate with character
/// classification here. Sufficient for the MVP.
fn expr_contains_token(expr: &str, id: &str) -> bool {
    let body = expr.strip_prefix('=').unwrap_or(expr);
    for (i, _) in body.match_indices(id) {
        let before = body[..i].chars().last();
        let after = body[i + id.len()..].chars().next();
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
        if before.map(is_word_char).unwrap_or(false) {
            continue;
        }
        if after.map(is_word_char).unwrap_or(false) {
            continue;
        }
        return true;
    }
    false
}

/// Drive an async future to completion on the current Tokio runtime
/// if one is active. Otherwise, build a new multi-threaded runtime.
///
/// This is a temporary bridge: the engine is sync but the cell
/// evaluators for `api`, `program`, and `router` are async. In a
/// real deployment the engine should be async too, and this
/// function would go away. For the MVP it lets us ship a sync
/// engine with async cell evaluators.
///
/// The future does NOT need to be `'static` — we use
/// `Handle::block_on` (or fall back to a new runtime). This works
/// because we're either in an active runtime (and `block_on`
/// accepts non-static futures) or we own the runtime entirely.
fn drive_async<F>(future: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    // Box the future so its type is `Pin<Box<dyn Future<Output = T> + Send>>`.
    // This erases the future's own type, so the Output's lifetime
    // is no longer tied to the future's local variables. The
    // 'static bound on F (the future itself) ensures the
    // captured data is 'static.
    let boxed: std::pin::Pin<Box<dyn std::future::Future<Output = F::Output> + Send>> =
        Box::pin(future);
    drive_async_boxed(boxed)
}

fn drive_async_boxed<T: Send + 'static>(
    future: std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>,
) -> T {
    // The future is `Send + 'static` because cell evaluators take
    // owned data (Cell, CallerContext, Value, Arc<dyn ...>).
    // This means we can always spawn the future on a dedicated
    // thread with its own runtime — even when called from inside
    // an existing tokio runtime (where `Handle::block_on` would
    // panic). The dedicated thread is cheap (one per cell
    // evaluation) and avoids any cross-runtime issues.
    //
    // We use a Mutex<Option<...>> to pass the result back across
    // the thread boundary. The 'static bound on F::Output ensures
    // the value can be moved freely.
    use std::sync::{Arc, Mutex};
    let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let slot_for_thread = Arc::clone(&slot);
    let join = std::thread::Builder::new()
        .name("quilt-drive-async".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let result = rt.block_on(future);
            *slot_for_thread
                .lock()
                .expect("drive_async mutex poisoned") = Some(result);
        })
        .expect("failed to spawn thread");
    let _ = join.join().expect("drive_async thread panicked");
    // Take the result out. We need to keep the Mutex alive while
    // we hold the lock guard, otherwise the guard would dangle.
    let result = {
        let mut guard = slot.lock().expect("drive_async mutex poisoned");
        guard.take().expect("drive_async thread did not set result")
    };
    result
}

// Suppress unused-import warnings for items only used in async paths.
#[allow(dead_code)]
fn _unused_api_executor_ref(_: ApiExecutorRef) {}
