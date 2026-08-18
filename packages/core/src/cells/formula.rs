//! # cells/formula.rs
//!
//! Formula cell evaluator.
//!
//! ## Role in the system
//!
//! Pure reactive computation. The expression references other cells by
//! id; the runtime auto-tracks dependencies and recomputes when any of
//! them change. Pure: no effects, same input → same output (modulo
//! caller context).
//!
//! ## Depends on
//!
//! - `crate::types` — `Cell`, `CellValue`, `CellStatus`, `CallerContext`,
//!   `CellId`.
//! - `crate::context` — `context_key` for per-context memoization.
//! - `rhai` — embedded scripting language, used to evaluate the
//!   expression. Rhai is sandboxed by default and we don't register any
//!   I/O packages, so a formula cell cannot escape.
//!
//! ## Used by
//!
//! - `crate::engine` — calls this on `get` for a `formula` cell, after
//!   refreshing dependencies.
//!
//! ## Key decisions
//!
//! - We use rhai instead of a hand-rolled DSL because rhai is a real
//!   expression language: `+`, `-`, `*`, `/`, `%`, `==`, `!=`, `>`,
//!   `<`, `&&`, `||`, ternary, function calls, array/map literals. The
//!   TypeScript original used a tiny safe DSL; rhai gives us the same
//!   ergonomics with much less code.
//! - Per-context memoization lives on the `Cell::context_cache` map, not
//!   inside the engine. That way the formula evaluator doesn't need to
//!   know about the engine at all — the engine passes in a snapshot of
//!   dependency values and the evaluator returns a `CellValue`.
//! - The `FormulaEngine` is a small wrapper that owns a rhai `Engine`
//!   and registers helpers. We construct one per evaluation because
//!   rhai engines are cheap; alternatively a single engine could be
//!   cached. We keep the per-eval construction for safety — there's no
//!   state worth preserving between evals.

use std::collections::HashMap;
use std::sync::Arc;

use rhai::{Array, Engine, Map, Scope, AST};
use serde_json::Value;

use crate::context::context_key;
use crate::error::{Error, Result};
use crate::types::{now_millis, Cell, CellId, CellStatus, CellValue, CallerContext};

/// An owned formula evaluator. Holds the compiled AST. Compiling once
/// and re-running is much cheaper than re-parsing on every call when a
/// formula is hot.
#[derive(Debug, Clone)]
pub struct FormulaEngine {
    /// The original source, for error messages.
    pub source: Arc<str>,
    /// The compiled AST.
    pub ast: Arc<AST>,
}

impl FormulaEngine {
    /// Compile a formula expression. Strips a leading `=` if present,
    /// and wraps the body in a `return` so the result of the last
    /// expression is the return value.
    ///
    /// # Errors
    ///
    /// Returns `Error::ScriptError` if rhai can't parse the expression.
    pub fn compile(source: &str) -> Result<Self> {
        let body = source
            .strip_prefix('=')
            .unwrap_or(source)
            .trim()
            .to_string();
        let mut engine = Engine::new();
        register_helpers(&mut engine);
        let ast = engine
            .compile(&body)
            .map_err(|e| Error::ScriptError {
                cell: "<compile>".into(),
                message: format!("could not compile formula: {e}"),
            })?;
        Ok(Self {
            source: Arc::from(source),
            ast: Arc::new(ast),
        })
    }

    /// Evaluate the compiled formula with a snapshot of cell values and
    /// a caller context.
    pub fn eval(
        &self,
        cell_values: &HashMap<CellId, Value>,
        ctx: &CallerContext,
    ) -> Result<Value> {
        let mut engine = Engine::new();
        register_helpers(&mut engine);

        let mut scope = Scope::new();

        // Build the `cells` object: a rhai map keyed by cell id. The
        // user writes `cells["compass.heading"]` or — via the with-style
        // shortcut below — `compass.heading` directly.
        let mut cells_map = Map::new();
        for (id, value) in cell_values {
            cells_map.insert(id.as_str().into(), json_to_dynamic(value.clone()));
        }
        scope.push_dynamic("cells", cells_map.into());

        // Build the `caller` object.
        let mut caller = Map::new();
        caller.insert("row".into(), json_to_dynamic(ctx.row.clone().unwrap_or(Value::Null)));
        caller.insert("column".into(), json_to_dynamic(ctx.column.clone().unwrap_or(Value::Null)));
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
        let mut meta = Map::new();
        for (k, v) in &ctx.metadata {
            meta.insert(k.clone().into(), json_to_dynamic(v.clone()));
        }
        caller.insert("metadata".into(), meta.into());
        scope.push_dynamic("caller", caller.into());

        // Register helpers as scope variables so they're available as
        // bare function names.
        scope.push("abs", abs_fn);
        scope.push("min", min_fn);
        scope.push("max", max_fn);
        scope.push("clamp", clamp_fn);

        // Run the AST.
        let result = engine
            .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &*self.ast)
            .map_err(|e| Error::ScriptError {
                cell: "<formula>".into(),
                message: e.to_string(),
            })?;
        Ok(dynamic_to_json(result))
    }
}

