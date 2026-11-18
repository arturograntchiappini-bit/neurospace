//! The note in the centre pane: the editor with light Markdown colouring, or the preview
//! with clickable wikilinks (spec 9.2, 9.6, appendix A.3).

use std::sync::Arc;
use std::time::Instant;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    FontId, Frame, Galley, Id, Rect, RichText, ScrollArea, Stroke, TextBuffer, TextEdit, TextStyle,
    Ui, UiBuilder, pos2, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use sb_core::NoteKey;

use crate::state::markup::{self, SpanKind};
use crate::state::{NoteAction, Workspace};
use crate::ui::theme::{Palette, bold_family};

const MAX_LINE_WIDTH: f32 = 760.0;
const EDITOR_HINT: &str = "Write your note";
const LINK_UNDERLINE: f32 = 1.0;
const WEB_LINKS_ONLY: &str = "Only web links open from a note";

/// Whether the note is shown rendered, and the preview's cache.
#[derive(Default)]
pub struct EditorState {
    /// True while the preview replaces the editor.
    pub preview: bool,
    cache: CommonMarkCache,
    /// The note the editor last drew and the id of its `TextEdit`.
    drawn: Option<(NoteKey, Id)>,
}

/// Draws the open note (or a hint when none is open); returns the link the user followed.
pub fn show(
    ui: &mut Ui,
    p: &Palette,
    workspace: &mut Workspace,
    state: &mut EditorState,
) -> Option<NoteAction> {
    if workspace.open_note().is_none() {
        ui.label(RichText::new(NO_NOTE_HINT).color(p.text_secondary.to_color32()));
        return None;
    }
    let key = workspace.open_note().map(|open| open.path.key());
    ScrollArea::vertical()
        .id_salt((key, state.preview))
        .auto_shrink(false)
        .show(ui, |ui| {
            centred_column(ui, |ui| {
                if state.preview {
                    preview(ui, workspace, &mut state.cache)
                } else {
                    edit(ui, p, workspace, state);
                    None
                }
            })
        })
        .inner
}

const NO_NOTE_HINT: &str =
    "No note is open. Pick one in the list, press Ctrl+O to find one, or Ctrl+N to create one.";

/// Runs `add` in a column at most `MAX_LINE_WIDTH` wide, centred in the space left.
fn centred_column<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let space = ui.available_rect_before_wrap();
    let width = space.width().min(MAX_LINE_WIDTH);
    let left = space.left() + (space.width() - width) / 2.0;
    let column = Rect::from_min_size(pos2(left, space.top()), vec2(width, space.height()));
    ui.scope_builder(UiBuilder::new().max_rect(column), add)
        .inner
}

/// The text of the open note in a plain-text editor. The buffer is the workspace's own, read
/// afresh every frame, so a text the reload adopted shows at once. The editor takes the focus
/// when another note opens, and keeps it when the workspace replaces the text under it. Its id
/// changes with the note and with every such replacement, so Ctrl+Z cannot bring back older text.
///
/// shortcut: the buffer is cloned each frame (up to the 4 MiB note limit); upgrade to editing in
/// place if profiling ever shows it.
fn edit(ui: &mut Ui, p: &Palette, workspace: &mut Workspace, state: &mut EditorState) {
    let Some(open) = workspace.open_note() else {
        return;
    };
    let (key, mut text) = (open.path.key(), open.buffer.clone());
    let id = Id::unique(("editor", &key, workspace.buffer_generation()));
    let focus = take_focus(ui, state, &key, id);
    let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, wrap_width: f32| {
        colour(ui, p, buffer.as_str(), wrap_width)
    };
    let editor = TextEdit::multiline(&mut text)
        .id(id)
        .frame(Frame::NONE)
        .font(TextStyle::Monospace)
        .hint_text(EDITOR_HINT)
        .desired_width(f32::INFINITY)
        .min_size(vec2(0.0, ui.clip_rect().height()))
        .layouter(&mut layouter);
    let response = ui.add(editor).accessible_name(EDITOR_HINT);
    if focus {
        response.request_focus();
    }
    if response.changed() {
        workspace.edit(text, Instant::now());
    }
}

