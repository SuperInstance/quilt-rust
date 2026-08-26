# Cohesion, Fascia, v\*, REG-1 — the tissue layer in quilt-rust vocabulary

*2026-08-26 · companion to [cell-ledger.md](cell-ledger.md),
[field-edge-ledger-bridge.md](field-edge-ledger-bridge.md),
[bridge-cell-ledger.md](bridge-cell-ledger.md).*

Four terms from the **elephant** research corpus
(`memory/research-cohesion-2026-08-21.md`,
`memory/fascia-bridge-2026-08-21.md`, both filed 2026-08-21) describe a
layer **between** cells — the connective tissue that reads every cell's
first-person edge. Zero code in this repo implements them, and zero fleet
repos adopt them, because until now they only existed in elephant
vocabulary. This page is the translation: each concept restated as
**types you already have, fields you already seal, and functions you can
write tomorrow** against `quilt_core`. Every symbol below is real; file
paths and line numbers refer to commit `306d73d`.

> **Provenance of the numbers.** Every constant quoted here (λ\* ≈
> 0.15–0.27, COH 0.14–0.36, cos(W, v\*) = 0.14, …) is an *empirical
> prior measured on the elephant corpus*, registered 2026-08-21. They are
> calibration for your deadbands, not laws. The recipes below are how you
> re-measure them on your own fleet.

---

## 1. The four concepts, one line each — and their quilt-rust seat

| concept | one line | quilt-rust seat |
|---|---|---|
| **Fascia** | the layer *between* cells that carries each cell's sealed first-person edge | the read side of a `HashMap<CellId, CellLedger>` (`packages/core/src/ledger.rs:546`) + the engine's `subscribe_all()` seam (`packages/core/src/engine.rs:494`) |
| **COH** (cohesion) | how far the whole roster moved *as one* at a boundary: `‖ĉ‖ / corpus_sd` | computed over `Replay.state` cuts (`ledger.rs:905`) — read-side fusion, never written back |
| **v\*** (the cohesion field) | the one direction in cell-state space that is *shared across cells* rather than idiosyncratic: leading generalized eigenvector of `(C_common, C_pers)` | the aggregation axis of a fascia reader; projects onto the same `Delta.before/after` arrays the ledger already seals (`ledger.rs:375`) |
| **REG-1** | the measurement verdict that disciplines the other three: the shared channel is participation-energy (volume+/presence−), *not* valence; warmth is a per-cell trait; the tissue signal is small (λ\* < 1) | an annotation contract + deadband law on any cell whose job is cross-cell aggregation |

The one-paragraph version: a `CellLedger` entry already records the
before→after edge (`delta`), the forecast it was scored against
(`expected`, sealed), and the surprise (`imbalance`) —
[cell-ledger.md](cell-ledger.md) calls this the cell's autobiography.
Fascia is the *reader* of N of those autobiographies at once. COH is one
number it computes per boundary (did the school translate together). v\*
is the axis it projects motion onto (which direction counts as
"together"). REG-1 is the guard that keeps it honest (don't read the
tissue as a mood ring).

---

## 2. Fascia — the seam, in types that exist today

Fascia's thesis (elephant, verbatim): *JEPA + DoubleEntry as inter-cell
connective tissue* — the layer between cells that carries each cell's
first-person edge (`before→after`, sealed `expected`, `imbalance` =
surprise) and lets a school of cheap cells behave as one organism.

Every word of that maps onto something already in this crate:

