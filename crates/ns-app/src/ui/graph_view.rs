//! The graph pane (spec 9.4, appendix A.3): draws the force layout on the document surface and
//! turns clicks on nodes into note actions.

use std::time::{Duration, Instant};

use eframe::egui::{
    Align2, Button, Color32, ColorImage, Context, CornerRadius, Event, FontId, Key, Label, Mesh,
    Painter, Pos2, Rect, Response, RichText, Role, Sense, Shape, Stroke, StrokeKind, TextStyle,
    TextureFilter, TextureHandle, TextureId, TextureOptions, Ui, Vec2, WidgetInfo, pos2, vec2,
};
use ns_core::{Graph, NotePath};

use crate::state::graph_layout::Layout;
use crate::state::graph_viewport::{LABEL_ZOOM, Viewport};
use crate::state::{NoteAction, Workspace};
use crate::ui::theme::{Palette, RADIUS};

const GRAPH_NAME: &str = "Graph of notes";
const EMPTY_HINT: &str = "No notes yet";
const CROWDED_HINT: &str = "Zoom in to see every link";
const SIMPLIFIED_HINT: &str = "Large vault: simplified layout";
/// Time one frame may spend on layout steps; at least one step always runs.
const STEP_BUDGET: Duration = Duration::from_millis(8);
/// With «Reduce motion» a frame runs the layout to rest, but still not longer than this.
const REDUCED_MOTION_BUDGET: Duration = Duration::from_millis(200);
/// The longest the layout keeps animating after a wake. At 10 000 notes a step costs a whole
/// frame, so the step cap alone would let it run for minutes.
const WAKE_TIME_CAP: Duration = Duration::from_secs(10);
const SCROLL_ZOOM_RATE: f32 = 0.002;
const DIMMED: f32 = 0.25;
const GHOST_FILL: f32 = 0.5;
const EDGE_WIDTH: f32 = 1.0;
/// What one edge adds to the edge mesh: four vertices at each end, three quads between them.
const EDGE_VERTICES: usize = 8;
const EDGE_TRIANGLES: usize = 6;
const OUTLINE_WIDTH: f32 = 1.0;
const OPEN_RING_WIDTH: f32 = 2.0;
const OPEN_RING_GAP: f32 = 3.0;
const LABEL_GAP: f32 = 3.0;
const FOCUS_RING_WIDTH: f32 = 2.0;
/// Space between the crowded hint's text and the edge of its backing.
const HINT_PADDING: Vec2 = vec2(10.0, 4.0);
/// With more edges than this in view, only those of the hovered and the open note are drawn.
const MAX_DRAWN_EDGES: usize = 7_000;
/// With more notes than this in view, only the hovered one is labelled, even when zoomed in: a
/// label is about 600 bytes of mesh, and a dense graph at 150 % has thousands in view.
const MAX_LABELS: usize = 500;
/// A note whose circle would be smaller than this (in points) is drawn as a plain square.
const DOT_RADIUS: f32 = 1.5;
/// Side of the square texture holding the one disc every note is a tinted quad of. With mipmaps
/// the edge stays smooth from a few pixels across to a large zoom.
const DISC_TEXELS: usize = 64;

/// How long one frame may spend on layout steps. Tests replace the defaults so that they do not
/// depend on how fast the host is.
#[derive(Clone, Copy, Debug)]
pub struct StepBudgets {
    /// While the layout animates.
    pub animated: Duration,
    /// With «Reduce motion», when the layout runs to rest.
    pub reduced_motion: Duration,
}

impl Default for StepBudgets {
    fn default() -> StepBudgets {
        StepBudgets {
            animated: STEP_BUDGET,
            reduced_motion: REDUCED_MOTION_BUDGET,
        }
    }
}

