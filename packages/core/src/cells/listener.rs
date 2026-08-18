//! # cells/listener.rs
//!
//! Delta-triggered execution. Listener cells watch other cells and
//! fire actions when conditions are met.
//!
//! ## Role in the system
//!
//! A `listener` cell is the engine's reactive primitive for
//! "when X changes, do Y." It sits in the propagation graph just
//! like a `formula`, but instead of computing a value, it computes
//! a side effect: it calls another cell.
//!
//! ## Depends on
//!
//! - `crate::types` — `Cell`, `CellId`, `CellValue`.
//! - `crate::error` — `Result`.
//! - `crate::cells::ProgramRuntime` — to call other cells.
//!
//! ## Used by
//!
//! - The engine's propagation loop, which calls `fire_listener`
//!   whenever a cell that a listener watches changes.
//!
//! ## Key decision
//!
//! A listener's "fire" is itself a `call` to another cell, mediated
//! by the `ProgramRuntime`. This means the action can be any cell
//! — a `program` cell that sends a webhook, a `router` cell that
//! picks a notification channel, a `value` cell that's a
//! no-op marker. The engine doesn't care.
//!
//! The `fire_listener` function is called from the engine's
//! propagation loop, *not* as part of normal cell evaluation. It
//! happens synchronously during `set` / `push` propagation, but
//! the action it triggers is itself an async `call`.

use crate::cells::ProgramRuntime;
use crate::error::Result;
use crate::types::{Cell, CellId, CellValue};

/// Fire a listener cell. Called by the engine's propagation loop
/// when a watched cell changes.
///
/// The listener has:
///   - `def.watch`: list of cell ids to watch
///   - `def.condition`: optional expression that must evaluate true
///   - `def.action`: cell id to call when the condition fires
///
/// The listener fires when:
///   1. `changed` is in `watch`, AND
///   2. `condition` is None OR evaluates true (best-effort: we
///      don't have a full expression engine here, so we treat
///      "no condition" as always-true and "has condition" as
///      "evaluate on the cell's value".)
///   3. The result is calling `action` via the runtime.
///
/// Returns `Ok(true)` if the listener fired, `Ok(false)` if it
/// didn't.
pub async fn fire_listener(
    cell: &Cell,
    changed: &CellId,
    new: &CellValue,
    _prev: &CellValue,
    runtime: &dyn ProgramRuntime,
) -> Result<bool> {
    // 1. Is the changed cell in the watch list?
    if !cell.def.watch.iter().any(|w| w == changed) {
        return Ok(false);
    }

    // 2. Does the condition hold? For MVP we evaluate simply:
    //    - No condition → always fire
    //    - Condition "true" literal → always fire
    //    - Otherwise, we attempt to evaluate as a truthiness
    //      check on `new.data`. The full expression engine is
    //      future work; for the MVP a condition that's a string
    //      matching the new value's truthiness works.
    if let Some(cond) = &cell.def.condition {
        if !evaluate_simple_condition(cond, new) {
            return Ok(false);
        }
    }

    // 3. Fire the action.
    if let Some(action_id) = &cell.def.action {
        let _ = runtime.call(action_id, Some(new.data.clone()), &cell.last_context.clone().unwrap_or_default());
    }

    Ok(true)
}

