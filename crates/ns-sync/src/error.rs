use std::path::PathBuf;

/// Everything that can go wrong in `ns-sync`.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// An error from the vault library.
    #[error(transparent)]
    Core(#[from] ns_core::CoreError),
    /// The operating system reported an error.
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        /// The file or directory the operation was acting on.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Neither `NS_STATE_DIR` nor a platform default location is available.
    #[error("cannot find a place for sync state: set NS_STATE_DIR")]
    NoStateDir,
    /// A pairing ticket does not parse.
    #[error("invalid ticket: {0}")]
    InvalidTicket(String),
    /// The engine lock of this vault is held by another process.
    #[error("another sync engine is already running for this vault")]
    AlreadyRunning,
    /// The vault has no group secret yet.
    #[error("sync is not set up for this vault")]
    NotConfigured,
    /// A state file exists but its content is unusable.
    #[error("corrupt sync state in {}: {reason}", .path.display())]
    CorruptState {
        /// The offending file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The vault folder lost the marker that tells it is the synced vault.
    #[error(
        "{} is missing: this does not look like the vault that was synced. If this is the vault, \
         create an empty `.neurospace` folder in it; otherwise run `ns sync leave`.",
        .0.display()
    )]
    VaultMarkerMissing(PathBuf),
    /// The sync document rejected an operation.
    #[error("sync document error: {0}")]
    Doc(String),
    /// A peer broke the wire protocol.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The peer's proof of group membership did not verify.
    #[error("the peer is not a member of this sync group")]
    AuthFailed,
    /// The list of known devices is full, so the device cannot be remembered.
    #[error(
        "the device list is full ({}): the new device cannot be remembered",
        crate::state::MAX_STORED_PEERS
    )]
    PeerListFull,
    /// The transport failed.
    #[error("network error: {0}")]
    Net(String),
}

/// Result alias used across `ns-sync`.
pub type Result<T> = std::result::Result<T, SyncError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_tell_how_to_get_out_when_the_marker_is_missing() {
        let marker = PathBuf::from("v").join(".neurospace");
        let text = SyncError::VaultMarkerMissing(marker).to_string();
        assert!(
            text.contains("create an empty `.neurospace` folder"),
            "{text}"
        );
        assert!(text.contains("`ns sync leave`"), "{text}");
    }
}