/// What the graph pane remembers between frames: the layout, the view and an ongoing drag.
pub struct GraphView {
    layout: Layout,
    revision: Option<u64>,
    viewport: Viewport,
    pane: Option<Rect>,
    /// The note being dragged (by id: indices change when the layout updates) and where it goes.
    dragging: Option<(String, [f32; 2])>,
    reduce_motion: bool,
    budgets: StepBudgets,
    /// How long the layout may keep animating, and since when it has.
    wake_cap: Duration,
    animating_since: Option<Instant>,
    /// How many edges may be in view before only some are drawn.
    max_drawn_edges: usize,
    /// How many notes may be in view before only the hovered one is labelled.
    max_labels: usize,
    /// The antialiased white disc that every note's quad samples; made on the first frame.
    disc: Option<TextureHandle>,
    /// Whether the view is refitted every frame: from a wake of the layout until the user moves
    /// the view or the layout is at rest.
    auto_fit: bool,
}

impl Default for GraphView {
    fn default() -> GraphView {
        GraphView {
            layout: Layout::new(&Graph::default()),
            revision: None,
            viewport: Viewport::default(),
            pane: None,
            dragging: None,
            reduce_motion: false,
            budgets: StepBudgets::default(),
            wake_cap: WAKE_TIME_CAP,
            animating_since: None,
            max_drawn_edges: MAX_DRAWN_EDGES,
            max_labels: MAX_LABELS,
            disc: None,
            auto_fit: false,
        }
    }
}

/// What the user asked for in the graph pane this frame.
pub struct GraphOutcome {
    /// A node was clicked.
    pub action: Option<NoteAction>,
    /// «List view» was pressed, or Enter or Space on the focused graph.
    pub list_view: bool,
}

/// What painting a frame needs besides the view itself.
struct Scene<'a> {
    painter: &'a Painter,
    palette: &'a Palette,
    label_font: FontId,
    disc: TextureId,
    open: Option<&'a str>,
    hovered: Option<usize>,
}

