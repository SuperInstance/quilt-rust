# quilt-py — the Python binding (gym / connecting tier)

An independent, **stdlib-only** Python implementation of the quilt
cellular runtime: sheet parsing (YAML), value/formula evaluation with
the same lazy-reactive semantics as the Rust/TS engines, and the
hash-chained double-entry cell ledger — sealed and projected onto the
[quilt-compat](../../docs/quilt-compat-contract.md) wire edge. Python
is where PyTorch / JEPA / elephant live; this tier is how the gym
consumes and produces ledger corpora.

## Conformance

Conforms to **quilt-compat/1** at the Python tier's declared class
(contract §4):

| op | tolerance |
| --- | --- |
| (a) value read | exact |
| (b) formula eval | 1e-12 |
| (c) propagation order | exact |
| (d) edge delta / imbalance | 1e-9 (dyadic golden vectors hold exactly) |
| (e) chain hashes | **bit-for-bit** |
| (e′) reconcile totals | 1e-6 (holds exactly) |

Run the harness (mirrors `compat/conformance_test.rs`):

```sh
cd bindings/python
python3 tests/test_compat.py
```

It runs the five core ops against the normative `compat/golden.json`
and prints PASS/FAIL plus the golden numbers (value evals, formula
evals, edge deltas, chain hashes, reconcile totals). Exit 0 = PASS.
The golden file is never regenerated here — it is the contract's, and
the reference (Rust) tier generates it.

## Layout

```
quilt/
  miniyaml.py   stdlib YAML-subset parser for quilt sheets
  formula.py    expression evaluator (portable subset: + - * / %, comparisons,
                ternary, && || !, abs/min/max/clamp, cell refs by address)
  ledger.py     canonical JSON (ryū-style floats), sha256, value_distance,
                wire-edge functions (§1), and the sealed CellLedger port
  engine.py     QuiltEngine: lazy recompute, dependency detection,
                propagation order (Kahn, lexicographic ties), edge recording
tests/
  test_compat.py  the conformance harness
```

## Usage

```python
from quilt import QuiltEngine

engine = QuiltEngine.from_yaml("""
id: bilge-reflex
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
""")

engine.get("pump.should_run").data      # False
engine.push("bilge.level", 85.0, ts=2000.0)
engine.get("pump.should_run").data      # True  (lazy recompute on read)

engine.wire_edges("bilge.level")[0]
# {'v': 1, 'cell': 'bilge.level', 'ts': 2000.0, 'before': 40.0,
#  'after': 85.0, 'delta': 45.0, 'imbalance': 45.0,
#  'provenance': 'c488fc2a…', 'chain': '<genesis root>', 'seq': 1}

engine.chain_hash("bilge.level")        # tamper-evident head
engine.ledgers["bilge.level"].reconcile()  # the books balance
```

## Semantics notes (documented divergences)

* **Division is real division** (`21 * 9 / 5 + 32 == 69.8`), matching
  the TS engine and the sheet examples. The Rust tier's rhai would
  integer-divide all-int operands; the contract's portable subset
  avoids the case by writing floats.
* **Int/float distinction is preserved** through arithmetic and into
  the canonical hash preimage (`85` and `85.0` are different bytes).
* **`%` is the JS truncated remainder** (sign of dividend):
  `-7 % 360 == -7`.
* Formula recomputes post the **dependency snapshot** (dep values in
  dependency-address order) as the input side of the double entry;
  `set`/`push` post the written value on both sides.
* Everything is deterministic: callers pass timestamps; no clocks, no
  I/O, no randomness in the ledger path.
