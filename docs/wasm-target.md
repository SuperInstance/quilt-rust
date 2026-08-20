# The WASM target — `quilt-core-wasm`

> **Status: real.** The sync core (value/formula eval + the cell ledger) compiles
> to `wasm32-unknown-unknown` and conforms to `compat/golden.json` — verified
> natively (cross-tier against the reference implementation) and *inside the
> compiled `.wasm` artifact itself* via a runtime-invoked self-check.

This document is the blocker audit for `packages/core`, the description of what
was built for the web tier, and the build report.

---

## 1. Blocker audit — why `quilt-core` itself cannot compile to wasm32

`cargo build -p quilt-core --target wasm32-unknown-unknown` fails (**exit 101**).
The dependency graph contains three unconditional crates that hard-fail on this
target:

| Dependency | Pulled by | Failure | Verdict |
| --- | --- | --- | --- |
| `mio 1.x` | `tokio` (`features = ["full"]` → `net`/`signal`) | `compile_error!("This wasm target is unsupported by mio. If using Tokio, disable the net feature.")` + 48 resolution errors (`sys::tcp`, `Selector`, `Waker`, ...) | **Blocked** |
| `getrandom 0.3` | `rhai` → `ahash` → `getrandom` | `compile_error!` — wasm32-unknown-unknown needs the `wasm_js` cfg (JS glue) | **Blocked** |
| `uuid` v4 | direct dep | `compile_error!` — v4 needs a randomness source; on this target you must opt into `js` / `rng-*` features | **Blocked** |

What *does* compile cleanly for `wasm32-unknown-unknown` (verified with
individual probe builds, toolchain `rustc 1.97.1`):

| Dependency | Compiles? | Notes |
| --- | --- | --- |
| `serde`, `serde_json` | ✅ | pure Rust; `serde_json` is pinned by the canonical-JSON contract anyway (ryū shortest-round-trip floats) |
| `serde_yml` | ✅ | `libyml` is pure Rust — YAML parsing is portable if the web tier ever wants it |
| `chrono` (`clock`) | ✅ | links fine; note `Utc::now()` has no wall clock on this target — irrelevant to the ledger, which is pure data and takes caller-provided timestamps |
| `crossbeam-channel`, `parking_lot`, `indexmap`, `once_cell`, `regex`, `thiserror`, `anyhow`, `tracing` | ✅ | none are wasm blockers |
| `reqwest 0.12` (`json`, `rustls-tls`) | ✅ (links) | compiles, but a fetch on this target goes through its inert/js path — the HTTP story needs bindgen glue, so `api` cells stay native |

**Conclusion:** the blockers are not the sync core — they are the *effectful*
surface (`tokio` runtime, `rhai` scripting, `uuid` ids) that `quilt-core` pulls
unconditionally. The engine's async cells (`api`, `program`, `router`) and the
subscription plumbing are native-tier by nature. The reactive value/formula
semantics and the entire ledger are pure data + serde + SHA-256 and port
cleanly. Hence: a separate crate rather than a feature-gated `packages/core`
(features could not remove `uuid`/`rhai` from `error.rs`/`cells/` without
touching core semantics, which this task forbids).

---

## 2. What was built — `crates/quilt-core-wasm`

```
crates/quilt-core-wasm/
├── Cargo.toml            # deps: serde + serde_json. Nothing else.
├── src/
│   ├── lib.rs            # docs, module wiring, C-ABI anchors (below)
│   ├── error.rs          # minimal hand-rolled error type (no thiserror)
│   ├── types.rs          # CellId, CellKind{Value,Formula,Sensor}, CellDef, Sheet
│   ├── formula.rs        # dependency-free expression evaluator
│   ├── engine.rs         # WasmEngine — the sync reactive engine
│   └── ledger.rs         # NOT a copy: see below
└── tests/
    └── wasm_conformance.rs
```

### The ledger is single-sourced, not ported

`src/ledger.rs` does not exist as a file. The module is compiled **from the
canonical `packages/core/src/ledger.rs` itself**:

```rust
#[path = "../../../packages/core/src/ledger.rs"]
pub mod ledger;
```

`ledger.rs` was designed to be pure data (no clocks, no I/O, no async — callers
pass timestamps), so it compiles unmodified inside the wasm crate; it only needs
the `crate::error::Error::other` and `crate::types::CellId` shims this crate
provides. The two tiers therefore cannot drift: any ledger change lands in
`packages/core` once and both tiers pick it up. The conformance test makes that
guarantee *executable* (see §3).

### The formula evaluator

