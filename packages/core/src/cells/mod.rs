//! # cells
//!
//! The eight cell evaluators.
//!
//! ## Role in the system
//!
//! Each cell kind has its own evaluation function. The engine
//! dispatches based on `CellDef::kind`. All evaluators take a `Cell`,
//! a `CallerContext`, and (for effectful kinds) a `ProgramRuntime`
//! handle. They return a `CellValue`.
//!
//! ## Depends on
//!
//! - `crate::types` — the cell, value, context, and effect types.
//! - `crate::context` — for `context_key` (per-context memoization).
//! - `crate::error` — error type.
//! - `rhai` (program, formula) — embedded scripting.
//! - `reqwest` (api) — HTTP transport.
//!
//! ## Used by
//!
//! - `crate::engine` — dispatches to the right evaluator based on kind.
//!   (The engine is in-progress; until it lands, call the evaluators
//!   directly from your code or tests.)
//!
//! ## Status
//!
//! - ✅ `value` — implemented + tests
//! - ✅ `formula` — implemented (rhai-based) + tests
//! - ✅ `api` — implemented (reqwest-based) + tests
//! - ✅ `program` — implemented (rhai-based) + tests
//! - 🚧 `sensor` — placeholder (no value source; the engine will
//!   integrate with MQTT/Modbus/GPIO adapters in v0.2)
//! - 🚧 `io` — placeholder (same as sensor; bidirectional later)
//! - 🚧 `listener` — placeholder (depends on the engine's
//!   propagation loop)
//! - 🚧 `router` — placeholder (depends on engine and program
//!   runtime to delegate to)

pub mod api;
pub mod formula;
pub mod program;
pub mod value;

pub use api::{evaluate_api, ApiExecutor};
pub use formula::{evaluate_formula, FormulaEngine};
pub use program::{evaluate_program, ProgramRuntime};
pub use value::evaluate_value;

// Placeholders for the remaining cell kinds. These will be filled in
// alongside the engine in the next iteration. They exist as no-op
// stubs so the module compiles and consumers can refer to the names
// without a "module not found" error.

/// Push-based input (MQTT, Modbus, GPIO). Placeholder.
///
/// Real implementation will be an adapter that receives external
/// events and calls `engine.push(id, data)`. The engine is the one
/// that actually stores the value; this module is just a factory.
pub mod sensor {
    use crate::types::CellValue;

    /// Build a `CellValue` wrapping a sensor reading. Used by the
    /// engine's `push` method.
    pub fn make_sensor_value(data: serde_json::Value) -> CellValue {
        CellValue {
            data,
            status: crate::types::CellStatus::Ready,
            computed_at: Some(crate::types::now_millis()),
            error: None,
            effects: Vec::new(),
        }
    }
}

/// Bidirectional I/O port. Placeholder. See `sensor` for notes.
pub mod io {
    use crate::types::CellValue;

    /// Build a `CellValue` wrapping an I/O event.
    pub fn make_io_value(data: serde_json::Value) -> CellValue {
        CellValue {
            data,
            status: crate::types::CellStatus::Ready,
            computed_at: Some(crate::types::now_millis()),
            error: None,
            effects: Vec::new(),
        }
    }
}

/// Delta-triggered execution. Placeholder.
///
/// Real implementation will be called by the engine's propagation
/// loop. The TypeScript version is in
/// `quilt-ts/packages/core/src/cells/listener.ts` — port it once the
/// engine exists.
pub mod listener {
    use crate::error::Result;
    use crate::types::{Cell, CellId, CellValue};

    /// Fire a listener cell if its condition is met. Placeholder
    /// that always returns false.
    pub async fn fire_listener(
        _cell: &Cell,
        _changed: &CellId,
        _new: &CellValue,
        _prev: &CellValue,
        _runtime: &dyn super::ProgramRuntime,
    ) -> Result<bool> {
        Ok(false)
    }
}

/// Caller-aware policy. Placeholder.
///
/// Real implementation will use `crate::context::eval_when` to
/// evaluate rules and `ProgramRuntime::call` to delegate. Mirror
/// the TypeScript version in `quilt-ts/packages/core/src/cells/router.ts`.
pub mod router {
    use crate::error::Result;
    use crate::types::{Cell, CallerContext, CellValue};
    use serde_json::Value;

    /// Evaluate a router cell. Placeholder that always returns
    /// `Value::Null`.
    pub async fn evaluate_router(
        _cell: &Cell,
        _ctx: &CallerContext,
        _input: Option<Value>,
        _runtime: &dyn super::ProgramRuntime,
    ) -> Result<CellValue> {
        Ok(CellValue {
            data: Value::Null,
            status: crate::types::CellStatus::Ready,
            computed_at: Some(crate::types::now_millis()),
            error: None,
            effects: Vec::new(),
        })
    }
}
