//! The `sb sync` commands: thin wrappers over `sb_sync::control` and the engine.

use std::io;
use std::sync::mpsc::{self, Sender};

use sb_core::Vault;
use sb_sync::control::{self, Applied};
use sb_sync::{
    EngineEvent, StateDir, Status, SyncError, Ticket, ensure_identity, start_engine, to_hex,
    unix_now,
};
use serde_json::json;

use crate::cli::SyncAction;
use crate::commands::CliError;
use crate::render;

/// Runs one `sb sync` command for `vault`.
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
    let applied = control::leave(vault, &state_of(vault)?)?;
    report_applied("leave", applied, json)
}

fn invite(state: &StateDir, json: bool) -> Result<(), CliError> {
    ensure_identity(state)?;
    let ticket = control::invite(state)?;
    if ticket.addrs.is_empty() {
        eprintln!(
            "note: no address hints, because no engine is running; \
             a running `sb sync run` adds them to the ticket"
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
        Some(status) => render::emit(json, &status, status_text),
        None => render::emit(json, &json!({ "running": false }), |_| {
            "engine not running\n".to_owned()
        }),
    }
}

fn status_text(status: &Status) -> String {
    let mut text = format!("device {}\n", status.device_id);
    if status.addresses.is_empty() {
        text += "listening on: (no address yet)\n";
    } else {
        text += &format!("listening on {}\n", status.addresses.join(", "));
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

/// Runs the engine in the foreground and logs its events to stderr (stdout stays empty).
fn run_engine(vault: &Vault, state: StateDir) -> Result<(), CliError> {
    let (events, inbox) = mpsc::channel();
    watch_for_interrupt(events.clone())?;
    // The network builds its own runtime, so the engine starts here, on a plain thread.
    let engine = start_engine(vault.root(), state.clone(), Some(events))?;
    if let Some(id) = state.device_id()? {
        eprintln!("device {}", to_hex(&id));
    }
    // The engine answers only after its first round, which can take long on a big vault: never fatal.
    eprintln!("{}", listening_line(engine.status()));
    while let Ok(event) = inbox.recv() {
        eprintln!("{}", describe(&event));
        if event == EngineEvent::Stopped {
            break;
        }
    }
    Ok(engine.shutdown()?)
}

fn listening_line(status: Result<Status, SyncError>) -> String {
    match status {
        Ok(status) => format!("listening on {}", status.addresses.join(", ")),
        Err(_) => {
            "starting: the engine is still scanning the vault; see `sb sync status`".to_owned()
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
        .name("sb-interrupt".into())
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
        let timeout = SyncError::Net("the engine did not answer in time".into());
        let line = listening_line(Err(timeout));
        assert!(line.starts_with("starting: the engine is still scanning the vault"));
        assert!(line.contains("sb sync status"), "{line}");
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
    fn should_say_so_in_the_text_status_when_there_is_no_address_yet() {
        let text = status_text(&Status::default());
        assert!(text.contains("listening on: (no address yet)\n"), "{text}");
    }
}
