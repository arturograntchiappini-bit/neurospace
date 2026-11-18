//! The sync window's model and the background worker that serves the sync engine (spec 9.5).
//! Nothing here draws and nothing here makes its caller wait.
//!
//! A ticket carries the group secret: it appears only in `SyncPanel::ticket` and in the
//! messages that carry it, and every `Debug` impl below prints `<redacted>` in its place.

use std::fmt;
use std::mem;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use sb_sync::{NetMode, Status};

use crate::state::panels::counted;

const WINDOW_REFRESH: Duration = Duration::from_secs(2);
const IDLE_REFRESH: Duration = Duration::from_secs(10);
const STOP_POLL: Duration = Duration::from_millis(20);

/// What `SyncControl::start` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    /// This process runs the engine now.
    Started,
    /// Another process holds the engine lock of the vault.
    HandledElsewhere,
}

/// What a status refresh found out about the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineStatus {
    /// The engine's status, or the status file of the process that runs it.
    Running(Status),
    /// Nothing new: the engine did not answer, or no fresh status file exists.
    Unknown,
    /// The engine this process started has ended without being asked to. `configured` says
    /// whether the vault still has a sync group.
    Gone {
        /// Whether a group secret still exists.
        configured: bool,
    },
}

/// What the sync window needs from the engine. Every call may block, so the window only
/// reaches an implementation through a `SyncWorker`.
pub trait SyncControl: Send {
    /// Whether the vault has a sync group (a group secret exists).
    fn configured(&self) -> bool;
    /// Starts the engine; `HandledElsewhere` when another process already runs it.
    fn start(&mut self) -> Result<StartOutcome, String>;
    /// A ticket for this device and group; starts the engine if needed.
    fn invite(&mut self) -> Result<String, String>;
    /// Joins the group of `ticket`; starts the engine if needed.
    fn join(&mut self, ticket: &str) -> Result<(), String>;
    /// Leaves the group, then stops the engine.
    fn leave(&mut self) -> Result<(), String>;
    /// What is known about the engine now; the status file when another process runs it.
    fn status(&mut self) -> EngineStatus;
    /// Saves the network mode and restarts the engine if it runs. `HandledElsewhere` when the
    /// restart lost the lock to another process; `Started` otherwise, also when nothing ran.
    fn set_mode(&mut self, mode: NetMode) -> Result<StartOutcome, String>;
    /// Stops the engine and waits until it released the vault.
    fn stop(&mut self);
    /// The saved network mode.
    fn mode(&self) -> NetMode;
}

/// Something the window asks the worker to do.
pub enum Request {
    /// Try to start the engine again (the lock decides who runs it).
    Start,
    /// Create a ticket.
    Invite,
    /// Join the group of this ticket.
    Join(String),
    /// Leave the group and stop the engine.
    Leave,
    /// Switch the network mode.
    SetMode(NetMode),
    /// Read the status again.
    RefreshStatus,
    /// Stop the engine and end the worker.
    Stop,
}

impl fmt::Debug for Request {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Start => out.write_str("Start"),
            Request::Invite => out.write_str("Invite"),
            Request::Join(_) => out.write_str("Join(<redacted>)"),
            Request::Leave => out.write_str("Leave"),
            Request::SetMode(mode) => out.debug_tuple("SetMode").field(mode).finish(),
            Request::RefreshStatus => out.write_str("RefreshStatus"),
            Request::Stop => out.write_str("Stop"),
        }
    }
}

/// The worker's answer to a request, or its first word after it started.
pub enum Reply {
    /// The vault has no sync group, so no engine was started.
    NotConfigured,
    /// The engine was started, or someone else runs it.
    Started(Result<StartOutcome, String>),
    /// The ticket, or why there is none.
    Invited(Result<String, String>),
    /// Whether joining worked.
    Joined(Result<(), String>),
    /// Whether leaving worked.
    Left(Result<(), String>),
    /// Whether the mode was saved and the engine restarted, and who runs the engine now.
    ModeSet(Result<StartOutcome, String>),
    /// What a refresh found out.
    Status(EngineStatus),
    /// The worker ended.
    Stopped,
}

