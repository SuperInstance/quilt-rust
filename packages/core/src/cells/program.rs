//! # cells/program.rs
//!
//! Program cell evaluator.
//!
//! ## Role in the system
//!
//! Stateful, side-effectful logic. The user provides a script (rhai in
//! the Rust implementation). The script receives the cell's input,
//! the caller context, a `runtime` handle, and helper functions. It
//! returns the new value.
//!
//! ## Depends on
//!
//! - `rhai` — embedded scripting language.
//! - `crate::types` — `Cell`, `CellValue`, `CallerContext`.
//!
//! ## Used by
//!
//! - `crate::engine` — dispatches to this on `get`/`call` for `program`
//!   cells.
//!
//! ## Key decisions
//!
//! - Rhai is sandboxed by default. We do not register any of the I/O
//!   packages (`File`, `Http`, etc.) in the engine, so a program cell
//!   cannot read files or make network requests. The `runtime` handle
//!   it does receive is the *only* way to reach the outside world.
//! - The runtime handle exposed to the script is a small `ProgramRuntime`
//!   trait object. The script calls `runtime.get("foo")`, `runtime.set("foo", x)`,
//!   `runtime.call("foo", x)`, and `runtime.list()`.
//! - We compile once per evaluation. The script body can be tiny, so
//!   the compile cost is dominated by the rhai parse, not the eval. If
//!   programs become hot we can cache `AST`s in the cell def.

use std::sync::Arc;

use rhai::{Array, Dynamic, Engine, Map, Scope};
use serde_json::Value;

use crate::error::Result;
use crate::types::{now_millis, CallerContext, Cell, CellStatus, CellValue};

/// What a program cell sees as `runtime`. A small subset of the engine's
/// public surface — get, set, call, list — enough to build interesting
/// reactive logic without exposing internals.
pub trait ProgramRuntime: Send + Sync {
    /// Get a cell's current value. Wraps `QuiltEngine::get`.
    fn get(&self, id: &str, ctx: &CallerContext) -> Result<CellValue>;
    /// Set a cell's value. Wraps `QuiltEngine::set`.
    fn set(&self, id: &str, value: Value, ctx: &CallerContext) -> Result<()>;
    /// Call a cell as a capability. Wraps `QuiltEngine::call`.
    fn call(&self, id: &str, input: Option<Value>, ctx: &CallerContext) -> Result<CellValue>;
    /// List all defined cell ids.
    fn list(&self) -> Vec<String>;
}

impl<T: ProgramRuntime + ?Sized> ProgramRuntime for Arc<T> {
    fn get(&self, id: &str, ctx: &CallerContext) -> Result<CellValue> {
        (**self).get(id, ctx)
    }
    fn set(&self, id: &str, value: Value, ctx: &CallerContext) -> Result<()> {
        (**self).set(id, value, ctx)
    }
    fn call(&self, id: &str, input: Option<Value>, ctx: &CallerContext) -> Result<CellValue> {
        (**self).call(id, input, ctx)
    }
    fn list(&self) -> Vec<String> {
        (**self).list()
    }
}

/// A no-op runtime, used in unit tests that exercise a program cell
/// without the full engine.
pub struct NullRuntime;

