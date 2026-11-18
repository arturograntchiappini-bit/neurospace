//! The `secondbrain` desktop binary.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use eframe::egui::ViewportBuilder;
use sb_app::app::{AppOptions, SecondBrainApp};

/// Second Brain: Markdown notes with links, graph and peer-to-peer sync.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The vault folder to open; without it the last vault is reopened.
    vault_dir: Option<PathBuf>,
}

fn options(cli: Cli) -> Result<AppOptions, sb_sync::SyncError> {
    let base = sb_sync::base_dir_from(cfg!(windows), &|key| std::env::var(key).ok())?;
    Ok(AppOptions {
        settings_path: base.join("app.json"),
        state_base: base,
        vault: cli.vault_dir,
    })
}

fn native_options() -> eframe::NativeOptions {
    let viewport = ViewportBuilder::default()
        .with_title("Second Brain")
        .with_app_id("secondbrain")
        .with_transparent(cfg!(windows))
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([720.0, 480.0]);
    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

fn main() -> ExitCode {
    let options = match options(Cli::parse()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("secondbrain: {error}");
            return ExitCode::FAILURE;
        }
    };
    let created = Box::new(move |cc: &eframe::CreationContext<'_>| {
        let mut app = SecondBrainApp::new(&cc.egui_ctx, options);
        app.sync_material(cc, cc.egui_ctx.theme());
        Ok(Box::new(app) as Box<dyn eframe::App>)
    });
    match eframe::run_native("Second Brain", native_options(), created) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("secondbrain: {error}");
            ExitCode::FAILURE
        }
    }
}
