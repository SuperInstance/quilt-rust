# ESP32 Limb Feasibility — audit, no_std split, and the muscle-memory design

*Engineering note — can quilt-rust actually run on an ESP32 as the README advertises, and what would the "limb with muscle-memory" local/cloud split look like? Companion to [fleet-as-fractal-jepa.md](fleet-as-fractal-jepa.md) (Casey's Codespace-as-cortex / ESP32-as-muscle-memory idea) and [cell-ledger.md](cell-ledger.md). ~2026-08-20.*

---

## 0. Verdict

| Question | Answer |
| --- | --- |
| Does `quilt-core` compile for **bare-metal** ESP32 (`esp-hal`, `no_std`)? | **No.** Five hard blockers in the dependency graph (tokio-full, reqwest, rhai+sync, chrono-clock, crossbeam-channel), plus direct `std::thread` / `std::future` use in the engine. |
| Does it compile for **esp-idf** ESP32 (`xtensa-esp32-espidf` / `riscv32imc-esp-espidf`, std available)? | **Probably compiles, practically dubious.** esp-idf provides std, so tokio/reqwest/rhai are not *compile* blockers there — but the RAM/flash budget (520 KB SRAM class) makes tokio + rustls + reqwest + rhai a bad resident of the device. Untested here (no toolchain installed; see §1.4). |
| Is the README claim ("Embedded / IoT / edge (RPi, ESP32) ✅", line ~42; "cross-compile it to a bare-metal target", line ~25) honest? | **Half.** RPi (Linux, musl static binary): true today. ESP32 bare-metal: false today. The claim should be scoped to "Linux-class boards" until the split in §3 lands. |
| Is the *idea* feasible? | **Yes.** The valuable core — the reactive graph, value/formula/sensor/io/listener cells, and the double-entry ledger — is either already dependency-free (`ledger.rs` is pure data by design) or a small, mechanical refactor away from alloc-only `no_std`. |
| Biggest single risk | **rhai.** It has an unaudited `no_std` feature, but the workspace uses `rhai/sync`, and rhai's own source flags the `no_std + sync` combination as unfinished. §5 gives the fallback. |

---

## 1. The audit — exact blockers

### 1.1 Two different "ESP32" questions

"Bare metal" and "ESP32" get conflated. They are different targets with different answers:

- **Bare-metal** (`esp-hal`): targets `xtensa-esp32-none-elf` (Xtensa, needs nightly rust) and `riscv32imc-esp-none-elf` (ESP32-C3). No OS, no std, no threads, no sockets, no filesystem. `alloc` over a static heap is fine. This is the target the muscle-memory limb actually wants — always-on, pennies of power, no WiFi stack unless you pay for `esp-wifi`.
- **esp-idf** (`xtensa-esp32-espidf` / `riscv32imc-esp-espidf`): Espressif's FreeRTOS-based framework. **std works** — threads, `std::net`, filesystem, and a WiFi/lwIP stack via `esp-idf-svc`. Most "std-only" crates compile. The costs are RAM (WiFi + lwIP eat ~100 KB+ of the ~520 KB SRAM before your code runs) and flash.

### 1.2 Hard blockers for bare-metal (per crate, with evidence)

All dependencies of `quilt-core` are **unconditional** — there are no feature gates to turn any of these off (`packages/core/Cargo.toml` lists all 16 deps flat).

| Crate | Where it bites (file:line) | Why it blocks `no_std` |
| --- | --- | --- |
| `tokio` (features = `full`) | `engine.rs:925` — `drive_async_boxed` builds a current-thread runtime per effectful evaluation | Needs OS threads, timers, net. `full` is the maximal feature set. |
| `reqwest` 0.12 (+ hyper, rustls) | `cells/api.rs:37-104` (the `ApiExecutor`), `error.rs:174` (`From<reqwest::Error>`) | The entire hyper/tokio/rustls TLS stack. No sockets on bare metal. |
| `rhai` 1.25 (features = `sync`, `serde`) | `cells/formula.rs:46`, `cells/program.rs:37`, `context.rs` (`eval_when`) | Has a `no_std` feature — but see §1.3. Not enabled here, and `sync` is suspected incompatible with it. |
| `chrono` (features = `clock`) | **Single choke point**: `types.rs:859-861` — `now_millis()` → `chrono::Utc::now().timestamp_millis()` | `clock` requires std. (chrono's core is `no_std` with `alloc`.) Good news: this is the *only* chrono call in the crate — ~30 call sites all route through `now_millis()`. |
| `crossbeam-channel` | `engine.rs:61` — subscription channels (`unbounded()`) | Futex/thread-parking based. std-only. |
| `parking_lot` | `engine.rs:63` (engine locks), `context.rs:40` | `no_std` only on nightly; on stable effectively std-only. |
| `once_cell` (sync) | `engine.rs:59` — `OnceCell` for the engine self-reference | Solvable: `once_cell` has a `critical-section` feature for `no_std`. |
| `serde_yml` (+ `libyml`) | `parser.rs:68,86`, `error.rs:180` | Sheet format. Swap for a binary/embedded sheet format on device (§3.3). |

Direct std use inside the engine itself (not a dep — the code):

- `engine.rs:922` — `std::thread::Builder::spawn` — **one OS thread per effectful cell evaluation** (`drive_async`). Architecturally wrong for an ESP32 even on esp-idf.
- `engine.rs:892-906` — `std::future` / `std::pin` boxing to bridge sync→async.
- `parser.rs:77,92` — `std::fs` (host-side sheet loading; fine to keep host-only).
- `std::sync::Arc` — fine; `Arc` lives in `alloc`.

### 1.3 The rhai question (the headline risk)

Checked against the actual vendored source (`rhai 1.25.1`, the version in `Cargo.lock`):

- rhai **does** ship a `no_std` feature: `no_std = ["no-std-compat", "num-traits/libm", "core-error", "libm", "hashbrown", "no_time"]`.
- Its `src/lib.rs` carries `#![cfg_attr(feature = "no_std", no_std)]` **and** a `TODO: Further audit no_std compatibility`, with a note that `no_std + sync` needs explicit alloc imports — i.e. exactly the feature pair we'd want (`sync` because the engine shares `Engine` across threads today).
- Verdict: **treat rhai-on-no_std as an unresolved spike, not a plan.** It may work single-threaded without `sync`; it may not work at all. Budget the fallback (§5): a ~300-line expression interpreter covering the reflex subset (`>=`, `+`, `-`, `*`, `/`, `&&`, `||`, `if/else`, `min/max/clamp/abs`) — the dependency-graph auto-detection (`expr_contains_token`, `engine.rs:860`) is already regex-free and portable as-is.

Also note footprint: rhai is roughly 1–2 MB of flash and heap-hungry per `Engine` instance — and `formula.rs:114` constructs a **fresh `Engine` on every evaluation**. Even where rhai compiles, the per-eval construction pattern must change on-device.

### 1.4 What I could and could not test

- Installed targets: `x86_64-unknown-linux-gnu` only. Checking `esp32*`/`thumb*`/`no_std` targets requires `rustup target add` (plus nightly for Xtensa) — **not installed, per instructions**.
- What was verified: `cargo check -p quilt-core --offline` passes on the host (clean baseline). The blocker list above is from source/registry inspection (actual vendored crate manifests), not guesswork.
- Cheap host-side win for later: `cargo tree -p quilt-core` shows the whole closure is unconditional; there is no existing feature to flip.

### 1.5 What is *already* portable (the good news)

- **`ledger.rs` (1222 lines) is the crown jewel for this port**: no tokio, no clocks (callers pass `ts`), no I/O, its own dependency-free SHA-256, serde-only, and `forbid(unsafe_code)` throughout the crate. It was explicitly designed to be "embeddable anywhere the engine runs" (its header says so). The muscle-memory data model already exists.
- `types.rs` — serde only, apart from the one `now_millis()` choke point.
- Cell evaluators `value.rs`, `sensor.rs`, `io.rs` — pure `CellValue` constructors; adapters do the real I/O. `io.rs` is 63 lines and dependency-free.
- `listener.rs` — engine-propagation-driven; the rhai dependency is only in `eval_when` gating.
- `indexmap` 2.14 — vendored source is `#![no_std]` with an optional `std` feature. **Not a blocker** (`default-features = false`).
- `serde` / `serde_json` — `no_std` + `alloc` supported.
- `thiserror` — `no_std`-fine (core-based).
- **Dead weight found during the audit**: `quilt-core` declares `uuid`, `regex`, `futures`, and `anyhow` — none are used by the core code (grep-verified; `anyhow` appears only in comments). They can be dropped from core's manifest outright, shrinking the embedded closure for free.

---

## 2. README accuracy

The claim chain: README line ~42 ("Embedded / IoT / edge (RPi, ESP32) ✅") + line ~25 ("Cross-compile it to a bare-metal target and forget it") + the cross-compile table (which also lists `wasm32-unknown-unknown`). As of this audit:

- RPi / ARM Linux / musl: **true** (rustls tree is pure Rust; verified design).
- ESP32 bare-metal: **false** — §1.2 blockers.
- esp-idf: **unproven and impractical** as a full-workspace build (tokio+rustls+rhai resident on 520 KB SRAM class hardware).
- `wasm32-unknown-unknown`: **also false as-is** — same tokio-full/reqwest blockers (no threads/sockets). Worth fixing the table while fixing the ESP32 claim.

Suggested (not applied — audit-only): scope the claim to "Linux-class edge (RPi, Graviton, musl containers)" and mark ESP32 as `roadmap — quilt-embedded (§3)`.

---

## 3. The no_std split — `quilt-embedded`

### 3.1 Shape: one core, two profiles

Not a fork — a feature split inside `quilt-core`, mirroring how the engine already separates "pure graph" from "effectful evaluators":

```
quilt-core
├── (always, no_std + alloc)     types, context, ledger, graph engine,
│                                cells: value | formula* | sensor | io | listener
├── feature "std-host" (default) parser (serde_yml + std::fs), cells: api | program | router,
│                                drive_async, crossbeam subscriptions, chrono clock
└── feature "embedded"           Clock/IoPort/Storage trait shims, compact sheet loader
```

`*` formula keeps rhai on std-host; on embedded it is either rhai-no_std (spike) or the mini interpreter (§5).

### 3.2 Module-by-module changes

| Module | Change for no_std |
| --- | --- |
| `types.rs` | Replace `now_millis()` with `pub trait Clock { fn now_millis(&self) -> u64 }`, stored on the engine. Host impl: chrono. ESP32 impl: `esp-idf-svc` `sntp` time or a monotonic ms counter (ledgers only need a monotonic, comparable clock per device; wall-time is a sync concern, §4.3). |
| `engine.rs` | Keep the sync core (it is *already* sync — this is the port's biggest lucky break). Delete `drive_async`/`drive_async_boxed` + the `std::thread` spawn under `#[cfg(feature = "std-host")]`. `api`/`program`/`router` evaluators become host-only. `parking_lot::RwLock` → `spin::RwLock` or a `critical-section` wrapper (single-core C3 barely needs a lock at all). `once_cell::sync::OnceCell` → `once_cell` with `critical-section`. `IndexMap` stays, `default-features = false`. |
| Subscriptions | `crossbeam` channels → an `heapless::spsc::Queue` per subscriber (bounded, lock-free) or a simple registered-callback list (`&mut dyn FnMut(SubscriptionEvent)`). On a limb, the only subscriber that matters is the sync agent (§4.3) and the io adapters. |
| `cells/formula.rs` | Keep `FormulaEngine` (compile-once, run-many) — it already caches the AST. Kill the per-eval `Engine::new()` (reuse one `Engine` behind the cell). Evaluate the rhai-no_std spike before committing; fallback in §5. |
| `cells/api.rs`, `program.rs`, `router.rs` | `#[cfg(feature = "std-host")]`. On-device "call the world" is an *adapter*, not a cell evaluator (below). |
| `parser.rs` | Host-only YAML. Device loads sheets as embedded assets: either `serde_json` (debug builds) or [`postcard`](https://docs.rs/postcard) schema (release) compiled into the binary via `include_bytes!` / build script. A 12-cell reflex sheet is ~1 KB postcard. |
| `ledger.rs` | **No changes.** Wire it into the engine while doing the split (it is not yet hooked into evaluation — `docs/cell-ledger.md` says so) so every limb reflex posts entries from day one. |

### 3.3 The `io` cell shim — `embedded-hal` / esp-idf

The engine already treats io correctly for this: an `io` cell is a *value* plus a port name (`gpio:relay1`); the adapter does the work (`cells/io.rs` header: "the engine doesn't know how to talk to the port"). So the shim is one registry mapping port names to implementations:

```rust
pub trait IoPort {
    fn read(&mut self) -> Result<Value>;            // push-in side
    fn write(&mut self, v: &Value) -> Result<()>;   // set-out side
    fn direction(&self) -> Direction;
}

pub trait PortFactory {
    fn open(&self, port: &str) -> Result<Box<dyn IoPort>>;
}
```

- **esp-idf build** (`xtensa-esp32-espidf` / `riscv32imc-esp-espidf`): `esp-idf-hal` pins (`PinDriver`), I2C/SPI (`Master`), UART, and `esp-idf-svc` for WiFi + SNTP + NVS. Recommend this as the **first hardware target**: std keeps the build boring, and the no_std-clean core runs identically inside it.
- **bare-metal build** (`esp-hal`): same `IoPort` impls over raw `embedded-hal` traits; storage via `embedded-storage` NOR-flash driver; WiFi only if `esp-wifi` is ever worth its weight — the limb design (§4) deliberately does not need it to be fancy: one TLS-less push to the relay.

Storage for the ledger (the limb's local memory):

- Append-only entry log: NOR flash ring / SPIFFS file, postcard-serialized `LedgerEntry` (entry ≈ 80–150 B; a pump cell flipping twice a minute is ~10 KB/week — trivially in a 4 MB flash partition, and it ships-and-trims on sync).
- NVS for the *tuned* cells (thresholds, gains — §4.4) plus the chain head hash per cell, so reboot = replay-free resume.

### 3.4 Footprint budget (estimate, to be validated on hardware)

| Piece | Flash | RAM |
| --- | --- | --- |
| no_std core (engine + 5 cell kinds + ledger + serde_json) | ~150–300 KB | ~tens of KB heap for a <100-cell sheet |
| + rhai (if the spike works) | +1–2 MB | +10s of KB per Engine |
| + mini interpreter instead | +~15 KB | negligible |
| esp-idf + WiFi + lwIP | ~1 MB | ~100–150 KB |
| Full tokio+rustls+reqwest (rejected) | +2 MB+ | handshake buffers alone ≈ 16–32 KB, plus runtime — the reason §3.1 exists |

---

## 4. The muscle-memory split

This section designs Casey's architecture concretely, on top of the objects that already exist: `CellLedger`, `LedgerOrigin`, `Provenance`, `chain_hash`, `reconcile`, `replay` (`ledger.rs`), and the relay/repo topology fixed by [fleet-as-fractal-jepa.md](fleet-as-fractal-jepa.md) — limb pushes, cortex polls, relay buffers, repo is the hippocampus.

### 4.1 What "muscle memory" means at the cell level

- **Muscle** = the *reflex loop*: `sensor` → `formula` → `io`, always resident on-device, alloc-only, firing in microseconds-to-milliseconds with **zero round-trips**. Plus the tuned parameters it reads: hysteresis bands, gains, calibration offsets — stored as `value` cells under NVS.
- **Memory** = the *per-cell double-entry ledger* in flash: every input→output edge with its `expected` and `imbalance` (surprise). This is the limb's first-person record — the `(z_before, z_after)` pair of the fractal-JEPA claim, at pin grain.
- **Learning** = the cortex's job, off-device, *on the ledger the limb produced*. The limb never trains; it only records and executes. The only thing that ever "flows down" is new values for parameter cells — which is just a `set`, which is just another entry in the ledger.

Concrete bilge-pump example (the README's own sheet, limb-ified):

```yaml
# compiled into the firmware (postcard), not parsed from YAML on device
- id: bilge.level        # sensor; adapter pushes at 1–10 Hz from GPIO/I2C
- id: bilge.hyst.on      # value 80.0   ← MUSCLE: tuned by the cortex
- id: bilge.hyst.off     # value 60.0   ← MUSCLE: tuned by the cortex
- id: pump.should_run    # formula: bilge.level >= (pump.running ? hyst.off : hyst.on)
- id: pump.relay         # io, gpio:relay1, out  ← the reflex fires here
- id: alarm.listener     # listener: watch [pump.should_run]
```

Sensor crossing 85 cm → relay energizes in the same propagation walk, asleep cortex or none. Every flip writes a `LedgerEntry` on `pump.should_run` with `provenance.origin = push` and the surprise against the last state.

### 4.2 What lives where

| | ESP32 limb (local quilt) | Relay (synapse) | Codespace cortex (cloud quilt) |
| --- | --- | --- | --- |
| Reactive graph | yes — the reflex cells above | no | yes — a full `quilt-core` host build, including `api`/`program`/`router` |
| Cell ledgers | origin: appended in flash | buffer only | mirror: shipped-up entries appended to the repo |
| ML / training | **none** | none | JEPA-style training on `(before → after)` edges; imbalance series is a pre-computed loss |
| Sheet authoring | no (embedded asset) | no | yes (YAML → compiles/exports the limb sheet) |
| Wall clock | monotonic only | real time | real time (timestamps reconciled at sync, §4.3) |
| Availability | always on | always on | bursty (wake → poll → think → commit → sleep) |

### 4.3 Sync protocol shape (ledger-native)

Two message types and one invariant. Everything serializes as `serde`/postcard; the relay (Cloudflare Worker + D1/Queues — already owned per the design note) never interprets them.

**Uplink — `LedgerBatch` (device → relay, push, non-blocking, buffered):**

```jsonc
{
  "device": "esp32-bilge-07",
  "cells": [{
    "cell_id": "pump.should_run",
    "from_seq": 1201, "to_seq": 1483,
    "head_hash": "b01d…",              // chain head after `to_seq`
    "entries": [ /* compact LedgerEntry: seq, ts, in, out, delta, expected, imbalance, prev_hash, hash */ ]
  }]
}
```

- The chain travels **with its hashes** — the relay and cortex can `verify_chain()` what they received; tampering or truncation is detectable end-to-end (`ledger.rs` gives this for free).
- Device marks entries shipped but **keeps them** until an ack (`to_seq` + `head_hash` match) returns via the downlink mailbox, then trims flash.
- Clock skew: entries carry device-monotonic `ts`; the cortex anchors device time at first sync (`(device_ts, wall_ts)` pair stored per device) — the ledger design already demands callers pass timestamps, so this stays outside the data structure.

**Downlink — `TuningPacket` (cortex → repo → relay mailbox → device pulls):**

```jsonc
{
  "trained_against": { "pump.should_run": "b01d…" },  // chain heads used in training
  "updates": [
    { "cell_id": "bilge.hyst.on",  "value": 74.0, "why": "surprise trend 12→3 after change" }
  ],
  "cortex_commit": "9f2c…git-sha…"
}
```

- The limb **pulls** its mailbox when it has connectivity; it never blocks on it (rule: the limb never calls the brain).
- Application is `engine.set(cell_id, value, ctx)` with `Provenance { origin: System, caller: Some(cortex_id) }` — i.e. a tuning **is itself a ledger entry** on the parameter cell, so the next training round sees its own past interventions in the ledger. Rollback = the cortex shipping the previous value with a new packet (git-revert analog).

**The invariant — hash-anchored reconciliation:**

- A `TuningPacket` is applicable iff `trained_against[cell] == device.head_hash(cell)` (or the device is explicitly behind and replays forward).
- After applying, the parameter cell's ledger records the edge; subsequent reflex entries are scored against the new behavior. **Muscle-memory improvement is measurable on-device as shrinking `imbalance`** — the JEPA loss, computed by the same `value_distance` the ledger already runs, at pin grain, with no ML runtime on the chip.

### 4.4 Degraded modes

- **Offline (brain asleep / WiFi down):** reflexes keep firing; ledgers keep appending to flash; the uplink queue grows within its budget. When full, the device (a) drops the *lowest-surprise* entries first, keeping edges — they are the training gold — and (b) keeps chain-adjacency by sealing a "trimmed" marker entry so the cortex knows what it did not receive.
- **Cortex dies mid-thought:** nothing on the limb notices except the mailbox; last-committed `TuningPacket` remains the muscle. The repo is the checkpoint (per the design note's commit discipline).
- **Bad tuning (surprise rises after a packet):** visible as a rising imbalance series in the next uplink; cortex reverts. Worst case the limb runs its factory parameters forever — the failure mode is "unimproved," never "broken."

---

## 5. What it would take — honest estimate

| Phase | Work | Estimate | Notes / risk |
| --- | --- | --- | --- |
| 0. Hygiene | Drop unused `uuid`/`regex`/`futures`/`anyhow` from core; move `now_millis` behind a `Clock` trait | 1–2 days | Pure win regardless of ESP32; shrinks every future target |
| 1. Core split | Feature-gate `std-host` (parser, api/program/router, drive_async, crossbeam, parking_lot, once_cell-cs swaps); `no_std + alloc` CI target | 1–2 weeks | Mechanical; the sync engine core needs no redesign. Needs a `no_std` CI target (thumbv7em or riscv32imc) to stay honest |
| 2. Formula story | Spike rhai `no_std` (without `sync`, single-threaded); if it fails, write the mini expression interpreter (~300 lines: arithmetic, comparisons, bool ops, min/max/clamp/abs, `cells["id"]` map) | 1 week spike; 1–2 weeks fallback | **The headline risk.** rhai's own source marks `no_std` unaudited and `no_std+sync` unfinished. The fallback is bounded and covers the reflex subset fully |
| 3. esp-idf shim | `IoPort`/`PortFactory` over esp-idf-hal (GPIO/I2C/SPI/UART), NVS ledger storage, `esp-idf-svc` WiFi + SNTP, embedded postcard sheet loader; blink-the-relay demo of the §4.1 sheet | 1–2 weeks | Do esp-idf **before** bare-metal; std makes iteration fast, and the Phase-1 core runs unchanged inside it |
| 3b. (optional) bare-metal | Same shims over `esp-hal` + `embedded-storage`; drop WiFi or take the `esp-wifi` hit | +1–2 weeks | Xtensa needs nightly; C3 is stable. Only if the pennies-of-power always-on constraint demands it |
| 4. Sync agent + nerve test | Uplink `LedgerBatch` to the relay, downlink `TuningPacket` pull, ship-and-trim; proves "next move 2" of the design note (the nerve fires through a sleeping cortex) | 1–2 weeks | Relay/D1/Queues already exist per the design note |
| 5. Cortex loop | Codespace poll → append to repo → train JEPA-style on edges → emit TuningPacket → commit | 1–2 weeks (separate track) | Python/TS side; touches no embedded code |

**Bottom line: ~6–10 focused engineering weeks** to a limb whose reflexes fire locally, remember locally, and get smarter from commits. A credible "nerve fires" milestone (Phases 0+1+3+4, factory parameters, no learning) is **~3–4 weeks**.

Risk register (ordered):

1. **rhai `no_std`** — unresolved upstream; mitigation is the mini interpreter, which caps the blast radius at `formula.rs`'s evaluator choice. Everything else in the split is mechanical.
2. **RAM discipline on esp-idf** — WiFi + heap + ledgers on 520 KB; mitigation: reflex sheets are small (<100 cells), ledgers stream to flash, JSON only in debug (postcard in release).
3. **Per-eval `Engine`/allocation patterns** — the codebase allocates freely (clone-out cells, `format!` in hot paths); fine on a server, needs a pass on device. Bounded but tedious.
4. **Clock semantics** — device-monotonic vs wall time; solved at the protocol layer (§4.3), but must be pinned before the first shipped ledger, because chain hashes commit timestamps.
5. **Scope creep toward "run the whole 8-kind sheet on the chip"** — don't. The limb is value/formula/sensor/io/listener + ledger. `api`/`program`/`router` are cortex cells by definition in this architecture.

---

*Appendix — evidence commands: `rustup target list --installed` (host only), `cargo check -p quilt-core --offline` (passes), vendored-manifest inspection of `rhai-1.25.1`, `indexmap-2.14.0`, `chrono-0.4.39`, `parking_lot-0.12.5`, `once_cell-1.21.4`; source grep for `chrono|tokio::|reqwest::|rhai|std::thread|crossbeam|parking_lot|uuid|regex` across `packages/core/src`. No toolchains installed, no code modified.*
