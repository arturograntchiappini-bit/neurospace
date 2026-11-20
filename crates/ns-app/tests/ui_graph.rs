#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::epaint::Primitive;
use eframe::egui::{ClippedPrimitive, Color32, Mesh};
use eframe::egui::{
    Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, Theme,
    vec2,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ns_app::app::{NeurospaceApp, prepare_context};
use ns_app::state::Workspace;
use ns_app::ui::graph_view::{GraphView, StepBudgets};
use ns_app::ui::theme::{Material, palette};
use ns_core::NotePath;
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
    let files: Vec<(String, String)> = (0..10_001)
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
fn should_stop_asking_for_frames_when_the_wake_time_cap_is_spent() {
    let mut f = open_vault(&ring_vault(150));
    f.harness.state_mut().set_graph_step_budgets(StepBudgets {
        animated: Duration::ZERO,
        reduced_motion: Duration::ZERO,
    });
    f.harness.state_mut().set_graph_wake_cap(Duration::ZERO);
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

/// One frame of the graph pane alone, over `workspace`, with the pointer at `pointer`.
fn paint_frame(
    ctx: &Context,
    view: &mut GraphView,
    workspace: &Workspace,
    pointer: Option<Pos2>,
) -> FullOutput {
    paint_events(
        ctx,
        view,
        workspace,
        pointer.map(Event::PointerMoved).into_iter().collect(),
    )
}

/// One frame of the graph pane alone, over `workspace`, that receives `events`.
fn paint_events(
    ctx: &Context,
    view: &mut GraphView,
    workspace: &Workspace,
    events: Vec<Event>,
) -> FullOutput {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        events,
        ..RawInput::default()
    };
    let colours = palette(Theme::Light, Material::Opaque);
    let mut frame = ctx.run_ui(input, |ui| {
        view.show(ui, &colours, workspace);
    });
    frame.textures_delta.clear();
    frame
}

fn count_shapes(shape: &Shape) -> usize {
    match shape {
        Shape::Vec(inner) => inner.iter().map(count_shapes).sum(),
        _ => 1,
    }
}

fn tessellate(ctx: &Context, frame: FullOutput) -> Vec<ClippedPrimitive> {
    ctx.tessellate(frame.shapes, frame.pixels_per_point)
}

fn meshes(primitives: &[ClippedPrimitive]) -> impl Iterator<Item = &Mesh> {
    primitives.iter().filter_map(|p| match &p.primitive {
        Primitive::Mesh(mesh) => Some(mesh),
        Primitive::Callback(_) => None,
    })
}

/// Vertices and indices of everything drawn.
fn mesh_sizes(primitives: &[ClippedPrimitive]) -> (usize, usize) {
    let sizes = meshes(primitives).map(|m| (m.vertices.len(), m.indices.len()));
    sizes.fold((0, 0), |(v, i), (dv, di)| (v + dv, i + di))
}

/// The positions of the vertices drawn in exactly `colour`: the full-colour core of a graph edge
/// has two at each end, and nothing else the pane draws in a test has the edge colour.
fn positions_in(primitives: &[ClippedPrimitive], colour: Color32) -> Vec<Pos2> {
    let all = meshes(primitives).flat_map(|m| &m.vertices);
    all.filter(|v| v.color == colour).map(|v| v.pos).collect()
}

fn vertices_in(primitives: &[ClippedPrimitive], colour: Color32) -> usize {
    positions_in(primitives, colour).len()
}

fn texts(shape: &Shape, found: &mut Vec<String>) {
    match shape {
        Shape::Vec(inner) => inner.iter().for_each(|s| texts(s, found)),
        Shape::Text(text) => found.push(text.galley.text().to_owned()),
        _ => {}
    }
}

/// A tiny deterministic generator: each call gives a value below `bound`.
fn lcg(seed: u64) -> impl FnMut(usize) -> usize {
    let mut state = seed;
    move |bound| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize % bound
    }
}

/// `clusters` folders of `size` notes: each note links to 4 random others of its folder, and one
/// note in five also to a note of another folder (the shape of the layout's own scale test).
fn clustered_vault(clusters: usize, size: usize, seed: u64) -> Vec<(String, String)> {
    let mut next = lcg(seed);
    let mut files = Vec::new();
    for k in 0..clusters {
        for i in 0..size {
            let mut text: String = (0..4)
                .map(|_| format!("[[c{k}/n{}]] ", (i + 1 + next(size - 1)) % size))
                .collect();
            if i % 5 == 0 {
                let other = (k + 1 + next(clusters - 1)) % clusters;
                text.push_str(&format!("[[c{other}/n{}]]", next(size)));
            }
            files.push((format!("c{k}/n{i}.md"), text));
        }
    }
    files
}

