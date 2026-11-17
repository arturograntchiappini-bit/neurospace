#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::SecondBrainApp;
use tempfile::TempDir;

use common::{harness, run_until};

fn make_vault(parent: &Path) -> PathBuf {
    let vault = parent.join("vault");
    fs::create_dir_all(vault.join("sub")).unwrap();
    fs::write(vault.join("a.md"), "Link to [[b]] and [[ghost]] #tag").unwrap();
    fs::write(vault.join("b.md"), "Back to [[a]]").unwrap();
    fs::write(vault.join("sub/c.md"), "Deep note").unwrap();
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

fn open_path(harness: &Harness<'static, SecondBrainApp>) -> Option<String> {
    let workspace = harness.state().workspace()?;
    Some(workspace.open_note()?.path.to_string())
}

fn click_label(harness: &mut Harness<'static, SecondBrainApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn type_into(harness: &mut Harness<'static, SecondBrainApp>, field: &str, text: &str) {
    harness.get_by_label(field).focus();
    harness.run();
    harness.get_by_label(field).type_text(text);
    harness.run();
}

fn press_enter(harness: &mut Harness<'static, SecondBrainApp>) {
    harness.key_press(Key::Enter);
    harness.run();
}

#[test]
fn should_open_note_when_clicked_in_tree() {
    let mut f = open_fixture();
    click_label(&mut f.harness, "a");
    assert_eq!(open_path(&f.harness).as_deref(), Some("a.md"));
}

#[test]
fn should_create_note_with_ctrl_n() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    type_into(&mut f.harness, "Note name", "fresh");
    press_enter(&mut f.harness);
    assert!(f.vault.join("fresh.md").is_file());
    assert_eq!(open_path(&f.harness).as_deref(), Some("fresh.md"));
    assert!(f.harness.query_by_label("Note name").is_none());
}

#[test]
fn should_show_error_in_dialog_when_name_taken() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    type_into(&mut f.harness, "Note name", "a");
    press_enter(&mut f.harness);
    f.harness.get_by_label("Note name");
    f.harness.get_by_label_contains("already exists");
    let text = fs::read_to_string(f.vault.join("a.md")).unwrap();
    assert_eq!(text, "Link to [[b]] and [[ghost]] #tag");
}

#[test]
fn should_close_dialog_on_escape_without_creating_anything() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    type_into(&mut f.harness, "Note name", "never");
    f.harness.key_press(Key::Escape);
    f.harness.run();
    assert!(f.harness.query_by_label("Note name").is_none());
    assert!(!f.vault.join("never.md").exists());
}

#[test]
fn should_create_note_in_folder_from_context_menu() {
    let mut f = open_fixture();
    f.harness.get_by_label("sub").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "New note here…");
    type_into(&mut f.harness, "Note name", "inner");
    press_enter(&mut f.harness);
    assert!(f.vault.join("sub/inner.md").is_file());
}

#[test]
fn should_rename_note_from_context_menu() {
    let mut f = open_fixture();
    f.harness.get_by_label("b").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "Rename…");
    type_into(&mut f.harness, "New name", "b2");
    press_enter(&mut f.harness);
    assert!(f.vault.join("b2.md").is_file());
    assert!(!f.vault.join("b.md").exists());
}

#[test]
fn should_delete_note_to_trash_from_context_menu() {
    let mut f = open_fixture();
    f.harness.get_by_label("b").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "Delete");
    assert!(!f.vault.join("b.md").exists());
    let trash = f.vault.join(".secondbrain/trash");
    assert_eq!(fs::read_dir(trash).unwrap().count(), 1);
    f.harness.get_by_label("Moved to trash: b.md");
    assert!(f.harness.query_by_label("b").is_none());
}

/// Steps the harness every 20 ms until a label containing `text` shows; fails after 10 s.
fn wait_for_label(harness: &mut Harness<'static, SecondBrainApp>, text: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.query_by_label_contains(text).is_none() {
        assert!(Instant::now() < deadline, "timed out waiting for `{text}`");
        thread::sleep(Duration::from_millis(20));
        harness.step();
    }
}

#[test]
fn should_show_search_results_and_open_one() {
    let mut f = open_fixture();
    type_into(&mut f.harness, "Search notes", "back");
    wait_for_label(&mut f.harness, "Back to [[a]]");
    assert!(f.harness.query_by_label("sub").is_none());
    f.harness.get_by_label_contains("Back to [[a]]").click();
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
}

#[test]
fn should_focus_search_field_with_ctrl_f() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    f.harness.run();
    assert!(f.harness.get_by_label("Search notes").is_focused());
}

#[test]
fn should_list_backlinks_and_open_source() {
    let mut f = open_fixture();
    click_label(&mut f.harness, "a");
    f.harness.get_by_label("BACKLINKS");
    f.harness.get_by_label("OUTGOING LINKS");
    f.harness.get_by_label("TAGS");
    f.harness.get_by_label("#tag");
    f.harness.get_by_label_contains("Back to [[a]]").click();
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
}

#[test]
fn should_mark_unresolved_link_and_create_it_on_click() {
    let mut f = open_fixture();
    click_label(&mut f.harness, "a");
    f.harness.get_by_label_contains("not created yet").click();
    f.harness.run();
    assert!(f.vault.join("ghost.md").is_file());
    assert_eq!(open_path(&f.harness).as_deref(), Some("ghost.md"));
}

#[test]
fn should_mark_attachment_links_and_not_create_them() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    fs::write(vault.join("pics.md"), "![[photo.png]]").unwrap();
    let (mut harness, _dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    click_label(&mut harness, "pics");
    harness.get_by_label_contains("attachment").click();
    harness.run();
    assert!(!vault.join("photo.png.md").exists());
    assert_eq!(open_path(&harness).as_deref(), Some("pics.md"));
}

#[test]
fn should_show_word_count_and_ignored_files() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    fs::write(vault.join("bad.md"), [0xFF, 0xFE, 0xFD]).unwrap();
    let (mut harness, _dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    harness.get_by_label("1 file ignored");
    click_label(&mut harness, "a");
    harness.get_by_label("6 words");
    harness.get_by_label("Sync off");
}
