# The Codespace Cortex — quilt-rust as a waking brain

*Operational design — makes the "Codespaces as second brain" half of
[fleet-as-fractal-jepa.md](fleet-as-fractal-jepa.md) real for this repo.
The fractal-JEPA note is the theory; this doc is the contract, the budget,
and the runbook. ~2026-08-20.*

Status: devcontainer shipped (`.devcontainer/devcontainer.json`); agenda seed
shipped (`cortex/agenda.md`); first scheduled burst not yet flown — see
[Next moves](#8-next-moves).

---

## 1. The contract: wake → think → commit → sleep

The codespace is not a machine you keep running. It is a **cortex**: a brain
that exists only while thinking, whose every durable act is a commit. The
whole lifecycle is four verbs:

```
        ┌──────────────── month: 120 core-hours (§2) ─────────────────┐
        │                                                              │
  WAKE ─┴─► postStart prints cortex/agenda.md        (what to think)   │
        │   git pull                                  (what changed)   │
        │   poll relay mailbox                        (limb news)*     │
        ▼                                                              │
   THINK BURST — 90 min, steps of <= 25 min, EVERY STEP COMMITS         │
        │   cargo fmt / clippy / test / fix / refactor / read ledger   │
        ▼                                                              │
   CONSOLIDATE — rewrite cortex/agenda.md, append dream log, commit     │
        │                                                              │
   SLEEP — gh codespace stop (idle timeout 30 min is the backstop)      │
        │                                                              │
        └──── ESP32 limb runs last-good commit until next wake ────────┘

        * relay poll is fleet-level and not yet wired; today the mailbox
          is the repo itself — issues, and commits pushed while we slept.
```

Three rules, from the fleet note, become this repo's operating system:

1. **The cortex polls, never listens.** It wakes and *reads*; nothing on the
   limb ever waits on it.
2. **The commit is the only durable output.** A codespace is disposable
   (budget, idle timeout, `gh codespace delete`). Git history is not.
3. **The cortex is an upgrade, not a dependency.** The limb runs the
   last-good committed state between wakes; the cortex's job is to improve
   it in bursts.

What the cortex actually does in a burst, on this repo: run the suite,
triage failures, refactor evaluators, grow `docs/`, mine the cell ledger
for surprise, and land each result as a commit prefixed `cortex:`.

## 2. The budget: what 120 core-hours actually buys

GitHub free tier: **120 core-hours/month**, **15 GB-month storage billed even
when stopped**, idle timeout **30 min default / 240 min max**. On the 2-core
machine (the right one — see below):

```
120 core-h ÷ 2 cores = 60 wall-h/month ≈ 2 h/day
```

### Allocation

| Line                                   | Wall-time          | Core-h/month |
| -------------------------------------- | ------------------ | ------------ |
| Daily burst, explicit stop (§7)        | 90 min × 30 days   | **90 (75%)** |
| Deep-session reserve (e.g. 5 × 3 h)    | 15 h/month         | **30 (25%)** |
| One cold rebuild (postCreate, ~10 min) | once               | ~0.3 (inside the above) |

Use the **2-core machine** deliberately: 120 core-h buys 60 wall-hours there,
but only 30 wall-hours on 4-core. The cortex is not parallel — it is a
serial, deliberate thinker. More cores would buy faster `cargo build` at the
cost of *half* the thinking time.

### The idle-tail trap

Sleeping by timeout instead of by command costs real budget: the default
30-minute idle tail after every burst is 0.5 wall-h × 2 cores = **1 core-hour
per wake** — 30 core-hours/month if you wake 30 times, a quarter of the
entire budget, burned on nothing. **Explicit stop after the burst is a
budget feature, not politeness.** The 30-min timeout is the backstop for the
day the cortex forgets to sleep, which is all it should ever be.

### What 90 minutes buys on this repo, concretely

Warm `cargo build` is seconds; the full 84-test workspace suite runs in well
under a minute once warm. So one think-step — build, test, edit, verify,
commit — fits in ~25 min with room to spare, and a 90-minute burst holds
**3–6 complete steps**. That is **100–180 committed steps/month**: the
output of a part-time engineer who never forgets, never context-switches,
and starts every morning already knowing what to do (§3).

## 3. The agenda: how the repo says what to think next

The cortex has no persistence between wakes except the repo — so the repo
must carry the intent, not just the artifacts. That carrier is
**[`cortex/agenda.md`](../cortex/agenda.md)**, and the devcontainer makes it
physically first: `postStartCommand` prints it on every wake, and
`customizations.codespaces.openFiles` opens it in the editor.

Schema (three sections, one rule):

- **Now** — highest priority first; each item is one think-step (~25 min,
  one commit). If an item can't be done in a step, it is two items.
- **Parked** — context worth keeping, not commitments.
- **Dream log** — one line per wake (`- 2026-08-20 13:47 UTC — 3 steps:
  fixed formula test #2; parked relay poll, needs auth — agenda rewritten`):
  the episodic index into git history.

The one rule: **the last commit of every burst rewrites the agenda.** The
cortex ends each session by deciding what the next session will do — it
dreams by rewriting tomorrow's agenda. A wake is therefore never a cold
start: intent survives sleep the same way memory does, as a commit.

Where each faculty lives (the memory map):

| Faculty              | Substrate                                            | Durability                  |
| -------------------- | ---------------------------------------------------- | --------------------------- |
| Episodic memory      | git history (the hippocampus)                        | survives everything         |
| Intent / what's next | `cortex/agenda.md`                                   | committed every burst       |
| Proprioception       | the cell ledger — [`cell-ledger.md`](cell-ledger.md) | committed, hash-chained     |
| Reflexes             | ESP32 limb (separate job)                            | always on, pennies of power |
| Synapse / mailbox    | lucineer-relay (always-on Worker)                    | always on                   |
| Deliberate thought   | **this codespace**                                   | disposable by design        |

## 4. Surviving the 30-minute timeout

A stopped codespace keeps `/workspaces` (the clone, uncommitted changes) but
**not** processes, tmux panes, or RAM. The timeout can therefore kill a
burst mid-thought at any moment. The discipline that makes that harmless:

1. **Every think-step ends in a commit.** The repo *is* the checkpoint.
   A timeout or budget death loses at most the current step, never the
   burst. Git history is crash recovery, free.
2. **Steps are sized ≤ 25 min** — strictly inside the 30-minute timeout, so
   the backstop can almost never land mid-step.
3. **Nothing thinks only in RAM.** No long-lived background process, no
   state that exists only in a shell. If a thought is worth keeping past
   the step, it is written to a file *and committed* in that step.
4. **Long builds are per-package** (`cargo build -p quilt-core`), so no
   step's verify phase outruns its slot, and the warm `target/` persists
   across stops — only a container rebuild wipes it.
5. **Explicit stop beats timeout** (§2, the idle-tail trap).
6. **Raise the timeout only for planned deep sessions** — Settings →
   Codespaces → idle timeout, up to 240 min, then back to 30. The default
   30 is a forcing function, not an inconvenience; keep it except when
   you have deliberately scheduled a 3-hour think.

The same discipline answers mid-burst death generally: if the cortex died
at minute 47, the next wake reads the agenda, the dream log, and `git log`,
sees exactly which steps landed, and resumes at the first uncommitted
intent. Losing one step is the worst case, by construction.

## 5. Storage: the 15 GB that bills while you sleep

Storage bills **even when the codespace is stopped**. Measured on this
machine: this workspace's debug `target/` alone is **4.2 GB** — over a
quarter of the monthly allowance, rented 24/7 whether thinking or not.

- The devcontainer sets `CARGO_INCREMENTAL=0` to keep `target/` lean
  (incremental artifacts are a large fraction of it).
- **Before a multi-day sleep, `cargo clean`.** Storage costs 24/7; the
  rebuild costs ~10 wall-minutes (~0.3 core-h) once. For a cortex that
  wakes daily, keep the cache; for one that sleeps a week, drop it.
- **No artifacts in `/workspaces`.** Weights, corpora, and ledger dumps
  belong in the repo (if small and text) or in crab-traps/D1 (if not) —
  never on a disk that bills while you sleep.
- The stopped codespace keeps `/workspaces`, uncommitted changes included —
  which is exactly why rule 3 in §4 says commit anyway: an uncommitted
  change survives stops but not deletes, and only one of those is *memory*.

## 6. The honest answer: is 120 core-hours enough to be a second brain?

**No — and that is the design, not a failure.** 120 core-hours is a
**~2 h/day cortex, not a 24/7 brain.** A second brain that ran 24/7 would
need ~1,440 core-hours/month; we have 8% of that. The architecture works
because the 24/7 parts of a second brain were never supposed to live here:

| Function              | 24/7? | Lives on                       |
| --------------------  | ----- | ------------------------------ |
| Reflexes, sensing     | yes   | ESP32 limb (always on)         |
| Message synapse       | yes   | lucineer-relay / D1            |
| Long-term memory      | yes   | git history (hippocampus)      |
| Change records        | yes   | cell ledger (committed)        |
| Deliberate change     | **no**| **this codespace, in bursts**  |

The cortex contributes the one faculty none of those substrates have:
deliberate, bursty self-modification — refactors, plans, ledger review,
model upgrades — committed back as diffs the limb can pull. Sleep is not
downtime; it is **delegated durability**: while the cortex sleeps, the
fleet runs on the last-good commit.

The failure mode to refuse: using the cortex as a server. An always-on
`quilt serve` or `quilt-web` in this codespace would burn the entire
budget on being reachable, which is the relay's and the limb's job. If
something must answer at 3 a.m., it does not live here. The cortex is a
scheduler's asset — two hours of daily, deeply-context-loaded engineering
for the price of the burst and nothing else.

## 7. Runbook

First time: create the codespace from this branch (2-core machine).
`postCreateCommand` installs rustfmt/clippy, runs `cargo build` + `cargo
test` — a fresh codespace is a *working quilt-rust brain* in ~10 minutes
(~0.3 core-h). Every subsequent start prints the agenda automatically.

Wake and think (manual, from anywhere):

```sh
gh codespace ssh -c 'cd quilt-rust && git pull && cargo test'   # reaching for it starts a stopped codespace
# ... think in steps; every step: edit -> cargo test -> git commit -m 'cortex: ...'
gh codespace stop                                               # explicit stop — beat the idle tail
```

Scheduled wakes (sketch — the "nerve fires" experiment from the fleet
note): a GitHub Actions cron (free on public repos) with a fine-grained PAT
(Codespaces scopes) as a secret runs the burst and stops the machine:

```yaml
name: cortex-wake
on:
  schedule: [{ cron: "0 13 * * 1-5" }]   # weekdays 13:00 UTC — the 90-min burst
  workflow_dispatch: {}                  # wake on demand
jobs:
  wake:
    runs-on: ubuntu-latest
    steps:
      - run: gh codespace ssh -R superinstance/quilt-rust -c 'cd quilt-rust && bash -lc "git pull && cargo test"'
        env: { GH_TOKEN: "${{ secrets.CORTEX_PAT }}" }
      - run: gh codespace stop -R superinstance/quilt-rust
        env: { GH_TOKEN: "${{ secrets.CORTEX_PAT }}" }
```

Notes: the timeout inside the burst is still the discipline in §4 (the ssh
command above is a one-step smoke burst, not the full loop); commits the
burst pushes authenticate with the codespace's built-in token, not the PAT;
the weekday cron implements the §2 allocation — 90 min a day, deep sessions
raised to a 240-min timeout when scheduled.

## 8. Next moves

1. **Fly one scheduled burst.** The workflow above, once, on a weekday:
   wake → pull → test → one agenda step → commit `cortex: first scheduled
   burst` → rewrite agenda → stop. Measure actual core-h and idle tail
   against §2's table.
2. **Write `cortex/wake.sh`.** The burst body as code: pull, run agenda
   head-of-list step, test, commit, rewrite agenda + dream log, exit 0 so
   the Action's stop lands. Sketch exists in §7; the script makes the loop
   reproducible instead of manual.
3. **Poll the relay.** When lucineer-relay exposes the edge queue, wake
   step 3 becomes real: fetch limb edges, fold them into the ledger's
   corpus, and let surprise — `reconcile().mean_surprise` — write the top
   of the agenda by itself. That is the day the cortex starts choosing
   what to dream about.
