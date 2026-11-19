//! The production `SyncControl`: a thin adapter from the sync window to the M2 engine.
//!
//! Every method may block (starting the network, asking the engine, stopping it), so it is only
//! called from the `SyncWorker` thread, which is a plain thread, never a tokio one.

use std::mem;
use std::path::{Path, PathBuf};

use ns_sync::{
    EngineHandle, NetConfig, NetMode, Result, StateDir, Status, SyncError, Ticket, control,
    start_engine, unix_now,
};

use crate::state::sync_panel::{EngineStatus, StartOutcome, SyncControl};

/// How an engine is started; a test replaces it so that no engine or network is needed.
type StartFn = fn(&Path, StateDir) -> Result<EngineHandle>;

fn start_real_engine(vault_root: &Path, state: StateDir) -> Result<EngineHandle> {
    start_engine(vault_root, state, None)
}

/// The sync engine of one vault, started on demand.
pub struct EngineControl {
    vault_root: PathBuf,
    state: StateDir,
    handle: Option<EngineHandle>,
    start: StartFn,
    /// An engine was started and nobody asked it to stop, so its absence is news.
    engine_expected: bool,
    /// The last status read from the file; stands in when a later read fails.
    last_status: Option<Status>,
}

impl EngineControl {
    /// Control of the engine of `vault_root`, whose state lives under `state_base`. Starts
    /// nothing and touches no network.
    ///
    /// # Errors
    /// The state directory cannot be created or the vault path cannot be resolved.
    pub fn new(vault_root: &Path, state_base: &Path) -> Result<EngineControl> {
        Self::with_start(vault_root, state_base, start_real_engine)
    }

    fn with_start(vault_root: &Path, state_base: &Path, start: StartFn) -> Result<EngineControl> {
        Ok(EngineControl {
            vault_root: vault_root.to_path_buf(),
            state: StateDir::for_vault_in(state_base, vault_root)?,
            handle: None,
            start,
            engine_expected: false,
            last_status: None,
        })
    }

    /// The running engine; an engine that ended by itself (its group was left elsewhere) is dropped.
    fn running(&mut self) -> Option<&EngineHandle> {
        if self
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.is_running())
        {
            self.handle = None;
        }
        self.handle.as_ref()
    }

    fn engine(&mut self) -> Result<&EngineHandle> {
        if self.running().is_none() {
            let started = (self.start)(&self.vault_root, self.state.clone())?;
            self.engine_expected = true;
            return Ok(self.handle.insert(started));
        }
        self.handle.as_ref().ok_or(SyncError::NotConfigured)
    }

    /// Runs `operation` on the engine, starting it if needed. An engine this call started is
    /// stopped again when the operation fails, so none runs behind an «off» window.
    fn with_engine<T>(
        &mut self,
        operation: impl FnOnce(&EngineHandle) -> Result<T>,
    ) -> std::result::Result<T, String> {
        let started_here = self.running().is_none();
        let result = self.engine().and_then(operation);
        if started_here && result.is_err() {
            self.stop();
        }
        result.map_err(text)
    }

    /// The status from `status.json`; an unreadable file keeps the last good one while that is no
    /// older than `STATUS_MAX_AGE_SECS`,
    /// because Windows can refuse a read while the engine replaces the file.
    fn file_status(&mut self) -> EngineStatus {
        match control::status(&self.state, unix_now()) {
            Ok(Some(status)) => {
                self.last_status = Some(status.clone());
                EngineStatus::Running(status)
            }
            Ok(None) => {
                self.last_status = None;
                EngineStatus::Unknown
            }
            Err(_) => self
                .last_status
                .clone()
                .filter(|last| {
                    unix_now().saturating_sub(last.updated_unix) <= control::STATUS_MAX_AGE_SECS
                })
                .map_or(EngineStatus::Unknown, EngineStatus::Running),
        }
    }

    /// Ends our engine and waits for it; says nothing about whether that was asked for.
    fn halt(&mut self) {
        if let Some(engine) = self.handle.take() {
            // A panicked engine thread has nothing left to stop.
            let _ = engine.shutdown();
        }
    }
}

fn text(error: SyncError) -> String {
    error.to_string()
}

impl SyncControl for EngineControl {
    fn configured(&self) -> bool {
        matches!(self.state.secret(), Ok(Some(_)))
    }

