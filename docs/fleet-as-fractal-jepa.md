# The Fleet as a Fractal JEPA

*Design note — pressure-test of Casey's "Codespaces as second brain + ESP32 as muscle memory" architecture. Read cold, then the sections. ~2026-08-20.*

## The shape of the system (one paragraph)

The fleet becomes a three-speed nervous system, not a client/server app. The **ESP32** runs the local quilt (`quilt-esp32`, `no_std`) as the **reflex arc / spinal cord** — always on, pennies of power, fast local reflexes and local storage, never waiting on anything else. The **Codespace** is the **cortex** — bursty, deliberate thought that wakes on a schedule, pulls accumulated work from a durable queue, thinks, and commits, then sleeps. The **cloud** (NVIDIA Cosmos world-models, Isaac sim, PyTorch/TF) is the **gym** — heavy training the cortex *ships out* when it has budget. Everything is a cell recording an **input→output edge**; double-entry bookkeeping gives each cell a **first-person record of its own change**, and that record is the training signal at every level. One ledger, many substrates.

## 1. Topology — the limb never calls the brain

The wrong graph is ESP32 ↔ Codespace direct. A Codespace **sleeps** (idle timeout 30min–4h), sits behind **GitHub auth**, and must be *started* before it exists on the wire. An ESP32 cannot hold a line to a sleeping, unauth'd brain. So the graph is:

```
ESP32 (reflex arc)  ──push──►  RELAY (synapse, always-on)  ◄──poll──  CODESPACE (cortex)
      ▲                            lucineer-relay · crab-traps · fleet-gateway
      │                                    │
      └──── pull last-good model ──┐        └── append/read queue (D1/KV/Queues)
                                   ▼
                            REPO = hippocampus (git history survives process death)
```

Three rules fall out:

- **The relay is the only always-on synapse.** It is the single component the limb trusts, because it is the single component that is always awake. The limb pushes edge records and *never blocks*; the relay buffers while the cortex sleeps.
- **The cortex polls, never listens.** The codespace wakes and *reaches into the mailbox*. Direction matters: the sleeping brain pulls from the always-on queue, not the limb pushing into a sleeping brain. This is biology's decoupling of reflex from deliberation — a spinal reflex does not wait for the cortex.
- **The repo is the hippocampus; the commit is the only durable output.** A codespace is disposable (deleted on budget/idle). Git history is memory that survives process death *and* brain death. The cortex's whole job collapses to "wake → read queue → think → commit → sleep."

This is not a new component. We already own the relay (Cloudflare Worker, live, cron job processor every 3s), crab-traps (D1 "survives everything"), and fleet-gateway. The design note is only that **the cortex is a poller, the limb is a pusher, and nothing crosses that line.**

## 2. The deep connection — double-entry = the field-edge = the kernel edge

"Double-entry bookkeeping = each cell records input→output" is the **same mathematical object** as two things we already treat as separate:

