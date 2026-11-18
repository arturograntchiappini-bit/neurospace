use std::path::PathBuf;

use crate::path::PathError;

/// Everything that can go wrong in `ns-core`.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A path from outside the process is not a valid note path.
    #[error("invalid note path `{path}`: {source}")]
    InvalidPath {
        /// The rejected raw path.
        path: String,
        /// The rule it broke.
        source: PathError,
    },
    /// No note exists at the given path.
    #[error("note not found: {0}")]
    NotFound(String),
    /// Something already occupies the destination.
    #[error("a note already exists at {0}")]
    AlreadyExists(String),
    /// A location that must be a directory is not one.
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    /// A symbolic link sits between the vault root and the target.
    #[error("a symbolic link is in the way of {0}")]
    SymlinkInPath(String),
    /// A note's bytes are not valid UTF-8.
    #[error("{0} is not valid UTF-8")]
    NotUtf8(String),
    /// The caller supplied content the vault refuses.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The operating system reported an error.
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        /// The file or directory the operation was acting on.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Result alias used across `ns-core`.
pub type Result<T> = std::result::Result<T, CoreError>;
