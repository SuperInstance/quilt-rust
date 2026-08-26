//! # quilt-tui
//!
//! A terminal UI for Quilt sheets. Lists cells, lets you navigate,
//! inspect deps, set values, and watch reactivity happen in real
//! time. Designed to be tmux-friendly (we don't use the alt-screen
//! by default, so scrollback still works).
//!
//! ## What this crate provides
//!
//! - `Tui::new(engine, sheet_id)` — construct from an engine
//! - `tui.run()` — the main loop
//! - `tui.render(&state)` — pure renderer (testable)
//!
//! ## Layout
//!
//! ```text
//! ┌─ Quilt TUI — <sheet-id> (N cells) ──────────────┐
//! │ ID                KIND     VALUE                │
//! │ a                 value    10                   │
//! │ > sum             formula  30                   │  ← selected
//! │ ...
//! ├─ Dependencies of: sum ─────────────────────────┤
//! │ a, b                                            │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! ## Why crossterm
//!
//! - Cross-platform (Unix + Windows)
//! - Raw mode + key events out of the box
//! - Doesn't pull in a full TUI framework

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use quilt_core::types::{CellId, CellKind, CellStatus};
use quilt_core::CallerContext;
use quilt_core::QuiltEngine;
use serde::Serialize;
use std::io::{stdout, Write};
use std::sync::Arc;

// =============================================================================
// State types — kept separate from the engine so the renderer is pure
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TuiStatus {
    Idle,
    Ready,
    Stale,
    Error,
}

impl TuiStatus {
    pub fn label(self) -> &'static str {
        match self {
            TuiStatus::Idle => "idle",
            TuiStatus::Ready => "ready",
            TuiStatus::Stale => "stale",
            TuiStatus::Error => "error",
        }
    }
}

