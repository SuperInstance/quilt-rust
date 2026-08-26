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
use crate::types::{now_millis, CallerContext, Cell, CellId, CellStatus, CellValue};

/// An owned formula evaluator. Holds the compiled AST. Compiling once
/// and re-running is much cheaper than re-parsing on every call when a
/// formula is hot.
#[derive(Debug, Clone)]
pub struct FormulaEngine {
    /// The original source, for error messages.
    pub source: Arc<str>,
    /// The compiled AST.
    pub ast: Arc<AST>,
    /// The list of known cell ids that the engine pre-processes the
    /// expression for. Used at compile time to rewrite `id` →
    /// `cells["id"]` so the user can write `a + b` instead of
    /// `cells["a"] + cells["b"]`.
    pub known_ids: Arc<Vec<String>>,
}

impl FormulaEngine {
    /// Compile a formula expression. Strips a leading `=` if present.
    ///
    /// `known_ids` is the list of cell ids that the user is allowed to
    /// reference by their bare name. Each occurrence of an id in the
    /// expression is rewritten to `cells["id"]` at compile time, so the
    /// user can write `a + b` and it works.
    ///
    /// # Errors
    ///
    /// Returns `Error::ScriptError` if rhai can't parse the expression.
    pub fn compile(source: &str, known_ids: &[String]) -> Result<Self> {
        let body = source
            .strip_prefix('=')
            .unwrap_or(source)
            .trim()
            .to_string();

        // Rewrite known ids to `cells["id"]` bracket access. Sort
        // longest-first so that `compass.heading` is rewritten before
        // `compass`.
        let rewritten = rewrite_known_ids(&body, known_ids);

        let mut engine = Engine::new();
        register_helpers(&mut engine);
        let ast = engine.compile(&rewritten).map_err(|e| Error::ScriptError {
            cell: "<compile>".into(),
            message: format!("could not compile formula: {e}"),
        })?;
        Ok(Self {
            source: Arc::from(source),
            ast: Arc::new(ast),
            known_ids: Arc::new(known_ids.to_vec()),
        })
    }

