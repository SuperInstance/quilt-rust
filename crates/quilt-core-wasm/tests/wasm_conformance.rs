//! # wasm-core conformance — the web tier's proof
//!
//! The same contract as `compat/conformance_test.rs` (the reference
//! tier), driven through `quilt-core-wasm` instead of `quilt-core`:
//! loads `compat/golden.json`, runs the five CORE OPS, and asserts
//! agreement at the reference tolerances (1e-12 numeric; bit-for-bit
//! for chain hashes and provenance seals).
//!
//! It also proves the two tiers cannot drift: the single-sourced
//! ledger module is run side by side with the native `quilt-core`
//! ledger (a dev-dependency, host-only) and the chain hashes are
//! asserted identical.
//!
//! Run:
//!
//! ```text
//! cargo test -p quilt-core-wasm --test wasm_conformance -- --nocapture
//! ```

use std::collections::{BTreeMap, BTreeSet};

use quilt_core_wasm::engine::WasmEngine;
use quilt_core_wasm::ledger::{canonical_json, sha256, CellLedger};
use serde_json::{json, Value};

const TOL_FORMULA: f64 = 1e-12;
const TOL_EDGE: f64 = 1e-12;
const TOL_RECONCILE: f64 = 1e-12;

fn golden() -> Value {
    serde_json::from_str(include_str!("../../../compat/golden.json"))
        .expect("golden.json parses")
}

fn fresh_engine(g: &Value) -> WasmEngine {
    WasmEngine::from_sheet_json(&g["sheet"]).expect("golden sheet loads into the wasm engine")
}

// -- shared assertion helpers (mirroring the reference harness) ----------------

fn assert_close(what: &str, got: &Value, want: &Value, tol: f64) {
    match (got, want) {
        (Value::Number(gn), Value::Number(wn)) => {
            let (gv, wv) = (gn.as_f64().unwrap(), wn.as_f64().unwrap());
            assert!(
                (gv - wv).abs() <= tol,
                "{what}: got {gv}, want {wv} (tol {tol})"
            );
        }
        (Value::Array(gs), Value::Array(ws)) => {
            assert_eq!(gs.len(), ws.len(), "{what}: length mismatch {gs:?} vs {ws:?}");
            for (i, (gv, wv)) in gs.iter().zip(ws.iter()).enumerate() {
                assert_close(&format!("{what}[{i}]"), gv, wv, tol);
            }
        }
        (gv, wv) => assert_eq!(gv, wv, "{what}: got {gv:?}, want {wv:?}"),
    }
}

fn assert_sha256_hex(what: &str, got: &str, want: &str) {
    assert_eq!(got.len(), 64, "{what}: not a sha256 hex string: {got}");
    assert!(
        got.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{what}: must be lowercase hex: {got}"
    );
    assert_eq!(got, want, "{what}: must be bit-for-bit");
}

// -- op (d): the wire edge ------------------------------------------------------

fn wire_delta(before: &Value, after: &Value) -> Value {
    match (before, after) {
        (Value::Number(b), Value::Number(a)) => {
            json!(a.as_f64().unwrap() - b.as_f64().unwrap())
        }
        (Value::Array(bs), Value::Array(as_)) if bs.len() == as_.len() => {
            let mut out = Vec::with_capacity(bs.len());
            for (b, a) in bs.iter().zip(as_.iter()) {
                match (b.as_f64(), a.as_f64()) {
                    (Some(b), Some(a)) => out.push(json!(a - b)),
                    _ => return Value::Null,
                }
            }
            Value::Array(out)
        }
        _ => Value::Null,
    }
}

fn wire_imbalance(before: &Value, after: &Value) -> Value {
    match (before, after) {
        (Value::Number(b), Value::Number(a)) => {
            json!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs())
        }
        (Value::Array(bs), Value::Array(as_)) if bs.len() == as_.len() => {
            let mut sum = 0.0;
            for (b, a) in bs.iter().zip(as_.iter()) {
                match (b.as_f64(), a.as_f64()) {
                    (Some(b), Some(a)) => sum += (a - b) * (a - b),
                    _ => return Value::Null,
                }
            }
            json!(sum.sqrt())
        }
        _ => Value::Null,
    }
}

