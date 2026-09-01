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

## Generation 9 — blind — 2026-08-31 20:58:12
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g9-0` [REJECTED-invalid-expectation] 'a / b' — triage=False (short reason) — engine=null model_expect="NULL" 
- `g9-1` [REJECTED-invalid-expectation] '(a > 2 && b < 4) || c' — triage=False (short reason) — engine=null model_expect="true" 
- `g9-2` [REJECTED-invalid-expectation] '' — triage=False (The test checks clamp logic with specific bounds; however, the scenario describe) — engine errored on case 
- `g9-3` [REJECTED-invalid-expectation] 'cells["start"] + "-" + cells["mid"] + "-" + cells["end"]' — triage=False (short reason) — engine="--" model_expect="begin-middle-finish" 

## Generation 9 — blind — 2026-08-31 20:58:12
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g9-0` [REJECTED-filter] '' — triage=False (short reason) —  
- `g9-1` [REJECTED-filter] '' — triage=False (short reason) —  
- `g9-2` [REJECTED-filter] '' — triage=False (Tests clamp helper function returning the lower bound when value exceeds the upp) —  
- `g9-3` [REJECTED-filter] '' — triage=False (short reason) —  

## Generation 11 — blind — 2026-08-31 21:03:28
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g11-0` [REJECTED-invalid-expectation] 'a * 2' — triage=False (short reason) — engine=75 model_expect=60 
- `g11-1` [REJECTED-invalid-expectation] 'a + " " + b' — triage=False (short reason) — engine=null model_expect="hello more" 
- `g11-2` [REJECTED-no-new-kills] 'a / b' — triage=False (the expected outcome should not be NULL due to reactive evaluation) — match 
- `g11-3` [REJECTED-no-new-kills] 'a + 1' — triage=False (short reason) — match 

## Generation 10 — targeted — 2026-08-31 21:03:29
- live mutants: 8 | corpus kills before: 3 | after: 3
- `g10-0` [REJECTED-no-new-kills] 'compass * 2' — triage=False (shortest-first rewrite order corrupts dotted-id references; longest-first ensure) — match 
- `g10-1` [REJECTED-no-new-kills] 'a + 100' — triage=False (The test case describes a scenario that is syntactically valid but does not full) — match 
- `g10-2` [REJECTED-no-new-kills] 'x + 10' — triage=False (The test case describes a scenario that already exists; it is not a duplicate or) — match 
- `g10-3` [REJECTED-no-new-kills] 'min([arr, arr2])' — triage=False (The min() array helper should return the minimum (7). If it incorrectly computes) — match 

## ⚠️ Incident log (21:10)

Generations 1-10 ran with up to SIX concurrent harness processes (process-kill
requests did not propagate to the python children). Effects: serial Ollama queue
contention (7-15 min generations), duplicate GENERATIONS.md entries, and
gen-9.json written by both old and new code. Corpus itself stayed consistent
(all runs kept 0 cases; kills=3 baseline intact — verified via `status`).
Fix: pkill + flock single-instance guard. Duplicate entries above are left
in place as the honest record.

## Generation 11 — blind — 2026-08-31 21:08:55
- live mutants: 8 | corpus kills before: 3 | after: 8
- `g11-0` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g11-1` [KEPT] '' — triage=False (short reason) — expect ERROR kills=M01-rewrite-order,M04-set-cache-clear,M05-cache-value-update,M07-stale-ready,M12-call-cache-serve-error
- `g11-2` [REJECTED-invalid-expectation] '' — triage=False (short reason) — engine errored on case 
- `g11-3` [REJECTED-invalid-expectation] 'a + b' — triage=False (short reason) — engine=null model_expect="NULL" 
- **KEPT: g11-1**

## Generation 12 — targeted — 2026-08-31 21:13:31
- live mutants: 8 | corpus kills before: 8 | after: 8

## ⚠️ RETRACTION — gen 11's g11-1 (21:25)

g11-1 claimed kills on M01/M04/M05/M07/M12 — **all false**. The case was
`get f` on a nonexistent cell: identical CellNotFound error on clean and every
mutant. Bug: `reads_diverge` credited ANY mutant error/panic as divergence.
Fixed: error-vs-error now requires the error itself to differ; g11-1 removed
from corpus (preserved in git history + results/gen-11.json). True kill
baseline returns to 3/8.