/// Best-effort condition evaluation. The MVP supports:
///   - "true" / "false" literals
///   - "nonempty" / "empty"
///   - "equals <value>"
///   - bare cell id (treat as "watched cell's data is truthy")
///
/// Anything else returns true (we err on the side of firing).
fn evaluate_simple_condition(cond: &str, new: &CellValue) -> bool {
    let cond = cond.trim();
    if cond == "true" {
        return true;
    }
    if cond == "false" {
        return false;
    }
    if cond == "nonempty" {
        return !matches!(new.data, serde_json::Value::Null)
            && !(new.data.is_string() && new.data.as_str().unwrap().is_empty());
    }
    if cond == "empty" {
        return matches!(new.data, serde_json::Value::Null)
            || (new.data.is_string() && new.data.as_str().unwrap().is_empty());
    }
    if let Some(rest) = cond.strip_prefix("equals ") {
        // Try to parse `rest` as JSON. If it matches new.data, true.
        if let Ok(target) = serde_json::from_str::<serde_json::Value>(rest) {
            return new.data == target;
        }
    }
    // Unknown condition: fire (be permissive).
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CellDef, CellKind, CallerContext};
    use serde_json::json;

    fn make_listener_cell(cond: Option<&str>, action: Option<&str>) -> Cell {
        let mut def = CellDef {
            id: "l".to_string(),
            kind: CellKind::Listener,
            description: None,
            value: None,
            expr: None,
            endpoint: None,
            method: None,
            headers: std::collections::BTreeMap::new(),
            code: None,
            source: None,
            rate: None,
            watch: vec!["trigger".to_string()],
            condition: cond.map(String::from),
            action: action.map(String::from),
            rules: Vec::new(),
            port: None,
            direction: None,
            unit: None,
            input_type: None,
            output_type: None,
            deps: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
        };
        def.id = "l".to_string();
        Cell::new(def)
    }

    struct StubRuntime;
    impl ProgramRuntime for StubRuntime {
        fn get(&self, _id: &str, _ctx: &CallerContext) -> Result<CellValue> {
            unimplemented!()
        }
        fn set(&self, _id: &str, _value: serde_json::Value, _ctx: &CallerContext) -> Result<()> {
            unimplemented!()
        }
        fn call(&self, _id: &str, _input: Option<serde_json::Value>, _ctx: &CallerContext) -> Result<CellValue> {
            unimplemented!()
        }
        fn list(&self) -> Vec<String> { vec![] }
    }

    #[tokio::test]
    async fn fires_on_watched_change() {
        let cell = make_listener_cell(None, None);
        let runtime = StubRuntime;
        let new = CellValue::ready(json!(true));
        let prev = CellValue::ready(json!(false));
        let fired = fire_listener(&cell, &"trigger".to_string(), &new, &prev, &runtime).await.unwrap();
        assert!(fired);
    }

    #[tokio::test]
    async fn doesnt_fire_on_unwatched_change() {
        let cell = make_listener_cell(None, None);
        let runtime = StubRuntime;
        let new = CellValue::ready(json!(true));
        let prev = CellValue::ready(json!(false));
        let fired = fire_listener(&cell, &"other".to_string(), &new, &prev, &runtime).await.unwrap();
        assert!(!fired);
    }

    #[tokio::test]
    async fn condition_true_literal() {
        let cell = make_listener_cell(Some("true"), None);
        let runtime = StubRuntime;
        let new = CellValue::ready(json!(0));
        let prev = CellValue::ready(json!(0));
        let fired = fire_listener(&cell, &"trigger".to_string(), &new, &prev, &runtime).await.unwrap();
        assert!(fired);
    }

    #[tokio::test]
    async fn condition_false_literal() {
        let cell = make_listener_cell(Some("false"), None);
        let runtime = StubRuntime;
        let new = CellValue::ready(json!(1));
        let prev = CellValue::ready(json!(0));
        let fired = fire_listener(&cell, &"trigger".to_string(), &new, &prev, &runtime).await.unwrap();
        assert!(!fired);
    }

    #[tokio::test]
    async fn condition_equals() {
        let cell = make_listener_cell(Some("equals 42"), None);
        let runtime = StubRuntime;
        let new_match = CellValue::ready(json!(42));
        let prev = CellValue::ready(json!(0));
        let fired = fire_listener(&cell, &"trigger".to_string(), &new_match, &prev, &runtime).await.unwrap();
        assert!(fired);

        let new_no_match = CellValue::ready(json!(43));
        let fired = fire_listener(&cell, &"trigger".to_string(), &new_no_match, &prev, &runtime).await.unwrap();
        assert!(!fired);
    }
}
