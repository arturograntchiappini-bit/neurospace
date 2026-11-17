//! The sync engine: one actor thread that owns the replica, reconciles after file changes and
//! periodically, and moves messages between the replica and the network (spec section 8.9).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sb_core::{Vault, VaultWatcher, watch};
use tokio::sync::mpsc::UnboundedSender;

use crate::auth::{GroupTag, group_tag};
use crate::error::{Result, SyncError};
use crate::protocol::{FRAME_MAX, Message};
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
    /// When the secret differs from the previous call, abort every open session and every dial in flight.
    /// Session ids are never reused. All events of one session carry the same group tag.
    fn refresh(&mut self, secret: Secret, peers: &[PeerEntry], connected: &[DeviceId]);
    /// Ends the connection of one session.
    fn close_session(&mut self, session: SessionId);
    /// Stops listening and dialling; returns within a few seconds. An implementation also stops in `Drop`
    /// when `shutdown` was never called.
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
        /// The network stamps each event with the tag of the secret the session authenticated under.
        group: GroupTag,
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

const OVERSIZE: &str = "a message for a peer exceeds the 1 GiB frame limit and was not sent; the sync history is too large";

/// A burst of changes never postpones a round by more than this, so a file that changes
/// continuously still gets one.
const MAX_POSTPONE: Duration = Duration::from_secs(5);
/// How long a call on the handle waits for the actor.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// A running engine. Dropping it asks the engine to stop without waiting, so a `start` right after a drop may
/// get `AlreadyRunning`; `shutdown` waits until the lock is released.
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
        let folder = Folder {
            root: vault_root,
            make_watcher: watch_vault,
        };
        Self::start_watching(&folder, state, events, make_network)
    }

    /// `start` with the way the folder is watched injected, so a test can make watching fail.
    fn start_watching(
        folder: &Folder<'_>,
        state: StateDir,
        events: Option<Sender<EngineEvent>>,
        make_network: impl FnOnce(Sender<Command>, &StateDir) -> Result<Box<dyn Network>>,
    ) -> Result<EngineHandle> {
        let vault_root = folder.root;
        let lock = state.lock()?;
        let (commands, inbox) = mpsc::channel();
        let mut network = make_network(commands.clone(), &state)?;
        let id = network.device_id();
        let replica = match open_replica(vault_root, &state, id) {
            Ok(replica) => replica,
            Err(error) => {
                network.shutdown();
                return Err(error);
            }
        };
        let watching = Watching::start(vault_root, commands.clone(), folder.make_watcher);
        let actor = Actor::new(replica, network, events, watching);
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

    /// Joins the group of a ticket. After a timeout the command may still be carried out later.
    ///
    /// # Errors
    /// An invalid ticket, state failures, or `Net` when the engine is gone or does not answer within 10 s.
    pub fn join(&self, ticket: &str) -> Result<()> {
        let ticket: Ticket = ticket.parse()?;
        self.ask(|reply| Command::Join(ticket, reply))?
    }

    /// Leaves the group; the engine stops. After a timeout the command may still be carried out later.
    ///
    /// # Errors
    /// State failures, or `Net` when the engine is gone or does not answer within 10 s.
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

/// Opens the replica and makes sure the group secret and the device id are on disk.
fn open_replica(vault_root: &Path, state: &StateDir, device_id: DeviceId) -> Result<Replica> {
    let replica = Replica::open(Vault::open(vault_root)?, state.clone(), device_id)?;
    state.ensure_secret()?;
    state.save_device_id(&device_id)?;
    Ok(replica)
}

type WatchFn = fn(&Path, &Sender<Command>) -> Result<VaultWatcher>;

/// The vault folder and the way it is watched.
struct Folder<'a> {
    root: &'a Path,
    make_watcher: WatchFn,
}

fn watch_vault(root: &Path, commands: &Sender<Command>) -> Result<VaultWatcher> {
    let changes = commands.clone();
    Ok(watch(root, move || {
        let _ = changes.send(Command::VaultChanged);
    })?)
}

/// The folder watcher and what is needed to make a new one. Without a watcher the engine still
/// rescans every `RESCAN`.
struct Watching {
    root: PathBuf,
    commands: Sender<Command>,
    make: WatchFn,
    current: Option<VaultWatcher>,
    /// Said once, when the engine starts without a watcher.
    notice: Option<String>,
}

