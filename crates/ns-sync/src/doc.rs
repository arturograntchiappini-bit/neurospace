//! The sync document: a CRDT mirror of the vault's notes (spec section 8.4).
//!
//! Layout: root map `paths` (note id to display path), root map `tombs`
//! (`<note id>:<hex content hash>` to `true`) and one root text `t:<note id>` per note.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::sync::Arc;

use loro::event::{Diff, DiffBatch};
use loro::{
    ContainerID, ContainerType, ExportMode, Frontiers, LoroDoc, LoroMap, LoroValue, UpdateOptions,
    VersionVector,
};
use ns_core::{NoteKey, NotePath};

use crate::error::{Result, SyncError};
use crate::state::{random_bytes, to_hex};

const MAX_RESOLVE_ROUNDS: usize = 16;
// Mirrors the per-component limit of `NotePath::parse` (spec section 5.2).
const MAX_COMPONENT_BYTES: usize = 255;
const NOTE_ID_LEN: usize = 32;
// Measured (release, 10 KB notes): `LoroDoc::diff` costs about 1 ms per edited note and grows
// faster than linearly with the size of a new text (1 MB: 10 s), while recomputing a note costs
// about 30 us. Past this many new operations a full rebuild is cheaper than the diff.
const MAX_DIFF_ATOMS: usize = 20_000;

/// A 32-byte content hash.
pub type Hash32 = [u8; 32];

/// The BLAKE3 hash of a note's UTF-8 bytes.
pub fn content_hash(text: &str) -> Hash32 {
    *blake3::hash(text.as_bytes()).as_bytes()
}

fn doc_error(error: impl Display) -> SyncError {
    SyncError::Doc(error.to_string())
}

/// The random identity of a note: 32 lowercase hexadecimal characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NoteId(String);

impl NoteId {
    /// A fresh random id.
    pub fn random() -> Result<NoteId> {
        Ok(NoteId(to_hex(&random_bytes::<16>()?)))
    }

    /// Accepts exactly 32 characters of `0-9a-f`.
    pub fn parse(text: &str) -> Option<NoteId> {
        let well_formed = text.len() == NOTE_ID_LEN
            && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        well_formed.then(|| NoteId(text.to_owned()))
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first 8 characters, used in conflict names and logs.
    pub fn short(&self) -> &str {
        &self.0[..8]
    }
}

/// An alive note as the document currently holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteView {
    /// The note's id.
    pub id: NoteId,
    /// Its display path.
    pub path: NotePath,
    /// Its text, shared with the document's cache.
    pub text: Arc<str>,
    /// The hash of `text`.
    pub hash: Hash32,
}

/// What [`SyncDoc::resolve_claims`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// How many notes were tombstoned or moved.
    pub changed: usize,
    /// Whether a round found nothing left to settle before the round bound was reached.
    pub settled: bool,
}

/// What the document knows about one note's text (spec section 8.4, cached view).
struct CachedNote {
    text: Arc<str>,
    hash: Hash32,
    /// Whether a tombstone matches `hash`.
    dead: bool,
}

/// Per-note cache plus the document version it reflects; `None` means nothing is cached yet.
#[derive(Default)]
struct ViewCache {
    frontiers: Option<Frontiers>,
    /// Operations in the document when the cache last reflected it.
    atoms: usize,
    notes: HashMap<NoteId, CachedNote>,
    #[cfg(test)]
    recomputed: usize,
    #[cfg(test)]
    diffs: usize,
}

/// The CRDT document that mirrors the notes of one replica. It is `Send` but not `Sync` (the view
/// cache is a `RefCell`): keep it on one thread at a time.
pub struct SyncDoc {
    doc: LoroDoc,
    cache: RefCell<ViewCache>,
}

impl Default for SyncDoc {
    fn default() -> Self {
        Self::new()
    }
}

fn tomb_key(id: &NoteId, hash: &Hash32) -> String {
    format!("{}:{}", id.as_str(), to_hex(hash))
}

fn parse_entry(key: &str, value: &LoroValue) -> Option<(NoteId, NotePath)> {
    let LoroValue::String(path) = value else {
        return None;
    };
    Some((NoteId::parse(key)?, NotePath::parse(path).ok()?))
}

/// The notes whose cached entry a change batch may have invalidated: a changed text `t:<id>` or a
/// changed tombstone `<id>:<hash>`. `None` when anything is not of that shape: the cache is then
/// rebuilt in full rather than trusted.
fn changed_notes(batch: &DiffBatch) -> Option<Vec<NoteId>> {
    let mut ids = Vec::new();
    for (container, diff) in batch.iter() {
        let ContainerID::Root {
            name,
            container_type,
        } = container
        else {
            return None;
        };
        match (name.as_str(), container_type, diff) {
            ("paths", ContainerType::Map, _) => {}
            ("tombs", ContainerType::Map, Diff::Map(delta)) => {
                for key in delta.updated.keys() {
                    ids.push(NoteId::parse(key.split_once(':')?.0)?);
                }
            }
            (text, ContainerType::Text, _) => ids.push(NoteId::parse(text.strip_prefix("t:")?)?),
            _ => return None,
        }
    }
    Some(ids)
}

