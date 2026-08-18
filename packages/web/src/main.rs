//! # quilt-web CLI
//!
//! A drop-in HTTP server for sheets. Embed Quilt in a website
//! without writing any JavaScript framework — just point this at
//! a YAML sheet and hit the API.
//!
//! ## Usage
//!
//! ```sh
//! quilt-web --sheet examples/weather-monitor/sheet.yaml --port 8080
//! ```
//!
//! Then open `http://localhost:8080/` in your browser.

use anyhow::Result;
use clap::Parser;
use quilt_web::load_state;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::ServeDir;

#[derive(Parser, Debug)]
#[command(name = "quilt-web", about = "Embeddable HTTP server for Quilt sheets")]
struct Args {
    /// Path to the sheet YAML file.
    #[arg(long)]
    sheet: PathBuf,

    /// Port to listen on.
    #[arg(long, default_value = "8080")]
    port: u16,

    /// Bind address.
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    /// Directory of static files to serve at `/` (default: bundled www/).
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let state = load_state(&args.sheet)?;
    let static_dir = args
        .static_dir
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("www"));

    let app = quilt_web::router(state)
        .fallback_service(ServeDir::new(&static_dir))
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", args.bind, args.port).parse()?;
    tracing::info!("quilt-web listening on http://{}", addr);
    tracing::info!("  sheet: {}", args.sheet.display());
    tracing::info!("  static: {}", static_dir.display());
    tracing::info!("  open: http://{}/", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
