//! The `neurospace` desktop binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use eframe::egui::ViewportBuilder;
use ns_app::app::{AppOptions, NeurospaceApp};
use ns_app::ui::logo;

/// Neurospace: Markdown notes with links, graph and peer-to-peer sync.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The vault folder to open; without it the last vault is reopened.
    vault_dir: Option<PathBuf>,
}

fn options(cli: Cli) -> Result<AppOptions, ns_sync::SyncError> {
    let base = ns_sync::base_dir_from(cfg!(windows), &|key| std::env::var(key).ok())?;
    Ok(AppOptions {
        settings_path: base.join("app.json"),
        state_base: base,
        vault: cli.vault_dir,
    })
}

fn native_options() -> eframe::NativeOptions {
    let mut viewport = ViewportBuilder::default()
        .with_title("Neurospace")
        .with_app_id("neurospace")
        .with_transparent(cfg!(windows))
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([720.0, 480.0]);
    if let Some(icon) = logo::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

fn main() -> ExitCode {
    let options = match options(Cli::parse()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("neurospace: {error}");
            return ExitCode::FAILURE;
        }
    };
    let created = Box::new(move |cc: &eframe::CreationContext<'_>| {
        let mut app = NeurospaceApp::new(&cc.egui_ctx, options);
        app.sync_material(cc, cc.egui_ctx.theme());
        Ok(Box::new(app) as Box<dyn eframe::App>)
    });
    match eframe::run_native("Neurospace", native_options(), created) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neurospace: {error}");
            ExitCode::FAILURE
        }
    }
}
