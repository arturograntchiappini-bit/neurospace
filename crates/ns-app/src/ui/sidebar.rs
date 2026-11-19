//! The left panel: search field, folder tree and search results (spec 9.2).

use std::collections::BTreeSet;
use std::time::Instant;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    Atom, Button, Color32, Id, IdSalt, Key, KeyboardShortcut, Modifiers, Popup, Rect, Response,
    RichText, Role, ScrollArea, SetOpenCommand, Shape, Stroke, TextEdit, TextStyle, Ui, WidgetInfo,
    vec2,
};
use ns_core::NotePath;

use crate::state::tree_rows::{RowKind, TreeRow, visible_rows};
use crate::state::{FilesPanel, NoteAction, SearchBox, Workspace};
use crate::ui::theme::{self, Palette, Rgba};

/// Text on the search field's hint and its accessible name.
const SEARCH_HINT: &str = "Search notes";

const RENAME_KEY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F2);
const DELETE_KEY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Delete);
const MENU_KEYS: [KeyboardShortcut; 2] = [
    KeyboardShortcut::new(Modifiers::NONE, Key::Menu),
    KeyboardShortcut::new(Modifiers::SHIFT, Key::F10),
];

/// The id of the search field, so Ctrl+F can focus it.
pub fn search_field_id() -> Id {
    Id::unique("sidebar_search")
}

/// A flat, full-width button whose text is `parts` (text and colour) in one wrapping block.
///
/// Its accessible label is the concatenated text, so words, not colour, carry the meaning.
pub fn row_button(ui: &mut Ui, parts: &[(&str, Rgba)]) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let mut job = LayoutJob::default();
    for (text, color) in parts {
        let format = TextFormat::simple(font.clone(), color.to_color32());
        job.append(text, 0.0, format);
    }
    let width = ui.available_width();
    ui.add(
        Button::new(job)
            .frame_when_inactive(false)
            .min_size(vec2(width, 0.0)),
    )
}

/// Draws «FILES», the search field and the tree or the results; returns what the user asked for.
pub fn show(
    ui: &mut Ui,
    p: &Palette,
    workspace: &Workspace,
    files: &mut FilesPanel,
) -> Option<NoteAction> {
    theme::section_label(ui, p, "FILES");
    search_field(ui, &mut files.search);
    if let Some(wait) = files.search.update(workspace, Instant::now()) {
        ui.ctx().request_repaint_after(wait);
    }
    if files.search.text.is_empty() {
        return tree_view(ui, workspace, &mut files.expanded);
    }
    let results = ScrollArea::vertical().auto_shrink(false);
    results
        .show(ui, |ui| results_view(ui, p, &files.search))
        .inner
}

fn search_field(ui: &mut Ui, search: &mut SearchBox) {
    let field = TextEdit::singleline(&mut search.text)
        .id(search_field_id())
        .hint_text(SEARCH_HINT)
        .desired_width(f32::INFINITY);
    ui.add(field).accessible_name(SEARCH_HINT);
}

/// What the visible tree rows asked for in one frame.
#[derive(Default)]
struct RowsOutcome {
    action: Option<NoteAction>,
    toggled: Option<String>,
}

enum RowEvent {
    Toggle(String),
    Act(NoteAction),
}

/// The tree as a scroll area that draws only the rows in view.
fn tree_view(
    ui: &mut Ui,
    workspace: &Workspace,
    expanded: &mut BTreeSet<String>,
) -> Option<NoteAction> {
    let rows = visible_rows(workspace.tree(), expanded);
    let height = row_height(ui);
    let selected = workspace.open_note().map(|open| &open.path);
    let area = ScrollArea::vertical().auto_shrink(false);
    let outcome = area
        .show_rows(ui, height, rows.len(), |ui, range| {
            draw_rows(ui, &rows[range], selected)
        })
        .inner;
    if let Some(path) = outcome.toggled {
        if !expanded.remove(&path) {
            expanded.insert(path);
        }
        ui.ctx().request_repaint();
    }
    outcome.action
}

