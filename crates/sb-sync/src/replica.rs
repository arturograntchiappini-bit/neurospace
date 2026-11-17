//! A sync replica without I/O of its own (spec section 8.7): it turns peer messages into
//! document changes and outgoing messages. The caller moves the messages.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sb_core::Vault;

use crate::doc::SyncDoc;
use crate::error::{Result, SyncError};
use crate::local::{LocalReplica, Outcome};
use crate::protocol::Message;
use crate::state::{
    DeviceId, PeerEntry, PeerStatus, PendingCommand, Secret, StateDir, Status, from_hex32, to_hex,
};
use crate::ticket::Ticket;

/// The most device ids taken from one `Peers` message, or put into one.
pub const MAX_PEERS_PER_MESSAGE: usize = 64;

/// How long a replica that just joined a group waits for the group's state before it
/// records its own notes (spec section 8.6).
pub const FRESH_JOIN_WAIT: Duration = Duration::from_secs(20);

/// Seconds since the Unix epoch; 0 when the clock is before it.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// What the host must do after `tick`, `join` or `leave`.
#[derive(Debug, Default)]
pub struct Tick {
    /// Messages to send.
    pub outgoing: Vec<Outgoing>,
    /// Sessions the replica dropped; the host closes their connections.
    pub close_sessions: Vec<SessionId>,
    /// The replica left its group: sync is off and the engine should stop.
    pub left: bool,
    /// The group secret changed: connections and the tickets issued so far are void.
    pub secret_changed: bool,
}

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
    /// What the peer is known to hold, so nothing is sent back to where it came from. `None`
    /// until its `Version` arrives: nothing is exported to a peer whose holdings are unknown.
    known: Option<Vec<u8>>,
}

/// A vault, its sync document and the open sessions to peers.
pub struct Replica {
    local: LocalReplica,
    device_id: DeviceId,
    sessions: BTreeMap<SessionId, Session>,
    outcome: Outcome,
    addresses: Vec<SocketAddr>,
    relay: Option<String>,
    /// Mirrors the `fresh-join` marker: no reconciliation until a first state was imported.
    waiting: bool,
    /// Engine time at which the wait ends; set at the first tick when opened with the marker.
    wait_deadline: Option<Duration>,
    last_error: Option<String>,
}

