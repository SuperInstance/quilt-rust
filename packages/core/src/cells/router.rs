//! # cells/router.rs
//!
//! Caller-aware policy cells. Routers pick a destination based on
//! the caller's context, then delegate the call to that cell.
//!
//! ## Role in the system
//!
//! A `router` cell is the engine's primitive for "route this call
//! based on who's asking." It's a generalization of routing tables,
//! model selectors, and policy decision points. A router has a
//! list of `rules`, each with a `when` condition and a `route`
//! target. The first matching rule wins; if no rule matches, the
//! router returns the input unchanged (or a configurable default).
//!
//! ## Depends on
//!
//! - `crate::types` — `Cell`, `CallerContext`, `CellValue`, `RouteTarget`.
//! - `crate::error` — `Result`.
//! - `crate::cells::ProgramRuntime` — to call the chosen cell.
//!
//! ## Used by
//!
//! - The engine's `call` method, which dispatches to a router and
//!   receives whatever the router chose.
//!
//! ## Key decision
//!
//! A router is **synchronous** in the call chain: when you `call`
//! a router, the engine evaluates the rules in order, picks the
//! first match, calls the target cell, and returns *that* cell's
//! value. The router itself is invisible at the call site — the
//! caller just sees a cell that returned a value.
//!
//! The `route` target can be:
//!   - A bare cell id → delegate to that cell.
//!   - A `Cell` reference with `with` overrides → delegate, with
//!     context merged.
//!   - A `Model` spec → swap implementations (e.g. gpt-4o vs
//!     claude-sonnet-4-5). This is where the model-swap pattern
//!     lives.
//!   - A literal `Value` → return that without any further call.
//!
//! The MVP supports the `CellId` and literal `Value` variants.
//! `Cell` and `Model` are TODO.

use crate::cells::ProgramRuntime;
use crate::error::Result;
use crate::types::{now_millis, Cell, CallerContext, CellStatus, CellValue, RouteTarget};
use serde_json::Value;
use std::sync::Arc;

/// Evaluate a router cell. The first matching rule wins.
///
/// Takes the `Cell` by value (not by reference) so the returned
/// future is `Send` and can be moved across thread boundaries by
/// `drive_async`. The cell is cheap to clone (small struct).
pub async fn evaluate_router(
    cell: Cell,
    ctx: CallerContext,
    input: Option<Value>,
    runtime: Arc<dyn ProgramRuntime>,
) -> Result<CellValue> {
    let started_at = now_millis();

    for rule in &cell.def.rules {
        if rule_matches(&rule.when, &ctx, input.as_ref()) {
            return match &rule.route {
                RouteTarget::CellId(id) => runtime.call(id, input, &ctx),
                RouteTarget::Cell { cell: target, with } => {
                    // Build a merged context and delegate.
                    let mut new_ctx = ctx.clone();
                    if let Some(overrides) = with {
                        for (k, v) in overrides {
                            new_ctx.metadata.insert(k.clone(), v.clone());
                        }
                    }
                    runtime.call(target, input, &new_ctx)
                }
                RouteTarget::Model { model: _ } => {
                    // Model swap is a v0.2 feature. For now, return
                    // the input unchanged with an error.
                    Ok(CellValue {
                        data: input.unwrap_or(Value::Null),
                        status: CellStatus::Error,
                        computed_at: Some(started_at),
                        error: Some(crate::types::CellError::new(
                            "model swap routing is a v0.2 feature",
                        )),
                        effects: Vec::new(),
                    })
                }
                RouteTarget::Value { value } => {
                    Ok(CellValue {
                        data: value.clone(),
                        status: CellStatus::Ready,
                        computed_at: Some(started_at),
                        error: None,
                        effects: Vec::new(),
                    })
                }
            };
        }
    }

    // No rule matched. Return input (or null) unchanged.
    Ok(CellValue {
        data: input.unwrap_or(Value::Null),
        status: CellStatus::Ready,
        computed_at: Some(now_millis()),
        error: None,
        effects: Vec::new(),
    })
}

// Placeholder: the `with` variable in the Cell variant. Workaround
// for the borrow checker — we extract it explicitly.
fn _unused() {
    let _ = std::collections::BTreeMap::<String, Value>::new();
}

