//! What the welcome screen remembers: the typed path, the last error and the folder picker.

use std::path::PathBuf;

use crate::state::jobs::{Job, Poll};

/// The welcome screen's text field, its last error and the folder dialog in progress.
#[derive(Default)]
pub struct WelcomeForm {
    /// The text of the path field.
    pub path: String,
    /// Why the last attempt to open a vault failed.
    pub error: Option<String>,
    picker: Option<Job<Option<PathBuf>>>,
}

impl WelcomeForm {
    /// Runs `pick` (the blocking folder dialog) on its own thread; `wake` repaints the UI.
    pub fn start_picker(
        &mut self,
        pick: impl FnOnce() -> Option<PathBuf> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) {
        self.picker = Some(Job::spawn(pick, wake));
    }

    /// Whether a folder dialog is open.
    pub fn is_picking(&self) -> bool {
        self.picker.is_some()
    }

    /// Collects a finished dialog. A picked folder fills the path field and clears the stale
    /// error; a cancelled or crashed dialog changes nothing.
    pub fn poll_picker(&mut self) -> Option<PathBuf> {
        let job = self.picker.as_ref()?;
        let picked = match job.poll() {
            Poll::Running => return None,
            Poll::Ready(folder) => folder,
            Poll::Lost => None,
        };
        self.picker = None;
        let folder = picked?;
        self.path = folder.display().to_string();
        self.error = None;
        Some(folder)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    fn poll_until_done(form: &mut WelcomeForm) -> Option<PathBuf> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while form.is_picking() {
            if let Some(folder) = form.poll_picker() {
                return Some(folder);
            }
            assert!(Instant::now() < deadline, "picker never finished");
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn should_fill_path_and_clear_stale_error_when_a_folder_is_picked() {
        let mut form = WelcomeForm {
            error: Some("Could not open the vault: old".to_owned()),
            ..WelcomeForm::default()
        };
        form.start_picker(|| Some(PathBuf::from("picked/vault")), || {});
        assert!(form.is_picking());
        let picked = poll_until_done(&mut form);
        assert_eq!(picked, Some(PathBuf::from("picked/vault")));
        assert_eq!(
            form.path,
            PathBuf::from("picked/vault").display().to_string()
        );
        assert_eq!(form.error, None);
        assert!(!form.is_picking());
    }

    #[test]
    fn should_keep_error_and_path_when_the_dialog_is_cancelled_or_lost() {
        let mut form = WelcomeForm {
            path: "typed".to_owned(),
            error: Some("Could not open the vault: old".to_owned()),
            ..WelcomeForm::default()
        };
        form.start_picker(|| None, || {});
        assert_eq!(poll_until_done(&mut form), None);
        form.start_picker(|| panic!("dialog crashed"), || {});
        assert_eq!(poll_until_done(&mut form), None);
        assert_eq!(form.path, "typed");
        assert!(form.error.is_some());
        assert!(!form.is_picking());
    }
}