impl GraphView {
    /// Draws the pane in the space left in `ui`: the buttons, then the graph.
    pub fn show(&mut self, ui: &mut Ui, p: &Palette, workspace: &Workspace) -> GraphOutcome {
        self.follow(workspace);
        let (list_view, fit) = self.buttons(ui, p);
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::labeled(Role::Canvas, true, GRAPH_NAME));
        self.pane = Some(response.rect);
        let (activated, home) = focused_keys(ui, &response);
        if self.navigate(ui, &response) || key_pressed(ui, &response) {
            self.auto_fit = false;
        }
        self.auto_fit |= fit || home;
        self.settle(ui.ctx());
        self.keep_fitted();
        let disc = self.disc_texture(ui.ctx());
        let crowded = self.paint(&Scene {
            painter: &painter,
            palette: p,
            label_font: TextStyle::Small.resolve(ui.style()),
            disc,
            open: workspace.open_note().map(|open| open.path.as_str()),
            hovered: self.hovered(&response),
        });
        if response.has_focus() {
            paint_focus_ring(&painter, p, response.rect);
        }
        show_hints(ui, p, response.rect, [self.layout.is_empty(), crowded]);
        GraphOutcome {
            action: self.clicked_node(&response).filter(|_| !activated),
            list_view: list_view || activated,
        }
    }

    /// The node under the pointer, if it is over the pane.
    fn hovered(&self, response: &Response) -> Option<usize> {
        let at = response.hover_pos()?;
        self.layout.node_at(self.to_world(at))
    }

    /// «List view», «Fit graph» and, for a large vault, the note about the simplified layout.
    /// Returns whether the first two were pressed.
    fn buttons(&self, ui: &mut Ui, p: &Palette) -> (bool, bool) {
        let flat = |text| Button::new(text).frame_when_inactive(false);
        ui.horizontal(|ui| {
            let list_view = ui.add(flat("List view")).clicked();
            let fit = ui.add(flat("Fit graph")).clicked();
            if self.layout.is_simplified() {
                let hint = RichText::new(SIMPLIFIED_HINT).color(p.text_secondary.to_color32());
                ui.label(hint);
            }
            (list_view, fit)
        })
        .inner
    }

    /// The disc texture, uploaded once and reused by every later frame.
    fn disc_texture(&mut self, ctx: &Context) -> TextureId {
        let options = TextureOptions::LINEAR.with_mipmap_mode(Some(TextureFilter::Linear));
        self.disc
            .get_or_insert_with(|| ctx.load_texture("graph-disc", disc_image(), options))
            .id()
    }

    /// True when the layout has come to rest.
    pub fn is_settled(&self) -> bool {
        self.layout.is_settled()
    }

    /// Sets whether the layout runs to rest at once instead of animating.
    pub fn set_reduce_motion(&mut self, reduce: bool) {
        self.reduce_motion = reduce;
    }

    /// Replaces the per-frame time limits of the layout. Used by tests.
    pub fn set_step_budgets(&mut self, budgets: StepBudgets) {
        self.budgets = budgets;
    }

    /// Replaces the longest time the layout keeps animating after a wake. Used by tests.
    pub fn set_wake_cap(&mut self, cap: Duration) {
        self.wake_cap = cap;
    }

    /// Replaces the number of edges in view above which only some are drawn. Used by tests.
    pub fn set_max_drawn_edges(&mut self, limit: usize) {
        self.max_drawn_edges = limit;
    }

    /// Replaces the number of notes in view above which only the hovered one is labelled. Used by
    /// tests.
    pub fn set_max_labels(&mut self, limit: usize) {
        self.max_labels = limit;
    }

    /// The area the graph was drawn in last frame. Used by tests.
    pub fn pane_rect(&self) -> Option<Rect> {
        self.pane
    }

    /// Where node `id` was drawn last frame, if there is such a node. Used by tests.
    pub fn node_rect(&self, id: &str) -> Option<Rect> {
        let i = self.layout.index_of(id)?;
        let (centre, radius) = self.disc(i);
        Some(Rect::from_center_size(
            centre,
            vec2(2.0 * radius, 2.0 * radius),
        ))
    }

    /// Rebuilds the layout when the index changed since the last frame.
    fn follow(&mut self, workspace: &Workspace) {
        let revision = Some(workspace.index_revision());
        if self.revision != revision {
            self.layout.update(&workspace.index().graph());
            self.revision = revision;
            self.auto_fit = true;
        }
    }

    fn centre(&self) -> [f32; 2] {
        let centre = self.pane.map_or(Pos2::ZERO, |pane| pane.center());
        [centre.x, centre.y]
    }

    fn to_screen(&self, p: [f32; 2]) -> Pos2 {
        let [x, y] = self.viewport.to_screen(self.centre(), p);
        pos2(x, y)
    }

    fn to_world(&self, at: Pos2) -> [f32; 2] {
        self.viewport.to_world(self.centre(), [at.x, at.y])
    }

    /// Shows every node in the pane, as «Fit graph» does. Tests call it too.
    pub fn fit(&mut self) {
        let size = self
            .pane
            .map_or([1.0; 2], |pane| [pane.width(), pane.height()]);
        self.viewport.fit(self.layout.bounds(), size);
    }

    /// Fits the view to the layout while `auto_fit` holds, and stops when the layout is at rest.
    fn keep_fitted(&mut self) {
        if self.auto_fit {
            self.fit();
            self.auto_fit = !self.layout.is_settled();
        }
    }

    /// Applies zoom, pan and drag. Returns true when the user moved the view or a node.
    fn navigate(&mut self, ui: &Ui, response: &Response) -> bool {
        let zoomed = response
            .hover_pos()
            .is_some_and(|pointer| self.zoom_about(ui, pointer));
        if response.drag_started() {
            self.start_drag(ui);
        }
        if response.dragged() {
            self.drag_by(response.drag_delta());
        } else {
            self.dragging = None;
        }
        zoomed || response.drag_started() || response.dragged()
    }

    /// Scroll (or pinch) zooms, keeping the point under `pointer` where it is. Returns true when
    /// it zoomed.
    fn zoom_about(&mut self, ui: &Ui, pointer: Pos2) -> bool {
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta().y, input.zoom_delta()));
        let factor = pinch * (scroll * SCROLL_ZOOM_RATE).exp();
        let offset = pointer - self.pane.map_or(Pos2::ZERO, |pane| pane.center());
        self.viewport.zoom_about([offset.x, offset.y], factor);
        factor != 1.0
    }

    /// A press on a node starts moving it; a press on the background pans the view.
    fn start_drag(&mut self, ui: &Ui) {
        let origin = ui.input(|input| input.pointer.press_origin());
        let node = origin.and_then(|at| self.layout.node_at(self.to_world(at)));
        self.dragging = node.map(|i| (self.layout.id(i).to_owned(), self.layout.position(i)));
    }

    /// Moves the node being dragged, or else the whole view. A node that has left the graph
    /// since the drag began is not moved, and nothing else moves in its place.
    fn drag_by(&mut self, delta: Vec2) {
        let Some((id, at)) = self.dragging.as_mut() else {
            self.viewport.pan_by([delta.x, delta.y]);
            return;
        };
        let Some(node) = self.layout.index_of(id) else {
            return;
        };
        at[0] += delta.x / self.viewport.zoom;
        at[1] += delta.y / self.viewport.zoom;
        self.layout.set_position(node, *at);
    }

    /// Runs layout steps for a frame and asks for the next frame while the layout still moves.
    fn settle(&mut self, ctx: &Context) {
        let dragged = self
            .dragging
            .as_ref()
            .and_then(|(id, at)| Some((self.layout.index_of(id)?, *at)));
        let budget = if self.reduce_motion && dragged.is_none() {
            self.budgets.reduced_motion
        } else {
            self.budgets.animated
        };
        self.enforce_wake_cap(dragged.is_some());
        let started = Instant::now();
        while !self.layout.is_settled() {
            if let Some((node, at)) = dragged {
                self.layout.set_position(node, at);
            }
            self.layout.step();
            if started.elapsed() >= budget {
                break;
            }
        }
        if !self.layout.is_settled() {
            ctx.request_repaint();
        }
    }

    /// Rests the layout once it has animated for `wake_cap`. A drag keeps it animating, and the
    /// clock restarts when it ends.
    fn enforce_wake_cap(&mut self, dragging: bool) {
        if dragging || self.layout.is_settled() {
            self.animating_since = None;
            return;
        }
        let since = *self.animating_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= self.wake_cap {
            self.layout.rest();
            self.animating_since = None;
        }
    }

    fn clicked_node(&self, response: &Response) -> Option<NoteAction> {
        if !response.clicked() {
            return None;
        }
        let at = response.interact_pointer_pos()?;
        let node = self.layout.node_at(self.to_world(at))?;
        let id = self.layout.id(node);
        if self.layout.exists(node) {
            NotePath::parse(id).ok().map(NoteAction::Open)
        } else {
            Some(NoteAction::CreateFromGhost(id.to_owned()))
        }
    }

    /// Draws the pane. Returns true when edges were left out because too many were in view.
    fn paint(&self, scene: &Scene) -> bool {
        let crowded = self.paint_edges(scene);
        // The note meshes go under the ghosts, rings and labels that are drawn as shapes.
        let slot = scene.painter.add(Shape::Noop);
        let mut notes = NoteMeshes {
            discs: Mesh::with_texture(scene.disc),
            dots: Mesh::default(),
            labels: self.labels_fit(scene.painter.clip_rect()),
        };
        for i in 0..self.layout.len() {
            self.paint_node(scene, i, &mut notes);
        }
        let NoteMeshes { discs, dots, .. } = notes;
        scene.painter.set(
            slot,
            Shape::Vec(vec![Shape::mesh(discs), Shape::mesh(dots)]),
        );
        crowded
    }

    /// True when every note in view gets a label: zoomed in far enough, and not too many in view.
    fn labels_fit(&self, clip: Rect) -> bool {
        self.viewport.zoom >= LABEL_ZOOM
            && (0..self.layout.len())
                .filter(|&i| self.is_in_view(clip, i))
                .nth(self.max_labels)
                .is_none()
    }

    /// True when node `i`'s circle may reach into `clip`.
    fn is_in_view(&self, clip: Rect, i: usize) -> bool {
        let (centre, radius) = self.disc(i);
        clip.expand(radius).contains(centre)
    }

    /// 1 normally; `DIMMED` for what is neither the hovered node nor linked to it.
    fn emphasis(&self, hovered: Option<usize>, i: usize) -> f32 {
        match hovered {
            Some(h) if h != i && !self.layout.neighbours(h).contains(&i) => DIMMED,
            _ => 1.0,
        }
    }

    /// Every edge once, as the indices of its two nodes, the smaller first.
    fn edges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.layout.len()).flat_map(|i| {
            let linked = self.layout.neighbours(i).iter();
            linked.filter(move |&&j| j > i).map(move |&j| (i, j))
        })
    }

    /// The visible edges, in one mesh: a shape per edge costs tens of milliseconds to tessellate at
    /// 10 000 notes. Returns true when edges were left out (see `edges_to_draw`).
    fn paint_edges(&self, scene: &Scene) -> bool {
        let screen: Vec<Pos2> = (0..self.layout.len())
            .map(|n| self.to_screen(self.layout.position(n)))
            .collect();
        let (edges, crowded) = self.edges_to_draw(scene, &screen);
        let colour = scene.palette.edge.to_color32();
        let mut mesh = Mesh::default();
        mesh.reserve_vertices(EDGE_VERTICES * edges.len());
        mesh.reserve_triangles(EDGE_TRIANGLES * edges.len());
        let pixel = 1.0 / scene.painter.pixels_per_point();
        for (i, j) in edges {
            let touched = scene.hovered.is_none_or(|h| h == i || h == j);
            let shade = if touched {
                colour
            } else {
                colour.gamma_multiply(DIMMED)
            };
            push_edge(&mut mesh, [screen[i], screen[j]], shade, pixel);
        }
        scene.painter.add(Shape::mesh(mesh));
        crowded
    }

    /// The edges in view, given where every node is on screen. With more than `max_drawn_edges`
    /// of them only those of the hovered and of the open note remain, and the flag is set.
    fn edges_to_draw(&self, scene: &Scene, screen: &[Pos2]) -> (Vec<(usize, usize)>, bool) {
        let clip = scene.painter.clip_rect();
        let in_view = |&(i, j): &(usize, usize)| edge_visible(clip, screen[i], screen[j]);
        // One walk that stops one edge past the limit: a crowded view never collects its (up to
        // 100 000s of) edges.
        let seen: Vec<_> = self
            .edges()
            .filter(in_view)
            .take(self.max_drawn_edges + 1)
            .collect();
        if seen.len() <= self.max_drawn_edges {
            return (seen, false);
        }
        let open = scene.open.and_then(|id| self.layout.index_of(id));
        let focus = [scene.hovered, open].into_iter().flatten();
        let touching = focus.flat_map(|n| self.layout.neighbours(n).iter().map(move |&j| (n, j)));
        let mut edges: Vec<(usize, usize)> = touching
            .map(|(n, j)| (n.min(j), n.max(j)))
            .filter(in_view)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        (edges, true)
    }

    /// Draws node `i`: a note is a quad in `notes.discs`, or a square in `notes.dots` when too
    /// small to show a circle; a ghost is a shape.
    fn paint_node(&self, scene: &Scene, i: usize, notes: &mut NoteMeshes) {
        if !self.is_in_view(scene.painter.clip_rect(), i) {
            return;
        }
        let (centre, radius) = self.disc(i);
        let fill = scene.palette.accent.to_color32();
        let fill = fill.gamma_multiply(self.emphasis(scene.hovered, i));
        if !self.layout.exists(i) {
            self.paint_ghost(scene, i);
        } else if radius < DOT_RADIUS {
            let pixel = 1.0 / scene.painter.pixels_per_point();
            push_dot(&mut notes.dots, centre, (2.0 * radius).max(pixel), fill);
        } else {
            push_disc(&mut notes.discs, centre, radius, fill);
        }
        if scene.open == Some(self.layout.id(i)) {
            let ring = Stroke::new(OPEN_RING_WIDTH, scene.palette.accent.to_color32());
            let ring_radius = radius + OPEN_RING_GAP;
            scene.painter.circle_stroke(centre, ring_radius, ring);
        }
        if notes.labels || scene.hovered == Some(i) {
            self.paint_label(scene, i);
        }
    }

    /// Screen position and radius of node `i`.
    fn disc(&self, i: usize) -> (Pos2, f32) {
        let centre = self.to_screen(self.layout.position(i));
        (centre, self.layout.radius(i) * self.viewport.zoom)
    }

    /// A ghost is a half-transparent disc with an outline.
    fn paint_ghost(&self, scene: &Scene, i: usize) {
        let (centre, radius) = self.disc(i);
        let strength = self.emphasis(scene.hovered, i);
        let muted = scene.palette.text_secondary.to_color32();
        let outline = Stroke::new(OUTLINE_WIDTH, muted.gamma_multiply(strength));
        let fill = muted.gamma_multiply(GHOST_FILL * strength);
        scene.painter.circle(centre, radius, fill, outline);
    }

    fn paint_label(&self, scene: &Scene, i: usize) {
        let label = self.layout.label(i);
        let text = if self.layout.exists(i) {
            label.to_owned()
        } else {
            format!("{label} (not created yet)")
        };
        let strength = self.emphasis(scene.hovered, i);
        let colour = scene
            .palette
            .text_primary
            .to_color32()
            .gamma_multiply(strength);
        let (centre, radius) = self.disc(i);
        let at = centre + vec2(0.0, radius + LABEL_GAP);
        let font = scene.label_font.clone();
        scene
            .painter
            .text(at, Align2::CENTER_TOP, text, font, colour);
    }
}

