//! The welcome screen: pick a folder or type its path (spec 9.1).

use std::path::PathBuf;

use eframe::egui::{Button, CentralPanel, Key, Margin, RichText, TextEdit, Ui};

use crate::ui::theme::{self, Palette};

const CARD_WIDTH: f32 = 420.0;
const CARD_HEIGHT: f32 = 240.0;
const CARD_PADDING: i8 = 24;

/// What the welcome screen shows and what the user has typed.
#[derive(Debug, Default)]
pub struct WelcomeForm {
    /// The text of the path field.
    pub path: String,
    /// Why the last attempt to open a vault failed.
    pub error: Option<String>,
}

/// Draws the welcome card; returns the folder the user asked to open, if any.
pub fn show(ui: &mut Ui, p: &Palette, form: &mut WelcomeForm) -> Option<PathBuf> {
    let mut chosen = None;
    CentralPanel::no_frame().show(ui, |ui| {
        ui.add_space(((ui.available_height() - CARD_HEIGHT) / 2.0).max(0.0));
        ui.vertical_centered(|ui| {
            let card = theme::glass_frame(p).inner_margin(Margin::same(CARD_PADDING));
            card.show(ui, |ui| {
                ui.set_width(CARD_WIDTH);
                chosen = card_contents(ui, p, form);
            });
        });
    });
    chosen
}

fn card_contents(ui: &mut Ui, p: &Palette, form: &mut WelcomeForm) -> Option<PathBuf> {
    ui.heading("Second Brain");
    ui.label("Choose the folder that holds your notes.");
    ui.add_space(8.0);
    let mut chosen = None;
    if ui.button("Open a folder").clicked() {
        chosen = rfd::FileDialog::new().pick_folder();
        if let Some(folder) = &chosen {
            form.path = folder.display().to_string();
        }
    }
    let field = TextEdit::singleline(&mut form.path)
        .hint_text("Vault folder path")
        .desired_width(f32::INFINITY);
    let field = ui.add(field).accessible_name("Vault folder path");
    let typed = !form.path.trim().is_empty();
    let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let open = ui.add_enabled(typed, Button::new("Open")).clicked();
    if typed && (open || enter) {
        chosen = Some(PathBuf::from(form.path.trim()));
    }
    if let Some(error) = &form.error {
        ui.label(RichText::new(error).color(p.text_secondary.to_color32()));
    }
    chosen
}
