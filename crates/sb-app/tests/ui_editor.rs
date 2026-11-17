#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::App;
use eframe::egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::SecondBrainApp;
use tempfile::TempDir;

use common::{harness, run_until};

const EDITOR: &str = "Write your note";
const QUICK_OPEN_FIELD: &str = "Find a note";

fn make_vault(parent: &Path) -> PathBuf {
    let vault = parent.join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("a.md"), "# A\n\n[[b]] [[new one]] ![[p.png]]").unwrap();
    fs::write(vault.join("b.md"), "B").unwrap();
    fs::write(vault.join("c.md"), "[[bad:name]]").unwrap();
    vault
}

struct Fixture {
    harness: Harness<'static, SecondBrainApp>,
    vault: PathBuf,
    _dirs: (TempDir, TempDir),
}

fn open_fixture() -> Fixture {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let (mut harness, dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    Fixture {
        harness,
        vault,
        _dirs: (scratch, dir),
    }
}

/// A fixture with `a.md` open and both side panels closed, so a link text is unique on screen.
fn open_note_fixture(name: &str) -> Fixture {
    let mut f = open_fixture();
    f.harness.get_by_label(name).click();
    f.harness.run();
    f.harness.get_by_label("Toggle sidebar").click();
    f.harness.get_by_label("Toggle inspector").click();
    f.harness.run();
    f
}

fn open_path(harness: &Harness<'static, SecondBrainApp>) -> Option<String> {
    let workspace = harness.state().workspace()?;
    Some(workspace.open_note()?.path.to_string())
}

fn press_command(harness: &mut Harness<'static, SecondBrainApp>, key: Key) {
    harness.key_press_modifiers(Modifiers::COMMAND, key);
    harness.run();
}

fn type_into(harness: &mut Harness<'static, SecondBrainApp>, field: &str, text: &str) {
    harness.get_by_label(field).focus();
    harness.run();
    harness.get_by_label(field).type_text(text);
    harness.run();
}

fn conflict_copies(vault: &Path) -> Vec<String> {
    let names = fs::read_dir(vault)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned());
    names.filter(|name| name.contains("(conflict ")).collect()
}

#[test]
fn should_autosave_typed_text() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " more");
    let on_disk = || fs::read_to_string(f.vault.join("a.md")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !on_disk().ends_with(" more") {
        assert!(
            Instant::now() < deadline,
            "the typed text never reached disk"
        );
        f.harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(on_disk(), "# A\n\n[[b]] [[new one]] ![[p.png]] more");
}

#[test]
fn should_save_when_window_loses_focus() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " more");
    f.harness.input_mut().focused = false;
    f.harness.run();
    let text = fs::read_to_string(f.vault.join("a.md")).unwrap();
    assert!(text.ends_with(" more"), "{text:?}");
}

#[test]
fn should_save_typed_text_when_the_application_exits() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " more");
    f.harness.state_mut().on_exit(None);
    let text = fs::read_to_string(f.vault.join("a.md")).unwrap();
    assert!(text.ends_with(" more"), "{text:?}");
}

#[test]
fn should_toggle_preview_with_ctrl_e() {
    let mut f = open_note_fixture("a");
    assert!(!f.harness.state().is_previewing());
    f.harness.get_by_label(EDITOR);
    f.harness.get_by_label("Preview");
    press_command(&mut f.harness, Key::E);
    assert!(f.harness.state().is_previewing());
    assert!(f.harness.query_by_label(EDITOR).is_none());
    f.harness.get_by_label("Edit");
    press_command(&mut f.harness, Key::E);
    assert!(!f.harness.state().is_previewing());
    f.harness.get_by_label(EDITOR);
}

#[test]
fn should_toggle_graph_flag_with_ctrl_g() {
    let mut f = open_fixture();
    assert!(!f.harness.state().graph_shown());
    press_command(&mut f.harness, Key::G);
    assert!(f.harness.state().graph_shown());
    f.harness.get_by_label("Graph").click();
    f.harness.run();
    assert!(!f.harness.state().graph_shown());
}

#[test]
fn should_open_note_when_link_clicked_in_preview() {
    let mut f = open_note_fixture("a");
    press_command(&mut f.harness, Key::E);
    f.harness.get_by_label("b").click();
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
    assert!(f.harness.state().is_previewing());
}

