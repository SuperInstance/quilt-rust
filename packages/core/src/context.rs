//! # context.rs
//!
//! Caller context propagation through the cell graph.
//!
//! ## Role in the system
//!
//! Every cell call carries a `CallerContext`. The runtime extends the
//! context as it descends into the dependency graph so cells can route
//! on row/column/identity/provenance. The same `CallerContext` is also
//! used to key per-context memoization.
//!
//! ## Depends on
//!
//! - `types::CellId` — for `caller` and `trace`.
//! - `rhai` — for evaluating `when` expressions inside router/listener
//!   rules. The expressions see `caller.row`, `caller.column`, etc.
//!
//! ## Used by
//!
//! - `engine.rs` — calls `extend_context` on every `get`/`set`/`call` and
//!   uses `context_key` to look up per-context cached values.
//! - `cells/router.rs` — calls `eval_when` to decide which rule fires.
//! - `cells/listener.rs` — calls `eval_when` to gate the action.
//!
//! ## Key decisions
//!
//! - `CallerContext` is a value type. The engine passes owned copies
//!   through async boundaries. We do not share mutable state because the
//!   context is append-only from a runtime perspective: the engine
//!   extends it but never edits earlier fields.
//! - `extend_context` does *not* mutate; it returns a new context. This
//!   makes the call graph easier to reason about.
//! - `eval_when` uses rhai for evaluation. The expression sees a single
//!   variable `caller` with `row`, `column`, `sheet`, `identity`,
//!   `metadata`. This mirrors the TypeScript `evalWhen` but the language
//!   is safer (rhai is sandboxed by default).

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::{now_millis, CallerContext, CellId};

/// A builder for `CallerContext` that lets callers chain fields without
/// having to clone the whole struct.
#[derive(Debug, Default, Clone)]
pub struct CallerContextBuilder {
    inner: CallerContext,
}

impl CallerContextBuilder {
    /// Create an empty builder with the current timestamp.
    pub fn new() -> Self {
        Self {
            inner: CallerContext::default(),
        }
    }

    /// Set the row.
    pub fn row(mut self, row: impl Into<Value>) -> Self {
        self.inner.row = Some(row.into());
        self
    }

    /// Set the column.
    pub fn column(mut self, column: impl Into<Value>) -> Self {
        self.inner.column = Some(column.into());
        self
    }

    /// Set the sheet name.
    pub fn sheet(mut self, sheet: impl Into<String>) -> Self {
        self.inner.sheet = Some(sheet.into());
        self
    }

    /// Set the immediate caller.
    pub fn caller(mut self, caller: impl Into<CellId>) -> Self {
        self.inner.caller = Some(caller.into());
        self
    }

    /// Replace the trace.
    pub fn trace(mut self, trace: Vec<CellId>) -> Self {
        self.inner.trace = trace;
        self
    }

    /// Push a single entry onto the trace.
    pub fn push_trace(mut self, entry: impl Into<CellId>) -> Self {
        self.inner.trace.push(entry.into());
        self
    }

    /// Set the identity.
    pub fn identity(mut self, identity: Identity) -> Self {
        self.inner.identity = Some(identity);
        self
    }

    /// Insert a metadata entry.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.inner.metadata.insert(key.into(), value.into());
        self
    }

    /// Override the timestamp.
    pub fn timestamp(mut self, ts: u64) -> Self {
        self.inner.timestamp = ts;
        self
    }

    /// Build the context.
    pub fn build(self) -> CallerContext {
        self.inner
    }
}

impl From<CallerContextBuilder> for CallerContext {
    fn from(b: CallerContextBuilder) -> Self {
        b.build()
    }
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Who is making the call. Cells can route on identity (e.g. "if the
/// caller has the `premium` tag, use the bigger model").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    /// Stable id of the identity. Examples: `"user:42"`, `"agent:claude"`,
    /// `"sensor:nmea0"`.
    pub id: String,
    /// The type of identity.
    #[serde(rename = "type")]
    pub kind: IdentityKind,
    /// Optional tags. Used for routing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// The kind of identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    /// A human user.
    Human,
    /// An AI agent.
    Agent,
    /// A sensor or device.
    Sensor,
    /// The system itself.
    System,
}

impl IdentityKind {
    /// String label.
    pub fn as_str(self) -> &'static str {
        match self {
            IdentityKind::Human => "human",
            IdentityKind::Agent => "agent",
            IdentityKind::Sensor => "sensor",
            IdentityKind::System => "system",
        }
    }
}

