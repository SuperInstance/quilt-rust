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

**[Live simulator ⚡](https://superinstance.github.io/quilt/landing/simulator.html)** · **[TypeScript version →](https://github.com/superinstance/quilt)** · **[Read the manifesto →](https://github.com/superinstance/quilt/blob/main/docs/manifesto.md)**

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the engine.
    let engine = QuiltEngine::new("my-app").into_arc();

    // 2. Load a sheet.
    let yaml = std::fs::read_to_string("sheet.yaml")?;
    let sheet = parse_sheet(&yaml)?;
    engine.load_sheet(sheet)?;

    // 3. Read a cell.
    let v = engine.get("temperature", CallerContext::default()).await?;
    println!("temperature: {}", v.data);

    // 4. Set a cell (triggers downstream).
    engine.set("setpoint", serde_json::json!(21.5), CallerContext::default()).await?;

    Ok(())
}
```

That's it. No external services, no `npm install`, no runtime to ship.

---

## The 8 cell kinds

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

### When *not* to use the Rust port

- You need the **browser simulator** or the **web UI** — those are TypeScript only.
- You need the **TUI** — the TypeScript `@quilt/tui` is the supported one; the Rust CLI is for batch use.
- You need a **stable, fully-tested engine today** — use TypeScript.
- You need the **v0.1 Web UI** (planned) — TypeScript.
- You need **WebAssembly cells** (planned) — the engine is being written to support both, but only TypeScript has the running tooling today.

Use Rust when you need: **single binary**, **embedded**, **strict memory**, **Rhai's safety guarantees**, or **static cross-compilation**.

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
| **Report a bug** in the Rust port                        | [issues](https://github.com/superinstance/quilt-rust/issues) |

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

## License

Apache 2.0 — same as the TypeScript version.

---

> The two implementations are siblings. The TypeScript version is the writer; the Rust version is the compiler. Both speak the same sheet format. Use whichever matches your runtime.