    fn start(&mut self) -> std::result::Result<StartOutcome, String> {
        match self.engine() {
            Ok(_) => Ok(StartOutcome::Started),
            Err(SyncError::AlreadyRunning) => {
                self.engine_expected = false;
                Ok(StartOutcome::HandledElsewhere)
            }
            Err(error) => Err(text(error)),
        }
    }

    fn invite(&mut self) -> std::result::Result<String, String> {
        self.with_engine(EngineHandle::invite)
    }

    fn join(&mut self, ticket: &str) -> std::result::Result<(), String> {
        // Parsed before anything starts: a mistyped ticket must not bring the network up.
        let ticket: Ticket = ticket.parse().map_err(text)?;
        self.with_engine(|engine| engine.join(&ticket.to_string()))
    }

    fn leave(&mut self) -> std::result::Result<(), String> {
        let Some(engine) = self.running() else {
            return control::leave(&self.state).map(drop).map_err(text);
        };
        engine.leave().map_err(text)?;
        self.stop();
        Ok(())
    }

    fn status(&mut self) -> EngineStatus {
        // Never asks the engine: during a long round its actor cannot answer.
        if self.running().is_none() && mem::take(&mut self.engine_expected) {
            return EngineStatus::Gone {
                configured: self.configured(),
            };
        }
        self.file_status()
    }

    fn set_mode(&mut self, mode: NetMode) -> std::result::Result<StartOutcome, String> {
        let config = self.state.config().map_err(text)?;
        self.state
            .save_config(&NetConfig { mode, ..config })
            .map_err(text)?;
        if self.running().is_none() {
            return Ok(StartOutcome::Started);
        }
        self.halt();
        self.start()
    }

    fn stop(&mut self) {
        self.engine_expected = false;
        self.halt();
    }

