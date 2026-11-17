#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::PathBuf;

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::SecondBrainApp;
use tempfile::TempDir;

use common::{harness, run_until};

struct Fixture {
    harness: Harness<'static, SecondBrainApp>,
    vault: PathBuf,
    _dirs: (TempDir, TempDir),
}

/// A vault holding `files`, opened, with the graph shown and settled.
fn open_graph(files: &[(&str, &str)]) -> Fixture {
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
    harness.key_press_modifiers(Modifiers::COMMAND, Key::G);
    run_until(&mut harness, "graph settled", |app| {
        app.graph_shown() && app.graph_settled()
    });
    Fixture {
        harness,
        vault,
        _dirs: (scratch, dir),
    }
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

fn click_node(f: &mut Fixture, id: &str) {
    let rect = f.harness.state().graph_node_rect(id).unwrap();
    assert!(
        f.harness.ctx.content_rect().contains(rect.center()),
        "{id} is off screen"
    );
    click_at(&f.harness, rect.center());
    f.harness.run();
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
    let start = f.harness.state().graph_node_rect("b.md").unwrap().center();
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    f.harness.event(Event::PointerMoved(start));
    f.harness.event(button(start, true));
    f.harness.step();
    for step in 1..=5 {
        let at = start + vec2(20.0 * step as f32, 0.0);
        f.harness.event(Event::PointerMoved(at));
        f.harness.step();
    }
    let held = f.harness.state().graph_node_rect("b.md").unwrap().center();
    assert!(
        held.x - start.x > 60.0,
        "the node stayed: {start:?} -> {held:?}"
    );
    f.harness.event(button(held, false));
    run_until(&mut f.harness, "graph settled", |app| app.graph_settled());
    assert!(f.harness.state().graph_shown());
    assert_eq!(open_path(&f), None);
}
