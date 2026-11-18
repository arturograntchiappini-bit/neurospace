//! Layout of the main window (spec 9.2) on the surfaces of spec 9.6.

use std::time::Instant;

use eframe::egui::{
    Align, Button, CentralPanel, Frame, Layout, Margin, Panel, Popup, Response, RichText, Role,
    Sense, StrokeKind, Ui, WidgetInfo, pos2, vec2,
};

use crate::state::notice::{Notice, NoticeBoard, NoticeKind};
use crate::state::settings::Settings;
use crate::state::{Workspace, panels};
use crate::ui::menu::{self, MenuOutcome};
use crate::ui::theme::{self, GUTTER, Palette};

const ICON_INSET: f32 = 5.0;
const ICON_RADIUS: f32 = 3.0;
const ICON_DIVIDER: f32 = 0.35;
/// A side panel never takes more than this fraction of the window width.
const SIDE_PANEL_MAX_FRACTION: f32 = 0.3;
const GAP: i8 = GUTTER as i8;
const PADDING_X: i8 = 12;
const PADDING_Y: i8 = 8;
const DOCUMENT_PADDING: i8 = 32;
const LEFT_PANEL_WIDTH: f32 = 240.0;
const LEFT_PANEL_MIN: f32 = 180.0;
const RIGHT_PANEL_WIDTH: f32 = 260.0;
const RIGHT_PANEL_MIN: f32 = 200.0;

/// Which side panels are open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sidebars {
    /// The file panel on the left.
    pub left: bool,
    /// The links panel on the right.
    pub right: bool,
}

impl Default for Sidebars {
    fn default() -> Sidebars {
        Sidebars {
            left: true,
            right: true,
        }
    }
}

/// What the top bar shows and may change.
pub struct TopBar<'a> {
    /// Name of the open vault.
    pub vault_name: &'a str,
    /// Open state of the side panels, toggled by «Toggle sidebar».
    pub sidebars: &'a mut Sidebars,
    /// Settings the «Menu» menu edits.
    pub settings: &'a mut Settings,
    /// True while the note is shown rendered: the mode button then offers «Edit».
    pub preview: bool,
}

fn glass_panel_frame(p: &Palette, outer: Margin) -> Frame {
    theme::glass_frame(p)
        .inner_margin(Margin::symmetric(PADDING_X, PADDING_Y))
        .outer_margin(outer)
}

/// Draws the highlight along the top edge of the glass panel `ui` is the content of.
fn paint_panel_highlight(ui: &Ui, p: &Palette) {
    let padding = vec2(f32::from(PADDING_X), f32::from(PADDING_Y));
    theme::paint_top_highlight(ui.painter(), ui.max_rect().expand2(padding), p);
}

fn flat_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(Button::new(text).frame_when_inactive(false))
}

/// What the user asked for in the top bar this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopBarOutcome {
    /// What the «Menu» menu did.
    pub menu: MenuOutcome,
    /// «New» was pressed.
    pub new_note: bool,
    /// «Quick open» was pressed.
    pub quick_open: bool,
    /// «Graph» was pressed.
    pub toggle_graph: bool,
    /// «Edit» or «Preview» was pressed.
    pub toggle_preview: bool,
    /// «Sync» was pressed.
    pub open_sync: bool,
}

/// The top bar: sidebar toggle, actions, vault name, sync, the «Menu» menu and inspector toggle.
pub fn top_bar(ui: &mut Ui, p: &Palette, bar: TopBar<'_>) -> TopBarOutcome {
    let outer = Margin {
        left: GAP,
        right: GAP,
        top: GAP,
        bottom: 0,
    };
    let panel = Panel::top("top_bar")
        .show_separator_line(false)
        .frame(glass_panel_frame(p, outer));
    panel
        .show(ui, |ui| {
            paint_panel_highlight(ui, p);
            ui.horizontal(|ui| top_bar_row(ui, bar)).inner
        })
        .inner
}

