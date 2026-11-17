//! Durable storage of the sync document (snapshot plus update log) and of the reconciliation base.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sb_core::{NoteKey, NotePath};

use crate::doc::{Hash32, SyncDoc};
use crate::error::{Result, SyncError};
use crate::state::{
    StateDir, corrupt, from_hex32, io_error, private_options, read_json, read_optional,
    remove_if_exists, to_hex, write_atomic, write_json,
};

/// Log size from which the engine should compact the document into a new snapshot.
pub const COMPACT_LOG_BYTES: u64 = 8 * 1024 * 1024;

const FRAME_HEADER_BYTES: usize = 4;

/// The sync document on disk: `doc.snap` plus the append-only `doc.log` of update frames.
#[derive(Debug)]
pub struct DocStore {
    snap: PathBuf,
    log: PathBuf,
    persisted: Vec<u8>,
    /// Length of the log up to the end of the last good frame; anything after it is garbage.
    log_good_len: u64,
}

/// Splits one complete frame (4-byte big-endian length, then the payload) off the front.
/// An incomplete frame, or one with a zero length (`persist` never writes one; a zero-filled
/// tail is what an operating-system crash can leave), is a torn tail: `None`.
fn split_frame(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (header, body) = bytes.split_first_chunk::<FRAME_HEADER_BYTES>()?;
    let length = usize::try_from(u32::from_be_bytes(*header)).ok()?;
    if length == 0 {
        return None;
    }
    body.split_at_checked(length)
}

fn truncate_log(log: &Path, len: u64) -> Result<()> {
    let file = private_options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(log)
        .map_err(io_error(log))?;
    file.set_len(len)
        .and_then(|()| file.sync_all())
        .map_err(io_error(log))
}

/// Imports every complete frame and returns the length of the good part of the log. A torn
/// tail is ignored and cut off. A frame that waits for missing history cannot come from this
/// store, so it is corruption.
fn replay_log(log: &Path, doc: &mut SyncDoc) -> Result<u64> {
    let Some(bytes) = read_optional(log)? else {
        return Ok(0);
    };
    let mut rest = bytes.as_slice();
    while let Some((frame, tail)) = split_frame(rest) {
        let pending = doc.import(frame).map_err(|error| corrupt(log, error))?;
        if pending {
            return Err(corrupt(
                log,
                "a frame waits for history the log does not hold",
            ));
        }
        rest = tail;
    }
    let good_len = (bytes.len() - rest.len()) as u64;
    if !rest.is_empty() {
        truncate_log(log, good_len)?;
    }
    Ok(good_len)
}

/// Writes one frame at `offset`, cutting off whatever follows it first, in a single write,
/// and makes it durable. The file is not opened for appending: Windows refuses to resize such
/// a handle. Returns the frame's length.
fn write_frame_at(log: &Path, offset: u64, updates: &[u8]) -> Result<u64> {
    let length = u32::try_from(updates.len())
        .map_err(|_| SyncError::Doc("update larger than 4 GiB".to_string()))?;
    let mut frame = length.to_be_bytes().to_vec();
    frame.extend_from_slice(updates);
    let mut file = private_options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(log)
        .map_err(io_error(log))?;
    file.set_len(offset)
        .and_then(|()| file.seek(SeekFrom::Start(offset)))
        .and_then(|_| file.write_all(&frame))
        .and_then(|()| file.sync_data())
        .map_err(io_error(log))?;
    Ok(frame.len() as u64)
}

impl DocStore {
    /// Loads the snapshot, then every complete frame of the log, and returns the document.
    pub fn open(state: &StateDir) -> Result<(DocStore, SyncDoc)> {
        let snap = state.file("doc.snap");
        let log = state.file("doc.log");
        let mut doc = match read_optional(&snap)? {
            Some(bytes) => SyncDoc::from_snapshot(&bytes).map_err(|error| corrupt(&snap, error))?,
            None => SyncDoc::new(),
        };
        let log_good_len = replay_log(&log, &mut doc)?;
        let persisted = doc.version();
        let store = DocStore {
            snap,
            log,
            persisted,
            log_good_len,
        };
        Ok((store, doc))
    }

    /// Appends what changed since the last persist and syncs it to disk; writes nothing when
    /// unchanged. The frame goes right after the last good one, so garbage left by an earlier
    /// failed append is cut off first.
    pub fn persist(&mut self, doc: &SyncDoc) -> Result<()> {
        let Some(updates) = doc.updates_for(&self.persisted)? else {
            return Ok(());
        };
        self.log_good_len += write_frame_at(&self.log, self.log_good_len, &updates)?;
        self.persisted = doc.version();
        Ok(())
    }

