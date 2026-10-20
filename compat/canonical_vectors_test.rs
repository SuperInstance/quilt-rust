// THE CANONICAL FORM, CHECKED AGAINST THE SHARED VECTORS.
//
// `compat/canonical-vectors.json` is the shared artefact behind the claim in
// `docs/cell-ledger.md` that "every quilt tier (Rust, TS, Go) produces
// byte-identical preimages from the same data".
//
// The Rust and Python tiers already share `compat/golden.json` and are
// genuinely differential-tested. The Go tier had its own golden -- a
// different spec, a different sheet, a different chain envelope -- so for Go
// the claim was ASSERTED, not checked. And the TypeScript tier the spec
// names does not exist.
//
// **THE CANONICAL FORM IS THE PART ALL THE TIERS MUST AGREE ON, AND IT IS
// THE PART THAT IS SMALL AND CHECKABLE.** So it gets its own vector file, and
// every tier reads it.
//
// A port conforms by reproducing every `canonical` string in it. One check,
// no engine, no sheet, no chain.
//
// Run:
//   cargo test -p quilt-core --test quilt_canonical_vectors -- --nocapture

use quilt_core::ledger::{canonical_json, sha256};
use serde_json::Value;

/// The vectors, COMPILED IN -- the same idiom as the sibling conformance
/// test's `include_str!("golden.json")`. A runtime path is CWD-dependent
/// (cargo runs a test binary from the package root, sometimes from the
/// workspace root), and a check that cannot find its own fixture is a
/// check that fails for the wrong reason. `include_str!` is resolved
/// relative to THIS FILE, at compile time, and it fails the BUILD loudly
/// if the fixture is missing.
const VECTORS_JSON: &str = include_str!("canonical-vectors.json");

#[test]
fn the_canonical_form_matches_the_shared_vectors() {
    let doc: Value = match serde_json::from_str(VECTORS_JSON) {
        Ok(v) => v,
        Err(e) => panic!("the vector file is not valid JSON: {e}"),
    };

    let cases = match doc["cases"].as_array() {
        Some(c) => c,
        None => panic!("cases is not an array"),
    };
    let recorded = match doc["case_count"].as_u64() {
        Some(n) => n as usize,
        None => panic!("case_count is not a number"),
    };
    assert_eq!(
        cases.len(),
        recorded,
        "the vector file's recorded count does not match its contents"
    );
    assert!(!cases.is_empty(), "the vector file is empty");

    let mut failures: Vec<String> = Vec::new();
    for c in cases {
        let name = c["name"].as_str().unwrap_or("<unnamed>").to_string();
        let want = c["canonical"].as_str().unwrap_or("").to_string();
        let got = canonical_json(&c["value"]);
        if got != want {
            failures.push(format!("{name}: got {got:?} want {want:?}"));
            continue;
        }
        // The recorded digest is a receipt for the vector file. The crate's
        // own inline SHA-256 is the same implementation the chain uses, so
        // this is deliberately not a second opinion: the vectors pin the
        // CANONICAL FORM, and the digest pins the file.
        let want_sha = c["sha256"].as_str().unwrap_or("").to_string();
        let got_sha = sha256::hex(got.as_bytes());
        if got_sha != want_sha {
            failures.push(format!("{name}: sha256 {got_sha} != {want_sha}"));
        }
    }

    if !failures.is_empty() {
        panic!(
            "the canonical form does not match the shared vectors:\n  {}",
            failures.join("\n  ")
        );
    }
    eprintln!("canonical vectors: {} cases match", cases.len());
}
