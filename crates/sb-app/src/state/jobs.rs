//! A unit of work on its own thread whose result the UI polls.

use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Work running on a background thread; its result is collected with `poll`.
pub struct Job<T> {
    rx: Receiver<T>,
}

impl<T: Send + 'static> Job<T> {
    /// Runs `work` on a new thread, sends its result, then calls `wake` (the UI passes
    /// `ctx.request_repaint()`).
    pub fn spawn(
        work: impl FnOnce() -> T + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Job<T> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // A send fails only when the job was dropped: nobody wants the result.
            let _ = tx.send(work());
            wake();
        });
        Job { rx }
    }

    /// The result if the work has finished (once), else `None`. Never blocks.
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn should_deliver_result_and_wake_when_work_finishes() {
        let (woken, woke) = std::sync::mpsc::channel();
        let job = Job::spawn(|| 7, move || woken.send(()).unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some(result) = job.poll() {
                break result;
            }
            assert!(Instant::now() < deadline, "job never finished");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(result, 7);
        assert!(woke.recv_timeout(Duration::from_secs(10)).is_ok());
        assert_eq!(job.poll(), None);
    }
}
