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

## Generation 1 — blind — 2026-08-31 19:50:41
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 2 — CRASHED: 'desc'

## Generation 2 — targeted — 2026-08-31 19:52:56
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g2-0` [REJECTED-filter] 'compass.heading' — triage=False (short reason) —  
- `g2-1` [REJECTED-filter] 'a + 100' — triage=False (The cache_result() does not update cell.value after a set, causing the formula t) —  
- `g2-2` [REJECTED-filter] 'min([arr])' — triage=False (The min() array helper incorrectly computes the maximum instead of the minimum d) —  
- `g2-3` [REJECTED-filter] 'a + 100' — triage=False (The test case incorrectly expects a value of 120 despite the dependency logic ca) —  

## Generation 3 — blind — 2026-08-31 19:55:35
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g3-0` [REJECTED-filter] 'a * b' — triage=False (Multiplication of 2147483647 * 2 exceeds the 32-bit signed integer range, trigge) —  
- `g3-1` [REJECTED-filter] 'a + b' — triage=False (Null arithmetic is undefined behavior and should trigger an error.) —  
- `g3-2` [REJECTED-filter] 'cells["a"] + cells["b"]' — triage=False (short reason) —  
- `g3-3` [REJECTED-filter] 'min([a, b])' — triage=False (The test case verifies correct usage of min/max on array literals with a script ) —  

## Generation 3 — blind — 2026-08-31 19:55:35
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g3-0` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine succeeded, model expected error 
- `g3-1` [REJECTED-invalid-expectation] 'a / b' — triage=False (Division by zero detected and the engine treats it as an error) — engine succeeded, model expected error 
- `g3-2` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine succeeded, model expected error 
- `g3-3` [REJECTED-no-new-kills] 'min([x, y, z])' — triage=False (The test checks array min logic but the engine rejected it due to value mismatch) — match 

## Generation 4 — targeted — 2026-08-31 19:59:50
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 4 — targeted — 2026-08-31 20:02:16
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 5 — blind — 2026-08-31 20:07:00
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g5-0` [REJECTED-no-new-kills] '((a + b) * c) / 2' — triage=False (short reason) — match 
- `g5-1` [REJECTED-no-new-kills] '' — triage=False (short reason) — match 
- `g5-2` [REJECTED-invalid-expectation] 'a / b' — triage=False (Dividing 5 by 0 is undefined; the engine defines this as an ERROR.) — engine succeeded, model expected error 
- `g5-3` [REJECTED-invalid-expectation] 'a * b + c' — triage=False (multiplying 3000000 * 4000 causes integer overflow) — engine succeeded, model expected error 

## Generation 5 — blind — 2026-08-31 20:07:01
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g5-0` [REJECTED-filter] '' — triage=False (short reason) —  
- `g5-1` [REJECTED-filter] '' — triage=False (short reason) —  
- `g5-2` [REJECTED-filter] '' — triage=False (short reason) —  
- `g5-3` [REJECTED-filter] '' — triage=False (short reason) —  
