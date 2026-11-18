//! Shared helpers of the UI tests: a headless harness over temporary settings and state.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::Context;
use egui_kittest::Harness;
use sb_app::app::{AppOptions, SecondBrainApp, prepare_context};
use tempfile::TempDir;

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const DEADLINE: Duration = Duration::from_secs(10);

/// A harness over `options`; the caller keeps the temporary directories alive.
pub fn harness_from(options: AppOptions) -> Harness<'static, SecondBrainApp> {
    harness_for(SecondBrainApp::new(&Context::default(), options))
}

/// A harness that shows `app`.
pub fn harness_for(app: SecondBrainApp) -> Harness<'static, SecondBrainApp> {
    // The app prepared its own context when it was made; the harness renders with another one.
    Harness::builder()
        .with_size([1200.0, 800.0])
        .with_context_setup(prepare_context)
        .build_ui_state(|ui, app: &mut SecondBrainApp| app.show(ui), app)
}

/// A harness with temporary settings and state base, optionally starting on `vault`.
pub fn harness(vault: Option<&Path>) -> (Harness<'static, SecondBrainApp>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let options = AppOptions {
        settings_path: dir.path().join("app.json"),
        state_base: dir.path().join("state"),
        vault: vault.map(Path::to_path_buf),
    };
    (harness_from(options), dir)
}

/// Steps the harness every 20 ms until `done` holds; fails after 10 s naming `what`.
pub fn run_until(
    harness: &mut Harness<SecondBrainApp>,
    what: &str,
    done: impl Fn(&SecondBrainApp) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        harness.step();
        if done(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(POLL_INTERVAL);
    }
}
