//! Relax sync engine: scanning, comparing and (later) synchronizing folders.
//! Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

pub mod compare;
pub mod db;
pub mod execute;
pub mod filter;
pub mod plan;
pub mod scan;
pub mod winfs;

use serde::{Serialize, Serializer};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared flag that lets the UI stop a long-running operation.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Which side of a folder pair an item or error belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Left,
    Right,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Left => write!(f, "left"),
            Side::Right => write!(f, "right"),
        }
    }
}

/// Errors that stop an engine operation completely.
/// Problems with single files never stop the operation; they are collected as `ItemError`s instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    Cancelled,
    /// Neither folder of a pair exists, so there is nothing to compare.
    BothFoldersMissing {
        left: String,
        right: String,
    },
    /// A base path exists but is a file, not a folder.
    NotAFolder {
        path: String,
    },
    /// The base folder exists but cannot be read (permissions, disconnected drive, ...).
    CannotReadFolder {
        path: String,
        message: String,
    },
    /// The sync plan failed a safety check; the text explains why.
    Blocked(String),
    /// The target drive does not have enough free space for the files to be copied.
    NotEnoughSpace {
        path: String,
        needed: u64,
        available: u64,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Cancelled => write!(f, "The operation was cancelled."),
            EngineError::BothFoldersMissing { left, right } => {
                write!(f, "Neither folder exists:\n{left}\n{right}")
            }
            EngineError::NotAFolder { path } => write!(f, "This path is a file, not a folder:\n{path}"),
            EngineError::CannotReadFolder { path, message } => {
                write!(f, "Cannot read folder:\n{path}\n{message}")
            }
            EngineError::Blocked(reason) => write!(f, "{reason}"),
            EngineError::NotEnoughSpace {
                path,
                needed,
                available,
            } => write!(
                f,
                "Not enough free space on the drive of:\n{path}\nNeeded: {:.2} GB, available: {:.2} GB.",
                *needed as f64 / 1e9,
                *available as f64 / 1e9
            ),
        }
    }
}

impl std::error::Error for EngineError {}

impl Serialize for EngineError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// A problem with a single file or folder that was skipped, shown to the user after the operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemError {
    pub side: Side,
    pub path: String,
    pub message: String,
}
