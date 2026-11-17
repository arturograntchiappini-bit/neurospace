//! Follows the vault folder: rebuilds the index in the background after external changes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use sb_core::{Index, Result, Vault, VaultWatcher, watch};

use super::jobs::Job;
use super::notice::Notice;
use super::workspace::{ApplyOutcome, Workspace};

/// How long the folder must stay quiet before the index is rebuilt.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(300);

type Wake = Arc<dyn Fn() + Send + Sync>;
/// An index built when the workspace had saved this many times, or why it could not be built.
type Reload = (u64, Result<Index>);

/// Watches the vault folder and rebuilds the index off the UI thread after a quiet moment.
pub struct Reloader {
    root: PathBuf,
    wake: Wake,
    last_change: Arc<Mutex<Option<Instant>>>,
    job: Option<Job<Reload>>,
    _watcher: Option<VaultWatcher>,
}

impl Reloader {
    /// Starts watching `root`; every change calls `wake`.
    ///
    /// # Errors
    /// Whatever `sb_core::watch` returns.
    pub fn start(root: &Path, wake: impl Fn() + Send + Sync + 'static) -> Result<Reloader> {
        let mut reloader = Reloader::new(root, Arc::new(wake));
        let last_change = Arc::clone(&reloader.last_change);
        let wake = Arc::clone(&reloader.wake);
        reloader._watcher = Some(watch(root, move || {
            record(&last_change, Instant::now());
            wake();
        })?);
        Ok(reloader)
    }

    fn new(root: &Path, wake: Wake) -> Reloader {
        Reloader {
            root: root.to_path_buf(),
            wake,
            last_change: Arc::default(),
            job: None,
            _watcher: None,
        }
    }

    /// A reloader that sees no folder events, for tests that cannot keep the folder watched.
    #[cfg(test)]
    fn without_watcher(root: &Path) -> Reloader {
        Reloader::new(root, Arc::new(|| {}))
    }

    /// Records a change at `now`; the watcher callback does the same for real events.
    pub fn mark_changed(&mut self, now: Instant) {
        record(&self.last_change, now);
    }

    /// True while a rebuild is running.
    pub fn is_reloading(&self) -> bool {
        self.job.is_some()
    }

    /// Applies a finished rebuild, then starts a new one if a change has been quiet for
    /// `RELOAD_DEBOUNCE` and none is running. A failed rebuild keeps the old index.
    pub fn tick(&mut self, workspace: &mut Workspace, now: Instant) -> Vec<Notice> {
        let notices = self.collect(workspace, now);
        if self.job.is_none() && self.quiet_change_pending(now) {
            self.spawn_rebuild(workspace.save_generation());
        }
        notices
    }

    fn collect(&mut self, workspace: &mut Workspace, now: Instant) -> Vec<Notice> {
        let Some((tag, built)) = self.job.as_ref().and_then(Job::poll) else {
            return Vec::new();
        };
        self.job = None;
        match built {
            Ok(index) => match workspace.apply_index(index, tag) {
                ApplyOutcome::Applied(notices) => notices,
                ApplyOutcome::Stale => {
                    self.mark_changed(now);
                    Vec::new()
                }
            },
            Err(error) => vec![Notice::error(format!(
                "Could not reload the vault: {error}"
            ))],
        }
    }

    fn quiet_change_pending(&self, now: Instant) -> bool {
        let last = *self
            .last_change
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        last.is_some_and(|at| now.saturating_duration_since(at) >= RELOAD_DEBOUNCE)
    }

    fn spawn_rebuild(&mut self, save_generation: u64) {
        *self
            .last_change
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        let root = self.root.clone();
        let wake = Arc::clone(&self.wake);
        let build = move || {
            (
                save_generation,
                Vault::open(root).and_then(|v| Index::build(&v)),
            )
        };
        self.job = Some(Job::spawn(build, move || wake()));
    }
}

