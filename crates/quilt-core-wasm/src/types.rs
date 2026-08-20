//! # types.rs (wasm tier)
//!
//! The minimal type vocabulary for the wasm core.
//!
//! Mirrors the subset of `packages/core/src/types.rs` the sync engine
//! needs: `CellId`, the three portable cell kinds, a `CellDef` that
//! deserializes straight from the golden sheet JSON, and a `Sheet`.
//! The full `CallerContext`/`Effect`/subscription surface stays on the
//! native tier (it drags chrono clocks and channel plumbing); the ledger
//! takes its provenance by value instead.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The stable identity of a cell (same convention as the native tier:
/// a name, not a coordinate).
pub type CellId = String;

/// The cell kinds supported on the wasm tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    /// Static data, no dependencies.
    #[serde(rename = "value")]
    Value,
    /// Pure reactive computation, deps auto-detected from `expr`.
    #[serde(rename = "formula")]
    Formula,
    /// Streaming input, push-based; seeds from `default`.
    #[serde(rename = "sensor")]
    Sensor,
}

impl CellKind {
    /// Short string label, as it appears in sheet JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            CellKind::Value => "value",
            CellKind::Formula => "formula",
            CellKind::Sensor => "sensor",
        }
    }
}

/// A cell definition for the wasm tier. Unknown fields in the sheet
/// (`source`, `rate`, `endpoint`, ...) are tolerated and ignored, so a
/// full native sheet loads into the wasm engine and simply leaves its
/// effectful cells inert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellDef {
    /// The stable id of the cell. Required.
    pub id: CellId,
    /// What kind of cell this is. Required.
    pub kind: CellKind,
    /// Static value (`value` cells).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Expression body (`formula` cells). Leading `=` optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// Initial value (`sensor` cells) until something pushes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Explicit dependencies, merged with auto-detection for formulas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
}

/// A sheet: the unit of load for the wasm engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sheet {
    /// Stable id for the sheet.
    pub id: String,
    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// All the cells.
    pub cells: Vec<CellDef>,
}