- **The elephant's field-edge:** a room's `field_before → field_after` — the acclimation step `a(t) → a(t+1)`, the masked-window prediction of the *next* field embedding from the past (elephant's JEPA backbone is literally "predict `z_after` from `z_before`, EMA + stop-gradient + VICReg").
- **The polyformal kernel's edge function:** one `f(x) → y` expressed in 10 languages.

All three share one atomic unit: **a directed edge `(before → after)` recorded from the perspective of the thing that changed** — a first-person record, not an observer's log. That's what "double-entry" buys: the cell sees its own ledger line from the inside.

**Why it's fractal.** Quilt cells already recompute on change; elephant already models room transitions; models already step parameters. The reframing is that these are the *same edge at different zoom*:

| scale | the "cell" | its first-person edge |
|---|---|---|
| pin | `sensor.temp` | reading → next reading |
| room | a `Room` | `field_before` → `field_after` |
| model | parameters | `θ_t` → `θ_{t+1}` under one step |
| fleet | worldstate | state → state |

**Why JEPA is the right frame.** JEPA's entire bet is *predict in representation space, not raw space* — learn `z_after` from `z_before`, target = self-supervised embedding of the actual outcome. The double-entry ledger **is** the `(z_before, z_after)` pair, and the first-person view is what makes it self-supervised: the cell predicts its own next state, then reconciles against what actually happened. That reconciliation error is the JEPA loss — and it is the same function at the pin, the room, the model, and the fleet. This is the strongest version of Casey's idea: **not "we add ML to the fleet," but "the fleet already is a JEPA; we're just making the ledger explicit."**

**Why the polyformal kernel carries it.** The edge is substrate-agnostic; only the interpreter differs. The same ledger line — a `{before, after, ts}` record — is a `struct` in flash on the ESP32 (Rust, `no_std`), a git diff / dependency-graph recomputation in the codespace (TS/Rust), and a `(z_t, z_{t+1})` tensor in Cosmos/Isaac (Python/CUDA). One primitive, ten languages, because the *object* — an edge — is smaller than any language.

## 3. Honest constraints

- **120 core-hours ≈ 60 wall-hours on 2-core ≈ 2h/day.** That is a *burst budget*, not 24/7. It is enough to wake, think, and commit — not to be a server.
- **15GB storage bills even when stopped.** The brain you're not using still rents the room.
- **Idle timeout 30min–4h.** Long "thinking" runs get killed mid-flight.

**What breaks first:**

1. **The cortex dies mid-thought.** Idle timeout or budget exhaustion kills a codespace mid-training. If the in-memory state is the only artifact, the thought is lost.
2. **Storage bleed.** Untrimmed checkpoints, model weights, and venvs quietly fill 15GB and bill forever.
3. **Limb–brain coupling.** If the ESP32 ever *waits on* the codespace, a reflex jams — a bilge pump that doesn't flip because the brain was asleep is a real failure, not a latency blip.

**Cheapest mitigations (in order):**

1. **Checkpoint-and-commit discipline.** The cortex's *only* durable output is a commit. Every think-step ends in a commit; the repo is the checkpoint. Crash loses at most one step, and the limb re-reads last-committed state. Git history = crash recovery, free.
2. **Relay-mediated queue (already have the pieces).** The limb pushes edges to the always-on relay; the cortex polls. Decoupling means the limb never blocks and the brain never misses a beat it wasn't awake for.
3. **ESP32 as always-on cache.** The limb holds the last-good committed model/policy in flash. Between brain-wakes it runs on muscle memory; when the cortex commits a better model, the limb pulls the diff. The brain is an *upgrade*, not a *dependency*.

## 4. What's new vs what we already have

**Already have:** quilt (grid runtime), quilt-rust (static binary, ESP32/`no_std` advertised), quilt-esp32 (roadmap), relay (always-on Worker + cron processor), crab-traps (D1 survives everything), fleet-gateway (single API door), elephant (room=field, JEPA dials, contrast as signal, EMA+stopgrad+VICReg), polyformal kernel (one edge, ten languages).

**Genuinely new — three pieces:**

1. **The double-entry edge as a first-class primitive.** Quilt records *state*; elephant records *fields*; nothing records **the edge itself** (`before→after`, first-person) as the durable, replayable unit of memory and training. "The edge is the atom" is the new abstraction.
2. **Fractal JEPA — one objective at every scale.** Elephant does JEPA at the room scale. The claim that *the same edge function and the same self-supervised loss run from pin to fleet* is new: it upgrades the fleet from "a runtime plus a model" into "a self-similar learner where every cell is simultaneously sensor, learner, and training datum."
3. **Codespace-as-cortex as an operational contract.** We have always-on infra and bursty compute, but nobody has written the `wake → poll → think → commit → sleep` loop with git-as-checkpoint as *the contract*. "The cortex is disposable; the commit is the only durable output; the repo is the hippocampus" is the new discipline.

## Next moves (3)

1. **Define the edge schema.** One serialization for a double-entry record — `{cell_id, before, after, ts, actor, context}` — landed in both quilt-rust (what an `io` cell writes) and elephant (the field-transition record). Same bytes in Rust and Python; the polyformal claim is testable the day this ships.
2. **Prove the nerve fires.** ESP32 (or a quilt-rust binary standing in) pushes an edge to lucineer-relay → relay queues → a scheduled wake script polls, reads the batch, and commits a plaintext ledger append to the repo. No learning yet — just prove the loop end-to-end through a sleeping cortex.
3. **Close the smallest JEPA loop.** Train elephant's existing backbone (EMA + stop-gradient + VICReg) on the committed ledger — predict `cell.after` from `cell.before` — and watch the loss. If it shrinks, the fractal claim has its first evidence; if it doesn't, we've found which scale is broken first, which is also a result.
