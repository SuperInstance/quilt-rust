# The quilt-compat Contract — one edge, many substrates, one golden file

Status: **v1, in force.** Machine-checkable at `compat/golden.json`; reference-tier
proof at `compat/conformance_test.rs` (run below). This document is the
compatibility contract between every quilt substrate — the TypeScript quilt, the
Rust quilt, and the tiers beyond them — and it is the direct descendant of the
polyformal-kernel move (`~/projects/zeroclaw-dissertation/research/polyformal-kernel/SPEC.md`):
*specify once, implement many times, differential-test against golden
vectors.* The kernel proved the pattern for one `f(x) → y` in ten languages,
bit-for-bit. This contract applies the same pattern at runtime scale: one
**edge record**, every tier, one `golden.json`.

Why this is the load-bearing artifact: [fleet-as-fractal-jepa](fleet-as-fractal-jepa.md)
claims "one ledger, many substrates" — the same before→after edge at pin, room,
model, and fleet grain. That claim is only true if a Rust edge, a TypeScript
edge, a Python edge, and a CUDA edge can be made to *agree*. Agreement is not a
hope; it is a test. This file is the test's definition.

Three documents, one primitive:

| Document | Role |
| --- | --- |
| `docs/cell-ledger.md` | the sealed unit — the hash-chained double entry (internal shape) |
| **`docs/quilt-compat-contract.md`** (this file) | the interchange contract — wire schema, golden vectors, tolerances, tiers |
| `compat/golden.json` | the machine-checkable vectors every tier must reproduce |

---

## 1. The canonical edge schema (v1)

The language-neutral ledger record **every tier reads and writes**. This exact
shape, these exact field names — they are frozen:

```jsonc
{
  "v": 1,                        // schema version (always 1 for this contract)
  "cell": "stable.address",      // the cell's stable id — an address, never a coordinate
  "ts": 1000.0,                  // float milliseconds since unix epoch
  "before": <value|vector>,      // cell state before the transaction
  "after": <value|vector>,       // cell state after the transaction
  "delta": <after-before>,       // see §1.1 — same shape as before/after, or null
  "imbalance": <|after - predict(before)|>,  // see §1.2 — number or null
  "provenance": "<sha256 of inputs>",        // see §1.3 — 64 lowercase hex chars
  "chain": "<sha256 of prior edge for this cell>"  // see §1.4 — 64 lowercase hex chars
}
```

**Minimal extension (the only one v1 allows):** `"seq": <integer>` — the
per-cell sequence number, contiguous from 1, chain order. It is optional on the
wire but required in any sealed ledger (the existing `CellLedger` already
carries it); it makes ordering and reconcile checks total without parsing
timestamps.

No field of v1 may ever be renamed or repurposed. Additions in v2+ must be
optional fields that old tiers can ignore (see §7).

### 1.1 `delta` — the edge

`delta = after − before`, first-person: the change as the cell experienced it.

- **number → number:** `after − before` (scalar difference).
- **vector → vector:** element-wise `after[i] − before[i]` (equal lengths, all
  numeric; otherwise `null`).
- **anything else** (strings, booleans, objects, mixed types, `before: null`):
  **`null`** — never fake a number. A non-numeric edge is recorded as having
  happened, not as having a magnitude; tiers that need a magnitude for
  structured values use the ledger's `value_distance` (§2 of `cell-ledger.md`)
  *outside* the wire `delta`.

Golden vectors pin the dyadic cases (`0.375 − 0.25 = 0.125` exactly) so the
bit-for-bit tier has zero float-rendering ambiguity in `delta`.

### 1.2 `imbalance` — the surprise

`imbalance = |after − predict(before)|` — the JEPA loss at cell grain: how wrong
the cell's own forecast of its next state was.

- **Default predictor: the persistence prior** — `predict(before) = before`
  ("the world stays as it was"). Then surprise *is* the edge:
  - scalar: `|after − before|`
  - vector: the L2 norm `‖after − before‖₂` (the kernel's `d_mu` shape — a
    norm, not a vector)
- **Explicit predictor:** a tier may record its model's forecast; the
  imbalance is then scored against that forecast and the two come apart
  (edge = what changed, imbalance = what the predictor missed).
