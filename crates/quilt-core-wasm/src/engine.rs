//! # engine.rs (wasm tier)
//!
//! The sync reactive engine for the wasm core.
//!
//! ## Role in the system
//!
//! A dependency-free echo of `packages/core/src/engine.rs`, restricted
//! to the portable cell kinds (`value`, `formula`, `sensor`):
//!
//! - `value` cells hold static data.
//! - `sensor` cells hold the latest push (seeded from `default`).
//! - `formula` cells re-evaluate on every `get`, pulling their inputs
//!   (auto-detected from the expression, merged with declared `deps`)
//!   recursively — lazy, always fresh, no staleness bookkeeping needed
//!   because there is no async tier to race with.
//!
//! No locks (single-threaded wasm), no channels, no subscriptions:
//! those belong to the native engine. What this engine preserves is the
//! *contract*: the dependency graph matches the native engine's edge
//! detection, and evaluation is deterministic.
//!
//! ## Used by
//!
//! - The wasm-bindgen surface (future work) and the conformance test
//!   (`tests/wasm_conformance.rs`), which drives the golden sheet
//!   through this engine and checks it against `compat/golden.json`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::formula::Formula;
use crate::types::{CellDef, CellId, CellKind, Sheet};

/// The sync reactive engine: the whole wasm-tier runtime.
#[derive(Debug, Clone, Default)]
pub struct WasmEngine {
    /// Cell definitions by id. `BTreeMap` for deterministic order.
    cells: BTreeMap<CellId, CellDef>,
    /// Pushed values for sensor cells (overrides `default`).
    pushed: BTreeMap<CellId, Value>,
    /// Compiled formulas, cached by cell id.
    formulas: BTreeMap<CellId, Formula>,
}

impl WasmEngine {
    /// An empty engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an engine from a sheet definition object (the `sheet`
    /// field of `compat/golden.json` deserializes directly). Cells of
    /// unsupported kinds are rejected loudly rather than silently
    /// misbehaving.
    pub fn from_sheet_json(sheet: &Value) -> Result<Self> {
        let sheet: Sheet = serde_json::from_value(sheet.clone())
            .map_err(|e| Error::InvalidSheet(format!("{e}")))?;
        Self::from_sheet(&sheet)
    }

    /// Build an engine from a parsed sheet.
    pub fn from_sheet(sheet: &Sheet) -> Result<Self> {
        let mut engine = Self::new();
        for def in &sheet.cells {
            engine.add_cell(def.clone())?;
        }
        Ok(engine)
    }

    /// Register one cell. Compiles formula bodies eagerly so a bad
    /// expression fails at load time, not first read.
    pub fn add_cell(&mut self, def: CellDef) -> Result<()> {
        if let CellKind::Formula = def.kind {
            let expr = def.expr.as_deref().ok_or_else(|| {
                Error::InvalidSheet(format!("formula cell '{}' has no expr", def.id))
            })?;
            let formula = Formula::compile(expr).map_err(|e| {
                Error::InvalidSheet(format!("formula cell '{}': {e}", def.id))
            })?;
            self.formulas.insert(def.id.clone(), formula);
        }
        self.cells.insert(def.id.clone(), def);
        Ok(())
    }

    /// All cell ids, sorted (BTreeMap order — deterministic).
    pub fn cell_ids(&self) -> Vec<String> {
        self.cells.keys().cloned().collect()
    }

    /// Read a cell: the universal `get` verb (sync tier). Formula cells
    /// pull their inputs recursively; cycles are detected, not spun.
    pub fn get(&self, id: &str) -> Result<Value> {
        let mut visited = HashSet::new();
        self.get_inner(id, &mut visited)
    }

    fn get_inner(&self, id: &str, visited: &mut HashSet<String>) -> Result<Value> {
        let def = self.cells.get(id).ok_or_else(|| Error::CellNotFound(id.into()))?;
        if !visited.insert(id.to_string()) {
            return Err(Error::FormulaEval(format!(
                "dependency cycle detected at '{id}'"
            )));
        }
        let value = match def.kind {
            CellKind::Value => def.value.clone().unwrap_or(Value::Null),
            CellKind::Sensor => self
                .pushed
                .get(id)
                .cloned()
                .or_else(|| def.default.clone())
                .unwrap_or(Value::Null),
            CellKind::Formula => {
                let deps = self.dependencies_inner(def)?;
                let mut env = BTreeMap::new();
                for dep in deps {
                    env.insert(dep.clone(), self.get_inner(&dep, visited)?);
                }
                let formula = self
                    .formulas
                    .get(id)
                    .ok_or_else(|| Error::FormulaEval(format!("cell '{id}': formula not compiled")))?;
                formula.eval(&env)?
            }
        };
        visited.remove(id);
        Ok(value)
    }

