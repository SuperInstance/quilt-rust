# ◳ Quilt (Rust port)

**The Rust port of [Quilt](https://github.com/superinstance/quilt) — a reactive, typed, cellular runtime where every cell is a live, addressable capability.**

> Status: **v0.1.0-alpha — foundation laid, engine in progress.** The TypeScript version is complete and shipping; this is the Rust sibling.

---

## What is Quilt?

A cell is not a value. A cell is a *socket* — a stable, typed, addressable capability. The spreadsheet is the runtime. Change one cell, every dependent rewires.

Quilt is what you get when you take the spreadsheet model seriously: cells compose by reference, rows/columns are policy dimensions, and the whole grid is an MCP server that any AI agent can call.

Read the [TypeScript repo's manifesto](https://github.com/superinstance/quilt/blob/main/docs/manifesto.md) for the full vision. The Rust port is API-compatible at the sheet level: the same `.cellflow.yaml` files work in both implementations.

---

## Status

This is the Rust port. It is in active development.

### What's done (foundation)

- ✅ **Type vocabulary** (`types.rs`, ~800 lines) — `Cell`, `CellDef`, `CellValue`, `CellStatus`, `Effect`, `CallerContext`, `RouterRule`, `SheetDef`, `Subscription`, `EvaluationTrace`. The schema of the universe. Heavily commented; every type has a doc-comment explaining its role.
- ✅ **Error types** (`error.rs`, ~200 lines) — typed errors via `thiserror`. Every error variant is documented.
- ✅ **Caller context** (`context.rs`, ~450 lines) — `empty_context`, `extend_context`, `context_key`, `eval_when`. The primitive that makes routing possible. Heavily commented.
- ✅ **Cell evaluators** (`cells/`, ~1,500 lines) — `value`, `formula`, `api`, `program`. Each is a complete implementation with unit tests.
- ✅ **Workspace setup** — `Cargo.toml`, dev-deps, profiles, all workspace dependencies declared.
- ✅ **Comments on every file** — every file has a header block explaining its role, what it depends on, what depends on it, and key design decisions. A zero-shot agent landing in any single file can understand how it fits.

### What's in progress

- 🚧 **Engine** (`engine.rs`) — the runtime that holds the cell graph, tracks dependencies, propagates changes, and exposes the universal verbs (`get` / `set` / `call` / `push` / `subscribe`). This is the most important missing piece. The TypeScript version in [`quilt-ts/packages/core/src/engine.ts`](https://github.com/superinstance/quilt/blob/main/packages/core/src/engine.ts) is the spec — port the structure to Rust idioms (e.g. `RwLock<HashMap>` instead of plain `HashMap`, `tokio::sync::Mutex` for async state).
- 🚧 **Parser** (`parser.rs`) — YAML loader using `serde_yml`. Mirror the TypeScript parser in [`quilt-ts/packages/core/src/parser.ts`](https://github.com/superinstance/quilt/blob/main/packages/core/src/parser.ts).
- 🚧 **Cell evaluators**: `sensor`, `io`, `listener`, `router`. These are small placeholders in `cells/mod.rs`. Each is ~50 lines once filled in.

### What's not started

- ❌ **Scheduler** — async evaluation queue with backpressure. (Optional for MVP.)
- ❌ **MCP server** — wrap the engine in an `rmcp` server. (Once engine exists.)
- ❌ **CLI** — `clap`-based `quilt` command. (Once engine exists.)
- ❌ **Examples** — port the TypeScript examples (boat-autopilot, model-router, etc.) to YAML/JSON fixtures.
- ❌ **Integration tests** — `tests/engine_integration.rs` covering the full lifecycle.
- ❌ **Browser simulator** — the TypeScript repo has `landing/simulator.html`. A Rust port could ship as a WASM module embedded in the same HTML.

---

## Layout

```
quilt-rust/
├── Cargo.toml                  # workspace
├── LICENSE
├── README.md                   # this file
└── packages/
    └── core/
        ├── Cargo.toml
        └── src/
            ├── lib.rs           # re-exports + crate-level docs
            ├── types.rs         # the schema
            ├── error.rs         # error types
            ├── context.rs       # caller context propagation
            └── cells/
                ├── mod.rs       # cell evaluator module
                ├── value.rs     # ✅
                ├── formula.rs   # ✅ (rhai-based)
                ├── api.rs       # ✅ (reqwest-based)
                ├── program.rs   # ✅ (rhai-based)
                ├── sensor.rs    # 🚧 (placeholder in mod.rs)
                ├── io.rs        # 🚧 (placeholder in mod.rs)
                ├── listener.rs  # 🚧 (placeholder in mod.rs)
                └── router.rs    # 🚧 (placeholder in mod.rs)
```

---

## How to use what's there

Even without the engine, the foundation is useful — you can call the cell evaluators directly:

```rust
use quilt_core::*;
use serde_json::json;

let cell = Cell::new(CellDef {
    id: "answer".into(),
    kind: CellKind::Value,
    value: Some(json!(42)),
    ..Default::default()
});

let value = cells::evaluate_value(&cell, &CallerContext::default());
assert_eq!(value.data, json!(42));
assert_eq!(value.status, CellStatus::Ready);
```

Formula cells work too:

```rust
let formula = Cell::new(CellDef {
    id: "sum".into(),
    kind: CellKind::Formula,
    expr: Some("=a + b".into()),
    ..Default::default()
});

let engine = FormulaEngine::new();
let value = cells::evaluate_formula(&formula, &CallerContext::default(), &engine);
```

API cells work with a pluggable executor (for tests):

```rust
let api = Cell::new(CellDef {
    id: "weather".into(),
    kind: CellKind::Api,
    endpoint: Some("https://api.example.com/weather".into()),
    ..Default::default()
});

// In a real binary: let v = cells::evaluate_api(&api, &ctx, None, &ApiExecutor::default()).await?;
// In tests: pass a mock that returns canned responses.
```

Program cells work with the runtime handle:

```rust
let program = Cell::new(CellDef {
    id: "compute".into(),
    kind: CellKind::Program,
    code: Some("42".into()),
    ..Default::default()
});

// Pass a `ProgramRuntime` (e.g. `Arc<NullRuntime>` for tests).
```

---

## Roadmap

The next milestones, in order:

1. **Engine** — `QuiltEngine` with `get` / `set` / `call` / `push` / `subscribe`. This unblocks everything else. ~600 lines.
2. **Parser** — YAML loader + serializer. ~200 lines.
3. **Remaining cell evaluators** — `sensor`, `io`, `listener`, `router`. ~50 lines each.
4. **Integration tests** — `tests/engine_integration.rs`. ~300 lines.
5. **MCP server** — `quilt-mcp` package using `rmcp`. ~300 lines.
6. **CLI** — `quilt-cli` package using `clap`. ~400 lines.
7. **Examples** — port the YAML examples from the TypeScript repo.
8. **WASM build** — compile `quilt-core` to WASM for the browser simulator.

Estimated total: ~2,500 lines of additional Rust. Most of it is mechanical (mirror the TypeScript).

---

## Why Rust?

The TypeScript version is the canonical implementation. The Rust port exists for:

- **Edge deployment**. Compiles to a single ~2MB binary, runs on a Raspberry Pi with no Node.js runtime.
- **Latency-critical systems**. No GC pauses, predictable timing for control loops.
- **Embedded targets**. The `core` crate compiles on `no_std` targets with some tweaks — useful for true edge devices.
- **Performance**. For high-frequency sensor ingestion (10kHz+), Rust + Tokio wins.

Pick the implementation that matches your deployment. They share the sheet format.

---

## License

Apache 2.0. Same as the TypeScript version.