/// A button drawn as a window with a divider on one side: the font has no glyph for a menu or
/// panel symbol. It shows and hides the side panel on `on_left`.
fn panel_toggle(ui: &mut Ui, on_left: bool, name: &'static str) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), name));
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        let (fill, ring) = (visuals.weak_bg_fill, visuals.bg_stroke);
        painter.rect(rect, visuals.corner_radius, fill, ring, StrokeKind::Inside);
    }
    let icon = rect.shrink(ICON_INSET);
    painter.rect_stroke(icon, ICON_RADIUS, visuals.fg_stroke, StrokeKind::Inside);
    let inset = icon.width() * ICON_DIVIDER;
    let x = if on_left {
        icon.left() + inset
    } else {
        icon.right() - inset
    };
    let divider = [pos2(x, icon.top()), pos2(x, icon.bottom())];
    painter.line_segment(divider, visuals.fg_stroke);
    response
}

/// Adds `add` at the right end of the row, in visual order (left to right) so that Tab follows
/// the eye. A right-to-left layout would create the widgets in reverse order, so the width of
/// the cluster is measured and remembered from the previous frame instead.
fn right_aligned<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.scope_id().with("right_cluster_width");
    let known: f32 = ui.data(|data| data.get_temp(id)).unwrap_or(0.0);
    ui.add_space((ui.available_width() - known).max(0.0));
    let start = ui.cursor().left();
    let result = add(ui);
    let width = ui.cursor().left() - start;
    if (width - known).abs() > 0.5 {
        ui.data_mut(|data| data.insert_temp(id, width));
        ui.ctx().request_repaint();
    }
    result
}

fn top_bar_row(ui: &mut Ui, bar: TopBar<'_>) -> TopBarOutcome {
    let mut outcome = TopBarOutcome::default();
    if panel_toggle(ui, true, "Toggle sidebar").clicked() {
        bar.sidebars.left = !bar.sidebars.left;
    }
    outcome.new_note = flat_button(ui, "New").clicked();
    outcome.quick_open = flat_button(ui, "Quick open").clicked();
    outcome.toggle_graph = flat_button(ui, "Graph").clicked();
    let mode = if bar.preview { "Edit" } else { "Preview" };
    outcome.toggle_preview = flat_button(ui, mode).clicked();
    outcome.menu = right_aligned(ui, |ui| {
        ui.label(bar.vault_name);
        outcome.open_sync = flat_button(ui, "Sync").clicked();
        let menu = menu::menu_button(ui, bar.settings);
        if panel_toggle(ui, false, "Toggle inspector").clicked() {
            bar.sidebars.right = !bar.sidebars.right;
        }
        menu
    });
    outcome
}

/// What the status strip reports besides the notice.
pub struct Strip<'a> {
    /// The open vault: word count and ignored files.
    pub workspace: &'a Workspace,
    /// The sync summary, shown as a button that opens the sync window.
    pub sync_summary: &'a str,
}

/// The slim glass strip at the bottom: word count, ignored files, the notice, sync summary.
/// True when the sync summary was pressed.
pub fn status_strip(ui: &mut Ui, p: &Palette, notices: &mut NoticeBoard, strip: Strip<'_>) -> bool {
    let now = Instant::now();
    let outer = Margin {
        left: GAP,
        right: GAP,
        top: 0,
        bottom: GAP,
    };
    let frame = theme::strip_frame(p)
        .inner_margin(Margin::symmetric(PADDING_X, 4))
        .outer_margin(outer);
    if let Some(left) = notices.time_left(now) {
        ui.ctx().request_repaint_after(left);
    }
    let panel = Panel::bottom("status_strip")
        .show_separator_line(false)
        .frame(frame);
    panel
        .show(ui, |ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            ui.horizontal(|ui| strip_row(ui, p, strip, notices)).inner
        })
        .inner
}

