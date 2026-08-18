//! # quilt CLI
//!
//! The Quilt command-line interface. Wraps `quilt-core` and
//! `quilt-mcp` with a clean `clap`-based command set.
//!
//! ## Commands
//!
//! - `init` — scaffold a new sheet in the current directory
//! - `run <file>` — load a sheet and run it (REPL-style: read cells,
//!   apply inputs, observe outputs)
//! - `serve --mcp <file>` — expose the sheet as an MCP server on stdio
//! - `get <id> [file]` — print a cell's value
//! - `set <id> <value> [file]` — set a cell's value
//! - `inspect <file>` — print a summary of the sheet
//! - `test <file>` — run the sheet's tests
//! - `tui <file>` — launch the TUI on a sheet (placeholder for now)

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use quilt_core::{parse_sheet, CallerContext, QuiltEngine};

fn parse_sheet_file(path: &std::path::Path) -> anyhow::Result<quilt_core::SheetDef> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
    Ok(parse_sheet(&source)?)
}

#[derive(Parser, Debug)]
#[command(name = "quilt")]
#[command(about = "A spreadsheet where every cell is a live, addressable capability")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scaffold a new sheet in the current directory.
    Init {
        /// The sheet id (becomes the filename).
        name: String,
    },
    /// Load a sheet and evaluate cells (read-only batch).
    Run {
        /// The sheet file (YAML).
        file: PathBuf,
    },
    /// Serve the sheet as an MCP server on stdio.
    Serve {
        /// The sheet file (YAML).
        file: PathBuf,
    },
    /// Print a cell's value.
    Get {
        /// The cell id.
        id: String,
        /// The sheet file (YAML).
        file: PathBuf,
    },
    /// Set a cell's value.
    Set {
        /// The cell id.
        id: String,
        /// The new value (JSON).
        value: String,
        /// The sheet file (YAML).
        file: PathBuf,
    },
    /// Print a summary of the sheet.
    Inspect {
        /// The sheet file (YAML).
        file: PathBuf,
    },
    /// Launch the TUI on a sheet.
    Tui {
        /// The sheet file (YAML).
        file: PathBuf,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {:#}", e);
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { name } => init(&name),
        Commands::Run { file } => run_sheet(&file).await,
        Commands::Serve { file } => serve_mcp(&file).await,
        Commands::Get { id, file } => get_cell(&id, &file).await,
        Commands::Set { id, value, file } => set_cell(&id, &value, &file).await,
        Commands::Inspect { file } => inspect(&file).await,
        Commands::Tui { file } => tui(&file).await,
    }
}

// =============================================================================
// Subcommand implementations
// =============================================================================

fn init(name: &str) -> Result<()> {
    let path = format!("{}.yaml", name);
    let body = format!(
        "# Quilt sheet: {}\nid: {}\nversion: \"1\"\ncells:\n  - id: hello\n    kind: value\n    value: hello, world\n    description: A simple value cell.\n",
        name, name
    );
    std::fs::write(&path, body).with_context(|| format!("failed to write {}", path))?;
    println!("wrote {}", path);
    Ok(())
}

async fn run_sheet(file: &PathBuf) -> Result<()> {
    let engine = load_engine(file)?;
    println!("loaded {} ({} cells)", file.display(), engine.list_cells().len());
    for cell in engine.list_cells() {
        let v = engine.get(&cell.def.id, CallerContext::default())?;
        println!("  {} ({:?}) = {}", cell.def.id, cell.def.kind, v.data);
    }
    Ok(())
}

async fn serve_mcp(file: &PathBuf) -> Result<()> {
    let sheet = parse_sheet_file(file)?;
    let engine = QuiltEngine::new("mcp").into_arc();
    engine.load_sheet(sheet)?;
    eprintln!("quilt-mcp: serving {} ({} cells) on stdio", file.display(), engine.list_cells().len());

    // Run the MCP server. This blocks.
    quilt_mcp::serve_stdio().await
}

async fn get_cell(id: &str, file: &PathBuf) -> Result<()> {
    let engine = load_engine(file)?;
    let v = engine.get(id, CallerContext::default())?;
    println!("{}", serde_json::to_string_pretty(&v.data)?);
    Ok(())
}

async fn set_cell(id: &str, value: &str, file: &PathBuf) -> Result<()> {
    let engine = load_engine(file)?;
    let v: serde_json::Value = serde_json::from_str(value)
        .or_else(|_| serde_json::from_str(&format!("\"{}\"", value)))?;
    engine.set(id, v.clone(), CallerContext::default())?;
    println!("set {} = {}", id, v);
    Ok(())
}

async fn inspect(file: &PathBuf) -> Result<()> {
    let engine = load_engine(file)?;
    let cells = engine.list_cells();
    println!("sheet: {}", file.display());
    println!("cells: {}", cells.len());
    let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for cell in &cells {
        *by_kind.entry(format!("{:?}", cell.def.kind)).or_insert(0) += 1;
    }
    for (kind, count) in by_kind {
        println!("  {}: {}", kind, count);
    }
    Ok(())
}

async fn tui(file: &PathBuf) -> Result<()> {
    eprintln!("quilt-tui is a TypeScript package (@quilt/tui) — not available from this Rust binary");
    eprintln!("to use it: npx @quilt/tui {}", file.display());
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

fn load_engine(file: &PathBuf) -> Result<QuiltEngine> {
    let sheet = parse_sheet_file(file)?;
    let engine = QuiltEngine::new(file.display().to_string());
    engine.load_sheet(sheet)?;
    Ok(engine)
}