impl From<CellStatus> for TuiStatus {
    fn from(s: CellStatus) -> Self {
        match s {
            CellStatus::Idle => TuiStatus::Idle,
            CellStatus::Ready => TuiStatus::Ready,
            CellStatus::Stale => TuiStatus::Stale,
            CellStatus::Error => TuiStatus::Error,
            CellStatus::Computing => TuiStatus::Idle,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CellRow {
    pub id: String,
    pub kind: CellKind,
    pub value: String,
    pub status: TuiStatus,
    pub error: Option<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TuiMode {
    Normal,
    Set,
}

#[derive(Debug, Clone, Serialize)]
pub struct TuiState {
    pub sheet_id: String,
    pub cells: Vec<CellRow>,
    pub selected: usize,
    pub view_top: usize,
    pub viewport_height: usize,
    pub mode: TuiMode,
    pub edit_buffer: String,
    pub status: Option<String>,
}

impl TuiState {
    pub fn selected_id(&self) -> Option<&str> {
        self.cells.get(self.selected).map(|c| c.id.as_str())
    }
}

// =============================================================================
// ANSI codes (kept inline because we use only a few)
// =============================================================================

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const REVERSED: &str = "\x1b[7m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
const CLEAR: &str = "\x1b[2J\x1b[H";

fn kind_color(k: CellKind) -> &'static str {
    match k {
        CellKind::Value => BLUE,
        CellKind::Formula => GREEN,
        CellKind::Api => YELLOW,
        CellKind::Program => MAGENTA,
        CellKind::Sensor => CYAN,
        CellKind::Io => YELLOW,
        CellKind::Listener => RED,
        CellKind::Router => MAGENTA,
    }
}

fn status_color(s: TuiStatus) -> &'static str {
    match s {
        TuiStatus::Ready => GREEN,
        TuiStatus::Stale => YELLOW,
        TuiStatus::Error => RED,
        TuiStatus::Idle => DIM,
    }
}

// =============================================================================
// Renderer — pure, testable
// =============================================================================

pub fn render(state: &TuiState) -> String {
    let mut out = String::new();
    out.push_str(CLEAR);

    out.push_str(&format!(
        "{BOLD}┌─ Quilt TUI — {sheet} ({n} cells) ─{RESET}\n",
        sheet = state.sheet_id,
        n = state.cells.len()
    ));

    let end = (state.view_top + state.viewport_height).min(state.cells.len());
    for i in state.view_top..end {
        let row = &state.cells[i];
        let marker = if i == state.selected { ">" } else { " " };
        let prefix = if i == state.selected {
            REVERSED
        } else {
            ""
        };
        // NOTE: highlight and normal rows previously used the same RESET
        // suffix (clippy if_same_then_else); the selection styling now comes
        // from the prefix above, so a single RESET closes both cases.
        let suffix = RESET;
        out.push_str(&format!(
            "{prefix}{marker} {id:<20} {kind_color}{kind:<10}{RESET} {status_color}{status:<8}{RESET} {value}{suffix}\n",
            marker = marker,
            id = truncate(&row.id, 20),
            kind = format!("{:?}", row.kind).to_lowercase(),
            status = row.status.label(),
            value = truncate(&row.value, 60),
            kind_color = kind_color(row.kind),
            status_color = status_color(row.status),
            prefix = prefix,
            suffix = suffix,
            RESET = RESET,
        ));
    }

    if let Some(row) = state.cells.get(state.selected) {
        out.push_str(&format!("{BOLD}├─ {id} ─{RESET}\n", id = row.id));
        out.push_str(&format!(
            "  deps:     {}\n",
            if row.dependencies.is_empty() {
                "(none)".to_string()
            } else {
                row.dependencies.join(", ")
            }
        ));
        out.push_str(&format!(
            "  used by:  {}\n",
            if row.dependents.is_empty() {
                "(none)".to_string()
            } else {
                row.dependents.join(", ")
            }
        ));
        if let Some(err) = &row.error {
            out.push_str(&format!("  {RED}error: {err}{RESET}\n"));
        }
    }

    match state.mode {
        TuiMode::Normal => {
            out.push_str(&format!(
                "{BOLD}├─ Keys: j/k=navigate  s=set  r=reload  q=quit ─{RESET}\n"
            ));
        }
        TuiMode::Set => {
            out.push_str(&format!(
                "{BOLD}├─ set {id} = {buffer}█ (Enter to commit, Esc to cancel) ─{RESET}\n",
                id = state.selected_id().unwrap_or("?"),
                buffer = state.edit_buffer,
            ));
        }
    }

    if let Some(msg) = &state.status {
        out.push_str(&format!("  {DIM}{msg}{RESET}\n"));
    }

    out
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

// =============================================================================
// TUI driver
// =============================================================================

pub struct Tui {
    engine: Arc<QuiltEngine>,
    state: TuiState,
}

impl Tui {
    pub fn new(engine: Arc<QuiltEngine>) -> Self {
        let sheet_id = engine.id().to_string();
        let cells = snapshot_cells(&engine);
        let viewport_height = 20;
        Self {
            engine,
            state: TuiState {
                sheet_id,
                cells,
                selected: 0,
                view_top: 0,
                viewport_height,
                mode: TuiMode::Normal,
                edit_buffer: String::new(),
                status: None,
            },
        }
    }

    pub fn state(&self) -> &TuiState {
        &self.state
    }

    pub fn render_to_string(&self) -> String {
        render(&self.state)
    }

    pub fn refresh(&mut self) {
        self.state.cells = snapshot_cells(&self.engine);
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode().context("enable raw mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).context("enter alternate screen")?;

        let result = self.run_loop(&mut out);

        let _ = execute!(out, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        result
    }

    fn run_loop(&mut self, out: &mut std::io::Stdout) -> Result<()> {
        write!(out, "{}", render(&self.state))?;
        out.flush()?;

        loop {
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            let ev = event::read()?;
            let Event::Key(key) = ev else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if self.handle_key(key) == TuiAction::Quit {
                return Ok(());
            }
            write!(out, "{}", render(&self.state))?;
            out.flush()?;
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> TuiAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return TuiAction::Quit;
        }

        match self.state.mode {
            TuiMode::Normal => self.handle_normal(key),
            TuiMode::Set => self.handle_set(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => TuiAction::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                if self.state.selected + 1 < self.state.cells.len() {
                    self.state.selected += 1;
                    self.maybe_scroll();
                }
                TuiAction::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.state.selected > 0 {
                    self.state.selected -= 1;
                    self.maybe_scroll();
                }
                TuiAction::Continue
            }
            KeyCode::Char('g') => {
                self.state.selected = 0;
                self.state.view_top = 0;
                TuiAction::Continue
            }
            KeyCode::Char('G') => {
                if !self.state.cells.is_empty() {
                    self.state.selected = self.state.cells.len() - 1;
                    self.maybe_scroll();
                }
                TuiAction::Continue
            }
            KeyCode::Char('s') => {
                if let Some(row) = self.state.cells.get(self.state.selected) {
                    if matches!(row.kind, CellKind::Value | CellKind::Sensor) {
                        self.state.mode = TuiMode::Set;
                        self.state.edit_buffer = row.value.clone();
                        self.state.status = Some(format!("setting {}", row.id));
                    } else {
                        self.state.status = Some(format!(
                            "cannot set: {} is a {:?} cell",
                            row.id, row.kind
                        ));
                    }
                }
                TuiAction::Continue
            }
            KeyCode::Char('r') => {
                self.refresh();
                self.state.status = Some("reloaded".to_string());
                TuiAction::Continue
            }
            _ => TuiAction::Continue,
        }
    }

    fn handle_set(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Enter => {
                let value = self.state.edit_buffer.clone();
                let id = self.state.selected_id().unwrap_or("").to_string();
                if id.is_empty() {
                    self.state.mode = TuiMode::Normal;
                    return TuiAction::Continue;
                }
                let parsed: serde_json::Value = serde_json::from_str(&value)
                    .or_else(|_| serde_json::from_str(&format!("\"{}\"", value)))
                    .unwrap_or(serde_json::Value::Null);
                match self
                    .engine
                    .set(&id, parsed.clone(), CallerContext::default())
                {
                    Ok(_) => {
                        self.state.status = Some(format!("set {} = {}", id, parsed));
                        self.refresh();
                    }
                    Err(e) => {
                        self.state.status = Some(format!("set failed: {}", e));
                    }
                }
                self.state.mode = TuiMode::Normal;
                self.state.edit_buffer.clear();
                TuiAction::Continue
            }
            KeyCode::Esc => {
                self.state.mode = TuiMode::Normal;
                self.state.edit_buffer.clear();
                self.state.status = Some("cancelled".to_string());
                TuiAction::Continue
            }
            KeyCode::Backspace => {
                self.state.edit_buffer.pop();
                TuiAction::Continue
            }
            KeyCode::Char(c) => {
                self.state.edit_buffer.push(c);
                TuiAction::Continue
            }
            _ => TuiAction::Continue,
        }
    }

    fn maybe_scroll(&mut self) {
        if self.state.selected >= self.state.view_top + self.state.viewport_height {
            self.state.view_top = self.state.selected + 1 - self.state.viewport_height;
        } else if self.state.selected < self.state.view_top {
            self.state.view_top = self.state.selected;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TuiAction {
    Continue,
    Quit,
}

// =============================================================================
// Helpers
// =============================================================================

fn snapshot_cells(engine: &Arc<QuiltEngine>) -> Vec<CellRow> {
    let cells = engine.list_cells();
    cells
        .iter()
        .map(|cell| {
            let id = cell.def.id.clone();
            let deps: Vec<String> = cell.dependencies.iter().cloned().collect();
            let dependents: Vec<String> = cell.dependents.iter().cloned().collect();
            let value = cell.value.data.to_string();
            let status = TuiStatus::from(cell.value.status);
            let error = cell.value.error.as_ref().map(|e| e.message.clone());
            CellRow {
                id,
                kind: cell.def.kind,
                value,
                status,
                error,
                dependencies: deps,
                dependents,
            }
        })
        .collect()
}

#[allow(dead_code)]
fn _unused(_: CellId) {}

#[cfg(test)]
mod render_test;