/// Records the note and id being drawn; true when the editor should take the focus now: a
/// different note, or the same one with the focus on the id it replaces.
fn take_focus(ui: &Ui, state: &mut EditorState, key: &NoteKey, id: Id) -> bool {
    let previous = state.drawn.replace((key.clone(), id));
    let Some((old_key, old_id)) = previous else {
        return true;
    };
    old_key != *key || (old_id != id && ui.memory(|memory| memory.has_focus(old_id)))
}

/// Lays `text` out in the monospace font: headings semi-bold, links accent and underlined, code
/// in the secondary colour on a hover-coloured background.
fn colour(ui: &Ui, p: &Palette, text: &str, wrap_width: f32) -> Arc<Galley> {
    let mono = TextStyle::Monospace.resolve(ui.style());
    let heading = FontId::new(mono.size, bold_family());
    let primary = p.text_primary.to_color32();
    let mut job = LayoutJob::default();
    for (range, kind) in markup::highlight(text) {
        let format = match kind {
            SpanKind::Plain => TextFormat::simple(mono.clone(), primary),
            SpanKind::Heading => TextFormat::simple(heading.clone(), primary),
            SpanKind::Link => TextFormat {
                underline: Stroke::new(LINK_UNDERLINE, p.accent.to_color32()),
                ..TextFormat::simple(mono.clone(), p.accent.to_color32())
            },
            SpanKind::Code => TextFormat {
                background: p.hover.to_color32(),
                ..TextFormat::simple(mono.clone(), p.text_secondary.to_color32())
            },
        };
        job.append(&text[range], 0.0, format);
    }
    if job.sections.is_empty() {
        // An empty layout has no height, and the caret would be invisible.
        job.append("", 0.0, TextFormat::simple(mono, primary));
    }
    job.wrap.max_width = wrap_width;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The rendered note. Wikilinks became links to `sb-note:` destinations; those and every link
/// that must not open (anything but http and https) are registered as link hooks, so that a click
/// is reported to us instead of opening a browser or a file.
fn preview(ui: &mut Ui, workspace: &Workspace, cache: &mut CommonMarkCache) -> Option<NoteAction> {
    let open = workspace.open_note()?;
    let markdown = markup::wikilinks_to_markdown(&open.buffer);
    let destinations = markup::note_link_destinations(&markdown);
    let blocked = markup::blocked_link_destinations(&markdown);
    cache.link_hooks_clear();
    for destination in destinations.iter().chain(&blocked) {
        cache.add_link_hook(destination.clone());
    }
    CommonMarkViewer::new().show(ui, cache, &markdown);
    let clicked = |destination: &&String| cache.get_link_hook(destination) == Some(true);
    if let Some(destination) = destinations.iter().find(clicked) {
        return markup::note_link_target(destination).map(NoteAction::Follow);
    }
    blocked
        .iter()
        .any(|destination| clicked(&destination))
        .then(|| NoteAction::Inform(WEB_LINKS_ONLY.to_owned()))
}

#[cfg(test)]
mod tests {
    use eframe::egui::{CentralPanel, Context, Theme};

    use super::*;
    use crate::ui::theme::{Material, palette};

    #[test]
    fn should_lay_out_an_empty_note_with_a_visible_caret_height() {
        let ctx = Context::default();
        let p = palette(Theme::Light, Material::Opaque);
        let mut height = 0.0;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            CentralPanel::default().show(ui, |ui| {
                height = colour(ui, &p, "", 300.0).size().y;
            });
        });
        output.textures_delta.clear();
        assert!(height > 0.0, "an empty note lays out {height} high");
    }
}
