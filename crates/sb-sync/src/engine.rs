//! The sync engine: one actor thread that owns the replica, reconciles after file changes and
//! periodically, and moves messages between the replica and the network (spec section 8.9).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sb_core::{Vault, VaultWatcher, watch};
use tokio::sync::mpsc::UnboundedSender;

use crate::error::{Result, SyncError};
use crate::protocol::Message;
use crate::replica::{Outgoing, Replica, SessionId, Tick};
use crate::session::SessionEvent;
use crate::state::{DeviceId, EngineLock, PeerEntry, Secret, StateDir, Status, to_hex};
use crate::ticket::Ticket;

/// What the engine reports to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// A peer's session opened; the device id in lowercase hex.
    PeerConnected(String),
    /// A peer's session ended; the device id in lowercase hex.
    PeerDisconnected(String),
    /// A round changed the document or the disk.
    Synced,
    /// The number of blocked notes changed.
    Blocked(usize),
    /// A round, a tick or a session failed; the engine keeps running.
    Error(String),
    /// The engine thread is over and the lock is released.
    Stopped,
}

/// What the network adapter must do for the engine. Implemented by `net::IrohNetwork` (Task 10) and by a fake in tests.
pub trait Network: Send + 'static {
    /// This device's id.
    fn device_id(&self) -> DeviceId;
    /// The addresses this device listens on.
    fn addresses(&self) -> Vec<SocketAddr>;
    /// Called on every tick with the current secret, the known peers and the peers that already have a session.
    fn refresh(&mut self, secret: Secret, peers: &[PeerEntry], connected: &[DeviceId]);
    /// Ends the connection of one session.
    fn close_session(&mut self, session: SessionId);
    /// Stops listening and dialling.
    fn shutdown(&mut self);
}

/// What the actor's queue carries.
#[derive(Debug)]
pub enum Command {
    /// Something under the vault changed.
    VaultChanged,
    /// A session reports an event.
    Session {
        /// The session.
        session: SessionId,
        /// What happened.
        event: SessionEvent,
    },
    /// Asks for a ticket.
    Invite(Sender<Result<Ticket>>),
    /// Joins the group of a ticket.
    Join(Ticket, Sender<Result<()>>),
    /// Leaves the group.
    Leave(Sender<Result<()>>),
    /// Asks for the current status.
    Status(Sender<Status>),
    /// Stops the engine.
    Shutdown,
}

/// A burst of changes becomes one round this long after the last change.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// The period of ticks.
pub const TICK: Duration = Duration::from_secs(5);
/// The longest time without a round.
pub const RESCAN: Duration = Duration::from_secs(60);

/// A burst of changes never postpones a round by more than this, so a file that changes
/// continuously still gets one.
const MAX_POSTPONE: Duration = Duration::from_secs(5);
/// How long a call on the handle waits for the actor.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// A running engine. Dropping it asks the engine to stop without waiting; `shutdown` waits.
pub struct EngineHandle {
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Takes the engine lock, opens the replica, starts the watcher and the actor thread.
    /// `make_network` receives the sender the network uses to report session events.
    ///
    /// # Errors
    /// `AlreadyRunning`, and whatever the network, the vault or the replica report.
    pub fn start(
        vault_root: &Path,
        state: StateDir,
        events: Option<Sender<EngineEvent>>,
        make_network: impl FnOnce(Sender<Command>, &StateDir) -> Result<Box<dyn Network>>,
    ) -> Result<EngineHandle> {
        let lock = state.lock()?;
        let (commands, inbox) = mpsc::channel();
        let mut network = make_network(commands.clone(), &state)?;
        let id = network.device_id();
        let (replica, watcher) = match open_parts(vault_root, &state, id, &commands) {
            Ok(parts) => parts,
            Err(error) => {
                network.shutdown();
                return Err(error);
            }
        };
        let actor = Actor::new(replica, network, events, watcher);
        let thread = std::thread::Builder::new()
            .name("sb-sync-engine".into())
            .spawn(move || actor.run_until_stopped(lock, &inbox))
            .map_err(|error| SyncError::Net(format!("cannot start the engine thread: {error}")))?;
        Ok(EngineHandle {
            commands,
            thread: Some(thread),
        })
    }

    /// A ticket for this device and group.
    ///
    /// # Errors
    /// `NotConfigured`, or `Net` when the engine is gone.
    pub fn invite(&self) -> Result<String> {
        Ok(self.ask(Command::Invite)??.to_string())
    }