/// Decodes a version vector and refuses trailing bytes: the decoder ignores them, so `00ff` would
/// read as the empty version, which every document includes.
fn decode_version(bytes: &[u8]) -> Result<VersionVector> {
    let version = VersionVector::decode(bytes).map_err(doc_error)?;
    // The encoding is canonical in length, so a longer input than its re-encoding has trailing bytes.
    if version.encode().len() != bytes.len() {
        return Err(doc_error("version vector has trailing bytes"));
    }
    Ok(version)
}

impl SyncDoc {
    /// An empty document with a fresh random editing identity.
    pub fn new() -> SyncDoc {
        SyncDoc {
            doc: LoroDoc::new(),
            cache: RefCell::default(),
        }
    }

    /// Restores a document written by [`SyncDoc::snapshot`].
    pub fn from_snapshot(bytes: &[u8]) -> Result<SyncDoc> {
        let doc = LoroDoc::from_snapshot(bytes).map_err(doc_error)?;
        Ok(SyncDoc {
            doc,
            cache: RefCell::default(),
        })
    }

    /// The whole document, history included.
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        self.doc.export(ExportMode::Snapshot).map_err(doc_error)
    }

    fn paths(&self) -> LoroMap {
        self.doc.get_map("paths")
    }

    fn tombs(&self) -> LoroMap {
        self.doc.get_map("tombs")
    }

    fn text_of(&self, id: &NoteId) -> String {
        self.doc.get_text(format!("t:{}", id.as_str())).to_string()
    }

    /// Drops the cache entries that changed since the cache was last used. Call before reading
    /// through [`SyncDoc::cached`]. It depends on `LoroDoc::diff`: any `Err` falls back to a full
    /// rebuild, but a panic inside loro (its restore step unwraps) is not caught.
    fn refresh_cache(&self) {
        let mut guard = self.cache.borrow_mut();
        let cache = &mut *guard;
        // An error path that left a transaction open would otherwise hide its edits from the frontiers.
        self.doc.commit();
        let now = self.doc.state_frontiers();
        let atoms = self.atom_count();
        let stale = match &cache.frontiers {
            Some(seen) if *seen == now => return,
            Some(seen) if atoms.saturating_sub(cache.atoms) <= MAX_DIFF_ATOMS => {
                #[cfg(test)]
                {
                    cache.diffs += 1;
                }
                self.doc
                    .diff(seen, &now)
                    .ok()
                    .and_then(|batch| changed_notes(&batch))
            }
            _ => None,
        };
        match stale {
            Some(ids) => ids.iter().for_each(|id| {
                cache.notes.remove(id);
            }),
            None => cache.notes.clear(),
        }
        cache.frontiers = Some(now);
        cache.atoms = atoms;
    }

    /// How many operations the document holds, counted from its version vector.
    fn atom_count(&self) -> usize {
        let version = self.doc.oplog_vv();
        version
            .values()
            .map(|&counter| usize::try_from(counter).unwrap_or(0))
            .sum()
    }

    /// The cached state of a note, computed on first use. Needs a prior `refresh_cache`.
    fn cached(&self, id: &NoteId) -> (Arc<str>, Hash32, bool) {
        let mut cache = self.cache.borrow_mut();
        #[cfg(test)]
        let before = cache.notes.len();
        let note = cache.notes.entry(id.clone()).or_insert_with(|| {
            let text: Arc<str> = self.text_of(id).into();
            let hash = content_hash(&text);
            let dead = self.is_tombstoned(id, &hash);
            CachedNote { text, hash, dead }
        });
        let state = (Arc::clone(&note.text), note.hash, note.dead);
        #[cfg(test)]
        {
            cache.recomputed += cache.notes.len() - before;
        }
        state
    }

    fn path_of(&self, id: &NoteId) -> Option<NotePath> {
        let value = self.paths().get(id.as_str())?.into_value().ok()?;
        parse_entry(id.as_str(), &value).map(|(_, path)| path)
    }

    fn require_known(&self, id: &NoteId) -> Result<()> {
        if self.paths().get(id.as_str()).is_none() {
            return Err(doc_error(format!("unknown note {}", id.short())));
        }
        Ok(())
    }

    fn write_text(&self, id: &NoteId, content: &str) -> Result<()> {
        let text = self.doc.get_text(format!("t:{}", id.as_str()));
        text.update_by_line(content, UpdateOptions::default())
            .map_err(doc_error)?;
        self.doc.commit();
        Ok(())
    }

    /// Creates a note at `path` under a new random id.
    pub fn create_note(&mut self, path: &NotePath, content: &str) -> Result<NoteId> {
        let id = NoteId::random()?;
        self.create_note_with_id(&id, path, content)?;
        Ok(id)
    }

    /// Creates a note under a given id; an id that already has a path is an error.
    pub fn create_note_with_id(
        &mut self,
        id: &NoteId,
        path: &NotePath,
        content: &str,
    ) -> Result<()> {
        if self.paths().get(id.as_str()).is_some() {
            return Err(doc_error(format!("note {} already exists", id.short())));
        }
        self.paths()
            .insert(id.as_str(), path.as_str())
            .map_err(doc_error)?;
        self.write_text(id, content)
    }

    /// Replaces a note's text, line by line.
    pub fn update_note(&mut self, id: &NoteId, content: &str) -> Result<()> {
        self.require_known(id)?;
        self.write_text(id, content)
    }

    /// Renames a note.
    pub fn set_path(&mut self, id: &NoteId, path: &NotePath) -> Result<()> {
        self.require_known(id)?;
        self.paths()
            .insert(id.as_str(), path.as_str())
            .map_err(doc_error)?;
        self.doc.commit();
        Ok(())
    }

    /// Deletes a note as it is now: records a tombstone for its current text.
    /// Does nothing for a note that is not alive.
    pub fn tombstone(&mut self, id: &NoteId) -> Result<()> {
        if self.path_of(id).is_none() {
            return Ok(());
        }
        let hash = content_hash(&self.text_of(id));
        self.tombs()
            .insert(&tomb_key(id, &hash), true)
            .map_err(doc_error)?;
        self.doc.commit();
        Ok(())
    }

    /// Whether `id` has a tombstone for exactly this text hash.
    pub fn is_tombstoned(&self, id: &NoteId, hash: &Hash32) -> bool {
        self.tombs().get(&tomb_key(id, hash)).is_some()
    }

    /// Well-formed entries of `paths` and the number of malformed ones.
    fn claimed_paths(&self) -> (BTreeMap<NoteId, NotePath>, usize) {
        let LoroValue::Map(entries) = self.paths().get_value() else {
            return (BTreeMap::new(), 0);
        };
        let claimed: BTreeMap<NoteId, NotePath> = entries
            .iter()
            .filter_map(|(key, value)| parse_entry(key, value))
            .collect();
        let rejected = entries.len() - claimed.len();
        (claimed, rejected)
    }

    /// The note as the document holds it now; `None` when its text carries a tombstone.
    fn alive_view(&self, id: NoteId, path: NotePath) -> Option<NoteView> {
        let (text, hash, dead) = self.cached(&id);
        (!dead).then_some(NoteView {
            id,
            path,
            text,
            hash,
        })
    }

    /// Alive notes, ordered by id.
    pub fn alive_notes(&self) -> Vec<NoteView> {
        self.refresh_cache();
        let claimed = self.claimed_paths().0;
        let alive = claimed
            .into_iter()
            .filter_map(|(id, path)| self.alive_view(id, path));
        alive.collect()
    }

    /// How many entries of `paths` were skipped as malformed.
    pub fn rejected_entries(&self) -> usize {
        self.claimed_paths().1
    }

    fn claimants_by_key(&self) -> BTreeMap<NoteKey, Vec<NoteView>> {
        let mut groups: BTreeMap<NoteKey, Vec<NoteView>> = BTreeMap::new();
        for note in self.alive_notes() {
            groups.entry(note.path.key()).or_default().push(note);
        }
        groups
    }

    /// The keys that more than one alive note claims, with their claimants ordered by id. Text
    /// is read and hashed only for the notes whose key is claimed more than once.
    fn contested_claims(&self) -> Vec<Vec<NoteView>> {
        self.refresh_cache();
        let mut by_key: BTreeMap<NoteKey, Vec<(NoteId, NotePath)>> = BTreeMap::new();
        for (id, path) in self.claimed_paths().0 {
            by_key.entry(path.key()).or_default().push((id, path));
        }
        let contested = by_key.into_values().filter(|claims| claims.len() > 1);
        let alive = contested.map(|claims| {
            let views = claims.into_iter();
            views
                .filter_map(|(id, path)| self.alive_view(id, path))
                .collect::<Vec<_>>()
        });
        alive.filter(|claimants| claimants.len() > 1).collect()
    }

    /// One note per key: the one with the smallest id.
    pub fn view(&self) -> BTreeMap<NoteKey, NoteView> {
        let groups = self.claimants_by_key();
        groups
            .into_iter()
            .filter_map(|(key, claimants)| Some((key, claimants.into_iter().next()?)))
            .collect()
    }

    /// Settles every path claimed by more than one alive note (spec section 8.4).
    pub fn resolve_claims(&mut self) -> Result<Resolved> {
        self.resolve_within(MAX_RESOLVE_ROUNDS)
    }

    fn resolve_within(&mut self, rounds: usize) -> Result<Resolved> {
        let mut changed = 0;
        for _ in 0..rounds {
            let round = self.resolve_round()?;
            if round == 0 {
                return Ok(Resolved {
                    changed,
                    settled: true,
                });
            }
            changed += round;
        }
        Ok(Resolved {
            changed,
            settled: false,
        })
    }

    fn resolve_round(&mut self) -> Result<usize> {
        let mut changed = 0;
        for claimants in self.contested_claims() {
            let Some((keeper, others)) = claimants.split_first() else {
                continue;
            };
            for note in others {
                self.settle_against(keeper, note)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    fn settle_against(&mut self, keeper: &NoteView, note: &NoteView) -> Result<()> {
        if note.hash == keeper.hash {
            return self.tombstone(&note.id);
        }
        let candidate = conflict_path(&note.path, &note.id);
        // A name cut to fit can map back onto itself; move to the root name to make progress.
        let target = if candidate.key() == note.path.key() {
            root_conflict_path(&note.id, &note.path)
        } else {
            candidate
        };
        self.set_path(&note.id, &target)
    }

    /// The encoded version vector of everything this document holds.
    pub fn version(&self) -> Vec<u8> {
        self.doc.oplog_vv().encode()
    }

    /// Whether this document holds all history up to `version` (an encoded version vector).
    pub fn includes_version(&self, version: &[u8]) -> Result<bool> {
        Ok(self.doc.oplog_vv().includes_vv(&decode_version(version)?))
    }

    /// Everything a document at `version` lacks.
    pub fn updates_since(&self, version: &[u8]) -> Result<Vec<u8>> {
        let version = decode_version(version)?;
        self.doc
            .export(ExportMode::updates(&version))
            .map_err(doc_error)
    }

    /// What a peer at `peer_version` lacks, or `None` when it lacks nothing.
    pub fn updates_for(&self, peer_version: &[u8]) -> Result<Option<Vec<u8>>> {
        let peer = decode_version(peer_version)?;
        if peer.includes_vv(&self.doc.oplog_vv()) {
            return Ok(None);
        }
        self.updates_since(peer_version).map(Some)
    }

    /// The smallest version that includes both `a` and `b`.
    pub fn merge_versions(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
        let mut merged = decode_version(a)?;
        merged.merge(&decode_version(b)?);
        Ok(merged.encode())
    }

    /// Applies updates; `true` when part of them waits for missing history.
    pub fn import(&mut self, updates: &[u8]) -> Result<bool> {
        let status = self.doc.import(updates).map_err(doc_error)?;
        Ok(status.pending.is_some())
    }
}

fn truncate_to_boundary(text: &str, max_bytes: usize) -> &str {
    let end = (0..=max_bytes.min(text.len()))
        .rev()
        .find(|&index| text.is_char_boundary(index))
        .unwrap_or(0);
    &text[..end]
}

fn root_conflict_path(id: &NoteId, original: &NotePath) -> NotePath {
    // `conflict-<32 hex>.md` always parses; `original` only avoids a panic path.
    NotePath::parse(&format!("conflict-{}.md", id.as_str())).unwrap_or_else(|_| original.clone())
}

/// The path a conflict copy moves to: `<folder>/<stem> (conflict <id8>).md`, with the
/// stem shortened to fit, or `conflict-<id>.md` at the root when it still does not.
pub fn conflict_path(original: &NotePath, id: &NoteId) -> NotePath {
    let suffix = format!(" (conflict {}).md", id.short());
    let stem = truncate_to_boundary(original.stem(), MAX_COMPONENT_BYTES - suffix.len());
    let folder = original.folder();
    let separator = if folder.is_empty() { "" } else { "/" };
    NotePath::parse(&format!("{folder}{separator}{stem}{suffix}"))
        .unwrap_or_else(|_| root_conflict_path(id, original))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn p(path: &str) -> NotePath {
        NotePath::parse(path).unwrap()
    }

    fn id(c: char) -> NoteId {
        NoteId::parse(&c.to_string().repeat(32)).unwrap()
    }

    fn sync(a: &mut SyncDoc, b: &mut SyncDoc) {
        if let Some(updates) = a.updates_for(&b.version()).unwrap() {
            b.import(&updates).unwrap();
        }
        if let Some(updates) = b.updates_for(&a.version()).unwrap() {
            a.import(&updates).unwrap();
        }
    }

    fn texts(doc: &SyncDoc) -> BTreeMap<String, String> {
        doc.alive_notes()
            .into_iter()
            .map(|note| (note.path.as_str().to_owned(), note.text.to_string()))
            .collect()
    }

    fn only_text(doc: &SyncDoc) -> String {
        let notes = doc.alive_notes();
        assert_eq!(notes.len(), 1);
        notes[0].text.to_string()
    }

    fn pair_with_note(content: &str) -> (SyncDoc, SyncDoc, NoteId) {
        let mut a = SyncDoc::new();
        let mut b = SyncDoc::new();
        let note = a.create_note(&p("n.md"), content).unwrap();
        sync(&mut a, &mut b);
        (a, b, note)
    }

    #[test]
    fn should_keep_bytes_exactly_when_creating_and_updating() {
        let contents = [
            "",
            "a",
            "a\r\nb\r\n",
            "no newline at end",
            "\u{feff}# t\n🧠 ñ\n",
            "x\ry\n",
        ];
        for first in contents {
            let mut doc = SyncDoc::new();
            let note = doc.create_note(&p("n.md"), first).unwrap();
            assert_eq!(only_text(&doc), first);
            for next in contents {
                doc.update_note(&note, next).unwrap();
                assert_eq!(only_text(&doc), next);
            }
        }
    }

    #[test]
    fn should_merge_edits_of_different_lines() {
        let (mut a, mut b, note) = pair_with_note("l1\nl2\nl3\n");
        a.update_note(&note, "l1 A\nl2\nl3\n").unwrap();
        b.update_note(&note, "l1\nl2\nl3 B\n").unwrap();
        sync(&mut a, &mut b);
        assert_eq!(only_text(&a), "l1 A\nl2\nl3 B\n");
        assert_eq!(only_text(&b), "l1 A\nl2\nl3 B\n");
    }

    #[test]
    fn should_keep_both_versions_when_one_line_is_edited_twice() {
        let (mut a, mut b, note) = pair_with_note("l1\nl2\nl3\n");
        a.update_note(&note, "l1\nl2 A\nl3\n").unwrap();
        b.update_note(&note, "l1\nl2 B\nl3\n").unwrap();
        sync(&mut a, &mut b);
        let text = only_text(&a);
        assert_eq!(text, only_text(&b));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.contains(&"l2 A") && lines.contains(&"l2 B"));
        assert!(text.starts_with("l1\n") && text.ends_with("l3\n"));
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn should_let_an_edit_beat_a_delete() {
        let (mut a, mut b, note) = pair_with_note("t0\n");
        a.tombstone(&note).unwrap();
        b.update_note(&note, "t1\n").unwrap();
        sync(&mut a, &mut b);
        assert_eq!(only_text(&a), "t1\n");
        assert_eq!(only_text(&b), "t1\n");
    }

    #[test]
    fn should_stay_dead_when_deleted_at_two_states() {
        let (mut a, mut b, note) = pair_with_note("t0\n");
        a.tombstone(&note).unwrap();
        b.update_note(&note, "t1\n").unwrap();
        b.tombstone(&note).unwrap();
        sync(&mut a, &mut b);
        assert!(a.alive_notes().is_empty());
        assert!(b.alive_notes().is_empty());
    }

    #[test]
    fn should_remember_tombstones_per_text() {
        let mut doc = SyncDoc::new();
        let note = doc.create_note(&p("n.md"), "t0\n").unwrap();
        doc.tombstone(&note).unwrap();
        assert!(doc.is_tombstoned(&note, &content_hash("t0\n")));
        assert!(!doc.is_tombstoned(&note, &content_hash("t1\n")));
    }

    #[test]
    fn should_ignore_tombstone_of_a_note_that_is_not_alive() {
        let mut doc = SyncDoc::new();
        assert!(doc.tombstone(&id('a')).is_ok());
        let note = doc.create_note(&p("n.md"), "t\n").unwrap();
        doc.tombstone(&note).unwrap();
        assert!(doc.tombstone(&note).is_ok());
        assert!(matches!(
            doc.update_note(&id('a'), "x"),
            Err(SyncError::Doc(_))
        ));
        assert!(matches!(
            doc.set_path(&id('a'), &p("m.md")),
            Err(SyncError::Doc(_))
        ));
    }

    fn claim_pair(path_a: &str, text_a: &str, path_b: &str, text_b: &str) -> (SyncDoc, SyncDoc) {
        let mut a = SyncDoc::new();
        let mut b = SyncDoc::new();
        a.create_note_with_id(&id('0'), &p(path_a), text_a).unwrap();
        b.create_note_with_id(&id('f'), &p(path_b), text_b).unwrap();
        sync(&mut a, &mut b);
        (a, b)
    }

    #[test]
    fn should_drop_a_duplicate_when_two_notes_claim_a_path_with_equal_text() {
        let (mut a, mut b) = claim_pair("x/Idea.md", "same\n", "x/idea.md", "same\n");
        assert_eq!(a.resolve_claims().unwrap().changed, 1);
        let alive = a.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].id, id('0'));
        sync(&mut a, &mut b);
        b.resolve_claims().unwrap();
        assert_eq!(a.view(), b.view());
        assert_eq!(a.resolve_claims().unwrap().changed, 0);
        assert_eq!(b.resolve_claims().unwrap().changed, 0);
    }

    #[test]
    fn should_make_a_conflict_copy_when_two_notes_claim_a_path_with_different_text() {
        let (mut a, mut b) = claim_pair("x/Idea.md", "A\n", "x/idea.md", "B\n");
        a.resolve_claims().unwrap();
        sync(&mut a, &mut b);
        b.resolve_claims().unwrap();
        sync(&mut a, &mut b);
        let expected = BTreeMap::from([
            ("x/Idea.md".to_owned(), "A\n".to_owned()),
            ("x/idea (conflict ffffffff).md".to_owned(), "B\n".to_owned()),
        ]);
        assert_eq!(texts(&a), expected);
        assert_eq!(texts(&b), expected);
    }

    fn resolve_until_stable(doc: &mut SyncDoc) {
        while doc.resolve_claims().unwrap().changed > 0 {}
    }

    #[test]
    fn should_reach_the_same_result_when_both_replicas_resolve_before_syncing() {
        let mut a = SyncDoc::new();
        let mut b = SyncDoc::new();
        a.create_note_with_id(&id('2'), &p("n.md"), "2").unwrap();
        sync(&mut a, &mut b);
        a.create_note_with_id(&id('1'), &p("n.md"), "1").unwrap();
        b.create_note_with_id(&id('3'), &p("n.md"), "2").unwrap();
        a.resolve_claims().unwrap();
        b.resolve_claims().unwrap();
        sync(&mut a, &mut b);
        resolve_until_stable(&mut a);
        resolve_until_stable(&mut b);
        sync(&mut a, &mut b);
        resolve_until_stable(&mut a);
        resolve_until_stable(&mut b);
        assert_eq!(a.view(), b.view());
        assert_eq!(a.view().len(), a.alive_notes().len());
        let alive: Vec<String> = a
            .alive_notes()
            .into_iter()
            .map(|n| n.text.to_string())
            .collect();
        assert!(alive.contains(&"1".to_owned()) && alive.contains(&"2".to_owned()));
    }

    #[test]
    fn should_resolve_again_when_the_conflict_name_is_taken() {
        let mut doc = SyncDoc::new();
        doc.create_note_with_id(&id('0'), &p("n.md"), "a").unwrap();
        doc.create_note_with_id(&id('f'), &p("n.md"), "b").unwrap();
        doc.create_note_with_id(&id('5'), &p("n (conflict ffffffff).md"), "c")
            .unwrap();
        resolve_until_stable(&mut doc);
        assert_eq!(doc.view().len(), doc.alive_notes().len());
        let mut alive: Vec<String> = doc
            .alive_notes()
            .into_iter()
            .map(|n| n.text.to_string())
            .collect();
        alive.sort();
        assert_eq!(alive, ["a", "b", "c"]);
    }

    #[test]
    fn should_move_to_the_root_name_when_the_conflict_name_maps_onto_itself() {
        let stuck = format!("{}{}", "a".repeat(232), " (conflict ffffffff).md");
        assert_eq!(conflict_path(&p(&stuck), &id('f')).as_str(), stuck);
        let mut doc = SyncDoc::new();
        doc.create_note_with_id(&id('0'), &p(&stuck), "a").unwrap();
        doc.create_note_with_id(&id('f'), &p(&stuck), "b").unwrap();
        assert_eq!(doc.resolve_claims().unwrap().changed, 1);
        assert_eq!(doc.resolve_claims().unwrap().changed, 0);
        assert_eq!(
            texts(&doc)["conflict-ffffffffffffffffffffffffffffffff.md"],
            "b"
        );
    }

    #[test]
    fn should_say_when_path_claims_did_not_settle_within_the_bound() {
        let (mut doc, _) = claim_pair("x/Idea.md", "A\n", "x/idea.md", "B\n");
        let cut_short = doc.resolve_within(1).unwrap();
        assert_eq!((cut_short.changed, cut_short.settled), (1, false));
        let rest = doc.resolve_within(1).unwrap();
        assert_eq!((rest.changed, rest.settled), (0, true));
    }

    #[test]
    fn should_build_valid_conflict_paths() {
        let f = id('f');
        assert_eq!(
            conflict_path(&p("x/Idea.md"), &f).as_str(),
            "x/Idea (conflict ffffffff).md"
        );
        assert_eq!(
            conflict_path(&p("Idea.md"), &f).as_str(),
            "Idea (conflict ffffffff).md"
        );
        let long_stem = format!("{}.md", "é".repeat(125));
        let shortened = conflict_path(&p(&long_stem), &f);
        let last = shortened.as_str().rsplit('/').next().unwrap();
        assert!(last.len() <= 255);
        assert!(NotePath::parse(shortened.as_str()).is_ok());
        assert!(shortened.as_str().ends_with(" (conflict ffffffff).md"));
        let long_path = format!(
            "{}{}.md",
            format!("{}/", "d".repeat(200)).repeat(4),
            "x".repeat(213)
        );
        assert_eq!(long_path.len(), 1020);
        assert_eq!(
            conflict_path(&p(&long_path), &f).as_str(),
            "conflict-ffffffffffffffffffffffffffffffff.md"
        );
    }

    #[test]
    fn should_skip_entries_a_peer_made_up() {
        let doc = SyncDoc::new();
        let paths = doc.doc.get_map("paths");
        paths.insert("not-an-id", "a.md").unwrap();
        paths.insert(id('a').as_str(), "../evil.md").unwrap();
        paths.insert(id('b').as_str(), 5i64).unwrap();
        paths.insert(id('c').as_str(), "ok.md").unwrap();
        doc.doc.commit();
        let alive = doc.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].id, id('c'));
        assert_eq!(doc.rejected_entries(), 3);
    }

    #[test]
    fn should_exchange_only_what_is_missing() {
        let mut a = SyncDoc::new();
        let mut b = SyncDoc::new();
        assert_eq!(a.updates_for(&b.version()).unwrap(), None);
        a.create_note(&p("n.md"), "x\n").unwrap();
        let updates = a.updates_for(&b.version()).unwrap().unwrap();
        assert!(!b.import(&updates).unwrap());
        assert_eq!(a.updates_for(&b.version()).unwrap(), None);
        assert_eq!(b.updates_for(&a.version()).unwrap(), None);
        b.import(&updates).unwrap();
        assert_eq!(texts(&a), texts(&b));
    }

    #[test]
    fn should_report_pending_when_history_is_missing() {
        let mut a = SyncDoc::new();
        let v0 = a.version();
        let note = a.create_note(&p("n.md"), "one\n").unwrap();
        let u1 = a.updates_since(&v0).unwrap();
        let v1 = a.version();
        a.update_note(&note, "two\n").unwrap();
        let u2 = a.updates_since(&v1).unwrap();
        let mut c = SyncDoc::new();
        assert!(c.import(&u2).unwrap());
        assert!(c.alive_notes().is_empty());
        assert!(!c.import(&u1).unwrap());
        assert_eq!(texts(&c), texts(&a));
    }

    #[test]
    fn should_merge_versions() {
        let mut a = SyncDoc::new();
        let mut b = SyncDoc::new();
        a.create_note(&p("a.md"), "a").unwrap();
        b.create_note(&p("b.md"), "b").unwrap();
        let merged = SyncDoc::merge_versions(&a.version(), &b.version()).unwrap();
        sync(&mut a, &mut b);
        assert_eq!(a.updates_for(&merged).unwrap(), None);
        assert!(SyncDoc::merge_versions(&a.version(), b"garbage").is_err());
    }

    #[test]
    fn should_tell_whether_the_document_includes_a_version() {
        let mut doc = SyncDoc::new();
        doc.create_note(&p("a.md"), "a").unwrap();
        let earlier = doc.version();
        doc.create_note(&p("b.md"), "b").unwrap();
        assert!(doc.includes_version(&earlier).unwrap());
        assert!(doc.includes_version(&doc.version()).unwrap());
        assert!(!SyncDoc::new().includes_version(&doc.version()).unwrap());
        assert!(matches!(
            doc.includes_version(b"garbage"),
            Err(SyncError::Doc(_))
        ));
    }

    #[test]
    fn should_reject_a_version_that_does_not_decode() {
        let doc = SyncDoc::new();
        assert!(matches!(
            doc.updates_since(b"garbage"),
            Err(SyncError::Doc(_))
        ));
        assert!(matches!(
            doc.updates_for(b"garbage"),
            Err(SyncError::Doc(_))
        ));
    }

    #[test]
    fn should_restore_from_snapshot() {
        let mut doc = SyncDoc::new();
        let kept = doc.create_note(&p("kept.md"), "kept\n").unwrap();
        let dead = doc.create_note(&p("dead.md"), "dead\n").unwrap();
        let moved = doc.create_note(&p("old.md"), "moved\n").unwrap();
        doc.tombstone(&dead).unwrap();
        doc.set_path(&moved, &p("new/place.md")).unwrap();
        let restored = SyncDoc::from_snapshot(&doc.snapshot().unwrap()).unwrap();
        assert_eq!(restored.alive_notes(), doc.alive_notes());
        assert!(restored.is_tombstoned(&dead, &content_hash("dead\n")));
        assert_eq!(texts(&restored)["kept.md"], "kept\n");
        assert_eq!(texts(&restored)["new/place.md"], "moved\n");
        assert!(restored.alive_notes().iter().any(|n| n.id == kept));
        assert!(matches!(
            SyncDoc::from_snapshot(b"garbage"),
            Err(SyncError::Doc(_))
        ));
    }

    fn assert_matches_scratch(doc: &SyncDoc) {
        let fresh = SyncDoc::from_snapshot(&doc.snapshot().unwrap()).unwrap();
        assert_eq!(doc.view(), fresh.view());
        assert_eq!(doc.alive_notes(), fresh.alive_notes());
    }

    fn assert_both_match_scratch(a: &SyncDoc, b: &SyncDoc) {
        assert_matches_scratch(a);
        assert_matches_scratch(b);
    }

    fn create_and_sync_checked(a: &mut SyncDoc, b: &mut SyncDoc) -> [NoteId; 3] {
        let ids = [
            (
                "one.md", "1
",
            ),
            (
                "two.md", "2
",
            ),
            (
                "three.md", "3
",
            ),
        ]
        .map(|(path, text)| a.create_note(&p(path), text).unwrap());
        assert_both_match_scratch(a, b);
        sync(a, b);
        assert_both_match_scratch(a, b);
        ids
    }

    fn edit_rename_and_delete_checked(a: &mut SyncDoc, b: &mut SyncDoc) -> NoteId {
        let [n1, n2, n3] = create_and_sync_checked(a, b);
        b.update_note(
            &n1,
            "1 edited
",
        )
        .unwrap();
        assert_both_match_scratch(a, b);
        a.tombstone(&n2).unwrap();
        assert_both_match_scratch(a, b);
        b.set_path(&n3, &p("moved/three.md")).unwrap();
        sync(a, b);
        assert_both_match_scratch(a, b);
        n1
    }

    fn resolve_claims_checked(a: &mut SyncDoc, b: &mut SyncDoc) {
        a.create_note_with_id(
            &id('0'),
            &p("x.md"),
            "A
",
        )
        .unwrap();
        b.create_note_with_id(
            &id('f'),
            &p("X.md"),
            "B
",
        )
        .unwrap();
        sync(a, b);
        assert_both_match_scratch(a, b);
        a.resolve_claims().unwrap();
        assert_both_match_scratch(a, b);
        b.resolve_claims().unwrap();
        sync(a, b);
        assert_both_match_scratch(a, b);
    }

    #[test]
    fn should_match_a_view_computed_from_scratch_after_every_step() {
        let (mut a, mut b) = (SyncDoc::new(), SyncDoc::new());
        let n1 = edit_rename_and_delete_checked(&mut a, &mut b);
        resolve_claims_checked(&mut a, &mut b);
        a.tombstone(&n1).unwrap();
        assert_both_match_scratch(&a, &b);
        a.update_note(
            &n1, "back
",
        )
        .unwrap();
        assert_both_match_scratch(&a, &b);
        assert!(a.alive_notes().iter().any(|n| n.id == n1));
        a.update_note(
            &n1,
            "1 edited
",
        )
        .unwrap();
        assert_both_match_scratch(&a, &b);
        assert!(!a.alive_notes().iter().any(|n| n.id == n1));
    }

    #[test]
    fn should_match_the_scratch_view_across_a_partial_import() {
        let mut a = SyncDoc::new();
        let v0 = a.version();
        let note = a
            .create_note(
                &p("n.md"),
                "one
",
            )
            .unwrap();
        let u1 = a.updates_since(&v0).unwrap();
        let v1 = a.version();
        a.update_note(
            &note, "two
",
        )
        .unwrap();
        let u2 = a.updates_since(&v1).unwrap();
        let mut b = SyncDoc::new();
        assert_matches_scratch(&b);
        assert!(b.import(&u2).unwrap());
        assert_matches_scratch(&b);
        assert!(!b.import(&u1).unwrap());
        assert_matches_scratch(&b);
        assert_eq!(texts(&b), texts(&a));
    }

    #[test]
    fn should_recompute_only_the_notes_that_changed() {
        let mut doc = SyncDoc::new();
        let ids: Vec<NoteId> = (0..100)
            .map(|i| {
                doc.create_note(
                    &p(&format!("n{i}.md")),
                    "t
",
                )
                .unwrap()
            })
            .collect();
        doc.view();
        doc.update_note(
            &ids[7], "u
",
        )
        .unwrap();
        doc.cache.borrow_mut().recomputed = 0;
        assert_eq!(doc.view().len(), 100);
        assert_eq!(doc.cache.borrow().recomputed, 1);
        doc.tombstone(&ids[8]).unwrap();
        doc.cache.borrow_mut().recomputed = 0;
        assert_eq!(doc.view().len(), 99);
        assert_eq!(doc.cache.borrow().recomputed, 1);
    }

    #[test]
    fn should_not_diff_the_document_once_per_tombstone() {
        let mut doc = SyncDoc::new();
        let ids: Vec<NoteId> = (0..50)
            .map(|i| {
                doc.create_note(
                    &p(&format!("n{i}.md")),
                    "t
",
                )
                .unwrap()
            })
            .collect();
        doc.view();
        doc.cache.borrow_mut().diffs = 0;
        ids.iter().for_each(|id| doc.tombstone(id).unwrap());
        assert!(doc.cache.borrow().diffs <= 1);
        assert!(doc.view().is_empty());
        assert_matches_scratch(&doc);
    }

    #[test]
    fn should_rebuild_instead_of_diffing_after_a_large_change() {
        let mut doc = SyncDoc::new();
        let ids: Vec<NoteId> = (0..10)
            .map(|i| {
                doc.create_note(
                    &p(&format!("n{i}.md")),
                    "t
",
                )
                .unwrap()
            })
            .collect();
        doc.view();
        doc.update_note(&ids[3], &"x".repeat(MAX_DIFF_ATOMS + 1))
            .unwrap();
        doc.cache.borrow_mut().recomputed = 0;
        assert_eq!(doc.view().len(), 10);
        assert_eq!(doc.cache.borrow().recomputed, 10);
        assert_matches_scratch(&doc);
    }

    #[test]
    fn should_see_an_edit_that_was_never_committed() {
        let mut doc = SyncDoc::new();
        let note = doc.create_note(&p("n.md"), "old\n").unwrap();
        doc.view();
        let text = doc.doc.get_text(format!("t:{}", note.as_str()));
        text.insert(0, "new ").unwrap();
        assert_eq!(only_text(&doc), "new old\n");
    }

    #[test]
    fn should_see_a_tombstone_that_was_never_committed() {
        let mut doc = SyncDoc::new();
        let note = doc.create_note(&p("n.md"), "old\n").unwrap();
        doc.view();
        let key = tomb_key(&note, &content_hash("old\n"));
        doc.tombs().insert(&key, true).unwrap();
        assert!(doc.alive_notes().is_empty());
    }

    #[test]
    fn should_recompute_only_the_note_a_peer_edited() {
        let (mut a, mut b, note) = pair_with_note("l1\nl2\n");
        for i in 0..20 {
            a.create_note(&p(&format!("m{i}.md")), "t\n").unwrap();
        }
        sync(&mut a, &mut b);
        b.view();
        a.update_note(&note, "l1\nl2 peer\n").unwrap();
        sync(&mut a, &mut b);
        b.cache.borrow_mut().recomputed = 0;
        let edited = b.alive_notes().into_iter().find(|n| n.id == note).unwrap();
        assert_eq!(&*edited.text, "l1\nl2 peer\n");
        assert_eq!(b.cache.borrow().recomputed, 1);
        assert_matches_scratch(&b);
    }

    fn assert_full_rebuild_after(inject: impl Fn(&LoroDoc)) {
        let (mut a, mut b, _) = pair_with_note("t\n");
        for i in 0..5 {
            a.create_note(&p(&format!("m{i}.md")), "t\n").unwrap();
        }
        sync(&mut a, &mut b);
        b.view();
        inject(&a.doc);
        a.doc.commit();
        sync(&mut a, &mut b);
        b.cache.borrow_mut().recomputed = 0;
        assert_eq!(b.view().len(), 6);
        assert_eq!(b.cache.borrow().recomputed, 6);
        assert_matches_scratch(&b);
    }

    #[test]
    fn should_rebuild_when_a_peer_adds_a_foreign_text_container() {
        assert_full_rebuild_after(|doc| doc.get_text("foreign").insert(0, "x").unwrap());
    }

    #[test]
    fn should_rebuild_when_a_peer_adds_a_tombstone_key_without_a_colon() {
        assert_full_rebuild_after(|doc| doc.get_map("tombs").insert("no-colon", true).unwrap());
    }

    #[test]
    fn should_stay_send_so_the_engine_can_move_it_to_its_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<SyncDoc>();
    }

    #[test]
    fn should_parse_note_ids_strictly() {
        let good = "0123456789abcdef0123456789abcdef";
        assert!(NoteId::parse(good).is_some());
        assert!(NoteId::parse(&good[..31]).is_none());
        assert!(NoteId::parse(&format!("{good}0")).is_none());
        assert!(NoteId::parse(&good.to_uppercase()).is_none());
        assert!(NoteId::parse(&format!("{}g", &good[..31])).is_none());
        let (first, second) = (NoteId::random().unwrap(), NoteId::random().unwrap());
        assert_ne!(first, second);
        assert_eq!(first.short().len(), 8);
    }
}