impl fmt::Debug for Reply {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reply::NotConfigured => out.write_str("NotConfigured"),
            Reply::Started(result) => out.debug_tuple("Started").field(result).finish(),
            Reply::Invited(Ok(_)) => out.write_str("Invited(Ok(<redacted>))"),
            Reply::Invited(Err(error)) => write!(out, "Invited(Err({error:?}))"),
            Reply::Joined(result) => out.debug_tuple("Joined").field(result).finish(),
            Reply::Left(result) => out.debug_tuple("Left").field(result).finish(),
            Reply::ModeSet(result) => out.debug_tuple("ModeSet").field(result).finish(),
            Reply::Status(status) => out.debug_tuple("Status").field(status).finish(),
            Reply::Stopped => out.write_str("Stopped"),
        }
    }
}

/// A `SyncControl` on its own thread, served through channels.
pub struct SyncWorker {
    requests: Sender<Request>,
    replies: Receiver<Reply>,
    thread: JoinHandle<()>,
    mode: NetMode,
}

impl SyncWorker {
    /// Runs `control` on a new thread. The thread first starts the engine when the vault has a
    /// sync group (replying `Started`) and otherwise starts nothing (replying `NotConfigured`,
    /// spec 9.5); then it serves requests one at a time. `wake` is called after every reply.
    pub fn spawn(control: Box<dyn SyncControl>, wake: impl Fn() + Send + 'static) -> SyncWorker {
        let mode = control.mode();
        let (requests, inbox) = mpsc::channel();
        let (outbox, replies) = mpsc::channel();
        let thread = thread::spawn(move || run(control, &inbox, &outbox, &wake));
        SyncWorker {
            requests,
            replies,
            thread,
            mode,
        }
    }

    /// The network mode saved when the worker was created.
    pub fn mode(&self) -> NetMode {
        self.mode
    }

    /// Queues `request`; never waits.
    pub fn send(&self, request: Request) {
        // A send fails only when the worker has ended: nobody is left to serve it.
        let _ = self.requests.send(request);
    }

    /// The replies that arrived since the last call.
    pub fn poll(&self) -> Vec<Reply> {
        self.replies.try_iter().collect()
    }

    /// Asks the worker to stop its engine and waits up to `timeout` for the thread to end.
    /// False when it did not end in time (a call to the engine is still running).
    pub fn stop_and_join(self, timeout: Duration) -> bool {
        self.send(Request::Stop);
        let deadline = Instant::now() + timeout;
        while !self.thread.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(STOP_POLL);
        }
        self.thread.join().is_ok()
    }
}

fn run(
    mut control: Box<dyn SyncControl>,
    inbox: &Receiver<Request>,
    outbox: &Sender<Reply>,
    wake: &impl Fn(),
) {
    let send = |reply| {
        // A send fails only when the window side is gone.
        let _ = outbox.send(reply);
        wake();
    };
    send(first_reply(control.as_mut()));
    // Also ends when the `SyncWorker` is dropped: the engine never outlives its owner.
    while let Ok(request) = inbox.recv() {
        let Some(reply) = answer(control.as_mut(), request) else {
            break;
        };
        send(reply);
    }
    control.stop();
    send(Reply::Stopped);
}

fn first_reply(control: &mut dyn SyncControl) -> Reply {
    if control.configured() {
        Reply::Started(control.start())
    } else {
        Reply::NotConfigured
    }
}

fn answer(control: &mut dyn SyncControl, request: Request) -> Option<Reply> {
    Some(match request {
        // Same rule as the first start: never start an engine for a vault without a group.
        Request::Start => first_reply(control),
        Request::Invite => Reply::Invited(control.invite()),
        Request::Join(ticket) => Reply::Joined(control.join(&ticket)),
        Request::Leave => Reply::Left(control.leave()),
        Request::SetMode(mode) => Reply::ModeSet(control.set_mode(mode)),
        Request::RefreshStatus => Reply::Status(control.status()),
        Request::Stop => return None,
    })
}

