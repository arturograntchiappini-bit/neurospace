//! Incremental vault scanning (spec 5.3): re-read only the files that may have changed.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::path::{NoteKey, NotePath};
use crate::vault::{
    Candidate, IgnoreReason, Ignored, ReadOutcome, Scan, Vault, io_error, read_content,
};

/// A file modified less than this before it was read is read again at every scan, because a
/// write in the same clock tick would leave its size and time unchanged.
pub const RACY_WINDOW: Duration = Duration::from_secs(3);

/// Size and modification time of a file as the directory listing reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stat {
    size: u64,
    modified: SystemTime,
}

impl Stat {
    fn of(meta: &Metadata) -> Option<Stat> {
        Some(Stat {
            size: meta.len(),
            modified: meta.modified().ok()?,
        })
    }
}

#[derive(Debug, Clone)]
struct Entry {
    spelling: NotePath,
    /// `false` for a file that was read and found not to be UTF-8.
    is_note: bool,
    /// `None` when the listing gave no usable size and time: the file is always read again.
    stat: Option<Stat>,
    racy: bool,
}

impl Entry {
    fn matches(&self, path: &NotePath, stat: Option<Stat>) -> bool {
        !self.racy && stat.is_some() && self.stat == stat && self.spelling == *path
    }
}

/// What the previous scans saw, per note file (spec 5.3). Cheap to clone.
#[derive(Debug, Default, Clone)]
pub struct LoadCache {
    entries: BTreeMap<NoteKey, Entry>,
}

impl LoadCache {
    /// The caller wrote this note without a scan: the next scan reads its file whatever its size
    /// and time, and reports the key as removed when the file is gone, also when no scan has
    /// ever seen it (a placeholder entry stands for it).
    pub fn expect(&mut self, key: &NoteKey) {
        if let Some(entry) = self.entries.get_mut(key) {
            // A former not-UTF-8 file is a note now, so its disappearance must be reported.
            entry.is_note = true;
            return self.forget(key);
        }
        // A key comes from a valid note path, so it parses; the spelling is never compared
        // because the placeholder has no stat.
        if let Ok(spelling) = NotePath::parse(key.as_str()) {
            let placeholder = Entry {
                spelling,
                is_note: true,
                stat: None,
                racy: false,
            };
            self.entries.insert(key.clone(), placeholder);
        }
    }

    /// The next scan reads this note's file again, whatever its size and time.
    pub fn forget(&mut self, key: &NoteKey) {
        if let Some(entry) = self.entries.get_mut(key) {
            // The entry stays so that a file that is gone by then is still reported as removed.
            entry.stat = None;
        }
    }
}

/// What changed since the previous scan with the same cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LoadDelta {
    /// Notes that are new or may have changed, with on-disk spelling and content, ordered by key.
    /// A spelling-only change appears here, never in `removed`.
    pub changed: Vec<(NotePath, String)>,
    /// Notes of the previous scan whose key is no longer a note (gone, or now ignored), ordered by key.
    pub removed: Vec<NoteKey>,
    /// Every ignored file of this scan, exactly as `Vault::load` reports them.
    pub ignored: Vec<Ignored>,
}

impl Vault {
    /// Scans like `load`, reading only files that may have changed (spec 5.3).
    ///
    /// # Errors
    /// `Io` when the root cannot be listed; the cache is then unchanged.
    pub fn load_changes(&self, cache: &mut LoadCache) -> Result<LoadDelta> {
        let mut scan = Scan::default();
        scan.walk(self.root(), "", true)
            .map_err(io_error(self.root()))?;
        let mut delta = LoadDelta::default();
        let mut entries = BTreeMap::new();
        for candidate in scan.winners() {
            let key = candidate.path.key();
            let old = cache.entries.get(&key);
            if let Some(entry) = revisit(candidate, old, &mut delta, &mut scan.ignored) {
                entries.insert(key, entry);
            }
        }
        delta.removed = cache
            .entries
            .iter()
            .filter(|(key, old)| old.is_note && entries.get(key).is_none_or(|new| !new.is_note))
            .map(|(key, _)| key.clone())
            .collect();
        delta.changed.sort_by_key(|(path, _)| path.key());
        scan.ignored.sort_by(|a, b| a.path.cmp(&b.path));
        delta.ignored = scan.ignored;
        cache.entries = entries;
        Ok(delta)
    }
}