    /// Joins the group of a ticket.
    ///
    /// # Errors
    /// An invalid ticket, state failures, or `Net` when the engine is gone.
    pub fn join(&self, ticket: &str) -> Result<()> {
        let ticket: Ticket = ticket.parse()?;
        self.ask(|reply| Command::Join(ticket, reply))?
    }

    /// Leaves the group; the engine stops.
    ///
    /// # Errors
    /// State failures, or `Net` when the engine is gone.
    pub fn leave(&self) -> Result<()> {
        self.ask(Command::Leave)?
    }

    /// The current status.
    ///
    /// # Errors
    /// `Net` when the engine is gone.
    pub fn status(&self) -> Result<Status> {
        self.ask(Command::Status)
    }

    /// Whether the actor thread still runs.
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Stops the engine and waits until the thread has released the lock.
    ///
    /// # Errors
    /// `Net` when the thread panicked.
    pub fn shutdown(mut self) -> Result<()> {
        let _ = self.commands.send(Command::Shutdown);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| SyncError::Net("the engine thread panicked".into())),
            None => Ok(()),
        }
    }

    /// Sends a command and waits for its answer; never longer than `ANSWER_TIMEOUT`.
    fn ask<T>(&self, command: impl FnOnce(Sender<T>) -> Command) -> Result<T> {
        let (reply, answer) = mpsc::channel();
        let stopped = || SyncError::Net("engine stopped".into());
        self.commands.send(command(reply)).map_err(|_| stopped())?;
        answer
            .recv_timeout(ANSWER_TIMEOUT)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => {
                    SyncError::Net("the engine did not answer in time".into())
                }
                RecvTimeoutError::Disconnected => stopped(),
            })
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.commands.send(Command::Shutdown);
        }
    }
}

/// Opens the replica and makes sure the group secret and the device id are on disk, then starts the watcher.
fn open_parts(
    vault_root: &Path,
    state: &StateDir,
    device_id: DeviceId,
    commands: &Sender<Command>,
) -> Result<(Replica, VaultWatcher)> {
    let replica = Replica::open(Vault::open(vault_root)?, state.clone(), device_id)?;
    state.ensure_secret()?;
    state.save_device_id(&device_id)?;
    let changes = commands.clone();
    let watcher = watch(vault_root, move || {
        let _ = changes.send(Command::VaultChanged);
    })?;
    Ok((replica, watcher))
}

fn emit(events: Option<&Sender<EngineEvent>>, event: EngineEvent) {
    if let Some(events) = events {
        let _ = events.send(event);
    }
}

/// When the next round and the next tick are due.
struct Schedule {
    /// First and last change since the last round.
    changes: Option<(Instant, Instant)>,
    rescan_at: Instant,
    next_tick: Instant,
}

impl Schedule {
    fn new(now: Instant) -> Schedule {
        Schedule {
            changes: None,
            rescan_at: now + RESCAN,
            next_tick: now,
        }
    }

    fn changed(&mut self, now: Instant) {
        let first = self.changes.map_or(now, |(first, _)| first);
        self.changes = Some((first, now));
    }

    fn round_at(&self) -> Instant {
        let debounced = self
            .changes
            .map(|(first, last)| (last + DEBOUNCE).min(first + MAX_POSTPONE));
        debounced.map_or(self.rescan_at, |at| at.min(self.rescan_at))
    }

    fn deadline(&self) -> Instant {
        self.round_at().min(self.next_tick)
    }

    fn round_done(&mut self, now: Instant) {
        self.changes = None;
        self.rescan_at = now + RESCAN;
    }

    fn tick_done(&mut self, now: Instant) {
        self.next_tick = now + TICK;
    }
}

struct SessionLink {
    peer: DeviceId,
    outbound: UnboundedSender<Message>,
}

/// The state of the actor thread: the only owner of the replica and the network.
struct Actor {
    replica: Replica,
    network: Box<dyn Network>,
    events: Option<Sender<EngineEvent>>,
    sessions: BTreeMap<SessionId, SessionLink>,
    started: Instant,
    schedule: Schedule,
    blocked: usize,
    last_error: Option<String>,
    _watcher: VaultWatcher,
}

impl Actor {
    fn new(
        replica: Replica,
        network: Box<dyn Network>,
        events: Option<Sender<EngineEvent>>,
        watcher: VaultWatcher,
    ) -> Actor {
        let started = Instant::now();
        Actor {
            replica,
            network,
            events,
            sessions: BTreeMap::new(),
            started,
            schedule: Schedule::new(started),
            blocked: 0,
            last_error: None,
            _watcher: watcher,
        }
    }

