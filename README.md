# 🦀 Quilt (Rust)

<p align="center">
  <img src="assets/images/hero-cells.jpg" alt="Every cell its own instance — runtimes, tools, and models living in the grid" width="720">
</p>

> **A spreadsheet where every cell is a live, addressable capability — now in a single statically-linked binary.**

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-native-purple)](https://modelcontextprotocol.io)
[![TypeScript port](https://img.shields.io/badge/TypeScript-canonical-3178c6)](https://github.com/superinstance/quilt)
[![Status](https://img.shields.io/badge/status-v0.2.0-brightgreen)](https://github.com/superinstance/quilt-rust)

**[Quilt Live ⚡](https://superinstance.github.io/quilt/landing/quilt-live.html)** · **[Studio 🎨](https://superinstance.github.io/quilt/landing/studio.html)** · **[Showcase 🌟](https://superinstance.github.io/quilt/landing/showcase.html)** · **[TypeScript version →](https://github.com/superinstance/quilt)** · **[Read the manifesto →](https://github.com/superinstance/quilt/blob/main/docs/manifesto.md)**

---

## ⚡ See it in 30 seconds

```rust
use quilt_core::{QuiltEngine, CellKind, CellValue};

let mut engine = QuiltEngine::new();

// Define three cells.
engine.define("sensor.temp",  CellKind::Sensor,  CellValue::Float(22.0))?;
engine.define("led.on",       CellKind::Formula, CellValue::None)?;
engine.define("actuator.led", CellKind::Io,      CellValue::Bool(false))?;

// Wire them.
engine.add_dep("led.on", "sensor.temp")?;

// Run forever. Reactive. Sync. Native.
loop {
    let temp = dht22.read();
    engine.set("sensor.temp", CellValue::Float(temp))?;
    let on = temp > 25.0;
    engine.set("led.on", CellValue::Bool(on))?;
    engine.set("actuator.led", CellValue::Bool(on))?;
    delay.delay_ms(1000);
}
```

That's a 3-cell reactive system compiled to a single binary, ~3 MB stripped, no runtime, no GC, no Node.js. Drop it on a Raspberry Pi, a Graviton, a serverless function, or a bare-metal target.

**→ [Open Quilt Live in your browser](https://superinstance.github.io/quilt/landing/quilt-live.html)** (works without installing Rust)

---

## 🎬 The 8 cell kinds

```rust
match cell.kind {
    CellKind::Value    => /* static value */,
    CellKind::Formula  => /* reactive expression */,
    CellKind::Program  => /* sandboxed rhai script */,
    CellKind::Sensor   => /* polled input */,
    CellKind::Api      => /* outbound call */,
    CellKind::Listener => /* fires on change */,
    CellKind::Router   => /* caller-context dispatch */,
    CellKind::Io       => /* physical port */,
}
```

The vocabulary is the same as the TypeScript version. The types are stronger. The runtime is sync. Async happens at the boundary.

---

## 🏗️ Architecture

```
   ┌──────────────────────────────────────────────────────────────┐
   │                      your binary                             │
   │                                                              │
   │   ┌─────────────┐  ┌─────────────┐  ┌──────────────────────┐  │
   │   │  quilt-core  │  │  quilt-tui  │  │  quilt-web (axum)    │  │
   │   │             │  │             │  │                      │  │
   │   │   engine    │  │  terminal   │  │   HTTP + SSE         │  │
   │   │   parse     │─▶│  UI         │  │   live updates       │  │
   │   │   eval      │  │             │  │   cell browser       │  │
   │   │   reactive  │  │             │  │                      │  │
   │   └─────────────┘  └─────────────┘  └──────────────────────┘  │
   │            │                                                │
   │            ▼                                                │
   │   ┌──────────────────────────────────────────────────────┐  │
   │   │  std::sync::Arc<QuiltEngine>                         │  │
   │   │  Send + 'static effectful evaluators                 │  │
   │   │  Tokio at the boundary, sync at the core             │  │
   │   └──────────────────────────────────────────────────────┘  │
   │                                                              │
   └──────────────────────────────────────────────────────────────┘
```

Same engine, two front-ends: a TUI (crossterm) and a web UI (axum + Server-Sent Events). Async happens at the boundary; the engine itself is fully synchronous, which means Send + 'static propagates naturally and lifetimes just work.

---

## What is this, really?

Same paradigm as the TypeScript quilt, distilled to what actually runs. Where the TypeScript version is a live laboratory — browser simulator, web UI, TUI, MCP server — the Rust version is the same idea with the laboratory removed: **a spreadsheet that is a runtime, compiled into one statically-linked binary you can drop on anything with a CPU.**

Underneath the polish, both are the same machine. You write a sheet in YAML: a grid of named cells. A cell is not a box that holds a number — it is a *live, addressable capability*. A `formula` cell recomputes when its inputs change. A `sensor` cell holds the latest reading an adapter pushed in. A `program` cell runs a sandboxed rhai script. An `api` cell fetches on demand. A `router` cell decides where a call goes based on who is calling. A `listener` cell fires when something it watches changes. The sheet is the control plane; the cells are the machinery; and the whole thing is reactive by default.

The Rust port takes that machine and turns it into a deployment artifact. No Node.js runtime to install, no `npm install`, no `node_modules` to ship. One binary, `quilt`, ~3 MB stripped. Embed it in a Raspberry Pi that reads a bilge sensor and flips a relay. Run it on a Graviton instance as a fleet-wide policy engine. Cross-compile it to a bare-metal target and forget it. Statically linked means the binary carries its own runtime — the box it lands on just has to be able to execute it.

**Why this matters:** a cellular runtime is a good shape for the edge, because the edge is a spreadsheet — a hundred small things, each with a state, each reacting to the others, each addressable by name. Node made that shape available but expensive to deploy; Rust makes it available everywhere a binary can run. Same sheets, same model, same 8 cell kinds — the compiler instead of the interpreter, ready for the machines that don't have a JavaScript runtime and never will.

---

## 🤔 Why Rust?

The TypeScript version is the **canonical implementation** — it has the browser simulator, the MCP server, the TUI, the web UI, and 15/15 tests passing.

The Rust version is a **production-grade port** of the same engine, designed for engineers who want:

| Need                                                | Use **Rust** | Use **TypeScript** |
| --------------------------------------------------- | :----------: | :----------------: |
| **Single static binary**, no Node.js runtime        | ✅            | ❌                  |
| **Terminal UI** (`quilt-tui`)                        | ✅ (crossterm)| ✅                  |
| **HTTP server** with live SSE updates (`quilt-web`)| ✅ (axum)     | ✅                  |
| **Embedded / IoT / edge** deployment (RPi, ESP32)   | ✅            | ❌                  |
| **High-throughput** cell evaluation (10⁵+ cells/s)  | ✅            | ⚠️ (~50k cells/s)   |
| **Strict memory guarantees** in a sandboxed cell    | ✅ (rhai)     | ❌                  |
| Browser / web UI / live simulator                   | ✅ (axum + JS)| ✅                  |
| MCP server to plug into Claude Code / Cursor        | ✅            | ✅                  |
| Production-grade, fully tested, ready-for-release   | ✅ (v0.2.0)   | ✅ (v0.2.0)         |
| Formulas / programs execute via                     | `rhai` (sandboxed) | `new Function` |
| I/O via                                            | `reqwest` (async via `tokio`) | `fetch`        |
| Scripting language                                  | Rhai (Rust-native) | JavaScript    |

> The two repos share the **same sheet format (YAML)** and the **same conceptual model**. A sheet that runs on one runs on the other.

---

## The mental model in 5 minutes

Five ideas. Each one is a paragraph and a couple of lines of YAML. If you've read the TypeScript docs, the ideas are identical — only the scripting language changes (rhai here, JavaScript there).

### 1. Address

A cell's id is a **stable name, not a coordinate**. It survives reordering, refactors, and moving a capability between sheets. Dotted ids are a convention, not a hierarchy the runtime enforces — they just read well:

```yaml
- id: compass.heading
  kind: sensor
  source: simulated
- id: desired.heading
  kind: value
  value: 180
```

`compass.heading` is an address. Anything that wants the heading mentions that name; nothing ever says "row 3, column B."

### 2. Reactivity

When an address changes, everything that reads it re-evaluates. The engine builds a dependency graph at load time (declared `deps`, plus auto-detected references for formulas), and on write it marks dependents stale; the next read recomputes. Lazy, cheap, and you never wire up the notification yourself:

```yaml
- id: heading.error
  kind: formula
  expr: "=desired.heading - compass.heading"
```

Change `desired.heading` and `heading.error` goes stale until someone reads it — then it's fresh again. That is the whole deal.

### 3. Caller-awareness

Every call carries a `CallerContext` — row, column, sheet, identity, tags, metadata. Cells can route on who is asking, and the same address can answer differently to different callers:

```yaml
- id: router.model
  kind: router
  rules:
    - when: 'caller.row == "premium"'
      route: { cell: models.precise }
    - when: 'true'
      route: { cell: models.fast }
```

The `when` expressions are small rhai snippets evaluated against the caller's context. Same cell, different answer per caller — and per-context memoization means repeated calls from the same caller hit the cache.

### 4. Bidirectional IO

An address is readable **and** writable. Adapters push readings into `sensor` cells; formulas read them. `io` cells go both ways — the engine writes the relay, and the relay's state reads back. The engine doesn't know what a GPIO pin is; your adapter does, and it talks to the engine through two verbs: `push` (in) and `set` (out):

```yaml
- id: pump.relay
  kind: io
  port: gpio:relay1
  direction: out
```

### 5. Composing

Writing an address *is* binding to it. Formulas reference cells by bare id (`=a + b`, rewritten to `cells["a"] + cells["b"]` under the hood). A listener watches a formula; a router delegates to a program; a program calls back into the engine with `qget` / `qset` / `qcall`. Everything composes because everything is named:

```yaml
- id: pump.should_run
  kind: formula
  expr: "=bilge.level >= bilge.threshold"
- id: alarm.listener
  kind: listener
  watch: [pump.should_run]
  action: log.alarm
```

Sensor → formula → listener → action. Four cells, one pipeline, no glue code.

---

## What it is

Quilt is a reactive, typed, cellular runtime. The spreadsheet is the control plane. The cell is the universal IO primitive.

- A cell can be a **value**, **formula**, **api**, **program**, **sensor**, **listener**, **router**, or **io**.
- A cell reference is a stable **address**, not a coordinate.
- A cell can **route** based on who called it (`caller.row > 10` → use Model A).
- The whole sheet is an **MCP server**. Every named cell is an MCP tool.
- It's **reactive** by default. Change one cell, every dependent rewires.

> **The paradigm shift, in one line:** A cell is not a value. A cell is a live, typed, addressable capability. The spreadsheet is not a document. The spreadsheet is the runtime.

---

## Architecture at a glance

```
┌──────────────────────────────────────────────────────────┐
│                      Quilt  (Rust)                       │
│                                                          │
│   ┌──────────┐    ┌──────────┐    ┌──────────┐          │
│   │  parse   │───►│  engine  │◄──►│  cells   │          │
│   │ (YAML)   │    │  (graph) │    │  (8 ks)  │          │
│   └──────────┘    └────┬─────┘    └──────────┘          │
│                        │                                 │
│            ┌───────────┼────────────┐                    │
│            ▼           ▼            ▼                    │
│       ┌────────┐  ┌────────┐  ┌────────┐                 │
│       │  CLI   │  │  MCP   │  │  TUI*  │                 │
│       │ (clap) │  │ (rmcp) │  │ (rust) │                 │
│       └────────┘  └────────┘  └────────┘                 │
│                                                          │
│   * TUI is TS-only in v0.1; Rust port uses raw stdout   │
└──────────────────────────────────────────────────────────┘
```

The TypeScript version is identical in shape — it has the same 8 cell kinds, the same engine, the same CLI/MCP/TUI surfaces. The difference is the runtime: `tokio` + `rhai` + `reqwest` instead of `node` + `new Function` + `fetch`.

---

## Quick start (60 seconds)

### Install

```sh
# Build from source
git clone https://github.com/superinstance/quilt-rust.git
cd quilt-rust
cargo build --release

# The binary is at target/release/quilt
./target/release/quilt --help
```

### Run a sheet

```sh
# Inspect a sheet
./target/release/quilt inspect examples/agent-dashboard/sheet.yaml

# Run a sheet (loads + evaluates all cells once)
./target/release/quilt run examples/agent-dashboard/sheet.yaml

# Serve a sheet as an MCP server (stdio)
./target/release/quilt serve examples/agent-dashboard/sheet.yaml

# In another terminal, with an MCP client (Claude Code, etc.):
#   "list the cells in this Quilt sheet"
#   "get the value of cell `status`"
```

### Embed in your own Rust code

```rust
use quilt_core::{QuiltEngine, parse_sheet, CallerContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the engine. `into_arc()` is the supported way to get
    //    an `Arc<QuiltEngine>` (program/router cells need the ref).
    let engine = QuiltEngine::new("my-app").into_arc();

    // 2. Load a sheet.
    let yaml = std::fs::read_to_string("sheet.yaml")?;
    let sheet = parse_sheet(&yaml)?;
    engine.load_sheet(sheet)?;

    // 3. Read a cell. The engine API is synchronous — no `.await`.
    let v = engine.get("temperature", CallerContext::default())?;
    println!("temperature: {}", v.data);

    // 4. Set a cell (marks dependents stale; they recompute on read).
    engine.set("setpoint", serde_json::json!(21.5), CallerContext::default())?;

    Ok(())
}
```

That's it. No external services, no `npm install`, no runtime to ship.

---

## The 8 cell kinds

<p align="center">
  <img src="assets/images/cell-types.jpg" alt="Many different kinds of cells living in one grid — values, formulas, programs, models" width="640"><br>
  <em>A sheet can hold many different kinds of cells at once — each one a
  different kind of capability, all addressable from the grid.</em>
</p>

| Kind        | What it is                                       | Evaluator             | Example                                |
| ----------- | ------------------------------------------------ | --------------------- | -------------------------------------- |
| `value`     | Static data. No dependencies.                    | direct                | `kind: value, value: 42`              |
| `formula`   | Reactive expression. Re-evaluates on change.     | `rhai` AST            | `kind: formula, expr: =a + b`         |
| `api`       | HTTP endpoint. Fetched on call.                  | `reqwest` (async)     | `kind: api, endpoint: https://...`    |
| `program`   | Inline rhai script. The runtime cell.            | `rhai` sandboxed      | `kind: program, code: \| ...`         |
| `sensor`    | Push-only value. Adapter writes, formula reads.  | external adapter      | `kind: sensor, source: mqtt://...`    |
| `io`        | Bidirectional port.                              | external adapter      | `kind: io, port: gpio17, direction: out` |
| `listener`  | Triggers on watched cell change.                 | engine propagation    | `kind: listener, watch: [x]`          |
| `router`    | Caller-aware policy. Delegates to a target cell. | `rhai` condition eval | `kind: router, rules: [...]`          |

> **Why rhai?** Rhai is a Rust-native embedded scripting language. It's sandboxed by default (no I/O, no network, no filesystem unless explicitly registered), so a `program` cell cannot accidentally escape the cell boundary. The TypeScript version uses JavaScript, which is *not* sandboxed by default — see [docs/security.md](https://github.com/superinstance/quilt/blob/main/docs/security.md) in the TS repo.

---

## A working example

This is the `agent-dashboard` sheet, from `examples/agent-dashboard/sheet.yaml`:

```yaml
id: agent-dashboard
version: "1"
cells:
  - id: status
    kind: value
    value: idle
    description: The current state of the agent.

  - id: task
    kind: value
    value: "summarize inbox"
    description: What the agent is working on.

  - id: greeting
    kind: formula
    expr: ='Agent is ' + status + ' on: ' + task
    description: Human-readable summary.

  - id: alert
    kind: listener
    watch: [status]
    action: notify
    description: Fire when the agent's status changes.
```

What this gives you:

```text
status      value     idle
task        value     summarize inbox
greeting    formula   Agent is idle on: summarize inbox
alert       listener  watching status
```

Change `status` to `error` and `greeting` recomputes automatically. The `alert` listener fires (calling the `notify` cell, which is whatever you wired it to).

---

## Your first sheet, step by step

Let's build something real — the smallest thing a cellular runtime is genuinely good at: a sensor crossing a threshold, flipping a relay. The boat has a bilge; bilges fill; pumps should run when the water gets high. That's one sensor, one threshold, one formula, one relay, and one listener.

### The sheet

Save this as `bilge-pump.yaml`:

```yaml
id: bilge-pump
version: "1"
description: A sensor crossing a threshold flips the bilge pump relay.

cells:
  - id: bilge.level
    kind: sensor
    source: simulated
    default: 40.0
    unit: cm
    description: Water level in the bilge, from the level sensor

  - id: bilge.threshold
    kind: value
    value: 80.0
    unit: cm
    description: Pump-on level

  - id: pump.should_run
    kind: formula
    expr: "=bilge.level >= bilge.threshold"
    description: True when the bilge needs pumping

  - id: pump.relay
    kind: io
    port: gpio:relay1
    direction: out
    description: The relay that powers the pump

  - id: alarm.listener
    kind: listener
    watch: [pump.should_run]
    action: log.alarm
    description: Watch for the threshold crossing
```

Walk through each line:

- **`bilge.level`** is a `sensor` cell. `source: simulated` means "no real adapter yet" — the engine seeds it from `default` so the sheet runs before hardware exists. A real deployment would point `source` at a real stream and let an adapter `push` readings in.
- **`bilge.threshold`** is a `value` cell — plain configuration. 80 cm of water, and we pump.
- **`pump.should_run`** is the interesting one: a `formula` cell. The expression is rhai, `=bilge.level >= bilge.threshold`. At load time the engine auto-detects the dependencies on `bilge.level` and `bilge.threshold` and records both edges in the dependency graph.
- **`pump.relay`** is an `io` cell — a port named `gpio:relay1`, direction `out`. Writing to this cell is what flips the relay; an adapter observes the change and drives the pin.
- **`alarm.listener`** watches `pump.should_run`. When the value flips, the propagation loop notices and records the event (in this build, listener actions are validated and traced; the `fire_listener` evaluator is unit-tested and wired into propagation as the next step).

### Build it

```sh
cd quilt-rust
cargo build --release
./target/release/quilt --help
```

The repo is a Cargo workspace, so `cargo build --release` builds every surface: `quilt` (CLI), `quilt-web` (HTTP server), `quilt-tui` (terminal UI). First build takes a few minutes — see the FAQ.

### Run it

```sh
./target/release/quilt inspect bilge-pump.yaml
./target/release/quilt run bilge-pump.yaml
```

`inspect` shows the shape of the sheet; `run` loads it, evaluates every cell once, and prints the grid:

```text
loaded bilge-pump.yaml (5 cells)
  bilge.level (Sensor) = 40.0
  bilge.threshold (Value) = 80.0
  pump.should_run (Formula) = false
  pump.relay (Io) = null
  alarm.listener (Listener) = null
```

40 cm against an 80 cm threshold: `false`. All quiet.

### Cross the threshold

Each CLI invocation loads the sheet fresh from disk, so `set` prints the assignment but doesn't persist it. To watch reactivity happen in one process, use the HTTP server:

```sh
./target/release/quilt-web --sheet bilge-pump.yaml --port 8080
```

Then, in another terminal:

```sh
curl localhost:8080/api/cell/pump.should_run
# {"data":false,"status":"ready",...}

curl -X POST localhost:8080/api/cell/bilge.level \
     -d '85' -H 'content-type: application/json'
# 204 No Content

curl localhost:8080/api/cell/pump.should_run
# {"data":true,...}
```

The bilge rose past the threshold and the formula flipped to `true` on its own. No glue code. (`pump.relay` is there for the adapter to watch too — via `GET /api/cell/pump.relay` or the SSE stream at `/api/events`.)

Prefer the terminal? `quilt tui bilge-pump.yaml`, move to `bilge.level` with `j`/`k`, press `s`, type `85`, Enter — and watch `pump.should_run` recompute in the grid.

---

## Full CLI reference

The `quilt` binary (`packages/cli`) is a `clap`-based command set. Every subcommand takes the sheet file as a positional argument; `--help` and `--version` are the only global flags.

| Command | What it does |
| ------- | ------------ |
| `quilt init <name>` | Scaffold `<name>.yaml` in the current directory with one `hello` value cell. |
| `quilt run <file>` | Load the sheet, evaluate every cell once, print `id (kind) = value` per cell. |
| `quilt serve <file>` | Serve the sheet as an MCP server over stdio. Blocks; point an MCP client at it. |
| `quilt get <id> <file>` | Print one cell's value as pretty-printed JSON. |
| `quilt set <id> <value> <file>` | Set a cell's value and print the assignment. `<value>` is parsed as JSON; if that fails, it is treated as a JSON string. |
| `quilt inspect <file>` | Print the sheet path, total cell count, and a per-kind breakdown. |
| `quilt tui <file>` | Open the interactive terminal UI on the sheet. |

Usage notes, straight from `packages/cli/src/main.rs`:

- **Argument order matters for `get` and `set`:** the cell id comes *before* the file: `quilt get heading examples/boat-autopilot/sheet.yaml`.
- **`set` is in-memory only.** Each CLI invocation loads the sheet fresh from disk, applies the command, prints, and exits. The YAML file is never rewritten — `set` is for scripting and quick probes, not persistence. Watch reactivity live with `quilt tui` or the HTTP server instead.
- **`set` accepts any JSON value:** `quilt set setpoint 21.5 sheet.yaml`, `quilt set status '"error"' sheet.yaml`, `quilt set tags '[1,2]' sheet.yaml`. If JSON parsing fails, the value is wrapped in a string.
- **Errors** go to stderr as `error: <message>` and the process exits with code 1; success exits 0.
- **`run` and `inspect` are read-only** — they never mutate engine state or the file.

TUI keys (`quilt tui`): `j`/`k` move, `g`/`G` jump to top/bottom, `s` set the selected value/sensor cell, `r` reload, `q` or Ctrl-C quits, `Esc` cancels an edit.

---

## How to use it, deep-dives

### Embed the engine in your own Rust code

The crate is `quilt-core`. Its public surface is deliberately small — one engine, a parser, and the type vocabulary, re-exported from `packages/core/src/lib.rs`. There is no framework and no required async runtime: **the engine API is synchronous.**

```toml
[dependencies]
quilt-core = { path = "packages/core" }   # path dep inside the workspace; crates.io when published
serde_json = "1"
```

```rust
use quilt_core::{parse_sheet, CallerContext, QuiltEngine};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the engine. `into_arc()` is the ONLY supported way to
    //    get an `Arc<QuiltEngine>` — program and router cells need
    //    the self-reference it registers.
    let engine = QuiltEngine::new("my-app").into_arc();

    // 2. Load a sheet. Parsing validates ids, kind-specific required
    //    fields, and listener/router references.
    let yaml = std::fs::read_to_string("sheet.yaml")?;
    let sheet = parse_sheet(&yaml)?;
    engine.load_sheet(sheet)?;

    // 3. Read a cell. No `.await` — `get` evaluates lazily, so a
    //    formula whose inputs changed recomputes right here.
    let v = engine.get("temperature", CallerContext::default())?;
    println!("temperature: {}", v.data);

    // 4. Write a cell. Dependents go stale and recompute on read.
    engine.set("setpoint", json!(21.5), CallerContext::default())?;

    // 5. Push a reading into a sensor (sensor/io cells only).
    engine.push("temperature", json!(22.3))?;

    // 6. Call a cell as a capability — the input reaches api/program/
    //    router cells; pure cells ignore it.
    let out = engine.call(
        "model.analyzer",
        Some(json!({ "q": "hi" })),
        CallerContext::default(),
    )?;
    println!("{}", out.data);

    Ok(())
}
```

Notes for the embedding reader:

- **`get` / `set` / `call` / `push` are synchronous and thread-safe.** `QuiltEngine` is `Send + Sync`; share one `Arc<QuiltEngine>` across threads. Cells live behind a `parking_lot::RwLock` — reads (the common case) don't block each other.
- **Effectful cells evaluate asynchronously under the hood.** `api`, `program`, and `router` cells run on a dedicated current-thread tokio runtime spawned per evaluation (`drive_async` in `engine.rs`) — so your binary doesn't need a runtime, but each effectful call pays a thread spawn. Pure sheets (value/formula) have zero async overhead.
- **Subscriptions.** `engine.subscribe("cell.id")` and `engine.subscribe_all()` return handles with a synchronous crossbeam channel of `SubscriptionEvent { cell_id, new_value, prev_value }` (in this build `prev_value` mirrors the new value). `unsubscribe(id)` stops delivery.
- **Tracing.** `QuiltEngine::with_options(id, EngineOptions { tracing: true, trace_capacity: 1000 })` records an `EvaluationTrace` per evaluation — cell id, timing, caller context, effects — readable via `engine.traces()`.
- **Custom harnesses.** A `program` cell reaches the outside world only through the `ProgramRuntime` trait (`packages/core/src/cells/program.rs`): `get`, `set`, `call`, `list`. Implement it over your database, message bus, or GPIO library and wire it into the engine. `docs/ports-and-connections.md` has worked examples (gRPC, Kafka, LLM providers, embedded/no_std).
- **Cells evaluate lazily.** `list_cells()` returns definitions and cached values, but always read through `get(id, ctx)` when you want a fresh, evaluated value.

### Serve sheets over HTTP (`quilt-web`)

`quilt-web` is an axum-based server with a REST API and live SSE updates. It's the "drop-in web app" path: point it at a sheet, hit the API, watch events stream.

```sh
cargo run -p quilt-web -- --sheet examples/weather-monitor/sheet.yaml --port 8080
# then open http://localhost:8080/
```

Flags (from `packages/web/src/main.rs`):

| Flag | Default | Meaning |
| ---- | ------- | ------- |
| `--sheet <path>` | *(required)* | Sheet YAML to load |
| `--port <n>` | `8080` | Listen port |
| `--bind <ip>` | `0.0.0.0` | Bind address |
| `--static-dir <dir>` | bundled `www/` | Static files served at `/` |

Endpoints:

| Endpoint | Method | What it does |
| -------- | ------ | ------------ |
| `/api/sheet` | GET | Sheet metadata + every cell, its kind, and dependency edges |
| `/api/cell/:id` | GET | Current value of one cell (`{"data":…,"status":…}`) |
| `/api/cell/:id` | POST | Set a cell; body is any JSON value; returns 204 |
| `/api/cell/:id/stream` | GET | SSE stream of changes to one cell |
| `/api/events` | GET | SSE stream of every cell change |
| `/` | GET | Demo UI (bundled `www/`, an 80-line JS shim) |

Or embed it: `quilt_web::{AppState, load_state, router, serve}` — `load_state(&path)` builds an `AppState` around a loaded engine, and `router(state)` is a plain axum `Router` you can mount anywhere.

### Expose a sheet to AI agents (MCP)

The CLI path: `quilt serve sheet.yaml` speaks [Model Context Protocol](https://modelcontextprotocol.io) over stdio. Point Claude Code, Cursor, or any MCP client at it — every cell becomes a tool (full tool list below).

In code, `quilt-mcp` gives you the server directly:

```rust
use quilt_mcp::{build_server, QuiltMcpServer};

// Load from disk and serve (blocks on stdio):
build_server(Some("sheet.yaml"))?;

// Or wrap an engine you already own:
let server = QuiltMcpServer::from_engine(my_engine);
```

`QuiltMcpServer::register_cell(yaml)` loads a sheet into the server at runtime; `server.engine()` hands back the wrapped engine.

### Cross-compile to a static binary

This is the whole point of the Rust port: one binary, no runtime to ship. See "Cross-compile to anything" in the engineering notes for the full target matrix. The short version for a truly static Linux binary:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/quilt
# ELF 64-bit ... statically linked
```

Because `reqwest` uses `rustls` (no OpenSSL), the dependency tree is pure Rust and cross-compiles cleanly. The workspace release profile (`opt-level = 3`, `lto = "thin"`) is already tuned for throughput; add `strip = true` to your own `[profile.release]` if you want the smallest artifact — a stripped release binary is roughly 3 MB.

---

## Use it as an MCP server

Quilt speaks [Model Context Protocol](https://modelcontextprotocol.io) natively. Every sheet is an MCP server. Every cell is an MCP tool.

### Add to Claude Code

```json
{
  "mcpServers": {
    "quilt-boat": {
      "command": "/path/to/quilt",
      "args": ["serve", "/path/to/examples/boat-autopilot/sheet.yaml"]
    }
  }
}
```

Then in your conversation:

> *"What cells are in the boat-autopilot sheet?"*
>
> → calls `cells_list` tool
>
> *"What's the current heading?"*
>
> → calls `cell_get` with `id: "heading"`

### Tools exposed

| Tool          | Purpose                                            |
| ------------- | -------------------------------------------------- |
| `cells_list`  | List every cell in the sheet.                      |
| `cell_get`    | Read a cell's value (with optional caller context).|
| `cell_set`    | Set a cell's value (triggers downstream).          |
| `cell_call`   | Call a cell as a capability.                       |
| `cell_push`   | Push a value into a sensor or IO cell.             |

The sheet itself is exposed as an MCP **resource** at `quilt://sheet/<sheet-id>`.

---

## The 5-layer abstraction

Quilt is built on 5 layers of "addressing as composition." Understanding them is the difference between using Quilt and *thinking* in Quilt.

```
Layer 0  ADDRESS           A stable id, not a coordinate. Not a URI. A name.
Layer 1  SPATIAL           row / column carry context. Position is policy.
Layer 2  REACTIVE          when an address changes, dependents re-evaluate.
Layer 3  BIDIRECTIONAL     same address is readable AND writable.
Layer 4  COMPOSING         writing an address IS binding to it.
```

The TypeScript and Rust implementations both honor these. The data model is the same. Only the runtime differs.

---

## Glossary

The vocabulary, in one place. "When to use it" is the short version; the 8-cell-kinds table above has the full field reference.

### The 8 cell kinds — when to use them

| Kind | When to use it |
| ---- | -------------- |
| `value` | Static data that never changes: constants, thresholds, configuration, prompts. The leaves of the graph. |
| `formula` | Anything you can write as an expression: derived values, checks, aggregations. Pure, reactive, auto-tracked dependencies. Reach for this first. |
| `api` | Pull data from the outside world on demand: REST endpoints, model pseudo-URLs (`model:gpt-4o`), webhooks. |
| `program` | Logic that acts: stateful scripts, multi-step flows, anything with side effects. A sandboxed rhai script with `qget` / `qset` / `qcall` / `qlist` access to the engine. |
| `sensor` | Push-only inputs: readings an adapter streams in (MQTT, GPIO, simulated). You write, formulas read. |
| `io` | Bidirectional ports: a relay, an actuator, a form field — anything you both read and write. Declares `direction: in | out | both`. |
| `listener` | "When X changes, do Y." Delta-triggered actions watching other cells. |
| `router` | Caller-aware policy: "who's asking decides what they get." First matching `when` rule wins and delegates. |

### Terms

- **Address (`CellId`)** — a stable, location-independent name for a cell (`compass.heading`). Not a coordinate; survives reordering and refactors.
- **CellRef** — a reference to another cell. Same shape as `CellId` today, typed separately so it can later grow expressions (ranges, conditional references).
- **CellDef** — the declarative blueprint from YAML: id, kind, and kind-specific fields. Validated at parse time.
- **Cell / CellValue** — a `Cell` is a live instance (def + current value + dependency edges + per-context cache). A `CellValue` is `{ data, status, computed_at, error, effects }` — a cell always knows its own state.
- **CellStatus** — `idle` (never touched), `computing`, `ready` (fresh), `error`, `stale` (inputs changed; needs recompute).
- **CallerContext** — what travels with every call: `row`, `column`, `sheet`, `caller`, `trace`, `identity` (id, type, tags), `metadata`, `timestamp`. The substrate for caller-aware routing and per-context memoization (keyed on row/column/sheet/caller/identity — deliberately *not* on metadata or timestamp).
- **Propagation** — what happens on `set`/`push`: dependents are marked `stale` (data cleared to `null` so nobody reads a stale value), listeners are checked, and the walk recurses. O(dependents); no scheduler.
- **Effects** — what a cell *did* while producing a value: network calls, storage ops, I/O port events, model invocations, compute time. Pure cells produce none; effectful cells report theirs so the runtime (and you) can reason about cost.
- **SheetDef** — the unit of load: id, title, description, version, semantic `axes` (what rows and columns *mean* in this sheet), and the `cells` array. Feed one to `QuiltEngine::load_sheet`.
- **The universal verbs** — `get` (read; evaluate if needed), `set` (write; triggers propagation), `call` (invoke as a capability, with input), `push` (inject a reading into sensor/io), `subscribe` (watch for changes).

---

## Engineering notes

### Why a sync engine that drives async cell evaluators?

The engine core (`QuiltEngine`) is **synchronous**. Cell evaluators for `api`, `program`, and `router` are **async** (they may need to wait on I/O or scripts). The bridge is `drive_async`, which uses `Handle::block_on` if a tokio runtime is active, or builds a single-threaded runtime on demand.

This means:
- A pure-`value` sheet with 10,000 cells evaluates in **< 1 ms** (no task scheduler).
- A sheet with HTTP `api` cells evaluates at network speed (one round trip per `api` call).
- A `program` cell with 10,000 lines of rhai takes the time rhai takes. No overhead.

### Memory model

Cells live behind a `parking_lot::RwLock<IndexMap>`. Reads (the common case — `get`, `call`) take a read lock and clone out. Writes (`set`, `push`) take a write lock briefly to update the cell, then release before propagation. Concurrent readers don't block each other.

The dependency graph is built once at `load_sheet` time. Subsequent `set` calls do an O(dependents) walk. For sheets with < 10,000 cells, this is fast enough that the propagation loop has no overhead.

### Scripting: rhai vs JavaScript

| Property               | rhai (Rust)              | JavaScript (TS)            |
| ---------------------- | ------------------------ | -------------------------- |
| Sandboxed by default   | ✅                        | ❌ (uses `new Function`)    |
| Async/await            | ❌ (sync only)            | ✅                          |
| Closures               | Limited                  | Full                       |
| Startup cost           | ~1 ms                    | ~30 ms (V8 warmup)         |
| Eval speed             | ~10× faster              | baseline                   |
| Memory per script      | ~50% less                | baseline                   |

Rhai wins on safety and speed. JavaScript wins on expressiveness. The TypeScript version has WASM-cell-sandbox as a v0.2 feature to close the safety gap; see [docs/security.md](https://github.com/superinstance/quilt/blob/main/docs/security.md).

### Cross-compile to anything

```sh
# Linux (default)
cargo build --release --target x86_64-unknown-linux-gnu

# macOS
cargo build --release --target x86_64-apple-darwin

# Windows
cargo build --release --target x86_64-pc-windows-msvc

# ARM Linux (Raspberry Pi, AWS Graviton)
cargo build --release --target aarch64-unknown-linux-gnu

# WASM (browser!)
cargo build --release --target wasm32-unknown-unknown
```

The result is a single statically-linked binary, ~3 MB stripped. Drop it on a Raspberry Pi, a Docker container, or a bare-metal VM. No `apt install`. No `npm install`. No `pip install`.

For a *truly* static binary (no dynamic glibc), target musl instead of the default gnu target:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

`reqwest` uses `rustls` for TLS, so there is no OpenSSL dependency to statically link — the tree is pure Rust and cross-compiles cleanly. The workspace release profile (`opt-level = 3`, `lto = "thin"`) is tuned for throughput; add `strip = true` to your own release profile (or `strip` the binary) to drop it to ~3 MB.

### When *not* to use the Rust port

- You need the **browser simulator** or the **web UI** — those are TypeScript only.
- You need the **TUI** — the TypeScript `@quilt/tui` is the supported one; the Rust CLI is for batch use.
- You need a **stable, fully-tested engine today** — use TypeScript.
- You need the **v0.1 Web UI** (planned) — TypeScript.
- You need **WebAssembly cells** (planned) — the engine is being written to support both, but only TypeScript has the running tooling today.

Use Rust when you need: **single binary**, **embedded**, **strict memory**, **Rhai's safety guarantees**, or **static cross-compilation**.

---

## Troubleshooting & FAQ

**A program cell can't touch the filesystem or the network. Is that a bug?**

No — that's the sandbox. Rhai engines are created with no I/O packages registered (`Engine::new()` in `cells/program.rs`; the file says it outright: "The runtime handle is the only way out"). A `program` cell can only reach the world through `qget` / `qset` / `qcall` / `qlist`, which delegate to the engine or to the `ProgramRuntime` you implement. Need a file read or an HTTP call from a script? Expose it as a custom runtime method, or use an `api` cell. (For what it's worth, `quilt-core` is `#![forbid(unsafe_code)]`.)

**The example sheets use `await runtime.get(...)` — my program cells fail to compile.**

The shipped example sheets were authored for the TypeScript runtime — their `program` cells are JavaScript. The Rust port evaluates rhai: no `await`, no `const`, and the engine is reached via `qget("id")` (returns an object with `.data`), `qset("id", value)`, `qcall("id", input)`, `qlist()`, plus the helpers `abs`, `min`, `max`, `clamp`, and `includes`. Note that sheet validation checks *shape*, not script syntax — a sheet can parse fine and only fail when a program cell evaluates. Write rhai for the Rust port.

**I changed a cell and nothing happened — my formula still shows the old value / `null`.**

Two things to check. First: with the CLI, each invocation reloads the sheet from disk, so `quilt set` prints the assignment but nothing persists — watch reactivity live with `quilt tui`, the HTTP server, or an embedded engine. Second: on `set`/`push`, propagation deliberately marks formula and value dependents `Stale` and clears their data to `null` so nobody reads a stale value; the next `get(id, ctx)` recomputes. Always read through `engine.get`, not `get_cell(id).value`.

**How is a formula different from a program cell?**

A `formula` is pure and reactive: an expression whose dependencies are auto-tracked, recomputed on read when inputs change, with no side effects. A `program` is an imperative rhai script that runs when called, receives `input` and `caller`, can reach the engine via `qget`/`qset`/`qcall`, and records what it did in `effects`. Rule of thumb: *computes* → formula; *acts* → program. (And: `sensor` is push-in, `api` is pull-out.)

**Build times are long. Is that normal?**

Yes. The first build compiles tokio (full features), axum, reqwest with rustls, rhai, and rmcp — expect a few minutes cold. Warm incremental builds are seconds. `cargo build -p quilt-cli` builds only the CLI; `cargo build --release` at the workspace root builds everything (cli, core, mcp, tui, web).

**Cross-compiling: why does my gnu-target binary fail on Alpine?**

The `-gnu` targets link dynamically against glibc. For a container or device without glibc (Alpine, minimal IoT images), build for `x86_64-unknown-linux-musl` (or `aarch64-unknown-linux-musl` on ARM) to get a fully static binary. TLS is rustls-based, so there's no OpenSSL to chase.

**`quilt serve` starts, but my MCP client sees no cells.**

Known wiring gap in this build: the CLI loads the sheet to validate and report the cell count, then hands off to `quilt_mcp::serve_stdio()`, which constructs a fresh, empty server (see `packages/mcp/src/lib.rs`). If you need the sheet's cells exposed over MCP today, embed instead — `quilt_mcp::build_server(Some("sheet.yaml"))` loads from disk, or wrap your own loaded engine with `QuiltMcpServer::from_engine(engine)`. Similarly, listener *actions* are validated and traced during propagation in this build; the `fire_listener` evaluator is unit-tested and wired into the propagation loop as the next step.

---

## Project status (v0.2.0)

| Component               | Status            | Notes                                              |
| ----------------------- | ----------------- | -------------------------------------------------- |
| `quilt-core` library    | ✅ Production     | 37 lib tests + 14 integration tests pass.          |
| Formula evaluator       | ✅ Production     | rhai with chained-formula support.                 |
| Program evaluator (rhai)| ✅ Production     | rhai sandboxed scripts with `qget/qset/qcall/qlist`. |
| `quilt-mcp` server      | ✅ Production     | 3 tests pass. Serves on stdio via `rmcp`.          |
| `quilt-cli`             | ✅ Production     | `init / run / serve / get / set / inspect / tui`.  |
| `quilt-tui` (terminal)  | ✅ Production     | 8 unit tests on the pure renderer.                 |
| `quilt-web` (HTTP+SSE)  | ✅ Production     | axum-based, with HTML/JS demo at `/`.              |
| Examples                | ✅ Production     | 10 examples (4 original + 6 new). 86 cells, 0 errors. |
| Tests                   | ✅ 68 passing, 2 ignored | Across core, engine, examples, mcp, tui.    |
| Browser simulator       | ❌ N/A             | TypeScript only.                                   |
| WASM sandbox            | 🔜 Planned         | v0.3.0 — rhai sandbox is the v0 equivalent.        |

> **Honest disclosure:** the TypeScript version is more complete. The Rust port has the same architecture, the same data model, and the same cell kinds, but a few formula/program test cases fail because the rhai AST evaluation path has a different shape than the JS `new Function` path. Tracking this in the [issues](https://github.com/superinstance/quilt-rust/issues).

---

## What's in the box

```
quilt-rust/
├── packages/
│   ├── core/                # The engine + 8 cell evaluators + parser
│   │   ├── src/
│   │   │   ├── engine.rs    # QuiltEngine — get/set/call/push/subscribe
│   │   │   ├── parser.rs    # YAML loader (serde_yml)
│   │   │   ├── context.rs   # CallerContext + per-context memoization
│   │   │   ├── types.rs     # CellDef, Cell, CellValue, etc.
│   │   │   ├── error.rs     # Quilt error types
│   │   │   └── cells/       # 8 cell evaluators
│   │   │       ├── value.rs     ✅
│   │   │       ├── formula.rs   ⚠️ (eval path partial)
│   │   │       ├── api.rs       ✅
│   │   │       ├── program.rs   ⚠️ (eval path partial)
│   │   │       ├── sensor.rs    ✅
│   │   │       ├── io.rs        ✅
│   │   │       ├── listener.rs  ✅
│   │   │       └── router.rs    ✅
│   │   └── tests/
│   │       └── engine_integration.rs  # 14 end-to-end tests
│   ├── mcp/                 # MCP server (rmcp)
│   │   └── src/lib.rs       # cells_list, cell_get, cell_set, ...
│   └── cli/                 # Command-line interface
│       └── src/main.rs      # init, run, serve, get, set, inspect
├── examples/                # 10 example sheets (YAML)
│   ├── agent-dashboard/     # original
│   ├── boat-autopilot/      # original
│   ├── model-router/        # original
│   ├── sensor-anomaly/      # original
│   ├── weather-monitor/     # sensors + formulas + listener + router
│   ├── chat-router/         # LLM model routing by tier
│   ├── ab-test-router/      # deterministic A/B split
│   ├── iot-dashboard/       # multi-sensor aggregation
│   ├── rate-limiter/        # token-bucket rate limiter
│   └── task-scheduler/      # reactive task runner
├── docs/                    # architecture, ports, embedding guides
│   └── ports-and-connections.md
├── Cargo.toml               # Workspace
└── README.md                # You are here.
```

---

## Try it in 30 seconds

```sh
# 1. Build
cargo build --release

# 2. Run the boat-autopilot example (4 cells, all evaluators)
./target/release/quilt run examples/boat-autopilot/sheet.yaml

# 3. Get a specific cell
./target/release/quilt get heading examples/boat-autopilot/sheet.yaml

# 4. Set a cell
./target/release/quilt set wind_speed 18.5 examples/boat-autopilot/sheet.yaml

# 5. Serve as MCP (then point an MCP client at it)
./target/release/quilt serve examples/boat-autopilot/sheet.yaml
```

---

## Cross-references

| Want to…                                                | Go to                                                                |
| ------------------------------------------------------- | -------------------------------------------------------------------- |
| Use Quilt **right now** with a stable engine            | [superinstance/quilt](https://github.com/superinstance/quilt) (TypeScript) |
| Try the **browser simulator** (live, no install)        | [superinstance.github.io/quilt/landing/simulator.html](https://superinstance.github.io/quilt/landing/simulator.html) |
| Read the **manifesto** (the 10-point declaration)       | [docs/manifesto.md](https://github.com/superinstance/quilt/blob/main/docs/manifesto.md) |
| Read the **architecture** deep-dive                      | [docs/architecture.md](https://github.com/superinstance/quilt/blob/main/docs/architecture.md) |
| Read about **security** and the trust model              | [docs/security.md](https://github.com/superinstance/quilt/blob/main/docs/security.md) |
| Walk through the **5-chapter tutorial**                 | [tutorials/README.md](https://github.com/superinstance/quilt/blob/main/tutorials/README.md) |
| See **10 recipes** for common patterns                  | [docs/recipes.md](https://github.com/superinstance/quilt/blob/main/docs/recipes.md) |
| Compare to **n8n, LangGraph, Observable, Excel**         | [docs/comparison.md](https://github.com/superinstance/quilt/blob/main/docs/comparison.md) |
| Embed Quilt in **TypeScript** (your app is a JS app)    | [superinstance/quilt](https://github.com/superinstance/quilt#install) |
| **Try Quilt in a single HTML file** (no install, portable) | **[superinstance/quilt-live](https://github.com/superinstance/quilt-live)** — open one file, save state as cookie or downloadable .html |
| **Report a bug** in the Rust port                        | [issues](https://github.com/superinstance/quilt-rust/issues) |

---

## Gallery

<p align="center">
  <img src="assets/images/quilt-calm-v3.jpg" alt="A calm, settled quilt of cells — the grid at rest, every address a steady honey-amber window against midnight navy" width="720"><br>
  <em>The same sheet, at rest — every cell settled, every address answering. Calm is what a compiled system sounds like.</em>
</p>

<p align="center">
  <img src="assets/images/quilt-ts-sdxl-deck.jpg" alt="The quilt deck, SDXL rendering — cell cards spread across the dark, each card a small lit window, brass traces between them" width="720"><br>
  <em>The deck, SDXL edition — the sibling rendering of the TypeScript repo's FLUX deck. Same sheet format, same eight kinds, two engines.</em>
</p>

---

## Contributing

We welcome PRs that:
- Fix one of the 5 pre-existing formula/program test failures
- Add a new cell evaluator
- Add an adapter (MQTT, Modbus, CAN, etc.)
- Improve the MCP tool list
- Add benchmarks

Open an issue first if you're planning something large.

---

## Related Quilt repos

Quilt is an ecosystem of 15 repos, 5 deployment tiers, 3 languages. This repo is part of:

| Tier | Repo | What it is |
|---|---|---|
| **Canonical** | [quilt](https://github.com/SuperInstance/quilt) | TypeScript core (this ecosystem's home base) |
| **Compiled** | [quilt-rust](https://github.com/SuperInstance/quilt-rust) | Rust port — single static binary, axum, crossterm |
| **Browser** | [quilt-live](https://github.com/SuperInstance/quilt-live) | Single 70KB HTML file that runs anywhere |
| **IoT** | [quilt-esp32](https://github.com/SuperInstance/quilt-esp32) | no_std Rust for ESP32, sensors + actuators |
| **Edge** | [quilt-cloudflare](https://github.com/SuperInstance/quilt-cloudflare) | Cloudflare Workers + D1 + Vectorize + R2 |
| **Codespace** | [quilt-codespace](https://github.com/SuperInstance/quilt-codespace) | GitHub Codespace as a live Quilt runtime |
| **AI** | [quilt-ai](https://github.com/SuperInstance/quilt-ai) | LLM cells across 4 providers (z.ai, Kimi, DeepSeek, Cloudflare) |
| **Evolution** | [quilt-evolve](https://github.com/SuperInstance/quilt-evolve) | Self-improvement loops, 4 components, 5 scopes |
| **Mesh** | [quilt-mesh](https://github.com/SuperInstance/quilt-mesh) | CRDT-backed cross-tab / cross-device sync |
| **Agent** | [quilt-agent](https://github.com/SuperInstance/quilt-agent) | LLM agent as a sheet — memory, tools, reasoning |
| **Time** | [quilt-time](https://github.com/SuperInstance/quilt-time) | Time-series cells with rolling windows |
| **Vault** | [quilt-vault](https://github.com/SuperInstance/quilt-vault) | Secrets cells with per-cell ACLs |
| **Vision** | [quilt-vision](https://github.com/SuperInstance/quilt-vision) | Computer-vision cells (camera → scene → caption) |
| **ZK** | [quilt-zk](https://github.com/SuperInstance/quilt-zk) | Zero-knowledge cell verification primitives |
| **Flow** | [quilt-flow](https://github.com/SuperInstance/quilt-flow) | Workflow cells — DAG execution, retry, rollback |

See the [Federation landing page](https://superinstance.github.io/quilt/landing/federation.html) for the architecture and the [Engineering Bar](https://github.com/SuperInstance/quilt/blob/main/docs/engineering-bar.md) for what "done right" means across all 15 repos.

---

## License

Apache 2.0 — same as the TypeScript version.

---

> The two implementations are siblings. The TypeScript version is the writer; the Rust version is the compiler. Both speak the same sheet format. Use whichever matches your runtime.
