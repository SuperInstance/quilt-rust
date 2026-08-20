//! # quilt-core-wasm
//!
//! The WASM-compatible sync core — the web tier of quilt.
//!
//! ## Role in the system
//!
//! `packages/core` (quilt-core) cannot compile for `wasm32-unknown-unknown`:
//! its dependency set includes tokio (`full` pulls `net` → `mio`, which
//! hard-fails on this target), rhai (→ `ahash` → `getrandom` 0.3, which
//! `compile_error!`s without JS glue), and uuid v4 (same getrandom wall).
//! See `docs/wasm-target.md` for the full audit.
//!
//! This crate exposes the part of the core that *is* portable:
//!
//! - **Value / formula evaluation** — the reactive sync subset of the
//!   engine (`value`, `formula`, `sensor` cells) with a dependency-free
//!   expression evaluator that covers the arithmetic/comparison/clamp
//!   surface of the golden contract (rhai stays on the native tier).
//! - **The cell ledger** — record / reconcile / replay / chain, compiled
//!   **from the canonical `packages/core/src/ledger.rs` itself** via a
//!   `#[path]` include, so the two tiers can never drift: the chain
//!   hashes are bit-for-bit identical because it is literally the same
//!   file.
//!
//! ## Depends on
//!
//! - `serde`, `serde_json` — wasm32-clean, pure Rust, and pinned by the
//!   canonical-JSON contract (ryū shortest-round-trip floats).
//! - Nothing else. No clock (callers pass timestamps), no rng, no net,
//!   no threads. The ledger's SHA-256 is inline in ledger.rs.
//!
//! ## Used by
//!
//! - The web tier: `wasm-bindgen` / JS glue (out of scope here) wraps
//!   this crate for the browser. Compiling to
//!   `wasm32-unknown-unknown` is the deliverable; the conformance test
//!   (`tests/wasm_conformance.rs`) proves the tier conforms to
//!   `compat/golden.json` and to the native reference tier.
//!
//! ## Status
//!
//! - ✅ Ledger: full parity, single-sourced from `packages/core`.
//! - ✅ Sync engine: value / formula / sensor cells, auto-detected
//!   dependencies, deterministic evaluation.
//! - ❌ `api`, `program`, `router`, `io`, `listener` cells — these need
//!   reqwest / rhai / the full engine and stay on the native tier.

// `deny` (not `forbid`) so the two C-ABI anchors below can carry
// `#[no_mangle]`, which rustc classifies as an unsafe attribute. They
// contain no unsafe code — no unsafe block exists anywhere in this
// crate; the deny below still fires on any that appears.
#![deny(unsafe_code)]

use serde_json::Value;

pub mod engine;
pub mod error;
pub mod formula;
pub mod types;

// The canonical ledger, single-sourced. `packages/core/src/ledger.rs` is
// deliberately pure data + serde (no clocks, no I/O, no async) so that it
// can be compiled here unmodified. It only references `crate::error`
// (`Error::other`) and `crate::types` (`CellId`), both provided by the
// shims below. Any change to the ledger lands in packages/core once and
// both tiers pick it up.
#[path = "../../../packages/core/src/ledger.rs"]
pub mod ledger;

pub use crate::engine::WasmEngine;
pub use crate::error::{Error, Result};
pub use crate::formula::Formula;
pub use crate::types::{CellDef, CellId, CellKind, Sheet};

// ---------------------------------------------------------------------------
// C-ABI anchors (not bindgen glue)
// ---------------------------------------------------------------------------

/// ABI anchor: any wasm runtime can confirm the module loaded and which
/// ABI it speaks. The real JS surface is future wasm-bindgen work (out
/// of scope here); these exports exist so the `.wasm` artifact is a
/// self-contained, verifiable product rather than a stripped husk.
#[allow(unsafe_code)] // attribute-only: `no_mangle` is an "unsafe attribute"; no unsafe block here
#[no_mangle]
pub extern "C" fn quilt_core_wasm_abi_version() -> u32 {
    1
}