    /// Evaluate the compiled formula with a snapshot of cell values and
    /// a caller context.
    pub fn eval(&self, cell_values: &HashMap<CellId, Value>, ctx: &CallerContext) -> Result<Value> {
        let mut engine = Engine::new();
        register_helpers(&mut engine);

        let mut scope = Scope::new();

        // Build the `cells` object: a rhai map keyed by cell id.
        let mut cells_map = Map::new();
        for (id, value) in cell_values {
            cells_map.insert(id.as_str().into(), json_to_dynamic(value.clone()));
        }
        scope.push_dynamic("cells", cells_map.into());

        // Build the `caller` object.
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
        let mut meta = Map::new();
        for (k, v) in &ctx.metadata {
            meta.insert(k.clone().into(), json_to_dynamic(v.clone()));
        }
        caller.insert("metadata".into(), meta.into());
        scope.push_dynamic("caller", caller.into());

        // Register helpers as scope variables (used to be here for
        // backwards compat; we now register on the engine in
        // `register_helpers`).
        let _ = abs_fn; // suppress unused warnings
        let _ = min_fn;
        let _ = max_fn;
        let _ = clamp_fn;

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

/// Rewrite occurrences of known cell ids in an expression to
/// `cells["id"]` bracket access. This is what lets the user write
/// `a + b` instead of `cells["a"] + cells["b"]`.
///
/// The rewriter is aware of:
/// - `cells["..."]` blocks: we don't rewrite inside an existing
///   `cells[...]` block (the id is already a string literal there).
/// - String literals: we don't rewrite inside `"..."` or `'...'`.
///
/// We use a character-by-character scan with whole-token matching so
/// we don't replace substrings of longer identifiers. The token
/// boundary is "not alphanumeric, not underscore, not dot" on either
/// side. The dot exclusion means `compass` won't match inside
/// `compass.heading` (the longer id will).
fn rewrite_known_ids(body: &str, known_ids: &[String]) -> String {
    if known_ids.is_empty() {
        return body.to_string();
    }

    // Sort longest-first so that `compass.heading` is rewritten
    // before `compass`.
    let mut sorted: Vec<&str> = known_ids.iter().map(|s| s.as_str()).collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.len()));

    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len() + 64);
    let mut i = 0;
    while i < chars.len() {
        // Are we inside a string literal? If so, copy through to
        // the closing quote.
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            out.push(chars[i]);
            i += 1;
            while i < chars.len() && chars[i] != quote {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // Are we inside an existing `cells[...]` block? Track
        // bracket depth.
        if chars[i] == 'c' && i + 5 < chars.len() && &body[i..i + 5] == "cells" {
            // Check it's followed by `[`.
            if i + 5 < chars.len() && chars[i + 5] == '[' {
                out.push_str("cells[");
                i += 6;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '[' {
                        depth += 1;
                    } else if chars[i] == ']' {
                        depth -= 1;
                    }
                    out.push(chars[i]);
                    i += 1;
                }
                continue;
            }
        }
        // Try to match a known id at this position.
        let mut matched = false;
        for id in &sorted {
            let id_chars: Vec<char> = id.chars().collect();
            if i + id_chars.len() > chars.len() {
                continue;
            }
            let mut equal = true;
            for (j, c) in id_chars.iter().enumerate() {
                if chars[i + j] != *c {
                    equal = false;
                    break;
                }
            }
            if !equal {
                continue;
            }
            let left_ok = if i == 0 {
                true
            } else {
                let prev = chars[i - 1];
                !prev.is_alphanumeric() && prev != '_' && prev != '.'
            };
            let right_ok = if i + id_chars.len() == chars.len() {
                true
            } else {
                let next = chars[i + id_chars.len()];
                !next.is_alphanumeric() && next != '_' && next != '.'
            };
            if left_ok && right_ok {
                out.push_str(&format!("cells[\"{}\"]", id));
                i += id_chars.len();
                matched = true;
                break;
            }
        }
        if !matched {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
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

    // The known ids for the rewrite pass. We use the keys of
    // `cell_values` (which the engine built up from the cell's
    // dependencies). The set may be empty for formulas that have
    // no dependencies.
    let known_ids: Vec<String> = cell_values.keys().cloned().collect();

    let engine = match FormulaEngine::compile(&expr, &known_ids) {
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
    // Register the numeric helpers on the engine. We register
    // multiple overloads so the user can call them with both
    // integer and float arguments, and also with rhai arrays
    // (so the user can write `max([a, b, c])`). Rhai doesn't
    // have a single `register_fn` for `Dynamic` that accepts
    // variadic types, so we register one overload per (name,
    // arg type) combination.
    engine.register_fn("abs", abs_i64_fn);
    engine.register_fn("abs", abs_f64_fn);
    engine.register_fn("min", min_i64_fn);
    engine.register_fn("min", min_f64_fn);
    engine.register_fn("min", min_array_fn);
    engine.register_fn("max", max_i64_fn);
    engine.register_fn("max", max_f64_fn);
    engine.register_fn("max", max_array_fn);
    engine.register_fn("clamp", clamp_i64_fn);
    engine.register_fn("clamp", clamp_f64_fn);
    engine.register_fn("clamp", clamp_array_fn);
}

// Concrete typed implementations of the helpers. Rhai requires
// concrete types for `register_fn`, so we register one overload
// per (name, arg type) combination.

fn abs_i64_fn(x: i64) -> i64 {
    x.abs()
}
fn abs_f64_fn(x: f64) -> f64 {
    x.abs()
}
fn min_i64_fn(a: i64, b: i64) -> i64 {
    a.min(b)
}
fn min_f64_fn(a: f64, b: f64) -> f64 {
    a.min(b)
}
fn max_i64_fn(a: i64, b: i64) -> i64 {
    a.max(b)
}
fn max_f64_fn(a: f64, b: f64) -> f64 {
    a.max(b)
}
fn clamp_i64_fn(n: i64, lo: i64, hi: i64) -> i64 {
    n.clamp(lo, hi)
}
fn clamp_f64_fn(n: f64, lo: f64, hi: f64) -> f64 {
    n.clamp(lo, hi)
}

/// Array variants. Allow the user to write `max([a, b, c])` and
/// `clamp([n, lo, hi])` in addition to the binary forms.
fn min_array_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let mut best: Option<f64> = None;
    for a in args {
        let n = a
            .as_float()
            .or_else(|_| a.as_int().map(|i| i as f64))
            .map_err(|e| {
                Box::new(rhai::EvalAltResult::ErrorRuntime(
                    e.to_string().into(),
                    rhai::Position::NONE,
                ))
            })?;
        best = Some(best.map_or(n, |b| b.min(n)));
    }
    best.map(rhai::Dynamic::from).ok_or_else(|| {
        Box::new(rhai::EvalAltResult::ErrorRuntime(
            "min() requires at least one argument".into(),
            rhai::Position::NONE,
        ))
    })
}
fn max_array_fn(args: rhai::Array) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    let mut best: Option<f64> = None;
    for a in args {
        let n = a
            .as_float()
            .or_else(|_| a.as_int().map(|i| i as f64))
            .map_err(|e| {
                Box::new(rhai::EvalAltResult::ErrorRuntime(
                    e.to_string().into(),
                    rhai::Position::NONE,
                ))
            })?;
        best = Some(best.map_or(n, |b| b.max(n)));
    }
    best.map(rhai::Dynamic::from).ok_or_else(|| {
        Box::new(rhai::EvalAltResult::ErrorRuntime(
            "max() requires at least one argument".into(),
            rhai::Position::NONE,
        ))
    })
}
fn clamp_array_fn(
    args: rhai::Array,
) -> std::result::Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
    if args.len() != 3 {
        return Err(Box::new(rhai::EvalAltResult::ErrorRuntime(
            "clamp(arr) needs [n, lo, hi]".into(),
            rhai::Position::NONE,
        )));
    }
    let n = args[0]
        .as_float()
        .or_else(|_| args[0].as_int().map(|i| i as f64))
        .map_err(|e| {
            Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            ))
        })?;
    let lo = args[1]
        .as_float()
        .or_else(|_| args[1].as_int().map(|i| i as f64))
        .map_err(|e| {
            Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            ))
        })?;
    let hi = args[2]
        .as_float()
        .or_else(|_| args[2].as_int().map(|i| i as f64))
        .map_err(|e| {
            Box::new(rhai::EvalAltResult::ErrorRuntime(
                e.to_string().into(),
                rhai::Position::NONE,
            ))
        })?;
    Ok(rhai::Dynamic::from(n.clamp(lo, hi)))
}

