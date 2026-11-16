//! Peer-to-peer sync for «Second Brain» vaults: per-vault state, pairing tickets and peer authentication.

mod auth;
mod error;
mod state;
mod ticket;

pub use auth::{proof, verify};
pub use error::{Result, SyncError};
pub use state::{
    BlockedNote, DeviceId, EngineLock, NetConfig, NetMode, PeerEntry, PeerStatus, PendingCommand,
    Secret, StateDir, Status, base_dir_from, from_hex32, random_bytes, to_hex, vault_dir_name,
};
pub use ticket::Ticket;