`rhai` is a wasm blocker (see §1), so the wasm tier ships its own
recursive-descent evaluator (`src/formula.rs`) covering the portable expression
surface the contract pins: literals, dotted cell references, `+ - * / %`,
comparisons, `&& || !`, parentheses, and the `clamp`/`min`/`max`/`abs` helpers
(the same set the native tier registers into rhai). Dependencies are
auto-detected from the parsed AST — the same edge set the native engine
produces (asserted in the conformance test, op c). Known gaps vs rhai, by
design: string literals, ternary, arrays/maps, script statements.

### The sync engine

`WasmEngine` (`src/engine.rs`) is the wasm-tier echo of `engine.rs`: `value`
(static), `sensor` (latest push, seeded from `default`), `formula`
(re-evaluated on every `get` with inputs pulled recursively; cycles detected,
not spun). No locks, no channels, no subscriptions — single-threaded wasm
doesn't want them, and they are what drags the native engine's dependency set.

### C-ABI anchors (not bindgen glue)

JS glue is out of scope, but a `cdylib` with no exports gets stripped to an
empty husk, so the crate exports two tiny entry points:

- `quilt_core_wasm_abi_version() -> u32` — ABI marker.
- `quilt_core_wasm_golden_check() -> u32` — runs the embedded golden contract
  (`compat/golden.json`, `include_str!`-ed, so never stale) through *this
  build's* engine and ledger inside the wasm module; returns `1` iff every op
  conforms.

---

## 3. Conformance

Three layers of proof, all green:

1. **Native conformance** — `tests/wasm_conformance.rs` drives ops (a)–(e) of
   `compat/golden.json` through the wasm crate compiled for the host:
   ```
   === wasm-core conformance (web tier: quilt-core-wasm) ===
     [a] value cell read .............. PASS (3 vectors)
     [b] formula cell eval ........... PASS (5 vectors)
     [c] propagation order ........... PASS (topo order + wasm-engine graph agrees)
     [d] edge record ................. PASS (3 vectors)
     [e] chain + reconcile ........... PASS (3 seals bit-for-bit, books balanced)
     [x] cross-tier chain identity ... PASS (wasm == native == golden)
   RESULT: PASS — web tier (quilt-core-wasm) conforms to quilt-compat/1
   ```
2. **Cross-tier identity** — the same test runs the golden transcript through
   the native `quilt-core` ledger (a host-only dev-dependency) and asserts the
   two chain hashes are identical *to each other and to golden*. Because the
   ledger is single-sourced, this is guaranteed by construction — the test
   exists so a future refactor that breaks the `#[path]` trick fails loudly.
3. **In-artifact self-check** — the compiled module verifies itself:
   ```
   $ node -e '...instantiate target/wasm32-unknown-unknown/release/quilt_core_wasm.wasm...'
   quilt_core_wasm_abi_version()  -> 1
   quilt_core_wasm_golden_check() -> 1
   ```

Run everything:

```text
cargo test -p quilt-core-wasm                       # all layers 1+2 (plus ledger unit tests)
cargo build -p quilt-core-wasm --target wasm32-unknown-unknown --release
```

---

## 4. Build report

| Command | Exit | Artifact |
| --- | --- | --- |
| `cargo build -p quilt-core-wasm --target wasm32-unknown-unknown` | **0** | `target/wasm32-unknown-unknown/debug/quilt_core_wasm.wasm` (~7.4 MB dev) |
| `cargo build -p quilt-core-wasm --target wasm32-unknown-unknown --release` | **0** | `target/wasm32-unknown-unknown/release/quilt_core_wasm.wasm` (357 KB, golden self-check passes under Node/V8) |
| `cargo build -p quilt-core --target wasm32-unknown-unknown` | 101 | blocked (mio/getrandom/uuid — §1) |

The wasm build's entire dep tree: `serde`, `serde_json` (+ their proc-macros).
That's the whole thing.

**What compiles to wasm:** value/formula/sensor evaluation, the formula
language subset, dependency auto-detection, and the complete ledger — `record`,
`record_with`, `open_input`/`settle_output`, `verify_chain`, `reconcile`,
`replay`, `chain_hash` — bit-for-bit compatible with the native tier.

**What stays native:** `api`/`program`/`router`/`io`/`listener` cells (tokio,
reqwest, rhai), subscriptions/channels, the rhai formula language beyond the
subset above, and YAML sheet loading (portable if wanted — `serde_yml` compiles
— but unnecessary for the JSON-shaped web tier).

## 5. Out of scope / next steps

- `wasm-bindgen` surface (JS classes, `QuiltEngine` bindings) — the Rust API
  and C-ABI anchors are ready to be wrapped.
- Running the conformance suite *as a wasm test* via `wasm-bindgen-test` or a
  wasip1 target — the in-artifact `golden_check` covers the gap meanwhile.
- Smaller artifacts: `wee_alloc` / `opt-level = "z"` / `wasm-opt` passes; the
  357 KB release blob is uncompressed debug-symbol-adjacent weight, not logic.
