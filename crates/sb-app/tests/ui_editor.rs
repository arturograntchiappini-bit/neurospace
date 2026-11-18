#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::App;
use eframe::egui::{
    Event, Key, Modifiers, OutputCommand, PointerButton, Pos2, ViewportCommand, ViewportEvent,
    ViewportId,
};
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
    let links =
        "[x](file:///C:/secret) [web](https://example.org) [rel](other.md) [[report.pdf]] [[ ]]";
    fs::write(vault.join("d.md"), links).unwrap();
    let syntaxes = concat!(
        "note[^1]\n\n[^1]: [foot](file:///foot)\n\n",
        "| h |\n|---|\n| [cell](file:///cell) |\n\n",
        "- [ ] [task](file:///task)\n\n",
        "term\n: [def](file:///def)\n\n",
        "[upper](SB-NOTE:foo)",
    );
    fs::write(vault.join("e.md"), syntaxes).unwrap();
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
    assert_eq!(fs::read_dir(&f.vault).unwrap().count(), 5);
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

fn buffer(harness: &Harness<'static, SecondBrainApp>) -> String {
    let workspace = harness.state().workspace().unwrap();
    workspace.open_note().unwrap().buffer.clone()
}

fn set_readonly(file: &Path, readonly: bool) {
    let mut permissions = fs::metadata(file).unwrap().permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(file, permissions).unwrap();
}

fn root_output<'a>(
    harness: &'a Harness<'static, SecondBrainApp>,
) -> &'a eframe::egui::ViewportOutput {
    &harness.output().viewport_output[&ViewportId::ROOT]
}

/// Asks the window to close and runs one frame; true when the application cancelled the close.
fn request_close(harness: &mut Harness<'static, SecondBrainApp>) -> bool {
    let root = harness.input_mut().viewports.get_mut(&ViewportId::ROOT);
    root.unwrap().events.push(ViewportEvent::Close);
    harness.step();
    let commands = &root_output(harness).commands;
    commands
        .iter()
        .any(|command| matches!(command, ViewportCommand::CancelClose))
}

fn click_at(harness: &Harness<'static, SecondBrainApp>, pos: Pos2) {
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

fn opened_url(harness: &Harness<'static, SecondBrainApp>) -> bool {
    let commands = &harness.output().platform_output.commands;
    commands
        .iter()
        .any(|command| matches!(command, OutputCommand::OpenUrl(_)))
}

#[test]
fn should_keep_typed_text_in_conflict_note_and_close_when_file_changed_before_close() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " mine");
    fs::write(f.vault.join("a.md"), "theirs").unwrap();
    assert!(!request_close(&mut f.harness), "the close was cancelled");
    let copies = conflict_copies(&f.vault);
    assert_eq!(copies.len(), 1, "{copies:?}");
    let kept = fs::read_to_string(f.vault.join(&copies[0])).unwrap();
    assert_eq!(kept, "# A\n\n[[b]] [[new one]] ![[p.png]] mine");
    assert_eq!(fs::read_to_string(f.vault.join("a.md")).unwrap(), "theirs");
}

#[test]
fn should_cancel_close_when_save_fails_and_close_on_second_request() {
    let mut f = open_note_fixture("a");
    let file = f.vault.join("a.md");
    set_readonly(&file, true);
    type_into(&mut f.harness, EDITOR, " mine");
    let cancelled = request_close(&mut f.harness);
    f.harness
        .get_by_label_contains("Close again to quit without saving");
    let second = request_close(&mut f.harness);
    set_readonly(&file, false);
    assert!(cancelled, "the first close was not cancelled");
    assert!(!second, "the second close was cancelled too");
    assert_eq!(
        buffer(&f.harness),
        "# A\n\n[[b]] [[new one]] ![[p.png]] mine"
    );
}

#[test]
fn should_close_at_once_when_nothing_is_unsaved() {
    let mut f = open_note_fixture("a");
    assert!(!request_close(&mut f.harness));
}

#[test]
fn should_type_into_an_empty_note_created_with_ctrl_n() {
    let mut f = open_fixture();
    press_command(&mut f.harness, Key::N);
    type_into(&mut f.harness, "Note name", "fresh");
    f.harness.key_press(Key::Enter);
    f.harness.run();
    f.harness.get_by_label(EDITOR).type_text("hi");
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("fresh.md"));
    assert_eq!(buffer(&f.harness), "hi");
}

#[test]
fn should_focus_the_editor_when_a_note_opens_from_the_tree() {
    let mut f = open_fixture();
    f.harness.get_by_label("b").click();
    f.harness.run();
    f.harness.get_by_label(EDITOR).type_text("!");
    f.harness.run();
    assert_eq!(buffer(&f.harness), "B!");
}

#[test]
fn should_focus_the_editor_when_a_note_opens_from_quick_open() {
    let mut f = open_fixture();
    press_command(&mut f.harness, Key::O);
    type_into(&mut f.harness, QUICK_OPEN_FIELD, "b");
    f.harness.key_press(Key::Enter);
    f.harness.run();
    f.harness.get_by_label(EDITOR).type_text("!");
    f.harness.run();
    assert_eq!(buffer(&f.harness), "B!");
}

