#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! End-to-end acceptance of milestone M3 (spec 11.2): every test starts on the welcome screen
//! and drives the real UI code headless.

#[allow(dead_code)] // this file uses only part of the shared helpers
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::App;
use eframe::egui::{Context, Event, Key, Modifiers, PointerButton, ViewportEvent, ViewportId};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::{AppOptions, SecondBrainApp};
use sb_app::state::sync_panel::{EngineStatus, StartOutcome, SyncControl};
use sb_sync::NetMode;
use tempfile::TempDir;

use common::{harness, harness_for, run_until};

const EDITOR: &str = "Write your note";
const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    harness: Harness<'static, SecondBrainApp>,
    vault: PathBuf,
    settings: PathBuf,
    _dirs: (TempDir, TempDir),
}

fn make_vault(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    for (name, text) in files {
        fs::write(vault.join(name), text).unwrap();
    }
    (scratch, vault)
}

/// Types the vault path on the welcome screen, opens it and waits for the vault screen.
fn open_from_welcome(harness: &mut Harness<'static, SecondBrainApp>, vault: &Path) {
    assert_eq!(harness.state().screen_name(), "welcome");
    type_into(harness, "Vault folder path", &vault.to_string_lossy());
    harness.get_by_label("Open").click();
    run_until(harness, "vault screen", |app| app.screen_name() == "vault");
    harness.run();
}

fn fixture(files: &[(&str, &str)]) -> Fixture {
    let (scratch, vault) = make_vault(files);
    let (mut harness, dir) = harness(None);
    open_from_welcome(&mut harness, &vault);
    let settings = dir.path().join("app.json");
    Fixture {
        harness,
        vault,
        settings,
        _dirs: (scratch, dir),
    }
}

fn type_into(harness: &mut Harness<'static, SecondBrainApp>, field: &str, text: &str) {
    harness.get_by_label(field).focus();
    harness.run();
    harness.get_by_label(field).type_text(text);
    harness.run();
}

fn press_command(harness: &mut Harness<'static, SecondBrainApp>, key: Key) {
    harness.key_press_modifiers(Modifiers::COMMAND, key);
    harness.run();
}

fn click_label(harness: &mut Harness<'static, SecondBrainApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn open_path(harness: &Harness<'static, SecondBrainApp>) -> Option<String> {
    let workspace = harness.state().workspace()?;
    Some(workspace.open_note()?.path.to_string())
}

fn wait_until(what: &str, done: impl Fn() -> bool, harness: &mut Harness<SecondBrainApp>) {
    let deadline = Instant::now() + DEADLINE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn conflict_copies(vault: &Path) -> Vec<String> {
    let names = fs::read_dir(vault)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned());
    names.filter(|name| name.contains("(conflict ")).collect()
}

fn click_at(harness: &Harness<'static, SecondBrainApp>, pos: eframe::egui::Pos2) {
    harness.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
}

fn create_note(harness: &mut Harness<'static, SecondBrainApp>, name: &str) {
    press_command(harness, Key::N);
    type_into(harness, "Note name", name);
    harness.key_press(Key::Enter);
    harness.run();
}

#[test]
fn should_complete_note_lifecycle_through_the_ui() {
    let mut f = fixture(&[]);
    create_note(&mut f.harness, "first");
    assert_eq!(open_path(&f.harness).as_deref(), Some("first.md"));
    f.harness.get_by_label(EDITOR).type_text("Hello [[second]]");
    f.harness.run();
    let first = f.vault.join("first.md");
    wait_until(
        "the autosave",
        || fs::read_to_string(&first).is_ok_and(|text| text == "Hello [[second]]"),
        &mut f.harness,
    );

    click_label(&mut f.harness, "Toggle inspector");
    press_command(&mut f.harness, Key::E);
    click_label(&mut f.harness, "second");
    assert!(f.vault.join("second.md").is_file());
    assert_eq!(open_path(&f.harness).as_deref(), Some("second.md"));

    f.harness.get_by_label("second").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "Rename…");
    type_into(&mut f.harness, "New name", "renamed");
    f.harness.key_press(Key::Enter);
    f.harness.run();
    assert!(f.vault.join("renamed.md").is_file());
    assert!(!f.vault.join("second.md").exists());

    f.harness.get_by_label("renamed").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "Delete");
    assert!(!f.vault.join("renamed.md").exists());
    let trash = f.vault.join(".secondbrain/trash");
    assert_eq!(fs::read_dir(trash).unwrap().count(), 1);
}

