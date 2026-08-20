#!/usr/bin/env python3
"""test_compat.py — quilt-py conformance harness (the Python tier's proof).

Mirrors `compat/conformance_test.rs` (the reference harness) against
the normative `compat/golden.json`, at the Python tier's declared
conformance class (quilt-compat-contract.md §4):

    (a) value read      exact
    (b) formula eval    1e-12
    (c) propagation     exact (ordered list)
    (d) edge            1e-9   (dyadic golden vectors hold exactly)
    (e) chain hashes    bit-for-bit
    (e') reconcile      1e-6   (holds exactly here)

Run:  python3 tests/test_compat.py     (from bindings/python)
Exit: 0 = PASS, 1 = FAIL. Prints PASS/FAIL per op + the golden numbers.
"""

from __future__ import annotations

import json
import math
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
PKG_ROOT = HERE.parent                  # bindings/python
REPO_ROOT = PKG_ROOT.parents[1]         # quilt-rust
sys.path.insert(0, str(PKG_ROOT))

from quilt import (  # noqa: E402
    QuiltEngine,
    canonical_json,
    sha256_hex,
    value_distance,
    wire_delta,
    wire_imbalance,
    wire_provenance,
    wire_edge,
    CellLedger,
    parse_sheet,
    sheet_from_dict,
    detect_dependencies,
)

GOLDEN_PATH = REPO_ROOT / "compat" / "golden.json"
if not GOLDEN_PATH.is_file():
    raise SystemExit(f"golden.json not found at {GOLDEN_PATH}")
GOLDEN = json.loads(GOLDEN_PATH.read_text())

# -- the Python tier's declared conformance class (contract §4) ---------------
TOL_FORMULA = 1e-12
TOL_EDGE = 1e-9
TOL_RECONCILE = 1e-6


# ---------------------------------------------------------------- helpers


def fresh_engine() -> QuiltEngine:
    """Engine built from the golden sheet (same path the Rust harness
    takes: SheetDef straight from the golden JSON)."""
    return QuiltEngine(sheet_from_dict(GOLDEN["sheet"]))


def assert_close(testcase, what, got, want, tol):
    """Numeric closeness with exact fallback — same shape as the Rust
    harness's assert_close."""
    if isinstance(got, (int, float)) and not isinstance(got, bool) and \
       isinstance(want, (int, float)) and not isinstance(want, bool):
        testcase.assertTrue(
            abs(float(got) - float(want)) <= tol,
            f"{what}: got {got}, want {want} (tol {tol})",
        )
    elif isinstance(got, list) and isinstance(want, list):
        testcase.assertEqual(len(got), len(want), f"{what}: length mismatch {got} vs {want}")
        for i, (gv, wv) in enumerate(zip(got, want)):
            assert_close(testcase, f"{what}[{i}]", gv, wv, tol)
    else:
        testcase.assertEqual(got, want, f"{what}: got {got!r}, want {want!r}")


def assert_sha256_hex(testcase, what, got, want):
    testcase.assertEqual(len(got), 64, f"{what}: not a sha256 hex string: {got}")
    testcase.assertTrue(
        all(c in "0123456789abcdef" for c in got),
        f"{what}: must be lowercase hex: {got}",
    )
    testcase.assertEqual(got, want, f"{what}: must be bit-for-bit")


# ---------------------------------------------------------------- the five core ops


class OpAValueRead(unittest.TestCase):
    def test_value_reads_exact(self):
        engine = fresh_engine()
        for v in GOLDEN["op_a_value_read"]:
            got = engine.get(v["cell"])
            self.assertEqual(got.status, "ready")
            assert_close(self, f"(a) value read {v['cell']}", got.data, v["expect"], 0.0)


class OpBFormulaEval(unittest.TestCase):
    def test_initial_then_post_push(self):
        engine = fresh_engine()
        section = GOLDEN["op_b_formula_eval"]
        for v in section["initial"]:
            got = engine.get(v["cell"])
            assert_close(self, f"(b) formula {v['cell']} (initial)", got.data, v["expect"], TOL_FORMULA)

        push = section["after_push"]
        engine.push(push["cell"], push["value"])
        for v in section["post"]:
            got = engine.get(v["cell"])
            assert_close(self, f"(b) formula {v['cell']} (post-push)", got.data, v["expect"], TOL_FORMULA)