    /// Writes a new snapshot durably, then empties the log. A crash in between is harmless:
    /// replaying old frames over a newer snapshot changes nothing.
    pub fn compact(&mut self, doc: &SyncDoc) -> Result<()> {
        write_atomic(&self.snap, &doc.snapshot()?)?;
        truncate_log(&self.log, 0)?;
        self.log_good_len = 0;
        self.persisted = doc.version();
        Ok(())
    }

    /// Size of the log in bytes; 0 when there is no log.
    pub fn log_len(&self) -> Result<u64> {
        match std::fs::metadata(&self.log) {
            Ok(metadata) => Ok(metadata.len()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
            Err(error) => Err(io_error(&self.log)(error)),
        }
    }

    /// Removes both files and returns an empty document. The log goes first, so a crash
    /// in between leaves an older but complete snapshot rather than orphaned frames.
    pub fn reset(&mut self) -> Result<SyncDoc> {
        remove_if_exists(&self.log)?;
        self.log_good_len = 0;
        remove_if_exists(&self.snap)?;
        let doc = SyncDoc::new();
        self.persisted = doc.version();
        Ok(doc)
    }
}

/// For each note key, the hash of the content on which disk and document last agreed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Base {
    entries: BTreeMap<NoteKey, Hash32>,
}

impl Base {
    /// Reads `base.json`; empty when missing. Entries whose key is not a valid note path
    /// or whose value is not 64 hexadecimal characters are dropped.
    pub fn load(state: &StateDir) -> Result<Base> {
        let stored: BTreeMap<String, String> =
            read_json(&state.file("base.json"))?.unwrap_or_default();
        let entries = stored
            .iter()
            .filter_map(|(key, hash)| Some((NotePath::parse(key).ok()?.key(), from_hex32(hash)?)))
            .collect();
        Ok(Base { entries })
    }

    /// Writes `base.json` atomically, keys in sorted order.
    pub fn save(&self, state: &StateDir) -> Result<()> {
        let stored: BTreeMap<&str, String> = self
            .entries
            .iter()
            .map(|(key, hash)| (key.as_str(), to_hex(hash)))
            .collect();
        write_json(&state.file("base.json"), &stored)
    }

    /// The agreed hash of a key, if any.
    pub fn get(&self, key: &NoteKey) -> Option<&Hash32> {
        self.entries.get(key)
    }

    /// Records the agreed hash of a key.
    pub fn set(&mut self, key: NoteKey, hash: Hash32) {
        self.entries.insert(key, hash);
    }

    /// Forgets a key.
    pub fn remove(&mut self, key: &NoteKey) {
        self.entries.remove(key);
    }

    /// Every key with an entry, in sorted order.
    pub fn keys(&self) -> impl Iterator<Item = &NoteKey> {
        self.entries.keys()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forgets every key.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Base, DocStore};
    use crate::{NoteView, StateDir, SyncError, to_hex};
    use sb_core::{NoteKey, NotePath};

    fn fresh() -> (tempfile::TempDir, StateDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = StateDir::at(tmp.path().join("state")).unwrap();
        (tmp, state)
    }

    fn path(text: &str) -> NotePath {
        NotePath::parse(text).unwrap()
    }

    fn key(text: &str) -> NoteKey {
        path(text).key()
    }

    fn reopen(state: &StateDir) -> (DocStore, Vec<NoteView>) {
        let (store, doc) = DocStore::open(state).unwrap();
        (store, doc.alive_notes())
    }

    fn append_to_log(state: &StateDir, bytes: &[u8]) {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(state.file("doc.log"))
            .unwrap();
        log.write_all(bytes).unwrap();
    }

    fn file_names(state: &StateDir) -> Vec<String> {
        std::fs::read_dir(state.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn should_start_empty_when_nothing_is_stored() {
        let (_tmp, state) = fresh();
        let (store, doc) = DocStore::open(&state).unwrap();
        assert!(doc.alive_notes().is_empty());
        assert_eq!(store.log_len().unwrap(), 0);
    }

    #[test]
    fn should_reload_what_was_persisted() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        let first = doc.create_note(&path("a.md"), "one").unwrap();
        doc.create_note(&path("b.md"), "two").unwrap();
        store.persist(&doc).unwrap();
        doc.update_note(&first, "one and more").unwrap();
        store.persist(&doc).unwrap();
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, doc.alive_notes());
        assert_eq!(reloaded.len(), 2);
    }

