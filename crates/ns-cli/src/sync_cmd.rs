//! The `ns sync` commands: thin wrappers over `ns_sync::control` and the engine.

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use ns_core::Vault;
use ns_sync::control::{self, Applied};
use ns_sync::{
    EngineEvent, EngineHandle, NetMode, StateDir, Status, SyncError, Ticket, ensure_identity,
    start_engine, to_hex, unix_now,
};
use serde_json::json;

use crate::cli::SyncAction;
use crate::commands::CliError;
use crate::render;

/// Runs one `ns sync` command for `vault`.
///
/// # Errors
/// Any `CliError`; the caller prints it and exits with code 1.
pub fn run(vault: &Vault, action: SyncAction, json: bool) -> Result<(), CliError> {
    match action {
        SyncAction::Run => run_engine(vault, state_of(vault)?),
        SyncAction::Invite => invite(&state_of(vault)?, json),
        SyncAction::Join { ticket } => join(vault, &ticket.0, json),
        SyncAction::Leave => leave(vault, json),
        SyncAction::Status => status(&state_of(vault)?, json),
    }
}

fn state_of(vault: &Vault) -> Result<StateDir, CliError> {
    Ok(StateDir::for_vault(vault.root())?)
}

/// Parses the ticket first, so that a bad one creates no state.
fn join(vault: &Vault, ticket: &str, json: bool) -> Result<(), CliError> {
    let ticket: Ticket = ticket.parse()?;
    let applied = control::join(vault, &state_of(vault)?, &ticket)?;
    report_applied("join", applied, json)
}

fn leave(vault: &Vault, json: bool) -> Result<(), CliError> {
    let applied = control::leave(&state_of(vault)?)?;
    report_applied("leave", applied, json)
}

fn invite(state: &StateDir, json: bool) -> Result<(), CliError> {
    ensure_identity(state)?;
    let ticket = control::invite(state)?;
    if ticket.addrs.is_empty() {
        eprintln!(
            "note: no address hints, because no engine is running; \
             a running `ns sync run` adds them to the ticket"
        );
    }
    render::emit(json, &json!({ "ticket": ticket.to_string() }), |_| {
        format!("{ticket}\n")
    })
}

fn report_applied(verb: &str, applied: Applied, json: bool) -> Result<(), CliError> {
    let (key, text) = match applied {
        Applied::Now => ("now", format!("{verb} applied now\n")),
        Applied::Queued => ("queued", format!("{verb} handed to the running engine\n")),
    };
    render::emit(json, &json!({ "applied": key }), |_| text)
}

fn status(state: &StateDir, json: bool) -> Result<(), CliError> {
    match control::status(state, unix_now())? {
        Some(status) => {
            let mode = state.config()?.mode;
            render::emit(json, &status, |status| status_text(status, mode))
        }
        None => render::emit(json, &json!({ "running": false }), |_| {
            "engine not running\n".to_owned()
        }),
    }
}

const NO_RELAY_YET: &str = "relay: none yet (if this lasts more than a minute, a firewall may be blocking ns; allow it outbound)\n";

fn status_text(status: &Status, mode: NetMode) -> String {
    let mut text = format!("device {}\n", status.device_id);
    if status.addresses.is_empty() {
        text += "listening on: (no address yet)\n";
    } else {
        text += &format!("listening on {}\n", status.addresses.join(", "));
    }
    match (&status.relay, mode) {
        (Some(relay), _) => text += &format!("relay: {relay}\n"),
        (None, NetMode::Internet) => text += NO_RELAY_YET,
        (None, NetMode::Lan) => {}
    }
    text += &format!("notes {}\n", status.notes);
    for peer in &status.peers {
        let state = if peer.connected {
            "connected"
        } else {
            "not connected"
        };
        text += &format!("peer {} {state}\n", peer.id);
    }
    for note in &status.blocked {
        text += &format!("blocked {}: {}\n", note.path, note.reason);
    }
    if let Some(error) = &status.last_error {
        text += &format!("last error: {error}\n");
    }
    text
}