#[test]
fn should_find_note_by_search_and_graph() {
    let mut f = fixture(&[("first.md", "Hello world"), ("second.md", "Other")]);
    type_into(&mut f.harness, "Search notes", "hello");
    let deadline = Instant::now() + DEADLINE;
    while f.harness.query_by_label_contains("Hello world").is_none() {
        assert!(Instant::now() < deadline, "no search result appeared");
        std::thread::sleep(Duration::from_millis(20));
        f.harness.step();
    }
    f.harness.get_by_label_contains("Hello world").click();
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("first.md"));

    press_command(&mut f.harness, Key::O);
    type_into(&mut f.harness, "Find a note", "second");
    f.harness.key_press(Key::Enter);
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("second.md"));

    press_command(&mut f.harness, Key::E);
    assert!(f.harness.state().is_previewing());
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::G);
    run_until(&mut f.harness, "the graph to settle", |app| {
        app.graph_shown() && app.graph_settled()
    });
    let rect = f.harness.state().graph_node_rect("first.md").unwrap();
    click_at(&f.harness, rect.center());
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("first.md"));
    assert!(!f.harness.state().graph_shown());
    assert!(!f.harness.state().is_previewing());
    f.harness.get_by_label(EDITOR);
}

#[test]
fn should_show_external_edit_while_open() {
    let mut f = fixture(&[("first.md", "before")]);
    click_label(&mut f.harness, "first");
    assert_eq!(open_path(&f.harness).as_deref(), Some("first.md"));
    fs::write(f.vault.join("first.md"), "after").unwrap();
    run_until(&mut f.harness, "the editor to adopt the file", |app| {
        app.workspace().unwrap().open_note().unwrap().buffer() == "after"
    });
    f.harness.run();
    let editor = f.harness.get_by_label(EDITOR);
    assert_eq!(editor.value().as_deref(), Some("after"));
}

struct QuietControl(Arc<AtomicBool>);