    #[test]
    fn should_not_grow_the_log_when_nothing_changed() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let written = store.log_len().unwrap();
        assert!(written > 0);
        store.persist(&doc).unwrap();
        assert_eq!(store.log_len().unwrap(), written);
    }

    #[test]
    fn should_ignore_a_torn_last_frame() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        let id = doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let after_first = doc.alive_notes();
        let first_frame_len = store.log_len().unwrap();
        doc.update_note(&id, "one and two").unwrap();
        store.persist(&doc).unwrap();
        let log = std::fs::read(state.file("doc.log")).unwrap();
        let cut_in_payload = log.len() - 3;
        let cut_in_header = usize::try_from(first_frame_len).unwrap() + 2;
        for cut in [cut_in_payload, cut_in_header] {
            std::fs::write(state.file("doc.log"), &log[..cut]).unwrap();
            let (mut store, mut doc) = DocStore::open(&state).unwrap();
            assert_eq!(doc.alive_notes(), after_first, "cut at {cut}");
            assert_eq!(store.log_len().unwrap(), first_frame_len, "cut at {cut}");
            doc.update_note(&id, "one and three").unwrap();
            store.persist(&doc).unwrap();
            let (_store, reloaded) = reopen(&state);
            assert_eq!(reloaded, doc.alive_notes(), "cut at {cut}");
            assert_eq!(reloaded[0].text, "one and three", "cut at {cut}");
        }
    }

    #[test]
    fn should_treat_a_zero_filled_tail_as_torn() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        let id = doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let after_first = doc.alive_notes();
        let first_frame_len = store.log_len().unwrap();
        append_to_log(&state, &[0; 16]);

        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        assert_eq!(doc.alive_notes(), after_first);
        assert_eq!(store.log_len().unwrap(), first_frame_len);
        doc.update_note(&id, "one and two").unwrap();
        store.persist(&doc).unwrap();
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, doc.alive_notes());
        assert_eq!(reloaded[0].text, "one and two");
    }

    #[test]
    fn should_heal_a_partial_frame_left_by_a_failed_append() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        let id = doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let mut partial = 100u32.to_be_bytes().to_vec();
        partial.extend_from_slice(&[7; 10]);
        append_to_log(&state, &partial);
        doc.update_note(&id, "one and two").unwrap();
        store.persist(&doc).unwrap();
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, doc.alive_notes());
        assert_eq!(reloaded[0].text, "one and two");
    }

    #[test]
    fn should_report_corrupt_state_when_a_frame_waits_for_missing_history() {
        let (_tmp, source) = fresh();
        let (mut store, mut doc) = DocStore::open(&source).unwrap();
        let id = doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let earlier = doc.version();
        doc.update_note(&id, "one and two").unwrap();
        let later_only = doc.updates_since(&earlier).unwrap();

        let (_tmp, state) = fresh();
        let mut log = u32::try_from(later_only.len())
            .unwrap()
            .to_be_bytes()
            .to_vec();
        log.extend_from_slice(&later_only);
        std::fs::write(state.file("doc.log"), log).unwrap();
        let error = DocStore::open(&state).err().unwrap();
        assert!(
            matches!(&error, SyncError::CorruptState { path, .. } if path.ends_with("doc.log"))
        );
    }

    #[test]
    fn should_compact_into_a_snapshot() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        doc.create_note(&path("a.md"), "one").unwrap();
        doc.create_note(&path("b.md"), "two").unwrap();
        store.persist(&doc).unwrap();
        store.compact(&doc).unwrap();
        assert_eq!(store.log_len().unwrap(), 0);
        assert!(state.file("doc.snap").exists());
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, doc.alive_notes());
        assert!(
            file_names(&state)
                .iter()
                .all(|name| !name.ends_with(".tmp"))
        );
        doc.create_note(&path("c.md"), "three").unwrap();
        store.persist(&doc).unwrap();
        let (_store, after_more) = reopen(&state);
        assert_eq!(after_more, doc.alive_notes());
        assert_eq!(after_more.len(), 3);
    }

    #[test]
    fn should_survive_a_crash_between_snapshot_and_log_truncation() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        let id = doc.create_note(&path("a.md"), "hello").unwrap();
        store.persist(&doc).unwrap();
        doc.update_note(&id, "hello world").unwrap();
        store.persist(&doc).unwrap();
        let old_log = std::fs::read(state.file("doc.log")).unwrap();
        store.compact(&doc).unwrap();
        std::fs::write(state.file("doc.log"), old_log).unwrap();
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, doc.alive_notes());
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].text, "hello world");
    }

    #[test]
    fn should_reset_to_an_empty_document() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        store.compact(&doc).unwrap();
        doc.create_note(&path("b.md"), "two").unwrap();
        store.persist(&doc).unwrap();
        let mut empty = store.reset().unwrap();
        assert!(empty.alive_notes().is_empty());
        assert!(!state.file("doc.snap").exists());
        assert!(!state.file("doc.log").exists());
        store.persist(&empty).unwrap();
        assert_eq!(store.log_len().unwrap(), 0);
        empty.create_note(&path("c.md"), "three").unwrap();
        store.persist(&empty).unwrap();
        let (_store, reloaded) = reopen(&state);
        assert_eq!(reloaded, empty.alive_notes());
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].text, "three");
    }

    #[test]
    fn should_report_corrupt_state_when_the_snapshot_does_not_decode() {
        let (_tmp, state) = fresh();
        std::fs::write(state.file("doc.snap"), b"not a snapshot").unwrap();
        let error = DocStore::open(&state).err().unwrap();
        assert!(
            matches!(&error, SyncError::CorruptState { path, .. } if path.ends_with("doc.snap"))
        );
    }

    #[test]
    fn should_report_corrupt_state_when_a_complete_frame_is_not_an_update() {
        let (_tmp, state) = fresh();
        let mut log = 7u32.to_be_bytes().to_vec();
        log.extend_from_slice(b"garbage");
        std::fs::write(state.file("doc.log"), log).unwrap();
        let error = DocStore::open(&state).err().unwrap();
        assert!(
            matches!(&error, SyncError::CorruptState { path, .. } if path.ends_with("doc.log"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_the_document_files_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        let mode = |name: &str| {
            std::fs::metadata(state.file(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("doc.log"), 0o600);
        store.compact(&doc).unwrap();
        assert_eq!(mode("doc.snap"), 0o600);
        assert_eq!(mode("doc.log"), 0o600);
    }

    #[test]
    fn should_round_trip_the_base() {
        let (_tmp, state) = fresh();
        assert!(Base::load(&state).unwrap().is_empty());
        let mut base = Base::default();
        base.set(key("a.md"), [1; 32]);
        base.set(key("dir/b.md"), [2; 32]);
        base.save(&state).unwrap();
        assert_eq!(Base::load(&state).unwrap(), base);
        base.remove(&key("a.md"));
        base.save(&state).unwrap();
        let loaded = Base::load(&state).unwrap();
        assert_eq!(loaded, base);
        assert_eq!(loaded.get(&key("dir/b.md")), Some(&[2; 32]));
        assert_eq!(loaded.get(&key("a.md")), None);
        assert_eq!(loaded.keys().count(), 1);
        base.clear();
        assert!(base.is_empty());
    }

    #[test]
    fn should_write_the_base_with_sorted_keys_and_hex_values() {
        let (_tmp, state) = fresh();
        let mut base = Base::default();
        base.set(key("b.md"), [0xab; 32]);
        base.set(key("a.md"), [0xcd; 32]);
        base.save(&state).unwrap();
        let text = std::fs::read_to_string(state.file("base.json")).unwrap();
        let expected = format!(
            r#"{{"a.md":"{}","b.md":"{}"}}"#,
            to_hex(&[0xcd; 32]),
            to_hex(&[0xab; 32])
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn should_drop_base_entries_that_are_not_note_keys() {
        let (_tmp, state) = fresh();
        let hex = "ab".repeat(32);
        let json = format!(r#"{{"../x.md": "{hex}", "ok.md": "nothex", "a.md": "{hex}"}}"#);
        std::fs::write(state.file("base.json"), json).unwrap();
        let loaded = Base::load(&state).unwrap();
        assert_eq!(loaded.keys().collect::<Vec<_>>(), vec![&key("a.md")]);
        assert_eq!(loaded.get(&key("a.md")), Some(&[0xab; 32]));
    }

    #[test]
    fn should_forget_the_log_offset_even_when_the_snapshot_cannot_be_removed() {
        let (_tmp, state) = fresh();
        let (mut store, mut doc) = DocStore::open(&state).unwrap();
        doc.create_note(&path("a.md"), "one").unwrap();
        store.persist(&doc).unwrap();
        std::fs::create_dir(state.file("doc.snap")).unwrap();
        assert!(store.reset().is_err());
        assert!(!state.file("doc.log").exists());
        assert_eq!(store.log_good_len, 0);
    }

    #[test]
    fn should_report_corrupt_state_when_the_base_is_not_json() {
        let (_tmp, state) = fresh();
        std::fs::write(state.file("base.json"), "{nope").unwrap();
        assert!(matches!(
            Base::load(&state),
            Err(SyncError::CorruptState { .. })
        ));
    }
}
