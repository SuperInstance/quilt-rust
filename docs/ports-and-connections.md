# Rust: Ports, Connections, and Embedding

The Rust engine is a small, fast, `no_std`-friendly core. It's
designed to be embedded in any Rust application — from a single-
binary CLI to a Rust daemon to a long-running agent. This doc
covers the patterns we use internally and the ones the
ecosystem is starting to develop.

## What "porting" means

Quilt can connect to other systems in three ways:

1. **Inbound** — a `program` cell calls into your system via a
   registered `ProgramRuntime` helper.
2. **Outbound** — an `api` cell makes an HTTP call (via
   `reqwest`) and returns the response.
3. **Side-by-side** — Quilt is one of many services in your
   architecture; it talks to them via your own bus.

## The `ProgramRuntime` trait

The `program` cell evaluates a script (rhai). Inside the
script, the user calls `qget`, `qset`, `qcall`, `qlist` to
interact with the engine. These functions delegate to a
`ProgramRuntime` trait that the harness implements.

```rust
use quilt_core::cells::ProgramRuntime;
use quilt_core::types::{CallerContext, CellValue, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use anyhow::Result;

struct MyRuntime {
    // Whatever state your harness has: DB pool, gRPC client,
    // message queue, etc.
    cache: Mutex<HashMap<String, CellValue>>,
}

impl ProgramRuntime for MyRuntime {
    fn get(&self, id: &str, _ctx: &CallerContext) -> Result<CellValue> {
        Ok(self.cache.lock().unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| CellValue::ready(Value::Null)))
    }

    fn set(&self, id: &str, value: Value, _ctx: &CallerContext) -> Result<()> {
        self.cache.lock().unwrap()
            .insert(id.to_string(), CellValue::ready(value));
        Ok(())
    }

    fn call(&self, id: &str, input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
        // Dispatch to whatever your harness provides.
        match id {
            "db.query" => self.query_db(input),
            "kafka.publish" => self.publish_kafka(input),
            "http.get" => self.http_get(input),
            _ => Ok(CellValue::default()),
        }
    }

    fn list(&self) -> Vec<String> {
        self.cache.lock().unwrap().keys().cloned().collect()
    }
}
```

Wire it into the engine:

```rust
let engine = QuiltEngine::new("my-sheet").into_arc();
engine.load_sheet(sheet)?;
let runtime = Arc::new(MyRuntime::new());
// The engine's evaluate_effectful() wraps this for program cells.
```

In the current API, the runtime is passed implicitly via the
`Arc<QuiltEngine>` itself. To wire your runtime, you can use
the `qget` / `qcall` calls inside program cells, and have the
harness's `program` evaluator use the custom runtime.

A future v0.3 API will make the runtime explicit:

```rust
let engine = QuiltEngine::new("my-sheet")
    .with_runtime(Arc::new(MyRuntime::new()))
    .into_arc();
```

## Connecting to specific systems

### HTTP / REST

Use the `api` cell kind. The default executor uses `reqwest`:

```yaml
- id: github.user
  kind: api
  endpoint: "https://api.github.com/users/octocat"
  method: GET
  headers: { Accept: application/json }
```

For custom needs (auth, retries, rate limiting), implement
your own executor:

```rust
use quilt_core::cells::api::{ApiExecutor, ApiResponse};
use std::sync::Arc;

struct MyExecutor {
    http: reqwest::Client,
    auth: Arc<dyn AuthProvider>,
}

#[async_trait]
impl ApiExecutor for MyExecutor {
    async fn execute(&self, method: &str, url: &str,
                     headers: &BTreeMap<String, String>,
                     body: Option<&str>) -> ApiResponse {
        let token = self.auth.token().await;
        // ... use reqwest with auth
    }
}
```

### gRPC

Wrap a gRPC service as a `program` cell:

```rust
struct GrpcRuntime {
    client: my_service::Client,
}

impl ProgramRuntime for GrpcRuntime {
    fn call(&self, id: &str, input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
        match id {
            "users.get" => {
                let req: GetUserRequest = serde_json::from_value(input.unwrap_or(Value::Null))?;
                let resp = self.client.get_user(&req)?;
                Ok(CellValue::ready(serde_json::to_value(resp)?))
            }
            // ...
        }
    }
}
```

### Message queues (Kafka, NATS, RabbitMQ)

The `program` cell calls a helper that publishes or consumes:

```rust
impl ProgramRuntime for MyRuntime {
    fn call(&self, id: &str, input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
        match id {
            "kafka.publish" => {
                let msg: KafkaMessage = serde_json::from_value(input.unwrap())?;
                self.producer.send(&msg)?;
                Ok(CellValue::ready(serde_json::json!({"published": true})))
            }
            "kafka.consume" => {
                let msg = self.consumer.poll()?;
                Ok(CellValue::ready(serde_json::to_value(msg)?))
            }
            _ => Ok(CellValue::default())
        }
    }
}
```

### Databases

Same pattern. Wrap a connection pool, expose query helpers.

```rust
fn call(&self, id: &str, input: Option<Value>, _ctx: &CallerContext) -> Result<CellValue> {
    if id == "db.query" {
        let sql: String = serde_json::from_value(input.unwrap())?;
        let rows: Vec<Row> = self.db.query(&sql)?;
        Ok(CellValue::ready(serde_json::to_value(rows)?))
    }
    // ...
}
```