#[test]
fn should_focus_the_editor_when_a_note_opens_from_the_graph() {
    let mut f = open_fixture();
    press_command(&mut f.harness, Key::G);
    run_until(&mut f.harness, "the graph to settle", |app| {
        app.graph_shown() && app.graph_settled()
    });
    let rect = f.harness.state().graph_node_rect("b.md").unwrap();
    click_at(&f.harness, rect.center());
    f.harness.run();
    assert_eq!(open_path(&f.harness).as_deref(), Some("b.md"));
    f.harness.get_by_label(EDITOR).type_text("!");
    f.harness.run();
    assert_eq!(buffer(&f.harness), "B!");
}

#[test]
fn should_not_bring_back_text_from_before_an_external_edit_with_undo() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " x");
    run_until(&mut f.harness, "the autosave", |_| {
        fs::read_to_string(f.vault.join("a.md"))
            .unwrap()
            .ends_with(" x")
    });
    fs::write(f.vault.join("a.md"), "remote").unwrap();
    run_until(&mut f.harness, "the editor to adopt the file", |app| {
        app.workspace().unwrap().open_note().unwrap().buffer == "remote"
    });
    f.harness.run();
    press_command(&mut f.harness, Key::Z);
    assert_eq!(buffer(&f.harness), "remote");
    f.harness.get_by_label(EDITOR).type_text("!");
    f.harness.run();
    assert_eq!(buffer(&f.harness), "remote!");
}

#[test]
fn should_tell_the_user_and_reload_when_a_save_finds_the_file_changed() {
    let mut f = open_note_fixture("a");
    type_into(&mut f.harness, EDITOR, " mine");
    fs::write(f.vault.join("a.md"), "theirs").unwrap();
    f.harness.input_mut().focused = false;
    f.harness.step();
    f.harness
        .get_by_label_contains("a.md changed on disk; reloading");
    run_until(&mut f.harness, "the conflict copy", |_| {
        !conflict_copies(&f.vault).is_empty()
    });
    assert_eq!(fs::read_to_string(f.vault.join("a.md")).unwrap(), "theirs");
}

#[test]
fn should_stay_on_the_vault_when_leaving_and_the_save_fails() {
    let mut f = open_note_fixture("a");
    let file = f.vault.join("a.md");
    set_readonly(&file, true);
    type_into(&mut f.harness, EDITOR, " mine");
    f.harness.get_by_label("Menu").click();
    f.harness.run();
    f.harness.get_by_label("Open another vault").click();
    f.harness.run();
    set_readonly(&file, false);
    assert_eq!(f.harness.state().screen_name(), "vault");
    f.harness.get_by_label_contains("Could not save a.md");
    assert!(buffer(&f.harness).ends_with(" mine"));
}

#[test]
fn should_leave_shortcuts_alone_while_a_popup_is_open() {
    let mut f = open_note_fixture("a");
    press_command(&mut f.harness, Key::O);
    press_command(&mut f.harness, Key::E);
    press_command(&mut f.harness, Key::G);
    assert!(!f.harness.state().is_previewing());
    assert!(!f.harness.state().graph_shown());
    f.harness.key_press(Key::Escape);
    f.harness.run();
    press_command(&mut f.harness, Key::N);
    press_command(&mut f.harness, Key::E);
    assert!(!f.harness.state().is_previewing());
    f.harness.key_press(Key::Escape);
    f.harness.run();
    press_command(&mut f.harness, Key::E);
    assert!(f.harness.state().is_previewing());
}

#[test]
fn should_not_open_links_that_are_not_web_links() {
    let mut f = open_note_fixture("d");
    press_command(&mut f.harness, Key::E);
    for label in ["x", "rel"] {
        f.harness.get_by_label(label).click();
        f.harness.step();
        assert!(!opened_url(&f.harness), "{label} was opened");
        f.harness.run();
        f.harness
            .get_by_label_contains("Only web links open from a note");
    }
    f.harness.get_by_label("web").click();
    f.harness.step();
    assert!(opened_url(&f.harness));
}

#[test]
fn should_not_open_links_hidden_in_footnotes_tables_task_lists_and_definition_lists() {
    let mut f = open_note_fixture("e");
    press_command(&mut f.harness, Key::E);
    for label in ["foot", "cell", "task", "def", "upper"] {
        f.harness.get_by_label(label).click();
        f.harness.step();
        assert!(!opened_url(&f.harness), "{label} was opened");
        f.harness.run();
        f.harness
            .get_by_label_contains("Only web links open from a note");
        f.harness.get_by_label("Menu").click();
        f.harness.key_press(Key::Escape);
        f.harness.run();
    }
}

#[test]
fn should_show_attachment_and_blank_wikilinks_as_text_in_the_preview() {
    let mut f = open_note_fixture("d");
    press_command(&mut f.harness, Key::E);
    // `[[ ]]` stays literal: the preview draws its brackets as separate text pieces.
    assert_eq!(f.harness.get_all_by_label("[").count(), 2);
    assert_eq!(f.harness.get_all_by_label("]").count(), 2);
    f.harness.get_by_label_contains("report.pdf").click();
    f.harness.run();
    assert!(!f.vault.join("report.pdf.md").exists());
    assert!(!f.vault.join("report.pdf").exists());
    assert_eq!(open_path(&f.harness).as_deref(), Some("d.md"));
    assert!(!opened_url(&f.harness));
}