/// True when the segment's bounding box meets `clip` grown by half the stroke, so a horizontal or
/// vertical edge whose stroke reaches into the view is drawn. A diagonal that only clips a corner
/// of the box may still be drawn; the painter cuts it.
fn edge_visible(clip: Rect, a: Pos2, b: Pos2) -> bool {
    clip.expand(EDGE_WIDTH / 2.0)
        .intersects(Rect::from_two_pos(a, b))
}

/// Adds a line whose core, `pixel` wide (one physical pixel, in points), is in full `colour`, so
/// that wherever it lands on the pixel grid some pixel centre gets the whole token; half a pixel
/// of feathering on each side then fades it to nothing, which keeps it as thin as a 1 px stroke.
fn push_edge(mesh: &mut Mesh, [a, b]: [Pos2; 2], colour: Color32, pixel: f32) {
    let along = (b - a).normalized();
    if !along.is_finite() {
        return;
    }
    let side = vec2(-along.y, along.x);
    let first = mesh.vertices.len() as u32;
    for end in [a, b] {
        for (offset, shade) in [
            (-pixel, Color32::TRANSPARENT),
            (-pixel / 2.0, colour),
            (pixel / 2.0, colour),
            (pixel, Color32::TRANSPARENT),
        ] {
            mesh.colored_vertex(end + side * offset, shade);
        }
    }
    for k in 0..3 {
        mesh.add_triangle(first + k, first + k + 1, first + k + 5);
        mesh.add_triangle(first + k, first + k + 5, first + k + 4);
    }
}

