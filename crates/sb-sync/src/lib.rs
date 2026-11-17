//! Peer-to-peer sync for «Second Brain» vaults: per-vault state, pairing tickets and peer authentication.

mod auth;
pub mod control;
mod doc;
mod engine;
mod error;
mod local;
mod net;
mod plan;
mod protocol;
mod replica;
mod session;
mod state;
mod store;
mod ticket;

pub use auth::{GroupTag, group_tag, proof, verify};
pub use doc::{Hash32, NoteId, NoteView, SyncDoc, conflict_path, content_hash};
pub use engine::{Command, DEBOUNCE, EngineEvent, EngineHandle, Network, RESCAN, TICK};
pub use error::{Result, SyncError};
pub use local::{LocalReplica, MAX_PASSES, Outcome};
pub use net::{DIAL_TIMEOUT, IrohNetwork, ensure_identity, start_engine};
pub use plan::{
    BaseChange, DiskAction, DiskEntry, DiskScan, DiskState, DocAction, PlanInput, Step, plan, scan,
};
pub use protocol::{
    ALPN, FRAME_MAX, HELLO_MAX, Message, PROTOCOL_VERSION, read_frame, write_frame,
};
pub use replica::{FRESH_JOIN_WAIT, MAX_PEERS_PER_MESSAGE, Outgoing, Replica, SessionId, Tick};
pub use session::{Role, SessionEvent, SessionParams, run_session};
pub use state::{
    BlockedNote, DeviceId, EngineLock, NetConfig, NetMode, PeerEntry, PeerStatus, PendingCommand,
    Secret, StateDir, Status, base_dir_from, from_hex32, random_bytes, to_hex, vault_dir_name,
};
pub use store::{Base, COMPACT_LOG_BYTES, DocStore};
pub use ticket::Ticket;