fn wire_provenance(inputs: &[Value]) -> String {
    let canonical = canonical_json(&Value::Array(inputs.to_vec()));
    sha256::hex(canonical.as_bytes())
}

// -- op (c): deterministic topological order ------------------------------------

fn topo_order(graph: &BTreeMap<String, Vec<String>>, nodes: &BTreeSet<String>) -> Vec<String> {
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for id in nodes {
        indegree.insert(id.as_str(), 0);
        dependents.entry(id.as_str()).or_default();
    }
    for id in nodes {
        for dep in &graph[id.as_str()] {
            if nodes.contains(dep) {
                *indegree.get_mut(id.as_str()).unwrap() += 1;
                dependents.entry(dep.as_str()).or_default().push(id.as_str());
            }
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(id) = ready.pop_first() {
        order.push(id.to_string());
        for &dep_id in &dependents[id] {
            let d = indegree.get_mut(dep_id).unwrap();
            *d -= 1;
            if *d == 0 {
                ready.insert(dep_id);
            }
        }
    }
    assert_eq!(order.len(), nodes.len(), "dependency graph has a cycle");
    order
}

fn affected_closure(graph: &BTreeMap<String, Vec<String>>, roots: &[String]) -> BTreeSet<String> {
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, deps) in graph {
        for dep in deps {
            rev.entry(dep.as_str()).or_default().push(id.as_str());
        }
    }
    let mut seen: BTreeSet<String> = roots.iter().cloned().collect();
    let mut queue: Vec<String> = roots.to_vec();
    while let Some(id) = queue.pop() {
        if let Some(dependents) = rev.get(id.as_str()) {
            for d in dependents {
                if seen.insert(d.to_string()) {
                    queue.push(d.to_string());
                }
            }
        }
    }
    seen
}

fn golden_graph(g: &Value) -> BTreeMap<String, Vec<String>> {
    g["graph"]
        .as_object()
        .expect("golden graph is an object")
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_array()
                    .expect("deps are an array")
                    .iter()
                    .map(|d| d.as_str().expect("dep is a string").to_string())
                    .collect(),
            )
        })
        .collect()
}

// -- the conformance test --------------------------------------------------------

