//! Folder watching: tells the sync engine that something under the vault changed.

use std::io;
use std::path::Path;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{CoreError, Result};

/// Keeps the folder watcher alive; dropping it stops the notifications.
pub struct VaultWatcher {
    _watcher: RecommendedWatcher,
}

/// Calls `on_change` whenever something changes under `root`, except inside directories or for files whose name starts with `.`.
/// It only says that something changed, never what.
///
/// # Errors
/// `CoreError::Io` when the folder cannot be watched.
pub fn watch(root: &Path, on_change: impl Fn() + Send + 'static) -> Result<VaultWatcher> {
    let watched_root = root.to_path_buf();
    let handler = move |event: notify::Result<Event>| {
        if signals(&watched_root, &event) {
            on_change();
        }
    };
    let io_error = |source: notify::Error| CoreError::Io {
        path: root.to_path_buf(),
        source: io::Error::other(source),
    };
    let mut watcher = notify::recommended_watcher(handler).map_err(io_error)?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(io_error)?;
    Ok(VaultWatcher { _watcher: watcher })
}

/// Event kinds are not reliable facts (spec A.4), so errors and rescan requests count as changes too.
/// Only reads (`Access`, which inotify reports for every open) never do: a write always comes with a `Modify` as well.
fn signals(root: &Path, event: &notify::Result<Event>) -> bool {
    match event {
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => false,
        Ok(event) => !is_hidden_only(root, event),
        Err(_) => true,
    }
}

/// An event without paths (a rescan request) is never hidden.
fn is_hidden_only(root: &Path, event: &Event) -> bool {
    !event.paths.is_empty() && event.paths.iter().all(|path| is_hidden(root, path))
}

fn is_hidden(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use notify::event::{AccessKind, AccessMode, ModifyKind};

    use super::*;

    const LIMIT: Duration = Duration::from_secs(5);
    const SETTLE: Duration = Duration::from_millis(300);

    fn counting_watcher(root: &Path) -> (VaultWatcher, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let watcher = watch(root, move || {
            seen.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        (watcher, count)
    }

    fn grows_after(count: &AtomicUsize, action: impl FnOnce()) -> bool {
        let before = count.load(Ordering::SeqCst);
        action();
        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) > before {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Waits until the count has not changed for `SETTLE`, so a late event of one step cannot count for the next.
    fn settle(count: &AtomicUsize) {
        let mut last = count.load(Ordering::SeqCst);
        let mut stable_since = Instant::now();
        while stable_since.elapsed() < SETTLE {
            std::thread::sleep(Duration::from_millis(50));
            let now = count.load(Ordering::SeqCst);
            if now != last {
                last = now;
                stable_since = Instant::now();
            }
        }
    }

    #[test]
    fn should_signal_when_a_note_is_written_created_renamed_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (_watcher, count) = counting_watcher(dir.path());
        let note = dir.path().join("n.md");
        assert!(grows_after(&count, || fs::write(&note, "a\n").unwrap()));
        settle(&count);
        assert!(grows_after(&count, || fs::write(&note, "b\n").unwrap()));
        settle(&count);
        let renamed = dir.path().join("m.md");
        assert!(grows_after(&count, || fs::rename(&note, &renamed).unwrap()));
        settle(&count);
        assert!(grows_after(&count, || fs::remove_file(&renamed).unwrap()));
    }

    #[test]
    fn should_not_signal_for_an_access_event() {
        let root = Path::new("vault");
        let path = root.join("n.md");
        let open = EventKind::Access(AccessKind::Open(AccessMode::Any));
        let read = Event::new(open).add_path(path.clone());
        assert!(!signals(root, &Ok(read)));
        let modify = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path);
        assert!(signals(root, &Ok(modify)));
        assert!(signals(root, &Ok(Event::new(EventKind::Other))));
    }

    #[test]
    fn should_stay_quiet_when_a_note_is_only_read() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("n.md");
        fs::write(&note, "a\n").unwrap();
        let (_watcher, count) = counting_watcher(dir.path());
        std::thread::sleep(Duration::from_millis(500));
        let before = count.load(Ordering::SeqCst);
        for _ in 0..3 {
            fs::read_to_string(&note).unwrap();
        }
        fs::read_dir(dir.path()).unwrap().count();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(count.load(Ordering::SeqCst), before);
    }

    #[test]
    fn should_signal_for_changes_in_new_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let (_watcher, count) = counting_watcher(dir.path());
        let sub = dir.path().join("sub");
        assert!(grows_after(&count, || fs::create_dir(&sub).unwrap()));
        std::thread::sleep(Duration::from_millis(200));
        assert!(grows_after(&count, || {
            fs::write(sub.join("n.md"), "a\n").unwrap();
        }));
    }

    #[test]
    fn should_stay_quiet_for_hidden_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".secondbrain/trash")).unwrap();
        fs::create_dir(dir.path().join(".obsidian")).unwrap();
        let (_watcher, count) = counting_watcher(dir.path());
        std::thread::sleep(Duration::from_millis(500));
        let before = count.load(Ordering::SeqCst);
        fs::write(dir.path().join(".sb-1.tmp"), "x").unwrap();
        fs::write(dir.path().join(".secondbrain/trash/x"), "x").unwrap();
        fs::write(dir.path().join(".obsidian/a.md"), "x").unwrap();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(count.load(Ordering::SeqCst), before);
    }

    #[test]
    fn should_stop_signalling_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (watcher, count) = counting_watcher(dir.path());
        assert!(grows_after(&count, || {
            fs::write(dir.path().join("a.md"), "a\n").unwrap();
        }));
        drop(watcher);
        std::thread::sleep(Duration::from_millis(500));
        let settled = count.load(Ordering::SeqCst);
        fs::write(dir.path().join("b.md"), "b\n").unwrap();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(count.load(Ordering::SeqCst), settled);
    }
}
