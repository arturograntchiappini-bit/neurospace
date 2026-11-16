//! Peer-to-peer sync for «Second Brain» vaults: per-vault state, pairing tickets and peer authentication.

mod auth;
mod doc;
mod error;
mod local;
mod plan;
mod state;
mod store;
mod ticket;

pub use auth::{proof, verify};
pub use doc::{Hash32, NoteId, NoteView, SyncDoc, conflict_path, content_hash};
pub use error::{Result, SyncError};
pub use local::{LocalReplica, MAX_PASSES, Outcome};
pub use plan::{
    BaseChange, DiskAction, DiskEntry, DiskScan, DiskState, DocAction, PlanInput, Step, plan, scan,
};
pub use state::{
    BlockedNote, DeviceId, EngineLock, NetConfig, NetMode, PeerEntry, PeerStatus, PendingCommand,
    Secret, StateDir, Status, base_dir_from, from_hex32, random_bytes, to_hex, vault_dir_name,
};
pub use store::{Base, COMPACT_LOG_BYTES, DocStore};
pub use ticket::Ticket;
