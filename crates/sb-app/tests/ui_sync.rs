#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

#[allow(dead_code)] // this file uses only part of the shared helpers
mod common;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{Context, Key, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use sb_app::app::{AppOptions, SecondBrainApp};
use sb_app::state::sync_panel::{StartOutcome, SyncControl};
use sb_sync::{BlockedNote, NetMode, PeerStatus, Status};
use tempfile::TempDir;

use common::{harness_for, run_until};

const TICKET: &str = "sb1/FAKE-TICKET";
const DEADLINE: Duration = Duration::from_secs(10);

struct FakeControl {
    configured: bool,
    start: StartOutcome,
    invite_delay: Duration,
    status: Option<Status>,
    mode: NetMode,
}

impl FakeControl {
    fn off() -> FakeControl {
        FakeControl {
            configured: false,
            start: StartOutcome::Started,
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
        Ok(self.start)
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

    fn status(&mut self) -> Option<Status> {
        self.status.clone()
    }

    fn set_mode(&mut self, mode: NetMode) -> Result<(), String> {
        self.mode = mode;
        Ok(())
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
fn open_with(control: FakeControl) -> (Harness<'static, SecondBrainApp>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("Alpha.md"), "# Alpha\n").unwrap();
    let options = AppOptions {
        settings_path: dir.path().join("app.json"),
        state_base: dir.path().join("state"),
        vault: Some(vault),
    };
    let app = SecondBrainApp::with_sync_control(&Context::default(), options, Box::new(control));
    let mut harness = harness_for(app);
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    (harness, dir)
}

/// Steps until a label exists; fails after 10 s.
fn wait_for_label(harness: &mut Harness<'static, SecondBrainApp>, label: &str) {
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

fn open_window(harness: &mut Harness<'static, SecondBrainApp>) {
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
    control.invite_delay = Duration::from_millis(1500);
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync off");
    open_window(&mut harness);
    harness.get_by_label("Create invite").click();
    harness.step();
    harness.step();
    harness.get_by_label("Creating invite…");
    let disabled = |harness: &Harness<'static, SecondBrainApp>, label: &str| {
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
    harness.get_by_label("Sync is handled by another process (for example `sb sync run`).");
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
fn should_summarise_peers_in_the_status_strip_when_sync_runs() {
    let mut control = FakeControl::off();
    control.configured = true;
    control.status = Some(status());
    let (mut harness, _dir) = open_with(control);
    wait_for_label(&mut harness, "Sync: 1 of 2 peers online");
}
