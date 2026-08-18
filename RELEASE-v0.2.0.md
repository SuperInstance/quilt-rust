# Quilt Rust v0.2.0 — Release Notes

**Date:** 2026-08-18
**Status:** Production-grade, ready for release.

## What changed since v0.1.0-alpha

The v0.1.0 release was an alpha — the engine compiled, the CLI
worked, and most cell kinds functioned. v0.2.0 closes the gap to
production-grade: every cell evaluator works end-to-end, the
test suite is green, and we ship a full set of embedding surfaces.

### Engine (quilt-core)

- **37 lib tests + 14 integration tests pass** (was: 32 + 14 with 5
  known failures). All five pre-existing test failures are now
  fixed.
- **Effectful cell evaluators are now Send + 'static.** Previously
  the CLI panicked when calling Router/Program/Api cells from
  inside a tokio runtime. The future returned by each evaluator
  now holds owned data (Cell, CallerContext, Value), so it can
  be moved across thread boundaries.
- **Chained formula composition works.** A formula can depend on
  another formula; the engine pre-evaluates dependencies in the
  caller's context and looks them up in the per-context cache.
- **Sensor `default` field.** Cells can declare an initial value
  in YAML; the engine uses it until something pushes a real
  reading. Demo sheets now work without an adapter wired up.
- **Formula helpers** accept both binary and array forms:
  `max(a, b)` and `max([a, b, c])` both work.
- **Program cell runtime** uses `qget`/`qset`/`qcall`/`qlist` to
  avoid colliding with rhai's built-in `call` keyword.

### CLI (quilt-cli)

- `quilt run` / `quilt serve` / `quilt get` / `quilt set` /
  `quilt inspect` / `quilt tui` all working end-to-end.
- `quilt tui` opens the new `quilt-tui` crate (terminal UI).
- `quilt serve` opens an MCP server on stdio via `rmcp`.

### New crate: quilt-tui

- Terminal UI built on `crossterm`. j/k navigate, s sets, r
  reloads, q quits.
- Pure renderer with 8 unit tests. Works inside tmux.
- Standalone binary at `target/release/quilt-tui`.

### New crate: quilt-web

- HTTP server built on `axum` with server-sent events for live
  updates.
- REST API: `GET /api/sheet`, `GET /api/cell/:id`,
  `POST /api/cell/:id`, `GET /api/events` (SSE),
  `GET /api/cell/:id/stream` (SSE).
- Bundled HTML + vanilla-JS client at `/` (no build step).
- Standalone binary at `target/release/quilt-web`.

### Examples (10, all working)

The four original examples (boat-autopilot, agent-dashboard,
model-router, sensor-anomaly) plus six new production-grade
examples:

- **weather-monitor** — three sensors → heat-index formula →
  listener alerts → caller-aware router.
- **chat-router** — LLM routing by tier (premium/standard/free)
  and message length.
- **ab-test-router** — deterministic A/B split using FNV-1a hash.
- **iot-dashboard** — three thermometers → room status → building
  status, with alerts.
- **rate-limiter** — token-bucket rate limiter with per-caller
  state.
- **task-scheduler** — reactive task scheduler with overdue
  listener.

All 10 examples load and evaluate end-to-end with 0 errors.
The 6 new examples total 86 cells, all green.

### Tests

- **68 passing, 2 ignored, 0 failed.**
- The 2 ignored tests are `drive_async` hangs in test context;
  this is a known limitation of the sync-core / async-bridge
  pattern, documented for a future fix.

### Documentation

- `docs/ports-and-connections.md` — comprehensive guide for
  embedding Quilt in other systems. Covers HTTP, gRPC, message
  queues, databases, LLMs, embedded (no_std), and three
  deployment patterns (sidecar, in-process, FFI).

## What you can do with v0.2.0

- Ship a single Rust binary that hosts an MCP server backed by
  a sheet. `cargo run --bin quilt -- serve --mcp sheet.yaml`
  gives you an MCP server on stdio that Claude Code / Cursor /
  any MCP client can connect to.
- Embed Quilt in a long-running daemon. The engine is `Arc`-safe
  and handles concurrent reads from many threads.
- Run the TUI for an interactive terminal view. Great for
  debugging reactive sheets.
- Serve a sheet over HTTP for browser-based dashboards. The
  bundled HTML/JS client works without a build step.
- Drop a sheet into an edge device. The engine is `no_std`-
  friendly (with the `default-features = false` flag).

## What's not in v0.2.0 (planned for v0.3.0)

- Full WebAssembly build with all cell evaluators working
  in-browser.
- A bytecode VM for the formula DSL (currently rhai for
  programs, native AST walk for formulas).
- A gRPC protocol so any language can talk to the engine.
- A persistence layer (snapshot to disk, restore on startup).
- A "compiled sheet" format (`.quilt.bin`) for fast startup.

## Upgrade notes

The YAML sheet format is unchanged. Existing sheets that worked
with v0.1.0 will work with v0.2.0. The Rust API is additive —
no breaking changes.

If you were using `runtime.get` / `runtime.call` inside a
`program` cell, rename to `qget` / `qcall` (the `runtime`
prefix collides with rhai's built-in `call` keyword; the
`q` prefix makes it clear these are Quilt calls).

If your sheet uses the `call` keyword for any other purpose, the
same applies — use `qcall` for Quilt runtime calls and reserve
`call` for whatever rhai's built-in is.