/// A line in the middle of `pane` when there are no notes, and one along its bottom, on a backing
/// of the document surface so that it reads over anything, when not every edge is drawn:
/// `[empty, crowded]`.
fn show_hints(ui: &mut Ui, p: &Palette, pane: Rect, [empty, crowded]: [bool; 2]) {
    let hint = |text| RichText::new(text).color(p.text_secondary.to_color32());
    if empty {
        ui.put(pane, Label::new(hint(EMPTY_HINT)).selectable(false));
    }
    if crowded {
        let font = TextStyle::Body.resolve(ui.style());
        let colour = p.text_secondary.to_color32();
        let wrap = (pane.width() - 2.0 * HINT_PADDING.x).max(0.0);
        let text = ui
            .painter()
            .layout(CROWDED_HINT.to_owned(), font, colour, wrap);
        let size = text.size() + 2.0 * HINT_PADDING;
        let bottom = pane.center_bottom() - vec2(0.0, 16.0);
        let backing = Align2::CENTER_BOTTOM.anchor_size(bottom, size);
        let fill = p.document.to_color32();
        ui.painter()
            .rect_filled(backing, CornerRadius::same(RADIUS), fill);
        let text_area = backing.shrink2(HINT_PADDING);
        ui.put(text_area, Label::new(hint(CROWDED_HINT)).selectable(false));
    }
}

