//! What the application keeps for the sync engine of the open vault: the worker, the window's
//! model and the vault switch, none of which ever waits on the engine (constraint 7).

use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::Context;

use crate::state::sync_panel::{Request, SyncControl, SyncPanel, SyncState, SyncWorker};
use crate::state::{Job, Poll};
use crate::ui::sync_window::{self, SyncWindowOutcome};
use crate::ui::theme::Palette;

/// How long a vault switch or an exit waits for the old engine to release the vault.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const STARTING_POLL: Duration = Duration::from_millis(20);

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
    /// both on one background thread so that two engines never race for a lock. A worker the
    /// previous switch is still making is waited for there and stopped too. Each stop waits at
    /// most `STOP_TIMEOUT`; an old engine that has not stopped by then is given up on, and the new
    /// worker may find the lock taken.
    pub fn switch_to(&mut self, path: &Path, ctx: &Context) {
        let pending = self.starting.take();
        let (previous, factory, path) = (self.worker.take(), self.factory.clone(), path.to_owned());
        let wake = ctx.clone();
        let spawn_wake = ctx.clone();
        self.panel = SyncPanel::default();
        self.window_open = false;
        self.leaving = false;
        self.starting = Some(Job::spawn(
            move || {
                let unseen = pending.and_then(Job::wait).and_then(Result::ok);
                for old in [unseen, previous].into_iter().flatten() {
                    old.stop_and_join(STOP_TIMEOUT);
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

    /// The window is closing; nothing is drawn any more, so waiting is fine. A worker still being
    /// made is waited for and stopped too; the whole wait is bounded by `STOP_TIMEOUT`. A job still
    /// running at the bound ends with the process, and the engine's lock is a file lock the
    /// operating system releases then.
    pub fn shut_down(&mut self) {
        let deadline = Instant::now() + STOP_TIMEOUT;
        self.await_starting(deadline);
        if let Some(worker) = self.worker.take() {
            worker.stop_and_join(deadline.saturating_duration_since(Instant::now()));
        }
    }

    /// Waits until `deadline` for the worker being made and keeps it as the current one.
    fn await_starting(&mut self, deadline: Instant) {
        let Some(job) = self.starting.take() else {
            return;
        };
        loop {
            match job.poll() {
                Poll::Running if Instant::now() < deadline => thread::sleep(STARTING_POLL),
                Poll::Ready(Ok(worker)) => {
                    self.worker = Some(worker);
                    return;
                }
                _ => return,
            }
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
    pub fn show_window(&mut self, ctx: &Context, p: &Palette, reduce_motion: bool) {
        if !self.window_open {
            return;
        }
        let SyncWindowOutcome { request, close } =
            sync_window::show(ctx, p, &mut self.panel, reduce_motion);
        match (request, &self.worker) {
            (Some(request), Some(worker)) => self.panel.request(worker, request),
            (Some(_), None) => self.panel.error = Some("sync is not available".to_owned()),
            (None, _) => {}
        }
        self.window_open &= !close;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::thread;

    use ns_sync::NetMode;

    use super::*;
    use crate::state::sync_panel::{EngineStatus, StartOutcome};

    const SLOW: Duration = Duration::from_millis(300);

    type Log = Arc<Mutex<Vec<String>>>;

    /// A control that records its stop. A vault named `slow-stop` stops slowly.
    struct Recorder {
        name: String,
        log: Log,
    }

    impl SyncControl for Recorder {
        fn configured(&self) -> bool {
            false
        }
        fn start(&mut self) -> Result<StartOutcome, String> {
            Err("not used".into())
        }
        fn invite(&mut self) -> Result<String, String> {
            Err("not used".into())
        }
        fn join(&mut self, _ticket: &str) -> Result<(), String> {
            Err("not used".into())
        }
        fn leave(&mut self) -> Result<(), String> {
            Err("not used".into())
        }
        fn status(&mut self) -> EngineStatus {
            EngineStatus::Unknown
        }
        fn set_mode(&mut self, _mode: NetMode) -> Result<StartOutcome, String> {
            Err("not used".into())
        }
        fn stop(&mut self) {
            if self.name == "slow-stop" {
                thread::sleep(SLOW);
            }
            self.log.lock().unwrap().push(format!("stop {}", self.name));
        }
        fn mode(&self) -> NetMode {
            NetMode::Internet
        }
    }

    /// A host whose factory logs `factory <vault name>`; a vault named `slow-make` takes a while.
    fn host() -> (SyncHost, Log) {
        let log = Log::default();
        let recorded = log.clone();
        let factory: ControlFactory = Arc::new(move |path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "slow-make" {
                thread::sleep(SLOW);
            }
            recorded.lock().unwrap().push(format!("factory {name}"));
            let log = recorded.clone();
            Ok(Box::new(Recorder { name, log }) as Box<dyn SyncControl>)
        });
        (SyncHost::new(factory), log)
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_worker(host: &mut SyncHost, ctx: &Context) {
        wait_until("the worker", || {
            host.poll(ctx);
            host.worker.is_some()
        });
    }

    fn entries(log: &Log) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    #[test]
    fn should_stop_the_old_worker_in_the_background_before_making_the_new_one() {
        let (mut host, log) = host();
        let ctx = Context::default();
        host.switch_to(Path::new("slow-stop"), &ctx);
        wait_for_worker(&mut host, &ctx);
        host.switch_to(Path::new("b"), &ctx);
        // The old stop sleeps SLOW before it logs: a switch that waited for it would see it here.
        assert!(
            !entries(&log).contains(&"stop slow-stop".to_owned()),
            "the switch waited for the old stop"
        );
        wait_for_worker(&mut host, &ctx);
        assert_eq!(
            entries(&log),
            ["factory slow-stop", "stop slow-stop", "factory b"]
        );
    }

    #[test]
    fn should_stop_a_worker_that_arrives_after_the_vault_was_left() {
        let (mut host, log) = host();
        let ctx = Context::default();
        host.switch_to(Path::new("slow-make"), &ctx);
        host.stop_for_leaving();
        wait_for_worker(&mut host, &ctx);
        wait_until("the stop", || {
            entries(&log).contains(&"stop slow-make".to_owned())
        });
    }

    #[test]
    fn should_stop_the_control_when_shutting_down() {
        let (mut host, log) = host();
        let ctx = Context::default();
        host.switch_to(Path::new("a"), &ctx);
        wait_for_worker(&mut host, &ctx);
        host.shut_down();
        assert_eq!(entries(&log), ["factory a", "stop a"]);
    }

    #[test]
    fn should_stop_a_worker_still_being_made_when_shutting_down() {
        let (mut host, log) = host();
        host.switch_to(Path::new("slow-make"), &Context::default());
        host.shut_down();
        assert_eq!(entries(&log), ["factory slow-make", "stop slow-make"]);
    }

    #[test]
    fn should_stop_a_worker_still_being_made_before_the_next_vault_is_made() {
        let (mut host, log) = host();
        let ctx = Context::default();
        host.switch_to(Path::new("slow-make"), &ctx);
        host.switch_to(Path::new("b"), &ctx);
        wait_for_worker(&mut host, &ctx);
        assert_eq!(
            entries(&log),
            ["factory slow-make", "stop slow-make", "factory b"]
        );
    }
}
