//! The file-based control plane the command line uses (spec section 8.8): it works whether
//! or not an engine is running, and never touches the network.

use std::time::Duration;

use ns_core::Vault;

use crate::error::{Result, SyncError};
use crate::replica::{Replica, unix_now};
use crate::state::{PendingCommand, StateDir, Status, remove_if_exists};
use crate::ticket::Ticket;

/// A status older than this many seconds means that no engine is running.
pub const STATUS_MAX_AGE_SECS: u64 = 30;

/// What happened to a `join` or `leave`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// It was carried out; no engine was running.
    Now,
    /// An engine is running; it carries the command out on its next tick.
    Queued,
}

/// A ticket for this device, with the addresses a running engine published less than
/// `STATUS_MAX_AGE_SECS` ago.
///
/// # Errors
/// `NotConfigured` when the device id or the group secret is missing.
pub fn invite(state: &StateDir) -> Result<Ticket> {
    let device_id = state.device_id()?.ok_or(SyncError::NotConfigured)?;
    let secret = state.secret()?.ok_or(SyncError::NotConfigured)?;
    let addresses = status(state, unix_now())?.map(|status| status.addresses);
    let addrs = addresses.unwrap_or_default();
    Ok(Ticket {
        device_id,
        secret,
        addrs: addrs.iter().filter_map(|text| text.parse().ok()).collect(),
    })
}

/// Joins the group of a ticket now, or queues the join for the running engine.
///
/// # Errors
/// Failures of the state directory or the vault's state.
pub fn join(vault: &Vault, state: &StateDir, ticket: &Ticket) -> Result<Applied> {
    let command = PendingCommand::Join(ticket.to_string());
    apply_or_queue(state, &command, || {
        // The id is not used on this path: no session is opened.
        let device_id = state.device_id()?.unwrap_or_default();
        let mut replica = Replica::open(vault.clone(), state.clone(), device_id)?;
        replica.join(ticket, Duration::ZERO).map(drop)
    })
}

/// Leaves the group now, or queues the command for the running engine. Leaving needs neither
/// the vault nor the document, so it works when either is broken.
///
/// # Errors
/// Failures of the state directory.
pub fn leave(state: &StateDir) -> Result<Applied> {
    apply_or_queue(state, &PendingCommand::Leave(true), || state.clear_group())
}

/// The engine's status, or `None` when there is none or it is older than `STATUS_MAX_AGE_SECS`.
///
/// # Errors
/// `CorruptState` when `status.json` is unusable.
pub fn status(state: &StateDir, now_unix: u64) -> Result<Option<Status>> {
    let fresh =
        |status: &Status| now_unix.saturating_sub(status.updated_unix) <= STATUS_MAX_AGE_SECS;
    Ok(state.read_status()?.filter(fresh))
}