class OpCPropagation(unittest.TestCase):
    def test_topological_order_and_engine_graph(self):
        graph = {k: list(v) for k, v in GOLDEN["graph"].items()}
        section = GOLDEN["op_c_propagation"]
        root = section["mutate"]["cell"]

        engine = fresh_engine()
        order = engine.propagation_order(root)
        self.assertEqual(
            order,
            section["expected_order"],
            "(c) propagation order must be the deterministic topo order",
        )

        # The engine's live dependency sets must equal the golden graph.
        self.assertEqual(
            {cid: engine.dependencies(cid) for cid in graph},
            {cid: sorted(deps) for cid, deps in graph.items()},
            "(c) engine dependency graph must match the golden graph",
        )
        for cell, deps in section["engine_dependency_graph_must_match"].items():
            self.assertEqual(engine.dependencies(cell), sorted(deps), f"(c) deps of {cell}")

        engine.push(root, section["mutate"]["value"])
        got = engine.get("bilge.level")
        assert_close(self, "(c) post-mutation read", got.data, 85.0, 0.0)


class OpDEdge(unittest.TestCase):
    def test_wire_edges(self):
        for v in GOLDEN["op_d_edge"]:
            name = v["name"]
            delta = wire_delta(v["before"], v["after"])
            assert_close(self, f"(d) edge {name} delta", delta, v["expect"]["delta"], TOL_EDGE)
            imbalance = wire_imbalance(v["before"], v["after"])
            assert_close(self, f"(d) edge {name} imbalance", imbalance, v["expect"]["imbalance"], TOL_EDGE)
            prov = wire_provenance(v["inputs"])
            assert_sha256_hex(self, f"(d) edge {name} provenance", prov, v["expect"]["provenance"])

    def test_full_wire_edge_record_shape(self):
        edge = wire_edge("x", 1000.0, 40.0, 85.0, [85.0], chain="ab" * 32, seq=1)
        self.assertEqual(
            edge,
            {
                "v": 1, "cell": "x", "ts": 1000.0, "before": 40.0, "after": 85.0,
                "delta": 45.0, "imbalance": 45.0,
                "provenance": sha256_hex(b"[85.0]"),
                "chain": "ab" * 32, "seq": 1,
            },
        )
        # Non-numeric edge: recorded as having happened, not faked.
        self.assertIsNone(wire_delta("idle", "running"))
        self.assertIsNone(wire_delta(None, 7.0))


class OpEChain(unittest.TestCase):
    def test_transcript_seals_and_reconcile(self):
        section = GOLDEN["op_e_chain"]
        transcript = section["transcript"]
        cell = transcript["cell"]
        ledger = CellLedger.with_genesis(
            cell, transcript["genesis"], int(transcript["genesis_ts"])
        )
        for rec in transcript["records"]:
            ledger.record(rec["input"], rec["output"], int(rec["ts"]))

        entries = ledger.entries
        for entry, want in zip(entries, section["entries"]):
            self.assertEqual(entry.seq, want["seq"], "(e) seq contiguous from 1")
            assert_sha256_hex(self, f"(e) entry {entry.seq} prev_hash", entry.prev_hash, want["prev_hash"])
            assert_sha256_hex(self, f"(e) entry {entry.seq} seal", entry.hash, want["hash"])

        assert_sha256_hex(self, "(e) chain_hash (head)", ledger.chain_hash(), section["chain_hash"])

        report = ledger.reconcile()
        want = section["reconcile"]
        self.assertEqual(report["cell_id"], cell)
        self.assertEqual(report["entries"], want["entries"])
        self.assertEqual(report["open_inputs"], want["open_inputs"])
        self.assertEqual(report["matched_pairs"], want["matched_pairs"])
        self.assertEqual(report["chain_intact"], want["chain_intact"])
        self.assertEqual(report["continuity_intact"], want["continuity_intact"])
        self.assertEqual(report["balanced"], want["balanced"])
        assert_close(self, "(e) total_surprise", report["total_surprise"], want["total_surprise"], TOL_RECONCILE)
        assert_close(self, "(e) mean_surprise", report["mean_surprise"], want["mean_surprise"], TOL_RECONCILE)

    def test_open_input_settle_balances(self):
        ledger = CellLedger("slow.cell")
        ticket = ledger.open_input({"request": 1}, 5_000)
        self.assertEqual(ledger.reconcile()["open_inputs"], 1)
        self.assertFalse(ledger.reconcile()["balanced"])
        entry = ledger.settle_output(ticket, {"answer": 42}, 5_050)
        self.assertEqual(entry.input_value, {"request": 1})
        self.assertEqual(entry.output_value, {"answer": 42})
        report = ledger.reconcile()
        self.assertEqual(report["open_inputs"], 0)
        self.assertTrue(report["balanced"])

    def test_tamper_breaks_chain(self):
        ledger = CellLedger.with_genesis("sensor.a", 1.0, 0)
        ledger.record(2.0, 2.0, 1_000)
        ledger.record(3.0, 3.0, 2_000)
        self.assertTrue(ledger.verify_chain()["intact"])
        ledger.entries[1].body["output"]["value"] = 99.0
        audit = ledger.verify_chain()
        self.assertFalse(audit["intact"])
        self.assertEqual(audit["first_break"], 2)