| fascia claim | quilt-rust fact |
|---|---|
| "each cell's first-person edge" | `LedgerEntry.delta` — `Delta { before, after, changed, magnitude }` (`ledger.rs:375`) |
| "sealed expected" | `LedgerEntry.expected: Option<Value>` is **hashed into `entry.hash`** (`ledger.rs:402` doc comment: *"forecasts cannot be rewritten later"*) |
| "imbalance = surprise" | `LedgerEntry.imbalance: Option<f64>` = `value_distance(expected, output)` (`ledger.rs:258`) |
| "JEPA" | the persistence prior *is* the null predictor: `record_with(..., None)` scores `expected = before`, so `imbalance == delta.magnitude` (`append_entry`, `ledger.rs:758`). A learned predictor that beats it by >15% is the registered falsification bar — elephant's rule, directly applicable |
| "the layer between cells" | nothing: `QuiltEngine` keeps `cells`; [cell-ledger.md §9](cell-ledger.md) proposes `ledgers: RwLock<HashMap<CellId, CellLedger>>` beside them. Fascia = the **read-only aggregator over that map** |

**The structural guarantee fascia needs is already here: seal-first /
fuse-second.** Elephant's Ask 2 was *"how does the tissue seal a
q-thresholded loss without look-ahead laundering?"* The answer falls out
of the type layout: each `CellLedger` seals every entry at append time
(`append_entry` → `entry.hash = entry.seal()`), and *nothing in this
crate writes back into a ledger after the fact*. So as long as tissue
outputs (COH, q, v\*-projections) go to **their own cell** — never into a
member cell's `record_with` — the roster mean can never launder a
forecast. The coordinate firewall is a naming discipline: tissue cells
and member cells are disjoint sets.

```rust
use quilt_core::{CellLedger, LedgerOrigin, Provenance};
use serde_json::json;

// The member cell: one ledger, sealed edges. (Elephant's CellLedgerProducer
// is exactly this shape — see bridge-cell-ledger.md, vocabulary row 1.)
let mut ledger = CellLedger::with_genesis(
    "room.field.warmth",
    json!([0.25, -0.125, 0.5]),   // genesis state
    1_000,
);
let entry = ledger.record_with(
    json!({"tick": 1_060}),           // input posting (debit)
    json!([0.375, -0.0625, 0.625]),   // output posting (credit) = new state
    1_060,
    Provenance {
        origin: LedgerOrigin::Push,   // remote read, per bridge-cell-ledger.md
        caller: Some("crab-traps.relay".into()),
        trace: vec![],
    },
    None,   // persistence prior: expected = before; surprise = edge magnitude
);

// The fascia half: read the sealed edge, project it — never write it back.
let d: Vec<f64> = entry.delta.after.as_array().unwrap().iter()
    .zip(entry.delta.before.as_array().unwrap())
    .map(|(a, b)| a.as_f64().unwrap_or(0.0) - b.as_f64().unwrap_or(0.0))
    .collect();
assert_eq!(d, vec![0.125, 0.0625, 0.125]);           // Δ, the golden room.field edge
assert_eq!(entry.imbalance, Some(0.3125 / 3.0));     // mean-L1, see note below
assert!(ledger.verify_chain().intact);                // the tissue only reads verified chains
```

