//! Layout of the main window (spec 9.2) on the surfaces of spec 9.6.

use std::time::Instant;

use eframe::egui::{
    Align, Button, CentralPanel, Frame, Layout, Margin, Panel, Response, RichText, Sense, Ui, pos2,
    vec2,
};

use crate::state::notice::{Notice, NoticeBoard, NoticeKind};
use crate::state::settings::Settings;
use crate::state::{Workspace, panels};
use crate::ui::menu::{self, MenuOutcome};
use crate::ui::theme::{self, GUTTER, Palette};

const BAR_HALF_WIDTH: f32 = 7.0;
const BAR_SPACING: f32 = 5.0;
const GAP: i8 = GUTTER as i8;
const PADDING_X: i8 = 12;
const PADDING_Y: i8 = 8;
const DOCUMENT_PADDING: i8 = 16;
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

/// The top bar: sidebar toggle, actions, vault name, sync and the «Menu» menu.
pub fn top_bar(ui: &mut Ui, p: &Palette, bar: TopBar<'_>) -> MenuOutcome {
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

/// A button drawn as three bars: the font has no glyph for «Toggle sidebar».
fn sidebar_toggle(ui: &mut Ui) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
    }
    for offset in [-BAR_SPACING, 0.0, BAR_SPACING] {
        let y = rect.center().y + offset;
        let (left, right) = (
            rect.center().x - BAR_HALF_WIDTH,
            rect.center().x + BAR_HALF_WIDTH,
        );
        painter.line_segment([pos2(left, y), pos2(right, y)], visuals.fg_stroke);
    }
    response.accessible_name("Toggle sidebar")
}

fn top_bar_row(ui: &mut Ui, bar: TopBar<'_>) -> MenuOutcome {
    if sidebar_toggle(ui).clicked() {
        bar.sidebars.left = !bar.sidebars.left;
    }
    for action in ["New", "Quick open", "Graph", "Edit"] {
        flat_button(ui, action);
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let outcome = menu::menu_button(ui, bar.settings);
        flat_button(ui, "Sync");
        ui.label(bar.vault_name);
        outcome
    })
    .inner
}

/// The slim glass strip at the bottom: word count, ignored files, the notice, sync summary.
pub fn status_strip(ui: &mut Ui, p: &Palette, notices: &mut NoticeBoard, workspace: &Workspace) {
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
    Panel::bottom("status_strip")
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            ui.horizontal(|ui| strip_row(ui, p, workspace, notices));
        });
}

fn strip_row(ui: &mut Ui, p: &Palette, workspace: &Workspace, notices: &mut NoticeBoard) {
    if workspace.open_note().is_some() {
        ui.label(panels::counted(workspace.word_count(), "word", "words"));
    }
    ignored_label(ui, workspace);
    if let Some(notice) = notices.current(Instant::now()).cloned() {
        notice_row(ui, p, &notice, notices);
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        // The sync engine (a later task) replaces this with its summary.
        ui.label(RichText::new("Sync off").color(p.text_secondary.to_color32()));
    });
}

fn ignored_label(ui: &mut Ui, workspace: &Workspace) {
    let ignored = workspace.index().ignored();
    if ignored.is_empty() {
        return;
    }
    let text = panels::counted(ignored.len(), "file ignored", "files ignored");
    ui.label(text).on_hover_ui(|ui| {
        for file in ignored {
            ui.label(format!(
                "{}: {}",
                file.path,
                panels::ignore_reason_text(file.reason)
            ));
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
    let panel = Panel::left("left_panel")
        .default_size(LEFT_PANEL_WIDTH)
        .min_size(LEFT_PANEL_MIN)
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
    let panel = Panel::right("right_panel")
        .default_size(RIGHT_PANEL_WIDTH)
        .min_size(RIGHT_PANEL_MIN)
        .frame(glass_panel_frame(p, side_margin(false)));
    show_side_panel(ui, panel, open, |ui| {
        paint_panel_highlight(ui, p);
        content(ui)
    })
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

/// The solid document surface; empty until later tasks fill it.
pub fn document_area(ui: &mut Ui, p: &Palette) {
    let frame = theme::document_frame(p)
        .inner_margin(Margin::same(DOCUMENT_PADDING))
        .outer_margin(Margin::same(GAP));
    CentralPanel::default().frame(frame).show(ui, |_ui| {});
}
