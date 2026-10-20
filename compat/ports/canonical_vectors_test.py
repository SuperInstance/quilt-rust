#!/usr/bin/env python3
"""THE CANONICAL FORM, CHECKED AGAINST THE SHARED VECTORS.

`compat/canonical-vectors.json` is the shared artefact behind the claim in
`docs/cell-ledger.md` that "every quilt tier (Rust, TS, Go) produces
byte-identical preimages from the same data".

The Rust and Python tiers already share `compat/golden.json` and are
genuinely differential-tested. The Go tier had its own golden -- a
different spec, a different sheet, a different chain envelope -- so for Go
the claim was ASSERTED, not checked. And the TypeScript tier the spec
names does not exist.

**THE CANONICAL FORM IS THE PART ALL THE TIERS MUST AGREE ON, AND IT IS
THE PART THAT IS SMALL AND CHECKABLE.** So it gets its own vector file, and
every tier reads it.

A port conforms by reproducing every `canonical` string in it. One check,
no engine, no sheet, no chain.

Run:  python3 compat/ports/canonical_vectors_test.py
"""
import hashlib
import importlib.util
import json
import pathlib
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
VECTORS = REPO_ROOT / "compat" / "canonical-vectors.json"
LEDGER = REPO_ROOT / "bindings" / "python" / "quilt" / "ledger.py"


def load_ledger():
    spec = importlib.util.spec_from_file_location("quilt_ledger", LEDGER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    if not VECTORS.is_file():
        print(f"FAIL: {VECTORS} not found")
        return 1
    doc = json.loads(VECTORS.read_text(encoding="utf-8"))
    cases = doc["cases"]
    assert doc["case_count"] == len(cases), "the vector file's count is wrong"

    ledger = load_ledger()
    failures = []
    for c in cases:
        got = ledger.canonical_json(c["value"])
        if got != c["canonical"]:
            failures.append(f"{c['name']}: got {got!r} want {c['canonical']!r}")
            continue
        want_sha = c["sha256"]
        got_sha = hashlib.sha256(got.encode("utf-8")).hexdigest()
        if got_sha != want_sha:
            failures.append(f"{c['name']}: sha256 {got_sha} != {want_sha}")

    print(f"canonical vectors: {len(cases)} cases, spec {doc['spec']}")
    for c in cases:
        print(f"  ok  {c['name']}")
    if failures:
        print()
        for f in failures:
            print("  FAIL: " + f)
        print(f"\nFAIL: {len(failures)} canonical vector(s) do not match.")
        return 1
    print(f"\nPASS: {len(cases)} canonical vectors match.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
