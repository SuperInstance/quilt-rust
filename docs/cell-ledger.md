# The Cell Ledger — double-entry memory for every quilt cell

> A cell is not a value. It is a live, addressable capability. This document
> adds the corollary: **a cell is also a witness**. The `CellLedger`
> (`packages/core/src/ledger.rs`) gives every cell a first-person,
> replayable input→output record — an append-only, hash-chained, double-entry
> ledger where the *imbalance* (surprise / prediction-error) is a first-class
> recorded value.

Status: working prototype (pure data structure + serde, 16 unit tests,
`cargo test -p quilt-core`). Not yet wired into engine evaluation — see
[Integration path](#9-integration-path--scope).

---

## 1. The idea in one paragraph

The elephant project treats `field_before → field_after` as the unit of
perception: a room is not its state, it is the *change* in its state. The
polyformal kernel proved the same shape for its `edge(fb, fa)` — a
before→after record with three distances — is language-independent,
reproducible bit-for-bit across 10 languages. The `CellLedger` instantiates
that shape at **cell grain**: every time a quilt cell is touched, one ledger
entry records what came in (input posting), what went out (output posting),
the edge of the cell's state (`before → after`), and how surprising the
outcome was (`imbalance`). Chained by hash, these entries become a
tamper-evident autobiography the cell can replay, an auditor can verify, and
a training corpus can consume.

## 2. The double-entry invariant

Accounting keeps honest books by posting every transaction twice: a *debit*
and a *credit*. The cell ledger does the same for the only transaction a
cell ever performs — being asked something and answering.

| Accounting          | Cell ledger                                                  |
| ------------------- | ------------------------------------------------------------ |
| debit               | **input posting** — what the world gave the cell             |
| credit              | **output posting** — what the cell gave back                 |
| open receivable     | **open input** — posted debit, no credit yet                 |
| books balance       | every entry has both sides; no open inputs                   |
| trial balance       | `reconcile()`                                                |
| fraud / error       | broken hash chain, discontinuous history, open inputs        |
| profit-and-loss     | the **imbalance** series — the cell's surprise over time     |

Two consequences:

1. **Structural balance.** `record(input, output, ts)` posts both sides
   atomically. For async cells (`api`, `program`, `router` — where the answer
   arrives later than the question), `open_input(input, ts)` posts the debit,
   and `settle_output(ticket, output, ts)` posts the matching credit. An
   input left open is a first-class fact: the cell owes the world a response,
   and `reconcile()` reports it as `open_inputs`.
2. **Predictive balance.** The two sides of an entry are usually different
   *kinds* of things (an 85 cm reading in, a `true` out), so reconciliation
   is not "input == output" — it is "output == what the input entailed, as
   far as anyone claimed." That claim is the **prediction**, and the gap
   between claim and outcome is the **imbalance** (§5).

## 3. Schema

One entry per transaction:

```jsonc
{
  "seq": 1,                       // contiguous from 1, chain order
  "ts": 1726243200000,            // the input posting's time
  "input":  { "side": "input",  "value": 85.0, "ts": 1726243200000 },
  "output": { "side": "output", "value": 85.0, "ts": 1726243200004 },
  "provenance": {
    "origin": "push",             // get | set | call | push | system
    "caller": "bilge.adapter",    // who touched the cell
    "trace": ["pump.should_run"]  // ancestor chain
  },
  "delta": {                      // the edge — before → after
    "before": 40.0,
    "after": 85.0,
    "changed": true,
    "magnitude": 45.0             // value_distance(before, after)
  },
  "expected": 40.0,               // the prediction this entry scored against
  "imbalance": 45.0,              // value_distance(expected, output) — surprise
  "prev_hash": "9f2c…",           // previous entry's hash (or the genesis commit)
  "hash": "b01d…"                 // seal — see §4
}
```

- **`delta`** is the perception record: the change, not the state. Its
  `magnitude` is the cell-grain analogue of the polyformal kernel's `d_mu`.
- **`expected`** is the forecast the outcome was scored against. Under the
  default **persistence prior** — "the world stays as it was" — the
  prediction is the cell's `before` state, and then
  `imbalance == delta.magnitude` *by construction*: surprise *is* the edge.
  This is the elephant's claim, made operational. When a caller supplies an
  explicit forecast (`record_with(..., expected)` or
  `settle_output_with`), surprise and edge come apart: the edge measures what
  changed, the imbalance measures what the predictor missed.
- **`expected` is hashed into the seal.** A prediction cannot be rewritten
  after the outcome is known. This is what makes a ledger usable as honest
  training data (§8).
- The first entry of a genesis-less ledger records `expected: null` /
  `imbalance: null` — with no prior, no surprise is claimed. Never fake a
  number (the kernel's rule for κ; the same discipline here).

The distance metric `value_distance` is total over JSON values: numbers give
`|a − b|`; equal values give `0`; arrays score the mean of element-wise
distances with missing elements costing `1.0`; objects score the key union
with missing keys costing `1.0`; any type shift costs `1.0`. Domains that
want richer edges (string edit distance, `d_log_kappa`, …) extend the
metric; the schema does not change.

## 4. Hashing — the chain

Every entry commits to everything before it:

```
hash(e) = sha256_hex( canonical_json( e minus its hash field ) )
```

where `e` includes `prev_hash`, and `canonical_json` is exactly:

- compact JSON, no whitespace;
- object keys sorted by UTF-8 byte order (insertion order is irrelevant);
- integers rendered as integers; floats as Rust's shortest-round-trip
  decimal (serde_json / ryū semantics — e.g. `2.5`, `40.0`);
- strings via standard JSON escaping.

The chain root for an empty ledger is the **genesis commit**:

```
sha256_hex( canonical_json({
  "kind": "quilt-cell-ledger/1",
  "cell_id": …, "genesis": …, "genesis_ts": …
}) )
```

so the head hash (`chain_hash()`) commits to the cell's *identity*, its
initial state, and every transaction it ever recorded — even before the
first entry. Consequences:

- **Tamper detection.** Editing any entry (or the genesis, or the cell id)
  breaks that entry's seal and every seal after it. `verify_chain()` walks
  and recomputes, reporting the `first_break`.
- **Bit-for-bit portability.** SHA-256 and the canonical form are pinned by
  this document and `ledger.rs` (the hash is implemented inline — zero new
  dependencies — precisely so the specification travels with the code). A
  TypeScript or Python port that implements the same canonical rules
  reproduces the same chain hashes from the same entries: the polyformal
  property (one kernel, many languages, differential-tested) applied to the
  ledger. Known hazard: JavaScript renders `40.0` as `40`; ports must
  preserve float-vs-int distinction to stay on-chain.
- **Aggregation by Merkle-style citation.** A corpus that consumed ledgers
  at head `H` can be re-derived exactly; any later append produces a
  different head. Dataset provenance becomes a hash, not a promise.

## 5. Reconciliation — what `reconcile()` checks

`reconcile()` walks the books and returns a report:

| Check          | Meaning                                                            |
| -------------- | ------------------------------------------------------------------ |
| `matched_pairs`| entries carrying both an input and an output posting                |
| `open_inputs`  | debits without credits — the cell owes answers                      |
| `chain_intact` | every seal and prev-link verifies (else `first_break`)              |
| `continuity`   | `entry[i].before == entry[i−1].after`, and `entry[0].before == genesis` |
| `total_surprise` / `mean_surprise` | the cell's accumulated and average imbalance     |
| `balanced`     | all of the above clean — the books balance                          |

The **imbalance field is the cell's first-person view**: structurally, what
it has not yet answered; predictively, how wrong it (or its predictor) was.
Mean surprise is a volatility measure — a cell whose surprise is persistently
high lives in a region its model does not cover, which is precisely the
signal for what to learn next.

## 6. Replay — reconstructing any past state

`replay(until_ts)` returns every entry at or before the cutoff, plus the
state reconstructed from them (the `after` of the last replayed entry, else
the genesis, else `null`), plus the cumulative surprise of the prefix.

- **State replay** is exact and cheap: the ledger *is* the history, so
  reconstruction is a fold over recorded `after` values. `replay(t)` on a
  clean ledger always agrees with what `state()` would have returned at time
  `t` — the continuity check in `reconcile()` is what guarantees this.
- **Computation replay** composes the ledger with the engine: each entry
  carries its input, so a caller can feed the input series back through a
  cell evaluator — unchanged, to verify recordings, or modified, to ask
  counterfactuals ("what would this cell have answered under the new
  formula?") — and score the divergence against the recorded outputs.
- **Time-cut discipline.** Because replay takes a timestamp, corpora can be
  cut at a moment with no leakage across the cut: train on
  `replay(T_train)`, evaluate on the suffix after `T_train`.

## 7. From ledgers to corpora — the self-improvement loop

Ledgers aggregate upward without new machinery, because a `CellLedger` is
plain serde data:

```
entry          = (input, expected, output, delta, imbalance, provenance, seal)
cell corpus    = one CellLedger                        — the cell's autobiography
sheet corpus   = { cell_id: CellLedger }               — a sheet's shared memory
fleet corpus   = { sheet_id: sheet corpus }            — the training ground
```

The training loop this enables:

1. **Record.** Cells run; ledgers fill with paired transactions and
   hash-committed predictions.
2. **Extract.** Every entry is a supervised example — input and outcome —
   *and* a scored forecast (the `expected`/`imbalance` pair). No separate
   logging path, no schema drift: the ledger is the dataset.
3. **Cut.** `replay(T)` produces point-in-time-consistent training sets; the
   chain hash cites exactly which data a model saw.
4. **Fit.** Train predictors (per cell, per sheet, per fleet) to minimize
   future `imbalance`.
5. **Deploy predictions.** Feed the predictor's forecast back in as
   `expected` on the next round of records — where it is sealed into the
   chain *before* the outcome exists.
6. **Measure.** `reconcile().mean_surprise` over time is the honest learning
   curve: it cannot be gamed by rewriting forecasts, because forecasts are
   hashed. A cell (or a fleet) improves exactly when its recorded surprise
   falls.

This is the "self-improvement" claim reduced to bookkeeping: **a system that
keeps honest double-entry books about its own surprise has, in the ledger
itself, the training corpus, the evaluation harness, and the audit trail.**

## 8. Rust API (prototype)

```rust
use quilt_core::{CellLedger, Provenance, LedgerOrigin};

let mut ledger = CellLedger::with_genesis("bilge.level", serde_json::json!(40.0), 0);
ledger.record(serde_json::json!(85.0), serde_json::json!(85.0), 1_000);   // sealed pair

// Async cell: post the debit now, the credit when the answer exists.
let t = ledger.open_input(serde_json::json!({"q": "status"}), 2_000);
ledger.settle_output(t, serde_json::json!({"status": "ok"}), 2_050)?;

let report = ledger.reconcile();     // balanced? chain? surprise totals?
let view   = ledger.replay(2_000);   // state + surprise as of t=2_000
let head   = ledger.chain_hash();    // commits to everything above
```

Surface: `new` / `with_genesis`, `record[_with]`, `open_input[_with]`,
`settle_output[_with]`, `reconcile`, `replay`, `chain_hash`, `verify_chain`,
plus accessors (`entries`, `head`, `state`, `pending`). Pure data structure:
no tokio, no clocks (callers pass timestamps in millis), no I/O — only
`serde`/`serde_json` and a private, in-file SHA-256.

## 9. Integration path & scope

Honest limits of the prototype:

- **Not yet engine-wired.** The natural joint is `QuiltEngine`'s evaluation
  paths: `get`/`set`/`call`/`push` already carry `CallerContext`
  (provenance) and produce `CellValue`s (outcomes); a `ledgers:
  RwLock<HashMap<CellId, CellLedger>>` beside `cells` records them. Formula
  dependency snapshots give the input side; effects give cost fields worth
  adding to entries later.
- **Capacity policy** (ring-buffer vs. snapshot-to-disk vs. offload) is
  deliberately unchosen; the append-only core supports all three.
- **One distance metric.** `value_distance` is total but generic; cell kinds
  with richer geometry (vector-valued cells) will want specialized edges —
  the kernel's `d_mu` / `d_warmth` / `d_log_kappa` is the template.
- **Concurrency.** `CellLedger` is not internally synchronized; embed it
  behind the engine's existing `parking_lot` locks (same pattern as cells).

The claim this prototype establishes: the before→after edge, proven
language-independent at field grain, works the same way at cell grain — and
when you make surprise a *posted, hashed, first-class* value, memory,
audit, and training data become one structure.