#[test]
fn wasm_core_conforms_to_quilt_compat_1() {
    let g = golden();
    assert_eq!(g["contract"].as_str(), Some("quilt-compat/1"));
    assert_eq!(g["spec"]["edge_schema_v"].as_u64(), Some(1));
    println!("=== wasm-core conformance (web tier: quilt-core-wasm) ===");
    println!("contract: {}  golden: compat/golden.json", g["contract"].as_str().unwrap());

    // (a) value cell read -----------------------------------------------------

    {
        let engine = fresh_engine(&g);
        let vectors = g["op_a_value_read"].as_array().expect("op_a vectors");
        for v in vectors {
            let cell = v["cell"].as_str().unwrap();
            let got = engine.get(cell).expect("get");
            assert_close(
                &format!("(a) value read {cell}"),
                &got,
                &v["expect"],
                0.0,
            );
        }
        println!("  [a] value cell read .............. PASS ({} vectors)", vectors.len());
    }

    // (b) formula cell eval ----------------------------------------------------

    {
        let mut engine = fresh_engine(&g);
        let section = &g["op_b_formula_eval"];
        for v in section["initial"].as_array().unwrap() {
            let cell = v["cell"].as_str().unwrap();
            let got = engine.get(cell).expect("get");
            assert_close(
                &format!("(b) formula eval {cell} (initial)"),
                &got,
                &v["expect"],
                TOL_FORMULA,
            );
        }
        let push = &section["after_push"];
        engine
            .push(push["cell"].as_str().unwrap(), push["value"].clone())
            .expect("push");
        for v in section["post"].as_array().unwrap() {
            let cell = v["cell"].as_str().unwrap();
            let got = engine.get(cell).expect("get");
            assert_close(
                &format!("(b) formula eval {cell} (post-push)"),
                &got,
                &v["expect"],
                TOL_FORMULA,
            );
        }
        println!(
            "  [b] formula cell eval ........... PASS ({} vectors)",
            section["initial"].as_array().unwrap().len()
                + section["post"].as_array().unwrap().len()
        );
    }

    // (c) reactive propagation order (topological) ------------------------------

    {
        let graph = golden_graph(&g);
        let section = &g["op_c_propagation"];
        let mutate = &section["mutate"];
        let root = mutate["cell"].as_str().unwrap();
        let closure = affected_closure(&graph, &[root.to_string()]);
        let order = topo_order(&graph, &closure);
        let want: Vec<String> = section["expected_order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            order, want,
            "(c) propagation order must be the deterministic topological order"
        );

        let mut engine = fresh_engine(&g);
        let declared = section["engine_dependency_graph_must_match"].as_object().unwrap();
        for (cell, deps) in declared {
            let got = engine.dependencies(cell).expect("cell exists");
            let mut expected: Vec<String> = deps
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d.as_str().unwrap().to_string())
                .collect();
            expected.sort();
            assert_eq!(
                got, expected,
                "(c) wasm-engine dependency set for {cell} must match the golden graph"
            );
        }
        engine
            .push(mutate["cell"].as_str().unwrap(), mutate["value"].clone())
            .expect("push");
        let final_level = engine.get("bilge.level").unwrap();
        assert_close("(c) post-mutation read", &final_level, &json!(85.0), 0.0);
        println!("  [c] propagation order ........... PASS (topo order + wasm-engine graph agrees)");
    }

    // (d) edge record -------------------------------------------------------------

    {
        let vectors = g["op_d_edge"].as_array().expect("op_d vectors");
        for v in vectors {
            let name = v["name"].as_str().unwrap();
            let inputs: Vec<Value> = v["inputs"].as_array().unwrap().clone();
            let delta = wire_delta(&v["before"], &v["after"]);
            assert_close(
                &format!("(d) edge {name} delta"),
                &delta,
                &v["expect"]["delta"],
                TOL_EDGE,
            );
            let imbalance = wire_imbalance(&v["before"], &v["after"]);
            assert_close(
                &format!("(d) edge {name} imbalance"),
                &imbalance,
                &v["expect"]["imbalance"],
                TOL_EDGE,
            );
            let provenance = wire_provenance(&inputs);
            assert_sha256_hex(
                &format!("(d) edge {name} provenance"),
                &provenance,
                v["expect"]["provenance"].as_str().unwrap(),
            );
        }
        println!("  [d] edge record ................. PASS ({} vectors)", vectors.len());
    }

    // (e) ledger chain-hash + reconcile ---------------------------------------------

    {
        let section = &g["op_e_chain"];
        let transcript = &section["transcript"];
        let cell = transcript["cell"].as_str().unwrap();
        let mut ledger = CellLedger::with_genesis(
            cell,
            transcript["genesis"].clone(),
            transcript["genesis_ts"].as_f64().unwrap() as u64,
        );
        for rec in transcript["records"].as_array().unwrap() {
            ledger.record(
                rec["input"].clone(),
                rec["output"].clone(),
                rec["ts"].as_f64().unwrap() as u64,
            );
        }

        for (entry, want) in ledger.entries().iter().zip(section["entries"].as_array().unwrap()) {
            assert_eq!(
                entry.seq, want["seq"].as_u64().unwrap(),
                "(e) seq must be contiguous from 1"
            );
            assert_sha256_hex(
                &format!("(e) entry {} prev_hash (chain link)", entry.seq),
                &entry.prev_hash,
                want["prev_hash"].as_str().unwrap(),
            );
            assert_sha256_hex(
                &format!("(e) entry {} seal", entry.seq),
                &entry.hash,
                want["hash"].as_str().unwrap(),
            );
        }
        assert_sha256_hex(
            "(e) chain_hash (head)",
            &ledger.chain_hash(),
            section["chain_hash"].as_str().unwrap(),
        );

        let report = ledger.reconcile();
        let want = &section["reconcile"];
        assert_eq!(report.cell_id, cell, "(e) reconcile cell_id");
        assert_eq!(report.entries, want["entries"].as_u64().unwrap() as usize);
        assert_eq!(report.open_inputs, want["open_inputs"].as_u64().unwrap() as usize);
        assert_eq!(
            report.matched_pairs,
            want["matched_pairs"].as_u64().unwrap() as usize
        );
        assert_eq!(report.chain_intact, want["chain_intact"].as_bool().unwrap());
        assert_eq!(
            report.continuity_intact,
            want["continuity_intact"].as_bool().unwrap()
        );
        assert_eq!(report.balanced, want["balanced"].as_bool().unwrap());
        assert_close(
            "(e) total_surprise",
            &json!(report.total_surprise),
            &want["total_surprise"],
            TOL_RECONCILE,
        );
        assert_close(
            "(e) mean_surprise",
            &json!(report.mean_surprise.unwrap_or(f64::NAN)),
            &want["mean_surprise"],
            TOL_RECONCILE,
        );
        println!(
            "  [e] chain + reconcile ........... PASS ({} seals bit-for-bit, books balanced)",
            ledger.len()
        );
    }

    println!("RESULT: PASS — web tier (quilt-core-wasm) conforms to quilt-compat/1");
}

