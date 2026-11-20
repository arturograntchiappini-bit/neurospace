#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{Key, Modifiers, Rect, Role, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ns_app::app::NeurospaceApp;
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
    harness: Harness<'static, NeurospaceApp>,
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

fn open_path(harness: &Harness<'static, NeurospaceApp>) -> Option<String> {
    let workspace = harness.state().workspace()?;
    Some(workspace.open_note()?.path.to_string())
}

fn click_label(harness: &mut Harness<'static, NeurospaceApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn type_into(harness: &mut Harness<'static, NeurospaceApp>, field: &str, text: &str) {
    harness.get_by_label(field).focus();
    harness.run();
    harness.get_by_label(field).type_text(text);
    harness.run();
}

fn press_enter(harness: &mut Harness<'static, NeurospaceApp>) {
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
fn should_open_a_note_of_a_large_folder_after_expanding_it() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    fs::create_dir_all(vault.join("big")).unwrap();
    for number in 0..3000 {
        fs::write(vault.join(format!("big/n{number:04}.md")), "x").unwrap();
    }
    let (mut harness, _dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    assert!(harness.query_by_label("n0000").is_none());
    click_label(&mut harness, "big");
    click_label(&mut harness, "n0000");
    assert_eq!(open_path(&harness).as_deref(), Some("big/n0000.md"));
    assert!(harness.query_by_label("n2999").is_none(), "drew every row");
}

#[test]
fn should_toggle_a_focused_folder_with_enter() {
    let mut f = open_fixture();
    assert!(f.harness.query_by_label("c").is_none());
    focus_row(&mut f.harness, "sub");
    press_enter(&mut f.harness);
    f.harness.get_by_label("c");
    press_enter(&mut f.harness);
    assert!(f.harness.query_by_label("c").is_none());
}

/// The folder row's role, whether it reads as expanded, and whether it carries a toggle state.
fn folder_node(harness: &Harness<'static, NeurospaceApp>) -> (Role, Option<bool>, bool) {
    let node = harness.get_by_label("sub").accesskit_node();
    let data = node.data();
    (data.role(), data.is_expanded(), data.toggled().is_some())
}

#[test]
fn should_announce_a_folder_row_as_a_disclosure_row_without_a_toggle_state() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "sub");
    assert_eq!(
        folder_node(&f.harness),
        (Role::DisclosureTriangle, Some(false), false)
    );
    press_enter(&mut f.harness);
    assert_eq!(
        folder_node(&f.harness),
        (Role::DisclosureTriangle, Some(true), false)
    );
    f.harness.key_press(Key::Space);
    f.harness.run();
    assert_eq!(
        folder_node(&f.harness),
        (Role::DisclosureTriangle, Some(false), false)
    );
}

#[test]
fn should_close_every_folder_when_another_vault_opens() {
    let mut f = open_fixture();
    click_label(&mut f.harness, "sub");
    f.harness.get_by_label("c");
    click_label(&mut f.harness, "Menu");
    click_label(&mut f.harness, "Open another vault");
    assert_eq!(f.harness.state().screen_name(), "welcome");
    type_into(
        &mut f.harness,
        "Vault folder path",
        &f.vault.to_string_lossy(),
    );
    f.harness.get_by_label("Open").click();
    run_until(&mut f.harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    f.harness.run();
    f.harness.get_by_label("sub");
    assert!(
        f.harness.query_by_label("c").is_none(),
        "folder stayed open"
    );
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
    assert!(f.harness.get_by_label("Note name").is_focused());
    let text = fs::read_to_string(f.vault.join("a.md")).unwrap();
    assert_eq!(text, "Link to [[b]] and [[ghost]] #tag");
}

#[test]
fn should_keep_dialog_and_files_when_rename_target_exists() {
    let mut f = open_fixture();
    f.harness.get_by_label("b").click_secondary();
    f.harness.run();
    click_label(&mut f.harness, "Rename…");
    type_into(&mut f.harness, "New name", "a");
    press_enter(&mut f.harness);
    f.harness.get_by_label("New name");
    f.harness.get_by_label_contains("already exists");
    let text = fs::read_to_string(f.vault.join("a.md")).unwrap();
    assert_eq!(text, "Link to [[b]] and [[ghost]] #tag");
    assert!(f.vault.join("b.md").is_file());
}

#[test]
fn should_not_move_focus_to_search_with_ctrl_f_while_dialog_is_open() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    f.harness.run();
    assert!(!f.harness.get_by_label("Search notes").is_focused());
    assert!(f.harness.get_by_label("Note name").is_focused());
}

#[test]
fn should_keep_tab_focus_inside_open_dialog() {
    let mut f = open_fixture();
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    f.harness.get_by_label("Cancel").focus();
    f.harness.run();
    let inside = ["Note name", "Create", "Cancel"];
    for _ in 0..5 {
        f.harness.key_press(Key::Tab);
        f.harness.run();
        assert!(
            inside
                .iter()
                .any(|l| f.harness.get_by_label(l).is_focused())
        );
    }
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
    let trash = f.vault.join(".neurospace/trash");
    assert_eq!(fs::read_dir(trash).unwrap().count(), 1);
    f.harness.get_by_label("Moved to trash: b.md");
    assert!(f.harness.query_by_label("b").is_none());
}