### LLM providers

Use the `api` cell kind with a `model:foo` endpoint, plus a
custom executor that makes the real call:

```rust
struct LlmExecutor {
    openai: OpenAIClient,
    anthropic: AnthropicClient,
}

impl ApiExecutor for LlmExecutor {
    async fn execute(&self, method: &str, url: &str,
                     headers: &BTreeMap<String, String>,
                     body: Option<&str>) -> ApiResponse {
        if let Some(model) = url.strip_prefix("model:") {
            if model.starts_with("gpt-") {
                return self.openai.complete(model, body).await;
            }
            if model.starts_with("claude-") {
                return self.anthropic.complete(model, body).await;
            }
        }
        ApiResponse::error(400, "unknown model")
    }
}
```

### Embedded devices (no_std)

The core engine works in `no_std` environments. Drop the
`tokio` and `reqwest` dependencies, and use the `program` cell
to call your custom runtime:

```toml
[dependencies]
quilt-core = { path = "../core", default-features = false }
```

```rust
#![no_std]
extern crate alloc;

use quilt_core::{QuiltEngine, parse_sheet};

let yaml = include_str!("sheet.yaml"); // embed at compile time
let sheet = parse_sheet(yaml).unwrap();
let engine = QuiltEngine::new("device").into_arc();
engine.load_sheet(sheet).unwrap();
```

For embedded targets, drop the rhai program cells (they pull
in a lot). Use only `value` and `formula` cells, with the
harness's custom runtime providing all logic.

### WebAssembly

WASM is a special case. The `quilt-core` crate compiles to
WASM (with `wasm-bindgen`) for browser embedding. The async
cell evaluators (api, program, router) don't work in WASM by
default — provide synchronous alternatives.

For browser usage, use the `quilt-web` crate (HTTP server with
SSE) and have the browser connect to it. This gives you the
full engine in the browser, but the heavy lifting happens
server-side.

For full client-side evaluation in the browser, see the
`quilt-wasm` crate (planned for v0.3.0).

### Embedded scripting (rhai vs Lua vs Wasmtime)

The current `program` cell uses rhai. If you need a different
scripting language:

- **Lua**: rewrite the `program` evaluator to use `mlua`. The
  script syntax is similar enough that most cells port
  unchanged.
- **Wasmtime**: compile user scripts to WASM, load them as
  modules, call them. Slower startup, faster execution, more
  secure sandbox.
- **JavaScript**: use `boa_engine` or `deno_core`. The
  downside: the engine dependency is heavy.

For the v0.2.0 release, we ship rhai. Other engines are
plug-in points for v0.3.0.

## Making Quilt "production-grade" in your stack

A few patterns that we've seen work in real deployments:

### 1. The `quilt-serve` sidecar

Run Quilt as a separate process. Your main app talks to it
over gRPC or HTTP. This isolates the engine's resource usage
(memory, CPU) from the rest of the system.

```
┌──────────────┐      gRPC       ┌──────────────┐
│  main app    │ ─────────────── │ quilt-serve  │
│  (Rust/TS)   │                 │  (Rust)      │
└──────────────┘                 └──────────────┘
                                       │
                                       ▼
                                ┌──────────────┐
                                │ sheets,      │
                                │ cells,       │
                                │ runtime      │
                                └──────────────┘
```

Pros: clean isolation, easy to scale, well-defined protocol.
Cons: latency, deployment complexity.

### 2. In-process library

Embed `quilt-core` directly in your Rust binary. Use the
`QuiltEngine` as a stateful object that lives alongside your
other state.

Pros: zero latency, simple deployment.
Cons: shares memory + CPU with your app.

### 3. Embedded scripting

Compile Quilt as a `cdylib` and load it from a host language
(Python, Ruby, etc.) via FFI. This is the "embed Quilt
everywhere" path.

Pros: Quilt becomes available to any language.
Cons: FFI is fiddly, error handling is harder, no async.

We recommend pattern 1 (sidecar) for production deployments,
pattern 2 (in-process) for tight integration, and pattern 3
only for niche cases.

## Operational concerns

- **Memory**: the engine is small (a few MB for hundreds of
  cells). The rhai scripts are JIT-cached, so they don't add
  much.
- **CPU**: formula evaluation is microseconds. The hot path
  is api/program/router cells, which do I/O.
- **Latency**: a sync `get()` call blocks the calling thread.
  For high-throughput, use the `Arc<QuiltEngine>` from many
  threads (it has internal locking).
- **Reactivity**: setting a value triggers synchronous
  recomputation of dependents. For a 1000-cell sheet, this
  is sub-millisecond.
- **Persistence**: there's no built-in persistence. The
  harness should snapshot the engine state periodically
  (e.g. via `engine.list_cells()` + `engine.get()` for each
  cell) and restore on startup.

## Roadmap (Rust side)

What we'd love to add:

- A native WebAssembly build with full async support
  (v0.3.0)
- A bytecode VM for the formula DSL, like the TS one
  (v0.3.0)
- A persistence layer (snapshot to disk, restore on startup)
  (v0.3.0)
- A "compiled sheet" format (`.quilt.bin`) for fast startup
  (v0.4.0)
- A gRPC protocol definition, so any language can talk to
  the engine (v0.3.0)

For the v0.2.0 release, the engine is feature-complete for
single-process use. The roadmap above is for the next phase.
