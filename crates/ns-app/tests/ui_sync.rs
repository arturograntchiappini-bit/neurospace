#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

#[allow(dead_code)] // this file uses only part of the shared helpers
mod common;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{Context, Key, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ns_app::app::{AppOptions, NeurospaceApp};
use ns_app::state::settings::Settings;
use ns_app::state::sync_panel::{EngineStatus, StartOutcome, SyncControl};
use ns_sync::{BlockedNote, NetMode, PeerStatus, Status};
use tempfile::TempDir;

use common::{harness_for, run_until};

const TICKET: &str = "ns1/FAKE-TICKET";
const DEADLINE: Duration = Duration::from_secs(10);

struct FakeControl {
    configured: bool,
    start: StartOutcome,
    start_error: Option<&'static str>,
    invite_delay: Duration,
    status: Option<Status>,
    mode: NetMode,
}

impl FakeControl {
    fn off() -> FakeControl {
        FakeControl {
            configured: false,
            start: StartOutcome::Started,
            start_error: None,
            invite_delay: Duration::ZERO,
            status: None,
            mode: NetMode::Internet,
        }
    }
}

impl SyncControl for FakeControl {
    fn configured(&self) -> bool {
        self.configured
    }

    fn start(&mut self) -> Result<StartOutcome, String> {
        self.start_error
            .map_or(Ok(self.start), |error| Err(error.to_owned()))
    }

    fn invite(&mut self) -> Result<String, String> {
        thread::sleep(self.invite_delay);
        Ok(TICKET.to_owned())
    }

    fn join(&mut self, _ticket: &str) -> Result<(), String> {
        Ok(())
    }

    fn leave(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn status(&mut self) -> EngineStatus {
        self.status
            .clone()
            .map_or(EngineStatus::Unknown, EngineStatus::Running)
    }

    fn set_mode(&mut self, mode: NetMode) -> Result<StartOutcome, String> {
        self.mode = mode;
        Ok(StartOutcome::Started)
    }

    fn stop(&mut self) {}

    fn mode(&self) -> NetMode {
        self.mode
    }
}

fn status() -> Status {
    let peer = |digit: char, connected| PeerStatus {
        id: digit.to_string().repeat(64),
        connected,
    };
    Status {
        device_id: "d".repeat(64),
        peers: vec![peer('a', true), peer('b', false)],
        blocked: vec![BlockedNote {
            path: "Broken.md".to_owned(),
            reason: "not valid UTF-8".to_owned(),
        }],
        relay: Some("https://relay.example".to_owned()),
        ..Status::default()
    }
}

/// A harness on a vault whose sync control is `control`; the directories must outlive it.
fn open_with(control: FakeControl) -> (Harness<'static, NeurospaceApp>, TempDir) {
    open_with_settings(control, Settings::default())
}

fn open_with_settings(
    control: FakeControl,
    settings: Settings,
) -> (Harness<'static, NeurospaceApp>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    settings.save(&dir.path().join("app.json")).unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("Alpha.md"), "# Alpha\n").unwrap();
    let options = AppOptions {
        settings_path: dir.path().join("app.json"),
        state_base: dir.path().join("state"),
        vault: Some(vault),
    };
    let app = NeurospaceApp::with_sync_control(&Context::default(), options, Box::new(control));
    let mut harness = harness_for(app);
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    (harness, dir)
}

/// Steps until a label exists; fails after 10 s.
fn wait_for_label(harness: &mut Harness<'static, NeurospaceApp>, label: &str) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        harness.step();
        if harness.query_by_label(label).is_some() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn open_window(harness: &mut Harness<'static, NeurospaceApp>) {
    harness.get_by_role_and_label(Role::Button, "Sync").click();
    harness.run();
}

#[test]
fn should_open_sync_window_and_show_sync_off() {
    let (mut harness, _dir) = open_with(FakeControl::off());
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Sync is off");
    harness.get_by_label("Create invite");
    harness.get_by_label("Paste a ticket");
    harness.get_by_label("Internet");
    harness.get_by_label("LAN only");
    assert!(harness.query_by_label("Turn sync off").is_none());
}

#[test]
fn should_close_sync_window_with_escape() {
    let (mut harness, _dir) = open_with(FakeControl::off());
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Create invite");
    harness.key_press(Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Create invite").is_none());
}

#[test]
fn should_show_ticket_after_create_invite() {
    let (mut harness, _dir) = open_with(FakeControl::off());
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Create invite").click();
    wait_for_label(&mut harness, "Copy ticket");
    let field = harness.get_by_label("Invite ticket");
    assert_eq!(field.accesskit_node().value().as_deref(), Some(TICKET));
    harness.get_by_label("Anyone with this ticket can join your notes. Share it privately.");
    harness.get_by_label("Turn sync off");
}

#[test]
fn should_disable_buttons_while_busy() {
    let mut control = FakeControl::off();
    control.invite_delay = Duration::from_secs(4);
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Create invite").click();
    wait_for_label(&mut harness, "Creating invite…");
    let disabled = |harness: &Harness<'static, NeurospaceApp>, label: &str| {
        let node = harness.get_by_label(label);
        node.accesskit_node().is_disabled()
    };
    assert!(disabled(&harness, "Create invite"));
    assert!(disabled(&harness, "LAN only"));
    wait_for_label(&mut harness, "Copy ticket");
    harness.step();
    assert!(!disabled(&harness, "Create invite"));
    assert!(harness.query_by_label("Creating invite…").is_none());
}

#[test]
fn should_show_read_only_status_when_handled_elsewhere() {
    let mut control = FakeControl::off();
    control.configured = true;
    control.start = StartOutcome::HandledElsewhere;
    control.status = Some(status());
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync handled by another process");
    open_window(&mut harness);
    wait_for_label(&mut harness, "Relay: https://relay.example");
    harness.get_by_label("Sync is handled by another process (for example `ns sync run`).");
    harness.get_by_label("online");
    harness.get_by_label("offline");
    harness.get_by_label("Broken.md: blocked (not valid UTF-8)");
    for hidden in [
        "Create invite",
        "Paste a ticket",
        "Join",
        "Turn sync off",
        "LAN only",
    ] {
        assert!(
            harness.query_by_label(hidden).is_none(),
            "{hidden} is shown"
        );
    }
}

#[test]
fn should_offer_turn_sync_off_when_the_engine_failed_to_start() {
    let mut control = FakeControl::off();
    control.configured = true;
    control.start_error = Some("state is corrupt");
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync error");
    open_window(&mut harness);
    harness.get_by_label("Sync is not running: state is corrupt");
    harness.get_by_label("Turn sync off").click();
    wait_for_label(&mut harness, "Sync off");
}

#[test]
fn should_summarise_peers_in_the_status_strip_when_sync_runs() {
    let mut control = FakeControl::off();
    control.configured = true;
    control.status = Some(status());
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync: 1 of 2 peers online");
}

#[test]
fn should_not_animate_a_spinner_when_reduce_motion_is_on() {
    let mut control = FakeControl::off();
    control.invite_delay = Duration::from_secs(2);
    let settings = Settings {
        reduce_motion: true,
        ..Settings::default()
    };
    let (mut harness, _dir) = open_with_settings(control, settings);
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Create invite").click();
    wait_for_label(&mut harness, "Creating invite…");
    // `run` fails when the page keeps asking for frames, as a spinning spinner does.
    harness.run();
    harness.get_by_label("Creating invite…");
}
