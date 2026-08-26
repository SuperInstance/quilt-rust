# VESSEL-FIT — quilt-rust against F/V EILEEN

*Playtest lane, 2026-08-26. The vessel is real: F/V EILEEN, a fishing boat home-ported in Kodiak, AK. The brain must run **offline, 60 miles offshore, no cloud**. Edge doctrine: ESP32 limbs + Liquid LFM2.5 local models ("hundred boats" — many cheap local agents, zero per-token cost). This doc records what I actually ran, what passed, what failed, and what the boat should do with it. Undersell, overdeliver.*

---

## 0. What I ran today (hands on keyboard)

| Check | Result |
|---|---|
| `cargo test --workspace` | **✅ all green** — quilt-core 53 unit + 14 integration + 6 examples tests; quilt-cabi ABI smoke 2/2; quilt-core-wasm 26 unit + 3 wasm conformance; quilt-mcp 3; quilt-tui 8; compat conformance 1. Two tests ignored, zero failures. |
| `quilt inspect examples/boat-autopilot/sheet.yaml` | ✅ 13 cells parsed (Formula/Io/Listener/Program/Router/Sensor/Value). |
| `quilt run examples/sensor-anomaly/sheet.yaml` | ✅ loads, evaluates. Note the honest output: `model.analyzer (Api) = {"model":"claude-sonnet-4-5","note":"model calls not yet implemented"}` — API cells are placeholders. |
| `quilt serve … --mcp` (stdio JSON-RPC probe) | ❌ **BUG.** Server banner says "10 cells" but `cells_list` returns `[]` and every `cell_get/cell_push` errors `cell not found`. Root cause: `packages/cli/src/main.rs::serve_mcp()` loads the sheet into one engine, then calls `quilt_mcp::serve_stdio()`, which constructs a **fresh empty server** (`QuiltMcpServer::new()`). `build_server(sheet_path)` exists in `packages/mcp/src/lib.rs` and does the right thing — it is simply never called by the CLI. The protocol layer itself works (initialize/tools-list respond correctly). |
| `cargo build -p quilt-cabi` | ✅ builds clean. C ABI = the bridge a Linux-class boat computer (Raspberry Pi / mini-PC helm) uses to embed the same engine from Python. |
| Read-only audit: `docs/esp32-limb-feasibility.md` | quilt-core does **not** compile bare-metal for ESP32 (tokio-full, reqwest, rhai-sync, chrono-clock, crossbeam are unconditional deps; `engine.rs:925` spawns an OS thread per effectful eval). ESP32 belongs to **quilt-esp32 / quilt-vm-c**, not to this repo. quilt-rust's vessel seat is the **Linux-class helm box**. |

**One-line verdict:** quilt-rust is the *shoreside/helm tier* of the boat brain — reactive sensor graph, rule escalation, ledger, MCP tooling for a local agent — but it has no real IO, no real model calls, no on-disk persistence, and its MCP serve path is broken for any sheet.

---

## 1. Per use-case: what quilt-rust provides TODAY

Cited = files I actually exercised or read line-by-line.

### AIS nearby-vessel tracking
**Today:** the reactive graph fits perfectly — `sensor` cells (one per feed: `ais.position.MMSI…`, `ais.speed`, own GPS), `formula` cells for CPA/TCPA (the wrap-around heading-error formula in `examples/boat-autopilot/sheet.yaml:heading.error` is exactly the right shape for bearing math), `listener` cells with `condition:` for "vessel inside 2nm and closing → alert". `engine.push()` (`engine.rs:436`) is the ingest entry: push an AIS sentence's parsed fields, downstream recomputes.
**Missing:** no NMEA-0183/NMEA-2000 parser; no `source:` driver beyond `simulated` (the autopilot sheet even says so: `# In a real deployment, source would be 'nmea:/dev/ttyUSB0'`); no geo types (lat/lon as plain numbers, no haversine built in).

### Engine monitoring (RPM / temp / pressure)
**Today:** this is the strongest fit. `examples/sensor-anomaly/sheet.yaml` already implements the exact pattern the boat needs — rolling mean → z-score formula → `surprise.should_escalate` boolean → alert listener. Per-sensor sheets compose: `engine.rpm`, `engine.temp`, `engine.oil_pressure` each with their own band + z-score cells.
**Missing:** `ApiExecutor` (`cells/api.rs:37-104`) is the escalation target and model calls are "not yet implemented" (verified live above); no hysteresis/deadband primitive (must be hand-rolled in program cells); no unit-conversion cells.

### Log/debris detection from cameras
**Today:** almost nothing in this repo. quilt-vision is a 193-line sketch (no real models); quilt-rust has no vision cell kind. What *does* work: the escalation plumbing — a camera-side detector (ESP32-S3 or a small SBC) pushes detection scores into a sensor cell via `push()`, and the anomaly/z-score pattern above decides alert vs. ignore.
**Missing:** everything between "JPEG in" and "bounding box out". Realistically: local YOLO-class model on the helm box or a Jetson-class limb, feeding quilt cells.

