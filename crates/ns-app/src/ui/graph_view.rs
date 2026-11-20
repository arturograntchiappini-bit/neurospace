//! The graph pane (spec 9.4, appendix A.3): draws the force layout on the document surface and
//! turns clicks on nodes into note actions.

use std::time::{Duration, Instant};

use eframe::egui::{
    Align2, Button, Context, CornerRadius, FontId, Key, Label, Painter, Pos2, Rect, Response,
    RichText, Role, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2, WidgetInfo, pos2, vec2,
};
use ns_core::{Graph, NotePath};

use crate::state::graph_layout::Layout;
use crate::state::graph_viewport::{LABEL_ZOOM, Viewport};
use crate::state::{NoteAction, Workspace};
use crate::ui::theme::Palette;

const GRAPH_NAME: &str = "Graph of notes";
const EMPTY_HINT: &str = "No notes yet";
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
const OUTLINE_WIDTH: f32 = 1.0;
const OPEN_RING_WIDTH: f32 = 2.0;
const OPEN_RING_GAP: f32 = 3.0;
const LABEL_GAP: f32 = 3.0;
const FOCUS_RING_WIDTH: f32 = 2.0;

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
        if fit || home {
            self.fit();
        }
        self.navigate(ui, &response);
        self.settle(ui.ctx());
        let hovered = response
            .hover_pos()
            .and_then(|at| self.layout.node_at(self.to_world(at)));
        self.paint(&Scene {
            painter: &painter,
            palette: p,
            label_font: TextStyle::Small.resolve(ui.style()),
            open: workspace.open_note().map(|open| open.path.as_str()),
            hovered,
        });
        if response.has_focus() {
            paint_focus_ring(&painter, p, response.rect);
        }
        self.show_empty_hint(ui, p, response.rect);
        GraphOutcome {
            action: self.clicked_node(&response).filter(|_| !activated),
            list_view: list_view || activated,
        }
    }

    /// A line in the middle of `pane` when there are no notes.
    fn show_empty_hint(&self, ui: &mut Ui, p: &Palette, pane: Rect) {
        if self.layout.is_empty() {
            let hint = RichText::new(EMPTY_HINT).color(p.text_secondary.to_color32());
            ui.put(pane, Label::new(hint).selectable(false));
        }
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

    /// Shows every node in the pane.
    fn fit(&mut self) {
        let size = self
            .pane
            .map_or([1.0; 2], |pane| [pane.width(), pane.height()]);
        self.viewport.fit(self.layout.bounds(), size);
    }

    fn navigate(&mut self, ui: &Ui, response: &Response) {
        if let Some(pointer) = response.hover_pos() {
            self.zoom_about(ui, pointer);
        }
        if response.drag_started() {
            self.start_drag(ui);
        }
        if response.dragged() {
            self.drag_by(response.drag_delta());
        } else {
            self.dragging = None;
        }
    }

    /// Scroll (or pinch) zooms, keeping the point under `pointer` where it is.
    fn zoom_about(&mut self, ui: &Ui, pointer: Pos2) {
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta().y, input.zoom_delta()));
        let factor = pinch * (scroll * SCROLL_ZOOM_RATE).exp();
        let offset = pointer - self.pane.map_or(Pos2::ZERO, |pane| pane.center());
        self.viewport.zoom_about([offset.x, offset.y], factor);
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

    fn paint(&self, scene: &Scene) {
        self.paint_edges(scene);
        for i in 0..self.layout.len() {
            self.paint_node(scene, i);
        }
    }

    /// 1 normally; `DIMMED` for what is neither the hovered node nor linked to it.
    fn emphasis(&self, hovered: Option<usize>, i: usize) -> f32 {
        match hovered {
            Some(h) if h != i && !self.layout.neighbours(h).contains(&i) => DIMMED,
            _ => 1.0,
        }
    }

    fn paint_edges(&self, scene: &Scene) {
        let colour = scene.palette.edge.to_color32();
        for i in 0..self.layout.len() {
            for &j in self.layout.neighbours(i).iter().filter(|&&j| j > i) {
                let touched = scene.hovered.is_none_or(|h| h == i || h == j);
                let shade = if touched {
                    colour
                } else {
                    colour.gamma_multiply(DIMMED)
                };
                let ends = [i, j].map(|n| self.to_screen(self.layout.position(n)));
                if !edge_visible(scene.painter.clip_rect(), ends[0], ends[1]) {
                    continue;
                }
                scene
                    .painter
                    .line_segment(ends, Stroke::new(EDGE_WIDTH, shade));
            }
        }
    }

    fn paint_node(&self, scene: &Scene, i: usize) {
        let (centre, radius) = self.disc(i);
        if !scene.painter.clip_rect().expand(radius).contains(centre) {
            return;
        }
        self.paint_disc(scene, i);
        if scene.open == Some(self.layout.id(i)) {
            let ring = Stroke::new(OPEN_RING_WIDTH, scene.palette.accent.to_color32());
            let ring_radius = radius + OPEN_RING_GAP;
            scene.painter.circle_stroke(centre, ring_radius, ring);
        }
        if self.viewport.zoom >= LABEL_ZOOM || scene.hovered == Some(i) {
            self.paint_label(scene, i);
        }
    }

    /// Screen position and radius of node `i`.
    fn disc(&self, i: usize) -> (Pos2, f32) {
        let centre = self.to_screen(self.layout.position(i));
        (centre, self.layout.radius(i) * self.viewport.zoom)
    }

    /// A note is a filled accent disc; a ghost is a half-transparent disc with an outline.
    fn paint_disc(&self, scene: &Scene, i: usize) {
        let (centre, radius) = self.disc(i);
        let strength = self.emphasis(scene.hovered, i);
        if self.layout.exists(i) {
            let fill = scene.palette.accent.to_color32().gamma_multiply(strength);
            scene.painter.circle_filled(centre, radius, fill);
            return;
        }
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

fn paint_focus_ring(painter: &Painter, p: &Palette, pane: Rect) {
    let ring = Stroke::new(FOCUS_RING_WIDTH, p.accent.to_color32());
    painter.rect_stroke(pane, CornerRadius::ZERO, ring, StrokeKind::Inside);
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
