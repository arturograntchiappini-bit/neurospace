//! The «Menu» menu and the text size keys (spec 9.2).

use eframe::egui::{Context, Key, KeyboardShortcut, Modifiers, Ui};

use crate::state::settings::{Settings, ThemeChoice, ZOOM_STEP};

/// What the user did in the menu this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuOutcome {
    /// A setting changed and must be saved.
    pub settings_changed: bool,
    /// «Open another vault» was chosen.
    pub open_another_vault: bool,
}

/// Draws the «Menu» button and, while open, its menu.
pub fn menu_button(ui: &mut Ui, settings: &mut Settings) -> MenuOutcome {
    let mut outcome = MenuOutcome::default();
    let button = ui.menu_button("···", |ui| outcome = menu_items(ui, settings));
    button.response.accessible_name("Menu");
    outcome
}

fn menu_items(ui: &mut Ui, settings: &mut Settings) -> MenuOutcome {
    let mut outcome = MenuOutcome::default();
    ui.label("Theme");
    for (choice, name) in [
        (ThemeChoice::System, "System"),
        (ThemeChoice::Light, "Light"),
        (ThemeChoice::Dark, "Dark"),
    ] {
        outcome.settings_changed |= ui
            .selectable_value(&mut settings.theme, choice, name)
            .changed();
    }
    ui.add_space(4.0);
    let reduce = ui.checkbox(&mut settings.reduce_transparency, "Reduce transparency");
    outcome.settings_changed |= reduce.changed();
    let motion = ui.checkbox(&mut settings.reduce_motion, "Reduce motion");
    outcome.settings_changed |= motion.changed();
    ui.add_space(4.0);
    outcome.settings_changed |= text_size_items(ui, settings);
    ui.add_space(4.0);
    if ui.button("Open another vault").clicked() {
        outcome.open_another_vault = true;
        ui.close();
    }
    outcome
}

fn text_size_items(ui: &mut Ui, settings: &mut Settings) -> bool {
    let before = settings.zoom;
    ui.label("Text size");
    if ui.button("Smaller").clicked() {
        settings.step_zoom(-ZOOM_STEP);
    }
    if ui.button("Larger").clicked() {
        settings.step_zoom(ZOOM_STEP);
    }
    if ui.button("Reset").clicked() {
        settings.zoom = Settings::default().zoom;
    }
    settings.zoom != before
}

/// Applies Ctrl+Plus, Ctrl+Minus and Ctrl+0 to `settings`; true when the text size changed.
///
/// The toolkit's own zoom keys are switched off by the application so this is the only rule.
pub fn handle_zoom_keys(ctx: &Context, settings: &mut Settings) -> bool {
    let before = settings.zoom;
    let pressed = |key| {
        let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, key);
        ctx.input_mut(|input| input.consume_shortcut(&shortcut))
    };
    if pressed(Key::Num0) {
        settings.zoom = Settings::default().zoom;
    } else if pressed(Key::Plus) || pressed(Key::Equals) {
        settings.step_zoom(ZOOM_STEP);
    } else if pressed(Key::Minus) {
        settings.step_zoom(-ZOOM_STEP);
    }
    settings.zoom != before
}