/// The height of every row: one line of button text with its padding, never under the touch size.
/// `show_rows` needs it equal for all rows, so a long name is cut short, never wrapped.
fn row_height(ui: &Ui) -> f32 {
    let text = ui.text_style_height(&TextStyle::Button);
    let padded = text + 2.0 * ui.spacing().button_padding.y;
    padded.max(ui.spacing().interact_size.y)
}

fn draw_rows(ui: &mut Ui, rows: &[TreeRow], selected: Option<&NotePath>) -> RowsOutcome {
    let mut outcome = RowsOutcome::default();
    for row in rows {
        let indent = row.depth as f32 * ui.spacing().indent;
        let shown = ui.horizontal(|ui| {
            ui.add_space(indent);
            tree_row(ui, row, selected)
        });
        match shown.inner {
            Some(RowEvent::Toggle(path)) => outcome.toggled = Some(path),
            Some(RowEvent::Act(action)) => outcome.action = Some(action),
            None => {}
        }
    }
    outcome
}

/// The tree marks a note as open by its key, not its spelling: the open path may differ in letter
/// case from the tree's (a link written in another case, a case-only rename).
fn is_selected(selected: Option<&NotePath>, note: &NotePath) -> bool {
    selected.is_some_and(|open| open.key() == note.key())
}

/// One row. Row ids follow the row's position, so they hold while scrolling and change when rows
/// above appear or disappear.
fn tree_row(ui: &mut Ui, row: &TreeRow, selected: Option<&NotePath>) -> Option<RowEvent> {
    match &row.kind {
        RowKind::Folder {
            path,
            name,
            expanded,
        } => folder_row(ui, path, name, *expanded),
        RowKind::Note(note) => note_row(ui, note, is_selected(selected, note)),
    }
}

/// A full-width row button, `row_height` tall: a slot for the folder marker, then `name` cut to fit.
/// `selected` is `None` for a folder row, which has no selection state.
fn name_button<'a>(ui: &Ui, selected: Option<bool>, name: &'a str) -> (Button<'a>, IdSalt) {
    let slot = IdSalt::new("marker");
    let side = ui.spacing().icon_width;
    let atoms = (Atom::custom(slot, vec2(side, side)), name);
    let button = match selected {
        Some(selected) => Button::selectable(selected, atoms),
        None => Button::new(atoms).frame_when_inactive(false).frame(true),
    };
    let size = vec2(ui.available_width(), row_height(ui));
    (button.truncate().min_size(size), slot)
}

fn folder_row(ui: &mut Ui, path: &str, name: &str, expanded: bool) -> Option<RowEvent> {
    let (button, slot) = name_button(ui, None, name);
    let shown = button.atom_ui(ui);
    let response = &shown.response;
    // The name may be cut to fit the row; the tooltip and the accessible name keep it whole.
    response.clone().on_hover_text(name);
    response.widget_info(|| WidgetInfo::labeled(Role::DisclosureTriangle, true, name));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_expanded(expanded);
    });
    if let Some(rect) = shown.rect(slot) {
        let color = ui.style().interact(response).fg_stroke.color;
        paint_marker(ui, rect, expanded, color);
    }
    let mut event = response
        .clicked()
        .then(|| RowEvent::Toggle(path.to_owned()));
    row_menu(response, |ui| {
        if ui.button("New note here…").clicked() {
            event = Some(RowEvent::Act(NoteAction::NewIn(path.to_owned())));
            ui.close();
        }
    });
    event
}

