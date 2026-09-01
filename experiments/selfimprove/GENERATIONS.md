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

## Generation 6 — targeted — 2026-08-31 20:11:15
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 6 — targeted — 2026-08-31 20:13:09
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 7 — blind — 2026-08-31 20:16:31
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g7-0` [REJECTED-invalid-expectation] 'x + y' — triage=False (short reason) — engine=null model_expect=-2147483648 
- `g7-1` [REJECTED-invalid-expectation] 'a / b' — triage=False (Division by zero and expected error handling) — engine succeeded, model expected error 
- `g7-2` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine succeeded, model expected error 
- `g7-3` [REJECTED-invalid-expectation] 'clamp(a, lo, hi)' — triage=False (short reason) — engine=null model_expect=50 

## Generation 6 — targeted — 2026-08-31 20:16:32
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g6-0` [REJECTED-invalid-expectation] 'compass * 3' — triage=False (short reason) — engine=6 model_expect="6" 
- `g6-1` [REJECTED-invalid-expectation] 'a * 2' — triage=False (short reason) — engine=10 model_expect="10" 
- `g6-2` [REJECTED-invalid-expectation] 'min([arr, arr2, arr3])' — triage=False (short reason) — engine=5.0 model_expect="5" 
- `g6-3` [REJECTED-invalid-expectation] 'a + 1' — triage=False (The test case exhibits a logic bug where dependent cells are incorrectly marked ) — engine=3 model_expect="Ready" 

## Generation 7 — blind — 2026-08-31 20:21:11
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g7-0` [REJECTED-filter] '' — triage=False (short reason) —  
- `g7-1` [REJECTED-filter] '' — triage=False (short reason) —  
- `g7-2` [REJECTED-filter] '' — triage=False (short reason) —  
- `g7-3` [REJECTED-filter] '' — triage=False (short reason) —  

## Generation 7 — blind — 2026-08-31 20:21:12
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g7-0` [REJECTED-invalid-expectation] 'a / b' — triage=False (short reason) — engine=null model_expect="NULL" 
- `g7-1` [REJECTED-invalid-expectation] 'min([c,d,e])' — triage=False (short reason) — engine=null model_expect=[1, 4] 
- `g7-2` [REJECTED-invalid-expectation] 'a + 100' — triage=False (short reason) — engine=null model_expect=120 
- `g7-3` [REJECTED-invalid-expectation] 'compass.heading + 10' — triage=False (short reason) — engine=null model_expect="52" 

## Generation 7 — blind — 2026-08-31 20:21:12
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g7-0` [REJECTED-invalid-expectation] 'a / b' — triage=False (short reason) — engine=null model_expect="5" 
- `g7-1` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g7-2` [REJECTED-invalid-expectation] 'a + b' — triage=False (The formula result is stale after the set operation because the cache hasn't bee) — engine=null model_expect="15" 
- `g7-3` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 

## Generation 8 — targeted — 2026-08-31 20:21:14
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g8-0` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g8-1` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine=10 model_expect="10" 
- `g8-2` [REJECTED-invalid-expectation] 'a + 100' — triage=False (short reason) — engine=null model_expect="102" 
- `g8-3` [REJECTED-invalid-expectation] '' — triage=False (The known-id rewriting should skip string literals. 'a' and 'b' are string value) — engine errored on case 

## Generation 8 — targeted — 2026-08-31 20:25:11
- live mutants: 8 | corpus kills before: 3 | after: 3

## Generation 9 — blind — 2026-08-31 20:42:26
- live mutants: 8 | corpus kills before: 3 | after: 0

## Generation 8 — targeted — 2026-08-31 20:43:33
- live mutants: 8 | corpus kills before: 3 | after: 0

## Generation 10 — targeted — 2026-08-31 20:57:42
- live mutants: 8 | corpus kills before: 3 | after: 0

## Generation 9 — blind — 2026-08-31 20:58:09
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g9-0` [REJECTED-invalid-expectation] 'a / 0' — triage=True (triager-failed (ollama chat failed (LiquidAI/lfm2.5-1.2b-instruct:latest): timed) — engine=null model_expect="NULL" 
- `g9-1` [REJECTED-no-new-kills] 'a * b' — triage=True (triager-failed (ollama chat failed (LiquidAI/lfm2.5-1.2b-instruct:latest): timed) — match 
- `g9-2` [REJECTED-no-new-kills] 'a + " " + b' — triage=False (short reason) — match 
- `g9-3` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine=null model_expect="NULL" 

## Generation 8 — targeted — 2026-08-31 20:58:11
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g8-0` [REJECTED-invalid-expectation] '' — triage=True (triager-failed (ollama chat failed (LiquidAI/lfm2.5-1.2b-instruct:latest): timed) — engine errored on case 
- `g8-1` [REJECTED-no-new-kills] '' — triage=False (short reason) — match 
- `g8-2` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g8-3` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine=null model_expect="stale" 

## Generation 9 — blind — 2026-08-31 20:58:12
- live mutants: 8 | corpus kills before: 3 | after: 8
- `g9-0` [REJECTED-no-new-kills] '' — triage=True (triager-failed (ollama chat failed (LiquidAI/lfm2.5-1.2b-instruct:latest): timed) — match 
- `g9-1` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g9-2` [KEPT] '' — triage=False (short reason) — expect ERROR kills=M01-rewrite-order,M04-set-cache-clear,M05-cache-value-update,M07-stale-ready,M12-call-cache-serve-error
- `g9-3` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- **KEPT: g9-2**