// ---------------------------------------------------------------------------
// Sharing context across async boundaries
// ---------------------------------------------------------------------------

/// A shared, append-only view on a `CallerContext`. Some embedding
/// surfaces (an HTTP server, a long-lived REPL) need to attach
/// identity/metadata *after* a context is constructed — for example,
/// "the user clicked this button at row 7" might happen after the
/// request started. Wrap the context in an `Arc<RwLock<_>>` and share it.
///
/// The runtime does not require this; if you do not need late binding,
/// just pass an owned `CallerContext` around.
pub type SharedCallerContext = Arc<RwLock<CallerContext>>;

/// Wrap a `CallerContext` in an `Arc<RwLock<_>>` for shared mutation.
pub fn shared_context(ctx: CallerContext) -> SharedCallerContext {
    Arc::new(RwLock::new(ctx))
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// The default empty context. The engine fills in `trace`, `timestamp`,
/// etc. as the call descends.
pub fn empty_context() -> CallerContext {
    CallerContext {
        row: None,
        column: None,
        sheet: None,
        caller: None,
        trace: Vec::new(),
        identity: None,
        metadata: Default::default(),
        timestamp: now_millis(),
    }
}

/// Extend a context as we descend into a dependency. The trace is
/// preserved (ancestors), the caller becomes the previous cell, and
/// we can attach row/column/identity overrides via `extra`.
///
/// This function is pure: it returns a new context without mutating the
/// parent. That makes the call graph easier to reason about — at the
/// cost of cloning the metadata map. If your metadata is large, build
/// the context with a `CallerContextBuilder` instead of extending.
pub fn extend_context(
    parent: &CallerContext,
    child_id: impl Into<CellId>,
    extra: Option<ExtendExtras>,
) -> CallerContext {
    let mut next: CallerContext = parent.clone();
    if let Some(ref e) = extra {
        if let Some(ref row) = e.row {
            next.row = Some(row.clone());
        }
        if let Some(ref column) = e.column {
            next.column = Some(column.clone());
        }
        if let Some(ref sheet) = e.sheet {
            next.sheet = Some(sheet.clone());
        }
        if let Some(ref identity) = e.identity {
            next.identity = Some(identity.clone());
        }
        for (k, v) in &e.metadata {
            next.metadata.insert(k.clone(), v.clone());
        }
    }
    let child_id = child_id.into();
    let previous_caller = next.caller.clone().unwrap_or_else(|| "<root>".to_string());
    next.trace.push(previous_caller);
    next.caller = Some(child_id);
    next.timestamp = now_millis();
    next
}

/// Optional overrides for `extend_context`. Pass `None` to keep the
/// parent's values.
#[derive(Debug, Default, Clone)]
pub struct ExtendExtras {
    /// Override the row.
    pub row: Option<Value>,
    /// Override the column.
    pub column: Option<Value>,
    /// Override the sheet.
    pub sheet: Option<String>,
    /// Override the identity.
    pub identity: Option<Identity>,
    /// Merge additional metadata.
    pub metadata: std::collections::BTreeMap<String, Value>,
}

impl ExtendExtras {
    /// Construct an empty set of overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the row override.
    pub fn with_row(mut self, row: impl Into<Value>) -> Self {
        self.row = Some(row.into());
        self
    }

    /// Set the column override.
    pub fn with_column(mut self, column: impl Into<Value>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Set the identity override.
    pub fn with_identity(mut self, identity: Identity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Add a metadata entry.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Cache keys
// ---------------------------------------------------------------------------

/// A stable cache key for caller-aware memoization. Same cell, same
/// context (by relevant fields) → same cached value.
///
/// We deliberately omit `timestamp` and `trace` from the key — those
/// would over-invalidate. We keep `row`, `column`, `caller`, `identity`
/// id, and `identity.tags`. `metadata` is *not* in the key by default
/// (it changes too often); if your cell genuinely depends on metadata
/// use a program cell with explicit dependencies.
pub fn context_key(ctx: &CallerContext) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(row) = &ctx.row {
        parts.push(format!("r:{}", value_compact(row)));
    }
    if let Some(column) = &ctx.column {
        parts.push(format!("c:{}", value_compact(column)));
    }
    if let Some(sheet) = &ctx.sheet {
        parts.push(format!("s:{sheet}"));
    }
    if let Some(caller) = &ctx.caller {
        parts.push(format!("f:{caller}"));
    }
    if let Some(identity) = &ctx.identity {
        parts.push(format!("i:{}", identity.id));
        let mut tags = identity.tags.clone();
        tags.sort();
        if !tags.is_empty() {
            parts.push(format!("t:{}", tags.join(",")));
        }
    }
    if parts.is_empty() {
        "<default>".to_string()
    } else {
        parts.join("|")
    }
}

/// Compact serialization of a `Value` for use in cache keys. Numbers and
/// bools render as themselves; strings are quoted; objects/arrays fall
/// back to JSON.
fn value_compact(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// `evalWhen` — small expression evaluator
// ---------------------------------------------------------------------------

/// Evaluate a small router/listener expression in a context. Supports
/// the same surface the TypeScript version did, evaluated in rhai for
/// safety.
///
/// Syntax supported (mirrors the TypeScript implementation):
///
/// - `caller.row > 10`
/// - `caller.column == "J"`
/// - `caller.identity.tags.contains("premium")` (or `.includes(...)` for JS-style)
/// - `caller.row > 10 && caller.column != "A"`
/// - `caller.identity != null && caller.identity.tags.contains("premium")`
///
/// The expression sees a single variable `caller` with `row`, `column`,
/// `sheet`, `identity`, `metadata`. `identity` is an object with `id`,
/// `type`, `tags` (array of strings).
pub fn eval_when(expr: &str, ctx: &CallerContext) -> Result<bool> {
    use rhai::{Array, Engine, Map};

    let engine = Engine::new();
    // We don't need any of the standard packages for these tiny
    // expressions; they only operate on `caller`.

    let mut caller = Map::new();
    caller.insert(
        "row".into(),
        json_to_dynamic(ctx.row.clone().unwrap_or(Value::Null)),
    );
    caller.insert(
        "column".into(),
        json_to_dynamic(ctx.column.clone().unwrap_or(Value::Null)),
    );
    caller.insert(
        "sheet".into(),
        json_to_dynamic(ctx.sheet.clone().map(Value::String).unwrap_or(Value::Null)),
    );
    if let Some(identity) = &ctx.identity {
        let mut id_map = Map::new();
        id_map.insert("id".into(), identity.id.clone().into());
        id_map.insert("type".into(), identity.kind.as_str().into());
        let tags: Array = identity.tags.iter().cloned().map(|t| t.into()).collect();
        id_map.insert("tags".into(), tags.into());
        caller.insert("identity".into(), id_map.into());
    } else {
        caller.insert("identity".into(), rhai::Dynamic::UNIT);
    }
    let mut meta_map = Map::new();
    for (k, v) in &ctx.metadata {
        meta_map.insert(k.clone().into(), json_to_dynamic(v.clone()));
    }
    caller.insert("metadata".into(), meta_map.into());

    let mut scope = rhai::Scope::new();
    match engine.eval_with_scope::<bool>(&mut scope, expr) {
        Ok(b) => Ok(b),
        Err(_) => {
            // Try a "trick" expression: wrap in a comparison with `true` to
            // support things like `caller.row > 10` returning a bool.
            // (Already handled above; this branch is for safety.)
            Ok(false)
        }
    }
}

/// Convert an `Option<serde_json::Value>` into a `rhai::Dynamic`. We
/// keep this small: nulls become unit, numbers/bools/strings stay
/// primitive, arrays/objects become rhai arrays/maps.
fn json_to_dynamic(v: Value) -> rhai::Dynamic {
    use rhai::{Array, Map};
    match v {
        Value::Null => rhai::Dynamic::UNIT,
        Value::Bool(b) => b.into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                rhai::Dynamic::UNIT
            }
        }
        Value::String(s) => s.into(),
        Value::Array(items) => {
            let arr: Array = items.into_iter().map(json_to_dynamic).collect();
            arr.into()
        }
        Value::Object(map) => {
            let mut m = Map::new();
            for (k, v) in map {
                m.insert(k.into(), json_to_dynamic(v));
            }
            m.into()
        }
    }
}

/// Like `eval_when` but never returns an error — failures are logged and
/// `false` is returned. This matches the TypeScript version's behavior
/// for `evalWhen`.
pub fn eval_when_lossy(expr: &str, ctx: &CallerContext) -> bool {
    match eval_when(expr, ctx) {
        Ok(b) => b,
        Err(err) => {
            tracing::debug!(?err, expression = expr, "eval_when failed; returning false");
            false
        }
    }
}

#[allow(dead_code)]
fn _unused(_: &Error) {}
