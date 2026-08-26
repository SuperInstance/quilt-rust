//! # quilt-cabi smoke test — the ABI against `compat/golden.json`
//!
//! Exercises every exported `extern "C"` symbol over the golden vectors
//! for the ops this tier owns: (a) value cell read, (b) formula eval +
//! reactive propagation, (e) ledger chain seals bit-for-bit + verify +
//! reconcile, plus the error discipline. Expected values are parsed from
//! `compat/golden.json` at runtime — no hand-copied vectors — exactly like
//! the reference harness (`compat/conformance_test.rs`). True
//! cross-language linkage against the shipped `libquilt_cabi.so` is proven
//! by the C harness (`smoke/smoke.c`, run via `smoke/run.sh`).
//!
//! Run: `cargo test -p quilt-cabi --test abi_smoke -- --nocapture`

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr::null_mut;

use quilt_cabi::{
    quilt_abi_version, quilt_engine_free, quilt_engine_get, quilt_engine_load_sheet,
    quilt_engine_new, quilt_engine_set, quilt_last_error, quilt_ledger_chain_hash,
    quilt_ledger_init, quilt_ledger_reconcile, quilt_ledger_record, quilt_ledger_verify,
    quilt_ledgers_reset, quilt_string_free, QuiltEngine,
};
use serde_json::Value;

/// The contract, straight from the source of truth.
fn golden() -> Value {
    serde_json::from_str(include_str!("../../../compat/golden.json")).expect("golden.json parses")
}

/// The golden sheet in canonical YAML form (smoke/sheet.yaml is generated
/// from the same JSON — see smoke/gen-sheet.py).
const SHEET_YAML: &str = include_str!("../smoke/sheet.yaml");

/// Take ownership of a library-allocated string, return its contents.
fn take(ptr: *mut c_char) -> String {
    assert!(
        !ptr.is_null(),
        "expected a string, got NULL (last_error: {})",
        last_error()
    );
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    quilt_string_free(ptr);
    s
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(quilt_last_error()) }
        .to_string_lossy()
        .into_owned()
}

fn c(s: &str) -> CString {
    CString::new(s).expect("no interior NUL in test input")
}

fn get(engine: *mut QuiltEngine, cell: &str) -> String {
    take(quilt_engine_get(engine, c(cell).as_ptr()))
}

fn set(engine: *mut QuiltEngine, cell: &str, json: &str) {
    assert_eq!(
        quilt_engine_set(engine, c(cell).as_ptr(), c(json).as_ptr()),
        0,
        "set {cell} failed: {}",
        last_error()
    );
}

/// Exact-JSON comparison, rendered the way serde_json renders the golden
/// value (floats keep their marker: 80.0, never 80).
fn json_text(v: &Value) -> String {
    serde_json::to_string(v).expect("golden value serializes")
}

