//! The production `SyncControl`: a thin adapter from the sync window to the M2 engine.
//!
//! Every method may block (starting the network, asking the engine, stopping it), so it is only
//! called from the `SyncWorker` thread, which is a plain thread, never a tokio one.

use std::path::{Path, PathBuf};

use sb_sync::{
    EngineHandle, NetConfig, NetMode, Result, StateDir, Status, SyncError, control, start_engine,
    unix_now,
};

use crate::state::sync_panel::{StartOutcome, SyncControl};

/// The sync engine of one vault, started on demand.
pub struct EngineControl {
    vault_root: PathBuf,
    state: StateDir,
    handle: Option<EngineHandle>,
}

impl EngineControl {
    /// Control of the engine of `vault_root`, whose state lives under `state_base`. Starts
    /// nothing and touches no network.
    ///
    /// # Errors
    /// The state directory cannot be created or the vault path cannot be resolved.
    pub fn new(vault_root: &Path, state_base: &Path) -> Result<EngineControl> {
        Ok(EngineControl {
            vault_root: vault_root.to_path_buf(),
            state: StateDir::for_vault_in(state_base, vault_root)?,
            handle: None,
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
            let started = start_engine(&self.vault_root, self.state.clone(), None)?;
            return Ok(self.handle.insert(started));
        }
        self.handle.as_ref().ok_or(SyncError::NotConfigured)
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
            Err(SyncError::AlreadyRunning) => Ok(StartOutcome::HandledElsewhere),
            Err(error) => Err(text(error)),
        }
    }

    fn invite(&mut self) -> std::result::Result<String, String> {
        self.engine().and_then(EngineHandle::invite).map_err(text)
    }

    fn join(&mut self, ticket: &str) -> std::result::Result<(), String> {
        self.engine()
            .and_then(|engine| engine.join(ticket))
            .map_err(text)
    }

    fn leave(&mut self) -> std::result::Result<(), String> {
        let left = self.engine().and_then(EngineHandle::leave).map_err(text);
        self.stop();
        left
    }

    fn status(&mut self) -> Option<Status> {
        match self.running() {
            Some(engine) => engine.status().ok(),
            None => control::status(&self.state, unix_now()).ok().flatten(),
        }
    }

    fn set_mode(&mut self, mode: NetMode) -> std::result::Result<(), String> {
        let config = self.state.config().map_err(text)?;
        self.state
            .save_config(&NetConfig { mode, ..config })
            .map_err(text)?;
        if self.running().is_none() {
            return Ok(());
        }
        self.stop();
        self.start().map(drop)
    }

    fn stop(&mut self) {
        if let Some(engine) = self.handle.take() {
            // A panicked engine thread has nothing left to stop.
            let _ = engine.shutdown();
        }
    }

    fn mode(&self) -> NetMode {
        self.state
            .config()
            .map_or(NetMode::Internet, |config| config.mode)
    }
}

#[cfg(test)]
mod tests {
    use sb_sync::{NetConfig, NetMode, StateDir};

    use super::*;
    use crate::state::sync_panel::SyncControl;

    fn control() -> (EngineControl, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let control = EngineControl::new(&vault, &dir.path().join("state")).unwrap();
        (control, dir)
    }

    #[test]
    fn should_report_not_configured_for_fresh_state_dir() {
        let (mut control, _dir) = control();
        assert!(!control.configured());
        assert_eq!(control.status(), None);
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
}