/// The two meshes the notes are drawn into, so that 10 000 of them cost two draw calls.
struct NoteMeshes {
    /// A textured quad per note, tinted with its colour.
    discs: Mesh,
    /// A plain square per note too small for a circle.
    dots: Mesh,
    /// Whether every note in view is labelled (see `GraphView::labels_fit`).
    labels: bool,
}

/// Adds a square of `side` points, which no one can tell apart from a circle that small. The
/// caller keeps `side` at one physical pixel or more: a smaller square covers no pixel.
fn push_dot(mesh: &mut Mesh, centre: Pos2, side: f32, colour: Color32) {
    mesh.add_colored_rect(Rect::from_center_size(centre, Vec2::splat(side)), colour);
}

/// Adds the disc texture as a quad around `centre`, tinted with `colour`: four vertices a note.
fn push_disc(mesh: &mut Mesh, centre: Pos2, radius: f32, colour: Color32) {
    let quad = Rect::from_center_size(centre, Vec2::splat(2.0 * radius));
    let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    mesh.add_rect_with_uv(quad, whole, colour);
}

/// White, with an alpha that is 1 inside the disc filling the square and fades over the last
/// texel: antialiased once, tinted per note.
fn disc_image() -> ColorImage {
    let half = DISC_TEXELS as f32 / 2.0;
    let alpha = |k: usize| {
        let x = (k % DISC_TEXELS) as f32 + 0.5;
        let y = (k / DISC_TEXELS) as f32 + 0.5;
        let coverage = (half - (x - half).hypot(y - half)).clamp(0.0, 1.0);
        Color32::from_white_alpha((coverage * 255.0).round() as u8)
    };
    let pixels = (0..DISC_TEXELS * DISC_TEXELS).map(alpha).collect();
    ColorImage::new([DISC_TEXELS; 2], pixels)
}