fn strip_row(ui: &mut Ui, p: &Palette, strip: Strip<'_>, notices: &mut NoticeBoard) -> bool {
    let workspace = strip.workspace;
    if workspace.open_note().is_some() {
        ui.label(panels::counted(workspace.word_count(), "word", "words"));
    }
    ignored_label(ui, workspace);
    if let Some(notice) = notices.current(Instant::now()).cloned() {
        notice_row(ui, p, &notice, notices);
    }
    let layout = Layout::right_to_left(Align::Center);
    ui.with_layout(layout, |ui| flat_button(ui, strip.sync_summary).clicked())
        .inner
}

fn ignored_label(ui: &mut Ui, workspace: &Workspace) {
    let ignored = workspace.index().ignored();
    if ignored.is_empty() {
        return;
    }
    let text = panels::counted(ignored.len(), "file ignored", "files ignored");
    // A button, not a label: keyboard and screen-reader users open the list too.
    let button = flat_button(ui, &text);
    Popup::menu(&button).show(|ui| {
        for line in panels::ignored_lines(ignored) {
            ui.label(line);
        }
    });
}

fn notice_row(ui: &mut Ui, p: &Palette, notice: &Notice, notices: &mut NoticeBoard) {
    match notice.kind {
        NoticeKind::Info => {
            ui.label(RichText::new(&notice.text).color(p.text_secondary.to_color32()));
        }
        NoticeKind::Error => {
            ui.label(format!("Error: {}", notice.text));
            if flat_button(ui, "×").accessible_name("Dismiss").clicked() {
                notices.dismiss();
            }
        }
    }
}

/// The left glass panel; `content` draws inside it. `None` while it is collapsed.
pub fn left_panel<R>(
    ui: &mut Ui,
    p: &Palette,
    open: &mut bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let panel = sized(
        ui,
        Panel::left("left_panel"),
        LEFT_PANEL_WIDTH,
        LEFT_PANEL_MIN,
    )
    .frame(glass_panel_frame(p, side_margin(true)));
    show_side_panel(ui, panel, open, |ui| {
        paint_panel_highlight(ui, p);
        content(ui)
    })
}

/// The right glass panel; `content` draws inside it. `None` while it is collapsed.
pub fn right_panel<R>(
    ui: &mut Ui,
    p: &Palette,
    open: &mut bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let panel = sized(
        ui,
        Panel::right("right_panel"),
        RIGHT_PANEL_WIDTH,
        RIGHT_PANEL_MIN,
    )
    .frame(glass_panel_frame(p, side_margin(false)));
    show_side_panel(ui, panel, open, |ui| {
        paint_panel_highlight(ui, p);
        content(ui)
    })
}

/// Gives `panel` its default and minimum width, both capped so that it never takes more than
/// `SIDE_PANEL_MAX_FRACTION` of the window (the text size can make the window small in points).
fn sized(ui: &Ui, panel: Panel, default: f32, min: f32) -> Panel {
    let max = ui.ctx().content_rect().width() * SIDE_PANEL_MAX_FRACTION;
    // `default_size` widens the range to include itself, so the range comes last.
    panel.default_size(default).size_range(min.min(max)..=max)
}

/// Outer margin of a side panel: the window-edge gutter on its outer side only.
fn side_margin(on_left: bool) -> Margin {
    let (left, right) = if on_left { (GAP, 0) } else { (0, GAP) };
    Margin {
        left,
        right,
        top: GAP,
        bottom: GAP,
    }
}

fn show_side_panel<R>(
    ui: &mut Ui,
    panel: Panel,
    open: &mut bool,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let shown = panel
        .show_separator_line(false)
        .show_collapsible(ui, open, content);
    shown.map(|shown| shown.inner)
}

/// The solid document surface; `content` draws the note, the preview or the graph on it.
pub fn document_area<R>(ui: &mut Ui, p: &Palette, content: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = theme::document_frame(p)
        .inner_margin(Margin::same(DOCUMENT_PADDING))
        .outer_margin(Margin::same(GAP));
    CentralPanel::default().frame(frame).show(ui, content).inner
}