/// Where the sync engine of the open vault stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// No sync group, or the group was left.
    Off,
    /// The worker has not answered yet.
    Starting,
    /// This process runs the engine.
    On,
    /// Another process runs the engine.
    HandledElsewhere,
    /// Starting the engine failed; the text says why.
    Error(String),
}

/// What the sync window shows and edits.
pub struct SyncPanel {
    /// Where the engine stands.
    pub state: SyncState,
    /// What a pending request is doing, while one is.
    pub busy: Option<&'static str>,
    /// The ticket made by the last «Create invite».
    pub ticket: Option<String>,
    /// The text of the «Paste a ticket» field.
    pub join_text: String,
    /// The last status read.
    pub status: Option<Status>,
    /// The network mode in force.
    pub mode: NetMode,
    /// Why the last request failed.
    pub error: Option<String>,
    pending_mode: Option<NetMode>,
    refreshing: bool,
    retry_start: bool,
    last_refresh: Option<Instant>,
}

impl Default for SyncPanel {
    fn default() -> SyncPanel {
        SyncPanel {
            state: SyncState::Starting,
            busy: None,
            ticket: None,
            join_text: String::new(),
            status: None,
            mode: NetMode::Internet,
            error: None,
            pending_mode: None,
            refreshing: false,
            retry_start: false,
            last_refresh: None,
        }
    }
}

impl fmt::Debug for SyncPanel {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hidden = |text: &str| if text.is_empty() { "" } else { "<redacted>" };
        out.debug_struct("SyncPanel")
            .field("state", &self.state)
            .field("busy", &self.busy)
            .field("ticket", &self.ticket.as_deref().map(hidden))
            .field("join_text", &hidden(&self.join_text))
            .field("status", &self.status)
            .field("mode", &self.mode)
            .field("error", &self.error)
            .finish()
    }
}

fn busy_text(request: &Request) -> Option<&'static str> {
    match request {
        Request::Invite => Some("Creating invite…"),
        Request::Join(_) => Some("Joining…"),
        Request::Leave => Some("Turning sync off…"),
        Request::SetMode(_) => Some("Switching mode…"),
        Request::Start | Request::RefreshStatus | Request::Stop => None,
    }
}

impl SyncPanel {
    /// Sends `request` to the worker and shows that it is in progress.
    pub fn request(&mut self, worker: &SyncWorker, request: Request) {
        self.error = None;
        self.busy = busy_text(&request);
        if let Request::SetMode(mode) = request {
            self.pending_mode = Some(mode);
        }
        worker.send(request);
    }