    /// Push a new reading into a sensor cell (also accepts value cells,
    /// mirroring the native engine's set-paths). Formulas cannot be
    /// pushed; they derive.
    pub fn push(&mut self, id: &str, value: Value) -> Result<()> {
        let def = self.cells.get(id).ok_or_else(|| Error::CellNotFound(id.into()))?;
        match def.kind {
            CellKind::Sensor | CellKind::Value => {
                self.pushed.insert(id.to_string(), value);
                Ok(())
            }
            CellKind::Formula => Err(Error::NotPushable {
                id: id.into(),
                kind: def.kind.as_str().into(),
            }),
        }
    }

    /// The dependency edges of a cell: what `get(id)` reads. For
    /// formulas this is auto-detected identifiers ∩ known cell ids,
    /// unioned with declared `deps` — the same edge set the native
    /// engine must produce (golden op c).
    pub fn dependencies(&self, id: &str) -> Result<Vec<String>> {
        let def = self.cells.get(id).ok_or_else(|| Error::CellNotFound(id.into()))?;
        let mut deps = self.dependencies_inner(def)?;
        deps.sort();
        Ok(deps)
    }

    fn dependencies_inner(&self, def: &CellDef) -> Result<Vec<String>> {
        if let CellKind::Formula = def.kind {
            let known: HashSet<&str> = self.cells.keys().map(|s| s.as_str()).collect();
            let mut deps: BTreeSet<String> = def.deps.iter().cloned().collect();
            if let Some(formula) = self.formulas.get(&def.id) {
                for ident in formula.dependencies() {
                    if known.contains(ident.as_str()) {
                        deps.insert(ident);
                    }
                }
            }
            Ok(deps.into_iter().collect())
        } else {
            Ok(def.deps.clone())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn golden_sheet() -> Value {
        serde_json::json!({
            "id": "bilge-reflex",
            "cells": [
                {"id": "bilge.level", "kind": "sensor", "source": "simulated", "default": 40.0},
                {"id": "bilge.threshold", "kind": "value", "value": 80.0},
                {"id": "pump.should_run", "kind": "formula", "expr": "=bilge.level >= bilge.threshold"},
                {"id": "pump.relay_cmd", "kind": "formula", "expr": "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)"},
                {"id": "status", "kind": "value", "value": "idle"}
            ]
        })
    }

    #[test]
    fn reads_values_and_evaluates_formulas_lazily() {
        let engine = WasmEngine::from_sheet_json(&golden_sheet()).unwrap();
        assert_eq!(engine.get("bilge.threshold").unwrap(), json!(80.0));
        assert_eq!(engine.get("status").unwrap(), json!("idle"));
        assert_eq!(engine.get("bilge.level").unwrap(), json!(40.0));
        assert_eq!(engine.get("pump.should_run").unwrap(), json!(false));
        assert_eq!(engine.get("pump.relay_cmd").unwrap(), json!(-20.0));
    }

    #[test]
    fn push_propagates_to_formulas_on_next_read() {
        let mut engine = WasmEngine::from_sheet_json(&golden_sheet()).unwrap();
        engine.push("bilge.level", json!(85.0)).unwrap();
        assert_eq!(engine.get("bilge.level").unwrap(), json!(85.0));
        assert_eq!(engine.get("pump.should_run").unwrap(), json!(true));
        assert_eq!(engine.get("pump.relay_cmd").unwrap(), json!(2.5));
    }

    #[test]
    fn dependency_edges_match_the_golden_graph() {
        let engine = WasmEngine::from_sheet_json(&golden_sheet()).unwrap();
        assert_eq!(
            engine.dependencies("pump.should_run").unwrap(),
            vec!["bilge.level", "bilge.threshold"]
        );
        assert_eq!(
            engine.dependencies("pump.relay_cmd").unwrap(),
            vec!["bilge.level", "bilge.threshold"]
        );
        assert!(engine.dependencies("status").unwrap().is_empty());
        assert!(engine.dependencies("no.such.cell").is_err());
    }

    #[test]
    fn cycles_are_detected_not_spun() {
        let sheet = serde_json::json!({
            "id": "loop",
            "cells": [
                {"id": "a", "kind": "formula", "expr": "=b + 1"},
                {"id": "b", "kind": "formula", "expr": "=a + 1"}
            ]
        });
        let engine = WasmEngine::from_sheet_json(&sheet).unwrap();
        assert!(engine.get("a").is_err());
    }

    #[test]
    fn unknown_and_unsupported_kinds_fail_loudly() {
        let engine = WasmEngine::new();
        assert!(engine.get("ghost").is_err());
        let sheet = serde_json::json!({
            "id": "s",
            "cells": [{"id": "x", "kind": "api", "endpoint": "https://example.com"}]
        });
        assert!(WasmEngine::from_sheet_json(&sheet).is_err());
        // Formula push is rejected.
        let mut engine = WasmEngine::from_sheet_json(&golden_sheet()).unwrap();
        assert!(engine.push("pump.should_run", json!(1.0)).is_err());
    }
}