fn abs_fn(_x: rhai::Dynamic) -> rhai::Dynamic {
    rhai::Dynamic::UNIT
}
fn min_fn(_a: rhai::Dynamic, _b: rhai::Dynamic) -> rhai::Dynamic {
    rhai::Dynamic::UNIT
}
fn max_fn(_a: rhai::Dynamic, _b: rhai::Dynamic) -> rhai::Dynamic {
    rhai::Dynamic::UNIT
}
fn clamp_fn(_n: rhai::Dynamic, _lo: rhai::Dynamic, _hi: rhai::Dynamic) -> rhai::Dynamic {
    rhai::Dynamic::UNIT
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
        let v = run(
            "cells[\"a\"] + cells[\"b\"]",
            &[("a", json!(3)), ("b", json!(4))],
        );
        assert_eq!(v.data, json!(7));
    }

    #[test]
    fn helper_clamp() {
        let v = run("clamp(temp, 0, 100)", &[("temp", json!(150))]);
        eprintln!(
            "DEBUG: helper_clamp data={:?} status={:?} error={:?}",
            v.data, v.status, v.error
        );
        assert_eq!(v.data, json!(100));
    }

    #[test]
    fn caller_row_visible() {
        let cell = make_formula_cell("if caller.row > 10 { \"premium\" } else { \"basic\" }");
        let cell_values = HashMap::new();
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
        // The intent: a divide-by-zero should not crash the engine.
        // In JavaScript, `1 / 0` returns `Infinity` (status: Ready).
        // In rhai, it returns an error. Both are valid "didn't crash"
        // outcomes. The engine should produce *some* CellValue
        // (either Ready with Infinity, or Error with a message),
        // not panic.
        let v = run("1 / 0", &[]);
        // Just verify we got a result without panicking.
        // The exact status is implementation-defined: rhai errors,
        // JS returns Infinity. We accept either.
        assert!(
            v.status == CellStatus::Ready || v.status == CellStatus::Error,
            "expected Ready or Error, got {:?}",
            v.status
        );
    }
}
