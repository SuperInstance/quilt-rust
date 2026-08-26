//! # quilt-compat conformance harness — the reference tier's proof
//!
//! The machine-checkable side of `docs/quilt-compat-contract.md`. Loads
//! `compat/golden.json`, runs the five CORE OPS every tier must
//! reproduce, and asserts agreement at the reference tier's tolerances
//! (1e-12 numeric; bit-for-bit for chain hashes).
//!
//! Run:
//!
//! ```text
//! cargo test -p quilt-core --test quilt_compat_conformance -- --nocapture
//! ```
//!
//! Regenerate the reference hashes embedded in `golden.json` (after a
//! deliberate contract change, never to make a failure go away):
//!
//! ```text
//! cargo test -p quilt-core --test quilt_compat_conformance -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use quilt_core::ledger::{canonical_json, sha256};
use quilt_core::{CallerContext, CellLedger, QuiltEngine, SheetDef};
use serde_json::{json, Value};

const TOL_FORMULA: f64 = 1e-12;
const TOL_EDGE: f64 = 1e-12;
const TOL_RECONCILE: f64 = 1e-12;

fn golden() -> Value {
    serde_json::from_str(include_str!("golden.json")).expect("golden.json parses")
}

fn fresh_engine(g: &Value) -> Arc<QuiltEngine> {
    let sheet: SheetDef =
        serde_json::from_value(g["sheet"].clone()).expect("golden sheet deserializes");
    let engine = QuiltEngine::new("conformance").into_arc();
    engine.load_sheet(sheet).expect("sheet loads");
    engine
}

// -- shared assertion helpers ----------------------------------------------

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
            assert_eq!(
                gs.len(),
                ws.len(),
                "{what}: length mismatch {gs:?} vs {ws:?}"
            );
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
        got.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{what}: must be lowercase hex: {got}"
    );
    assert_eq!(got, want, "{what}: must be bit-for-bit");
}

// -- op (d): the wire edge, reference math -----------------------------------

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

// -- op (c): deterministic topological order ---------------------------------

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
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(id.as_str());
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

// -- the conformance test -----------------------------------------------------

#[test]
fn quilt_compat_conformance() {
    let g = golden();
    assert_eq!(g["contract"].as_str(), Some("quilt-compat/1"));
    assert_eq!(g["spec"]["edge_schema_v"].as_u64(), Some(1));
    println!("=== quilt-compat conformance (reference tier: rust) ===");
    println!(
        "contract: {}  golden: compat/golden.json",
        g["contract"].as_str().unwrap()
    );

    // (a) value cell read ---------------------------------------------------

    {
        let engine = fresh_engine(&g);
        let vectors = g["op_a_value_read"].as_array().expect("op_a vectors");
        for v in vectors {
            let cell = v["cell"].as_str().unwrap();
            let got = engine.get(cell, CallerContext::default()).expect("get");
            assert_close(
                &format!("(a) value read {cell}"),
                &got.data,
                &v["expect"],
                0.0,
            );
        }
        println!(
            "  [a] value cell read .............. PASS ({} vectors)",
            vectors.len()
        );
    }

    // (b) formula cell eval ---------------------------------------------------

    {
        let engine = fresh_engine(&g);
        let section = &g["op_b_formula_eval"];
        for v in section["initial"].as_array().unwrap() {
            let cell = v["cell"].as_str().unwrap();
            let got = engine.get(cell, CallerContext::default()).expect("get");
            assert_close(
                &format!("(b) formula eval {cell} (initial)"),
                &got.data,
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
            let got = engine.get(cell, CallerContext::default()).expect("get");
            assert_close(
                &format!("(b) formula eval {cell} (post-push)"),
                &got.data,
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

    // (c) reactive propagation order (topological) ----------------------------

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

        let engine = fresh_engine(&g);
        let declared = section["engine_dependency_graph_must_match"]
            .as_object()
            .unwrap();
        for (cell, deps) in declared {
            let cell_obj = engine.get_cell(cell).expect("cell exists");
            let mut got: Vec<String> = cell_obj
                .dependencies
                .iter()
                .map(|d| d.to_string())
                .collect();
            got.sort();
            let mut expected: Vec<String> = deps
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d.as_str().unwrap().to_string())
                .collect();
            expected.sort();
            assert_eq!(
                got, expected,
                "(c) engine dependency set for {cell} must match the golden graph"
            );
        }
        engine
            .push(mutate["cell"].as_str().unwrap(), mutate["value"].clone())
            .expect("push");
        let final_level = engine.get("bilge.level", CallerContext::default()).unwrap();
        assert_close(
            "(c) post-mutation read",
            &final_level.data,
            &json!(85.0),
            0.0,
        );
        println!("  [c] propagation order ........... PASS (topo order + engine graph agrees)");
    }

    // (d) edge record -----------------------------------------------------------

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
        println!(
            "  [d] edge record ................. PASS ({} vectors)",
            vectors.len()
        );
    }

    // (e) ledger chain-hash + reconcile ---------------------------------------

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

        for (entry, want) in ledger
            .entries()
            .iter()
            .zip(section["entries"].as_array().unwrap())
        {
            assert_eq!(
                entry.seq,
                want["seq"].as_u64().unwrap(),
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
        assert_eq!(
            report.open_inputs,
            want["open_inputs"].as_u64().unwrap() as usize
        );
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

    println!("RESULT: PASS — reference tier (rust) conforms to quilt-compat/1");
}

// -- regeneration helper -------------------------------------------------------

#[test]
#[ignore = "prints reference values for compat/golden.json; run with --ignored --nocapture"]
fn regenerate_golden_reference_values() {
    let g = golden();

    println!("-- op_d provenance hashes --");
    for v in g["op_d_edge"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let inputs: Vec<Value> = v["inputs"].as_array().unwrap().clone();
        println!(
            "{name}: provenance = \"{}\"   (preimage: {})",
            wire_provenance(&inputs),
            canonical_json(&Value::Array(inputs))
        );
    }

    println!("-- op_e chain seals --");
    let transcript = &g["op_e_chain"]["transcript"];
    let mut ledger = CellLedger::with_genesis(
        transcript["cell"].as_str().unwrap(),
        transcript["genesis"].clone(),
        transcript["genesis_ts"].as_f64().unwrap() as u64,
    );
    println!(
        "genesis root (empty-ledger chain_hash) = {}",
        ledger.chain_hash()
    );
    for rec in transcript["records"].as_array().unwrap() {
        let entry = ledger.record(
            rec["input"].clone(),
            rec["output"].clone(),
            rec["ts"].as_f64().unwrap() as u64,
        );
        println!(
            "entry {} {{\"seq\": {}, \"prev_hash\": \"{}\", \"hash\": \"{}\"}}",
            entry.seq, entry.seq, entry.prev_hash, entry.hash
        );
    }
    println!("chain_hash = \"{}\"", ledger.chain_hash());
    let report = ledger.reconcile();
    println!(
        "reconcile: total_surprise = {:?}, mean_surprise = {:?}, balanced = {}",
        report.total_surprise, report.mean_surprise, report.balanced
    );
}
