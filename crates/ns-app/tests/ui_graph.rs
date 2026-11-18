#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ns_app::app::NeurospaceApp;
use ns_app::ui::graph_view::StepBudgets;
use tempfile::TempDir;

use common::{harness, run_until};

struct Fixture {
    harness: Harness<'static, NeurospaceApp>,
    vault: PathBuf,
    dirs: (TempDir, TempDir),
}

/// A vault holding `files`, opened, with the open note shown rendered. The graph is not shown.
fn open_vault(files: &[(String, String)]) -> Fixture {
    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    for (name, text) in files {
        fs::write(vault.join(name), text).unwrap();
    }
    let (mut harness, dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::E);
    harness.run();
    Fixture {
        harness,
        vault,
        dirs: (scratch, dir),
    }
}

/// Switches the graph on and waits until its layout is at rest.
fn show_graph(f: &mut Fixture) {
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::G);
    run_until(&mut f.harness, "graph settled", |app| {
        app.graph_shown() && app.graph_settled()
    });
}

fn notes(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let owned = |(name, text): &(&str, &str)| ((*name).to_owned(), (*text).to_owned());
    files.iter().map(owned).collect()
}

/// A vault holding `files`, opened, with the graph shown and settled.
fn open_graph(files: &[(&str, &str)]) -> Fixture {
    let mut f = open_vault(&notes(files));
    show_graph(&mut f);
    f
}

fn pane(f: &Fixture) -> Rect {
    f.harness.state().graph_pane_rect().unwrap()
}

fn node_centre(f: &Fixture, id: &str) -> Pos2 {
    f.harness.state().graph_node_rect(id).unwrap().center()
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn click_at(harness: &Harness<'static, NeurospaceApp>, pos: Pos2) {
    harness.event(Event::PointerMoved(pos));
    harness.event(button(pos, true));
    harness.event(button(pos, false));
}

fn click_node(f: &mut Fixture, id: &str) {
    let rect = f.harness.state().graph_node_rect(id).unwrap();
    assert!(pane(f).contains(rect.center()), "{id} is outside the pane");
    click_at(&f.harness, rect.center());
    f.harness.run();
}

/// Presses at `from`, moves the pointer in `steps` frames by `by` in total and keeps the button
/// down. Returns where the pointer is.
fn drag_and_hold(f: &mut Fixture, from: Pos2, by: f32, steps: u32) -> Pos2 {
    f.harness.event(Event::PointerMoved(from));
    f.harness.event(button(from, true));
    f.harness.step();
    let mut at = from;
    for _ in 0..steps {
        at += vec2(by / steps as f32, 0.0);
        f.harness.event(Event::PointerMoved(at));
        f.harness.step();
    }
    at
}

/// Drags the empty background far to the right, so that the nodes leave the pane.
fn pan_away(f: &mut Fixture) {
    let from = pane(f).left_top() + vec2(30.0, 30.0);
    let end = drag_and_hold(f, from, 700.0, 7);
    f.harness.event(button(end, false));
    f.harness.run();
    let rect = f.harness.state().graph_node_rect("b.md").unwrap();
    assert!(!pane(f).intersects(rect), "setup: b should be out of view");
}

fn open_path(f: &Fixture) -> Option<String> {
    let workspace = f.harness.state().workspace()?;
    Some(workspace.open_note()?.path.to_string())
}

#[test]
fn should_name_the_graph_area_and_offer_a_list_view() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    f.harness.get_by_label("Graph of notes");
    f.harness.get_by_label("List view").click();
    f.harness.run();
    f.harness.get_by_label("Find a note");
    assert!(f.harness.state().graph_shown());
}

#[test]
fn should_open_list_view_when_enter_pressed_on_the_focused_graph() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    f.harness.get_by_label("Graph of notes").focus();
    f.harness.run();
    f.harness.key_press(Key::Enter);
    f.harness.run();
    f.harness.get_by_label("Find a note");
}

#[test]
fn should_open_note_when_graph_node_clicked() {
    let mut f = open_graph(&[("a.md", "[[b]] [[ghost]]"), ("b.md", "")]);
    assert!(f.harness.state().is_previewing());
    click_node(&mut f, "b.md");
    assert_eq!(open_path(&f).as_deref(), Some("b.md"));
    assert!(!f.harness.state().graph_shown());
    assert!(!f.harness.state().is_previewing());
}

#[test]
fn should_create_note_when_ghost_node_clicked() {
    let mut f = open_graph(&[("a.md", "[[Ghost]]"), ("b.md", "")]);
    click_node(&mut f, "?ghost");
    assert_eq!(open_path(&f).as_deref(), Some("Ghost.md"));
    let names: Vec<String> = fs::read_dir(&f.vault)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"Ghost.md".to_owned()), "{names:?}");
    assert!(!f.harness.state().graph_shown());
}

#[test]
fn should_stay_on_the_graph_when_a_click_fails() {
    let mut f = open_graph(&[("a.md", "[[Ghost]]")]);
    fs::write(f.vault.join("Ghost.md"), "precious").unwrap();
    click_node(&mut f, "?ghost");
    assert!(f.harness.state().graph_shown());
    assert_eq!(
        fs::read_to_string(f.vault.join("Ghost.md")).unwrap(),
        "precious"
    );
    f.harness.get_by_label_contains("already exists");
}

