//! The welcome screen: pick a folder or type its path (spec 9.1).

use std::path::PathBuf;

use eframe::egui::load::SizedTexture;
use eframe::egui::{Button, CentralPanel, Image, Key, Margin, RichText, TextEdit, Ui, vec2};

use crate::state::welcome::WelcomeForm;
use crate::ui::logo::Logo;
use crate::ui::theme::{self, Palette};

const CARD_WIDTH: f32 = 420.0;
const LOGO_SIZE: f32 = 96.0;
const CARD_HEIGHT: f32 = 344.0;
const CARD_PADDING: i8 = 24;

/// Draws the welcome card; returns the folder the user asked to open, if any.
pub fn show(ui: &mut Ui, p: &Palette, form: &mut WelcomeForm, logo: &Logo) -> Option<PathBuf> {
    let mut chosen = form.poll_picker();
    CentralPanel::no_frame().show(ui, |ui| {
        ui.add_space(((ui.available_height() - CARD_HEIGHT) / 2.0).max(0.0));
        ui.vertical_centered(|ui| {
            let card = theme::glass_frame(p).inner_margin(Margin::same(CARD_PADDING));
            card.show(ui, |ui| {
                ui.set_width(CARD_WIDTH);
                show_logo(ui, logo);
                chosen = card_contents(ui, p, form).or(chosen.take());
            });
        });
    });
    chosen
}

fn show_logo(ui: &mut Ui, logo: &Logo) {
    let Some(texture) = logo.texture(ui.ctx()) else {
        return;
    };
    ui.vertical_centered(|ui| {
        let image = Image::from_texture(SizedTexture::from(texture));
        ui.add(
            image
                .fit_to_exact_size(vec2(LOGO_SIZE, LOGO_SIZE))
                .alt_text("Neurospace"),
        );
    });
    ui.add_space(8.0);
}

fn card_contents(ui: &mut Ui, p: &Palette, form: &mut WelcomeForm) -> Option<PathBuf> {
    ui.heading("Neurospace");
    ui.label("Choose the folder that holds your notes.");
    ui.add_space(8.0);
    let mut chosen = None;
    let pick = ui.add_enabled(!form.is_picking(), Button::new("Open a folder"));
    if pick.clicked() {
        let ctx = ui.ctx().clone();
        let dialog = || rfd::FileDialog::new().pick_folder();
        form.start_picker(dialog, move || ctx.request_repaint());
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