fn write_vault(files: &[(String, String)]) -> (TempDir, PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("vault");
    for (name, text) in files {
        let path = vault.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    (scratch, vault)
}

const PAINT_BUDGET: Duration = Duration::from_millis(8);

/// One frame at rest: its paint plus tessellation time, its vertex count, and a line describing it.
fn measure_frame(
    ctx: &Context,
    view: &mut GraphView,
    workspace: &Workspace,
) -> (Duration, usize, String) {
    let started = Instant::now();
    let frame = paint_frame(ctx, view, workspace, None);
    let painted = started.elapsed();
    let shapes: usize = frame.shapes.iter().map(|c| count_shapes(&c.shape)).sum();
    let started = Instant::now();
    let primitives = tessellate(ctx, frame);
    let tessellated = started.elapsed();
    let (vertices, indices) = mesh_sizes(&primitives);
    let line = format!(
        "{shapes} shapes, {vertices} vertices, {indices} indices, paint {painted:?}, \
         tessellate {tessellated:?}"
    );
    (painted + tessellated, vertices, line)
}

/// Paints a clustered vault of `clusters` folders of `size` notes at the «Fit graph» zoom, once the
/// layout has come to rest, and returns the median over five frames of paint plus tessellation,
/// and the vertices of the last one.
fn measure_graph_painting(clusters: usize, size: usize) -> (Duration, usize) {
    let (_scratch, vault) = write_vault(&clustered_vault(clusters, size, 11));
    let workspace = Workspace::open(&vault).unwrap();
    let ctx = Context::default();
    prepare_context(&ctx);
    let mut view = GraphView::default();
    view.set_reduce_motion(true);
    view.set_step_budgets(StepBudgets {
        animated: Duration::ZERO,
        reduced_motion: Duration::from_secs(600),
    });
    paint_frame(&ctx, &mut view, &workspace, None);
    assert!(view.is_settled(), "the layout should have come to rest");
    view.fit();
    paint_frame(&ctx, &mut view, &workspace, None);
    let frames: Vec<_> = (0..5)
        .map(|_| measure_frame(&ctx, &mut view, &workspace))
        .collect();
    let edges = workspace.index().graph().edges.len();
    for (_, _, line) in &frames {
        println!("{} notes, {edges} edges: {line}", clusters * size);
    }
    let mut totals: Vec<Duration> = frames.iter().map(|(total, _, _)| *total).collect();
    totals.sort();
    (totals[2], frames[4].1)
}

#[test]
#[ignore = "measurement: cargo test --release -p ns-app --test ui_graph should_paint_a_large_graph -- --ignored --nocapture"]
fn should_paint_a_large_graph_in_a_few_milliseconds() {
    // 10 000 notes, about 41 000 edges: too many to draw, so only the notes are (measured before
    // the change: 365 408 vertices and 1 280 136 indices in 21–24 ms). Then 3 400 notes, about
    // 14 300 edges: just under the limit, so every edge is drawn, at 8 vertices each.
    for (clusters, size, max_vertices) in [(100, 100, 60_000), (100, 34, 175_000)] {
        let (median, vertices) = measure_graph_painting(clusters, size);
        assert!(median < PAINT_BUDGET, "{median:?} is over {PAINT_BUDGET:?}");
        assert!(vertices < max_vertices, "{vertices} vertices");
    }
}

const HINT: &str = "Zoom in to see every link";

/// What a pane test holds: a pane over a six-note vault in which `a` links to `b`, `c` and `d`
/// and `e` to `f`, fitted, with the layout at rest.
struct Crowd {
    ctx: Context,
    view: GraphView,
    workspace: Workspace,
    _scratch: TempDir,
}

fn crowd(limit: usize) -> Crowd {
    let files = notes(&[
        ("a.md", "[[b]] [[c]] [[d]]"),
        ("b.md", ""),
        ("c.md", ""),
        ("d.md", ""),
        ("e.md", "[[f]]"),
        ("f.md", ""),
    ]);
    let (scratch, vault) = write_vault(&files);
    let workspace = Workspace::open(&vault).unwrap();
    let ctx = Context::default();
    prepare_context(&ctx);
    let mut view = GraphView::default();
    view.set_reduce_motion(true);
    view.set_max_drawn_edges(limit);
    paint_frame(&ctx, &mut view, &workspace, None);
    assert!(view.is_settled());
    view.fit();
    Crowd {
        ctx,
        view,
        workspace,
        _scratch: scratch,
    }
}

impl Crowd {
    /// Paints twice (the pointer's target is known to egui one frame late) and returns what was
    /// drawn the second time, and its texts.
    fn paint(&mut self, pointer: Option<Pos2>) -> (Vec<ClippedPrimitive>, Vec<String>) {
        paint_frame(&self.ctx, &mut self.view, &self.workspace, pointer);
        let frame = paint_frame(&self.ctx, &mut self.view, &self.workspace, pointer);
        let mut found = Vec::new();
        frame
            .shapes
            .iter()
            .for_each(|c| texts(&c.shape, &mut found));
        (tessellate(&self.ctx, frame), found)
    }

    /// How many vertices of the full-colour core of an edge are drawn: four per edge.
    fn core_vertices(&mut self, pointer: Option<Pos2>) -> usize {
        let edge = palette(Theme::Light, Material::Opaque).edge.to_color32();
        vertices_in(&self.paint(pointer).0, edge)
    }

    fn says_hint(&mut self) -> bool {
        self.paint(None).1.iter().any(|t| t == HINT)
    }
}

#[test]
fn should_draw_every_edge_when_few_are_in_view() {
    let mut c = crowd(100);
    assert_eq!(
        c.core_vertices(None),
        16,
        "four edges, four core vertices each"
    );
    assert!(!c.says_hint());
}

#[test]
fn should_hide_the_edges_that_crowd_the_view_and_say_so() {
    let mut c = crowd(2);
    assert_eq!(c.core_vertices(None), 0);
    assert!(c.says_hint());
}

#[test]
fn should_draw_the_edges_of_the_hovered_node_at_any_zoom() {
    let mut c = crowd(2);
    let a = c.view.node_rect("a.md").unwrap().center();
    assert_eq!(
        c.core_vertices(Some(a)),
        12,
        "a's three edges, four core vertices each"
    );
}

#[test]
fn should_keep_the_edges_of_the_open_note_when_the_others_are_hidden() {
    let mut c = crowd(2);
    let open = NotePath::parse("e.md").unwrap();
    c.workspace.select(&open).unwrap();
    assert_eq!(c.core_vertices(None), 4, "the one edge of e");
}

#[test]
fn should_draw_an_edge_once_when_the_hovered_and_the_open_note_share_it() {
    let mut c = crowd(2);
    let open = NotePath::parse("e.md").unwrap();
    c.workspace.select(&open).unwrap();
    let f = c.view.node_rect("f.md").unwrap().center();
    assert_eq!(c.core_vertices(Some(f)), 4, "the edge of e and f, once");
}

/// The edge colour dimmed as a hovered node dims what it is not linked to.
fn dimmed_edge() -> Color32 {
    palette(Theme::Light, Material::Opaque)
        .edge
        .to_color32()
        .gamma_multiply(0.25)
}

#[test]
fn should_dim_the_edges_that_do_not_touch_the_hovered_node() {
    let mut c = crowd(100);
    let a = c.view.node_rect("a.md").unwrap().center();
    let (primitives, _) = c.paint(Some(a));
    let edge = palette(Theme::Light, Material::Opaque).edge.to_color32();
    assert_eq!(vertices_in(&primitives, edge), 12, "a's three edges stay");
    assert_eq!(
        vertices_in(&primitives, dimmed_edge()),
        4,
        "the edge of e and f dims"
    );
}

#[test]
fn should_draw_an_edge_with_a_full_colour_core_at_least_one_pixel_wide() {
    for scale in [1.0_f32, 2.0] {
        let mut c = crowd(100);
        c.ctx.set_zoom_factor(scale);
        let (primitives, _) = c.paint(None);
        assert_eq!(c.ctx.pixels_per_point(), scale, "setup: the scale applies");
        let edge = palette(Theme::Light, Material::Opaque).edge.to_color32();
        let core = positions_in(&primitives, edge);
        assert_eq!(core.len(), 16);
        let pixel = 1.0 / scale;
        for ends in core.chunks(2) {
            let width = (ends[0] - ends[1]).length();
            assert!(width >= pixel - 1e-3, "core {width} pt at scale {scale}");
            assert!(width <= 2.0 * pixel, "core {width} pt at scale {scale}");
        }
    }
}

fn all_shapes(shape: &Shape, found: &mut Vec<Shape>) {
    match shape {
        Shape::Vec(inner) => inner.iter().for_each(|s| all_shapes(s, found)),
        other => found.push(other.clone()),
    }
}

/// The shapes of the second of two frames over `c`.
fn shapes_of(c: &mut Crowd) -> Vec<Shape> {
    paint_frame(&c.ctx, &mut c.view, &c.workspace, None);
    let frame = paint_frame(&c.ctx, &mut c.view, &c.workspace, None);
    let mut found = Vec::new();
    for clipped in &frame.shapes {
        all_shapes(&clipped.shape, &mut found);
    }
    found
}

#[test]
fn should_draw_the_crowded_hint_on_a_backing_of_the_document_surface() {
    let document = palette(Theme::Light, Material::Opaque)
        .document
        .to_color32();
    let backings = |shapes: &[Shape]| -> Vec<Rect> {
        let is_backing = |s: &&Shape| matches!(s, Shape::Rect(r) if r.fill == document);
        let rect = |s: &Shape| s.visual_bounding_rect();
        shapes.iter().filter(is_backing).map(rect).collect()
    };
    let shapes = shapes_of(&mut crowd(2));
    let hint = shapes
        .iter()
        .find(|s| matches!(s, Shape::Text(t) if t.galley.text() == HINT))
        .expect("the hint is painted")
        .visual_bounding_rect();
    let behind = backings(&shapes)
        .into_iter()
        .filter(|rect| rect.contains_rect(hint))
        .count();
    assert_eq!(behind, 1, "one backing holds the hint at {hint:?}");
    assert!(backings(&shapes_of(&mut crowd(100))).is_empty());
}

/// A pane over a 300-note ring vault that runs one layout step per frame, so that it animates.
struct Settling {
    ctx: Context,
    view: GraphView,
    workspace: Workspace,
    _scratch: TempDir,
}

fn settling() -> Settling {
    let (scratch, vault) = write_vault(&ring_vault(300));
    let workspace = Workspace::open(&vault).unwrap();
    let ctx = Context::default();
    prepare_context(&ctx);
    let mut view = GraphView::default();
    view.set_step_budgets(StepBudgets {
        animated: Duration::ZERO,
        reduced_motion: Duration::ZERO,
    });
    Settling {
        ctx,
        view,
        workspace,
        _scratch: scratch,
    }
}

impl Settling {
    fn frame(&mut self, events: Vec<Event>) {
        paint_events(&self.ctx, &mut self.view, &self.workspace, events);
    }

    fn run_to_rest(&mut self) {
        self.frame(vec![]);
        for _ in 0..5_000 {
            if self.view.is_settled() {
                break;
            }
            self.frame(vec![]);
        }
        assert!(self.view.is_settled(), "the layout should come to rest");
        self.frame(vec![]);
    }

    fn node_rects(&self) -> Vec<Rect> {
        let ids = (0..300).map(|i| format!("n{i}.md"));
        ids.map(|id| self.view.node_rect(&id).unwrap()).collect()
    }

    fn pane(&self) -> Rect {
        self.view.pane_rect().unwrap()
    }
}

#[test]
fn should_keep_every_node_in_view_while_the_layout_settles_without_input() {
    let mut s = settling();
    s.run_to_rest();
    let all = s
        .node_rects()
        .iter()
        .fold(Rect::NOTHING, |a, r| a.union(*r));
    let pane = s.pane();
    assert!(pane.contains_rect(all), "{all:?} is outside {pane:?}");
    let fills = all.width() >= 0.6 * pane.width() || all.height() >= 0.6 * pane.height();
    assert!(fills, "the nodes {all:?} sit small in {pane:?}");
}

#[test]
fn should_stop_fitting_the_view_once_the_user_pans() {
    let mut s = settling();
    for _ in 0..3 {
        s.frame(vec![]);
    }
    let from = s.pane().left_top() + vec2(30.0, 30.0);
    s.frame(vec![Event::PointerMoved(from), button(from, true)]);
    let mut at = from;
    for _ in 0..7 {
        at += vec2(100.0, 0.0);
        s.frame(vec![Event::PointerMoved(at)]);
    }
    s.frame(vec![button(at, false)]);
    s.run_to_rest();
    let pane = s.pane();
    let outside = s.node_rects().iter().any(|r| !pane.intersects(*r));
    assert!(outside, "the view was fitted again after the pan");
}