/// How long a start retries while another command (`ns sync invite`) briefly holds the engine lock.
const LOCK_RETRY: Duration = Duration::from_secs(1);
const LOCK_RETRY_STEP: Duration = Duration::from_millis(50);
/// How long the first status is awaited before the log says that the engine is still scanning.
const FIRST_STATUS_WAIT: Duration = Duration::from_secs(10);
const EVENT_POLL: Duration = Duration::from_millis(100);

/// Runs the engine in the foreground and logs its events to stderr (stdout stays empty).
fn run_engine(vault: &Vault, state: StateDir) -> Result<(), CliError> {
    let (events, inbox) = mpsc::channel();
    watch_for_interrupt(events.clone())?;
    // The network builds its own runtime, so the engine starts here, on a plain thread.
    let engine = start_when_free(vault, &state, &events)?;
    // Not fatal: the engine is already running, and a stop must still reach it.
    match state.device_id() {
        Ok(Some(id)) => eprintln!("device {}", to_hex(&id)),
        Ok(None) => {}
        Err(error) => eprintln!("cannot read the device id: {error}"),
    }
    // The engine answers only after its first round, which can take long on a big vault: never fatal,
    // and never in the way of a stop request.
    let first = engine.request_status()?;
    follow(&inbox, &first, FIRST_STATUS_WAIT, &mut |line| {
        eprintln!("{line}")
    });
    Ok(engine.shutdown()?)
}

fn start_when_free(
    vault: &Vault,
    state: &StateDir,
    events: &Sender<EngineEvent>,
) -> Result<EngineHandle, SyncError> {
    retry_while_locked(LOCK_RETRY, LOCK_RETRY_STEP, || {
        start_engine(vault.root(), state.clone(), Some(events.clone()))
    })
}

/// Retries `attempt` while it reports `AlreadyRunning`, for at most `budget`.
fn retry_while_locked<T>(
    budget: Duration,
    step: Duration,
    mut attempt: impl FnMut() -> Result<T, SyncError>,
) -> Result<T, SyncError> {
    let deadline = Instant::now() + budget;
    loop {
        match attempt() {
            Err(SyncError::AlreadyRunning) if Instant::now() < deadline => {
                std::thread::sleep(step);
            }
            other => return other,
        }
    }
}

