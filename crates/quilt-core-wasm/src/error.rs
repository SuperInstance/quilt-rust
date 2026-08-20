//! # error.rs (wasm tier)
//!
//! A minimal, dependency-free error type for the wasm core.
//!
//! `thiserror` compiles for wasm32-unknown-unknown, but this crate keeps
//! its dependency surface to exactly `serde` + `serde_json`, so the
//! `Display`/`Error` impls are hand-rolled. The shape mirrors the
//! vocabulary of `packages/core/src/error.rs` closely enough that the
//! single-sourced ledger module (which calls `Error::other`) and callers
//! porting between tiers feel at home.

use std::fmt;

/// Convenient alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

/// The wasm-core error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A cell with the given id is not defined in the engine.
    CellNotFound(String),
    /// Tried to push to a cell that does not accept pushes.
    NotPushable {
        /// The cell id.
        id: String,
        /// The kind that doesn't support push.
        kind: String,
    },
    /// A formula could not be parsed.
    FormulaParse(String),
    /// A formula evaluated to an error (unknown cell, bad types, ...).
    FormulaEval(String),
    /// The sheet definition could not be deserialized.
    InvalidSheet(String),
    /// A ledger error (unknown ticket, ...). Used by the single-sourced
    /// ledger module via `Error::other`.
    Ledger(String),
}

impl Error {
    /// Catch-all constructor used by the single-sourced ledger module.
    pub fn other(message: impl Into<String>) -> Self {
        Self::Ledger(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CellNotFound(id) => write!(f, "cell not found: {id}"),
            Error::NotPushable { id, kind } => write!(f, "cannot push to {kind} cell '{id}'"),
            Error::FormulaParse(msg) => write!(f, "formula parse error: {msg}"),
            Error::FormulaEval(msg) => write!(f, "formula eval error: {msg}"),
            Error::InvalidSheet(msg) => write!(f, "invalid sheet: {msg}"),
            Error::Ledger(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}