    /// Runs the actor, then releases the lock and only then announces `Stopped`.
    fn run_until_stopped(self, lock: EngineLock, inbox: &Receiver<Command>) {
        let events = self.events.clone();
        self.run(inbox);
        drop(lock);
        emit(events.as_ref(), EngineEvent::Stopped);
    }

    fn run(mut self, inbox: &Receiver<Command>) {
        self.round();
        let mut running = self.tick();
        while running {
            running = self.step(inbox);
        }
        self.network.shutdown();
    }

    /// Does what is due, then waits for a command until the next deadline; `false` stops the actor.
    fn step(&mut self, inbox: &Receiver<Command>) -> bool {
        let now = Instant::now();
        if now >= self.schedule.round_at() {
            self.round();
        }
        if now >= self.schedule.next_tick && !self.tick() {
            return false;
        }
        let wait = self
            .schedule
            .deadline()
            .saturating_duration_since(Instant::now());
        match inbox.recv_timeout(wait) {
            Ok(command) => self.handle(command),
            Err(RecvTimeoutError::Timeout) => true,
            // Unreachable while the watcher holds a sender; kept so no other way out can hang.
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::VaultChanged => self.schedule.changed(Instant::now()),
            Command::Session { session, event } => self.on_session(session, event),
            Command::Invite(reply) => {
                let _ = reply.send(self.replica.invite());
            }
            Command::Status(reply) => {
                let _ = reply.send(self.status());
            }
            Command::Join(ticket, reply) => {
                let result = self.replica.join(&ticket, self.started.elapsed());
                return self.change_group(result, &reply);
            }
            Command::Leave(reply) => {
                let result = self.replica.leave();
                return self.change_group(result, &reply);
            }
            Command::Shutdown => return false,
        }
        true
    }

    /// Answers a join or leave and applies its tick; `false` when the replica left.
    fn change_group(&mut self, result: Result<Tick>, reply: &Sender<Result<()>>) -> bool {
        let (answer, left) = match result {
            Ok(tick) => (Ok(()), self.apply_tick(tick)),
            Err(error) => (Err(error), false),
        };
        let _ = reply.send(answer);
        if !left {
            self.refresh_network();
        }
        !left
    }

    fn round(&mut self) {
        self.schedule.round_done(Instant::now());
        match self.replica.reconcile() {
            Ok(outgoing) => {
                self.last_error = None;
                self.after_reconcile();
                self.send_all(outgoing);
            }
            Err(error) => self.report(&error),
        }
    }

    /// `false` when the replica left its group.
    fn tick(&mut self) -> bool {
        self.schedule.tick_done(Instant::now());
        self.replica.set_addresses(self.network.addresses());
        match self.replica.tick(self.started.elapsed()) {
            Ok(tick) => {
                if self.apply_tick(tick) {
                    return false;
                }
                self.publish_status();
            }
            Err(error) => self.report(&error),
        }
        self.refresh_network();
        true
    }

    /// `true` when the replica left its group.
    fn apply_tick(&mut self, tick: Tick) -> bool {
        for session in tick.close_sessions {
            self.close(session);
        }
        self.send_all(tick.outgoing);
        tick.left
    }

    /// Tells the host what the last reconciliation did.
    fn after_reconcile(&mut self) {
        if self.replica.is_waiting_for_first_state() {
            return;
        }
        let outcome = self.replica.last_outcome();
        let changed = outcome.doc_changed || outcome.disk_changed;
        let blocked = outcome.blocked.len();
        if blocked != self.blocked {
            self.blocked = blocked;
            self.emit(EngineEvent::Blocked(blocked));
        }
        if changed {
            self.emit(EngineEvent::Synced);
        }
    }

    fn on_session(&mut self, session: SessionId, event: SessionEvent) {
        match event {
            SessionEvent::Opened { peer, outbound } => self.open_session(session, peer, outbound),
            SessionEvent::Received(message) => self.receive(session, message),
            SessionEvent::Closed => self.forget(session),
        }
    }

    fn open_session(
        &mut self,
        session: SessionId,
        peer: DeviceId,
        outbound: UnboundedSender<Message>,
    ) {
        self.sessions
            .insert(session, SessionLink { peer, outbound });
        self.emit(EngineEvent::PeerConnected(to_hex(&peer)));
        let outgoing = self.replica.session_opened(session, peer);
        self.send_all(outgoing);
    }