# ---------------------------------------------------------------- supporting tiers


class CanonicalSerialization(unittest.TestCase):
    """The §2 pins every bit-for-bit claim stands on."""

    def test_compact_sorted_keys(self):
        self.assertEqual(
            canonical_json({"b": 1, "a": [2.5, True, None, "x"]}),
            '{"a":[2.5,true,null,"x"],"b":1}',
        )

    def test_int_float_distinction_is_in_the_preimage(self):
        self.assertEqual(canonical_json(85), "85")
        self.assertEqual(canonical_json(85.0), "85.0")
        self.assertEqual(canonical_json(2.5), "2.5")

    def test_float_exponents_normalized_to_ryu(self):
        self.assertEqual(canonical_json(1e-5), "1e-5")   # not 1e-05
        self.assertEqual(canonical_json(1e16), "1e16")   # not 1e+16

    def test_insertion_order_irrelevant(self):
        self.assertEqual(
            canonical_json({"x": 1, "y": 2}), canonical_json({"y": 2, "x": 1})
        )

    def test_sha256_known_vectors(self):
        self.assertEqual(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )
        self.assertEqual(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )


class ValueDistance(unittest.TestCase):
    """The sealed unit's delta.magnitude metric (cell-ledger.md §3)."""

    def test_vectors_from_the_rust_unit_tests(self):
        cases = [
            (3.0, 5.0, 2.0),
            (True, True, 0.0),
            (False, 0, 1.0),                      # type shift
            ([1.0, 2.0], [3.0, 2.0], 1.0),
            ([1.0], [1.0, 5.0], 0.5),             # missing element costs 1.0
            ({"a": 1}, {"a": 1, "b": 2}, 0.5),    # missing key costs 1.0
            ({}, {}, 0.0),
        ]
        for a, b, want in cases:
            self.assertTrue(math.isclose(value_distance(a, b), want, abs_tol=1e-12), (a, b))

    def test_kernel_bridge_d_mu(self):
        # The polyformal kernel's golden mu_hat vs WARM vectors: the same
        # mean-abs metric at field grain — the fractal claim, one line.
        mu_hat = [
            0.6079773483070409, -0.1542977767619208, 0.23858975545115765,
            0.13466234541390112, -0.48196639450032414, 0.18434012051083049,
            0.5149988698003736,
        ]
        warm = [
            0.7171371656006361, 0.23904572186687872, 0.23904572186687872,
            -0.35856858280031806, 0.35856858280031806, -0.23904572186687872,
            0.23904572186687872,
        ]
        want = sum(abs(a - b) for a, b in zip(mu_hat, warm)) / len(mu_hat)
        self.assertTrue(math.isclose(value_distance(mu_hat, warm), want, rel_tol=0, abs_tol=1e-15))


