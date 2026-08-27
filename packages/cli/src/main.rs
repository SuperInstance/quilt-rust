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
//! - `journal <sheet> --out <journal.bin>` — live black-box recorder:
//!   every `set`/`push` mutation sealed into a crash-safe journal
//! - `replay <journal.bin>` — rebuild and verify a journal (the
//!   black box played back; `--recover` truncates a torn tail)
//! - `journal-verify <journal.bin>` — structural verify (CRC + chain)

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

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
    /// Live black-box recorder: load the sheet, subscribe to every
    /// change, and seal each mutation into a crash-safe journal.
    Journal {
        /// The sheet file (YAML).
        sheet: PathBuf,
        /// The journal file to write (created; must not exist).
        #[arg(long)]
        out: PathBuf,
        /// Skip fsync-before-ack (tests / scratch only).
        #[arg(long)]
        no_fsync: bool,
    },
    /// Rebuild and verify a journal: every CRC, every chain link,
    /// every ledger seal. Divergences are printed, never swallowed.
    Replay {
        /// The journal file.
        journal: PathBuf,
        /// Truncate a torn tail in place (that write never happened).
        #[arg(long)]
        recover: bool,
        /// Write the embedded sheet source to this file.
        #[arg(long)]
        emit_sheet: Option<PathBuf>,
    },
    /// Structural verify only: frame CRCs and chain linkage, no
    /// ledger rebuild.
    JournalVerify {
        /// The journal file.
        journal: PathBuf,
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
        Commands::Journal {
            sheet,
            out,
            no_fsync,
        } => journal(&sheet, &out, !no_fsync).await,
        Commands::Replay {
            journal,
            recover,
            emit_sheet,
        } => replay(&journal, recover, emit_sheet.as_deref()).await,
        Commands::JournalVerify { journal } => journal_verify_cmd(&journal).await,
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

async fn run_sheet(file: &std::path::Path) -> Result<()> {
    let engine = load_engine(file)?;
    println!(
        "loaded {} ({} cells)",
        file.display(),
        engine.list_cells().len()
    );
    for cell in engine.list_cells() {
        let v = engine.get(&cell.def.id, CallerContext::default())?;
        println!("  {} ({:?}) = {}", cell.def.id, cell.def.kind, v.data);
    }
    Ok(())
}

async fn serve_mcp(file: &std::path::Path) -> Result<()> {
    // Load the sheet into the engine the MCP server will actually serve.
    // (This used to load a local engine and then call `serve_stdio()`,
    // which constructs a FRESH empty server — every client saw 0 cells
    // regardless of the sheet. Wire the loaded engine through instead.)
    let engine = QuiltEngine::new("mcp").into_arc();
    engine.load_sheet(parse_sheet_file(file)?)?;
    let server = quilt_mcp::QuiltMcpServer::from_engine(engine);
    eprintln!(
        "quilt-mcp: serving {} ({} cells) on stdio",
        file.display(),
        server.engine().list_cells().len()
    );

    // Run the MCP server. This blocks.
    quilt_mcp::serve_stdio_with(server).await
}

async fn get_cell(id: &str, file: &std::path::Path) -> Result<()> {
    let engine = load_engine(file)?;
    let v = engine.get(id, CallerContext::default())?;
    println!("{}", serde_json::to_string_pretty(&v.data)?);
    Ok(())
}

async fn set_cell(id: &str, value: &str, file: &std::path::Path) -> Result<()> {
    let engine = load_engine(file)?;
    let v: serde_json::Value =
        serde_json::from_str(value).or_else(|_| serde_json::from_str(&format!("\"{}\"", value)))?;
    engine.set(id, v.clone(), CallerContext::default())?;
    println!("set {} = {}", id, v);
    Ok(())
}

async fn inspect(file: &std::path::Path) -> Result<()> {
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

async fn tui(file: &std::path::Path) -> Result<()> {
    use quilt_tui::Tui;
    let sheet = parse_sheet_file(file)?;
    let engine = QuiltEngine::new(file.display().to_string()).into_arc();
    engine.load_sheet(sheet)?;
    let mut tui = Tui::new(engine);
    tui.run()
}

// =============================================================================
// Journal: the black-box recorder
// =============================================================================

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Live journaling: load the sheet, subscribe to everything, then
/// apply mutations from stdin — one line per mutation:
///
///   set <cell-id> <json>
///   push <cell-id> <json>
///
/// Every resulting engine event is sealed into the journal. EOF
/// (Ctrl-D) closes it cleanly.
async fn journal(sheet: &std::path::Path, out: &std::path::Path, fsync: bool) -> Result<()> {
    use std::io::BufRead;

    let source = std::fs::read_to_string(sheet)
        .map_err(|e| anyhow::anyhow!("read {}: {}", sheet.display(), e))?;
    let sheet_def = parse_sheet(&source)?;
    let sheet_id = sheet_def.id.clone();
    let sheet_version = sheet_def.version.clone().unwrap_or_else(|| "1".to_string());

    let policy = if fsync {
        quilt_core::SyncPolicy::EveryFrame
    } else {
        quilt_core::SyncPolicy::Off
    };
    let writer = quilt_core::JournalWriter::create(out, policy)?;
    let mut recorder =
        quilt_core::JournalRecorder::start(writer, &sheet_id, &sheet_version, &source)?;

    let engine = QuiltEngine::new(&sheet_id).into_arc();
    engine.load_sheet(sheet_def)?;
    let sub = engine.subscribe_all();

    println!(
        "journaling {} -> {} ({} cells); enter mutations on stdin, Ctrl-D to finish",
        sheet.display(),
        out.display(),
        engine.list_cells().len()
    );

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        let verb = parts.next().unwrap_or("");
        let id = parts.next().unwrap_or("");
        let value_str = parts.next().unwrap_or("");
        let value: serde_json::Value = serde_json::from_str(value_str)
            .or_else(|_| serde_json::from_str(&format!("\"{}\"", value_str)))?;
        match verb {
            "set" => engine.set(id, value, CallerContext::default())?,
            "push" => engine.push(id, value)?,
            other => anyhow::bail!("unknown verb '{}' (use set/push)", other),
        }
        // Seal every event the mutation produced.
        while let Ok(event) = sub.rx.try_recv() {
            let ack = recorder.record_event(&event.cell_id, &event.new_value.data, now_millis())?;
            println!(
                "  frame #{} {} = {} ({} bytes, head {}…)",
                ack.seq,
                event.cell_id,
                event.new_value.data,
                ack.bytes,
                hex_prefix(&ack.head)
            );
        }
    }

    let stats = recorder.writer_mut().stats();
    println!(
        "journal closed: {} frames, {} bytes, {} fsyncs, {} cells recorded",
        stats.frames,
        stats.bytes,
        stats.syncs,
        recorder.ledgers().len()
    );
    Ok(())
}

fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Replay a journal: verify everything, rebuild the ledgers, print
/// the honest report.
async fn replay(
    journal_path: &std::path::Path,
    recover: bool,
    emit_sheet: Option<&std::path::Path>,
) -> Result<()> {
    use quilt_core::{journal_replay, recover_file, VerifyOutcome};

    let mut bytes = std::fs::read(journal_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", journal_path.display(), e))?;

    if recover {
        let report = recover_file(journal_path)?;
        if report.dropped_bytes > 0 {
            println!(
                "recovered: truncated {} torn tail bytes (that write never happened); kept {}",
                report.dropped_bytes, report.kept_bytes
            );
        } else {
            println!("recovered: nothing to do (no torn tail)");
        }
        bytes = std::fs::read(journal_path)?;
    }

    let report = journal_replay(&bytes);
    println!(
        "journal: {} ({} bytes)",
        journal_path.display(),
        bytes.len()
    );
    println!("format:  v{}", report.verify.format_version);
    if let Some(meta) = &report.sheet {
        println!(
            "sheet:   {} (version {}, {}-byte source)",
            meta.id,
            meta.version,
            meta.source.len()
        );
        if let Some(path) = emit_sheet {
            std::fs::write(path, &meta.source)?;
            println!("sheet source written to {}", path.display());
        }
    } else {
        println!("sheet:   (no metadata frame)");
    }
    for (seq, cp) in &report.checkpoints {
        println!("checkpoint #{}: {}", seq, cp.note);
    }
    println!(
        "frames replayed: {} ledger entries",
        report.replayed_entries
    );

    for (id, ledger) in &report.ledgers {
        let rec = ledger.reconcile();
        println!(
            "  {} entries={} head={}… state={} surprise={:.3} balanced={}",
            id,
            ledger.len(),
            &ledger.chain_hash()[..16],
            ledger.state(),
            rec.total_surprise,
            rec.balanced
        );
    }

    // The outcome — printed verbatim, exit code follows it.
    let hard_failure = match &report.verify.outcome {
        VerifyOutcome::Clean { frames } => {
            println!("outcome: CLEAN ({} frames verified)", frames);
            false
        }
        VerifyOutcome::TornTail {
            good_frames,
            torn_bytes,
            torn_offset,
        } => {
            println!(
                "outcome: TORN TAIL — frame {} at byte {} is a partial write ({} bytes); its write never happened; {} good frames before it",
                good_frames + 1, torn_offset, torn_bytes, good_frames
            );
            false
        }
        VerifyOutcome::TornHeader { available } => {
            println!(
                "outcome: TORN HEADER — only {}/{} header bytes exist",
                available,
                quilt_core::HEADER_LEN
            );
            false
        }
        VerifyOutcome::NotAJournal(why) => {
            println!("outcome: NOT A JOURNAL — {}", why);
            true
        }
        VerifyOutcome::Corrupt { index, reason, .. } => {
            println!("outcome: CORRUPT — frame {} failed: {}", index + 1, reason);
            true
        }
        VerifyOutcome::Divergence { divergence, .. } => {
            println!(
                "outcome: DIVERGENCE — frame {}: {} ({:?})",
                divergence.seq, divergence.message, divergence.kind
            );
            true
        }
    };
    for d in &report.divergences {
        println!("divergence: frame {}: {} ({:?})", d.seq, d.message, d.kind);
    }

    if hard_failure || !report.divergences.is_empty() {
        return Err(anyhow::anyhow!("journal did not replay clean"));
    }
    Ok(())
}

/// Structural verify only.
async fn journal_verify_cmd(journal_path: &std::path::Path) -> Result<()> {
    use quilt_core::{journal_verify, VerifyOutcome};

    let bytes = std::fs::read(journal_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", journal_path.display(), e))?;
    let report = journal_verify(&bytes);
    println!(
        "journal: {} ({} bytes)",
        journal_path.display(),
        bytes.len()
    );
    println!("format:  v{}", report.format_version);
    for f in &report.frames {
        let type_name = match f.entry_type {
            quilt_core::ENTRY_SHEET_META => "sheet-meta",
            quilt_core::ENTRY_LEDGER_ENTRY => "ledger-entry",
            quilt_core::ENTRY_CHECKPOINT => "checkpoint",
            other => {
                println!(
                    "  #{} type={} seq={} head={}… UNKNOWN",
                    f.seq,
                    other,
                    f.seq,
                    hex_prefix(&f.head)
                );
                continue;
            }
        };
        println!(
            "  #{} {} payload={}B head={}… offset={}",
            f.seq,
            type_name,
            f.payload.len(),
            hex_prefix(&f.head),
            f.offset
        );
    }
    let hard_failure = match &report.outcome {
        VerifyOutcome::Clean { frames } => {
            println!("outcome: CLEAN ({} frames)", frames);
            false
        }
        VerifyOutcome::TornTail {
            good_frames,
            torn_bytes,
            torn_offset,
        } => {
            println!(
                "outcome: TORN TAIL — partial frame at byte {} ({} bytes); {} good frames",
                torn_offset, torn_bytes, good_frames
            );
            false
        }
        VerifyOutcome::TornHeader { available } => {
            println!(
                "outcome: TORN HEADER — {}/{} bytes",
                available,
                quilt_core::HEADER_LEN
            );
            false
        }
        VerifyOutcome::NotAJournal(why) => {
            println!("outcome: NOT A JOURNAL — {}", why);
            true
        }
        VerifyOutcome::Corrupt { index, reason, .. } => {
            println!("outcome: CORRUPT — frame {}: {}", index + 1, reason);
            true
        }
        VerifyOutcome::Divergence { divergence, .. } => {
            println!(
                "outcome: DIVERGENCE — frame {}: {} ({:?})",
                divergence.seq, divergence.message, divergence.kind
            );
            true
        }
    };
    if hard_failure {
        return Err(anyhow::anyhow!("journal failed verification"));
    }
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

fn load_engine(file: &std::path::Path) -> Result<Arc<QuiltEngine>> {
    let sheet = parse_sheet_file(file)?;
    let engine = QuiltEngine::new(file.display().to_string()).into_arc();
    engine.load_sheet(sheet)?;
    Ok(engine)
}