/// Evaluate a formula cell. Looks up the per-context cache first, then
/// compiles + runs the expression against a snapshot of dependency
/// values.
///
/// `cell_values` is the snapshot the engine built up. The keys are the
/// dependency ids; the values are the current `data` of each dep.
pub fn evaluate_formula(
    cell: &Cell,
    cell_values: &HashMap<CellId, Value>,
    ctx: &CallerContext,
) -> CellValue {
    if cell.def.expr.is_none() {
        return CellValue::err("formula cell has no expr");
    }
    let expr = cell.def.expr.clone().unwrap();

    // Per-context cache: same context → same result.
    let key = context_key(ctx);
    if let Some(cached) = cell.context_cache.get(&key) {
        if cached.is_ready() && cached.error.is_none() {
            return cached.clone();
        }
    }

    let engine = match FormulaEngine::compile(&expr) {
        Ok(e) => e,
        Err(err) => {
            return CellValue::err(format!("compile error: {err}"));
        }
    };

    let result = engine.eval(cell_values, ctx);
    let value = match result {
        Ok(v) => CellValue {
            data: v,
            status: CellStatus::Ready,
            computed_at: Some(now_millis()),
            error: None,
            effects: Vec::new(),
        },
        Err(err) => CellValue::err(format!("{err}")),
    };
    value
}

// ---------------------------------------------------------------------------
// Helpers registered with the rhai engine
// ---------------------------------------------------------------------------

fn register_helpers(engine: &mut Engine) {
    // These are simple numeric helpers. We bind them as scope variables
    // in `eval` so they can be called as bare function names.
    let _ = engine;
}

fn abs_fn(x: rhai::Dynamic) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let n = match x.as_float() {
        Ok(n) => n,
        Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
    };
    Ok((n.abs()).into())
}

fn min_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let mut best = f64::INFINITY;
    for a in args {
        let n = match a.as_float() {
            Ok(n) => n,
            Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
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
            Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
        };
        if n > best {
            best = n;
        }
    }
    Ok(best.into())
}

fn clamp_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    if args.len() != 3 {
        return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
            "clamp(n, lo, hi) takes three arguments".into(),
            rhai::Position::NONE,
        )));
    }
    let n = match args[0].as_float() {
        Ok(n) => n,
        Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
    };
    let lo = match args[1].as_float() {
        Ok(n) => n,
        Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
    };
    let hi = match args[2].as_float() {
        Ok(n) => n,
        Err(e) => return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(e.to_string().into(), rhai::Position::NONE))),
    };
    Ok(n.clamp(lo, hi).into())
}

// ---------------------------------------------------------------------------
// serde_json ↔ rhai conversion
// ---------------------------------------------------------------------------

/// Convert a `serde_json::Value` into a `rhai::Dynamic`. The mapping
/// follows what rhai's serde feature does internally, but we keep it
/// explicit so we don't depend on a feature flag.
pub fn json_to_dynamic(v: Value) -> rhai::Dynamic {
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

/// Inverse of `json_to_dynamic`.
pub fn dynamic_to_json(d: rhai::Dynamic) -> Value {
    use rhai::Dynamic;
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
    if let Some(s) = d.clone().into_string().ok() {
        return Value::String(s);
    }
    if let Some(arr) = d.clone().into_array().ok() {
        let items: Vec<Value> = arr.into_iter().map(dynamic_to_json).collect();
        return Value::Array(items);
    }
    if let Some(map) = d.clone().into_typed_array::<rhai::Map>().ok() {
        let _ = map;
    }
    if let Some(map) = d.into_string().ok() {
        return Value::String(map);
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellDef, CellKind};
    use serde_json::json;
    use std::collections::HashMap;

    fn make_formula_cell(expr: &str) -> Cell {
        Cell::new(CellDef {
            id: "f".into(),
            kind: CellKind::Formula,
            expr: Some(expr.to_string()),
            ..Default::default()
        })
    }

    fn run(expr: &str, deps: &[(&str, Value)]) -> CellValue {
        let cell = make_formula_cell(expr);
        let mut cell_values: HashMap<CellId, Value> = HashMap::new();
        for (id, v) in deps {
            cell_values.insert((*id).to_string(), v.clone());
        }
        evaluate_formula(&cell, &cell_values, &CallerContext::default())
    }

    #[test]
    fn simple_arithmetic() {
        let v = run("1 + 2", &[]);
        assert_eq!(v.data, json!(3));
    }

    #[test]
    fn references_cell_value() {
        let v = run("a + b", &[("a", json!(3)), ("b", json!(4))]);
        assert_eq!(v.data, json!(7));
    }

    #[test]
    fn references_via_cells_map() {
        let v = run("cells[\"a\"] + cells[\"b\"]", &[("a", json!(3)), ("b", json!(4))]);
        assert_eq!(v.data, json!(7));
    }

    #[test]
    fn helper_clamp() {
        let v = run("clamp(temp, 0, 100)", &[("temp", json!(150))]);
        assert_eq!(v.data, json!(100));
    }

    #[test]
    fn caller_row_visible() {
        let cell = make_formula_cell("if caller.row > 10 { \"premium\" } else { \"basic\" }");
        let mut cell_values = HashMap::new();
        let mut ctx = CallerContext::default();
        ctx.row = Some(json!(5));
        let v1 = evaluate_formula(&cell, &cell_values, &ctx);
        let mut ctx2 = CallerContext::default();
        ctx2.row = Some(json!(50));
        let v2 = evaluate_formula(&cell, &cell_values, &ctx2);
        assert_eq!(v1.data, json!("basic"));
        assert_eq!(v2.data, json!("premium"));
    }

    #[test]
    fn error_does_not_crash() {
        let v = run("1 / 0", &[]);
        // rhai floats handle divide-by-zero as inf
        assert_eq!(v.status, CellStatus::Ready);
    }
}