/// Reads `candidate` unless the cache vouches for it, records the result in `delta` and
/// `ignored`, and returns the cache entry to keep, if any.
fn revisit(
    candidate: Candidate,
    old: Option<&Entry>,
    delta: &mut LoadDelta,
    ignored: &mut Vec<Ignored>,
) -> Option<Entry> {
    let stat = candidate.meta.as_ref().and_then(Stat::of);
    match old.filter(|old| old.matches(&candidate.path, stat)) {
        Some(old) => {
            if !old.is_note {
                ignored.push(not_utf8(&candidate.path));
            }
            Some(old.clone())
        }
        None => read_fresh(candidate.path, &candidate.full, stat, (delta, ignored)),
    }
}

fn read_fresh(
    path: NotePath,
    full: &Path,
    stat: Option<Stat>,
    (delta, ignored): (&mut LoadDelta, &mut Vec<Ignored>),
) -> Option<Entry> {
    let read_at = SystemTime::now();
    let is_note = match read_content(full, stat.map(|stat| stat.size)) {
        ReadOutcome::Content(text) => {
            delta.changed.push((path.clone(), text));
            true
        }
        ReadOutcome::Ignore(IgnoreReason::NotUtf8) => {
            ignored.push(not_utf8(&path));
            false
        }
        ReadOutcome::Ignore(reason) => {
            ignored.push(Ignored {
                path: path.to_string(),
                reason,
            });
            return None;
        }
        ReadOutcome::Vanished => return None,
    };
    let racy = stat.is_none_or(|stat| is_racy(stat.modified, read_at));
    Some(Entry {
        spelling: path,
        is_note,
        stat,
        racy,
    })
}

/// Whether a file modified at `modified`, read at `read_at`, must be read again. A modification
/// time in the future is racy too, and so is a bound the platform cannot represent.
fn is_racy(modified: SystemTime, read_at: SystemTime) -> bool {
    modified
        .checked_add(RACY_WINDOW)
        .is_none_or(|bound| bound > read_at)
}

