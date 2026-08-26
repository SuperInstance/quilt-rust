# The Field-Edge / Ledger Bridge — `imbalance` IS the field-edge

*Companion to [fleet-as-fractal-jepa.md](fleet-as-fractal-jepa.md) and
[cell-ledger.md](cell-ledger.md); the tissue-layer follow-up is
[cohesion-and-fascia.md](cohesion-and-fascia.md). Proof-of-identity prototype:
`crates/field-edge-bridge/bridge_demo.py` (numpy-only, self-checking against
`compat/golden.json`). Every identity below is verified there to 1e-12.*

## Thesis

The cell-ledger's `imbalance` and the elephant's field-edge are two
projections of **one object**: the directed edge `Δ = after − before`.
The ledger reads its **norm** (unsigned surprise, hash-sealed into every
entry); the field reads its **direction** (`d_mu`, signed `d_warmth`) and
its **length** (radial / κ). Double-entry bookkeeping is the field-edge at
cell grain; the fleet is a fractal JEPA because every cell is a room with
its own first-person edge.

## The mapping

| ledger field (ledger.rs / quilt-compat/1) | field-edge (elephant vmf.py) | relation |
|---|---|---|
| `delta` = `after − before` (vector) | `Δμ̂ = μ̂_a − μ̂_b` (raw edge) | **identical object** |
| `imbalance` = `‖Δ‖₂` (wire spec, op_d) | `d_mu = ‖μ̂_a − μ̂_b‖₂` | **identical number** iff state is a unit direction (identity 4); otherwise imbalance ⊇ d_mu (identity 1) |
| `expected` under persistence prior (`predict(b)=b`) | zero-order field model | the *trivial* JEPA predictor: identity map in representation space |
| `imbalance = dist(expected, output)` | JEPA loss | surprise = prediction error, at cell grain |
| first entry: `imbalance: null` (no prior) | `κ = None` under N < 10; `real: null` deadband | same honesty gate — never fake a number |
| `provenance`, hash `chain` | — | field-edge has neither; the seal is what makes surprise un-gameable training data |
| — | `d_warmth = ŵ·Δμ̂` | **signed valence — the ledger discards it** (identity 2 recovers it) |
| — | `κ`, `ρ`, `d_log_kappa` | ledger state is a *point*; field state is a *distribution* (vMF) |

## Structurally identical

1. **The unit of perception is the edge**, first-person: `(before → after)`
   recorded by the thing that changed, not an observer.
2. **Persistence prior = zero-order JEPA.** `imbalance = ‖a − p(b)‖`; with
   `p = id`, `imbalance = ‖Δ‖` — surprise *is* the edge. A learned predictor
   only generalizes `p`; the ledger already accepts it (`record_with(expected)`)
   and **seals the forecast before the outcome exists**.
3. **L2 collapses to d_mu on the sphere** — same number, bit-for-bit.
4. **Honesty gates coincide**: null-prior ↔ deadband/NMIN.

## Where they genuinely differ

1. **Unsigned vs signed.** `imbalance` discards valence; `d_warmth` keeps it.
2. **Point vs distribution.** The ledger's `imbalance` conflates radial
   (magnitude / κ) and directional (μ̂) drift into one scalar; identity 1
   splits them exactly.
3. **Raw vs representation space.** The ledger seals `expected` in raw value
   space and is predictor-agnostic; JEPA predicts in embedding space
   (EMA + stop-grad + VICReg). Plumbing vs predictor — complementary, not rival.
4. **Chain.** No hash chain on the field-edge; without one, a transition log
   is not yet honest training data.

## The bridge identities (golden vector `room.field`, exact to 1e-12)

`before=[0.25,−0.125,0.5]`, `after=[0.375,−0.0625,0.625]`,
`Δ=[0.125,0.0625,0.125]`, `imbalance=0.1875` (golden: bit-for-bit),
`cos=+0.9881`, `d_mu=0.1542`, `d_warmth=+0.1112` (room warmed),
`radial=ln(‖a‖/‖b‖)=+0.2446` (field grew):

1. `imbalance² = (‖a‖−‖b‖)² + ‖a‖‖b‖·d_mu²` — surprise splits into magnitude
   drift + direction drift. *This edge: 71.6% radial (κ's side), 28.3%
   directional — the scalar imbalance hides which; the field tells you.*
2. `imbalance² = (ŵ·Δ)² + ‖Δ⊥‖²` — Pythagoras on the raw edge: warmth is one
   **signed leg** of the ledger's own surprise.
3. `|d_warmth| ≤ d_mu ≤ imbalance` — the projection chain.
4. `‖before‖=‖after‖=1 ⟹ imbalance ≡ d_mu` — the unit-cell collapse.

## Three next moves

1. **Add the signed leg to the wire edge.** `warmth = ŵ_cell·Δ` as an optional
   op_d field, with per-cell configured `ŵ` (elephant's `WARM` for room
   cells). One field, one golden vector, identity 2 as a conformance
   invariant — the ledger stops discarding valence.
2. **Make identity 4 contractual.** Add a unit-sphere edge to `golden.json`
   asserting `imbalance == d_mu` bit-for-bit; the bridge becomes part of
   `quilt-compat/1` and is tested automatically by every future port (TS,
   Python, ESP32) the day it lands.
3. **Close the learned loop.** Feed elephant's JEPA forecast (EMA predictor
   over μ̂) into a `room.field` ledger cell as `expected`; then
   `reconcile().mean_surprise` *is* the JEPA learning curve — hash-sealed,
   impossible to rewrite after outcomes. First real evidence for the
   fractal-JEPA claim (fleet-as-fractal-jepa.md, next move 3).