#[test]
fn should_move_node_and_not_open_it_when_dragged() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    let start = node_centre(&f, "b.md");
    let end = drag_and_hold(&mut f, start, 100.0, 5);
    let held = node_centre(&f, "b.md");
    assert!(
        held.x - start.x > 60.0,
        "the node stayed: {start:?} -> {held:?}"
    );
    f.harness.event(button(end, false));
    run_until(&mut f.harness, "graph settled", |app| app.graph_settled());
    assert!(f.harness.state().graph_shown());
    assert_eq!(open_path(&f), None);
}

#[test]
fn should_ignore_the_drag_when_the_dragged_note_disappears() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", ""), ("c.md", "[[a]]")]);
    let start = node_centre(&f, "b.md");
    let held = drag_and_hold(&mut f, start, 100.0, 5);
    fs::remove_file(f.vault.join("b.md")).unwrap();
    run_until(&mut f.harness, "b gone from the graph", |app| {
        app.graph_node_rect("b.md").is_none()
    });
    run_until(&mut f.harness, "graph settled", |app| app.graph_settled());
    let (a, c) = (node_centre(&f, "a.md"), node_centre(&f, "c.md"));
    for step in 1..=3 {
        f.harness
            .event(Event::PointerMoved(held + vec2(30.0 * step as f32, 0.0)));
        f.harness.step();
    }
    assert_eq!(
        node_centre(&f, "a.md"),
        a,
        "a drag of a vanished node moved another node"
    );
    assert_eq!(
        node_centre(&f, "c.md"),
        c,
        "a drag of a vanished node panned the view"
    );
}

#[test]
fn should_leave_the_graph_when_a_note_is_created_from_the_dialog() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    f.harness.run();
    f.harness.get_by_label("Note name").focus();
    f.harness.run();
    f.harness.get_by_label("Note name").type_text("fresh");
    f.harness.run();
    f.harness.get_by_label("Create").click();
    f.harness.run();
    assert!(!f.harness.state().graph_shown());
    assert_eq!(open_path(&f).as_deref(), Some("fresh.md"));
}

#[test]
fn should_bring_every_node_into_view_when_fit_graph_pressed() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    pan_away(&mut f);
    f.harness.get_by_label("Fit graph").click();
    f.harness.run();
    let b = f.harness.state().graph_node_rect("b.md").unwrap();
    assert!(pane(&f).contains_rect(b), "{b:?} is outside {:?}", pane(&f));
}

#[test]
fn should_bring_every_node_into_view_when_home_pressed_on_the_graph() {
    let mut f = open_graph(&[("a.md", "[[b]]"), ("b.md", "")]);
    pan_away(&mut f);
    f.harness.get_by_label("Graph of notes").focus();
    f.harness.run();
    f.harness.key_press(Key::Home);
    f.harness.run();
    let b = f.harness.state().graph_node_rect("b.md").unwrap();
    assert!(pane(&f).contains_rect(b), "{b:?} is outside {:?}", pane(&f));
}

#[test]
fn should_say_so_when_the_vault_has_no_notes() {
    let f = open_graph(&[]);
    f.harness.get_by_label("No notes yet");
}

#[test]
fn should_show_a_simplified_layout_that_still_opens_notes_when_the_vault_is_large() {
    let files: Vec<(String, String)> = (0..2001)
        .map(|i| (format!("n{i}.md"), String::new()))
        .collect();
    let mut f = open_vault(&files);
    show_graph(&mut f);
    f.harness.get_by_label("Large vault: simplified layout");
    click_node(&mut f, "n0.md");
    assert_eq!(open_path(&f).as_deref(), Some("n0.md"));
}

fn ring_vault(size: usize) -> Vec<(String, String)> {
    (0..size)
        .map(|i| {
            let text = format!("[[n{}]] [[n{}]]", (i + 1) % size, (i * 7 + 3) % size);
            (format!("n{i}.md"), text)
        })
        .collect()
}

#[test]
fn should_settle_within_one_frame_when_motion_is_reduced() {
    let mut f = open_vault(&ring_vault(150));
    f.harness.state_mut().set_graph_step_budgets(StepBudgets {
        animated: Duration::ZERO,
        reduced_motion: Duration::from_secs(600),
    });
    f.harness.get_by_label("Menu").click();
    f.harness.run();
    f.harness.get_by_label("Reduce motion").click();
    f.harness.run();
    let saved = fs::read_to_string(f.dirs.1.path().join("app.json")).unwrap();
    assert!(saved.contains("\"reduce_motion\": true"), "{saved}");
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::G);
    f.harness.step();
    assert!(f.harness.state().graph_shown());
    assert!(f.harness.state().graph_settled());
}

#[test]
fn should_animate_over_several_frames_when_motion_is_not_reduced() {
    let mut f = open_vault(&ring_vault(150));
    f.harness.state_mut().set_graph_step_budgets(StepBudgets {
        animated: Duration::ZERO,
        reduced_motion: Duration::ZERO,
    });
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::G);
    f.harness.step();
    assert!(f.harness.state().graph_shown());
    assert!(!f.harness.state().graph_settled());
}