/// Holding the engine lock means no engine runs, so the command is applied directly, with the
/// lock held until it is done.
fn apply_or_queue(
    state: &StateDir,
    command: &PendingCommand,
    apply: impl FnOnce() -> Result<()>,
) -> Result<Applied> {
    let _lock = match state.lock() {
        Ok(lock) => lock,
        Err(SyncError::AlreadyRunning) => {
            state.write_pending(command)?;
            return Ok(Applied::Queued);
        }
        Err(error) => return Err(error),
    };
    // A command left from before would otherwise run on top of the new state at the next engine start.
    remove_if_exists(&state.file("command.pending"))?;
    apply()?;
    Ok(Applied::Now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PeerEntry, to_hex};

    struct Fixture {
        _dirs: (tempfile::TempDir, tempfile::TempDir),
        vault: Vault,
        state: StateDir,
    }

    fn fixture() -> Fixture {
        let vault_dir = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(vault_dir.path()).unwrap();
        let state = StateDir::at(state_dir.path()).unwrap();
        Fixture {
            _dirs: (vault_dir, state_dir),
            vault,
            state,
        }
    }

    fn ticket() -> Ticket {
        Ticket {
            device_id: [9; 32],
            secret: [8; 32],
            addrs: vec!["127.0.0.1:4000".parse().unwrap()],
        }
    }

    fn pending(state: &StateDir) -> serde_json::Value {
        let bytes = std::fs::read(state.file("command.pending")).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn status_at(updated_unix: u64) -> Status {
        Status {
            addresses: vec!["127.0.0.1:4000".to_string()],
            updated_unix,
            ..Status::default()
        }
    }

    #[test]
    fn should_apply_join_directly_when_no_engine_runs() {
        let fx = fixture();
        let (vault, state) = (&fx.vault, &fx.state);
        state.save_secret(&[7; 32]).unwrap();
        assert_eq!(join(vault, state, &ticket()).unwrap(), Applied::Now);
        assert_eq!(state.secret().unwrap(), Some([8; 32]));
        let inviter = PeerEntry {
            id: to_hex(&[9; 32]),
            addrs: vec!["127.0.0.1:4000".to_string()],
        };
        assert_eq!(state.peers().unwrap(), [inviter]);
        assert!(state.fresh_join());
        assert!(state.lock().is_ok());
    }

    #[test]
    fn should_queue_join_when_an_engine_holds_the_lock() {
        let fx = fixture();
        let (vault, state) = (&fx.vault, &fx.state);
        let _engine = state.lock().unwrap();
        assert_eq!(join(vault, state, &ticket()).unwrap(), Applied::Queued);
        let expected = serde_json::json!({ "join": ticket().to_string() });
        assert_eq!(pending(state), expected);
        assert_eq!(state.secret().unwrap(), None);
    }

    #[test]
    fn should_discard_a_stale_pending_command_when_applying_directly() {
        let fx = fixture();
        let (vault, state) = (&fx.vault, &fx.state);
        state.save_secret(&[7; 32]).unwrap();
        let stale = PendingCommand::Leave(true);
        state.write_pending(&stale).unwrap();
        assert_eq!(join(vault, state, &ticket()).unwrap(), Applied::Now);
        assert!(!state.file("command.pending").exists());
        state.write_pending(&stale).unwrap();
        assert_eq!(leave(state).unwrap(), Applied::Now);
        assert!(!state.file("command.pending").exists());
    }

    #[test]
    fn should_apply_or_queue_leave_the_same_way() {
        let fx = fixture();
        let state = &fx.state;
        state.save_secret(&[7; 32]).unwrap();
        {
            let _engine = state.lock().unwrap();
            assert_eq!(leave(state).unwrap(), Applied::Queued);
            assert_eq!(pending(state), serde_json::json!({ "leave": true }));
            assert_eq!(state.secret().unwrap(), Some([7; 32]));
        }
        assert_eq!(leave(state).unwrap(), Applied::Now);
        assert_eq!(state.secret().unwrap(), None);
    }

    #[test]
    fn should_leave_even_when_the_document_is_corrupt() {
        let fx = fixture();
        let (vault, state) = (&fx.vault, &fx.state);
        state.save_secret(&[7; 32]).unwrap();
        std::fs::create_dir(vault.root().join(".neurospace")).unwrap();
        std::fs::write(state.file("doc.snap"), b"not a snapshot").unwrap();
        assert_eq!(leave(state).unwrap(), Applied::Now);
        assert_eq!(state.secret().unwrap(), None);
        assert!(!state.file("doc.snap").exists());
    }

    #[test]
    fn should_leave_even_when_the_vault_lost_its_marker_folder() {
        let fx = fixture();
        let state = &fx.state;
        state.save_secret(&[7; 32]).unwrap();
        std::fs::write(state.file("doc.log"), b"x").unwrap();
        assert_eq!(leave(state).unwrap(), Applied::Now);
        assert_eq!(state.secret().unwrap(), None);
    }

    #[test]
    fn should_invite_with_addresses_from_fresh_status_only() {
        let fx = fixture();
        let state = &fx.state;
        assert!(matches!(invite(state), Err(SyncError::NotConfigured)));
        state.save_device_id(&[1; 32]).unwrap();
        state.save_secret(&[7; 32]).unwrap();
        let address = "127.0.0.1:4000".parse().unwrap();
        for (age, addrs) in [(10, vec![address]), (31, vec![])] {
            state.write_status(&status_at(unix_now() - age)).unwrap();
            let expected = Ticket {
                device_id: [1; 32],
                secret: [7; 32],
                addrs,
            };
            assert_eq!(invite(state).unwrap(), expected, "status {age} s old");
        }
    }

    #[test]
    fn should_report_status_only_while_fresh() {
        let fx = fixture();
        let state = &fx.state;
        assert_eq!(status(state, 1000).unwrap(), None);
        state.write_status(&status_at(1000)).unwrap();
        assert!(status(state, 1030).unwrap().is_some());
        assert_eq!(status(state, 1031).unwrap(), None);
    }
}