/// Best-effort rule matching. The MVP supports:
///   - "true" / "false" literals
///   - "caller.row == <value>" (string compare)
///   - "caller.column == <value>" (string compare)
///   - bare cell id (match if the cell is truthy in current cache)
///   - "input == <value>" (compare to call input)
///
/// Anything else returns false (err on the side of skipping).
fn rule_matches(when: &str, ctx: &CallerContext, input: Option<&Value>) -> bool {
    let when = when.trim();
    if when == "true" {
        return true;
    }
    if when == "false" {
        return false;
    }
    if let Some(rest) = when.strip_prefix("caller.row == ") {
        let rest = rest.trim_matches('"');
        return ctx
            .row
            .as_ref()
            .and_then(|v| v.as_str())
            .map(|s| s == rest)
            .unwrap_or(false);
    }
    if let Some(rest) = when.strip_prefix("caller.column == ") {
        let rest = rest.trim_matches('"');
        return ctx
            .column
            .as_ref()
            .and_then(|v| v.as_str())
            .map(|s| s == rest)
            .unwrap_or(false);
    }
    if let Some(rest) = when.strip_prefix("input == ") {
        if let (Some(target), Some(input)) = (
            serde_json::from_str::<Value>(rest).ok(),
            input,
        ) {
            return *input == target;
        }
        return false;
    }
    // Unknown rule: don't match.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CellDef, CellKind, CellStatus, RouteTarget, RouterRule,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_router_cell(rules: Vec<RouterRule>) -> Cell {
        let mut def = CellDef {
            id: "r".to_string(),
            kind: CellKind::Router,
            description: None,
            value: None,
            expr: None,
            endpoint: None,
            method: None,
            headers: BTreeMap::new(),
            code: None,
            source: None,
            rate: None,
            default: None,
            watch: Vec::new(),
            condition: None,
            action: None,
            rules,
            port: None,
            direction: None,
            unit: None,
            input_type: None,
            output_type: None,
            deps: Vec::new(),
            permissions: BTreeMap::new(),
        };
        def.id = "r".to_string();
        Cell::new(def)
    }

    struct StaticRuntime;
    impl ProgramRuntime for StaticRuntime {
        fn get(&self, _id: &str, _ctx: &CallerContext) -> Result<CellValue> {
            unimplemented!()
        }
        fn set(&self, _id: &str, _value: Value, _ctx: &CallerContext) -> Result<()> {
            unimplemented!()
        }
        fn call(&self, id: &str, _input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
            // Always return a marker so we can see what was called.
            Ok(CellValue::ready(json!({"called": id})))
        }
        fn list(&self) -> Vec<String> { vec![] }
    }

    #[tokio::test]
    async fn matches_first_rule() {
        let rules = vec![
            RouterRule {
                when: "true".to_string(),
                route: RouteTarget::CellId("a".to_string()),
            },
            RouterRule {
                when: "true".to_string(),
                route: RouteTarget::CellId("b".to_string()),
            },
        ];
        let cell = make_router_cell(rules);
        let result = evaluate_router(cell, CallerContext::default(), None, Arc::new(StaticRuntime))
            .await
            .unwrap();
        assert_eq!(result.data["called"], "a");
    }

    #[tokio::test]
    async fn matches_on_caller_row() {
        let rules = vec![RouterRule {
            when: "caller.row == \"premium\"".to_string(),
            route: RouteTarget::CellId("expensive".to_string()),
        }];
        let cell = make_router_cell(rules);
        let mut ctx = CallerContext::default();
        ctx.row = Some(json!("premium"));
        let result = evaluate_router(cell, ctx, None, Arc::new(StaticRuntime)).await.unwrap();
        assert_eq!(result.data["called"], "expensive");
    }

    #[tokio::test]
    async fn no_match_returns_input() {
        let rules = vec![RouterRule {
            when: "false".to_string(),
            route: RouteTarget::CellId("nope".to_string()),
        }];
        let cell = make_router_cell(rules);
        let input = json!({"hello": "world"});
        let result = evaluate_router(
            cell,
            CallerContext::default(),
            Some(input.clone()),
            Arc::new(StaticRuntime),
        )
        .await
        .unwrap();
        assert_eq!(result.data, input);
        assert_eq!(result.status, CellStatus::Ready);
    }

    #[tokio::test]
    async fn literal_value_route() {
        let rules = vec![RouterRule {
            when: "true".to_string(),
            route: RouteTarget::Value { value: json!("fallback") },
        }];
        let cell = make_router_cell(rules);
        let result = evaluate_router(cell, CallerContext::default(), None, Arc::new(StaticRuntime))
            .await
            .unwrap();
        assert_eq!(result.data, json!("fallback"));
    }
}
