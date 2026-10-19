// THE CANONICAL FORM, CHECKED AGAINST THE SHARED VECTORS.
//
// `compat/canonical-vectors.json` is the shared artefact behind the claim in
// `docs/cell-ledger.md` that "every quilt tier (Rust, TS, Go) produces
// byte-identical preimages from the same data".
//
// The Rust and Python tiers already share `compat/golden.json` and are
// genuinely differential-tested. The Go tier had its own golden -- a
// different spec, a different sheet, a different chain envelope -- so for
// Go the claim was ASSERTED, not checked. And the TypeScript tier the spec
// names does not exist.
//
// **THE CANONICAL FORM IS THE PART ALL THE TIERS MUST AGREE ON, AND IT IS
// THE PART THAT IS SMALL AND CHECKABLE.** So it gets its own vector file, and
// every tier reads it.
//
// A port conforms by reproducing every `canonical` string in it.
//
// NOTE ON serde_json: `serde_json::Value` distinguishes an integer from a
// float ONLY while the number is still a `Number`. `from_str` on `40.0`
// gives a Number that serialises back as `40.0` (serde_json keeps the
// float marker), and `40` stays `40` -- which is exactly the distinction
// the vectors pin. If a future serde_json change makes `40.0` serialise
// as `40`, THIS TEST FAILS, and that is the point: the hazard is named in
// docs/cell-ledger.md and it is now pinned by a machine.

use std::fs;

use quilt_core::ledger::canonical_json;
use serde_json::Value;

/// The vectors live next to this file's crate root, two levels up.
const VECTORS: &str = "../../compat/canonical-vectors.json";

#[test]
fn the_canonical_form_matches_the_shared_vectors() {
    let raw = fs::read_to_string(VECTORS)
        .unwrap_or_else(|e| panic!("cannot read {VECTORS}: {e}"));
    let doc: Value = serde_json::from_str(&raw).expect("the vector file is not valid JSON");

    let cases = doc["cases"].as_array().expect("cases is not an array");
    assert_eq!(
        cases.len(),
        doc["case_count"].as_u64().expect("case_count is not a number") as usize,
        "the vector file's recorded count does not match its contents"
    );
    assert!(!cases.is_empty(), "the vector file is empty");

    let mut failures: Vec<String> = Vec::new();
    for c in cases {
        let name = c["name"].as_str().unwrap_or("<unnamed>");
        let value = &c["value"];
        let want = c["canonical"].as_str().expect("a case has no canonical string");
        let got = canonical_json(value);
        if got != want {
            failures.push(format!("{name}: got {got:?} want {want:?}"));
            continue;
        }
        // and the recorded digest must be the digest of the canonical bytes.
        // The crate's own inline SHA-256 (packages/core/src/ledger.rs, `pub
        // mod sha256`) is the same implementation the chain uses, so this
        // is not a second opinion -- it is the same one. That is deliberate:
        // the vectors pin the CANONICAL FORM, and the digest is a receipt
        // for the vector file, not a second hashing implementation.
        let want_sha = c["sha256"].as_str().expect("a case has no sha256");
        let got_sha = quilt_core::ledger::sha256::hex(got.as_bytes());
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