    fn mode(&self) -> NetMode {
        self.state
            .config()
            .map_or(NetMode::Internet, |config| config.mode)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ns_sync::{NetConfig, NetMode, StateDir};

    use super::*;
    use crate::state::sync_panel::{EngineStatus, SyncControl};

    fn control_with(start: StartFn) -> (EngineControl, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let state_base = dir.path().join("state");
        let control = EngineControl::with_start(&vault, &state_base, start).unwrap();
        (control, dir)
    }

    fn control() -> (EngineControl, tempfile::TempDir) {
        control_with(refuse_to_start)
    }

    fn refuse_to_start(_root: &Path, _state: StateDir) -> Result<EngineHandle> {
        Err(SyncError::Net("no engine in this test".into()))
    }

    fn state_of(dir: &tempfile::TempDir) -> StateDir {
        let vault = dir.path().join("vault");
        StateDir::for_vault_in(&dir.path().join("state"), &vault).unwrap()
    }

    #[test]
    fn should_report_not_configured_for_fresh_state_dir() {
        let (mut control, _dir) = control();
        assert!(!control.configured());
        assert_eq!(control.status(), EngineStatus::Unknown);
        assert_eq!(control.mode(), NetMode::Internet);
    }

    #[test]
    fn should_save_mode_without_running_engine() {
        let (mut control, dir) = control();
        control.set_mode(NetMode::Lan).unwrap();
        assert_eq!(control.mode(), NetMode::Lan);
        let vault = dir.path().join("vault");
        let state = StateDir::for_vault_in(&dir.path().join("state"), &vault).unwrap();
        let expected = NetConfig {
            mode: NetMode::Lan,
            ..NetConfig::default()
        };
        assert_eq!(state.config().unwrap(), expected);
    }

    #[test]
    fn should_keep_other_config_fields_when_saving_mode() {
        let (mut control, dir) = control();
        let vault = dir.path().join("vault");
        let state = StateDir::for_vault_in(&dir.path().join("state"), &vault).unwrap();
        let custom = NetConfig {
            mode: NetMode::Internet,
            bind_port: Some(4500),
            mdns: false,
        };
        state.save_config(&custom).unwrap();
        control.set_mode(NetMode::Lan).unwrap();
        let saved = state.config().unwrap();
        assert_eq!((saved.bind_port, saved.mdns), (Some(4500), false));
    }

    #[test]
    fn should_leave_without_starting_the_engine() {
        static STARTS: AtomicUsize = AtomicUsize::new(0);
        fn counting_start(root: &Path, state: StateDir) -> Result<EngineHandle> {
            STARTS.fetch_add(1, Ordering::SeqCst);
            refuse_to_start(root, state)
        }
        let (mut control, dir) = control_with(counting_start);
        state_of(&dir).save_secret(&[7; 32]).unwrap();
        assert!(control.configured());
        control.leave().unwrap();
        assert!(!control.configured());
        assert_eq!(STARTS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn should_reject_a_bad_ticket_before_starting_the_engine() {
        static STARTS: AtomicUsize = AtomicUsize::new(0);
        fn counting_start(root: &Path, state: StateDir) -> Result<EngineHandle> {
            STARTS.fetch_add(1, Ordering::SeqCst);
            refuse_to_start(root, state)
        }
        let (mut control, _dir) = control_with(counting_start);
        let error = control.join("garbage").unwrap_err();
        assert!(error.contains("invalid ticket"), "{error}");
        assert_eq!(STARTS.load(Ordering::SeqCst), 0);
        assert!(!control.configured());
    }

    #[test]
    fn should_report_the_engine_gone_once_when_it_ended_by_itself() {
        let (mut control, dir) = control();
        control.engine_expected = true;
        assert_eq!(control.status(), EngineStatus::Gone { configured: false });
        assert_eq!(control.status(), EngineStatus::Unknown);
        state_of(&dir).save_secret(&[7; 32]).unwrap();
        control.engine_expected = true;
        assert_eq!(control.status(), EngineStatus::Gone { configured: true });
    }

    #[test]
    fn should_report_the_start_error_and_expect_no_engine_when_an_operation_cannot_start_it() {
        // The branch that stops an engine started for a failing operation needs a real
        // `EngineHandle`, which no test may start; this covers the failing start.
        let (mut control, _dir) = control();
        let error = control.invite().unwrap_err();
        assert!(error.contains("no engine in this test"), "{error}");
        assert_eq!(control.status(), EngineStatus::Unknown);
    }

    fn fresh_status(pid: u32) -> ns_sync::Status {
        ns_sync::Status {
            pid,
            busy: true,
            notices: vec!["a standing condition".into()],
            updated_unix: unix_now(),
            ..ns_sync::Status::default()
        }
    }

    #[test]
    fn should_read_the_status_from_the_file_without_waiting_for_an_engine() {
        let (mut control, dir) = control();
        let state = state_of(&dir);
        state.write_status(&fresh_status(7)).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(control.status(), EngineStatus::Running(fresh_status(7)));
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        std::fs::remove_file(state.file("status.json")).unwrap();
        assert_eq!(control.status(), EngineStatus::Unknown);
    }

    #[test]
    fn should_keep_the_last_status_when_the_file_cannot_be_read() {
        let (mut control, dir) = control();
        let state = state_of(&dir);
        state.write_status(&fresh_status(7)).unwrap();
        assert_eq!(control.status(), EngineStatus::Running(fresh_status(7)));
        std::fs::write(state.file("status.json"), "{ not json").unwrap();
        assert_eq!(control.status(), EngineStatus::Running(fresh_status(7)));
    }

    #[test]
    fn should_stop_showing_the_last_status_once_it_is_older_than_the_freshness_limit() {
        let (mut control, dir) = control();
        std::fs::write(state_of(&dir).file("status.json"), "{ not json").unwrap();
        let aged = |secs: u64| ns_sync::Status {
            updated_unix: unix_now() - secs,
            ..fresh_status(7)
        };
        control.last_status = Some(aged(control::STATUS_MAX_AGE_SECS - 2));
        let shown = control.status();
        assert!(matches!(shown, EngineStatus::Running(_)), "{shown:?}");
        control.last_status = Some(aged(control::STATUS_MAX_AGE_SECS + 2));
        assert_eq!(control.status(), EngineStatus::Unknown);
    }

    #[test]
    fn should_report_unknown_when_the_first_read_of_the_file_fails() {
        let (mut control, dir) = control();
        std::fs::write(state_of(&dir).file("status.json"), "{ not json").unwrap();
        assert_eq!(control.status(), EngineStatus::Unknown);
    }

    #[test]
    fn should_not_report_gone_after_an_explicit_stop() {
        let (mut control, _dir) = control();
        control.engine_expected = true;
        control.stop();
        assert_eq!(control.status(), EngineStatus::Unknown);
    }
}
