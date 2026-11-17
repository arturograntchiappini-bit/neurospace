//! A unit of work on its own thread whose result the UI polls.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// Work running on a background thread; its result is collected with `poll`.
pub struct Job<T> {
    rx: Receiver<T>,
}

/// Where a `Job` stands.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll<T> {
    /// The work finished; this is its result, handed out once.
    Ready(T),
    /// The work is still running.
    Running,
    /// The thread ended without a result (it panicked), or the result was already taken.
    Lost,
}

/// Calls the wrapped function when dropped, so a thread that panics still wakes the UI.
struct WakeOnDrop<F: Fn()>(F);

impl<F: Fn()> Drop for WakeOnDrop<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

impl<T: Send + 'static> Job<T> {
    /// Runs `work` on a new thread, sends its result, then calls `wake` (the UI passes
    /// `ctx.request_repaint()`), also when `work` panics.
    pub fn spawn(
        work: impl FnOnce() -> T + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Job<T> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            // Declared first, so it is dropped last: after the result is queued or `tx` is gone.
            let _wake = WakeOnDrop(wake);
            let tx = tx;
            // A send fails only when the job was dropped: nobody wants the result.
            let _ = tx.send(work());
        });
        Job { rx }
    }

    /// Checks the job without blocking.
    pub fn poll(&self) -> Poll<T> {
        match self.rx.try_recv() {
            Ok(result) => Poll::Ready(result),
            Err(TryRecvError::Empty) => Poll::Running,
            Err(TryRecvError::Disconnected) => Poll::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn wait_until_not_running<T: Send + 'static>(job: &Job<T>) -> Poll<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match job.poll() {
                Poll::Running => {}
                done => return done,
            }
            assert!(Instant::now() < deadline, "job never finished");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn should_deliver_result_and_wake_when_work_finishes() {
        let (woken, woke) = mpsc::channel();
        let job = Job::spawn(|| 7, move || woken.send(()).unwrap());
        assert_eq!(wait_until_not_running(&job), Poll::Ready(7));
        assert!(woke.recv_timeout(Duration::from_secs(10)).is_ok());
        assert_eq!(job.poll(), Poll::Lost);
    }

    #[test]
    #[allow(clippy::panic)]
    fn should_report_lost_and_wake_when_work_panics() {
        let (woken, woke) = mpsc::channel();
        let job = Job::spawn(
            || -> u8 { panic!("worker failed on purpose") },
            move || woken.send(()).unwrap(),
        );
        assert_eq!(wait_until_not_running(&job), Poll::Lost);
        assert!(woke.recv_timeout(Duration::from_secs(10)).is_ok());
    }
}