impl Watching {
    fn start(root: &Path, commands: Sender<Command>, make: WatchFn) -> Watching {
        let current = make(root, &commands);
        let notice = current
            .as_ref()
            .err()
            .map(|error| format!("file watching is off, rescanning every {RESCAN:?}: {error}"));
        Watching {
            root: root.to_path_buf(),
            commands,
            make,
            current: current.ok(),
            notice,
        }
    }

    /// Replaces the watcher: after a drive came back, the old one may watch nothing.
    fn restart(&mut self) -> Result<()> {
        self.current = Some((self.make)(&self.root, &self.commands)?);
        Ok(())
    }
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

/// Releases the engine lock and only then announces `Stopped`.
struct StopGuard {
    lock: Option<EngineLock>,
    events: Option<Sender<EngineEvent>>,
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        drop(self.lock.take());
        emit(self.events.as_ref(), EngineEvent::Stopped);
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
    /// The tag of the current group secret; sessions authenticated under another one are refused.
    group: Option<GroupTag>,
    watching: Watching,
    /// The vault lost its marker folder; the watcher is replaced after the next good round.
    rewatch: bool,
    frame_limit: usize,
}

impl Actor {
    fn new(
        replica: Replica,
        network: Box<dyn Network>,
        events: Option<Sender<EngineEvent>>,
        watching: Watching,
    ) -> Actor {
        let started = Instant::now();
        let mut actor = Actor {
            replica,
            network,
            events,
            sessions: BTreeMap::new(),
            started,
            schedule: Schedule::new(started),
            blocked: 0,
            last_error: None,
            group: None,
            watching,
            rewatch: false,
            frame_limit: FRAME_MAX,
        };
        actor.group = actor.read_group();
        actor
    }

    /// The tag of the secret on disk; `None` (and an error report) when it cannot be read, so no session is accepted.
    fn read_group(&mut self) -> Option<GroupTag> {
        match self.replica.local().state.secret() {
            Ok(secret) => secret.map(|secret| group_tag(&secret)),
            Err(error) => {
                self.report(&error);
                None
            }
        }
    }

    /// Runs the actor; the guard then releases the lock and announces `Stopped`, also after a panic.
    fn run_until_stopped(self, lock: EngineLock, inbox: &Receiver<Command>) {
        let _guard = StopGuard {
            lock: Some(lock),
            events: self.events.clone(),
        };
        self.run(inbox);
    }