**Metric alert (read this before wiring anything).** `value_distance` on
arrays is the **mean per-coordinate distance** (`ledger.rs:264-276`), not
the L2 norm. On the golden `room.field` edge above, the ledger's
`imbalance` is `0.3125/3 ≈ 0.10417` (mean-L1), while
[field-edge-ledger-bridge.md](field-edge-ledger-bridge.md) quotes `0.1875`
for the *same* vector — that number is ‖Δ‖₂. Both are "the surprise of
this edge"; they differ by a constant-ish factor of √n only when
components are equal, and generally differ more. **Rule: the ledger's own
`imbalance` is mean-L1; the elephant's `d_mu` is L2; a fascia reader that
mixes them silently will mis-scale every deadband.** Compute your L2 from
`delta.before/after` in the reader (§3's `step_over` is the extractor)
when you need the field-edge convention.

---

## 3. COH — cohesion at cell grain

**Definition (elephant, registered COH-v1).** At a boundary `b` with
window `W`, per member cell take the step

```
δ_R = state_R(b+W−1) − state_R(b−1)          (per-cell step over the window)
ĉ   = mean_R δ_R                              (the common shift — the roster's step)
r_R = δ_R − ĉ                                 (residual — who didn't move with it)
COH = ‖ĉ‖₂ / corpus_sd                        (collective translation magnitude)
q   = RMS_R(r_R) / RMS_R(o_pre)               (purity — how much motion is common)
```

In quilt-rust, the window cut is exactly `replay(until_ts)` — it returns
the reconstructed `state` at any past timestamp with no leakage past the
cut (`ledger.rs:905`). The **common-roster guard** (elephant: entries and
exits must not move `ĉ` by composition) is a filter on which cells you
average: only cells with entries in *both* the pre and post windows.

```rust
use quilt_core::{CellId, CellLedger};

/// The per-cell step over a [pre_cut, post_cut] window, straight from replays.
/// `None` ⇒ cell has no vector state in both cuts — drop it from the roster.
fn step_over(ledger: &CellLedger, pre_cut: u64, post_cut: u64) -> Option<Vec<f64>> {
    let pre  = ledger.replay(pre_cut).state;
    let post = ledger.replay(post_cut).state;
    match (pre.as_array(), post.as_array()) {
        (Some(p), Some(q)) if p.len() == q.len() && !p.is_empty() => Some(
            q.iter().zip(p.iter())
                .map(|(a, b)| a.as_f64().unwrap_or(0.0) - b.as_f64().unwrap_or(0.0))
                .collect(),
        ),
        _ => None,
    }
}

/// COH and purity q for one boundary. Roster = cells stepping in BOTH cuts.
/// `offsets_pre` = each cell's stable idiosyncratic offset (personality
/// fiber), estimated on a slower clock than the tissue (elephant seam 3b).
fn cohesion_at_boundary(
    ledgers: &[(CellId, CellLedger)],       // the member roster
    b: u64, w: u64,                          // boundary, window
    corpus_sd: f64,
    offsets_pre: &[(CellId, Vec<f64>)],
) -> Option<(f64, f64, Vec<f64>)> {          // (COH, q, ĉ) — ĉ is the direction log
    let steps: Vec<Vec<f64>> = ledgers.iter()
        .filter_map(|(_, l)| step_over(l, b.wrapping_sub(1), b + w - 1))
        .collect();
    if steps.len() < 2 { return None; }      // a school needs ≥2 fish
    let n = steps[0].len();

    // ĉ = roster-mean step (the school's velocity vector, in event time).
    let c_hat: Vec<f64> = (0..n).map(|i|
        steps.iter().map(|s| s[i]).sum::<f64>() / steps.len() as f64
    ).collect();

    // r_R = δ_R − ĉ.
    let residuals: Vec<Vec<f64>> = steps.iter()
        .map(|s| (0..n).map(|i| s[i] - c_hat[i]).collect())
        .collect();

    let norm = |v: &[f64]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let coh = norm(&c_hat) / corpus_sd;

    let rms = |x: &[f64]| (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
    let r_flat: Vec<f64> = residuals.iter().flatten().copied().collect();
    let o_flat: Vec<f64> = offsets_pre.iter().filter_map(|(_, o)|
        (!o.is_empty()).then(|| o.clone())).flatten().collect();
    let q = if o_flat.is_empty() { f64::NAN } else { rms(&r_flat) / rms(&o_flat) };

    Some((coh, q, c_hat))
}
```

**Calibration (elephant priors, 2026-08-21 corpus).** Signal boundaries
measured COH 0.14–0.36 `corpus_sd` (2.5× dynamic range); rest floor
modelled at σ_ε/√(W·n) ≈ 0.03–0.05 but **never measured** — measuring it
is the experiment. Purity: hard flips q ≈ 0.079–0.133, roster-entry
boundaries 0.135–0.204 (the entrant hasn't synced; its residual is the
roster's largest). **Verdict rule (import verbatim):** COH_signal > 2×
COH_rest with CI separation in both waves → the school is a real object;
overlap → the "common motion" is schedule drift and any dashboard
rendering it is a mood ring. Register `cos(ĉ, v*)` beside every COH
claim — the school's synchronized motion ran *off* the valence axis
(cos = 0.147 on the elephant corpus).

**Where COH output goes.** Its own cell — `engine.push("room.coh",
json!({...}))` from a fascia task holding the engine's
`subscribe_all()` handle (`engine.rs:494`). Never into a member ledger:
that would feed the roster mean back, breaking seal-first/fuse-second
(§2).

---

## 4. v\* — the tissue channel (the "cohesion field")

**Definition (elephant REG-1 / Gift 1).** In the shared cell-state
space, form two covariances over the sealed step history:
`C_common` (across-time, within-cell — how a single cell moves) and
`C_pers` (across-cell — how cells differ from each other, the
personality fiber). The **tissue channel** v\* is the leading
generalized eigenvector of `C_common · v = λ · C_pers · v`: the
direction maximizing shared response per unit of idiosyncrasy. Elephant
measured it once: v\* ≈ volume(+)/presence(−) — *participation energy*,
how much each cell is producing vs how attentively present — with
λ\* ≈ 0.15–0.27 (the tissue is a **small perturbation riding a large
per-cell field**).

In quilt-rust: v\* is **reader configuration, not ledger state**. The
ledger stays metric-agnostic (it seals raw `delta` values; §5 of
[cell-ledger.md](cell-ledger.md) — "one distance metric… specialized
edges are the cell's business"). v\* lives in the fascia reader as a
registered `Vec<f64>` beam, and every tissue read is a projection onto
it — the exact shape of the already-proposed `warmth = ŵ_cell·Δ` wire
field ([field-edge-ledger-bridge.md](field-edge-ledger-bridge.md),
next-move 1: *the ledger stops discarding valence*). v\* generalizes
`ŵ` from "one hand-picked axis" to "the measured shared axis".

```rust
/// Assemble the two covariance matrices from sealed ledger steps.
/// Hand the result to your eigensolver (quilt-core has none by design —
/// bring nalgebra/ndarray in the *reader* crate, not in core).
fn tissue_covariances(steps_per_cell: &[Vec<Vec<f64>>]) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    // steps_per_cell[cell][k] = the k-th sealed step of that cell.
    // C_common: covariance of single-cell steps pooled over time (within-cell).
    // C_pers:   covariance of per-cell mean states (across-cell).
    //   — build exactly as in §3's step_over: mean of deltas per cell, etc.
    // (Assembly is plain f64 loops; the eigenproblem is one
    //  nalgebra::GeneralizedEigenAnalysis away. Kept schematic: the point
    //  is WHICH sealed quantities feed it — only replay cuts, never live
    //  unsealed state.)
    todo!("see recipe 3 — inputs are Replay cuts only")
}

/// The tissue read itself: project a sealed edge onto the registered beam.
/// d_vstar > 0 ⇒ the cell moved WITH the tissue; the signed leg the
/// scalar imbalance discards (bridge identity 2).
fn tissue_read(v_star: &[f64], entry: &quilt_core::LedgerEntry) -> Option<f64> {
    let (a, b) = (entry.delta.after.as_array()?, entry.delta.before.as_array()?);
    let step: Vec<f64> = a.iter().zip(b.iter())
        .map(|(x, y)| x.as_f64().unwrap_or(0.0) - y.as_f64().unwrap_or(0.0))
        .collect();
    let norm = step.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    Some(v_star.iter().zip(&step).map(|(v, s)| v * s).sum::<f64>() / norm)
}
```

**Calibration.** λ\* ≈ 0.15–0.27 ⇒ size tissue deadbands at
`0.27 · corpus_sd` ceilings, not at `corpus_sd`: a fascia cell that
alerts at full-cell-grain magnitude will never fire on real tissue
signal (it's 4–7× smaller), and one that fires easily will drown in
per-cell noise. cos(valence-axis, v\*) = 0.14 on the elephant corpus:
warmth/valence motion is nearly **orthogonal** to the tissue channel —
which is why §5's annotation guard exists.

---

## 5. REG-1 — the annotation contract and the deadband law

REG-1 is elephant's registered verdict, and it translates to three rules
for any cross-cell aggregate you build on this crate:

1. **The dual-annotation guard.** The elephant instrument fused energy
   with valence and "read a mood ring as a thermometer". Any valence-ish
   tissue output must carry *both* cosines: alignment to the tissue
   channel v\*, and alignment to the cell-personality axis (per-cell
   PC1). Expect the first small (~0.14) and the second large (~0.98) —
   if your aggregate shows the opposite, it is measuring the *mix of
   cells present*, not the tissue.

```rust
/// REG-1 guard for one tissue output. Ship both cosines with every read.
struct FasciaAnnotation { cos_to_tissue: f64, cos_to_fiber: f64 }

fn annotate(axis: &[f64], v_star: &[f64], pc1_pers: &[f64]) -> FasciaAnnotation {
    let cos = |a: &[f64], b: &[f64]| {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na = a.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
        let nb = b.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
        dot / (na * nb)
    };
    FasciaAnnotation { cos_to_tissue: cos(axis, v_star), cos_to_fiber: cos(axis, pc1_pers) }
    // REG-1 verdict shape: cos_to_fiber >> cos_to_tissue means the output
    // is roster composition, not tissue. Elephant priors: 0.14 vs 0.98.
    // Treat a valence jump at roster-change as composition until the
    // §3 decomposition (ĉ vs r_R) says otherwise.
}
```

2. **The deadband law.** Tissue signal ≈ λ\*·(per-cell field), λ\* ≤
   0.27. A `listener` cell watching a tissue aggregate needs its
   `condition` threshold scaled accordingly — `CellDef.condition`
   (`packages/core/src/types.rs:562`) is where it lands on a sheet.

3. **Entry semantics (the κ-verdict).** When a new cell joins the
   roster: the collective **re-aims** (direction event at full step
   magnitude) while agreement **loosens** (Δlog κ = −0.32 at entry vs
   −0.75 at hard flips, elephant corpus). Do not model entry as a spike
   or tightening; expect elevated q for a window before the entrant
   syncs (§3 priors), and keep the common-roster guard active so entry
   can't masquerade as a tissue event.

---

## 6. Recipes

### Recipe 1 — crab-traps relay: a COH panel over D1 `ledger_edges`

The crab-traps relay already buffers sealed edges in D1 (table
`ledger_edges`, `POST /edge` → `chain_head`, `GET /queue?since=` —
[bridge-cell-ledger.md](bridge-cell-ledger.md)). Tomorrow, without
touching the relay contract:

1. Drain `GET /queue?since=` into per-cell `CellLedger`s on your side —
   each row is a pre-sealed `push`-origin entry; rebuild with
   `record_with(input, after, ts, Provenance { origin:
   LedgerOrigin::Push, caller: Some("crab-traps.relay".into()), ..},
   None)` (elephant field reads carry `expected: null`; the relay is the
   sealing authority, you are re-deriving its chain for verification —
   `verify_chain()` must return `intact` before any tissue math).
2. Maintain `corpus_sd` over the sealed `delta.magnitude` history.
3. Per boundary, run `cohesion_at_boundary` (§3); post the result back
   through the relay as its **own cell** (`room.coh`), not into member
   cells.
4. Render COH(t) as the school's speed profile: near-floor between
   events, spikes at boundaries — with the rest-window floor measured,
   not assumed (that's the open elephant number; your fleet's answer is
   publishable).

### Recipe 2 — cell-cascade / embedded: the live fascia loop

For an author building cascades inside one engine (no relay in the
middle):

1. `let handle = engine.subscribe_all();` (`engine.rs:494`) — one
   subscription feeds the whole tissue.
2. A fascia task drains the handle: each
   `SubscriptionEvent { cell_id, prev_value, new_value }`
   (`engine.rs:806`) becomes one `record_with` on that cell's
   `CellLedger` — `prev_value.data` is your `before` (best-effort in the
   MVP; prefer the ledger's own `delta.before` once §9 engine wiring
   lands).
3. On a boundary clock (or a designated `listener` cell firing), run
   `cohesion_at_boundary` over the ledgers; `engine.push("room.coh",
   json!({"coh": …, "q": …}))` publishes the tissue cell.
4. Discipline: member cells and tissue cells are disjoint ids. The
   engine does not enforce this yet — the firewall is your naming
   convention (and the day `ledgers:` wiring lands in `QuiltEngine`, it
   becomes enforceable in one place).

### Recipe 3 — registering v\* and running the guard

1. Collect `steps_per_cell` from `replay` cuts only (§4) — sealed
   history, no live unsealed state.
2. Assemble `C_common`/`C_pers`, solve the generalized eigenproblem in
   your reader crate (nalgebra et al.); freeze v\* for the analysis
   window (elephant: the encoder is **frozen** — anchor, don't EMA).
3. Register v\* beside the tissue cell (`room.vstar`) so every consumer
   reads the same beam; log λ\* so deadband scaling is auditable.
4. Wrap every valence-flavored output with `annotate` (§5) and ship both
   cosines in the payload.

---

## 7. Vocabulary map (extend bridge-cell-ledger.md's table)

| elephant term | quilt-rust concept | what differs |
|---|---|---|
| fascia (the tissue) | read-only aggregator over `HashMap<CellId, CellLedger>` + `subscribe_all()` | doesn't exist yet as a type; §9 wiring is its landing spot — the *concept* needs no core changes |
| `COH = ‖ĉ‖/corpus_sd` | reader function over `Replay` cuts | per-cell steps from `delta.before/after`; mean-L1 `value_distance` ≠ L2 `d_mu` — pick your norm consciously (§2 note) |
| `q` (purity) | `RMS(r_R)/RMS(o_pre)` beside COH | needs the roster-mean, which **must not** exist before seals — satisfied structurally: fusion is read-side only |
| `ĉ` (common shift) | the roster-mean step vector | publish as the tissue cell's direction log; annotate with `cos(ĉ, v*)` |
| v\* (tissue channel) | registered beam `Vec<f64>` in the fascia reader | not core state — the ledger stays metric-agnostic; generalizes the proposed `warmth` wire field |
| REG-1 annotation | `FasciaAnnotation { cos_to_tissue, cos_to_fiber }` | ship with every aggregate; priors 0.14 / 0.98 |
| λ\* deadband law | `0.27 · corpus_sd` ceiling on tissue alert thresholds | lands in `CellDef.condition` for listeners |
| seal-first / fuse-second | `append_entry` seals at record time; nothing writes back | becomes enforceable when §9 lands; until then it's id-set discipline |

## 8. Honest scope

- **Nothing here changes core.** All recipes are reader-side against
  `CellLedger`'s public API; the engine-wiring joint remains §9 of
  [cell-ledger.md](cell-ledger.md).
- **No linear algebra in quilt-core.** v\* estimation happens in your
  crate; core's `value_distance` (mean-L1 on arrays) is generic on
  purpose — don't conflate it with the elephant's L2 `d_mu`.
- **Elephant's numbers are priors, not ports.** COH range, q families,
  λ\*, and the 0.14/0.98 cosines come from one corpus, registered
  2026-08-21 with a ~30% pre-registered chance COH collapses back to
  confound. Re-measure before trusting.
- **The one open protocol question** (elephant Ask 2, answered §2):
  sealing order is satisfied *structurally* by this repo's shapes — but
  only while tissue outputs stay in their own cells. That discipline is
  currently a convention, not a type. If it ever needs teeth, the
  `ledgers:` map is the place to put them.

*Docs-only translation — no code changed. The concepts were measured in
the elephant's vocabulary (`memory/research-cohesion-2026-08-21.md`,
`memory/fascia-bridge-2026-08-21.md`); this page is their door into
quilt-rust, crab-traps, and cell-cascade.*
