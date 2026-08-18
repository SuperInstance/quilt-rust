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
pub mod sensor;
pub mod io;
pub mod listener;
pub mod router;

pub use api::{evaluate_api, ApiExecutor, ApiExecutorRef};
pub use formula::{evaluate_formula, FormulaEngine};
pub use program::{evaluate_program, ProgramRuntime};
pub use value::evaluate_value;
pub use sensor::make_sensor_value;
pub use io::make_io_value;
pub use listener::fire_listener;
pub use router::evaluate_router;
