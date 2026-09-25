# SELFIMPROVE NIGHT LOOP — CONTINUATION BRIEF (night 1, part 2)

You are the SELF-IMPROVING QUILTS lane, resuming a run that died mid-flight (~2h in, harness intact). Repo: /home/eileen/projects/quilt-rust, branch `selfimprove-harness` — REVIEW ITS LOG FIRST (`git log selfimprove-harness`) and CONTINUE from where it stopped (last commits through gen 12 + a retraction commit 8f5dd3d fixing the harness's own oracle bug: false kills via error-vs-error divergence, reads_diverge fixed, flock guard added).

Local models via Ollama (/home/eileen/.local/bin/ollama; start `ollama serve &` if needed): Liquid-LFM2.5-2.6B (agentic, proposes mutations), LiquidAI/lfm2.5-1.2b-instruct (fast triage). DeepSeek/DeepInfra REVOKED — local only, free all night.

LOOP (unchanged): POPULATION from current quilt state/tests → MUTATE via LFM2.5-2.6B (scope: things the test battery can verdict — parameters, rule tweaks, test-case variants; no blind core rewrites) → FILTER via 1.2b → EVALUATE with the existing battery, objective verdict only → KEEP/REJECT as individual commits with generation numbers → append every generation to experiments/selfimprove/GENERATIONS.md (mutation, filter verdict, eval result, keep/reject reason).

The corpus-kills metric is the one that moved: 3→8 at gen 11 blind, then 8→8 (kept 0), with one retraction proving the loop polices itself. Note the pattern the log already shows: blind generations found test cases, targeted generations mostly didn't — investigate WHY and adjust the mutation surface accordingly (that's the actual experiment: is Liquid-driven mutation viable, and on what surface?).

SAFETY: never push, never touch master, mutations on scratch branches/worktrees.

RUN until morning or ~20 more generations. MORNING REPORT: generation count (incl. part 1), kept vs rejected, best improvement with numbers, failure patterns, and the honest verdict on Liquid-driven self-improvement — including what mutation surface the loop actually wants. Negative results first-class.
