//! The quick-open popup (spec 9.3): a centred modal that filters notes by name as you type.

use eframe::egui::{Context, Id, Key, Margin, Modal, Modifiers, RichText, TextEdit, Ui};
use sb_core::NotePath;

use crate::state::{NoteAction, QuickOpen};
use crate::ui::theme::{self, Palette};

const POPUP_WIDTH: f32 = 420.0;
const POPUP_PADDING: i8 = 20;
const FIELD_HINT: &str = "Find a note";

/// Shows the popup while `quick` is `Some`. Returns the action to run when a note was chosen;
/// the popup closes on a choice and on Escape.
pub fn show(
    ctx: &Context,
    p: &Palette,
    quick: &mut Option<QuickOpen>,
    workspace: &crate::state::Workspace,
) -> Option<NoteAction> {
    let open = quick.as_mut()?;
    let matches = open.matches(workspace);
    let frame = theme::document_frame(p).inner_margin(Margin::same(POPUP_PADDING));
    let modal = Modal::new(Id::unique("quick_open"))
        .frame(frame)
        .show(ctx, |ui| contents(ui, p, open, &matches));
    let closed = modal.should_close();
    let chosen = modal.inner;
    if chosen.is_some() || closed {
        *quick = None;
    }
    chosen.map(NoteAction::Open)
}

fn contents(
    ui: &mut Ui,
    p: &Palette,
    quick: &mut QuickOpen,
    matches: &[NotePath],
) -> Option<NotePath> {
    ui.set_width(POPUP_WIDTH);
    // Taken before the field sees them: in a one-line field the arrows would move the cursor.
    let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    if down || up {
        quick.step(down, matches.len());
    }
    let entered = query_field(ui, quick);
    let highlighted = quick.highlighted(matches.len());
    if matches.is_empty() && !quick.query.is_empty() {
        ui.label(RichText::new("No note matches").color(p.text_secondary.to_color32()));
    }
    let clicked = rows(ui, matches, highlighted);
    let pressed = entered.then(|| matches.get(highlighted).cloned()).flatten();
    clicked.or(pressed)
}

/// Draws the field; true when Enter was pressed in it.
fn query_field(ui: &mut Ui, quick: &mut QuickOpen) -> bool {
    let field = TextEdit::singleline(&mut quick.query)
        .hint_text(FIELD_HINT)
        .desired_width(f32::INFINITY);
    let response = ui.add(field).accessible_name(FIELD_HINT);
    if response.changed() {
        quick.reset_highlight();
    }
    if !quick.focused {
        response.request_focus();
        quick.focused = response.has_focus();
    }
    response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
}

fn rows(ui: &mut Ui, matches: &[NotePath], highlighted: usize) -> Option<NotePath> {
    let mut chosen = None;
    for (row, path) in matches.iter().enumerate() {
        if ui
            .selectable_label(row == highlighted, path.as_str())
            .clicked()
        {
            chosen = Some(path.clone());
        }
    }
    chosen
}