/// Self-check: run the embedded golden contract (`compat/golden.json`,
/// single-sourced via `include_str!`) through *this build's* engine and
/// ledger — value reads, formula eval pre/post push, edge provenance
/// seals, chain seals, reconciliation — and return `1` when every op
/// conforms, `0` otherwise. Lets any wasm runtime smoke-test the
/// artifact with zero JS glue:
///
/// ```text
/// wasmtime run --invoke quilt_core_wasm_golden_check quilt_core_wasm.wasm
/// ```
#[allow(unsafe_code)] // attribute-only: `no_mangle` is an "unsafe attribute"; no unsafe block here
#[no_mangle]
pub extern "C" fn quilt_core_wasm_golden_check() -> u32 {
    u32::from(run_golden_checks())
}

fn run_golden_checks() -> bool {
    let golden: Value = match serde_json::from_str(include_str!(
        "../../../compat/golden.json"
    )) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let close = |got: &Value, want: &Value, tol: f64| -> bool {
        match (got.as_f64(), want.as_f64()) {
            (Some(g), Some(w)) => (g - w).abs() <= tol,
            _ => got == want,
        }
    };

    // (a) value reads + (b) formula eval, initial and post-push.
    let mut engine = match WasmEngine::from_sheet_json(&golden["sheet"]) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let check_vecs = |engine: &WasmEngine, vecs: &Value| -> bool {
        vecs.as_array()
            .map(|vs| {
                vs.iter().all(|v| {
                    let cell = v["cell"].as_str().unwrap_or_default();
                    matches!(engine.get(cell), Ok(got) if close(&got, &v["expect"], 1e-12))
                })
            })
            .unwrap_or(false)
    };
    if !check_vecs(&engine, &golden["op_a_value_read"]) {
        return false;
    }
    if !check_vecs(&engine, &golden["op_b_formula_eval"]["initial"]) {
        return false;
    }
    let push = &golden["op_b_formula_eval"]["after_push"];
    if engine
        .push(push["cell"].as_str().unwrap_or_default(), push["value"].clone())
        .is_err()
    {
        return false;
    }
    if !check_vecs(&engine, &golden["op_b_formula_eval"]["post"]) {
        return false;
    }

    // (d) provenance seals for every edge vector.
    for v in golden["op_d_edge"].as_array().into_iter().flatten() {
        let inputs = match v["inputs"].as_array() {
            Some(i) => i.clone(),
            None => return false,
        };
        let canonical = ledger::canonical_json(&Value::Array(inputs));
        let prov = ledger::sha256::hex(canonical.as_bytes());
        if Some(prov.as_str()) != v["expect"]["provenance"].as_str() {
            return false;
        }
    }

    // (e) chain seals (seq / prev_hash / hash, bit-for-bit) + reconcile.
    let section = &golden["op_e_chain"];
    let t = &section["transcript"];
    let mut ledger = ledger::CellLedger::with_genesis(
        t["cell"].as_str().unwrap_or_default(),
        t["genesis"].clone(),
        t["genesis_ts"].as_f64().unwrap_or_default() as u64,
    );
    for rec in t["records"].as_array().into_iter().flatten() {
        ledger.record(
            rec["input"].clone(),
            rec["output"].clone(),
            rec["ts"].as_f64().unwrap_or_default() as u64,
        );
    }
    let entries: &[Value] = section["entries"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    if ledger.entries().len() != entries.len() {
        return false;
    }
    for (entry, want) in ledger.entries().iter().zip(entries) {
        if Some(entry.seq) != want["seq"].as_u64()
            || entry.prev_hash != want["prev_hash"].as_str().unwrap_or_default()
            || entry.hash != want["hash"].as_str().unwrap_or_default()
        {
            return false;
        }
    }
    if ledger.chain_hash() != section["chain_hash"].as_str().unwrap_or_default() {
        return false;
    }
    let report = ledger.reconcile();
    let want = &section["reconcile"];
    report.balanced
        && Some(report.entries as u64) == want["entries"].as_u64()
        && Some(report.open_inputs as u64) == want["open_inputs"].as_u64()
        && Some(report.matched_pairs as u64) == want["matched_pairs"].as_u64()
        && close(&serde_json::json!(report.total_surprise), &want["total_surprise"], 1e-12)
}