fn record(last_change: &Mutex<Option<Instant>>, at: Instant) {
    *last_change.lock().unwrap_or_else(PoisonError::into_inner) = Some(at);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use sb_core::NotePath;
    use tempfile::TempDir;

    use super::*;
    use crate::state::{ApplyOutcome, Notice, NoticeKind, Workspace};

    fn p(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    fn setup(files: &[(&str, &str)]) -> (TempDir, Workspace, Reloader) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            fs::write(dir.path().join(name), text).unwrap();
        }
        let ws = Workspace::open(dir.path()).unwrap();
        let reloader = Reloader::without_watcher(dir.path());
        (dir, ws, reloader)
    }

    /// Ticks (all at the same instant) until the running job's result has been applied.
    fn finish_job(ws: &mut Workspace, reloader: &mut Reloader, at: Instant) -> Vec<Notice> {
        let mut notices = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while reloader.is_reloading() {
            assert!(Instant::now() < deadline, "reload never finished");
            std::thread::sleep(Duration::from_millis(20));
            notices.extend(reloader.tick(ws, at));
        }
        notices
    }

    /// Marks a change at `t0`, lets the debounce pass and ticks until the result is applied.
    fn reload(ws: &mut Workspace, reloader: &mut Reloader, t0: Instant) -> Vec<Notice> {
        reloader.mark_changed(t0);
        let at = t0 + RELOAD_DEBOUNCE;
        let mut notices = reloader.tick(ws, at);
        notices.extend(finish_job(ws, reloader, at));
        notices
    }

    fn conflict_files(dir: &TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("(conflict "))
            .collect();
        names.sort();
        names
    }

    fn is_conflict_name(name: &str, stem: &str) -> bool {
        let hex = name
            .strip_prefix(&format!("{stem} (conflict "))
            .and_then(|rest| rest.strip_suffix(").md"));
        hex.is_some_and(|hex| hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()))
    }

    #[test]
    fn should_reload_silently_when_open_note_is_clean() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        fs::write(dir.path().join("n.md"), "theirs").unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        assert!(notices.is_empty(), "{notices:?}");
        let open = ws.open_note().unwrap();
        assert_eq!(
            (open.buffer.as_str(), open.saved.as_str()),
            ("theirs", "theirs")
        );
        assert!(!open.is_dirty() && !open.autosave_paused);
        assert!(conflict_files(&dir).is_empty());
    }

    #[test]
    fn should_write_conflict_copy_when_open_note_is_dirty() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        ws.edit("mine".into(), Instant::now());
        fs::write(dir.path().join("n.md"), "theirs").unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        let open = ws.open_note().unwrap();
        assert_eq!(open.path, p("n.md"));
        assert_eq!(
            (open.buffer.as_str(), open.saved.as_str()),
            ("theirs", "theirs")
        );
        assert!(!open.is_dirty() && !open.autosave_paused);
        let names = conflict_files(&dir);
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(is_conflict_name(&names[0], "n"), "{names:?}");
        let kept = fs::read_to_string(dir.path().join(&names[0])).unwrap();
        assert_eq!(kept, "mine");
        let original = fs::read_to_string(dir.path().join("n.md")).unwrap();
        assert_eq!(original, "theirs");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, NoticeKind::Info);
        assert!(notices[0].text.contains("n.md") && notices[0].text.contains(&names[0]));
        assert!(ws.index().get(&p(&names[0]).key()).is_some());
        assert!(ws.tree().notes.contains(&p(&names[0])));
    }

    #[test]
    fn should_close_note_when_removed_and_clean() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        fs::remove_file(dir.path().join("n.md")).unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        assert!(ws.open_note().is_none());
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, NoticeKind::Info);
        assert!(notices[0].text.contains("n.md"));
        assert!(conflict_files(&dir).is_empty());
    }

    #[test]
    fn should_keep_typed_text_when_removed_and_dirty() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        ws.edit("mine".into(), Instant::now());
        fs::remove_file(dir.path().join("n.md")).unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        let names = conflict_files(&dir);
        assert_eq!(names.len(), 1, "{names:?}");
        let kept = fs::read_to_string(dir.path().join(&names[0])).unwrap();
        assert_eq!(kept, "mine");
        let open = ws.open_note().unwrap();
        assert_eq!(open.path, p(&names[0]));
        assert_eq!(
            (open.buffer.as_str(), open.saved.as_str()),
            ("mine", "mine")
        );
        assert!(!open.is_dirty() && !open.autosave_paused);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].text.contains(&names[0]));
    }

    #[test]
    fn should_ignore_own_save_when_reloading() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        ws.edit("mine".into(), Instant::now());
        ws.save().unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        assert!(notices.is_empty(), "{notices:?}");
        assert!(conflict_files(&dir).is_empty());
        assert_eq!(ws.open_note().unwrap().buffer, "mine");
    }

    #[test]
    fn should_drop_stale_reload_when_save_happened_meanwhile() {
        let (dir, mut ws, _reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        let tag = ws.save_generation();
        let built_before_save = sb_core::Index::build(ws.vault()).unwrap();
        ws.edit("mine".into(), Instant::now());
        ws.save().unwrap();
        let outcome = ws.apply_index(built_before_save, tag);
        assert!(matches!(outcome, ApplyOutcome::Stale));
        let open = ws.open_note().unwrap();
        assert_eq!(
            (open.buffer.as_str(), open.saved.as_str()),
            ("mine", "mine")
        );
        assert_eq!(fs::read_to_string(dir.path().join("n.md")).unwrap(), "mine");
        assert_eq!(ws.index().get(&p("n.md").key()).unwrap().content, "mine");
    }

    #[test]
    fn should_mark_new_change_when_applied_result_is_stale() {
        let (_dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        let t0 = Instant::now();
        let at = t0 + RELOAD_DEBOUNCE;
        reloader.mark_changed(t0);
        reloader.tick(&mut ws, at);
        ws.edit("mine".into(), t0);
        ws.save().unwrap();
        finish_job(&mut ws, &mut reloader, at);
        assert!(!reloader.is_reloading());
        reloader.tick(&mut ws, at + RELOAD_DEBOUNCE);
        assert!(
            reloader.is_reloading(),
            "a stale result must schedule a new reload"
        );
    }

    #[test]
    fn should_debounce_bursts_of_changes() {
        let (_dir, mut ws, mut reloader) = setup(&[("n.md", "x")]);
        let t0 = Instant::now();
        for millis in [0, 50, 100] {
            reloader.mark_changed(t0 + Duration::from_millis(millis));
        }
        reloader.tick(&mut ws, t0 + Duration::from_millis(399));
        assert!(!reloader.is_reloading());
        let at = t0 + Duration::from_millis(400);
        reloader.tick(&mut ws, at);
        assert!(reloader.is_reloading());
        finish_job(&mut ws, &mut reloader, at);
        reloader.tick(&mut ws, t0 + Duration::from_secs(60));
        assert!(
            !reloader.is_reloading(),
            "the burst must cost exactly one reload"
        );
    }

    #[test]
    fn should_keep_index_when_vault_folder_disappears() {
        let (dir, mut ws, mut reloader) = setup(&[("n.md", "old")]);
        ws.select(&p("n.md")).unwrap();
        ws.edit("mine".into(), Instant::now());
        let moved = dir.path().with_extension("moved");
        fs::rename(dir.path(), &moved).unwrap();
        let notices = reload(&mut ws, &mut reloader, Instant::now());
        fs::rename(&moved, dir.path()).unwrap();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].kind, NoticeKind::Error);
        assert!(notices[0].text.starts_with("Could not reload the vault: "));
        assert_eq!(ws.index().len(), 1);
        let open = ws.open_note().unwrap();
        assert_eq!((open.buffer.as_str(), open.saved.as_str()), ("mine", "old"));
        assert!(open.is_dirty());
    }

    #[test]
    fn should_reload_when_watcher_sees_external_write() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        let mut ws = Workspace::open(dir.path()).unwrap();
        let mut reloader = Reloader::start(dir.path(), || {}).unwrap();
        fs::write(dir.path().join("new.md"), "fresh").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while ws.index().get(&p("new.md").key()).is_none() {
            assert!(
                Instant::now() < deadline,
                "the new note never reached the index"
            );
            std::thread::sleep(Duration::from_millis(30));
            reloader.tick(&mut ws, Instant::now());
        }
        assert_eq!(ws.tree().notes.len(), 2);
    }
}