- **No prior** (first edge of a cell with no genesis, i.e. `before: null`):
  **`null`**. Never fake a number. This is the kernel's κ-discipline applied to
  surprise: a value that was not measured is not guessed.

### 1.3 `provenance` — the input commitment

`provenance = sha256_hex( canonical_json( inputs ) )` where:

- `inputs` is the JSON **array** of input values that flowed into the cell for
  this transaction, in **dependency-address order** (dependencies sorted by
  UTF-8 byte order of their ids — deterministic in every language).
- Sensor push: `[pushed_value]`. Formula eval: the dependency snapshot
  `[dep_a_value, dep_b_value, …]`. Value read: `[]`.
- Single inputs are still wrapped in the array (`[85.0]`, not `85.0`) so the
  preimage grammar is uniform.

### 1.4 `chain` — the tamper-evident link

`chain` = the seal (`hash`) of the **prior sealed entry** for this cell; for a
cell's first entry it is the **genesis root**:

```
genesis_root = sha256_hex( canonical_json({
  "kind": "quilt-cell-ledger/1", "cell_id": …,
  "genesis": …, "genesis_ts": …
}) )
```

The sealed unit is the `LedgerEntry` of `packages/core/src/ledger.rs` (fully
specified in `docs/cell-ledger.md` §3–4): each entry's `hash` is
`sha256_hex(canonical_json(entry minus its hash))`, and `entry.prev_hash` is
what the wire edge's `chain` field carries. Editing any entry breaks every seal
after it. A consumer that only *routes* edges (relay, queue, corpus) can treat
`chain` as an opaque string; a consumer that *verifies* books ports the
canonicalization rules in §2 and recomputes seals.

### 1.5 Wire edge ↔ sealed entry projection

The wire edge is the projection of a sealed `LedgerEntry`:

