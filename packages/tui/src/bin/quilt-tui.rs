//! # quilt-tui binary
//!
//! Standalone terminal UI. Loads a sheet and opens an interactive
//! session. Press q to quit.

use anyhow::{Context, Result};
use quilt_core::parse_sheet;
use quilt_core::QuiltEngine;
use quilt_tui::Tui;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file = args
        .get(1)
        .map(PathBuf::from)
        .context("usage: quilt-tui <sheet.yaml>")?;
    let source =
        std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
    let sheet = parse_sheet(&source).context("parse sheet")?;
    let engine = QuiltEngine::new(file.display().to_string()).into_arc();
    engine.load_sheet(sheet).context("load sheet")?;
    let mut tui = Tui::new(engine);
    tui.run()
}
