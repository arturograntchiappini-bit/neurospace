//! The production `SyncControl`: a thin adapter from the sync window to the M2 engine.
//!
//! Every method may block (starting the network, asking the engine, stopping it), so it is only
//! called from the `SyncWorker` thread, which is a plain thread, never a tokio one.

use std::path::{Path, PathBuf};

use ns_sync::{
    EngineHandle, NetConfig, NetMode, Result, StateDir, SyncError, Ticket, control, start_engine,
    unix_now,
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
        if let Some(engine) = self.running() {
            return engine
                .status()
                .map_or(EngineStatus::Unknown, EngineStatus::Running);
        }
        if self.engine_expected {
            self.engine_expected = false;
            return EngineStatus::Gone {
                configured: self.configured(),
            };
        }
        match control::status(&self.state, unix_now()) {
            Ok(Some(status)) => EngineStatus::Running(status),
            _ => EngineStatus::Unknown,
        }
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

    #[test]
    fn should_not_report_gone_after_an_explicit_stop() {
        let (mut control, _dir) = control();
        control.engine_expected = true;
        control.stop();
        assert_eq!(control.status(), EngineStatus::Unknown);
    }
}