impl SyncControl for QuietControl {
    fn configured(&self) -> bool {
        false
    }
    fn start(&mut self) -> Result<StartOutcome, String> {
        Ok(StartOutcome::Started)
    }
    fn invite(&mut self) -> Result<String, String> {
        Ok("sb1/FAKE-TICKET".to_owned())
    }
    fn join(&mut self, _ticket: &str) -> Result<(), String> {
        Ok(())
    }
    fn leave(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn status(&mut self) -> EngineStatus {
        EngineStatus::Unknown
    }
    fn set_mode(&mut self, _mode: NetMode) -> Result<StartOutcome, String> {
        Ok(StartOutcome::Started)
    }
    fn stop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn mode(&self) -> NetMode {
        NetMode::Internet
    }
}

/// A harness on the welcome screen whose first vault gets a fake sync control.
fn fixture_with_fake_sync(stopped: &Arc<AtomicBool>) -> Fixture {
    let (scratch, vault) = make_vault(&[("first.md", "Hello")]);
    let dir = tempfile::tempdir().unwrap();
    let options = AppOptions {
        settings_path: dir.path().join("app.json"),
        state_base: dir.path().join("state"),
        vault: None,
    };
    let control = Box::new(QuietControl(stopped.clone()));
    let app = SecondBrainApp::with_sync_control(&Context::default(), options, control);
    let mut harness = harness_for(app);
    open_from_welcome(&mut harness, &vault);
    let settings = dir.path().join("app.json");
    Fixture {
        harness,
        vault,
        settings,
        _dirs: (scratch, dir),
    }
}

#[test]
fn should_open_sync_window() {
    let mut f = fixture_with_fake_sync(&Arc::new(AtomicBool::new(false)));
    f.harness.get_by_label("Sync").click();
    f.harness.run();
    f.harness.get_by_label("Sync is off");
    f.harness.get_by_label("Create invite");
}

#[test]
fn should_save_open_note_on_exit() {
    let mut f = fixture(&[("first.md", "Hello")]);
    click_label(&mut f.harness, "first");
    f.harness.get_by_label(EDITOR).type_text(" there");
    f.harness.run();
    f.harness.state_mut().on_exit(None);
    assert_eq!(
        fs::read_to_string(f.vault.join("first.md")).unwrap(),
        "Hello there"
    );
}

#[test]
fn should_keep_unsaved_text_in_conflict_note_on_exit_when_the_file_changed() {
    let mut f = fixture(&[("first.md", "Hello")]);
    click_label(&mut f.harness, "first");
    f.harness.get_by_label(EDITOR).type_text(" mine");
    f.harness.run();
    fs::write(f.vault.join("first.md"), "theirs").unwrap();
    f.harness.state_mut().on_exit(None);
    let copies = conflict_copies(&f.vault);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let kept = fs::read_to_string(f.vault.join(&copies[0])).unwrap();
    assert_eq!(kept, "Hello mine");
    assert_eq!(
        fs::read_to_string(f.vault.join("first.md")).unwrap(),
        "theirs"
    );
}

#[test]
fn should_write_one_conflict_note_when_the_window_closes_and_the_app_exits() {
    let mut f = fixture(&[("first.md", "Hello")]);
    click_label(&mut f.harness, "first");
    f.harness.get_by_label(EDITOR).type_text(" mine");
    f.harness.run();
    fs::write(f.vault.join("first.md"), "theirs").unwrap();
    let root = f.harness.input_mut().viewports.get_mut(&ViewportId::ROOT);
    root.unwrap().events.push(ViewportEvent::Close);
    f.harness.step();
    f.harness.state_mut().on_exit(None);
    let copies = conflict_copies(&f.vault);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let kept = fs::read_to_string(f.vault.join(&copies[0])).unwrap();
    assert_eq!(kept, "Hello mine");
}

#[test]
fn should_save_settings_on_exit() {
    let mut f = fixture(&[("first.md", "Hello")]);
    fs::remove_file(&f.settings).unwrap();
    f.harness.state_mut().on_exit(None);
    assert!(f.settings.is_file(), "the settings were not written");
}

#[test]
fn should_stop_sync_engine_on_exit() {
    let stopped = Arc::new(AtomicBool::new(false));
    let mut f = fixture_with_fake_sync(&stopped);
    f.harness.state_mut().on_exit(None);
    assert!(stopped.load(Ordering::SeqCst), "the engine was not stopped");
}

fn leave_vault(harness: &mut Harness<'static, SecondBrainApp>) {
    click_label(harness, "Menu");
    click_label(harness, "Open another vault");
}

#[test]
fn should_save_settings_and_keep_unsaved_text_when_opening_another_vault() {
    let mut f = fixture(&[("first.md", "Hello")]);
    click_label(&mut f.harness, "first");
    f.harness.get_by_label(EDITOR).type_text(" mine");
    f.harness.run();
    fs::write(f.vault.join("first.md"), "theirs").unwrap();
    fs::remove_file(&f.settings).unwrap();
    leave_vault(&mut f.harness);
    assert_eq!(f.harness.state().screen_name(), "welcome");
    let copies = conflict_copies(&f.vault);
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(
        fs::read_to_string(f.vault.join(&copies[0])).unwrap(),
        "Hello mine"
    );
    assert!(f.settings.is_file(), "the settings were not written");
}
