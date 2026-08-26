//! # cells/value.rs
//!
//! Value cell evaluator.
//!
//! ## Role in the system
//!
//! The simplest kind. A value cell is a static piece of data with no
//! dependencies, no computation, and no effects. It always returns the
//! same value for any caller. Used for constants, configuration, and
//! the leaves of the graph.
//!
//! ## Depends on
//!
//! - `crate::types` — `Cell`, `CellValue`, `CellStatus`, `CallerContext`.
//!
//! ## Used by
//!
//! - `crate::engine` — dispatches to this when `CellDef::kind == Value`.
//!
//! ## Key decisions
//!
//! - We do not cache the result. Computing `CellValue::ready(value)` is
//!   cheap (a `serde_json::Value` clone and a timestamp) and the
//!   per-context cache lives on the `Cell` itself for cases that need
//!   it (effectful cells).
//! - `value` may be any JSON value, including `null`. The TypeScript
//!   version lets the field be `undefined`; we represent that with
//!   `Value::Null` and a present-but-null `def.value`.

use crate::types::{CallerContext, Cell, CellStatus, CellValue};

/// Evaluate a value cell. Returns the static value, wrapped in a fresh
/// `CellValue` with `status == ready` and a current timestamp.
///
/// The `ctx` is unused — value cells are context-independent by
/// definition. We accept the parameter anyway so the call site can
/// dispatch uniformly.
///
/// If the cell's current value is `Ready` (i.e. it was set at runtime
/// via `engine.set`), we return that value. Otherwise we fall back
/// to the static value declared in the cell's definition.
pub fn evaluate_value(cell: &Cell, _ctx: &CallerContext) -> CellValue {
    // Prefer the runtime-updated value if it's Ready.
    if cell.value.status == CellStatus::Ready {
        return CellValue {
            data: cell.value.data.clone(),
            status: CellStatus::Ready,
            computed_at: Some(crate::types::now_millis()),
            error: None,
            effects: Vec::new(),
        };
    }
    let data = cell.def.value.clone().unwrap_or(serde_json::Value::Null);
    CellValue {
        data,
        status: CellStatus::Ready,
        computed_at: Some(crate::types::now_millis()),
        error: None,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellDef, CellKind};
    use serde_json::json;

    fn make_cell(value: serde_json::Value) -> Cell {
        Cell::new(CellDef {
            id: "test".into(),
            kind: CellKind::Value,
            value: Some(value),
            ..Default::default()
        })
    }

    #[test]
    fn returns_static_value() {
        let cell = make_cell(json!(42));
        let v = evaluate_value(&cell, &CallerContext::default());
        assert_eq!(v.status, CellStatus::Ready);
        assert_eq!(v.data, json!(42));
    }

    #[test]
    fn null_value_is_explicit_null() {
        let cell = make_cell(serde_json::Value::Null);
        let v = evaluate_value(&cell, &CallerContext::default());
        assert_eq!(v.data, serde_json::Value::Null);
    }

    #[test]
    fn no_value_field_defaults_to_null() {
        let cell = Cell::new(CellDef {
            id: "test".into(),
            kind: CellKind::Value,
            value: None,
            ..Default::default()
        });
        let v = evaluate_value(&cell, &CallerContext::default());
        assert_eq!(v.data, serde_json::Value::Null);
    }
}
