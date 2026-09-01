//! selfimprove-probe — objective evaluation harness for the quilt engine.
//!
//! Reads a JSON array of scenarios from stdin, runs each against a fresh
//! `QuiltEngine`, writes a JSON array of results to stdout. One line of
//! JSON in, one JSON array out. No randomness, no clocks in the verdict
//! path (the engine stamps times internally but the probe never compares
//! them).
//!
//! Scenario format:
//! {
//!   "id": "string",
//!   "cells": [
//!     {"id": "a", "kind": "value", "value": <json>},
//!     {"id": "f", "kind": "formula", "expr": "a * 3", "deps": ["a"]}
//!   ],
//!   "script": [
//!     {"op": "get",  "id": "f"},
//!     {"op": "set",  "id": "a", "value": 5},
//!     {"op": "call", "id": "f"}
//!   ]
//! }
//!
//! Result format:
//! {
//!   "id": "string",
//!   "ok": true|false,          // every op succeeded (no engine error, no panic)
//!   "steps": [{"op":"get","value":<json>|null,"error":null|"..."} , ...],
//!   "reads": [<value of every get/call op, in order>],
//!   "error": null | "panic: ..." | "setup: ..."
//! }
//!
//! The harness compares `reads` (or the last read) against expectations.

use std::io::{self, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use quilt_core::{CallerContext, CellDef, CellKind, QuiltEngine};

#[derive(Deserialize)]
struct Scenario {
    id: String,
    #[serde(default)]
    cells: Vec<CellSpec>,
    #[serde(default)]
    script: Vec<Op>,
}

#[derive(Deserialize)]
struct CellSpec {
    id: String,
    kind: String, // "value" | "formula"
    #[serde(default)]
    value: Value,
    #[serde(default)]
    expr: Option<String>,
    #[serde(default)]
    deps: Vec<String>,
}

#[derive(Deserialize)]
struct Op {
    op: String, // "get" | "set" | "call"
    id: String,
    #[serde(default)]
    value: Value,
}

#[derive(Serialize)]
struct StepOut {
    op: String,
    value: Value,
    error: Option<String>,
}

#[derive(Serialize)]
struct ResultOut {
    id: String,
    ok: bool,
    steps: Vec<StepOut>,
    reads: Vec<Value>,
    error: Option<String>,
}

fn build_cell(spec: &CellSpec) -> CellDef {
    let mut def = CellDef::default();
    def.id = spec.id.clone();
    def.kind = match spec.kind.as_str() {
        "value" => CellKind::Value,
        _ => CellKind::Formula,
    };
    if def.kind == CellKind::Value {
        def.value = Some(spec.value.clone());
    } else {
        def.expr = Some(spec.expr.clone().unwrap_or_else(|| "null".into()));
        def.deps = spec.deps.clone();
    }
    def
}

fn run_scenario(sc: &Scenario) -> ResultOut {
    let engine = QuiltEngine::new("probe").into_arc();
    for spec in &sc.cells {
        // Registration failures are recorded as setup errors.
        if let Err(e) = engine.register(build_cell(spec)) {
            return ResultOut {
                id: sc.id.clone(),
                ok: false,
                steps: vec![],
                reads: vec![],
                error: Some(format!("setup: {e}")),
            };
        }
    }

    let ctx = CallerContext::default();
    let mut steps = Vec::new();
    let mut reads = Vec::new();
    let mut all_ok = true;

    for op in &sc.script {
        let mut value = Value::Null;
        let mut err: Option<String> = None;
        match op.op.as_str() {
            "get" => match engine.get(&op.id, ctx.clone()) {
                Ok(cv) => value = cv.data,
                Err(e) => err = Some(e.to_string()),
            },
            "call" => match engine.call(&op.id, None, ctx.clone()) {
                Ok(cv) => value = cv.data,
                Err(e) => err = Some(e.to_string()),
            },
            "set" => match engine.set(&op.id, op.value.clone(), ctx.clone()) {
                Ok(()) => value = op.value.clone(),
                Err(e) => err = Some(e.to_string()),
            },
            other => err = Some(format!("unknown op: {other}")),
        }
        if err.is_some() {
            all_ok = false;
        }
        if op.op == "get" || op.op == "call" {
            reads.push(value.clone());
        }
        steps.push(StepOut {
            op: op.op.clone(),
            value,
            error: err,
        });
    }

    ResultOut {
        id: sc.id.clone(),
        ok: all_ok,
        steps,
        reads,
        error: None,
    }
}

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("probe: failed to read stdin");
        std::process::exit(2);
    }
    let scenarios: Vec<Scenario> = match serde_json::from_str(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("probe: bad input json: {e}");
            std::process::exit(2);
        }
    };

    let mut results = Vec::with_capacity(scenarios.len());
    for sc in &scenarios {
        let out = match catch_unwind(AssertUnwindSafe(|| run_scenario(sc))) {
            Ok(r) => r,
            Err(p) => {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                ResultOut {
                    id: sc.id.clone(),
                    ok: false,
                    steps: vec![],
                    reads: vec![],
                    error: Some(format!("panic: {msg}")),
                }
            }
        };
        results.push(out);
    }

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = serde_json::to_writer(&mut handle, &results);
    let _ = handle.write_all(b"\n");
}