fn paint_focus_ring(painter: &Painter, p: &Palette, pane: Rect) {
    let ring = Stroke::new(FOCUS_RING_WIDTH, p.accent.to_color32());
    painter.rect_stroke(pane, CornerRadius::ZERO, ring, StrokeKind::Inside);
}

/// True when a key went down while the graph has the keyboard focus.
fn key_pressed(ui: &Ui, response: &Response) -> bool {
    let pressed = |event: &Event| matches!(event, Event::Key { pressed: true, .. });
    response.has_focus() && ui.input(|input| input.events.iter().any(pressed))
}

/// Enter or Space (activate: open the list view) and Home (fit) pressed while the graph has
/// the keyboard focus.
fn focused_keys(ui: &Ui, response: &Response) -> (bool, bool) {
    if !response.has_focus() {
        return (false, false);
    }
    ui.input(|input| {
        let activated = input.key_pressed(Key::Enter) || input.key_pressed(Key::Space);
        (activated, input.key_pressed(Key::Home))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIP: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));

    #[test]
    fn should_draw_an_edge_inside_the_view() {
        assert!(edge_visible(CLIP, pos2(10.0, 10.0), pos2(90.0, 40.0)));
    }

    #[test]
    fn should_skip_an_edge_wholly_to_one_side_of_the_view() {
        assert!(!edge_visible(CLIP, pos2(-50.0, 10.0), pos2(-10.0, 90.0)));
    }

    #[test]
    fn should_draw_an_edge_whose_stroke_reaches_into_the_view() {
        let just_outside = CLIP.right() + EDGE_WIDTH / 2.0 - 0.1;
        assert!(edge_visible(
            CLIP,
            pos2(just_outside, 10.0),
            pos2(just_outside, 90.0)
        ));
        let too_far = CLIP.right() + EDGE_WIDTH / 2.0 + 0.1;
        assert!(!edge_visible(
            CLIP,
            pos2(too_far, 10.0),
            pos2(too_far, 90.0)
        ));
    }

    #[test]
    fn should_draw_an_edge_that_touches_the_view_edge_or_has_no_length() {
        assert!(edge_visible(CLIP, pos2(100.0, 10.0), pos2(150.0, 20.0)));
        assert!(edge_visible(CLIP, pos2(50.0, 50.0), pos2(50.0, 50.0)));
        assert!(!edge_visible(CLIP, pos2(500.0, 500.0), pos2(500.0, 500.0)));
    }

    #[test]
    fn should_draw_an_edge_that_crosses_the_view_with_both_ends_outside() {
        assert!(edge_visible(CLIP, pos2(-50.0, 50.0), pos2(150.0, 60.0)));
    }
}