/// Steps the harness every 20 ms until a label containing `text` shows; fails after 10 s.
fn wait_for_label(harness: &mut Harness<'static, NeurospaceApp>, text: &str) {
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

fn focus_row(harness: &mut Harness<'static, NeurospaceApp>, label: &str) {
    harness.get_by_label(label).focus();
    harness.run();
}

/// Where the text `text` was painted inside `row`: the left edge of the first matching glyph run.
fn painted_text_x(harness: &Harness<'static, NeurospaceApp>, text: &str, row: Rect) -> f32 {
    fn find(shape: &Shape, text: &str, row: Rect) -> Option<f32> {
        match shape {
            Shape::Text(run) if run.galley.text() == text && row.contains(run.pos) => {
                Some(run.pos.x)
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|inner| find(inner, text, row)),
            _ => None,
        }
    }
    let shapes = &harness.output().shapes;
    shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, text, row))
        .unwrap_or_else(|| panic!("no text {text:?} painted in the row {row:?}"))
}

#[test]
fn should_align_row_names_to_the_left_one_indent_per_level() {
    let mut f = open_fixture();
    click_label(&mut f.harness, "sub");
    let spacing = f.harness.ctx.global_style().spacing.clone();
    let name_offset = |f: &Fixture, label: &str| {
        let row = f.harness.get_by_label(label).rect();
        painted_text_x(&f.harness, label, row) - row.min.x
    };
    // Inside a row: button padding, the marker slot, the gap, then the name.
    let before_name = spacing.button_padding.x + spacing.icon_width + spacing.icon_spacing;
    for label in ["a", "sub", "c"] {
        let offset = name_offset(&f, label);
        assert!(
            (offset - before_name).abs() < 1.0,
            "{label}: name starts {offset} into its row, expected {before_name}"
        );
    }
    let row_x = |f: &Fixture, label: &str| f.harness.get_by_label(label).rect().min.x;
    let indent = row_x(&f, "c") - row_x(&f, "sub");
    assert!((indent - spacing.indent).abs() < 1.0, "indent {indent}");
}

#[test]
fn should_rename_focused_note_with_f2() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "b");
    f.harness.key_press(Key::F2);
    f.harness.run();
    type_into(&mut f.harness, "New name", "b2");
    press_enter(&mut f.harness);
    assert!(f.vault.join("b2.md").is_file());
    assert!(!f.vault.join("b.md").exists());
}

#[test]
fn should_trash_focused_note_with_delete_key() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "b");
    f.harness.key_press(Key::Delete);
    f.harness.run();
    assert!(!f.vault.join("b.md").exists());
    f.harness.get_by_label("Moved to trash: b.md");
}

#[test]
fn should_open_note_menu_with_menu_key() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "b");
    f.harness.key_press(Key::Menu);
    f.harness.run();
    click_label(&mut f.harness, "Rename…");
    f.harness.get_by_label("New name");
}

#[test]
fn should_choose_a_menu_item_with_arrow_down_and_enter_after_the_menu_key() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "b");
    f.harness.key_press(Key::Menu);
    f.harness.run();
    f.harness.key_press(Key::ArrowDown);
    f.harness.run();
    press_enter(&mut f.harness);
    f.harness.get_by_label("New name");
}

#[test]
fn should_return_the_focus_to_the_row_when_its_menu_closes_with_escape() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "b");
    f.harness.key_press(Key::Menu);
    f.harness.run();
    f.harness.key_press(Key::Escape);
    f.harness.run();
    assert!(
        f.harness.query_by_label("Rename…").is_none(),
        "menu still open"
    );
    assert!(f.harness.get_by_label("b").accesskit_node().is_focused());
}

#[test]
fn should_open_folder_menu_with_shift_f10() {
    let mut f = open_fixture();
    focus_row(&mut f.harness, "sub");
    f.harness.key_press_modifiers(Modifiers::SHIFT, Key::F10);
    f.harness.run();
    click_label(&mut f.harness, "New note here…");
    type_into(&mut f.harness, "Note name", "inner");
    press_enter(&mut f.harness);
    assert!(f.vault.join("sub/inner.md").is_file());
}

#[test]
fn should_list_ignored_files_when_label_is_clicked() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    fs::write(vault.join("bad.md"), [0xFF, 0xFE, 0xFD]).unwrap();
    let (mut harness, _dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    click_label(&mut harness, "1 file ignored");
    harness.get_by_label("bad.md: not UTF-8 text");
}

#[test]
fn should_label_search_hit_without_snippet_by_its_title_only() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    fs::write(vault.join("zebra.md"), "stripes").unwrap();
    let (mut harness, _dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    type_into(&mut harness, "Search notes", "zebra");
    wait_for_label(&mut harness, "zebra");
    harness.get_by_label("zebra");
}
