//! The «New note» and «Rename note» dialogs (spec 9.3), modal and closed by Escape.

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{Context, Id, Key, Margin, Modal, RichText, TextEdit, Ui};

use crate::state::{DialogKind, NoteDialog, Workspace};
use crate::ui::theme::{self, Palette};

const DIALOG_WIDTH: f32 = 340.0;
const DIALOG_PADDING: i8 = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Confirm,
    Cancel,
}

/// Shows the open dialog, if any, and carries out what the user chose.
pub fn show(
    ctx: &Context,
    p: &Palette,
    dialog: &mut Option<NoteDialog>,
    workspace: &mut Workspace,
) {
    let Some(open) = dialog.as_mut() else {
        return;
    };
    let frame = theme::document_frame(p).inner_margin(Margin::same(DIALOG_PADDING));
    let modal = Modal::new(Id::unique("note_dialog"))
        .frame(frame)
        .show(ctx, |ui| contents(ui, p, open));
    let choice = match modal.inner {
        Some(choice) => Some(choice),
        None => modal.should_close().then_some(Choice::Cancel),
    };
    let finished = match choice {
        Some(Choice::Confirm) => open.confirm(workspace),
        Some(Choice::Cancel) => true,
        None => false,
    };
    if finished {
        *dialog = None;
    }
}

fn contents(ui: &mut Ui, p: &Palette, dialog: &mut NoteDialog) -> Option<Choice> {
    ui.set_width(DIALOG_WIDTH);
    ui.heading(dialog.title());
    if let DialogKind::NewNote(Some(folder)) = &dialog.kind {
        let text = RichText::new(format!("In {folder}")).color(p.text_secondary.to_color32());
        ui.label(text);
    }
    let entered = name_field(ui, dialog);
    if let Some(error) = &dialog.error {
        ui.label(RichText::new(error).color(p.text_secondary.to_color32()));
    }
    let clicked = buttons(ui, dialog);
    clicked.or(entered.then_some(Choice::Confirm))
}

/// Draws the name field; true when Enter was pressed in it.
fn name_field(ui: &mut Ui, dialog: &mut NoteDialog) -> bool {
    let name = dialog.field_name();
    let output = TextEdit::singleline(&mut dialog.name)
        .hint_text(name)
        .desired_width(f32::INFINITY)
        .show(ui);
    output.response.response.clone().accessible_name(name);
    if !dialog.focused {
        output.response.request_focus();
        dialog.focused = output.response.has_focus();
        if dialog.focused {
            let length = dialog.name.chars().count();
            select_all(ui.ctx(), output.state, output.response.id, length);
        }
    }
    output.response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

/// Selects the whole text so typing replaces the prefilled name.
fn select_all(ctx: &Context, mut state: TextEditState, id: Id, length: usize) {
    let range = CCursorRange::two(CCursor::new(0), CCursor::new(length));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

fn buttons(ui: &mut Ui, dialog: &NoteDialog) -> Option<Choice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.button(dialog.confirm_label()).clicked() {
            choice = Some(Choice::Confirm);
        }
        if ui.button("Cancel").clicked() {
            choice = Some(Choice::Cancel);
        }
    });
    choice
}