impl ProgramRuntime for NullRuntime {
    fn get(&self, _id: &str, _ctx: &CallerContext) -> Result<CellValue> {
        Ok(CellValue::default())
    }
    fn set(&self, _id: &str, _value: Value, _ctx: &CallerContext) -> Result<()> {
        Ok(())
    }
    fn call(&self, _id: &str, _input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
        Ok(CellValue::default())
    }
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Evaluate a program cell. Takes the `Cell` by value so the
/// returned future is `Send` and can be moved across thread
/// boundaries by `drive_async`.
pub async fn evaluate_program(
    cell: Cell,
    ctx: CallerContext,
    input: Option<Value>,
    runtime: Arc<dyn ProgramRuntime>,
) -> CellValue {
    let started_at = now_millis();
    let code = match &cell.def.code {
        Some(c) => c.clone(),
        None => return CellValue::err("program cell has no code"),
    };

    let mut engine = Engine::new();
    // No I/O packages are registered. The runtime handle is the only
    // way out.

    let mut scope = Scope::new();

    // Bind the runtime handle. We expose the runtime methods as
    // top-level scope variables (`get`, `set`, `call`, `list`) so the
    // user script can write `get("a")` directly. The TS version of
    // Quilt uses `runtime.get("a")`; the Rust port uses the bare
    // form because rhai's map-with-callable-values story is
    // fragile.
    //
    // We use `Engine::register_fn` to bind real Rust functions
    // (which rhai can dispatch on) and capture the runtime + ctx
    // via move. This requires us to register on a fresh engine,
    // which is fine — the engine is per-evaluation.
    // We expose the runtime methods under the `q` namespace
    // (`qget`, `qset`, `qcall`, `qlist`) to avoid colliding with
    // rhai's built-in `call` (which is a keyword for calling a
    // function pointer). The TS version of Quilt uses
    // `runtime.get("a")`; the Rust port uses `qget("a")`.
    let rt = Arc::clone(&runtime);
    let rt_ctx = ctx.clone();
    engine.register_fn("qget", move |id: &str| -> rhai::Dynamic {
        match rt.get(id, &rt_ctx) {
            Ok(v) => cell_value_to_dynamic(v),
            Err(e) => {
                let mut m = rhai::Map::new();
                m.insert("__quilt_error".into(), true.into());
                m.insert("message".into(), format!("runtime.get failed: {e}").into());
                m.into()
            }
        }
    });
    let rt2 = Arc::clone(&runtime);
    let rt2_ctx = ctx.clone();
    engine.register_fn("qset", move |id: &str, value: rhai::Dynamic| -> () {
        let v = dynamic_to_json(value);
        rt2.set(id, v, &rt2_ctx)
            .map_err(|e| format!("{e}"))
            .unwrap_or(());
    });
    let rt3 = Arc::clone(&runtime);
    let rt3_ctx = ctx.clone();
    engine.register_fn(
        "qcall",
        move |id: &str, input: rhai::Dynamic| -> rhai::Dynamic {
            let input_value = dynamic_to_json(input);
            let input_opt = if input_value.is_null() {
                None
            } else {
                Some(input_value)
            };
            match rt3.call(id, input_opt, &rt3_ctx) {
                Ok(v) => cell_value_to_dynamic(v),
                Err(e) => {
                    let mut m = rhai::Map::new();
                    m.insert("__quilt_error".into(), true.into());
                    m.insert("message".into(), format!("runtime.call failed: {e}").into());
                    m.into()
                }
            }
        },
    );
    let rt4 = Arc::clone(&runtime);
    engine.register_fn("qlist", move || -> rhai::Array {
        rt4.list().into_iter().map(|s| s.into()).collect()
    });

    // `includes` and `contains` for arrays — rhai 1.x doesn't
    // expose these as built-ins on Dynamic arrays. We register
    // them so the user can write `tags.includes("premium")` in
    // their scripts.
    engine.register_fn("includes", |arr: rhai::Array, needle: &str| -> bool {
        arr.iter().any(|v| {
            v.clone()
                .into_string()
                .map(|s| s == needle)
                .unwrap_or(false)
        })
    });

    // Bind input.
    scope.push_dynamic("input", json_to_dynamic(input.unwrap_or(Value::Null)));

    // Bind caller.
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
        caller.insert("identity".into(), Dynamic::UNIT);
    }
    let mut meta = Map::new();
    for (k, v) in &ctx.metadata {
        meta.insert(k.clone().into(), json_to_dynamic(v.clone()));
    }
    caller.insert("metadata".into(), meta.into());
    scope.push_dynamic("caller", caller.into());

    // Bind helpers.
    scope.push(
        "clamp",
        clamp_fn as fn(rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>>,
    );
    scope.push(
        "abs",
        abs_fn as fn(Dynamic) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>>,
    );
    scope.push(
        "min",
        min_fn as fn(rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>>,
    );
    scope.push(
        "max",
        max_fn as fn(rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>>,
    );

    // Run.
    let result = match engine.eval_with_scope::<Dynamic>(&mut scope, &code) {
        Ok(v) => v,
        Err(err) => {
            return CellValue::err_with_stack(
                format!("script error: {err}"),
                format!("in cell '{}'", cell.id),
            );
        }
    };

    let data = dynamic_to_json(result);
    let duration = now_millis().saturating_sub(started_at);
    CellValue {
        data,
        status: CellStatus::Ready,
        computed_at: Some(now_millis()),
        error: None,
        effects: vec![crate::types::Effect::Compute { ms: duration }],
    }
}

// ---------------------------------------------------------------------------
// Runtime handle exposed to the script
// ---------------------------------------------------------------------------

fn cell_value_to_dynamic(v: CellValue) -> Dynamic {
    let mut map = Map::new();
    map.insert("data".into(), json_to_dynamic(v.data));
    map.insert("status".into(), v.status.as_str().to_string().into());
    if let Some(ts) = v.computed_at {
        map.insert("computedAt".into(), (ts as i64).into());
    }
    if let Some(err) = v.error {
        let mut err_map = Map::new();
        err_map.insert("message".into(), err.message.into());
        if let Some(s) = err.stack {
            err_map.insert("stack".into(), s.into());
        }
        map.insert("error".into(), err_map.into());
    }
    map.into()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn clamp_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    if args.len() != 3 {
        return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
            "clamp(n, lo, hi) takes three arguments".into(),
            rhai::Position::NONE,
        )));
    }
    let n = match args[0].as_float() {
        Ok(n) => n,
        Err(e) => {
            return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            )))
        }
    };
    let lo = match args[1].as_float() {
        Ok(n) => n,
        Err(e) => {
            return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            )))
        }
    };
    let hi = match args[2].as_float() {
        Ok(n) => n,
        Err(e) => {
            return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            )))
        }
    };
    Ok(n.clamp(lo, hi).into())
}