fn not_utf8(path: &NotePath) -> Ignored {
    Ignored {
        path: path.to_string(),
        reason: IgnoreReason::NotUtf8,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{RACY_WINDOW, is_racy};
    use crate::{LoadCache, LoadDelta, MAX_NOTE_BYTES, NoteKey, NotePath, Vault};

    type State = BTreeMap<NoteKey, (NotePath, String)>;

    fn set_time(path: &Path, time: SystemTime) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    fn secs_ago(secs: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(secs)
    }

    fn put_at(root: &Path, relative: &str, bytes: &[u8], time: SystemTime) {
        let file = root.join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, bytes).unwrap();
        set_time(&file, time);
    }

    fn put_old(root: &Path, relative: &str, bytes: &[u8]) {
        put_at(root, relative, bytes, secs_ago(10));
    }

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    fn changed(delta: &LoadDelta) -> Vec<(&str, &str)> {
        delta
            .changed
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect()
    }

    fn apply(state: &mut State, delta: &LoadDelta) {
        for key in &delta.removed {
            state.remove(key);
        }
        for (path, text) in &delta.changed {
            state.insert(path.key(), (path.clone(), text.clone()));
        }
    }

    fn assert_matches_full_load(vault: &Vault, cache: &mut LoadCache, state: &mut State) {
        let delta = vault.load_changes(cache).unwrap();
        for key in &delta.removed {
            assert!(delta.changed.iter().all(|(path, _)| path.key() != *key));
        }
        apply(state, &delta);
        let full = vault.load().unwrap();
        let expected: State = full
            .notes
            .into_iter()
            .map(|(path, text)| (path.key(), (path, text)))
            .collect();
        assert_eq!(*state, expected);
        assert_eq!(delta.ignored, full.ignored);
    }

    fn key(raw: &str) -> NoteKey {
        NotePath::parse(raw).unwrap().key()
    }

    #[test]
    fn should_return_every_note_when_the_cache_is_empty() {
        let (dir, vault) = vault();
        put_old(dir.path(), "b.md", b"two");
        put_old(dir.path(), "dir/a.md", b"one");
        put_old(dir.path(), "bad.md", &[0xff]);
        let delta = vault.load_changes(&mut LoadCache::default()).unwrap();
        let full = vault.load().unwrap();
        assert_eq!(delta.changed, full.notes);
        assert_eq!(delta.ignored, full.ignored);
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn should_report_an_expected_unknown_note_as_removed_when_its_file_is_gone() {
        let (dir, vault) = vault();
        put_old(dir.path(), "a.md", b"one");
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        cache.expect(&key("ghost.md"));
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(delta.removed, [key("ghost.md")]);
        assert!(delta.changed.is_empty());
        assert!(vault.load_changes(&mut cache).unwrap().removed.is_empty());
    }

    #[test]
    fn should_report_an_expected_unknown_note_as_changed_when_its_file_is_there() {
        let (dir, vault) = vault();
        put_old(dir.path(), "a.md", b"one");
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        put_old(dir.path(), "b.md", b"two");
        cache.expect(&key("b.md"));
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(changed(&delta), [("b.md", "two")]);
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn should_report_a_note_written_over_a_not_utf8_file_as_removed_when_it_is_gone() {
        let (dir, vault) = vault();
        put_old(dir.path(), "x.md", &[0xff]);
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        fs::write(dir.path().join("x.md"), "text").unwrap();
        cache.expect(&key("x.md"));
        fs::remove_file(dir.path().join("x.md")).unwrap();
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(delta.removed, [key("x.md")]);
    }

    #[test]
    fn should_report_a_note_written_over_a_not_utf8_file_as_changed_when_it_stays() {
        let (dir, vault) = vault();
        put_old(dir.path(), "x.md", &[0xff]);
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        fs::write(dir.path().join("x.md"), "text").unwrap();
        cache.expect(&key("x.md"));
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(changed(&delta), [("x.md", "text")]);
        assert!(delta.removed.is_empty());
        assert!(delta.ignored.is_empty());
    }

    #[test]
    fn should_read_a_known_note_again_when_it_is_expected() {
        let (dir, vault) = vault();
        put_old(dir.path(), "a.md", b"one");
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        assert!(vault.load_changes(&mut cache).unwrap().changed.is_empty());
        cache.expect(&key("a.md"));
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(changed(&delta), [("a.md", "one")]);
    }

    #[test]
    fn should_match_a_full_load_after_every_step() {
        let (dir, vault) = vault();
        let big = vec![b'x'; MAX_NOTE_BYTES + 1];
        let steps: [&dyn Fn(&Path); 9] = [
            &|r| {
                for name in ["a.md", "b.md", "c.md"] {
                    put_old(r, name, b"one");
                }
            },
            &|r| put_old(r, "b.md", b"longer text"),
            &|r| fs::remove_file(r.join("c.md")).unwrap(),
            &|r| fs::rename(r.join("a.md"), r.join("A.md")).unwrap(),
            &|r| put_old(r, "b.md", &[0xff, 0xfe]),
            &|r| put_old(r, "b.md", &big),
            &|r| put_old(r, "b.md", b"small again"),
            &|r| put_old(r, "image.png", b"not a note"),
            &|r| put_old(r, "folder/d.md", b"in a folder"),
        ];
        let (mut cache, mut state) = (LoadCache::default(), State::new());
        assert_matches_full_load(&vault, &mut cache, &mut state);
        for step in steps {
            step(dir.path());
            assert_matches_full_load(&vault, &mut cache, &mut state);
        }
    }

    #[cfg(unix)]
    #[test]
    fn should_match_a_full_load_through_unix_only_cases() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, vault) = vault();
        let root = dir.path();
        let (mut cache, mut state) = (LoadCache::default(), State::new());
        put_old(root, "b.md", b"lower");
        put_old(root, "B.md", b"upper");
        put_old(root, "locked/hidden.md", b"x");
        assert_matches_full_load(&vault, &mut cache, &mut state);
        fs::remove_file(root.join("B.md")).unwrap();
        assert_matches_full_load(&vault, &mut cache, &mut state);
        let locked = root.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_ok() {
            eprintln!("running as root: permissions do not apply; skipping");
        } else {
            assert_matches_full_load(&vault, &mut cache, &mut state);
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The latest whole second the platform can represent, if one more `RACY_WINDOW` overflows it.
    fn unrepresentable_racy_bound() -> Option<SystemTime> {
        let (mut ok, mut bad) = (0_u64, u64::MAX);
        while bad - ok > 1 {
            let mid = ok + (bad - ok) / 2;
            match UNIX_EPOCH.checked_add(Duration::from_secs(mid)) {
                Some(_) => ok = mid,
                None => bad = mid,
            }
        }
        let time = UNIX_EPOCH.checked_add(Duration::from_secs(ok))?;
        time.checked_add(RACY_WINDOW).is_none().then_some(time)
    }

    #[test]
    fn should_treat_an_unrepresentable_racy_bound_as_racy() {
        let read_at = SystemTime::now();
        assert!(is_racy(read_at + Duration::from_secs(60), read_at));
        assert!(!is_racy(read_at - Duration::from_secs(10), read_at));
        if let Some(time) = unrepresentable_racy_bound() {
            assert!(is_racy(time, read_at));
        }
    }

    #[test]
    fn should_read_again_a_racy_file_with_unchanged_size_and_time() {
        let (dir, vault) = vault();
        let file = dir.path().join("x.md");
        fs::write(&file, b"aaaa").unwrap();
        let time = fs::metadata(&file).unwrap().modified().unwrap();
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        fs::write(&file, b"bbbb").unwrap();
        set_time(&file, time);
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(changed(&delta), [("x.md", "bbbb")]);
    }

    /// Writes `aaaa`, scans, then swaps in `bbbb` with the same size and time.
    fn scan_then_swap_old_file(vault: &Vault, root: &Path) -> LoadCache {
        let time = secs_ago(10);
        put_at(root, "x.md", b"aaaa", time);
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        put_at(root, "x.md", b"bbbb", time);
        cache
    }

    #[test]
    fn should_trust_an_old_file_with_unchanged_size_and_time() {
        let (dir, vault) = vault();
        let mut cache = scan_then_swap_old_file(&vault, dir.path());
        let delta = vault.load_changes(&mut cache).unwrap();
        assert!(delta.changed.is_empty());
    }

    #[test]
    fn should_read_again_a_forgotten_file() {
        let (dir, vault) = vault();
        let mut cache = scan_then_swap_old_file(&vault, dir.path());
        cache.forget(&key("x.md"));
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(changed(&delta), [("x.md", "bbbb")]);
    }

    #[test]
    fn should_report_a_forgotten_file_as_removed_when_it_is_gone() {
        let (dir, vault) = vault();
        let mut cache = scan_then_swap_old_file(&vault, dir.path());
        cache.forget(&key("x.md"));
        fs::remove_file(dir.path().join("x.md")).unwrap();
        let delta = vault.load_changes(&mut cache).unwrap();
        assert_eq!(delta.removed, [key("x.md")]);
    }

    #[test]
    fn should_skip_unchanged_files() {
        let (dir, vault) = vault();
        for number in 0..50 {
            put_old(dir.path(), &format!("n{number}.md"), b"text");
        }
        let mut cache = LoadCache::default();
        assert_eq!(vault.load_changes(&mut cache).unwrap().changed.len(), 50);
        let delta = vault.load_changes(&mut cache).unwrap();
        assert!(delta.changed.is_empty() && delta.removed.is_empty());
    }

    #[test]
    fn should_keep_the_cache_when_the_root_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let away = dir.path().join("away");
        put_old(&root, "x.md", b"text");
        let vault = Vault::open(&root).unwrap();
        let mut cache = LoadCache::default();
        vault.load_changes(&mut cache).unwrap();
        fs::rename(&root, &away).unwrap();
        assert!(vault.load_changes(&mut cache).is_err());
        fs::rename(&away, &root).unwrap();
        let delta = vault.load_changes(&mut cache).unwrap();
        assert!(delta.changed.is_empty() && delta.removed.is_empty());
    }
}