class SheetParsing(unittest.TestCase):
    GOLDEN_YAML = """\
id: bilge-reflex
description: The golden sheet (YAML round-trip of the contract sheet)
cells:
  - id: bilge.level
    kind: sensor
    source: simulated
    default: 40.0
  - id: bilge.threshold
    kind: value
    value: 80.0
  - id: pump.should_run
    kind: formula
    expr: "=bilge.level >= bilge.threshold"
  - id: pump.relay_cmd
    kind: formula
    expr: "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)"
  - id: status
    kind: value
    value: idle
"""

    def test_yaml_parses_to_the_golden_sheet(self):
        sheet = parse_sheet(self.GOLDEN_YAML)
        golden = sheet_from_dict(GOLDEN["sheet"])
        self.assertEqual(sheet.id, golden.id)
        self.assertEqual([c.id for c in sheet.cells], [c.id for c in golden.cells])
        self.assertEqual([c.kind for c in sheet.cells], [c.kind for c in golden.cells])
        for a, b in zip(sheet.cells, golden.cells):
            self.assertEqual(a.value, b.value, f"{a.id} value")
            self.assertEqual(a.expr, b.expr, f"{a.id} expr")
            self.assertEqual(a.default, b.default, f"{a.id} default")
        # The YAML-built engine behaves identically (op b spot check).
        e = QuiltEngine(sheet)
        self.assertFalse(e.get("pump.should_run").data)
        self.assertEqual(e.get("pump.relay_cmd").data, -20.0)

    def test_dependency_detection(self):
        deps = detect_dependencies(
            "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)",
            ["bilge.level", "bilge.threshold", "pump.relay_cmd"],
        )
        self.assertEqual(deps, ["bilge.level", "bilge.threshold"])

    def test_longest_id_wins(self):
        deps = detect_dependencies(
            "=compass.heading > 10 ? a : b", ["compass", "compass.heading", "a", "b"]
        )
        self.assertEqual(deps, ["compass.heading", "a", "b"])

    def test_string_literals_not_references(self):
        self.assertEqual(detect_dependencies("='temp' + temp", ["temp"]), ["temp"])

    def test_rejects_bad_sheets(self):
        with self.assertRaises(ValueError):
            parse_sheet("id: dup\ncells:\n  - id: a\n    kind: value\n    value: 1\n  - id: a\n    kind: value\n    value: 2\n")
        with self.assertRaises(ValueError):
            parse_sheet("id: bad\ncells:\n  - id: x\n    kind: value\n")
        with self.assertRaises(ValueError):
            parse_sheet("id: bad\ncells:\n  - id: x\n    kind: formula\n")


class EngineLedgerIntegration(unittest.TestCase):
    """The gym-tier extra: the engine records sealed edges + wire edges."""

    def test_push_records_the_golden_scalar_edge(self):
        engine = fresh_engine()
        engine.push("bilge.level", 85.0, ts=2000.0)
        edges = engine.wire_edges("bilge.level")
        self.assertEqual(len(edges), 1)
        e = edges[0]
        self.assertEqual((e["before"], e["after"]), (40.0, 85.0))
        self.assertEqual(e["delta"], 45.0)
        self.assertEqual(e["imbalance"], 45.0)
        self.assertEqual(e["provenance"], wire_provenance([85.0]))
        self.assertEqual(e["chain"], engine.ledgers["bilge.level"].genesis_commit())
        self.assertEqual(e["ts"], 2000.0)
        self.assertEqual(e["v"], 1)
        self.assertEqual(e["seq"], 1)

    def test_formula_recompute_posts_dependency_snapshot(self):
        engine = fresh_engine()
        engine.push("bilge.level", 85.0, ts=2000.0)
        engine.get("pump.relay_cmd", ts=2001.0)
        entry = engine.ledgers["pump.relay_cmd"].head
        # Input posting: dep snapshot in dependency-address order.
        self.assertEqual(entry.input_value, [85.0, 80.0])
        self.assertEqual(entry.output_value, 2.5)
        self.assertEqual(entry.body["provenance"]["origin"], "get")

    def test_books_balance_after_scenario(self):
        engine = fresh_engine()
        engine.push("bilge.level", 85.0, ts=2000.0)
        for cell in ("pump.should_run", "pump.relay_cmd"):
            engine.get(cell, ts=2001.0)
        for cid, ledger in engine.ledgers.items():
            report = ledger.reconcile()
            self.assertTrue(report["balanced"], f"{cid}: {report}")
            self.assertTrue(ledger.verify_chain()["intact"], cid)