| Wire field | From `LedgerEntry` |
| --- | --- |
| `v` | constant `1` |
| `cell` | `cell_id` |
| `ts` | `entry.ts` (u64 millis) as float |
| `before` / `after` | `entry.delta.before` / `entry.delta.after` |
| `delta` | computed per §1.1 (numeric scalars/vectors only; the ledger's `delta.magnitude` is the generic `value_distance`, which coincides for scalars) |
| `imbalance` | `entry.imbalance` |
| `provenance` | §1.3 over `[entry.input.value]` (v1: one input posting; snapshot arrays when the engine posts multi-dependency inputs) |
| `chain` | `entry.prev_hash` |
| `seq` | `entry.seq` |

---

## 2. Canonical serialization and hashing

Pinned exactly as `docs/cell-ledger.md` §4 — restated here because every
bit-for-bit claim in this contract stands on it:

1. **Compact JSON** — no whitespace.
2. **Object keys sorted by UTF-8 byte order** — insertion order is irrelevant.
3. **Integers render as integers; floats as shortest-round-trip decimal with
   the float marker preserved** (serde_json / ryū semantics: `2.5` → `2.5`,
   `85.0` → `85.0`, `85` → `85`). The float/int distinction is part of the
   hash preimage. *(Found and fixed while landing this contract: the reference
   `canonical_number` had been rendering `85.0` as `"85"`, diverging from the
   pinned ryū rule. `ledger.rs` now honors the pin; `golden.json` hashes are
   generated under the corrected rule.)*
4. **Strings** via standard JSON escaping.
5. **SHA-256** (FIPS 180-4), lowercase hex, everywhere a hash appears.

Known port hazards:

- **JavaScript renders `85.0` as `85`** (`JSON.stringify`, number→string). A
  TS port must tag floats and emit the `.0`. Python's `json.dumps` and
  serde_json/ryū agree with the pin by default.
- **Do not re-serialize** a parsed edge with a stock serializer and expect
  chain agreement — canonical form is a function, not a library default.

---

## 3. The golden-vector contract

`compat/golden.json` is normative. Every tier must reproduce, for the five
**core ops**, the vectors it contains — bit-for-bit where §4 says exact, within
its declared tolerance otherwise. The golden sheet is `bilge-reflex`: a sensor
(`bilge.level`, default 40.0), a threshold (80.0), a decision formula, a
command formula, a status value. Small on purpose: the vectors must be cheap
enough to run in every CI on every substrate.

| Op | What it proves | Vector shape |
| --- | --- | --- |
| **(a) value cell read** | the leaf of reactivity — same bytes back everywhere | `{cell, expect}` — exact JSON equality |
| **(b) formula cell eval** | the portable expression subset computes identically | initial + post-push `{cell, expect}` |
| **(c) reactive propagation order** | the graph walk is a *deterministic* topological order: Kahn's algorithm over the affected closure, ties broken by lexicographic (UTF-8 byte) address order | `expected_order` list + the engine's dependency sets must equal the golden `graph` |
| **(d) edge record** | before/after/delta/imbalance/provenance per §1 | scalar, vector, and null-prior vectors |
| **(e) ledger chain-hash + reconcile** | seals are bit-for-bit; the books balance | fixed 3-record transcript → per-entry `prev_hash`/`hash`, `chain_hash`, full reconcile report |

**The portable formula subset** (what golden formulas may use, and all tiers
must evaluate): float arithmetic (`+ − * /` on floats), comparisons, `abs`,
`min`, `max`, `clamp(x, lo, hi)`, cell references by address. Avoid: integer
division semantics (`40 * 9 / 5` is int division in some tiers — write
`40.0 * 9.0 / 5.0`), string ops, anything language-specific. Golden values are
chosen dyadic (40.0, 85.0, 0.125, 0.1875) so reference tiers hold them exactly.

**How a tier joins the contract:** implement the five ops against
`compat/golden.json` in your substrate (mirror `compat/conformance_test.rs` —
it is the reference harness and the template), declare your tolerances per §4
(looser than the reference, never looser than the gate), and wire it into that
tier's CI. A tier is *conformant* when its harness passes in its own CI on its
own substrate. Conformance is per-release, not per-claim.

---

## 4. Tolerance table

Two numbers matter: what the **reference tier asserts** (Rust, this repo — the
tightest, since it generates the vectors) and the **gate** — the loosest any
conforming tier may declare, following the polyformal kernel's precedent
(`A7 ≤ 1e-9`, `fit ≤ 1e-6`, `edge ≤ 1e-6`).

| Op class | Reference (Rust) | Gate (loosest declared) | Precedent |
| --- | --- | --- | --- |
| (a) value cell read | **exact** (JSON equality) | exact | determinism |
| (b) formula eval | **1e-12** | **1e-9** | kernel `A7 ≤ 1e-9` |
| (b′) analytic/special-function ops¹ | 1e-12 | 1e-9 | kernel `A7 ≤ 1e-9` |
| (b″) iterative-fit ops² | 1e-12 | 1e-6 | kernel `fit ≤ 1e-6` |
| (c) propagation order | **exact** (ordered list) | exact | determinism |
| (d) edge delta / imbalance | **1e-12** | **1e-6** | kernel `edge ≤ 1e-6` |
| (e) chain hashes | **bit-for-bit** | **bit-for-bit** | hash equality |
| (e′) reconcile totals | 1e-12 | 1e-6 | kernel `fit ≤ 1e-6` |

¹ e.g. Bessel ratios, log-space κ — the Julia/R/Python representation tier's
bread. ² e.g. `vmf_fit`-class solvers, model fits consuming ledger corpora.

Per-tier declarations (the conformance class each tier claims):

| Tier | (a) read | (b) eval | (c) order | (d) edge | (e) chain | (e′) reconcile |
| --- | --- | --- | --- | --- | --- | --- |
| **Rust** (reference) | exact | 1e-12 | exact | 1e-12 | bit-for-bit | 1e-12 |
| TypeScript / Python / Go / WASM | exact | 1e-12 | exact | 1e-9³ | bit-for-bit | 1e-6 |
| Julia / R | exact | 1e-12 | exact | 1e-9 | bit-for-bit | 1e-6 |
| C (ABI) | exact | passthrough⁴ | exact | passthrough⁴ | bit-for-bit | passthrough⁴ |
| CUDA / PTX | exact | 1e-6 | exact | 1e-6 | bit-for-bit | 1e-6 |

³ float tiers should hold 1e-12 on dyadic golden vectors; 1e-9 is the headroom
they may declare for non-dyadic production edges (the A7 precedent).
⁴ the C ABI is the *boundary*, not a producer: it passes bytes through without
recomputing, so it inherits the producing tier's numbers — and still verifies
chain hashes bit-for-bit.

Rules:

- A tier declares per-op tolerances **at or below the gate**; never above.
- Chain hashes are **never** tolerant. Either the 64 hex chars match or the
  tier does not conform.
- Discrete values (reads, orders, booleans, reconcile structure) are **exact**,
  always.
- If a tier cannot meet the gate on an op, that is a finding, not a waiver —
  the kernel's rule: divergence past tolerance is a bug report about a tier,
  not a renegotiation of the contract.

---

## 5. The tier map

One edge, many substrates — who each substrate is in the fleet:

| Tier | Role in the fleet | Substrate notes | Conformance class |
| --- | --- | --- | --- |
| **TypeScript** (`quilt`) | **connecting** — the wiring surface: MCP servers, browser simulators, harnesses, the live laboratory | `@quilt/core`; the canonical sheet authoring experience | float tier |
| **Python** | **gym + connecting** — the training loop (elephant, Cosmos/Isaac world-models) and relay/cortex tooling; consumes ledger corpora as training data | the vMF/JEPA side already speaks the kernel's tolerances | float tier (fit-class ops 1e-6) |
| **Go** | **parallel / relay** — the always-on synapse (lucineer-relay, crab-traps, fleet-gateway): buffers and forwards edges while the cortex sleeps | edge routing is its day job; `chain` is an opaque string until verification is needed | float tier |
| **Julia / R** | **representation** — statistics and representation-space work: field fits, κ estimation, the analytic ops behind `d_mu` / `d_log_kappa` | the kernel's A7/fit precedent is native here | float tier (A7 1e-9, fit 1e-6) |
| **Rust** (`quilt-rust`) | **reference + embedded** — *this repo*: defines the golden vectors, generates the reference hashes, and is the ESP32/`no_std` limb (reflex arc) | `quilt-core` ledger + engine; single static binary; the cortex's commit discipline | **reference** (1e-12) |
| **C** | **ABI boundary** — the stable C-ABI any runtime can `dlopen`; the seam between fast tiers and the rest | passes records through; verifies chains; computes nothing it doesn't own | passthrough + bit-for-bit |
| **CUDA / PTX** | **raw speed** — tensor/field math in the gym: batch edge computation, world-model stepping | fast-math allowed at 1e-6 on (b)/(d); hashes still bit-for-bit | fast tier |
| **WASM** | **web / edge** — quilt-live in the browser, edge workers, the sandbox tier | the TS port's sandbox story and the browser runtime | float tier |

The three-speed reading (per `fleet-as-fractal-jepa`): Rust is the spinal cord
(embedded, always-on, first-person edges), TS/Go/WASM are the synapses
(connecting, relaying), Python/Julia/R/CUDA are the cortex and the gym
(thinking, training, representing). Every one of them speaks §1.

---

## 6. Running and regenerating

Reference-tier conformance (the proof):

```sh
cargo test -p quilt-core --test quilt_compat_conformance -- --nocapture
```

Regenerate reference values in `golden.json` — only after a **deliberate**
contract change, never to make a failure disappear (a hash that changes when
you didn't intend it is the contract telling you something):

```sh
cargo test -p quilt-core --test quilt_compat_conformance -- --ignored --nocapture
```

The harness is registered in `packages/core/Cargo.toml` as a `[[test]]` target
pointing at `compat/conformance_test.rs`, so plain `cargo test -p quilt-core`
runs it with the rest of the suite.

## 7. Versioning and evolution

- `"v": 1` is frozen: field names, semantics, canonicalization, and every hash
  already in `golden.json`. Chain hashes are forever — a v2 that changes any
  preimage rule forks the chain, and forking the chain is a new ledger, not an
  upgrade.
- Additions must be **optional fields** that v1 readers ignore; the golden
  vectors only ever pin fields that existed in v1 plus their own expected
  values.
- Tolerance gates may tighten, never loosen.
- The contract id (`"quilt-compat/1"`) travels in `golden.json` and in every
  harness assertion; a tier that reads a newer `golden.json` than it implements
  must fail loudly, not silently guess.

---

*The polyformal kernel proved that one edge function in ten languages can be
bit-for-bit identical. This contract is that proof, promoted to a runtime: the
fleet's ledger is only "one ledger" because every substrate can reproduce the
same edge, the same seal, and the same surprise — and now, for every tier that
claims a place in the fleet, there is a file to run.*