/// Logs the events until `Stopped`, and the listening line when the first status arrives. A stop is
/// seen within `EVENT_POLL` whether or not the first status has come.
fn follow(
    inbox: &Receiver<EngineEvent>,
    first: &Receiver<Status>,
    wait: Duration,
    log: &mut impl FnMut(&str),
) {
    let started = Instant::now();
    let mut waiting = true;
    loop {
        match inbox.recv_timeout(EVENT_POLL) {
            Ok(event) => {
                log(&describe(&event));
                if event == EngineEvent::Stopped {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if waiting {
            match first.try_recv() {
                Ok(status) => {
                    log(&listening_line(Ok(status)));
                    waiting = false;
                }
                Err(TryRecvError::Empty) if started.elapsed() < wait => {}
                Err(_) => {
                    log(&listening_line(Err(())));
                    waiting = false;
                }
            }
        }
    }
}

fn listening_line(status: Result<Status, ()>) -> String {
    match status {
        Ok(status) => format!("listening on {}", status.addresses.join(", ")),
        Err(()) => {
            "starting: the engine is still scanning the vault; see `ns sync status`".to_owned()
        }
    }
}

fn describe(event: &EngineEvent) -> String {
    match event {
        EngineEvent::PeerConnected(id) => format!("peer connected {id}"),
        EngineEvent::PeerDisconnected(id) => format!("peer disconnected {id}"),
        EngineEvent::Synced => "synced".to_owned(),
        EngineEvent::Blocked(count) => format!("blocked notes: {count}"),
        EngineEvent::Error(message) => format!("error: {message}"),
        EngineEvent::Notice(message) => format!("note: {message}"),
        EngineEvent::Stopped => "stopping".to_owned(),
    }
}

/// Reports an interrupt as `Stopped`: whether the engine stopped itself or was asked to, the run is over.
/// The runtime lives on its own thread so that it never surrounds the engine's network start.
fn watch_for_interrupt(events: Sender<EngineEvent>) -> Result<(), CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("ns-interrupt".into())
        .spawn(move || {
            let event = match runtime.block_on(interrupted()) {
                Ok(()) => EngineEvent::Stopped,
                Err(error) => EngineEvent::Error(format!("cannot listen for interrupts: {error}")),
            };
            let _ = events.send(event);
        })?;
    Ok(())
}

#[cfg(unix)]
async fn interrupted() -> io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn interrupted() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn should_print_a_starting_line_when_the_first_status_is_not_available() {
        let line = listening_line(Err(()));
        assert!(line.starts_with("starting: the engine is still scanning the vault"));
        assert!(line.contains("ns sync status"), "{line}");
    }

    #[test]
    fn should_list_the_addresses_of_a_status() {
        let status = Status {
            addresses: vec!["1.2.3.4:5".into(), "6.7.8.9:10".into()],
            ..Status::default()
        };
        let line = listening_line(Ok(status));
        assert_eq!(line, "listening on 1.2.3.4:5, 6.7.8.9:10");
    }

    #[test]
    fn should_retry_while_the_lock_is_held_and_then_succeed() {
        let mut calls = 0;
        let result = retry_while_locked(Duration::from_secs(5), Duration::from_millis(1), || {
            calls += 1;
            if calls < 3 {
                Err(SyncError::AlreadyRunning)
            } else {
                Ok(calls)
            }
        });
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn should_give_up_when_the_lock_stays_held() {
        let started = Instant::now();
        let result: Result<(), SyncError> = retry_while_locked(
            Duration::from_millis(100),
            Duration::from_millis(10),
            || Err(SyncError::AlreadyRunning),
        );
        assert!(matches!(result, Err(SyncError::AlreadyRunning)));
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn should_not_retry_other_errors() {
        let mut calls = 0;
        let result: Result<(), SyncError> =
            retry_while_locked(Duration::from_secs(5), Duration::from_millis(1), || {
                calls += 1;
                Err(SyncError::NotConfigured)
            });
        assert!(matches!(result, Err(SyncError::NotConfigured)));
        assert_eq!(calls, 1);
    }

    #[test]
    fn should_see_a_stop_while_the_first_status_is_still_pending() {
        let (events, inbox) = mpsc::channel();
        let (_never_answers, first) = mpsc::channel();
        events.send(EngineEvent::Stopped).unwrap();
        let started = Instant::now();
        let mut lines = Vec::new();
        follow(&inbox, &first, Duration::from_secs(30), &mut |line| {
            lines.push(line.to_owned());
        });
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(lines, ["stopping"]);
    }

    #[test]
    fn should_log_the_listening_line_when_the_first_status_arrives() {
        let (events, inbox) = mpsc::channel();
        let (answer, first) = mpsc::channel();
        let status = Status {
            addresses: vec!["1.2.3.4:5".into()],
            ..Status::default()
        };
        answer.send(status).unwrap();
        events.send(EngineEvent::Notice("hello".into())).unwrap();
        events.send(EngineEvent::Stopped).unwrap();
        let mut lines = Vec::new();
        follow(&inbox, &first, Duration::from_secs(30), &mut |line| {
            lines.push(line.to_owned());
        });
        assert_eq!(lines, ["note: hello", "listening on 1.2.3.4:5", "stopping"]);
    }

    #[test]
    fn should_say_so_in_the_text_status_when_there_is_no_address_yet() {
        let text = status_text(&Status::default(), NetMode::Lan);
        assert!(text.contains("listening on: (no address yet)\n"), "{text}");
    }

    #[test]
    fn should_show_the_relay_in_internet_mode() {
        let status = Status {
            relay: Some("https://relay.example./".into()),
            ..Status::default()
        };
        let text = status_text(&status, NetMode::Internet);
        assert!(text.contains("relay: https://relay.example./\n"), "{text}");
    }

    #[test]
    fn should_warn_about_a_firewall_when_internet_mode_has_no_relay_yet() {
        let text = status_text(&Status::default(), NetMode::Internet);
        let line = "relay: none yet (if this lasts more than a minute, a firewall may be blocking ns; allow it outbound)\n";
        assert!(text.contains(line), "{text}");
    }

    #[test]
    fn should_print_nothing_about_the_relay_in_lan_mode() {
        let text = status_text(&Status::default(), NetMode::Lan);
        assert!(!text.contains("relay"), "{text}");
    }
}
