# GENERATIONS — Self-Improving Quilts (Liquid models × quilt runtime)

Started 2026-08-31 ~19:40 AKDT. Mutator: Liquid-LFM2.5-2.6B. Trier: lfm2.5-1.2b-instruct.
Runtime: quilt-core @ caf8c3d (worktree branch `selfimprove-harness`).

Legend: `[KEPT]` = valid + kills new live mutant(s). Verdicts are objective
(clean-engine expectation match + mutant divergence). Modes alternate
blind (no bug info) / targeted (mutant diffs shown).

## Generation 0 — seed population

- 22 seed scenarios (arithmetic, precedence, floats, nulls, strings, dotted ids,
  '=' prefix, helpers, reactivity, call-cache staleness, chains).
- Expectations observed from the clean engine — fixture truth by construction.