    /// A refused message ends the session: only the next session's `Version` exchange fetches again what was dropped.
    fn receive(&mut self, session: SessionId, message: Message) {
        if !self.sessions.contains_key(&session) {
            return;
        }
        let is_updates = matches!(message, Message::Updates { .. });
        match self.replica.on_message(session, message) {
            Ok(outgoing) => {
                if is_updates {
                    self.after_reconcile();
                }
                self.send_all(outgoing);
            }
            Err(error) => {
                self.close(session);
                self.report(&error);
            }
        }
    }

    /// Sends each message on its session; a session that cannot take it is closed.
    fn send_all(&mut self, outgoing: Vec<Outgoing>) {
        for Outgoing { session, message } in outgoing {
            let link = self.sessions.get(&session);
            if !link.is_some_and(|link| link.outbound.send(message).is_ok()) {
                self.close(session);
            }
        }
    }

    fn close(&mut self, session: SessionId) {
        self.network.close_session(session);
        self.forget(session);
    }

    /// Drops the session's sender and tells the replica; a late `Closed` from the network is then ignored.
    fn forget(&mut self, session: SessionId) {
        let Some(link) = self.sessions.remove(&session) else {
            return;
        };
        self.replica.session_closed(session);
        self.emit(EngineEvent::PeerDisconnected(to_hex(&link.peer)));
    }

    fn refresh_network(&mut self) {
        let state = &self.replica.local().state;
        let (secret, peers) = match (state.secret(), state.peers()) {
            (Ok(Some(secret)), Ok(peers)) => (secret, peers),
            (Ok(None), _) => return,
            (Err(error), _) | (_, Err(error)) => return self.report(&error),
        };
        let connected = self.replica.connected_peers();
        self.network.refresh(secret, &peers, &connected);
    }

    /// The replica's status plus the engine's last error.
    fn status(&self) -> Status {
        let mut status = self.replica.status();
        status.last_error = self.last_error.clone().or(status.last_error);
        status
    }

    /// Overwrites the replica's `status.json` when the engine has an error to show.
    fn publish_status(&mut self) {
        if self.last_error.is_none() {
            return;
        }
        if let Err(error) = self.replica.local().state.write_status(&self.status()) {
            self.report(&error);
        }
    }

    // shortcut: one error slot, cleared by the next successful round; a failing tick stays
    // visible until then. Upgrade when a tick error must outlive successful rounds.
    fn report(&mut self, error: &SyncError) {
        let text = error.to_string();
        self.last_error = Some(text.clone());
        self.emit(EngineEvent::Error(text));
    }

    fn emit(&self, event: EngineEvent) {
        emit(self.events.as_ref(), event);
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn should_cap_the_postponement_of_a_round_while_changes_keep_coming() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        for step in 0..60 {
            schedule.changed(start + Duration::from_millis(step * 100));
        }
        assert_eq!(schedule.round_at(), start + MAX_POSTPONE);
    }

    #[test]
    fn should_wait_for_the_debounce_then_for_the_rescan() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        schedule.changed(start);
        assert_eq!(schedule.round_at(), start + DEBOUNCE);
        schedule.round_done(start + DEBOUNCE);
        assert_eq!(schedule.round_at(), start + DEBOUNCE + RESCAN);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;
    use crate::doc::SyncDoc;
    use crate::protocol::Message;
    use crate::state::to_hex;

    const LIMIT: Duration = Duration::from_secs(5);

    type Refresh = (Secret, Vec<PeerEntry>, Vec<DeviceId>);

    #[derive(Default)]
    struct Calls {
        refreshes: Vec<Refresh>,
        closed: Vec<SessionId>,
        shut_down: bool,
    }

    struct FakeNetwork {
        id: DeviceId,
        calls: Arc<Mutex<Calls>>,
    }

    impl Network for FakeNetwork {
        fn device_id(&self) -> DeviceId {
            self.id
        }
        fn addresses(&self) -> Vec<SocketAddr> {
            Vec::new()
        }
        fn refresh(&mut self, secret: Secret, peers: &[PeerEntry], connected: &[DeviceId]) {
            let entry = (secret, peers.to_vec(), connected.to_vec());
            self.calls.lock().unwrap().refreshes.push(entry);
        }
        fn close_session(&mut self, session: SessionId) {
            self.calls.lock().unwrap().closed.push(session);
        }
        fn shutdown(&mut self) {
            self.calls.lock().unwrap().shut_down = true;
        }
    }