    /// Takes in one reply of the worker.
    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::NotConfigured | Reply::Stopped => self.state = SyncState::Off,
            Reply::Started(result) => {
                self.refreshing = false;
                self.state = started_state(result);
            }
            Reply::Invited(result) => self.finish(result, |panel, ticket| {
                panel.ticket = Some(ticket);
                panel.state = SyncState::On;
            }),
            Reply::Joined(result) => self.finish(result, |panel, ()| {
                panel.join_text.clear();
                panel.state = SyncState::On;
            }),
            Reply::Left(result) => self.finish(result, |panel, ()| {
                panel.state = SyncState::Off;
                panel.ticket = None;
                panel.status = None;
            }),
            Reply::ModeSet(result) => self.set_mode_done(result),
            Reply::Status(found) => self.status_found(found),
        }
    }

    fn status_found(&mut self, found: EngineStatus) {
        self.refreshing = false;
        match found {
            EngineStatus::Running(status) => self.status = Some(status),
            // The other process may have stopped: ask the lock again, it decides.
            EngineStatus::Unknown => self.retry_start = self.state == SyncState::HandledElsewhere,
            EngineStatus::Gone { configured } => {
                self.status = None;
                self.state = if configured {
                    SyncState::Error("the engine ended on its own".to_owned())
                } else {
                    SyncState::Off
                };
            }
        }
    }

    fn finish<T>(&mut self, result: Result<T, String>, on_success: impl FnOnce(&mut Self, T)) {
        self.busy = None;
        match result {
            Ok(value) => on_success(self, value),
            Err(error) => {
                self.error = Some(error);
                // A failed request may have ended the engine: find out at once.
                self.last_refresh = None;
            }
        }
    }

    fn set_mode_done(&mut self, result: Result<StartOutcome, String>) {
        let wanted = self.pending_mode.take();
        self.finish(result, |panel, outcome| {
            panel.mode = wanted.unwrap_or(panel.mode);
            if outcome == StartOutcome::HandledElsewhere {
                panel.state = SyncState::HandledElsewhere;
            }
        });
    }

    /// One line for the status strip.
    pub fn summary(&self) -> String {
        match (&self.state, &self.status) {
            (SyncState::Off, _) => "Sync off".to_owned(),
            (SyncState::HandledElsewhere, _) => "Sync handled by another process".to_owned(),
            (SyncState::Error(_), _) => "Sync error".to_owned(),
            (SyncState::On, Some(status)) => {
                let online = status.peers.iter().filter(|peer| peer.connected).count();
                let peers = counted(status.peers.len(), "peer", "peers");
                format!("Sync: {online} of {peers} online")
            }
            (SyncState::On | SyncState::Starting, _) => "Sync starting…".to_owned(),
        }
    }

    /// Asks the worker for a fresh status when one is due: every 2 s while the sync window is
    /// open, every 10 s otherwise, never while a request or a refresh is pending. Returns when
    /// the next frame is needed, or `None` when nothing is scheduled.
    pub fn tick(
        &mut self,
        worker: &SyncWorker,
        now: Instant,
        window_open: bool,
    ) -> Option<Duration> {
        let watching = matches!(self.state, SyncState::On | SyncState::HandledElsewhere);
        if !watching || self.busy.is_some() || self.refreshing {
            return None;
        }
        if mem::take(&mut self.retry_start) {
            worker.send(Request::Start);
            self.refreshing = true;
            return None;
        }
        let interval = if window_open {
            WINDOW_REFRESH
        } else {
            IDLE_REFRESH
        };
        let since = self
            .last_refresh
            .map(|last| now.saturating_duration_since(last));
        let wait = since.map_or(Duration::ZERO, |since| interval.saturating_sub(since));
        if !wait.is_zero() {
            return Some(wait);
        }
        worker.send(Request::RefreshStatus);
        self.refreshing = true;
        self.last_refresh = Some(now);
        Some(interval)
    }
}