#[test]
fn quilt_cabi_smoke_against_golden() {
    let g = golden();
    assert_eq!(g["contract"].as_str(), Some("quilt-compat/1"));
    println!("=== quilt-cabi smoke (C ABI vs compat/golden.json) ===");

    assert!(quilt_ledgers_reset() == 0);
    assert_eq!(quilt_abi_version(), 1, "ABI version pin");

    // -- engine + golden sheet ---------------------------------------------

    let engine = quilt_engine_new();
    assert!(!engine.is_null(), "engine_new: {}", last_error());
    assert_eq!(
        quilt_engine_load_sheet(engine, c(SHEET_YAML).as_ptr()),
        0,
        "load_sheet: {}",
        last_error()
    );
    println!(
        "  [.] engine loaded golden sheet ({})",
        g["sheet"]["id"].as_str().unwrap()
    );

    // -- op (a): value cell read — exact JSON equality ----------------------

    for v in g["op_a_value_read"].as_array().unwrap() {
        let cell = v["cell"].as_str().unwrap();
        assert_eq!(
            get(engine, cell),
            json_text(&v["expect"]),
            "(a) read {cell}"
        );
    }
    println!("  [a] value cell read .............. PASS (3 vectors)");

    // -- op (b): formula eval, initial + post-push ---------------------------

    let op_b = &g["op_b_formula_eval"];
    for v in op_b["initial"].as_array().unwrap() {
        let cell = v["cell"].as_str().unwrap();
        assert_eq!(
            get(engine, cell),
            json_text(&v["expect"]),
            "(b) initial {cell}"
        );
    }
    let push = &op_b["after_push"];
    set(
        engine,
        push["cell"].as_str().unwrap(),
        &json_text(&push["value"]),
    );
    for v in op_b["post"].as_array().unwrap() {
        let cell = v["cell"].as_str().unwrap();
        assert_eq!(
            get(engine, cell),
            json_text(&v["expect"]),
            "(b) post {cell}"
        );
    }
    println!("  [b] formula cell eval ........... PASS (5 vectors)");

    // -- op (e): ledger record / verify / reconcile, seals bit-for-bit -------

    let op_e = &g["op_e_chain"];
    let transcript = &op_e["transcript"];
    let cell = c(transcript["cell"].as_str().unwrap());
    let genesis = json_text(&transcript["genesis"]);
    let genesis_ts = transcript["genesis_ts"].as_f64().unwrap() as u64;
    assert_eq!(
        quilt_ledger_init(cell.as_ptr(), c(&genesis).as_ptr(), genesis_ts),
        0,
        "ledger_init: {}",
        last_error()
    );
    // Retro-init must fail: a genesis cannot be retrofitted.
    assert_eq!(
        quilt_ledger_init(cell.as_ptr(), c(&genesis).as_ptr(), genesis_ts),
        -1
    );

    // The empty ledger's chain hash is the genesis commit — the golden
    // root that entry 1's prev-link seals against.
    let root = take(quilt_ledger_chain_hash(cell.as_ptr()));
    assert_eq!(
        root,
        op_e["entries"][0]["prev_hash"].as_str().unwrap(),
        "genesis root must be pinned"
    );

    for (rec, want) in transcript["records"]
        .as_array()
        .unwrap()
        .iter()
        .zip(op_e["entries"].as_array().unwrap())
    {
        let seal = take(quilt_ledger_record(
            cell.as_ptr(),
            c(&json_text(&rec["input"])).as_ptr(),
            c(&json_text(&rec["output"])).as_ptr(),
            rec["ts"].as_f64().unwrap() as u64,
        ));
        assert_eq!(
            seal,
            want["hash"].as_str().unwrap(),
            "(e) seal must be bit-for-bit"
        );
    }

    assert_eq!(quilt_ledger_verify(cell.as_ptr()), 1, "chain must verify");
    let head = take(quilt_ledger_chain_hash(cell.as_ptr()));
    assert_eq!(
        head,
        op_e["chain_hash"].as_str().unwrap(),
        "chain_hash must equal the golden head"
    );

    let report: Value = serde_json::from_str(&take(quilt_ledger_reconcile(cell.as_ptr())))
        .expect("reconcile returns JSON");
    let want = &op_e["reconcile"];
    for field in [
        "entries",
        "open_inputs",
        "matched_pairs",
        "chain_intact",
        "continuity_intact",
        "balanced",
    ] {
        assert_eq!(report[field], want[field], "(e) reconcile.{field}");
    }
    for field in ["total_surprise", "mean_surprise"] {
        let (got, exp) = (
            report[field].as_f64().unwrap(),
            want[field].as_f64().unwrap(),
        );
        assert!(
            (got - exp).abs() <= 1e-12,
            "(e) reconcile.{field}: got {got}, want {exp}"
        );
    }
    println!("  [e] chain + reconcile ........... PASS (3 seals bit-for-bit, books balanced)");

    // -- error discipline ----------------------------------------------------

    let missing = quilt_engine_get(engine, c("no.such.cell").as_ptr());
    assert!(
        missing.is_null(),
        "unknown cell must return NULL, not a string"
    );
    assert!(
        !last_error().is_empty(),
        "last_error must explain the failure"
    );
    assert_eq!(quilt_ledger_verify(c("no.such.ledger").as_ptr()), -1);
    let bad = quilt_ledger_record(cell.as_ptr(), c("{not json").as_ptr(), c("1").as_ptr(), 1);
    assert!(bad.is_null(), "bad JSON must return NULL");
    assert!(!last_error().is_empty());
    // NULL tolerance: no crash, just an error.
    assert!(quilt_engine_get(null_mut(), c("x").as_ptr()).is_null());
    assert_eq!(quilt_engine_load_sheet(null_mut(), std::ptr::null()), -1);
    println!(
        "  [x] error discipline ............ PASS (NULL returns + last_error + NULL-tolerance)"
    );

    quilt_engine_free(engine);
    println!("RESULT: PASS — quilt-cabi conforms to golden.json ops (a), (b), (e)");
}

#[test]
fn cstr_pointer_null_handling_is_documented_contract() {
    // quilt_string_free(NULL) and quilt_engine_free(NULL) must be no-ops.
    quilt_string_free(null_mut());
    quilt_engine_free(null_mut());
    assert_eq!(quilt_ledgers_reset(), 0);
}
