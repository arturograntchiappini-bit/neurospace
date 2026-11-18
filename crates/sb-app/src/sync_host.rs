//! What the application keeps for the sync engine of the open vault: the worker, the window's
//! model and the vault switch, none of which ever waits on the engine (constraint 7).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::Context;

use crate::state::sync_panel::{Request, SyncControl, SyncPanel, SyncState, SyncWorker};
use crate::state::{Job, Poll};
use crate::ui::sync_window::{self, SyncWindowOutcome};
use crate::ui::theme::Palette;

/// How long a vault switch or an exit waits for the old engine to release the vault.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Makes the control of the vault at a path; may fail, and runs on a background thread.
pub type ControlFactory = Arc<dyn Fn(&Path) -> Result<Box<dyn SyncControl>, String> + Send + Sync>;

pub(crate) struct SyncHost {
    factory: ControlFactory,
    pub panel: SyncPanel,
    worker: Option<SyncWorker>,
    starting: Option<Job<Result<SyncWorker, String>>>,
    pub window_open: bool,
    leaving: bool,
}

impl SyncHost {
    pub fn new(factory: ControlFactory) -> SyncHost {
        SyncHost {
            factory,
            panel: SyncPanel::default(),
            worker: None,
            starting: None,
            window_open: false,
            leaving: false,
        }
    }

    /// Stops the engine of the previous vault, then starts the worker of the vault at `path`,
    /// both on one background thread so that two engines never race for a lock.
    pub fn switch_to(&mut self, path: &Path, ctx: &Context) {
        let (previous, factory, path) = (self.worker.take(), self.factory.clone(), path.to_owned());
        let wake = ctx.clone();
        let spawn_wake = ctx.clone();
        self.panel = SyncPanel::default();
        self.window_open = false;
        self.leaving = false;
        self.starting = Some(Job::spawn(
            move || {
                if let Some(previous) = previous {
                    previous.stop_and_join(STOP_TIMEOUT);
                }
                let control = factory(&path)?;
                Ok(SyncWorker::spawn(control, move || {
                    spawn_wake.request_repaint()
                }))
            },
            move || wake.request_repaint(),
        ));
    }

    /// The vault was closed or did not open: ask its engine to stop now, also when the worker is
    /// still being made; the next `switch_to` waits for it.
    pub fn stop_for_leaving(&mut self) {
        self.leaving = true;
        if let Some(worker) = &self.worker {
            worker.send(Request::Stop);
        }
    }

    /// The window is closing; nothing is drawn any more, so waiting is fine.
    pub fn shut_down(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop_and_join(STOP_TIMEOUT);
        }
    }

    /// Takes in what the background threads have produced and schedules the next status refresh.
    pub fn poll(&mut self, ctx: &Context) {
        self.poll_starting();
        let Some(worker) = &self.worker else {
            return;
        };
        for reply in worker.poll() {
            self.panel.apply(reply);
        }
        let wait = self.panel.tick(worker, Instant::now(), self.window_open);
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
    }

    fn poll_starting(&mut self) {
        let Some(job) = &self.starting else {
            return;
        };
        match job.poll() {
            Poll::Running => return,
            Poll::Ready(Ok(worker)) => {
                self.panel.mode = worker.mode();
                if self.leaving {
                    worker.send(Request::Stop);
                }
                self.worker = Some(worker);
            }
            Poll::Ready(Err(error)) => self.panel.state = SyncState::Error(error),
            Poll::Lost => {
                let lost = "sync stopped unexpectedly".to_owned();
                self.panel.state = SyncState::Error(lost);
            }
        }
        self.starting = None;
    }

    /// Draws the sync window while it is open and sends what the user asked for.
    pub fn show_window(&mut self, ctx: &Context, p: &Palette) {
        if !self.window_open {
            return;
        }
        let SyncWindowOutcome { request, close } = sync_window::show(ctx, p, &mut self.panel);
        match (request, &self.worker) {
            (Some(request), Some(worker)) => self.panel.request(worker, request),
            (Some(_), None) => self.panel.error = Some("sync is not available".to_owned()),
            (None, _) => {}
        }
        self.window_open &= !close;
    }
}
