#!/usr/bin/env python3
"""test_ffi.py — golden-vector conformance for the ctypes binding.

Reproduces compat/golden.json ops (a)-(e) THROUGH the C ABI against
libquilt_cabi.so — the Python tier's proof for "one native core, N
bindings". All constants are read from compat/golden.json (never
hand-copied); semantics mirror compat/conformance_test.rs:

  (a) value cell read, tolerance 0.0 (exact JSON)
  (b) formula eval, initial + reactive post-push, tolerance 1e-12
  (c) implied by (b): the set() that propagates downstream
  (d) edge delta/imbalance/provenance (harness-side wire math, exactly
      like the reference tier; the scalar imbalance is additionally
      cross-checked against the native ledger's reconcile via the ABI)
  (e) ledger chain: seals + chain_hash BIT-FOR-BIT, reconcile fields

Run from anywhere:  python3 bindings/python-cabi/test_ffi.py
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import quilt_ffi as q  # noqa: E402

TOL = 1e-12

failures = 0
checks = 0


def check(cond: bool, msg: str) -> None:
    global failures, checks
    checks += 1
    print(f"  {'PASS' if cond else 'FAIL'} {msg}")
    if not cond:
        failures += 1


def canon(v) -> str:
    """Compact canonical JSON, golden spec form (sorted keys, shortest
    round-trip floats — Python repr matches ryu for these vectors)."""
    return json.dumps(v, separators=(",", ":"), sort_keys=True)


def close(got, want, tol: float) -> bool:
    if isinstance(got, bool) or isinstance(want, bool):
        return got is want
    if isinstance(got, (int, float)) and isinstance(want, (int, float)):
        return abs(got - want) <= tol
    if isinstance(got, list) and isinstance(want, list):
        return len(got) == len(want) and all(
            close(g, w, tol) for g, w in zip(got, want))
    return got == want


def get_json(engine, cell: str):
    return json.loads(engine.get(cell))


def main() -> int:
    golden_path = q.REPO_ROOT / "compat" / "golden.json"
    g = json.loads(golden_path.read_text(encoding="utf-8"))
    print("=== quilt python-cabi conformance (ctypes over libquilt_cabi.so) ===")
    print(f"library: {q.LIBRARY_PATH}")
    print(f"contract: {g['contract']}  golden: {golden_path.relative_to(q.REPO_ROOT)}")

    check(g["contract"] == "quilt-compat/1", "golden contract is quilt-compat/1")
    check(q.abi_version() == q.ABI_VERSION,
          f"ABI version matches quilt_cabi.h ({q.ABI_VERSION})")
    q.ledgers_reset()

    sheet = (q.REPO_ROOT / "crates" / "quilt-cabi" / "smoke" / "sheet.yaml"
             ).read_text(encoding="utf-8")

    # -- op (a): value cell read — exact JSON equality ----------------------

    with q.Engine() as e:
        e.load_sheet(sheet)
        for v in g["op_a_value_read"]:
            cell, want = v["cell"], v["expect"]
            raw = e.get(cell)
            ok = json.loads(raw) == want and raw == canon(want)
            if not ok:
                print(f"    get({cell}): got {raw!r}, want {canon(want)!r}")
            check(ok, f"(a) read {cell} == {canon(want)}")

        # -- op (b): formula eval, initial + reactive post-push -------------

        for v in g["op_b_formula_eval"]["initial"]:
            got, want = get_json(e, v["cell"]), v["expect"]
            check(close(got, want, TOL),
                  f"(b) initial {v['cell']} == {canon(want)}")
        push = g["op_b_formula_eval"]["after_push"]
        e.set(push["cell"], canon(push["value"]))
        for v in g["op_b_formula_eval"]["post"]:
            got, want = get_json(e, v["cell"]), v["expect"]
            check(close(got, want, TOL),
                  f"(b) post {v['cell']} == {canon(want)} (reactive after set)")

    # -- op (d): edge delta / imbalance / provenance --------------------------
    # Wire math is harness-side, mirroring conformance_test.rs (the ledger's
    # internal surprise metric is mean-of-abs; the wire imbalance for vectors
    # is Euclidean — both tiers compute op (d) outside the chain).

    for v in g["op_d_edge"]:
        name = v["name"]
        before, after, inputs = v["before"], v["after"], v["inputs"]
        exp = v["expect"]

        if isinstance(before, (int, float)) and not isinstance(before, bool) \
                and isinstance(after, (int, float)):
            delta = after - before
            imbalance = abs(after - before)
        elif isinstance(before, list) and isinstance(after, list) \
                and len(before) == len(after):
            ds = [a - b for b, a in zip(before, after)]
            delta = ds
            imbalance = sum(d * d for d in ds) ** 0.5
        else:
            delta = None
            imbalance = None

        provenance = hashlib.sha256(canon(inputs).encode("utf-8")).hexdigest()

        check(close(delta, exp["delta"], TOL), f"(d) {name} delta")
        check(close(imbalance, exp["imbalance"], TOL), f"(d) {name} imbalance")
        check(provenance == exp["provenance"], f"(d) {name} provenance")

        if exp["imbalance"] is None:
            # null-prior edge through the ABI: record with NO genesis so the
            # first edge is a null-prior edge — no surprise is claimed.
            q.ledgers_reset()
            seal = q.ledger_record(v["cell"], canon(after), canon(after),
                                   int(v["ts"]))
            rep = json.loads(q.ledger_reconcile(v["cell"]))
            check(len(seal) == 64 and rep["entries"] == 1
                  and rep["total_surprise"] == 0.0 and rep["balanced"],
                  f"(d) {name} null-prior edge: no surprise via ABI")
        elif not isinstance(before, list):
            # Scalar edge through the ABI: genesis commits `before`, the
            # record commits `after`; reconcile's total_surprise IS the
            # wire imbalance under the persistence prior.
            q.ledgers_reset()
            q.ledger_init(v["cell"], canon(before), int(v["ts"]) - 1)
            q.ledger_record(v["cell"], canon(after), canon(after),
                            int(v["ts"]))
            rep = json.loads(q.ledger_reconcile(v["cell"]))
            check(close(rep["total_surprise"], exp["imbalance"], TOL),
                  f"(d) {name} imbalance cross-checked via ABI reconcile")

    # -- op (e): ledger chain — seals BIT-FOR-BIT, reconcile -----------------

    q.ledgers_reset()
    tr = g["op_e_chain"]["transcript"]
    cell = tr["cell"]
    q.ledger_init(cell, canon(tr["genesis"]), int(tr["genesis_ts"]))

    root = q.ledger_chain_hash(cell)
    check(root == g["op_e_chain"]["entries"][0]["prev_hash"],
          "(e) genesis root pinned (entry 1 prev-link)")

    try:
        q.ledger_init(cell, canon(tr["genesis"]), int(tr["genesis_ts"]))
        double_init_rejected = False
    except q.QuiltError:
        double_init_rejected = True
    check(double_init_rejected, "(e) double ledger_init is rejected")

    for rec, want in zip(tr["records"], g["op_e_chain"]["entries"]):
        seal = q.ledger_record(cell, canon(rec["input"]),
                               canon(rec["output"]), int(rec["ts"]))
        check(seal == want["hash"],
              f"(e) seal {want['seq']} bit-for-bit")

    check(q.ledger_verify(cell) == 1, "(e) chain verifies (1)")
    check(q.ledger_verify("no.such.cell") == -1, "(e) unknown ledger -> -1")

    head = q.ledger_chain_hash(cell)
    chain_hash = g["op_e_chain"]["chain_hash"]
    check(head == chain_hash, "(e) chain_hash == golden head (bit-for-bit)")

    rep = json.loads(q.ledger_reconcile(cell))
    wr = g["op_e_chain"]["reconcile"]
    rec_ok = (rep.get("cell_id") == cell
              and rep.get("entries") == wr["entries"]
              and rep.get("open_inputs") == wr["open_inputs"]
              and rep.get("matched_pairs") == wr["matched_pairs"]
              and rep.get("chain_intact") is wr["chain_intact"]
              and rep.get("continuity_intact") is wr["continuity_intact"]
              and rep.get("balanced") is wr["balanced"]
              and close(rep.get("total_surprise"), wr["total_surprise"], TOL)
              and close(rep.get("mean_surprise"), wr["mean_surprise"], TOL))
    if not rec_ok:
        print(f"    reconcile got: {rep}")
    check(rec_ok, "(e) reconcile matches golden "
          f"(balanced, total {wr['total_surprise']}, mean {wr['mean_surprise']})")

    # -- error discipline ------------------------------------------------------

    with q.Engine() as e:
        e.load_sheet(sheet)
        try:
            e.get("no.such.cell")
            unknown_ok = False
        except q.QuiltError as ex:
            unknown_ok = len(str(ex)) > 0 and len(q.last_error()) > 0
        check(unknown_ok, "unknown cell errors with a last_error detail")
        check(q.get_null_engine("x") is None, "NULL engine tolerated")
        try:
            q.ledger_record("x.cell", "{not json", "1", 1)
            bad_json_ok = False
        except q.QuiltError:
            bad_json_ok = True
        check(bad_json_ok, "bad JSON input to ledger_record errors")
        q.string_free(None)  # must be a no-op
        check(True, "string_free(NULL) is a no-op")

    q.ledgers_reset()
    result = "PASS" if failures == 0 else "FAIL"
    print(f"RESULT: {result} — {checks} checks, {failures} failures")
    print(f"chain_hash: {head}")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