// -- cross-tier: the wasm ledger IS the native ledger ------------------------------

/// The wasm crate compiles `packages/core/src/ledger.rs` directly (via
/// `#[path]`). This test makes the guarantee executable: run the golden
/// transcript through BOTH ledgers and require identical output — not
/// just against golden.json, but against each other. If either side's
/// hashing ever changes, this fails.
#[test]
fn wasm_ledger_chain_is_bit_for_bit_identical_to_native_reference_tier() {
    let g = golden();
    let transcript = &g["op_e_chain"]["transcript"];
    let cell = transcript["cell"].as_str().unwrap();

    let run_wasm = || {
        let mut l = CellLedger::with_genesis(
            cell,
            transcript["genesis"].clone(),
            transcript["genesis_ts"].as_f64().unwrap() as u64,
        );
        for rec in transcript["records"].as_array().unwrap() {
            l.record(
                rec["input"].clone(),
                rec["output"].clone(),
                rec["ts"].as_f64().unwrap() as u64,
            );
        }
        (l.chain_hash(), l.verify_chain().intact, l.reconcile().balanced)
    };
    let run_native = || {
        let mut l = quilt_core::CellLedger::with_genesis(
            cell,
            transcript["genesis"].clone(),
            transcript["genesis_ts"].as_f64().unwrap() as u64,
        );
        for rec in transcript["records"].as_array().unwrap() {
            l.record(
                rec["input"].clone(),
                rec["output"].clone(),
                rec["ts"].as_f64().unwrap() as u64,
            );
        }
        (l.chain_hash(), l.verify_chain().intact, l.reconcile().balanced)
    };

    let wasm = run_wasm();
    let native = run_native();
    assert_eq!(wasm, native, "wasm and native ledger tiers must agree bit-for-bit");
    assert_eq!(
        wasm.0,
        g["op_e_chain"]["chain_hash"].as_str().unwrap(),
        "both tiers must equal the golden chain hash"
    );
    assert!(wasm.1, "wasm chain verifies");
    assert!(wasm.2, "wasm books balance");
    println!("  [x] cross-tier chain identity ... PASS (wasm == native == golden)");
}

// -- ledger replay parity (extra: op not in golden, but part of the ledger surface) --

#[test]
fn wasm_ledger_replay_and_open_inputs_round_trip() {
    let mut ledger = CellLedger::with_genesis("cell.t", json!(40.0), 0);
    ledger.record(json!(50.0), json!(50.0), 1_000);
    ledger.record(json!(80.0), json!(80.0), 2_000);

    let r = ledger.replay(1_000);
    assert_eq!(r.state, json!(50.0));
    assert_eq!(r.replayed, 1);
    assert!((r.surprise - 10.0).abs() < 1e-12);

    // Open input / settle round trip.
    let ticket = ledger.open_input(json!(9), 3_000);
    let rec = ledger.reconcile();
    assert_eq!(rec.open_inputs, 1);
    assert!(!rec.balanced);
    ledger.settle_output(ticket, json!(90.0), 3_100).unwrap();
    let rec = ledger.reconcile();
    assert!(rec.balanced);
    assert_eq!(ledger.state(), &json!(90.0));
}
