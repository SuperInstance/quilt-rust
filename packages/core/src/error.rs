//! # error.rs
//!
//! Error types for the quilt runtime.
//!
//! ## Role in the system
//!
//! Centralizes error handling. Library errors use `thiserror`; the CLI
//! uses `anyhow` at the binary boundary. The runtime never panics on
//! bad input — bad YAML, bad cell ids, broken formulas all surface as
//! `Error` values that the caller can render or ignore.
//!
//! ## Depends on
//!
//! - `thiserror` — for the `Error` derive.
//! - `std::cell` — for the `CellDef` reference (no `RefCell`, just a
//!   borrow of the static string).
//!
//! ## Used by
//!
//! - Every module returns `Result<T, Error>`.
//!
//! ## Key decisions
//!
//! - We don't implement `From<anyhow::Error>`; `anyhow` is for the CLI
//!   and stays at the binary boundary.
//! - `Error::CellNotFound` is the only "not found" variant; everything
//!   else is a typed error with context.

use crate::types::CellId;

/// Convenient alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

/// The library error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A cell with the given id is not defined in the engine.
    #[error("cell not found: {0}")]
    CellNotFound(CellId),

    /// A cell with the given id is already defined.
    #[error("cell already defined: {0}")]
    CellAlreadyDefined(CellId),

    /// The YAML could not be parsed.
    #[error("failed to parse sheet: {message}")]
    ParseError {
        /// Human-readable message.
        message: String,
        /// Optional source location.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// A cell's definition is invalid (e.g. formula with no expr).
    #[error("invalid cell definition for '{id}': {message}")]
    InvalidCellDef {
        /// The cell id.
        id: CellId,
        /// Why it's invalid.
        message: String,
    },

    /// A sheet's definition is invalid.
    #[error("invalid sheet: {message}")]
    InvalidSheet {
        /// Why it's invalid.
        message: String,
    },

    /// Tried to push to a non-push cell.
    #[error("cannot push to {kind} cell '{id}'")]
    NotPushable {
        /// The cell id.
        id: CellId,
        /// The kind that doesn't support push.
        kind: String,
    },

    /// Tried to set a non-settable cell.
    #[error("cannot set {kind} cell '{id}'")]
    NotSettable {
        /// The cell id.
        id: CellId,
        /// The kind that doesn't support set.
        kind: String,
    },

    /// An HTTP error during an API cell evaluation.
    #[error("http error: {0}")]
    Http(String),

    /// A network or IO error during an API cell evaluation.
    #[error("network error: {0}")]
    Network(String),

    /// A scripting error from rhai (program or formula).
    #[error("script error in '{cell}': {message}")]
    ScriptError {
        /// The cell id.
        cell: CellId,
        /// The error message.
        message: String,
    },

    /// A JSON serialization error.
    #[error("serialization error: {0}")]
    Serde(String),

    /// YAML serialization error.
    #[error("yaml error: {0}")]
    Yaml(String),

    /// A configuration error (bad EngineOptions, bad URL, etc.).
    #[error("config error: {0}")]
    Config(String),

    /// Catch-all for things we didn't expect. Always accompanied by a
    /// message; the `source` chain (if any) lives in the `#[source]`
    /// field below.
    #[error("{message}")]
    Other {
        /// What went wrong.
        message: String,
        /// Underlying cause, if any.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl Error {
    /// Convenience constructor for `Error::Other`.
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            message: message.into(),
            source: None,
        }
    }

    /// Convenience constructor for `Error::Other` with a source.
    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Other {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Convenience constructor for `Error::ParseError`.
    pub fn parse(message: impl Into<String>) -> Self {
        Self::ParseError {
            message: message.into(),
            source: None,
        }
    }

    /// Convenience constructor for `Error::InvalidCellDef`.
    pub fn invalid_cell(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidCellDef {
            id: id.into(),
            message: message.into(),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err.to_string())
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<serde_yml::Error> for Error {
    fn from(err: serde_yml::Error) -> Self {
        Self::Yaml(err.to_string())
    }
}
