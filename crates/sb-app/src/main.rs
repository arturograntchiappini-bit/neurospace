//! The `secondbrain` desktop binary.

use eframe::egui::{Context, Theme, ViewportBuilder};
use sb_app::ui::theme::{self, Material};

/// Empty window with the backdrop; Task 5 replaces it with the real application.
struct BlankApp;

impl eframe::App for BlankApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx();
        theme::paint_backdrop(ctx, &theme::palette(ctx.theme(), Material::Simulated));
    }
}

fn install_theme(ctx: &Context) {
    theme::install_fonts(ctx);
    for theme in [Theme::Light, Theme::Dark] {
        theme::apply(ctx, theme, &theme::palette(theme, Material::Simulated));
    }
}

fn main() -> eframe::Result {
    let viewport = ViewportBuilder::default()
        .with_title("Second Brain")
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([720.0, 480.0]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Second Brain",
        options,
        Box::new(|cc| {
            install_theme(&cc.egui_ctx);
            Ok(Box::new(BlankApp))
        }),
    )
}