/// A triangle in `slot`: pointing right when the folder is closed, down when open.
fn paint_marker(ui: &Ui, slot: Rect, expanded: bool, color: Color32) {
    let centre = slot.center();
    let r = slot.width() * 0.25;
    let offsets = if expanded {
        [(-r, -r / 2.0), (r, -r / 2.0), (0.0, r)]
    } else {
        [(-r / 2.0, -r), (r, 0.0), (-r / 2.0, r)]
    };
    let points = offsets.iter().map(|&(x, y)| centre + vec2(x, y)).collect();
    let marker = Shape::convex_polygon(points, color, Stroke::NONE);
    ui.painter().add(marker);
}

fn note_row(ui: &mut Ui, note: &NotePath, is_open: bool) -> Option<RowEvent> {
    let (button, _) = name_button(ui, Some(is_open), note.stem());
    let response = ui.add(button).on_hover_text(note.stem());
    let mut action = response.clicked().then(|| NoteAction::Open(note.clone()));
    if key_pressed(&response, &RENAME_KEY) {
        action = Some(NoteAction::Rename(note.clone()));
    }
    if key_pressed(&response, &DELETE_KEY) {
        action = Some(NoteAction::Delete(note.clone()));
    }
    row_menu(&response, |ui| {
        if ui.button("Rename…").clicked() {
            action = Some(NoteAction::Rename(note.clone()));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            action = Some(NoteAction::Delete(note.clone()));
            ui.close();
        }
    });
    action.map(RowEvent::Act)
}

/// True when `shortcut` was pressed while the keyboard focus is on `response`.
fn key_pressed(response: &Response, shortcut: &KeyboardShortcut) -> bool {
    response.has_focus()
        && response
            .ctx
            .input_mut(|input| input.consume_shortcut(shortcut))
}

/// The context menu of a row: opened by a right click, or by the Menu key or Shift+F10 while the
/// row has the keyboard focus. It sits under the row.
fn row_menu(response: &Response, items: impl FnOnce(&mut Ui)) {
    let by_key = MENU_KEYS.iter().any(|keys| key_pressed(response, keys));
    let command = if response.secondary_clicked() || by_key {
        Some(SetOpenCommand::Bool(true))
    } else if response.clicked() {
        Some(SetOpenCommand::Bool(false))
    } else {
        None
    };
    let shown = Popup::menu(response).open_memory(command).show(items);
    return_focus_when_closed(response, shown.is_some());
}

/// A menu that just closed leaves the keyboard focus nowhere: give it back to its row, unless the
/// user already moved it elsewhere.
fn return_focus_when_closed(response: &Response, open: bool) {
    let id = response.id.with("menu_was_open");
    let was_open: bool = response.ctx.data(|data| data.get_temp(id)).unwrap_or(false);
    response.ctx.data_mut(|data| data.insert_temp(id, open));
    if was_open && !open && response.ctx.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
}

fn results_view(ui: &mut Ui, p: &Palette, search: &SearchBox) -> Option<NoteAction> {
    if search.has_answer() && search.hits().is_empty() {
        ui.label(RichText::new("No notes found").color(p.text_secondary.to_color32()));
    }
    let mut action = None;
    for hit in search.hits() {
        let snippet = hit
            .snippets
            .first()
            .map_or("", |snippet| snippet.text.as_str());
        let parts = [
            (hit.title.as_str(), p.text_primary),
            ("\n", p.text_secondary),
            (snippet, p.text_secondary),
        ];
        // Without a snippet the row is the title alone: no blank line, no trailing newline.
        let shown = if snippet.is_empty() {
            &parts[..1]
        } else {
            &parts[..]
        };
        if row_button(ui, shown).clicked() {
            action = Some(NoteAction::Open(hit.path.clone()));
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    #[test]
    fn should_mark_the_open_note_whatever_the_letter_case_of_its_path() {
        let open = path("notes/idea.MD");
        assert!(is_selected(Some(&open), &path("Notes/Idea.md")));
        assert!(!is_selected(Some(&open), &path("notes/other.md")));
        assert!(!is_selected(None, &path("Notes/Idea.md")));
    }
}