impl Replica {
    /// Opens the replica of a vault.
    ///
    /// # Errors
    /// Whatever `LocalReplica::open` reports.
    pub fn open(vault: Vault, state: StateDir, device_id: DeviceId) -> Result<Replica> {
        let waiting = state.fresh_join();
        let mut local = LocalReplica::open(vault, state)?;
        let last_error = local.notice.take();
        Ok(Replica {
            local,
            device_id,
            sessions: BTreeMap::new(),
            outcome: Outcome::default(),
            addresses: Vec::new(),
            relay: None,
            waiting,
            wait_deadline: None,
            last_error,
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

    /// One local round, then updates for every session that lacks something. While the
    /// replica waits for the group's first state it does nothing.
    ///
    /// # Errors
    /// Failures of the vault's state, e.g. `VaultMarkerMissing`.
    pub fn reconcile(&mut self) -> Result<Vec<Outgoing>> {
        if self.waiting {
            return Ok(Vec::new());
        }
        self.reconcile_local()?;
        self.updates_for_sessions()
    }

    /// Registers a session and remembers its peer, so that a device that dialled in is dialled back and
    /// passed on in `Peers`; the answer is this side's `Version` and `Peers`. A peers file that cannot be
    /// written is recorded in `last_error` and never fails the session.
    pub fn session_opened(&mut self, session: SessionId, peer: DeviceId) -> Vec<Outgoing> {
        if let Err(error) = self.remember_peer(&peer) {
            self.last_error = Some(error.to_string());
        }
        let version = self.local.doc.version();
        let known = None;
        self.sessions.insert(session, Session { peer, known });
        [
            Message::Version(version),
            Message::Peers(self.known_peer_ids()),
        ]
        .map(|message| Outgoing { session, message })
        .into()
    }

    fn remember_peer(&self, peer: &DeviceId) -> Result<()> {
        if *peer == self.device_id {
            return Ok(());
        }
        let entry = PeerEntry {
            id: to_hex(peer),
            addrs: Vec::new(),
        };
        self.local.state.add_peer(entry).map(drop)
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
    /// and failures of the vault's state. The replica stays usable after any of them, but the
    /// host must close that session after an `Err`: this side may have dropped what the peer
    /// sent, and only the `Version` exchange of a new session fetches it again.
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

    /// Whether the replica joined a group and still waits for that group's first state.
    pub fn is_waiting_for_first_state(&self) -> bool {
        self.waiting
    }

    /// Sets the addresses this device listens on, for tickets and status.
    pub fn set_addresses(&mut self, addresses: Vec<SocketAddr>) {
        self.addresses = addresses;
    }

    /// Sets the relay the network reached, for status.
    pub fn set_relay(&mut self, relay: Option<String>) {
        self.relay = relay;
    }

    /// A ticket for this device and the group.
    ///
    /// # Errors
    /// `NotConfigured` without a group secret.
    pub fn invite(&self) -> Result<Ticket> {
        let secret = self.local.state.secret()?.ok_or(SyncError::NotConfigured)?;
        Ok(Ticket {
            device_id: self.device_id,
            secret,
            addrs: self.addresses.clone(),
        })
    }

    /// What `status.json` shows now. The note count is the one the last round saw, so it can
    /// lag by a round: counting the document on every status request is too costly.
    pub fn status(&self) -> Status {
        let connected = self.connected_peers();
        // shortcut: an unreadable peers.json shows no peers; the engine meets the same error elsewhere.
        let known = self.local.state.peers().unwrap_or_default();
        let peers = known
            .into_iter()
            .map(|entry| peer_status(entry, &connected))
            .collect();
        Status {
            pid: std::process::id(),
            device_id: to_hex(&self.device_id),
            addresses: self.addresses.iter().map(ToString::to_string).collect(),
            peers,
            notes: self.outcome.notes,
            blocked: self.outcome.blocked.clone(),
            rejected_entries: self.outcome.rejected_entries,
            relay: self.relay.clone(),
            last_error: self.last_error.clone(),
            updated_unix: unix_now(),
        }
    }

    /// `now` is the time since the engine started. Carries out a pending command, ends an
    /// expired fresh-join wait and writes `status.json`. A bad pending command is dropped and
    /// reported in the status; only failures of the state directory or the document are errors.
    /// Once a command changed the group, its `Tick` is always returned: a later failure of the
    /// same tick is reported in `last_error` instead, so the host still learns of the change.
    /// Hosts should also re-read `secret` on every tick (spec section 8.3).
    ///
    /// # Errors
    /// Failures of the state directory, the document or the vault's state.
    pub fn tick(&mut self, now: Duration) -> Result<Tick> {
        let mut tick = self.run_pending_command(now)?;
        if tick.left {
            return Ok(tick);
        }
        if let Err(error) = self.finish_tick(now, &mut tick) {
            if !tick.secret_changed {
                return Err(error);
            }
            self.last_error = Some(error.to_string());
            // shortcut: the second status write is best effort, the failure is already recorded.
            let _ = self.local.state.write_status(&self.status());
        }
        Ok(tick)
    }

    fn finish_tick(&mut self, now: Duration, tick: &mut Tick) -> Result<()> {
        tick.outgoing.extend(self.end_expired_wait(now)?);
        self.local.state.write_status(&self.status())
    }

    /// Joins the group of a ticket. Its own group only adds the inviting peer. Another group
    /// replaces the sync state, in an order a crash survives: the marker first, then the
    /// document and base of the old group, then the peers, and the secret last, because the
    /// secret is what makes the device a member. Local notes are never touched.
    ///
    /// # Errors
    /// `InvalidTicket` for a ticket of this very device; failures of the state directory.
    pub fn join(&mut self, ticket: &Ticket, now: Duration) -> Result<Tick> {
        if ticket.device_id == self.device_id {
            return Err(SyncError::InvalidTicket(
                "the ticket belongs to this device".to_string(),
            ));
        }
        let inviter = PeerEntry {
            id: to_hex(&ticket.device_id),
            addrs: ticket.addrs.iter().map(ToString::to_string).collect(),
        };
        if self.local.state.secret()? == Some(ticket.secret) {
            self.local.state.add_peer(inviter)?;
            return Ok(Tick::default());
        }
        self.replace_group(ticket.secret, inviter, now)
    }

    /// Turns sync off: the document, base, secret, peers and markers go, the sessions close.
    /// The vault's files and the device key stay.
    ///
    /// # Errors
    /// Failures of the state directory.
    pub fn leave(&mut self) -> Result<Tick> {
        self.local.reset_to_empty()?;
        self.local.state.clear_group()?;
        self.end_wait_in_memory();
        Ok(Tick {
            close_sessions: self.drop_sessions(),
            left: true,
            ..Tick::default()
        })
    }

    fn replace_group(&mut self, secret: Secret, inviter: PeerEntry, now: Duration) -> Result<Tick> {
        self.local.state.set_fresh_join(true)?;
        self.local.reset_to_empty()?;
        self.local.state.save_peers(&[inviter])?;
        self.local.state.save_secret(&secret)?;
        self.waiting = true;
        self.outcome.notes = 0;
        self.wait_deadline = Some(now + FRESH_JOIN_WAIT);
        Ok(Tick {
            close_sessions: self.drop_sessions(),
            secret_changed: true,
            ..Tick::default()
        })
    }

    fn drop_sessions(&mut self) -> Vec<SessionId> {
        std::mem::take(&mut self.sessions).into_keys().collect()
    }

    fn run_pending_command(&mut self, now: Duration) -> Result<Tick> {
        let command = match self.local.state.take_pending() {
            Ok(Some(command)) => command,
            Ok(None) => return Ok(Tick::default()),
            Err(SyncError::CorruptState { .. }) => {
                return Ok(self.drop_command("a corrupt command.pending was dropped"));
            }
            Err(error) => return Err(error),
        };
        // A command that fails is reported below; one that works leaves nothing to report.
        self.last_error = None;
        match command {
            PendingCommand::Join(text) => self.join_from_text(&text, now),
            PendingCommand::Leave(true) => self.leave(),
            PendingCommand::Leave(false) => Ok(Tick::default()),
        }
    }

    fn join_from_text(&mut self, text: &str, now: Duration) -> Result<Tick> {
        let joined = text
            .parse::<Ticket>()
            .and_then(|ticket| self.join(&ticket, now));
        match joined {
            Err(SyncError::InvalidTicket(_)) => {
                Ok(self.drop_command("a pending join with an invalid ticket was dropped"))
            }
            other => other,
        }
    }

    /// The text never quotes the command: a ticket holds the group secret.
    fn drop_command(&mut self, why: &str) -> Tick {
        self.last_error = Some(why.to_string());
        Tick::default()
    }

    /// Starts the wait of a replica opened with the marker at its first tick, and ends it
    /// once the deadline is reached.
    fn end_expired_wait(&mut self, now: Duration) -> Result<Vec<Outgoing>> {
        if !self.waiting {
            return Ok(Vec::new());
        }
        let deadline = *self.wait_deadline.get_or_insert(now + FRESH_JOIN_WAIT);
        if now < deadline {
            return Ok(Vec::new());
        }
        self.end_wait()?;
        self.reconcile()
    }

    /// Clears the marker; the caller reconciles.
    fn end_wait(&mut self) -> Result<()> {
        if !self.waiting {
            return Ok(());
        }
        self.local.state.set_fresh_join(false)?;
        self.end_wait_in_memory();
        Ok(())
    }

    fn end_wait_in_memory(&mut self) {
        self.waiting = false;
        self.wait_deadline = None;
    }

    fn reconcile_local(&mut self) -> Result<()> {
        if self.waiting {
            return Ok(());
        }
        self.outcome = self.local.reconcile()?;
        if let Some(notice) = &self.outcome.notice {
            self.last_error = Some(notice.clone());
        }
        Ok(())
    }

    fn on_version(&mut self, session: SessionId, peer_version: &[u8]) -> Result<Vec<Outgoing>> {
        self.local.store.persist(&self.local.doc)?;
        let doc = &self.local.doc;
        let updates = doc.updates_for(peer_version)?.unwrap_or_default();
        let version = doc.version();
        let known = SyncDoc::merge_versions(peer_version, &version)?;
        if let Some(state) = self.sessions.get_mut(&session) {
            state.known = Some(known);
        }
        let message = Message::Updates { version, updates };
        Ok(vec![Outgoing { session, message }])
    }

    /// Local first, so pending local edits enter the document before the remote ones merge;
    /// the first state of a group is imported without it, then the wait is over. A message
    /// without updates changes nothing, so it costs no reconciliation (unless a wait ends).
    fn on_updates(
        &mut self,
        session: SessionId,
        peer_version: &[u8],
        updates: &[u8],
    ) -> Result<Vec<Outgoing>> {
        let merges = !updates.is_empty() || self.waiting;
        if merges {
            self.reconcile_local()?;
        }
        let pending = self.import_updates(updates)?;
        self.learn_version(session, peer_version)?;
        self.end_wait()?;
        if merges {
            self.reconcile_local()?;
        }
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
        let known = self
            .sessions
            .get_mut(&session)
            .and_then(|state| state.known.as_mut());
        if let Some(known) = known {
            *known = SyncDoc::merge_versions(known, version)?;
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
        self.local.store.persist(&self.local.doc)?;
        let doc = &self.local.doc;
        let version = doc.version();
        let mut outgoing = Vec::new();
        for (&session, state) in &mut self.sessions {
            let Some(known) = &state.known else {
                continue;
            };
            let Some(updates) = doc.updates_for(known)? else {
                continue;
            };
            state.known = Some(SyncDoc::merge_versions(known, &version)?);
            let message = Message::Updates {
                version: version.clone(),
                updates,
            };
            outgoing.push(Outgoing { session, message });
        }
        Ok(outgoing)
    }
}

fn peer_status(entry: PeerEntry, connected: &[DeviceId]) -> PeerStatus {
    let is_connected = from_hex32(&entry.id).is_some_and(|id| connected.contains(&id));
    PeerStatus {
        id: entry.id,
        connected: is_connected,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use sb_core::NotePath;

    use super::*;
    use crate::store::DocStore;

    struct Fixture {
        replica: Replica,
        _vault: tempfile::TempDir,
        state: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let vault = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let state_dir = StateDir::at(state.path()).unwrap();
        let replica =
            Replica::open(Vault::open(vault.path()).unwrap(), state_dir, [1; 32]).unwrap();
        Fixture {
            replica,
            _vault: vault,
            state,
        }
    }

    fn note_path() -> NotePath {
        NotePath::parse("x.md").unwrap()
    }

    fn durable_notes(fixture: &Fixture) -> usize {
        let state = StateDir::at(fixture.state.path()).unwrap();
        DocStore::open(&state).unwrap().1.alive_notes().len()
    }

    #[test]
    fn should_record_a_peer_that_dialled_in() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        fixture.replica.session_opened(2, [2; 32]);
        let state = StateDir::at(fixture.state.path()).unwrap();
        let recorded = PeerEntry {
            id: to_hex(&[2; 32]),
            addrs: Vec::new(),
        };
        assert_eq!(state.peers().unwrap(), [recorded]);
        let listed = PeerStatus {
            id: to_hex(&[2; 32]),
            connected: true,
        };
        assert_eq!(fixture.replica.status().peers, [listed]);
    }

    #[test]
    fn should_report_a_reset_base_in_the_status() {
        let (vault, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir(vault.path().join(".secondbrain")).unwrap();
        std::fs::write(state.path().join("base.json"), "{nope").unwrap();
        let state_dir = StateDir::at(state.path()).unwrap();
        let replica =
            Replica::open(Vault::open(vault.path()).unwrap(), state_dir, [1; 32]).unwrap();
        assert!(replica.status().last_error.unwrap().contains("base.json"));
    }

    #[test]
    fn should_send_nothing_to_a_session_before_its_version_arrives() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        std::fs::write(
            fixture._vault.path().join("x.md"),
            "x
",
        )
        .unwrap();
        assert!(fixture.replica.reconcile().unwrap().is_empty());
        let empty = SyncDoc::new().version();
        let reply = fixture
            .replica
            .on_message(1, Message::Version(empty))
            .unwrap();
        let [
            Outgoing {
                message: Message::Updates { updates, .. },
                ..
            },
        ] = &reply[..]
        else {
            panic!("expected one Updates, got {reply:?}");
        };
        let mut peer = SyncDoc::new();
        peer.import(updates).unwrap();
        assert_eq!(peer.alive_notes().len(), 1);
        assert!(fixture.replica.reconcile().unwrap().is_empty());
    }

    #[test]
    fn should_count_entries_a_peer_made_up_in_the_status() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        assert_eq!(fixture.replica.status().rejected_entries, 0);
        let peer = loro::LoroDoc::new();
        let paths = peer.get_map("paths");
        paths.insert("not-an-id", "a.md").unwrap();
        peer.commit();
        let updates = peer.export(loro::ExportMode::all_updates()).unwrap();
        let message = Message::Updates {
            version: SyncDoc::new().version(),
            updates,
        };
        fixture.replica.on_message(1, message).unwrap();
        assert_eq!(fixture.replica.status().rejected_entries, 1);
    }

    #[test]
    fn should_not_reconcile_for_an_empty_updates_message() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        std::fs::write(fixture._vault.path().join("x.md"), "x\n").unwrap();
        let empty = Message::Updates {
            version: SyncDoc::new().version(),
            updates: Vec::new(),
        };
        fixture.replica.on_message(1, empty).unwrap();
        assert!(fixture.replica.local().doc.alive_notes().is_empty());
        fixture.replica.reconcile().unwrap();
        assert_eq!(fixture.replica.local().doc.alive_notes().len(), 1);
    }

    #[test]
    fn should_count_notes_in_the_status_from_the_last_round() {
        let mut fixture = fixture();
        std::fs::write(fixture._vault.path().join("x.md"), "x\n").unwrap();
        fixture.replica.reconcile().unwrap();
        assert_eq!(fixture.replica.status().notes, 1);
        let doc = &mut fixture.replica.local.doc;
        doc.create_note(&NotePath::parse("y.md").unwrap(), "y\n")
            .unwrap();
        assert_eq!(fixture.replica.status().notes, 1);
    }

    #[test]
    fn should_show_the_relay_the_host_reported() {
        let mut fixture = fixture();
        assert_eq!(fixture.replica.status().relay, None);
        fixture
            .replica
            .set_relay(Some("https://relay.example./".into()));
        let status = fixture.replica.status();
        assert_eq!(status.relay.as_deref(), Some("https://relay.example./"));
    }

    #[test]
    fn should_not_record_this_device_as_a_peer() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [1; 32]);
        let state = StateDir::at(fixture.state.path()).unwrap();
        assert!(state.peers().unwrap().is_empty());
    }

    #[test]
    fn should_open_the_session_and_report_when_peers_json_cannot_be_written() {
        let mut fixture = fixture();
        std::fs::create_dir(fixture.state.path().join("peers.json")).unwrap();
        let outgoing = fixture.replica.session_opened(1, [2; 32]);
        assert_eq!(outgoing.len(), 2);
        assert!(fixture.replica.status().last_error.is_some());
    }

    #[test]
    fn should_persist_before_answering_a_version_request() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        let doc = &mut fixture.replica.local.doc;
        doc.create_note(&note_path(), "not yet durable\n").unwrap();
        let empty = SyncDoc::new().version();
        let reply = fixture.replica.on_message(1, Message::Version(empty));
        assert!(reply.is_ok());
        assert_eq!(durable_notes(&fixture), 1);
    }

    #[test]
    fn should_persist_before_sending_updates_for_a_session() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        let empty = SyncDoc::new().version();
        let reply = fixture.replica.on_message(1, Message::Version(empty));
        assert!(reply.is_ok());
        let doc = &mut fixture.replica.local.doc;
        doc.create_note(&note_path(), "not yet durable\n").unwrap();
        let outgoing = fixture.replica.updates_for_sessions().unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(durable_notes(&fixture), 1);
    }

    #[test]
    fn should_not_advance_the_known_version_when_updates_fail() {
        let mut fixture = fixture();
        fixture.replica.session_opened(1, [2; 32]);
        let mut peer = SyncDoc::new();
        peer.create_note(&note_path(), "peer\n").unwrap();
        let before = fixture.replica.sessions[&1].known.clone();
        let garbage = Message::Updates {
            version: peer.version(),
            updates: vec![1, 2, 3, 4],
        };
        assert!(fixture.replica.on_message(1, garbage).is_err());
        assert_eq!(fixture.replica.sessions[&1].known, before);
    }
}
