//! The graph pane (spec 9.4, appendix A.3): draws the force layout on the document surface and
//! turns clicks on nodes into note actions.

use std::time::{Duration, Instant};

use eframe::egui::{
    Align2, Button, Context, CornerRadius, FontId, Painter, Pos2, Rect, Response, Role, Sense,
    Stroke, StrokeKind, TextStyle, Ui, Vec2, WidgetInfo, vec2,
};
use sb_core::{Graph, NotePath};

use crate::state::graph_layout::Layout;
use crate::state::{NoteAction, Workspace};
use crate::ui::theme::Palette;

const GRAPH_NAME: &str = "Graph of notes";
/// Time one frame may spend on layout steps; at least one step always runs.
const STEP_BUDGET: Duration = Duration::from_millis(8);
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 5.0;
const SCROLL_ZOOM_RATE: f32 = 0.002;
/// From this zoom on every node shows its label.
const LABEL_ZOOM: f32 = 1.5;
const DIMMED: f32 = 0.25;
const GHOST_FILL: f32 = 0.5;
const EDGE_ALPHA: f32 = 0.5;
const EDGE_WIDTH: f32 = 1.0;
const OUTLINE_WIDTH: f32 = 1.0;
const OPEN_RING_WIDTH: f32 = 2.0;
const OPEN_RING_GAP: f32 = 3.0;
const LABEL_GAP: f32 = 3.0;
const FOCUS_RING_WIDTH: f32 = 2.0;

/// What the graph pane remembers between frames: the layout, the view and an ongoing drag.
pub struct GraphView {
    layout: Layout,
    revision: Option<u64>,
    pan: Vec2,
    zoom: f32,
    centre: Pos2,
    dragging: Option<(usize, [f32; 2])>,
}

impl Default for GraphView {
    fn default() -> GraphView {
        GraphView {
            layout: Layout::new(&Graph::default()),
            revision: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            centre: Pos2::ZERO,
            dragging: None,
        }
    }
}

/// What the user asked for in the graph pane this frame.
pub struct GraphOutcome {
    /// A node was clicked.
    pub action: Option<NoteAction>,
    /// «List view» was pressed.
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
    /// Draws the pane in the space left in `ui`: the «List view» button, then the graph.
    pub fn show(&mut self, ui: &mut Ui, p: &Palette, workspace: &Workspace) -> GraphOutcome {
        self.follow(workspace);
        let list_view = ui.add(Button::new("List view").frame_when_inactive(false));
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::labeled(Role::Image, true, GRAPH_NAME));
        self.centre = response.rect.center();
        self.navigate(ui, &response);
        self.settle(ui.ctx());
        let hovered = response
            .hover_pos()
            .and_then(|at| self.layout.node_at(self.to_world(at)));
        let scene = Scene {
            painter: &painter,
            palette: p,
            label_font: TextStyle::Small.resolve(ui.style()),
            open: workspace.open_note().map(|open| open.path.as_str()),
            hovered,
        };
        self.paint(&scene);
        if response.has_focus() {
            let ring = Stroke::new(FOCUS_RING_WIDTH, p.accent.to_color32());
            painter.rect_stroke(response.rect, CornerRadius::ZERO, ring, StrokeKind::Inside);
        }
        GraphOutcome {
            action: self.clicked_node(&response),
            list_view: list_view.clicked(),
        }
    }

    /// True when the layout has come to rest.
    pub fn is_settled(&self) -> bool {
        self.layout.is_settled()
    }

    /// Where node `id` was drawn last frame, if there is such a node. Used by tests.
    pub fn node_rect(&self, id: &str) -> Option<Rect> {
        let i = (0..self.layout.len()).find(|&i| self.layout.id(i) == id)?;
        let side = 2.0 * self.layout.radius(i) * self.zoom;
        let centre = self.to_screen(self.layout.position(i));
        Some(Rect::from_center_size(centre, vec2(side, side)))
    }

    /// Rebuilds the layout when the index changed since the last frame.
    fn follow(&mut self, workspace: &Workspace) {
        let revision = Some(workspace.index_revision());
        if self.revision != revision {
            self.layout.update(&workspace.index().graph());
            self.revision = revision;
        }
    }

    fn to_screen(&self, p: [f32; 2]) -> Pos2 {
        self.centre + self.pan + vec2(p[0], p[1]) * self.zoom
    }

    fn to_world(&self, at: Pos2) -> [f32; 2] {
        let offset = (at - self.centre - self.pan) / self.zoom;
        [offset.x, offset.y]
    }

    fn navigate(&mut self, ui: &Ui, response: &Response) {
        if let Some(pointer) = response.hover_pos() {
            self.zoom_about(ui, pointer);
        }
        if response.drag_started() {
            let origin = ui.input(|input| input.pointer.press_origin());
            let node = origin.and_then(|at| self.layout.node_at(self.to_world(at)));
            self.dragging = node.map(|i| (i, self.layout.position(i)));
        }
        if response.dragged() {
            self.drag_by(response.drag_delta());
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
    }

    /// Scroll (or pinch) zooms, keeping the point under `pointer` where it is.
    fn zoom_about(&mut self, ui: &Ui, pointer: Pos2) {
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta().y, input.zoom_delta()));
        let wanted = self.zoom * pinch * (scroll * SCROLL_ZOOM_RATE).exp();
        let zoom = wanted.clamp(MIN_ZOOM, MAX_ZOOM);
        let anchor = pointer - self.centre;
        self.pan = anchor - (anchor - self.pan) * (zoom / self.zoom);
        self.zoom = zoom;
    }

    /// Moves the node being dragged, or else the whole view.
    fn drag_by(&mut self, delta: Vec2) {
        let Some((node, at)) = self.dragging.as_mut() else {
            self.pan += delta;
            return;
        };
        at[0] += delta.x / self.zoom;
        at[1] += delta.y / self.zoom;
        self.layout.set_position(*node, *at);
    }

    /// Runs layout steps for a frame and asks for the next frame while the layout still moves.
    fn settle(&mut self, ctx: &Context) {
        let started = Instant::now();
        while !self.layout.is_settled() {
            if let Some((node, at)) = self.dragging {
                self.layout.set_position(node, at);
            }
            self.layout.step();
            if started.elapsed() >= STEP_BUDGET {
                break;
            }
        }
        if !self.layout.is_settled() {
            ctx.request_repaint();
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
        let colour = scene.palette.text_secondary.to_color32();
        for i in 0..self.layout.len() {
            for &j in self.layout.neighbours(i).iter().filter(|&&j| j > i) {
                let touched = scene.hovered.is_none_or(|h| h == i || h == j);
                let alpha = if touched {
                    EDGE_ALPHA
                } else {
                    EDGE_ALPHA * DIMMED
                };
                let ends = [i, j].map(|n| self.to_screen(self.layout.position(n)));
                let stroke = Stroke::new(EDGE_WIDTH, colour.gamma_multiply(alpha));
                scene.painter.line_segment(ends, stroke);
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
        if self.zoom >= LABEL_ZOOM || scene.hovered == Some(i) {
            self.paint_label(scene, i);
        }
    }

    /// Screen position and radius of node `i`.
    fn disc(&self, i: usize) -> (Pos2, f32) {
        let centre = self.to_screen(self.layout.position(i));
        (centre, self.layout.radius(i) * self.zoom)
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
        scene.painter.text(
            at,
            Align2::CENTER_TOP,
            text,
            scene.label_font.clone(),
            colour,
        );
    }
}
