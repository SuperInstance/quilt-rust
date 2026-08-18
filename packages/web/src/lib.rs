//! # quilt-web
//!
//! Quilt for the browser. A drop-in HTTP server that:
//!
//! - Serves static files (HTML/JS/CSS) for a sheet's UI
//! - Exposes a REST API: `GET /api/cells`, `GET /api/cell/:id`,
//!   `POST /api/cell/:id`, `GET /api/sheet`
//! - Streams live updates over SSE: `GET /api/events`
//!
//! This is the easy-embed path: drop a `quilt-web` binary in
//! your web app, point it at a sheet, and your front-end just
//! hits the API. No WASM, no bundler, no npm. The JS shim in
//! `www/quilt.js` is 80 lines.
//!
//! ## Run
//!
//! ```sh
//! cargo run -p quilt-web -- --sheet examples/weather-monitor/sheet.yaml --port 8080
//! ```
//!
//! Then open `http://localhost:8080/`.

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use quilt_core::types::CallerContext;
use quilt_core::{parse_sheet, QuiltEngine, SubscriptionEvent};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

// =============================================================================
// Shared state
// =============================================================================

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QuiltEngine>,
    /// Broadcast channel for cell-change events. Subscribers get
    /// a `SseEvent` for every change the engine reports.
    pub events: broadcast::Sender<SubscriptionEvent>,
}

impl AppState {
    pub fn new(engine: Arc<QuiltEngine>) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        // Bridge the engine's "subscribe all" channel to our
        // broadcast bus. The engine uses a sync crossbeam
        // channel, so we poll it from a dedicated OS thread
        // and forward to the async broadcast.
        let all_sub = engine.subscribe_all();
        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            loop {
                match all_sub.rx.recv() {
                    Ok(ev) => {
                        if tx_clone.send(ev).is_err() {
                            // No subscribers; keep polling.
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self { engine, events: tx }
    }
}

// =============================================================================
// JSON shapes — what the front-end sees
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsCellValue {
    pub data: serde_json::Value,
    pub status: String,
    pub computed_at: Option<i64>,
    pub error: Option<String>,
}

impl From<quilt_core::types::CellValue> for JsCellValue {
    fn from(v: quilt_core::types::CellValue) -> Self {
        Self {
            data: v.data,
            status: v.status.as_str().to_string(),
            computed_at: v.computed_at.map(|n| n as i64),
            error: v.error.map(|e| e.message),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JsCellInfo {
    pub id: String,
    pub kind: String,
    pub value: Option<serde_json::Value>,
    pub expr: Option<String>,
    pub endpoint: Option<String>,
    pub code: Option<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsSheet {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub cells: Vec<JsCellInfo>,
}

// =============================================================================
// Routes
// =============================================================================

/// GET /api/sheet — full sheet metadata + all cells.
async fn get_sheet(State(state): State<AppState>) -> Json<JsSheet> {
    let engine_id = state.engine.id().to_string();
    let cells: Vec<JsCellInfo> = state
        .engine
        .list_cells()
        .iter()
        .map(|c| JsCellInfo {
            id: c.def.id.clone(),
            kind: c.def.kind.as_str().to_string(),
            value: c.def.value.clone(),
            expr: c.def.expr.clone(),
            endpoint: c.def.endpoint.clone(),
            code: c.def.code.clone(),
            dependencies: c.dependencies.iter().cloned().collect(),
            dependents: c.dependents.iter().cloned().collect(),
        })
        .collect();
    Json(JsSheet {
        id: engine_id,
        title: None,
        description: None,
        version: None,
        cells,
    })
}

/// GET /api/cell/:id — current value of a cell.
async fn get_cell(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JsCellValue>, (StatusCode, String)> {
    let v = state
        .engine
        .get(&id, CallerContext::default())
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    Ok(Json(v.into()))
}

/// POST /api/cell/:id — set a value cell. Body is any JSON value.
async fn set_cell(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .engine
        .set(&id, value, CallerContext::default())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/events — SSE stream of all cell changes.
async fn events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<SseEvent, Infallible>>> {
    let mut rx = state.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    yield Ok(SseEvent::default().data(data));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    yield Ok(SseEvent::default().data(format!("{{\"lagged\":{n}}}")));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// GET /api/cell/:id/stream — SSE stream for a specific cell.
async fn cell_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl tokio_stream::Stream<Item = std::result::Result<SseEvent, Infallible>>>, (StatusCode, String)> {
    let engine = Arc::clone(&state.engine);
    // Subscribe to the cell. The handle's rx is a sync
    // crossbeam channel, so we bridge it to an async stream
    // via a channel.
    let handle = engine
        .subscribe(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    let cell_id = id.clone();
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<SubscriptionEvent>();
    std::thread::spawn(move || {
        loop {
            match handle.rx.recv() {
                Ok(ev) => {
                    if async_tx.send(ev).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let stream = async_stream::stream! {
        while let Some(ev) = async_rx.recv().await {
            if ev.cell_id == cell_id {
                let data = serde_json::to_string(&ev).unwrap_or_default();
                yield Ok(SseEvent::default().data(data));
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

// =============================================================================
// Public entry point
// =============================================================================

/// Build the axum router. Exposed so tests and custom binaries
/// can mount it inside a larger app.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/sheet", get(get_sheet))
        .route("/api/cell/:id", get(get_cell).post(set_cell))
        .route("/api/cell/:id/stream", get(cell_events))
        .route("/api/events", get(events))
        .with_state(state)
}

/// Run the server. Blocks until shutdown.
pub async fn serve(state: AppState, addr: SocketAddr) -> Result<()> {
    let app = router(state)
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(tower_http::trace::TraceLayer::new_for_http());
    tracing::info!("quilt-web listening on {}", addr);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {}", addr))?;
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

/// Load a sheet from a YAML file and return the state.
pub fn load_state(sheet_path: &PathBuf) -> Result<AppState> {
    let source = std::fs::read_to_string(sheet_path)
        .with_context(|| format!("read {}", sheet_path.display()))?;
    let sheet = parse_sheet(&source).context("parse sheet")?;
    let engine = QuiltEngine::new(sheet.id.clone())
        .into_arc()
        .tap_id_from_sheet(&sheet);
    engine.load_sheet(sheet).context("load sheet")?;
    Ok(AppState::new(engine))
}

// We need a helper to "tap" the sheet id into the engine. The
// engine id is set on construction; if the sheet has its own id
// we can use it instead.
trait TapSheet {
    fn tap_id_from_sheet(self, sheet: &quilt_core::SheetDef) -> Arc<QuiltEngine>;
}
impl TapSheet for Arc<QuiltEngine> {
    fn tap_id_from_sheet(self, _sheet: &quilt_core::SheetDef) -> Arc<QuiltEngine> {
        // For the MVP, we use the file path as the engine id.
        // Sheet id is informational.
        self
    }
}