#[test]
fn should_create_note_when_unresolved_link_clicked() {
    let mut f = open_note_fixture("a");
    press_command(&mut f.harness, Key::E);
    f.harness.get_by_label("new one").click();
    f.harness.run();
    assert!(f.vault.join("new one.md").is_file());
    assert_eq!(open_path(&f.harness).as_deref(), Some("new one.md"));
}

#[test]
fn should_not_create_attachment_note() {
    let mut f = open_note_fixture("a");
    press_command(&mut f.harness, Key::E);
    f.harness.get_by_label_contains("p.png").click();
    f.harness.run();
    assert!(!f.vault.join("p.png.md").exists());
    assert!(!f.vault.join("p.png").exists());
    assert_eq!(open_path(&f.harness).as_deref(), Some("a.md"));
}

#[test]
fn should_show_error_in_strip_when_link_cannot_be_followed() {
    let mut f = open_note_fixture("c");
    press_command(&mut f.harness, Key::E);
    f.harness.get_by_label("bad:name").click();
    f.harness.run();
    f.harness.get_by_label_contains("Error:");
    assert_eq!(open_path(&f.harness).as_deref(), Some("c.md"));
    assert_eq!(fs::read_dir(&f.vault).unwrap().count(), 3);
}

#[test]
fn should_show_external_edit_of_open_note() {
    let mut f = open_note_fixture("a");
    fs::write(f.vault.join("a.md"), "changed").unwrap();
    run_until(&mut f.harness, "the editor to adopt the file", |app| {
        let workspace = app.workspace().unwrap();
        workspace.open_note().unwrap().buffer == "changed"
    });
    f.harness.run();
    let editor = f.harness.get_by_label(EDITOR);
    assert_eq!(editor.value().as_deref(), Some("changed"));
}

#[test]
fn should_keep_typed_text_in_conflict_note_when_file_changed_under_it() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " mine");
    fs::write(f.vault.join("a.md"), "theirs").unwrap();
    run_until(&mut f.harness, "the conflict copy", |_| {
        !conflict_copies(&f.vault).is_empty()
    });
    let copies = conflict_copies(&f.vault);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let kept = fs::read_to_string(f.vault.join(&copies[0])).unwrap();
    assert_eq!(kept, "# A\n\n[[b]] [[new one]] ![[p.png]] mine");
    run_until(&mut f.harness, "the editor to show the file", |app| {
        app.workspace().unwrap().open_note().unwrap().buffer == "theirs"
    });
    assert_eq!(fs::read_to_string(f.vault.join("a.md")).unwrap(), "theirs");
}

#[test]
fn should_open_first_match_with_quick_open() {
    let mut f = open_fixture();
    press_command(&mut f.harness, Key::O);
    type_into(&mut f.harness, QUICK_OPEN_FIELD, "b");
    f.harness.key_press(Key::Enter);
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
    assert!(f.harness.query_by_label(QUICK_OPEN_FIELD).is_none());
}

#[test]
fn should_open_highlighted_match_after_arrow_down() {
    let mut f = open_fixture();
    press_command(&mut f.harness, Key::O);
    type_into(&mut f.harness, QUICK_OPEN_FIELD, "md");
    f.harness.key_press(Key::ArrowDown);
    f.harness.run();
    f.harness.key_press(Key::Enter);
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
}

#[test]
fn should_close_quick_open_with_escape_without_changing_the_note() {
    let mut f = open_note_fixture("a");
    press_command(&mut f.harness, Key::O);
    type_into(&mut f.harness, QUICK_OPEN_FIELD, "b");
    f.harness.key_press(Key::Escape);
    f.harness.run();
    assert!(f.harness.query_by_label(QUICK_OPEN_FIELD).is_none());
    assert_eq!(open_path(&f.harness).as_deref(), Some("a.md"));
}

#[test]
fn should_open_quick_open_from_the_top_bar() {
    let mut f = open_fixture();
    f.harness.get_by_label("Quick open").click();
    f.harness.run();
    f.harness.get_by_label(QUICK_OPEN_FIELD);
}