# ---------------------------------------------------------------- reporting


def run_ops() -> bool:
    """Run the five core ops with harness-style PASS/FAIL lines."""
    print("=== quilt-py conformance (tier: python) ===")
    print(f"contract: {GOLDEN['contract']}  golden: compat/golden.json")
    ok = True

    def run(cls) -> bool:
        suite = unittest.TestSuite()
        loader = unittest.defaultTestLoader
        for name in loader.getTestCaseNames(cls):
            suite.addTest(cls(name))
        result = unittest.TextTestRunner(stream=_NullStream(), verbosity=0).run(suite)
        return result.wasSuccessful()

    order = [
        (OpAValueRead, "(a) value cell read"),
        (OpBFormulaEval, "(b) formula cell eval"),
        (OpCPropagation, "(c) propagation order"),
        (OpDEdge, "(d) edge record"),
        (OpEChain, "(e) chain + reconcile"),
    ]
    for cls, label in order:
        good = run(cls)
        ok = ok and good
        print(f"  {label:<28} {'PASS' if good else 'FAIL'}")
    return ok


class _NullStream:
    def write(self, *_a):
        pass

    def flush(self):
        pass


def report_numbers():
    """Print the golden numbers the way the task asked for."""
    g = GOLDEN
    print()
    print("─" * 72)
    print("golden numbers (compat/golden.json)")
    print("─" * 72)
    print("(a) value reads:")
    for v in g["op_a_value_read"]:
        print(f"    {v['cell']:<16} = {v['expect']!r}")
    b = g["op_b_formula_eval"]
    print("(b) formula eval:")
    for v in b["initial"]:
        print(f"    {v['cell']:<16} = {v['expect']!r}   (initial)")
    print(f"    push bilge.level → {b['after_push']['value']}")
    for v in b["post"]:
        print(f"    {v['cell']:<16} = {v['expect']!r}   (post-push)")
    c = g["op_c_propagation"]
    print(f"(c) propagation order after {c['mutate']['cell']}={c['mutate']['value']}:")
    print(f"    {c['expected_order']}")
    print("(d) wire edges:")
    for v in g["op_d_edge"]:
        print(
            f"    {v['name']:<20} Δ={v['expect']['delta']!r:<28}"
            f"imb={v['expect']['imbalance']!r:<8}"
            f"prov={v['expect']['provenance'][:16]}…"
        )
    e = g["op_e_chain"]
    print(f"(e) chain ({e['transcript']['cell']}, genesis {e['transcript']['genesis']} @ {e['transcript']['genesis_ts']}):")
    for entry in e["entries"]:
        print(f"    seq {entry['seq']}  prev {entry['prev_hash'][:16]}…  seal {entry['hash'][:16]}…")
    print(f"    chain_hash          = {e['chain_hash']}")
    rec = e["reconcile"]
    print(
        f"    reconcile: entries={rec['entries']} balanced={rec['balanced']} "
        f"total_surprise={rec['total_surprise']} mean_surprise={rec['mean_surprise']}"
    )
    print("─" * 72)


def main() -> int:
    if GOLDEN.get("contract") != "quilt-compat/1":
        raise SystemExit(
            f"golden.json contract {GOLDEN.get('contract')!r} != 'quilt-compat/1' — "
            "this tier implements quilt-compat/1; fail loudly, never guess (§7)."
        )
    if GOLDEN.get("spec", {}).get("edge_schema_v") != 1:
        raise SystemExit("golden.json edge_schema_v != 1 — refusing to guess (§7).")

    ops_ok = run_ops()
    report_numbers()

    # Full suite (ops + canonical + parsing + integration) for the exit code.
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    print()
    if result.wasSuccessful() and ops_ok:
        print("RESULT: PASS — python tier conforms to quilt-compat/1")
        return 0
    print("RESULT: FAIL — see failures above")
    return 1


if __name__ == "__main__":
    sys.exit(main())