    fn run(mut self, inbox: &Receiver<Command>) {
        if let Some(notice) = self.watching.notice.take() {
            self.report(&SyncError::Net(notice));
        }
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
            Command::Session {
                session,
                group,
                event,
            } => self.on_session(session, group, event),
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
                self.replace_watcher_after_outage();
                self.after_reconcile();
                self.send_all(outgoing);
            }
            Err(error) => {
                self.rewatch |= matches!(error, SyncError::VaultMarkerMissing(_));
                self.report(&error);
            }
        }
    }

    /// An unmounted vault takes its watches with it; the first good round after it makes a new watcher.
    fn replace_watcher_after_outage(&mut self) {
        if !std::mem::take(&mut self.rewatch) {
            return;
        }
        if let Err(error) = self.watching.restart() {
            self.report(&error);
        }
    }

    /// `false` when the replica left its group.
    fn tick(&mut self) -> bool {
        self.schedule.tick_done(Instant::now());
        self.replica.set_addresses(self.network.addresses());
        let was_waiting = self.replica.is_waiting_for_first_state();
        match self.replica.tick(self.started.elapsed()) {
            Ok(tick) => {
                if self.apply_tick(tick) {
                    return false;
                }
                if was_waiting && !self.replica.is_waiting_for_first_state() {
                    self.after_reconcile();
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
        if tick.secret_changed {
            self.group = self.read_group();
        }
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

    /// A session authenticated under another secret is closed, never shown to the replica.
    fn on_session(&mut self, session: SessionId, group: GroupTag, event: SessionEvent) {
        if self.group != Some(group) {
            if matches!(event, SessionEvent::Opened { .. }) {
                self.network.close_session(session);
            }
            return;
        }
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

    /// Sends each message on its session; a session that cannot take it is closed. A message over the
    /// frame limit is never queued: the peer could not read it, so it is reported instead.
    fn send_all(&mut self, outgoing: Vec<Outgoing>) {
        for Outgoing { session, message } in outgoing {
            if message.encoded_len() > self.frame_limit {
                self.report(&SyncError::Net(OVERSIZE.into()));
                continue;
            }
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
        // The same secret the network authenticates with, so a failed read at one tick heals at the next.
        self.group = Some(group_tag(&secret));
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
    use crate::state::PendingCommand;
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
        start_in_watching(vault, state_dir, id, watch_vault)
    }

    fn start_in_watching(
        vault: tempfile::TempDir,
        state_dir: tempfile::TempDir,
        id: DeviceId,
        make_watcher: WatchFn,
    ) -> Result<Rig> {
        let state = StateDir::at(state_dir.path()).unwrap();
        let (event_tx, events) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Calls::default()));
        let shared = Arc::clone(&calls);
        let make_network = move |tx, _: &StateDir| -> Result<Box<dyn Network>> {
            command_tx.send(tx).unwrap();
            Ok(Box::new(FakeNetwork { id, calls: shared }))
        };
        let folder = Folder {
            root: vault.path(),
            make_watcher,
        };
        let handle =
            EngineHandle::start_watching(&folder, state.clone(), Some(event_tx), make_network)?;
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

    fn current_tag(rig: &Rig) -> GroupTag {
        group_tag(&rig.state.secret().unwrap().unwrap())
    }

    fn send_session_event(rig: &Rig, session: SessionId, group: GroupTag, event: SessionEvent) {
        let command = Command::Session {
            session,
            group,
            event,
        };
        rig.commands.send(command).unwrap();
    }

    fn open_session(rig: &Rig, session: SessionId, peer: DeviceId) -> UnboundedReceiver<Message> {
        open_session_in_group(rig, session, peer, current_tag(rig))
    }

    fn open_session_in_group(
        rig: &Rig,
        session: SessionId,
        peer: DeviceId,
        group: GroupTag,
    ) -> UnboundedReceiver<Message> {
        let (outbound, receiver) = unbounded_channel();
        send_session_event(rig, session, group, SessionEvent::Opened { peer, outbound });
        receiver
    }

    /// Asserts that no event matching `unwanted` arrives within `window`.
    fn expect_no_event(
        events: &Receiver<EngineEvent>,
        window: Duration,
        unwanted: impl Fn(&EngineEvent) -> bool,
    ) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
                assert!(!unwanted(&event), "unexpected event {event:?}");
            }
        }
    }

    fn garbage_updates() -> Message {
        Message::Updates {
            version: SyncDoc::new().version(),
            updates: vec![1, 2, 3, 4],
        }
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
        pump(from_a, b.commands.clone(), current_tag(&b));
        pump(from_b, a.commands.clone(), current_tag(&a));
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
    fn pump(mut outbound: UnboundedReceiver<Message>, other: Sender<Command>, group: GroupTag) {
        std::thread::spawn(move || {
            while let Some(message) = outbound.blocking_recv() {
                let event = SessionEvent::Received(message);
                let command = Command::Session {
                    session: 1,
                    group,
                    event,
                };
                if other.send(command).is_err() {
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
        send_session_event(&rig, 7, current_tag(&rig), SessionEvent::Closed);
        expect_event(
            &rig.events,
            &EngineEvent::PeerDisconnected(to_hex(&[9; 32])),
        );
        let events = rig.events;
        rig.handle.shutdown().unwrap();
        expect_event(&events, &EngineEvent::Stopped);
    }

    #[test]
    fn should_hand_secret_and_peers_to_the_network_after_a_join() {
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
        let event = SessionEvent::Received(garbage_updates());
        send_session_event(&rig, 5, current_tag(&rig), event);
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

    fn no_watcher(_: &Path, _: &Sender<Command>) -> Result<VaultWatcher> {
        Err(SyncError::Net("too many open files".into()))
    }

    #[test]
    fn should_start_without_a_watcher_and_say_so() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::write(vault.path().join("a.md"), "text\n").unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let rig = start_in_watching(vault, state_dir, [1; 32], no_watcher).unwrap();
        let mut said = false;
        let deadline = Instant::now() + LIMIT;
        while !said && Instant::now() < deadline {
            let event = rig.events.recv_timeout(Duration::from_millis(50));
            said = matches!(&event, Ok(EngineEvent::Error(text)) if text.contains("file watching is off"));
        }
        assert!(said, "no notice about the missing watcher");
        wait_until("the first round", || note_count(&rig) == 1);
        rig.handle.shutdown().unwrap();
    }

    static WATCHERS_MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counting_watcher(root: &Path, commands: &Sender<Command>) -> Result<VaultWatcher> {
        WATCHERS_MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        watch_vault(root, commands)
    }

    #[test]
    fn should_make_a_new_watcher_after_the_vault_came_back() {
        let vault = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let rig = start_in_watching(vault, state_dir, [1; 32], counting_watcher).unwrap();
        wait_until("the first round", || {
            rig.handle.status().unwrap().last_error.is_none()
        });
        let made = || WATCHERS_MADE.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(made(), 1);
        std::fs::remove_dir_all(rig.vault.path().join(".secondbrain")).unwrap();
        std::fs::write(rig.vault.path().join("b.md"), "b\n").unwrap();
        wait_until("the error in the status", || {
            rig.handle.status().unwrap().last_error.is_some()
        });
        std::fs::create_dir(rig.vault.path().join(".secondbrain")).unwrap();
        std::fs::write(rig.vault.path().join("c.md"), "c\n").unwrap();
        wait_until("both notes", || note_count(&rig) == 2);
        assert_eq!(made(), 2);
        rig.handle.shutdown().unwrap();
    }

    struct Bare {
        actor: Actor,
        state: StateDir,
        _dirs: (tempfile::TempDir, tempfile::TempDir),
    }

    /// An actor that was never run, with a replica whose group secret was not created yet.
    fn bare_actor() -> Bare {
        let (vault, state_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let state = StateDir::at(state_dir.path()).unwrap();
        let opened = Vault::open(vault.path()).unwrap();
        let replica = Replica::open(opened, state.clone(), [1; 32]).unwrap();
        let (commands, _inbox) = mpsc::channel();
        let watching = Watching::start(vault.path(), commands, watch_vault);
        let network = FakeNetwork {
            id: [1; 32],
            calls: Arc::default(),
        };
        let actor = Actor::new(replica, Box::new(network), None, watching);
        Bare {
            actor,
            state,
            _dirs: (vault, state_dir),
        }
    }

    fn link(actor: &mut Actor, session: SessionId) -> UnboundedReceiver<Message> {
        let (outbound, receiver) = unbounded_channel();
        let peer = [9; 32];
        actor
            .sessions
            .insert(session, SessionLink { peer, outbound });
        receiver
    }

    #[test]
    fn should_not_queue_a_message_over_the_frame_limit_and_say_so() {
        let mut bare = bare_actor();
        let mut receiver = link(&mut bare.actor, 1);
        bare.actor.frame_limit = 8;
        let big = Message::Version(vec![0; 100]);
        let outgoing = Outgoing {
            session: 1,
            message: big.clone(),
        };
        bare.actor.send_all(vec![outgoing]);
        assert!(receiver.try_recv().is_err());
        let said = bare.actor.last_error.clone().unwrap();
        assert!(said.contains("frame limit"), "{said}");
        assert!(bare.actor.sessions.contains_key(&1));
        bare.actor.frame_limit = 1000;
        let outgoing = Outgoing {
            session: 1,
            message: big.clone(),
        };
        bare.actor.send_all(vec![outgoing]);
        assert_eq!(receiver.try_recv().unwrap(), big);
    }

    #[test]
    fn should_accept_sessions_once_the_secret_becomes_readable() {
        let mut bare = bare_actor();
        assert_eq!(bare.actor.group, None);
        bare.state.save_secret(&[6; 32]).unwrap();
        let tag = group_tag(&[6; 32]);
        let (outbound, _receiver) = unbounded_channel();
        let opened = SessionEvent::Opened {
            peer: [9; 32],
            outbound,
        };
        bare.actor.on_session(1, tag, opened);
        assert!(!bare.actor.sessions.contains_key(&1));
        bare.actor.refresh_network();
        assert_eq!(bare.actor.group, Some(tag));
        let (outbound, _receiver) = unbounded_channel();
        let opened = SessionEvent::Opened {
            peer: [9; 32],
            outbound,
        };
        bare.actor.on_session(2, tag, opened);
        assert!(bare.actor.sessions.contains_key(&2));
    }

    #[test]
    fn should_stop_when_the_handle_is_dropped() {
        let rig = start_rig([1; 32], &[]);
        let events = rig.events;
        drop(rig.handle);
        expect_event(&events, &EngineEvent::Stopped);
    }

    fn other_group_ticket() -> String {
        let ticket = Ticket {
            device_id: [5; 32],
            secret: [6; 32],
            addrs: Vec::new(),
        };
        ticket.to_string()
    }

    #[test]
    fn should_refuse_a_session_of_the_old_group_after_a_join() {
        let rig = start_rig([1; 32], &[]);
        let old = current_tag(&rig);
        rig.handle.join(&other_group_ticket()).unwrap();
        let _old = open_session_in_group(&rig, 5, [9; 32], old);
        wait_until("the old session to be closed", || {
            rig.calls.lock().unwrap().closed.contains(&5)
        });
        let connected = |event: &EngineEvent| matches!(event, EngineEvent::PeerConnected(_));
        expect_no_event(&rig.events, Duration::from_secs(1), connected);
        let status = rig.handle.status().unwrap();
        assert!(status.peers.iter().all(|peer| !peer.connected));
        let _new = open_session_in_group(&rig, 6, [9; 32], current_tag(&rig));
        expect_event(&rig.events, &EngineEvent::PeerConnected(to_hex(&[9; 32])));
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_ignore_messages_stamped_with_another_group() {
        let rig = start_rig([1; 32], &[]);
        let _outbound = open_session(&rig, 5, [9; 32]);
        expect_event(&rig.events, &EngineEvent::PeerConnected(to_hex(&[9; 32])));
        let event = SessionEvent::Received(garbage_updates());
        send_session_event(&rig, 5, [0xAA; 8], event);
        let error = |event: &EngineEvent| matches!(event, EngineEvent::Error(_));
        expect_no_event(&rig.events, Duration::from_secs(1), error);
        assert!(!rig.calls.lock().unwrap().closed.contains(&5));
        rig.handle.shutdown().unwrap();
    }

    #[test]
    fn should_free_the_lock_when_the_network_cannot_be_built() {
        let vault = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let state = StateDir::at(state_dir.path()).unwrap();
        let failed = EngineHandle::start(vault.path(), state.clone(), None, |_, _| {
            Err(SyncError::Net("no network".into()))
        });
        assert!(matches!(failed, Err(SyncError::Net(_))));
        assert!(state.lock().is_ok());
    }

    #[test]
    fn should_free_the_lock_and_stop_the_network_when_the_vault_cannot_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_folder = dir.path().join("file.txt");
        std::fs::write(&not_a_folder, "x").unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let state = StateDir::at(state_dir.path()).unwrap();
        let calls = Arc::new(Mutex::new(Calls::default()));
        let shared = Arc::clone(&calls);
        let failed = EngineHandle::start(&not_a_folder, state.clone(), None, move |_, _| {
            Ok(Box::new(FakeNetwork {
                id: [1; 32],
                calls: shared,
            }))
        });
        assert!(failed.is_err());
        assert!(state.lock().is_ok());
        assert!(calls.lock().unwrap().shut_down);
    }

    #[test]
    fn should_stop_when_a_leave_arrives_through_command_pending() {
        let rig = start_rig([1; 32], &[]);
        let command = PendingCommand::Leave(true);
        rig.state.write_pending(&command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stopped = false;
        while !stopped && Instant::now() < deadline {
            let event = rig.events.recv_timeout(Duration::from_millis(50));
            stopped = event == Ok(EngineEvent::Stopped);
        }
        assert!(stopped, "Stopped never arrived");
        assert!(rig.state.lock().is_ok());
    }
}