### Course plotting
**Today:** `boat-autopilot` sheet proves the loop end-to-end at the cell level: compass sensor → heading-error formula (shortest-path, correct modulo math) → proportional rudder program → `io` cell (`actuator:rudder`) → off-course listener + log. Edit `desired.heading`, watch it propagate.
**Missing:** `io` cells are declarations — no actual hardware/serial/NMEA driver executes them; no chart/plot rendering (quilt-live's single-HTML grid is a starting point for a helm display); no waypoint list cell kind.

### Voice interaction at sea
**Today:** `voice.intent` → `voice.parser` router with `when:` rules on caller metadata exists in the autopilot sheet, and `model.router` shows caller-aware model routing. The `cell_call` MCP tool (`packages/mcp/src/lib.rs`) is the natural interface for a **local** LFM2.5 agent: it discovers cells with `cells_list` and reads/writes/pushes them — no cloud.
**Missing:** the MCP serve bug above blocks exactly this; no audio anywhere; program cells are JS-flavored pseudocode evaluated by... nothing yet on the Rust side (programs return null in `quilt run` — verified).

### Persistence / audit (cross-cutting)
**Today:** the in-memory double-entry ledger (`ledger.rs`) seals every cell transition — hash-chained `imbalance = ‖Δ‖‖` per `docs/cell-ledger.md` and the field-edge bridge (`crates/field-edge-bridge`, identities verified to 1e-12). For the boat this is the black-box recorder: un-gameable record of every sensor surprise.
**Missing:** **no on-disk persistence** — `quilt set` mutates an engine that dies with the process; nothing writes the ledger to disk/sdcard. For a boat this is a first-season blocker.

---

## 2. Top-3 suggestions, ranked by value-to-the-boat

1. **Fix the MCP serve bug (1-line-ish, unblocks the whole agent story).** `serve_mcp()` should call `quilt_mcp::build_server(Some(file))` instead of loading its own engine + calling `serve_stdio()`. Repro: `quilt serve any-sheet.yaml` → `cells_list` = `[]`. Once fixed, the offline LFM2.5 boat brain gets a native tool interface to the entire cell graph — AIS alerts, engine bands, voice commands — over stdio, no cloud.
2. **Ledger dump + replay-to-disk.** A `quilt commit <file>` / `--journal <path>` that appends the hash-chained ledger to disk (JSONL, like quilt-esp32's `dissent-ledger-host.jsonl`) and can reload it at boot. This turns the boat brain into a black-box recorder and makes "delayed cloud sync when in range" trivial: replay the journal to a shoreside quilt-cloudflare instance over the ledger bridge (`docs/field-edge-ledger-bridge.md`) when cell coverage returns.
3. **Real `source:` drivers — start with NMEA-0183 over serial.** A `source: serial:/dev/ttyUSB0,4800` sensor driver (plus a tiny NMEA sentence parser for GGA/RMB/VHW/XDR) makes AIS and engine instruments real with one feature. Everything downstream (formulas, listeners, escalation) already works. This is the difference between a demo and a thing you'd leave running on the helm during a season.

(Honorable mention: implement the `api` cell's pluggable backend so escalation routes to a **local** Ollama/LFM2.5 endpoint (`http://127.0.0.1:11434`) instead of a cloud model string. The escalation *pattern* is proven; only the target is missing.)

---

## 3. "First season" sketch — where the cells live on F/V EILEEN

Tiered per the fractal doctrine (`docs/fleet-as-fractal-jepa.md`); every tier works alone, radio or not.

```
TIER 0 — ESP32 limbs (quilt-esp32 / quilt-vm-c, no_std, proven on metal today)
  limb-engine     .qm rule table: RPM/temp/pressure bands → alert LED + buzzer
                  (110ns-class serves, radio dark — same pattern as critic-gate)
  limb-ais        .qm: nearest-vessel distance bands → proximity LED
                  (AIS RX over UART at the helm transponder, parsed upstream)
  limb-cam        frame-difference / motion energy → "something in the water"
                  score cell; threshold locally, escalate upstream only on hit

TIER 1 — helm box (Linux SBC, e.g. RPi 5; quilt-rust via quilt-cabi, this repo)
  Sheet: eileen-helm.yaml
    sensor cells    ← NMEA-0183 serial feeds (AIS, GPS, depth, engine via
                      NMEA-2000 bridge or analog→ESP32→ESP-NOW)
    formula cells   CPA/TCPA, z-score anomaly per engine metric (sensor-anomaly
                      pattern), course error vs. waypoint (autopilot pattern)
    listener cells  collision-risk, engine-out-of-band, over-wake — each with
                      severity + dissent-log entry when the critic gate disagrees
    ledger          hash-chained journal to SD card (suggestion #2)
    agent           Liquid LFM2.5 (local Ollama) as MCP client → cells_list /
                      cell_get / cell_push; voice intents land as cell_set on
                      voice.intent (router pattern from boat-autopilot)

TIER 2 — shoreside (only when in range; days delayed, that's fine)
  quilt-cloudflare worker receives the journaled ledger replay (field-edge
  bridge projections), archives season history, runs the cloud-only analyses.
```

Escalation path offline: limb rule table → (ESP-NOW/UART) → helm sheet listener → local LFM2.5 critique (the critic-gate pattern, already 100% replay-agreement on metal) → dissent ledger → sync when in range. Every step degrades gracefully: radio dark, the limbs still blink, buzz, and log.

**What NOT to put on the boat:** k3s/swarm/nomad variants — those are cloud-fleet control planes, dead weight 60 miles offshore.

---

*Evidence artifacts: test transcript in commit message; MCP bug repro = `printf '…tools/call cells_list…' | quilt serve examples/sensor-anomaly/sheet.yaml`.*