    struct Rig {
        handle: EngineHandle,
        events: Receiver<EngineEvent>,
        commands: Sender<Command>,
        calls: Arc<Mutex<Calls>>,
        vault: tempfile::TempDir,
        state: StateDir,
        _state_dir: tempfile::TempDir,
    }

    fn start_in(
        vault: tempfile::TempDir,
        state_dir: tempfile::TempDir,
        id: DeviceId,
    ) -> Result<Rig> {
        let state = StateDir::at(state_dir.path()).unwrap();
        let (event_tx, events) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Calls::default()));
        let shared = Arc::clone(&calls);
        let handle =
            EngineHandle::start(vault.path(), state.clone(), Some(event_tx), move |tx, _| {
                command_tx.send(tx).unwrap();
                Ok(Box::new(FakeNetwork { id, calls: shared }))
            })?;
        Ok(Rig {
            handle,
            events,
            commands: command_rx.recv().unwrap(),
            calls,
            vault,
            state,
            _state_dir: state_dir,
        })
    }

    fn start_rig(id: DeviceId, notes: &[&str]) -> Rig {
        let vault = tempfile::tempdir().unwrap();
        for name in notes {
            std::fs::write(vault.path().join(name), "text\n").unwrap();
        }
        start_in(vault, tempfile::tempdir().unwrap(), id).unwrap()
    }

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        wait_up_to(what, LIMIT, condition);
    }

    fn wait_up_to(what: &str, limit: Duration, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    fn note_count(rig: &Rig) -> usize {
        rig.handle.status().unwrap().notes
    }

    fn expect_event(events: &Receiver<EngineEvent>, wanted: &EngineEvent) {
        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            match events.recv_timeout(Duration::from_millis(50)) {
                Ok(event) if &event == wanted => return,
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        panic!("event {wanted:?} never arrived");
    }

    fn open_session(rig: &Rig, session: SessionId, peer: DeviceId) -> UnboundedReceiver<Message> {
        let (outbound, receiver) = unbounded_channel();
        let event = SessionEvent::Opened { peer, outbound };
        rig.commands
            .send(Command::Session { session, event })
            .unwrap();
        receiver
    }

    #[test]
    fn should_reconcile_at_start_and_report_status() {
        let rig = start_rig([1; 32], &["a.md", "b.md"]);
        wait_until("two notes", || note_count(&rig) == 2);
        assert_eq!(rig.handle.status().unwrap().device_id, to_hex(&[1; 32]));
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_reconcile_after_a_file_change_without_being_told() {
        let rig = start_rig([1; 32], &["a.md", "b.md"]);
        wait_until("two notes", || note_count(&rig) == 2);
        std::fs::write(rig.vault.path().join("c.md"), "third\n").unwrap();
        wait_until("three notes", || note_count(&rig) == 3);
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_refuse_a_second_engine_for_the_same_vault() {
        let rig = start_rig([1; 32], &[]);
        let refused = EngineHandle::start(rig.vault.path(), rig.state.clone(), None, |_, _| {
            Err(SyncError::Net("the network must not be built".into()))
        });
        assert!(matches!(refused, Err(SyncError::AlreadyRunning)));
        let state = rig.state.clone();
        rig.handle.shutdown().unwrap();
        let again = EngineHandle::start(rig.vault.path(), state, None, |_, _| {
            Ok(Box::new(FakeNetwork {
                id: [1; 32],
                calls: Arc::default(),
            }))
        });
        again.unwrap().shutdown().unwrap();
    }

    #[test]
    fn should_sync_two_engines_through_injected_sessions() {
        let a = start_rig([1; 32], &[]);
        let b = start_rig([2; 32], &[]);
        let from_a = open_session(&a, 1, [2; 32]);
        let from_b = open_session(&b, 1, [1; 32]);
        pump(from_a, b.commands.clone());
        pump(from_b, a.commands.clone());
        std::fs::write(a.vault.path().join("from-a.md"), "hello from a\n").unwrap();
        let in_b = b.vault.path().join("from-a.md");
        wait_up_to("note in b", Duration::from_secs(10), || in_b.exists());
        std::fs::write(b.vault.path().join("from-b.md"), "hello from b\n").unwrap();
        let in_a = a.vault.path().join("from-b.md");
        wait_up_to("note in a", Duration::from_secs(10), || in_a.exists());
        assert_eq!(std::fs::read_to_string(in_b).unwrap(), "hello from a\n");
        a.handle.shutdown().unwrap();
        b.handle.shutdown().unwrap();
    }

    /// Hands every message the engine puts on a session to the other engine.
    fn pump(mut outbound: UnboundedReceiver<Message>, other: Sender<Command>) {
        std::thread::spawn(move || {
            while let Some(message) = outbound.blocking_recv() {
                let event = SessionEvent::Received(message);
                if other.send(Command::Session { session: 1, event }).is_err() {
                    break;
                }
            }
        });
    }

    #[test]
    fn should_emit_events_for_peers_and_sync() {
        let rig = start_rig([1; 32], &["a.md"]);
        expect_event(&rig.events, &EngineEvent::Synced);
        let _outbound = open_session(&rig, 7, [9; 32]);
        expect_event(&rig.events, &EngineEvent::PeerConnected(to_hex(&[9; 32])));
        rig.commands
            .send(Command::Session {
                session: 7,
                event: SessionEvent::Closed,
            })
            .unwrap();
        expect_event(
            &rig.events,
            &EngineEvent::PeerDisconnected(to_hex(&[9; 32])),
        );
        let events = rig.events;
        rig.handle.shutdown().unwrap();
        expect_event(&events, &EngineEvent::Stopped);
    }

    #[test]
    fn should_hand_secret_and_peers_to_the_network_on_tick() {
        let rig = start_rig([1; 32], &[]);
        let ticket = Ticket {
            device_id: [5; 32],
            secret: [6; 32],
            addrs: Vec::new(),
        };
        rig.handle.join(&ticket.to_string()).unwrap();
        wait_until("a refresh with the new group", || {
            let calls = rig.calls.lock().unwrap();
            calls.refreshes.iter().any(|(secret, peers, _)| {
                *secret == [6; 32] && peers.iter().any(|peer| peer.id == to_hex(&[5; 32]))
            })
        });
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_stop_when_the_replica_leaves() {
        let rig = start_rig([1; 32], &[]);
        rig.handle.leave().unwrap();
        expect_event(&rig.events, &EngineEvent::Stopped);
        wait_until("the thread to end", || !rig.handle.is_running());
        assert!(rig.state.lock().is_ok());
        assert!(rig.calls.lock().unwrap().shut_down);
        assert!(rig.handle.status().is_err());
    }

    #[test]
    fn should_create_secret_and_marker_directory_on_first_start() {
        let rig = start_rig([1; 32], &[]);
        assert!(rig.state.secret().unwrap().is_some());
        assert!(rig.vault.path().join(".secondbrain").is_dir());
        assert_eq!(rig.state.device_id().unwrap(), Some([1; 32]));
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_close_the_session_when_a_message_is_refused() {
        let rig = start_rig([1; 32], &[]);
        let _outbound = open_session(&rig, 5, [9; 32]);
        let garbage = Message::Updates {
            version: SyncDoc::new().version(),
            updates: vec![1, 2, 3, 4],
        };
        let event = SessionEvent::Received(garbage);
        rig.commands
            .send(Command::Session { session: 5, event })
            .unwrap();
        wait_until("the session to be closed", || {
            rig.calls.lock().unwrap().closed.contains(&5)
        });
        expect_event(
            &rig.events,
            &EngineEvent::PeerDisconnected(to_hex(&[9; 32])),
        );
        assert!(rig.handle.status().is_ok());
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_report_a_missing_marker_and_recover_at_the_next_round() {
        let rig = start_rig([1; 32], &["a.md"]);
        wait_until("one note", || note_count(&rig) == 1);
        std::fs::remove_dir_all(rig.vault.path().join(".secondbrain")).unwrap();
        std::fs::write(rig.vault.path().join("b.md"), "b\n").unwrap();
        wait_until("the error in the status", || {
            rig.handle.status().unwrap().last_error.is_some()
        });
        std::fs::create_dir(rig.vault.path().join(".secondbrain")).unwrap();
        std::fs::write(rig.vault.path().join("c.md"), "c\n").unwrap();
        wait_until("three notes", || note_count(&rig) == 3);
        assert!(rig.handle.status().unwrap().last_error.is_none());
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_stop_when_the_handle_is_dropped() {
        let rig = start_rig([1; 32], &[]);
        let events = rig.events;
        drop(rig.handle);
        expect_event(&events, &EngineEvent::Stopped);
    }
}
