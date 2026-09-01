# Self-Improving Quilts — Liquid-model mutation loop (night of 2026-08-31)

First night the local GPU (RTX 4050, Ollama) works on quilts instead of journals.

## Thesis

A 2.6B local model (Liquid-LFM2.5-2.6B) + a 1.2B triager (LiquidAI/lfm2.5-1.2b-instruct)
can drive a real self-improvement loop against the quilt runtime, IF the mutation
surface is scoped to things an objective battery can verdict.

## Design

The mutable surface is the **test corpus** (scenarios: cells + ops + expectation).
The fitness landscape is a **fixed suite of handcrafted engine mutants**
(`mutants.json`): each is a small, realistic sabotage of `packages/core`
(rewrite ordering, '=' stripping, cache invalidation on set/propagate, chained-formula
snapshots, helper semantics, dependency wiring, string-literal rewriting).

- **POPULATE** — corpus seeded with 22 scenarios covering arithmetic, precedence,
  floats, nulls, strings, dotted ids, '=' prefix, helpers, reactivity, caching.
  Seed expectations are *observed from the clean engine* (fixture truth, not claims).
- **MUTATE** — LFM2.5-2.6B proposes candidate scenarios. Generations alternate
  **blind** (spec + corpus examples only) and **targeted** (shown the live mutants'
  diffs and asked to detect them).
- **FILTER** — lfm2.5-1.2b-instruct triages each candidate (keep_for_eval + reason).
  Default-pass on triager failure so the loop stays live.
- **EVALUATE** — candidates run on the clean probe (real `QuiltEngine`). Valid =
  model's `expect` matches engine reality (numeric-tolerant; `"ERROR"` matches a
  failing step). Validity is the model *correctly predicting* the engine.
- **VALUE / KEEP** — a valid candidate is KEPT iff it kills ≥1 *live* mutant
  (battery-green under mutation) that the current corpus does not. Kills are
  measured as divergent `reads` (or error/panic) on the mutant probe binary.
- **LOG** — `GENERATIONS.md` is the human ledger; `results/gen-N.json` the machine
  one; every generation commits to this branch (`selfimprove-harness`).

## Honest metrics

- Battery kill per mutant: `cargo test -p quilt-core` with the mutant applied
  (measured once at setup, cached in `setup.json`).
- Improvement = live mutants killed by the corpus, over generations.
- Model-prediction accuracy = valid candidates / triaged candidates.
- Negative results (wrong expectations, no-kill cases, JSON failures) are
  first-class citizens of the log.

## Safety

Worktree `../quilt-rust-selfimprove`, branch `selfimprove-harness`, never pushed.
Mutant application always reverts in `finally`. Probe builds are standalone
(empty `[workspace]`) so the quilt workspace tree stays pristine for battery runs.
`subprocess` list-form only.

## Reproduce

```
python3 harness.py setup          # build clean+mutant probes, battery baseline
python3 harness.py run            # generations (MAX_GENS, MAX_WALL_S env)
python3 harness.py status         # corpus size, live mutants, kills
```
