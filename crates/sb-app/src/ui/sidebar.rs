//! The left panel: search field, folder tree and search results (spec 9.2).

use std::time::Instant;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    Button, CollapsingHeader, Id, Key, KeyboardShortcut, Modifiers, Popup, Response, RichText,
    ScrollArea, SetOpenCommand, TextEdit, TextStyle, Ui, vec2,
};
use sb_core::NotePath;

use crate::state::tree::Folder;
use crate::state::{NoteAction, SearchBox, Workspace};
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
    search: &mut SearchBox,
) -> Option<NoteAction> {
    theme::section_label(ui, p, "FILES");
    search_field(ui, search);
    if let Some(wait) = search.update(workspace, Instant::now()) {
        ui.ctx().request_repaint_after(wait);
    }
    ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if search.text.is_empty() {
                tree_view(ui, workspace)
            } else {
                results_view(ui, p, search)
            }
        })
        .inner
}

fn search_field(ui: &mut Ui, search: &mut SearchBox) {
    let field = TextEdit::singleline(&mut search.text)
        .id(search_field_id())
        .hint_text(SEARCH_HINT)
        .desired_width(f32::INFINITY);
    ui.add(field).accessible_name(SEARCH_HINT);
}

fn tree_view(ui: &mut Ui, workspace: &Workspace) -> Option<NoteAction> {
    let selected = workspace.open_note().map(|open| &open.path);
    folder_contents(ui, workspace.tree(), selected)
}

fn folder_contents(
    ui: &mut Ui,
    folder: &Folder,
    selected: Option<&NotePath>,
) -> Option<NoteAction> {
    let mut action = None;
    for child in &folder.folders {
        action = action.or(folder_row(ui, child, selected));
    }
    for note in &folder.notes {
        action = action.or(note_row(ui, note, selected == Some(note)));
    }
    action
}

fn folder_row(ui: &mut Ui, folder: &Folder, selected: Option<&NotePath>) -> Option<NoteAction> {
    let shown = CollapsingHeader::new(&folder.name)
        .id_salt(folder.path.as_str())
        .show(ui, |ui| folder_contents(ui, folder, selected));
    let mut action = None;
    row_menu(&shown.header_response, |ui| {
        if ui.button("New note here…").clicked() {
            action = Some(NoteAction::NewIn(folder.path.clone()));
            ui.close();
        }
    });
    action.or(shown.body_returned.flatten())
}

fn note_row(ui: &mut Ui, note: &NotePath, is_open: bool) -> Option<NoteAction> {
    let width = ui.available_width();
    let row = Button::selectable(is_open, note.stem()).min_size(vec2(width, 0.0));
    let response = ui.add(row);
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
    action
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
    Popup::menu(response).open_memory(command).show(items);
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
