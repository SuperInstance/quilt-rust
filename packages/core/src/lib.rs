//! # quilt-core
//!
//! The reactive cell runtime. This is the substrate that every other
//! surface (CLI, MCP, future TUI/Web) sits on top of.
//!
//! ## Role in the system
//!
//! `quilt-core` holds the cell graph, tracks dependencies, propagates
//! changes, and exposes the universal verbs `get` / `set` / `call` / `push`.
//! It is the only crate that actually evaluates cells.
//!
//! ## Depends on
//!
//! - `serde`, `serde_json` — for `CellDef` parsing and runtime values
//! - `serde_yml` — YAML loader (the canonical sheet format)
//! - `tokio` — async runtime
//! - `rhai` — embedded scripting for `program` and `formula` cells
//! - `reqwest` — HTTP transport for `api` cells
//! - `thiserror` / `anyhow` — error types
//!
//! ## Used by
//!
//! - `quilt-mcp` — wraps an engine in an MCP server
//! - `quilt-cli` — wraps an engine in a command-line interface
//!
//! ## Status (v0.1.0-alpha)
//!
//! This crate is the Rust port of [`quilt`](https://github.com/superinstance/quilt).
//! As of this commit, the foundation is in place:
//!
//! - ✅ Type vocabulary (`types.rs`) — `Cell`, `CellDef`, `CellValue`, etc.
//! - ✅ Error types (`error.rs`) — typed errors via `thiserror`.
//! - ✅ Caller context (`context.rs`) — extension, hashing, `eval_when`.
//! - ✅ Cell evaluators: `value`, `formula`, `api`, `program`.
//! - 🚧 Engine (`engine.rs`) — the runtime that holds the cell graph.
//!   This is the most important missing piece. The TypeScript engine
//!   in `quilt/packages/core/src/engine.ts` is the spec; the Rust
//!   port should mirror it.
//! - 🚧 Parser (`parser.rs`) — YAML loader (the canonical sheet format).
//! - 🚧 Remaining cell evaluators: `sensor`, `io`, `listener`, `router`.
//!   These are small and mostly delegate to make_*_value or
//!   `fire_listener` from the context module.
//! - ❌ Scheduler (`scheduler.rs`) — async evaluation queue with
//!   backpressure. (Not strictly needed for MVP; the engine can
//!   evaluate synchronously and only go async for `api`/`program`.)
//! - ❌ MCP server (`quilt-mcp`) — exposes cells as MCP tools.
//! - ❌ CLI (`quilt-cli`) — command-line interface.
//!
//! What works today: the type system, the error system, the context
//! propagation, and four of eight cell evaluators. To run a sheet
//! you currently need to either complete the engine or call the
//! evaluators directly. See `tests/` for examples.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod cells;
pub mod context;
pub mod engine;
pub mod error;
pub mod parser;
pub mod types;

// Re-exports for convenience. Most users only need these.
pub use crate::context::{
    context_key, empty_context, eval_when, extend_context, Identity,
};
pub use crate::types::CallerContext;
pub use crate::engine::{EngineOptions, QuiltEngine, SubscriptionEvent, SubscriptionHandle};
pub use crate::error::{Error, Result};
pub use crate::parser::{parse_sheet, serialize_sheet, validate_sheet};
pub use crate::types::{
    Cell, CellDef, CellError, CellId, CellKind, CellRef, CellStatus, CellValue,
    Direction, Effect, EvaluationTrace, RouteTarget, RouterRule, SheetDef, Subscription,
    SubscriptionId,
};