fn started_state(result: Result<StartOutcome, String>) -> SyncState {
    match result {
        Ok(StartOutcome::Started) => SyncState::On,
        Ok(StartOutcome::HandledElsewhere) => SyncState::HandledElsewhere,
        Err(error) => SyncState::Error(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use sb_sync::{NetMode, PeerStatus, Status};

    use super::*;

    #[derive(Clone, Default)]
    struct Calls(Arc<Mutex<Vec<String>>>);

    impl Calls {
        fn push(&self, call: impl Into<String>) {
            self.0.lock().unwrap().push(call.into());
        }

        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct FakeControl {
        configured: bool,
        start: Result<StartOutcome, String>,
        later_start: Option<Result<StartOutcome, String>>,
        /// What `configured` answers once `start` has been called (the other process left).
        configured_after_start: Option<bool>,
        invite: Result<String, String>,
        invite_delay: Duration,
        status: Option<Status>,
        mode: NetMode,
        calls: Calls,
    }

    fn fake(configured: bool) -> (FakeControl, Calls) {
        let calls = Calls::default();
        let control = FakeControl {
            configured,
            start: Ok(StartOutcome::Started),
            later_start: None,
            configured_after_start: None,
            invite: Ok("sb1/SECRET-TICKET".into()),
            invite_delay: Duration::ZERO,
            status: None,
            mode: NetMode::Internet,
            calls: calls.clone(),
        };
        (control, calls)
    }

    impl SyncControl for FakeControl {
        fn configured(&self) -> bool {
            self.configured
        }

        fn start(&mut self) -> Result<StartOutcome, String> {
            self.calls.push("start");
            let answer = self.start.clone();
            if let Some(configured) = self.configured_after_start {
                self.configured = configured;
            }
            if let Some(next) = self.later_start.take() {
                self.start = next;
            }
            answer
        }

        fn invite(&mut self) -> Result<String, String> {
            self.calls.push("invite");
            thread::sleep(self.invite_delay);
            self.invite.clone()
        }

        fn join(&mut self, _ticket: &str) -> Result<(), String> {
            self.calls.push("join");
            Ok(())
        }

        fn leave(&mut self) -> Result<(), String> {
            self.calls.push("leave");
            Ok(())
        }

        fn status(&mut self) -> EngineStatus {
            self.calls.push("status");
            self.status
                .clone()
                .map_or(EngineStatus::Unknown, EngineStatus::Running)
        }

        fn set_mode(&mut self, mode: NetMode) -> Result<StartOutcome, String> {
            self.calls.push(format!("set_mode({mode:?})"));
            self.mode = mode;
            Ok(StartOutcome::Started)
        }

        fn stop(&mut self) {
            self.calls.push("stop");
        }

        fn mode(&self) -> NetMode {
            self.mode
        }
    }

    fn spawn(control: FakeControl) -> SyncWorker {
        SyncWorker::spawn(Box::new(control), || {})
    }

    /// Waits for `count` replies from the worker.
    fn replies(worker: &SyncWorker, count: usize) -> Vec<Reply> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = Vec::new();
        while got.len() < count {
            got.extend(worker.poll());
            assert!(Instant::now() < deadline, "got only {got:?}");
            thread::sleep(Duration::from_millis(20));
        }
        got
    }

    fn apply_next(panel: &mut SyncPanel, worker: &SyncWorker) {
        for reply in replies(worker, 1) {
            panel.apply(reply);
        }
    }

    fn status_with_peers(connected: &[bool]) -> Status {
        let peers = connected
            .iter()
            .enumerate()
            .map(|(index, &connected)| PeerStatus {
                id: format!("{index:064}"),
                connected,
            })
            .collect();
        Status {
            peers,
            ..Status::default()
        }
    }

    fn running(connected: &[bool]) -> Reply {
        Reply::Status(EngineStatus::Running(status_with_peers(connected)))
    }

    #[test]
    fn should_not_start_engine_when_sync_not_configured() {
        let (control, calls) = fake(false);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        assert_eq!(panel.state, SyncState::Starting);
        let first = replies(&worker, 1);
        assert!(matches!(first[0], Reply::NotConfigured), "{first:?}");
        panel.apply(Reply::NotConfigured);
        assert_eq!(panel.state, SyncState::Off);
        assert!(calls.all().is_empty(), "{:?}", calls.all());
    }

    #[test]
    fn should_start_engine_when_configured() {
        let (control, calls) = fake(true);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::On);
        assert_eq!(calls.all(), ["start"]);
    }

    #[test]
    fn should_show_handled_elsewhere_when_lock_taken() {
        let (mut control, _calls) = fake(true);
        control.start = Ok(StartOutcome::HandledElsewhere);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::HandledElsewhere);
        assert_eq!(panel.summary(), "Sync handled by another process");
    }

    #[test]
    fn should_stay_responsive_during_slow_invite() {
        let (mut control, _calls) = fake(false);
        control.invite_delay = Duration::from_secs(2);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        let sent = Instant::now();
        panel.request(&worker, Request::Invite);
        assert!(sent.elapsed() < Duration::from_millis(500));
        assert_eq!(panel.busy, Some("Creating invite…"));
        thread::sleep(Duration::from_secs(1));
        assert!(worker.poll().is_empty(), "the invite finished too early");
        apply_next(&mut panel, &worker);
        assert_eq!(panel.busy, None);
        assert_eq!(panel.ticket.as_deref(), Some("sb1/SECRET-TICKET"));
        assert_eq!(panel.state, SyncState::On);
    }

    #[test]
    fn should_show_error_when_invite_fails() {
        let (mut control, _calls) = fake(false);
        control.invite = Err("network down".into());
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        panel.request(&worker, Request::Invite);
        apply_next(&mut panel, &worker);
        assert_eq!(panel.busy, None);
        assert_eq!(panel.error.as_deref(), Some("network down"));
        assert_eq!(panel.ticket, None);
        assert_eq!(panel.state, SyncState::Off);
    }

    #[test]
    fn should_stop_engine_when_leaving() {
        let (control, calls) = fake(true);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        panel.request(&worker, Request::Leave);
        assert_eq!(panel.busy, Some("Turning sync off…"));
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::Off);
        assert_eq!(panel.busy, None);
        assert_eq!(calls.all(), ["start", "leave"]);
    }

    #[test]
    fn should_save_mode_and_restart_when_switching() {
        let (control, calls) = fake(true);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        panel.request(&worker, Request::SetMode(NetMode::Lan));
        assert_eq!(panel.busy, Some("Switching mode…"));
        apply_next(&mut panel, &worker);
        assert_eq!(panel.mode, NetMode::Lan);
        assert_eq!(panel.busy, None);
        assert!(calls.all().contains(&"set_mode(Lan)".to_owned()));
    }

    #[test]
    fn should_count_online_peers_in_summary() {
        let mut panel = SyncPanel::default();
        panel.apply(Reply::Started(Ok(StartOutcome::Started)));
        panel.apply(running(&[true, false, true]));
        assert_eq!(panel.summary(), "Sync: 2 of 3 peers online");
    }

    #[test]
    fn should_summarise_every_state_in_words() {
        let mut panel = SyncPanel::default();
        assert_eq!(panel.summary(), "Sync starting…");
        panel.apply(Reply::NotConfigured);
        assert_eq!(panel.summary(), "Sync off");
        panel.apply(Reply::Started(Err("boom".into())));
        assert_eq!(panel.summary(), "Sync error");
    }

    #[test]
    fn should_refresh_every_two_seconds_with_window_open_and_ten_otherwise() {
        let (mut control, calls) = fake(true);
        control.status = Some(status_with_peers(&[true]));
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        let t0 = Instant::now();
        let second = Duration::from_secs(1);
        assert_eq!(panel.tick(&worker, t0, true), Some(2 * second));
        assert_eq!(panel.tick(&worker, t0, true), None, "one refresh at a time");
        apply_next(&mut panel, &worker);
        assert_eq!(panel.tick(&worker, t0 + second, true), Some(second));
        assert_eq!(panel.tick(&worker, t0 + 2 * second, true), Some(2 * second));
        apply_next(&mut panel, &worker);
        assert_eq!(
            panel.tick(&worker, t0 + 3 * second, false),
            Some(9 * second)
        );
        let status_calls = calls.all().iter().filter(|call| *call == "status").count();
        assert_eq!(status_calls, 2);
    }

    #[test]
    fn should_not_refresh_while_off() {
        let (control, calls) = fake(false);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        assert_eq!(panel.tick(&worker, Instant::now(), true), None);
        assert!(!calls.all().contains(&"status".to_owned()));
    }

    #[test]
    fn should_stop_control_and_report_it_when_stopping_worker() {
        let (control, calls) = fake(true);
        let worker = spawn(control);
        assert!(worker.stop_and_join(Duration::from_secs(10)));
        assert_eq!(calls.all(), ["start", "stop"]);
    }

    #[test]
    fn should_give_up_waiting_when_the_control_is_stuck() {
        let (mut control, _calls) = fake(false);
        control.invite_delay = Duration::from_secs(2);
        let worker = spawn(control);
        worker.send(Request::Invite);
        assert!(!worker.stop_and_join(Duration::from_millis(100)));
    }

    #[test]
    fn should_never_print_a_ticket_in_debug_output() {
        let secret = "sb1/SECRET-TICKET";
        let mut panel = SyncPanel::default();
        panel.apply(Reply::Invited(Ok(secret.into())));
        panel.join_text = secret.into();
        let shown = [
            format!("{:?}", Request::Join(secret.into())),
            format!("{:?}", Reply::Invited(Ok(secret.into()))),
            format!("{panel:?}"),
        ];
        for text in shown {
            assert!(!text.contains("SECRET"), "{text}");
            assert!(text.contains("<redacted>"), "{text}");
        }
    }

    fn running_panel() -> SyncPanel {
        let mut panel = SyncPanel::default();
        panel.apply(Reply::Started(Ok(StartOutcome::Started)));
        panel
    }

    #[test]
    fn should_turn_off_when_the_engine_is_gone_and_the_group_was_left() {
        let mut panel = running_panel();
        panel.apply(Reply::Status(EngineStatus::Gone { configured: false }));
        assert_eq!(panel.state, SyncState::Off);
    }

    #[test]
    fn should_show_a_short_error_when_the_engine_is_gone_but_the_group_remains() {
        let mut panel = running_panel();
        panel.apply(running(&[true]));
        panel.apply(Reply::Status(EngineStatus::Gone { configured: true }));
        assert_eq!(
            panel.state,
            SyncState::Error("the engine ended on its own".into())
        );
        assert_eq!(panel.summary(), "Sync error");
        assert_eq!(panel.status, None);
    }

    #[test]
    fn should_show_handled_elsewhere_when_a_mode_restart_loses_the_lock() {
        let mut panel = running_panel();
        panel.apply(Reply::ModeSet(Ok(StartOutcome::HandledElsewhere)));
        assert_eq!(panel.state, SyncState::HandledElsewhere);
    }

    #[test]
    fn should_keep_state_on_when_a_mode_restart_succeeds() {
        let mut panel = running_panel();
        panel.apply(Reply::ModeSet(Ok(StartOutcome::Started)));
        assert_eq!(panel.state, SyncState::On);
    }

    #[test]
    fn should_keep_the_last_status_when_a_refresh_learns_nothing() {
        let mut panel = running_panel();
        panel.apply(running(&[true]));
        panel.apply(Reply::Status(EngineStatus::Unknown));
        assert_eq!(panel.summary(), "Sync: 1 of 1 peer online");
    }

    #[test]
    fn should_try_to_start_again_when_the_other_process_left_no_fresh_status() {
        let (mut control, calls) = fake(true);
        control.start = Ok(StartOutcome::HandledElsewhere);
        control.later_start = Some(Ok(StartOutcome::Started));
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::HandledElsewhere);
        let t0 = Instant::now();
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::HandledElsewhere);
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::On);
        assert_eq!(calls.all(), ["start", "status", "start"]);
    }

    #[test]
    fn should_not_start_an_engine_when_the_other_process_left_the_group() {
        let (mut control, calls) = fake(true);
        control.start = Ok(StartOutcome::HandledElsewhere);
        control.configured_after_start = Some(false);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::HandledElsewhere);
        let t0 = Instant::now();
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        assert_eq!(panel.state, SyncState::Off);
        assert_eq!(calls.all(), ["start", "status"]);
    }

    #[test]
    fn should_not_start_again_while_the_other_process_reports_a_status() {
        let (mut control, calls) = fake(true);
        control.start = Ok(StartOutcome::HandledElsewhere);
        control.status = Some(status_with_peers(&[true]));
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        let t0 = Instant::now();
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        assert_eq!(panel.tick(&worker, t0, true), Some(Duration::from_secs(2)));
        assert_eq!(calls.all(), ["start", "status"]);
    }

    #[test]
    fn should_refresh_at_once_after_a_failed_request() {
        let (control, _calls) = fake(true);
        let worker = spawn(control);
        let mut panel = SyncPanel::default();
        apply_next(&mut panel, &worker);
        let t0 = Instant::now();
        panel.tick(&worker, t0, true);
        apply_next(&mut panel, &worker);
        let soon = t0 + Duration::from_millis(100);
        assert!(panel.tick(&worker, soon, true) < Some(Duration::from_secs(2)));
        panel.apply(Reply::Left(Err("did not answer".into())));
        assert_eq!(
            panel.tick(&worker, soon, true),
            Some(Duration::from_secs(2))
        );
        assert!(panel.refreshing);
    }
}