fn abs_fn(x: Dynamic) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let n = match x.as_float() {
        Ok(n) => n,
        Err(e) => {
            return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            )))
        }
    };
    Ok(n.abs().into())
}

fn min_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let mut best = f64::INFINITY;
    for a in args {
        let n = match a.as_float() {
            Ok(n) => n,
            Err(e) => {
                return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                    e.to_string().into(),
                    rhai::Position::NONE,
                )))
            }
        };
        if n < best {
            best = n;
        }
    }
    Ok(best.into())
}

fn max_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let mut best = f64::NEG_INFINITY;
    for a in args {
        let n = match a.as_float() {
            Ok(n) => n,
            Err(e) => {
                return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
                    e.to_string().into(),
                    rhai::Position::NONE,
                )))
            }
        };
        if n > best {
            best = n;
        }
    }
    Ok(best.into())
}

// ---------------------------------------------------------------------------
// serde_json ↔ rhai conversion (mirrors formula.rs)
// ---------------------------------------------------------------------------

fn json_to_dynamic(v: Value) -> Dynamic {
    use rhai::{Array, Map};
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => b.into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                Dynamic::UNIT
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

fn dynamic_to_json(d: Dynamic) -> Value {
    if d.is_unit() {
        return Value::Null;
    }
    if let Some(b) = d.as_bool().ok() {
        return Value::Bool(b);
    }
    if let Some(i) = d.as_int().ok() {
        return Value::Number(i.into());
    }
    if let Some(f) = d.as_float().ok() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    if let Ok(s) = d.clone().into_string() {
        return Value::String(s);
    }
    if let Ok(arr) = d.clone().into_array() {
        let items: Vec<Value> = arr.into_iter().map(dynamic_to_json).collect();
        return Value::Array(items);
    }
    if let Some(map) = d.clone().try_cast::<Map>() {
        let mut obj = serde_json::Map::new();
        for (k, v) in map {
            obj.insert(k.to_string(), dynamic_to_json(v));
        }
        return Value::Object(obj);
    }
    if let Ok(s) = d.into_string() {
        return Value::String(s);
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellDef, CellKind};
    use std::sync::Arc;

    fn program_cell(code: &str) -> Cell {
        Cell::new(CellDef {
            id: "p".into(),
            kind: CellKind::Program,
            code: Some(code.to_string()),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn simple_return() {
        let cell = program_cell("1 + 2");
        let v = evaluate_program(cell, CallerContext::default(), None, Arc::new(NullRuntime)).await;
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data, serde_json::json!(3));
    }

    #[tokio::test]
    async fn returns_object() {
        let cell = program_cell("#{ action: \"turn_left\", degrees: 10 }");
        let v = evaluate_program(cell, CallerContext::default(), None, Arc::new(NullRuntime)).await;
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data["action"], "turn_left");
        assert_eq!(v.data["degrees"], 10);
    }

    #[tokio::test]
    async fn caller_visible() {
        let cell = program_cell("caller.row");
        let mut ctx = CallerContext::default();
        ctx.row = Some(serde_json::json!(42));
        let v = evaluate_program(cell, ctx, None, Arc::new(NullRuntime)).await;
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data, serde_json::json!(42));
    }

    #[tokio::test]
    async fn runtime_handle_works() {
        use std::sync::Mutex;
        struct CountingRuntime {
            gets: Mutex<Vec<String>>,
        }
        impl ProgramRuntime for CountingRuntime {
            fn get(&self, id: &str, _ctx: &CallerContext) -> Result<CellValue> {
                self.gets.lock().unwrap().push(id.to_string());
                Ok(CellValue::ready(serde_json::json!(99)))
            }
            fn set(&self, _id: &str, _v: Value, _ctx: &CallerContext) -> Result<()> {
                Ok(())
            }
            fn call(
                &self,
                _id: &str,
                _i: Option<Value>,
                _ctx: &CallerContext,
            ) -> Result<CellValue> {
                Ok(CellValue::default())
            }
            fn list(&self) -> Vec<String> {
                Vec::new()
            }
        }
        let rt = Arc::new(CountingRuntime {
            gets: Mutex::new(Vec::new()),
        });
        let cell = program_cell("let v = qget(\"a\"); v.data");
        let v = evaluate_program(
            cell,
            CallerContext::default(),
            None,
            Arc::clone(&rt) as Arc<dyn ProgramRuntime>,
        )
        .await;
        assert_eq!(v.data, serde_json::json!(99));
        assert_eq!(*rt.gets.lock().unwrap(), vec!["a"]);
    }
}
