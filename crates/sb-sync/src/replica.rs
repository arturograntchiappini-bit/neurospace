//! A sync replica without I/O of its own (spec section 8.7): it turns peer messages into
//! document changes and outgoing messages. The caller moves the messages.

use std::collections::{BTreeMap, BTreeSet};

use sb_core::Vault;

use crate::doc::SyncDoc;
use crate::error::{Result, SyncError};
use crate::local::{LocalReplica, Outcome};
use crate::protocol::Message;
use crate::state::{DeviceId, PeerEntry, StateDir, from_hex32, to_hex};

/// The most device ids taken from one `Peers` message, or put into one.
pub const MAX_PEERS_PER_MESSAGE: usize = 64;

/// Identifies one open session; chosen by the host.
pub type SessionId = u64;

/// A message for the peer on one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The session to send it on.
    pub session: SessionId,
    /// What to send.
    pub message: Message,
}

struct Session {
    peer: DeviceId,
    /// What the peer is known to hold, so nothing is sent back to where it came from.
    known: Vec<u8>,
}

/// A vault, its sync document and the open sessions to peers.
pub struct Replica {
    local: LocalReplica,
    device_id: DeviceId,
    sessions: BTreeMap<SessionId, Session>,
    outcome: Outcome,
}

impl Replica {
    /// Opens the replica of a vault.
    ///
    /// # Errors
    /// Whatever `LocalReplica::open` reports.
    pub fn open(vault: Vault, state: StateDir, device_id: DeviceId) -> Result<Replica> {
        Ok(Replica {
            local: LocalReplica::open(vault, state)?,
            device_id,
            sessions: BTreeMap::new(),
            outcome: Outcome::default(),
        })
    }

    /// The vault, state, document and base.
    pub fn local(&self) -> &LocalReplica {
        &self.local
    }

    /// This device's id.
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// What the last local reconciliation did.
    pub fn last_outcome(&self) -> &Outcome {
        &self.outcome
    }

    /// One local round, then updates for every session that lacks something.
    ///
    /// # Errors
    /// Failures of the vault's state, e.g. `VaultMarkerMissing`.
    pub fn reconcile(&mut self) -> Result<Vec<Outgoing>> {
        self.reconcile_local()?;
        self.updates_for_sessions()
    }

    /// Registers a session; the answer is this side's `Version` and `Peers`.
    pub fn session_opened(&mut self, session: SessionId, peer: DeviceId) -> Vec<Outgoing> {
        let version = self.local.doc.version();
        let known = SyncDoc::new().version();
        self.sessions.insert(session, Session { peer, known });
        [
            Message::Version(version),
            Message::Peers(self.known_peer_ids()),
        ]
        .map(|message| Outgoing { session, message })
        .into()
    }

    /// Forgets a session.
    pub fn session_closed(&mut self, session: SessionId) {
        self.sessions.remove(&session);
    }

    /// Handles a message from an authenticated peer. A message for a session that is not
    /// open is ignored.
    ///
    /// # Errors
    /// `Doc` for a version or updates the document refuses, `Protocol` for a `Hello`,
    /// and failures of the vault's state. The replica stays usable after any of them.
    pub fn on_message(&mut self, session: SessionId, message: Message) -> Result<Vec<Outgoing>> {
        if !self.sessions.contains_key(&session) {
            return Ok(Vec::new());
        }
        match message {
            Message::Hello { .. } => Err(SyncError::Protocol("Hello after the handshake".into())),
            Message::Version(version) => self.on_version(session, &version),
            Message::Updates { version, updates } => self.on_updates(session, &version, &updates),
            Message::Peers(ids) => self.on_peers(&ids).map(|()| Vec::new()),
        }
    }

    /// The devices with an open session, each once, in id order.
    pub fn connected_peers(&self) -> Vec<DeviceId> {
        let peers: BTreeSet<DeviceId> = self.sessions.values().map(|state| state.peer).collect();
        peers.into_iter().collect()
    }

    fn reconcile_local(&mut self) -> Result<()> {
        self.outcome = self.local.reconcile()?;
        Ok(())
    }

    fn on_version(&mut self, session: SessionId, peer_version: &[u8]) -> Result<Vec<Outgoing>> {
        let doc = &self.local.doc;
        let updates = doc.updates_for(peer_version)?.unwrap_or_default();
        let version = doc.version();
        let known = SyncDoc::merge_versions(peer_version, &version)?;
        if let Some(state) = self.sessions.get_mut(&session) {
            state.known = known;
        }
        let message = Message::Updates { version, updates };
        Ok(vec![Outgoing { session, message }])
    }

    /// Local first, so pending local edits enter the document before the remote ones merge.
    fn on_updates(
        &mut self,
        session: SessionId,
        peer_version: &[u8],
        updates: &[u8],
    ) -> Result<Vec<Outgoing>> {
        self.learn_version(session, peer_version)?;
        self.reconcile_local()?;
        let pending = self.import_updates(updates)?;
        self.reconcile_local()?;
        let mut outgoing = Vec::new();
        if pending {
            let message = Message::Version(self.local.doc.version());
            outgoing.push(Outgoing { session, message });
        }
        outgoing.extend(self.updates_for_sessions()?);
        Ok(outgoing)
    }

    /// Persists the document before anything else happens; `true` when history is missing.
    fn import_updates(&mut self, updates: &[u8]) -> Result<bool> {
        if updates.is_empty() {
            return Ok(false);
        }
        let pending = self.local.doc.import(updates)?;
        self.local.store.persist(&self.local.doc)?;
        Ok(pending)
    }

    fn learn_version(&mut self, session: SessionId, version: &[u8]) -> Result<()> {
        if let Some(state) = self.sessions.get_mut(&session) {
            state.known = SyncDoc::merge_versions(&state.known, version)?;
        }
        Ok(())
    }

    fn on_peers(&self, ids: &[DeviceId]) -> Result<()> {
        let foreign = ids.iter().filter(|id| **id != self.device_id);
        for id in foreign.take(MAX_PEERS_PER_MESSAGE) {
            let entry = PeerEntry {
                id: to_hex(id),
                addrs: Vec::new(),
            };
            self.local.state.add_peer(entry)?;
        }
        Ok(())
    }

    fn known_peer_ids(&self) -> Vec<DeviceId> {
        // shortcut: an unreadable peers.json only skips the gossip, the host reports it elsewhere.
        let entries = self.local.state.peers().unwrap_or_default();
        let ids = entries.iter().filter_map(|entry| from_hex32(&entry.id));
        ids.take(MAX_PEERS_PER_MESSAGE).collect()
    }

    /// For every session whose peer lacks something: the updates, then the peer counts as up to date.
    fn updates_for_sessions(&mut self) -> Result<Vec<Outgoing>> {
        let doc = &self.local.doc;
        let version = doc.version();
        let mut outgoing = Vec::new();
        for (&session, state) in &mut self.sessions {
            let Some(updates) = doc.updates_for(&state.known)? else {
                continue;
            };
            state.known = SyncDoc::merge_versions(&state.known, &version)?;
            let message = Message::Updates {
                version: version.clone(),
                updates,
            };
            outgoing.push(Outgoing { session, message });
        }
        Ok(outgoing)
    }
}
