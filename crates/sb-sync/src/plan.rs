//! The reconciliation planner (spec section 8.5): a scan of the vault's files and one pure
//! function that decides, per note key, what must change on disk, in the document and in the base.

use std::collections::{BTreeMap, BTreeSet};

use sb_core::{IgnoreReason, Ignored, NoteKey, NotePath, Vault};

use crate::doc::{Hash32, NoteId, NoteView, content_hash};
use crate::error::Result;
use crate::store::Base;

/// What the scan found at one note key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskEntry {
    /// A readable note.
    Present {
        /// The path as spelled on disk.
        path: NotePath,
        /// The file's text.
        content: String,
        /// The hash of `content`.
        hash: Hash32,
    },
    /// Something is there that is not a readable note: it is left alone, never treated as absent.
    Blocked {
        /// The path as the scan reported it.
        path: String,
        /// Why, in words for people.
        reason: String,
    },
}

/// The disk by note key. A key that is not in the map is absent.
pub type DiskState = BTreeMap<NoteKey, DiskEntry>;

/// The result of scanning a vault.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskScan {
    /// Every note file and every blocked file.
    pub disk: DiskState,
    /// Folders that could not be listed: lowercase, `/`-separated, each ending with `/`.
    pub blocked_folders: Vec<String>,
}

/// Reads the vault into a [`DiskScan`]. A file that is ignored because it cannot be used as a
/// note is `Blocked`, and a blocked entry replaces a note that shares its key.
pub fn scan(vault: &Vault) -> Result<DiskScan> {
    let loaded = vault.load()?;
    let mut disk: DiskState = loaded
        .notes
        .into_iter()
        .map(|(path, content)| {
            let hash = content_hash(&content);
            (
                path.key(),
                DiskEntry::Present {
                    path,
                    content,
                    hash,
                },
            )
        })
        .collect();
    let mut blocked_folders = Vec::new();
    for ignored in &loaded.ignored {
        if ignored.path.ends_with('/') {
            blocked_folders.push(ignored.path.to_lowercase());
        } else if let Some((key, entry)) = blocked_entry(ignored) {
            disk.insert(key, entry);
        }
    }
    Ok(DiskScan {
        disk,
        blocked_folders,
    })
}

/// `InvalidPath` cannot be a note and `CaseCollision` keys belong to the file that won.
fn blocked_entry(ignored: &Ignored) -> Option<(NoteKey, DiskEntry)> {
    let reason = match ignored.reason {
        IgnoreReason::NotUtf8 => "not valid UTF-8",
        IgnoreReason::TooLarge => "larger than 4 MiB",
        IgnoreReason::Unreadable => "cannot be read",
        IgnoreReason::InvalidPath | IgnoreReason::CaseCollision => return None,
    };
    let key = NotePath::parse(&ignored.path).ok()?.key();
    let entry = DiskEntry::Blocked {
        path: ignored.path.clone(),
        reason: reason.to_owned(),
    };
    Some((key, entry))
}

/// A change to the sync document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocAction {
    /// Create a note with a fresh id.
    Create {
        /// Where the note lives.
        path: NotePath,
        /// Its text.
        content: String,
    },
    /// Replace a note's text.
    Update {
        /// The note.
        id: NoteId,
        /// The new text.
        content: String,
    },
    /// Delete a note at its current text.
    Tombstone {
        /// The note.
        id: NoteId,
    },
    /// Move a note.
    SetPath {
        /// The note.
        id: NoteId,
        /// Its new display path.
        path: NotePath,
    },
    /// Tombstone `old` at its current text, then create a new note at `path` with `content`.
    Supersede {
        /// The note to tombstone.
        old: NoteId,
        /// Where the new note lives.
        path: NotePath,
        /// The new note's text.
        content: String,
    },
}

/// A change to the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskAction {
    /// Replace a file's content, if it still has the content `expect`.
    Replace {
        /// The path as spelled on disk.
        path: NotePath,
        /// The new text.
        content: String,
        /// The hash the file must still have.
        expect: Hash32,
    },
    /// Create a file; never replaces anything.
    Create {
        /// The note's display path.
        path: NotePath,
        /// The text.
        content: String,
    },
    /// Move a file to the trash, if it still has the content `expect`.
    Trash {
        /// The path as spelled on disk.
        path: NotePath,
        /// The hash the file must still have.
        expect: Hash32,
    },
    /// Rename a file, if it still has the content `expect`.
    Rename {
        /// The path as spelled on disk.
        from: NotePath,
        /// The note's display path.
        to: NotePath,
        /// The hash the file must still have.
        expect: Hash32,
    },
}

/// What the base becomes for a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseChange {
    /// Record this hash.
    Set(Hash32),
    /// Forget the key.
    Clear,
}

/// Everything that has to happen for one note key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The key the step is about.
    pub key: NoteKey,
    /// The document change, if any.
    pub doc: Option<DocAction>,
    /// The file change, if any.
    pub disk: Option<DiskAction>,
    /// Applied to `key` once the actions have succeeded.
    pub base: BaseChange,
    /// A rename also forgets the old key.
    pub clear_base: Option<NoteKey>,
}

/// What the planner looks at.
pub struct PlanInput<'a> {
    /// The scan.
    pub disk: &'a DiskState,
    /// The alive notes by key, after claim resolution.
    pub view: &'a BTreeMap<NoteKey, NoteView>,
    /// The hashes disk and document last agreed on.
    pub base: &'a Base,
    /// Whether a note id has a tombstone for exactly this content hash.
    pub tombstoned: &'a dyn Fn(&NoteId, &Hash32) -> bool,
    /// Every key that starts with one of these is blocked.
    pub blocked_folders: &'a [String],
}

/// Decides what to change, ordered by key; empty when nothing has to change.
pub fn plan(input: &PlanInput<'_>) -> Vec<Step> {
    let keys: BTreeSet<&NoteKey> = input
        .disk
        .keys()
        .chain(input.view.keys())
        .chain(input.base.keys())
        .collect();
    let steps = keys
        .into_iter()
        .filter(|key| !is_blocked(input, key))
        .filter_map(|key| row_step(input, key))
        .collect();
    let steps = pair_renames(steps, input, &LOCAL_RENAME);
    pair_renames(steps, input, &REMOTE_RENAME)
}

fn under_blocked_folder(input: &PlanInput<'_>, key: &NoteKey) -> bool {
    let name = key.as_str();
    input
        .blocked_folders
        .iter()
        .any(|f| name.starts_with(f.as_str()))
}

fn is_blocked(input: &PlanInput<'_>, key: &NoteKey) -> bool {
    matches!(input.disk.get(key), Some(DiskEntry::Blocked { .. }))
        || under_blocked_folder(input, key)
}

/// The readable file at a key.
struct File<'a> {
    path: &'a NotePath,
    content: &'a str,
    hash: Hash32,
}

/// D, H and B of spec section 8.5 for one key.
struct Row<'a> {
    key: &'a NoteKey,
    file: Option<File<'a>>,
    note: Option<&'a NoteView>,
    base: Option<Hash32>,
}

impl<'a> Row<'a> {
    fn new(input: &'a PlanInput<'_>, key: &'a NoteKey) -> Row<'a> {
        let file = match input.disk.get(key) {
            Some(DiskEntry::Present {
                path,
                content,
                hash,
            }) => Some(File {
                path,
                content,
                hash: *hash,
            }),
            _ => None,
        };
        Row {
            key,
            file,
            note: input.view.get(key),
            base: input.base.get(key).copied(),
        }
    }

    fn disk_hash(&self) -> Option<Hash32> {
        self.file.as_ref().map(|file| file.hash)
    }

    fn note_hash(&self) -> Option<Hash32> {
        self.note.map(|note| note.hash)
    }
}

fn step(
    key: &NoteKey,
    doc: Option<DocAction>,
    disk: Option<DiskAction>,
    base: Option<Hash32>,
) -> Step {
    Step {
        key: key.clone(),
        doc,
        disk,
        base: base.map_or(BaseChange::Clear, BaseChange::Set),
        clear_base: None,
    }
}

fn row_step(input: &PlanInput<'_>, key: &NoteKey) -> Option<Step> {
    let row = Row::new(input, key);
    let (disk, note, base) = (row.disk_hash(), row.note_hash(), row.base);
    if disk == note {
        return (disk != base).then(|| step(key, None, None, disk));
    }
    // With disk and note different, at least one of them differs from the base.
    match (disk != base, note != base) {
        (true, false) => document_follows_disk(input, &row),
        (false, true) => disk_follows_document(&row),
        _ => both_changed(input, &row),
    }
}

fn document_follows_disk(input: &PlanInput<'_>, row: &Row<'_>) -> Option<Step> {
    let action = match (&row.file, row.note) {
        (Some(file), Some(note)) if (input.tombstoned)(&note.id, &file.hash) => {
            DocAction::Supersede {
                old: note.id.clone(),
                path: file.path.clone(),
                content: file.content.to_owned(),
            }
        }
        (Some(file), Some(note)) => DocAction::Update {
            id: note.id.clone(),
            content: file.content.to_owned(),
        },
        (Some(file), None) => create_note(file),
        (None, Some(note)) => DocAction::Tombstone {
            id: note.id.clone(),
        },
        (None, None) => return None,
    };
    Some(step(row.key, Some(action), None, row.disk_hash()))
}

fn create_note(file: &File<'_>) -> DocAction {
    DocAction::Create {
        path: file.path.clone(),
        content: file.content.to_owned(),
    }
}

fn disk_follows_document(row: &Row<'_>) -> Option<Step> {
    let action = match (&row.file, row.note) {
        (Some(file), Some(note)) => DiskAction::Replace {
            path: file.path.clone(),
            content: note.text.clone(),
            expect: file.hash,
        },
        (None, Some(note)) => create_file(note),
        (Some(file), None) => DiskAction::Trash {
            path: file.path.clone(),
            expect: file.hash,
        },
        (None, None) => return None,
    };
    Some(step(row.key, None, Some(action), row.note_hash()))
}

fn create_file(note: &NoteView) -> DiskAction {
    DiskAction::Create {
        path: note.path.clone(),
        content: note.text.clone(),
    }
}

fn both_changed(input: &PlanInput<'_>, row: &Row<'_>) -> Option<Step> {
    let doc = match (&row.file, row.note) {
        (Some(file), Some(_)) if already_captured(input, &file.hash) => None,
        (Some(file), _) => Some(create_note(file)),
        (None, Some(note)) => {
            return Some(step(
                row.key,
                None,
                Some(create_file(note)),
                row.note_hash(),
            ));
        }
        (None, None) => return None,
    };
    Some(step(row.key, doc, None, row.disk_hash()))
}

/// An alive note without file or base that already holds the content: an earlier round was
/// interrupted after its document stage.
fn already_captured(input: &PlanInput<'_>, hash: &Hash32) -> bool {
    input.view.iter().any(|(key, note)| {
        note.hash == *hash
            && !input.disk.contains_key(key)
            && input.base.get(key).is_none()
            && !under_blocked_folder(input, key)
    })
}

/// How one kind of rename is recognised among the steps and what it becomes.
struct RenameKind {
    gone: fn(&Step, &PlanInput<'_>) -> Option<Hash32>,
    appeared: fn(&Step, &PlanInput<'_>) -> Option<Hash32>,
    merge: fn(&Step, &Step) -> Option<Step>,
}

const LOCAL_RENAME: RenameKind = RenameKind {
    gone: local_gone,
    appeared: local_appeared,
    merge: into_set_path,
};

const REMOTE_RENAME: RenameKind = RenameKind {
    gone: remote_gone,
    appeared: remote_appeared,
    merge: into_rename,
};

/// The hash of a step that sets a key nobody has a base for.
fn fresh_hash(step: &Step, input: &PlanInput<'_>) -> Option<Hash32> {
    match step.base {
        BaseChange::Set(hash) if input.base.get(&step.key).is_none() => Some(hash),
        _ => None,
    }
}

fn local_gone(step: &Step, input: &PlanInput<'_>) -> Option<Hash32> {
    let Some(DocAction::Tombstone { .. }) = &step.doc else {
        return None;
    };
    let hash = input.view.get(&step.key)?.hash;
    (input.base.get(&step.key) == Some(&hash)).then_some(hash)
}

fn local_appeared(step: &Step, input: &PlanInput<'_>) -> Option<Hash32> {
    let Some(DocAction::Create { .. }) = &step.doc else {
        return None;
    };
    if input.view.contains_key(&step.key) {
        return None;
    }
    fresh_hash(step, input)
}

fn remote_gone(step: &Step, input: &PlanInput<'_>) -> Option<Hash32> {
    let Some(DiskAction::Trash { expect, .. }) = &step.disk else {
        return None;
    };
    (input.base.get(&step.key) == Some(expect)).then_some(*expect)
}

fn remote_appeared(step: &Step, input: &PlanInput<'_>) -> Option<Hash32> {
    let Some(DiskAction::Create { .. }) = &step.disk else {
        return None;
    };
    fresh_hash(step, input)
}

fn into_set_path(gone: &Step, appeared: &Step) -> Option<Step> {
    let (Some(DocAction::Tombstone { id }), Some(DocAction::Create { path, .. })) =
        (&gone.doc, &appeared.doc)
    else {
        return None;
    };
    let action = DocAction::SetPath {
        id: id.clone(),
        path: path.clone(),
    };
    Some(Step {
        doc: Some(action),
        clear_base: Some(gone.key.clone()),
        ..appeared.clone()
    })
}

fn into_rename(gone: &Step, appeared: &Step) -> Option<Step> {
    let (Some(DiskAction::Trash { path: from, expect }), Some(DiskAction::Create { path: to, .. })) =
        (&gone.disk, &appeared.disk)
    else {
        return None;
    };
    let action = DiskAction::Rename {
        from: from.clone(),
        to: to.clone(),
        expect: *expect,
    };
    Some(Step {
        disk: Some(action),
        clear_base: Some(gone.key.clone()),
        ..appeared.clone()
    })
}

/// Index pairs (gone, appeared) with equal content hashes, in key order, each step used once.
fn pair_up(steps: &[Step], input: &PlanInput<'_>, kind: &RenameKind) -> Vec<(usize, usize)> {
    let mut taken = vec![false; steps.len()];
    let mut pairs = Vec::new();
    for (gone, gone_step) in steps.iter().enumerate() {
        let Some(hash) = (kind.gone)(gone_step, input) else {
            continue;
        };
        let found = steps
            .iter()
            .enumerate()
            .find(|(index, step)| !taken[*index] && (kind.appeared)(step, input) == Some(hash));
        if let Some((appeared, _)) = found {
            taken[appeared] = true;
            pairs.push((gone, appeared));
        }
    }
    pairs
}

/// Replaces each recognised pair of steps by one rename step; the result stays ordered by key.
fn pair_renames(steps: Vec<Step>, input: &PlanInput<'_>, kind: &RenameKind) -> Vec<Step> {
    let mut merged = Vec::new();
    let mut used = BTreeSet::new();
    for (gone, appeared) in pair_up(&steps, input, kind) {
        if let Some(step) = (kind.merge)(&steps[gone], &steps[appeared]) {
            merged.push(step);
            used.extend([gone, appeared]);
        }
    }
    let mut result: Vec<Step> = steps
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
        .map(|(_, step)| step)
        .chain(merged)
        .collect();
    result.sort_by(|a, b| a.key.cmp(&b.key));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::content_hash;

    fn h(text: &str) -> Hash32 {
        content_hash(text)
    }

    fn npath(text: &str) -> NotePath {
        NotePath::parse(text).unwrap()
    }

    fn key(text: &str) -> NoteKey {
        npath(text).key()
    }

    fn id(c: char) -> NoteId {
        NoteId::parse(&c.to_string().repeat(32)).unwrap()
    }

    fn present(path: &str, text: &str) -> DiskEntry {
        DiskEntry::Present {
            path: npath(path),
            content: text.to_owned(),
            hash: h(text),
        }
    }

    fn blocked(path: &str) -> DiskEntry {
        DiskEntry::Blocked {
            path: path.to_owned(),
            reason: "not valid UTF-8".to_owned(),
        }
    }

    fn note(c: char, path: &str, text: &str) -> NoteView {
        NoteView {
            id: id(c),
            path: npath(path),
            text: text.to_owned(),
            hash: h(text),
        }
    }

    fn base(entries: &[(&str, &str)]) -> Base {
        let mut base = Base::default();
        for (path, text) in entries {
            base.set(key(path), h(text));
        }
        base
    }

    fn disk_of(entries: Vec<DiskEntry>) -> DiskState {
        let key_of = |entry: &DiskEntry| match entry {
            DiskEntry::Present { path, .. } => path.key(),
            DiskEntry::Blocked { path, .. } => key(path),
        };
        entries
            .into_iter()
            .map(|entry| (key_of(&entry), entry))
            .collect()
    }

    fn view_of(notes: Vec<NoteView>) -> BTreeMap<NoteKey, NoteView> {
        notes
            .into_iter()
            .map(|note| (note.path.key(), note))
            .collect()
    }

    fn run(
        disk: Vec<DiskEntry>,
        notes: Vec<NoteView>,
        base: &Base,
        blocked_folders: &[&str],
    ) -> Vec<Step> {
        let folders: Vec<String> = blocked_folders.iter().map(|f| (*f).to_owned()).collect();
        plan(&PlanInput {
            disk: &disk_of(disk),
            view: &view_of(notes),
            base,
            tombstoned: &|_, _| false,
            blocked_folders: &folders,
        })
    }

    /// One key "n.md" with disk content `d`, note text `t` (id 'a') and base `b`.
    fn row(d: Option<&str>, t: Option<&str>, b: Option<&str>) -> Vec<Step> {
        let entries = b.map(|text| base(&[("n.md", text)])).unwrap_or_default();
        run(
            d.map(|text| present("n.md", text)).into_iter().collect(),
            t.map(|text| note('a', "n.md", text)).into_iter().collect(),
            &entries,
            &[],
        )
    }

    fn step(
        key_text: &str,
        doc: Option<DocAction>,
        disk: Option<DiskAction>,
        base: BaseChange,
    ) -> Step {
        Step {
            key: key(key_text),
            doc,
            disk,
            base,
            clear_base: None,
        }
    }

    fn create_doc(path: &str, text: &str) -> Option<DocAction> {
        Some(DocAction::Create {
            path: npath(path),
            content: text.to_owned(),
        })
    }

    fn create_file(path: &str, text: &str) -> Option<DiskAction> {
        Some(DiskAction::Create {
            path: npath(path),
            content: text.to_owned(),
        })
    }

    fn set(text: &str) -> BaseChange {
        BaseChange::Set(h(text))
    }

    #[test]
    fn should_scan_present_and_blocked_notes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("A.md"), "x").unwrap();
        std::fs::write(tmp.path().join("sub/b.md"), "y").unwrap();
        std::fs::write(tmp.path().join("bin.md"), [0xff]).unwrap();
        std::fs::write(tmp.path().join("img.png"), "png").unwrap();
        let scanned = scan(&Vault::open(tmp.path()).unwrap()).unwrap();
        assert_eq!(scanned.disk.len(), 3);
        assert_eq!(scanned.disk.get(&key("a.md")), Some(&present("A.md", "x")));
        assert_eq!(
            scanned.disk.get(&key("sub/b.md")),
            Some(&present("sub/b.md", "y"))
        );
        let Some(DiskEntry::Blocked { path, reason }) = scanned.disk.get(&key("bin.md")) else {
            panic!("bin.md must be blocked");
        };
        assert_eq!(path, "bin.md");
        assert!(reason.contains("UTF-8"), "reason was {reason:?}");
        assert!(scanned.blocked_folders.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn should_scan_unreadable_files_and_folders_as_blocked() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let (locked, private) = (tmp.path().join("locked.md"), tmp.path().join("Private"));
        std::fs::write(tmp.path().join("ok.md"), "ok").unwrap();
        std::fs::write(&locked, "secret").unwrap();
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("a.md"), "inside").unwrap();
        let set_modes = |file: u32, folder: u32| {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(file)).unwrap();
            std::fs::set_permissions(&private, std::fs::Permissions::from_mode(folder)).unwrap();
        };
        set_modes(0o000, 0o000);
        if std::fs::read(&locked).is_ok() {
            set_modes(0o644, 0o755);
            eprintln!("running as root: permissions are not enforced, test skipped");
            return;
        }
        let scanned = scan(&Vault::open(tmp.path()).unwrap());
        set_modes(0o644, 0o755);
        assert_locked_entries_blocked(&scanned.unwrap());
    }

    #[cfg(unix)]
    fn assert_locked_entries_blocked(scanned: &DiskScan) {
        assert_eq!(
            scanned.disk.get(&key("ok.md")),
            Some(&present("ok.md", "ok"))
        );
        let Some(DiskEntry::Blocked { reason, .. }) = scanned.disk.get(&key("locked.md")) else {
            panic!("locked.md must be blocked");
        };
        assert!(reason.contains("cannot be read"), "reason was {reason:?}");
        assert_eq!(scanned.blocked_folders, vec!["private/".to_owned()]);
        assert!(
            scanned
                .disk
                .keys()
                .all(|k| !k.as_str().starts_with("private/"))
        );
    }

    #[test]
    fn should_plan_nothing_when_all_agree() {
        assert_eq!(row(Some("x"), Some("x"), Some("x")), vec![]);
    }

    #[test]
    fn should_record_base_when_disk_and_note_agree_without_base() {
        let expected = step("n.md", None, None, set("x"));
        assert_eq!(row(Some("x"), Some("x"), None), vec![expected]);
    }

    #[test]
    fn should_clear_stale_base_when_both_are_absent() {
        let expected = step("n.md", None, None, BaseChange::Clear);
        assert_eq!(row(None, None, Some("x")), vec![expected]);
    }

    #[test]
    fn should_update_note_when_only_disk_changed() {
        let update = DocAction::Update {
            id: id('a'),
            content: "x2".to_owned(),
        };
        let expected = step("n.md", Some(update), None, set("x2"));
        assert_eq!(row(Some("x2"), Some("x"), Some("x")), vec![expected]);
    }

    #[test]
    fn should_create_note_when_file_is_new() {
        let expected = step("n.md", create_doc("n.md", "x"), None, set("x"));
        assert_eq!(row(Some("x"), None, None), vec![expected]);
    }

    #[test]
    fn should_tombstone_note_when_file_was_deleted() {
        let tombstone = DocAction::Tombstone { id: id('a') };
        let expected = step("n.md", Some(tombstone), None, BaseChange::Clear);
        assert_eq!(row(None, Some("x"), Some("x")), vec![expected]);
    }

    #[test]
    fn should_replace_file_when_only_note_changed() {
        let replace = DiskAction::Replace {
            path: npath("n.md"),
            content: "x2".to_owned(),
            expect: h("x"),
        };
        let expected = step("n.md", None, Some(replace), set("x2"));
        assert_eq!(row(Some("x"), Some("x2"), Some("x")), vec![expected]);
    }

    #[test]
    fn should_create_file_when_note_is_new() {
        let expected = step("n.md", None, create_file("n.md", "x"), set("x"));
        assert_eq!(row(None, Some("x"), None), vec![expected]);
    }

    #[test]
    fn should_trash_file_when_note_was_deleted() {
        let trash = DiskAction::Trash {
            path: npath("n.md"),
            expect: h("x"),
        };
        let expected = step("n.md", None, Some(trash), BaseChange::Clear);
        assert_eq!(row(Some("x"), None, Some("x")), vec![expected]);
    }

    #[test]
    fn should_add_second_note_when_both_changed() {
        let expected = step("n.md", create_doc("n.md", "x1"), None, set("x1"));
        assert_eq!(row(Some("x1"), Some("x2"), Some("x")), vec![expected]);
    }

    #[test]
    fn should_add_second_note_when_both_exist_without_base() {
        let expected = step("n.md", create_doc("n.md", "x1"), None, set("x1"));
        assert_eq!(row(Some("x1"), Some("x2"), None), vec![expected]);
    }

    #[test]
    fn should_recreate_note_when_edited_locally_and_deleted_remotely() {
        let expected = step("n.md", create_doc("n.md", "x1"), None, set("x1"));
        assert_eq!(row(Some("x1"), None, Some("x")), vec![expected]);
    }

    #[test]
    fn should_restore_file_when_deleted_locally_and_edited_remotely() {
        let expected = step("n.md", None, create_file("n.md", "x2"), set("x2"));
        assert_eq!(row(None, Some("x2"), Some("x")), vec![expected]);
    }

    #[test]
    fn should_supersede_note_when_edit_matches_a_tombstone() {
        let (disk, view, base) = (
            disk_of(vec![present("N.md", "x0")]),
            view_of(vec![note('a', "n.md", "x1")]),
            base(&[("n.md", "x1")]),
        );
        let tombstoned = |note_id: &NoteId, hash: &Hash32| *note_id == id('a') && *hash == h("x0");
        let steps = plan(&PlanInput {
            disk: &disk,
            view: &view,
            base: &base,
            tombstoned: &tombstoned,
            blocked_folders: &[],
        });
        let supersede = DocAction::Supersede {
            old: id('a'),
            path: npath("N.md"),
            content: "x0".to_owned(),
        };
        assert_eq!(steps, vec![step("n.md", Some(supersede), None, set("x0"))]);
    }

    #[test]
    fn should_not_capture_content_twice_when_an_unwritten_note_already_holds_it() {
        let conflict = "n (conflict cccccccc).md";
        let steps = run(
            vec![present("n.md", "mine")],
            vec![note('a', "n.md", "theirs"), note('c', conflict, "mine")],
            &base(&[("n.md", "orig")]),
            &[],
        );
        let expected = vec![
            step(conflict, None, create_file(conflict, "mine"), set("mine")),
            step("n.md", None, None, set("mine")),
        ];
        assert_eq!(steps, expected);
    }

    /// The interrupted-round setup with the other note at `conflict`, plus extra disk and base entries.
    fn n_step_with_other_note(
        conflict: &str,
        extra_disk: Vec<DiskEntry>,
        extra_base: &[(&str, &str)],
        blocked_folders: &[&str],
    ) -> Step {
        let mut entries = base(&[("n.md", "orig")]);
        for (path, text) in extra_base {
            entries.set(key(path), h(text));
        }
        let mut files = vec![present("n.md", "mine")];
        files.extend(extra_disk);
        let notes = vec![note('a', "n.md", "theirs"), note('c', conflict, "mine")];
        let steps = run(files, notes, &entries, blocked_folders);
        steps.into_iter().find(|s| s.key == key("n.md")).unwrap()
    }

    fn captured_again() -> Step {
        step("n.md", create_doc("n.md", "mine"), None, set("mine"))
    }

    #[test]
    fn should_capture_again_when_the_other_note_already_has_a_file() {
        let conflict = "n (conflict cccccccc).md";
        let found = n_step_with_other_note(conflict, vec![present(conflict, "mine")], &[], &[]);
        assert_eq!(found, captured_again());
    }

    #[test]
    fn should_capture_again_when_the_other_note_has_a_base() {
        let conflict = "n (conflict cccccccc).md";
        let found = n_step_with_other_note(conflict, vec![], &[(conflict, "old")], &[]);
        assert_eq!(found, captured_again());
    }

    #[test]
    fn should_capture_again_when_the_other_note_is_under_a_blocked_folder() {
        let found = n_step_with_other_note("private/c.md", vec![], &[], &["private/"]);
        assert_eq!(found, captured_again());
    }

    #[test]
    fn should_leave_blocked_keys_alone() {
        let with_note = run(
            vec![blocked("n.md")],
            vec![note('a', "n.md", "x")],
            &base(&[("n.md", "x")]),
            &[],
        );
        assert_eq!(with_note, vec![]);
        assert_eq!(
            run(vec![blocked("n.md")], vec![], &Base::default(), &[]),
            vec![]
        );
        let tombstone = DocAction::Tombstone { id: id('a') };
        let expected = step("n.md", Some(tombstone), None, BaseChange::Clear);
        let absent = run(
            vec![],
            vec![note('a', "n.md", "x")],
            &base(&[("n.md", "x")]),
            &[],
        );
        assert_eq!(absent, vec![expected]);
    }

    #[test]
    fn should_leave_keys_under_a_blocked_folder_alone() {
        let notes = || vec![note('a', "Private/a.md", "x")];
        let entries = base(&[("private/a.md", "x")]);
        assert_eq!(run(vec![], notes(), &entries, &["private/"]), vec![]);
        let tombstone = DocAction::Tombstone { id: id('a') };
        let expected = step("private/a.md", Some(tombstone), None, BaseChange::Clear);
        assert_eq!(run(vec![], notes(), &entries, &[]), vec![expected]);
    }

    #[test]
    fn should_not_pair_a_rename_with_a_key_under_a_blocked_folder() {
        let steps = run(
            vec![present("new.md", "x")],
            vec![note('a', "private/old.md", "x")],
            &base(&[("private/old.md", "x")]),
            &["private/"],
        );
        assert_eq!(
            steps,
            vec![step("new.md", create_doc("new.md", "x"), None, set("x"))]
        );
    }

    #[test]
    fn should_pair_a_local_rename() {
        let steps = run(
            vec![present("new.md", "x")],
            vec![note('a', "old.md", "x")],
            &base(&[("old.md", "x")]),
            &[],
        );
        let set_path = DocAction::SetPath {
            id: id('a'),
            path: npath("new.md"),
        };
        let mut expected = step("new.md", Some(set_path), None, set("x"));
        expected.clear_base = Some(key("old.md"));
        assert_eq!(steps, vec![expected]);
    }

    #[test]
    fn should_pair_a_remote_rename() {
        let steps = run(
            vec![present("Old.md", "x")],
            vec![note('a', "new.md", "x")],
            &base(&[("old.md", "x")]),
            &[],
        );
        let rename = DiskAction::Rename {
            from: npath("Old.md"),
            to: npath("new.md"),
            expect: h("x"),
        };
        let mut expected = step("new.md", None, Some(rename), set("x"));
        expected.clear_base = Some(key("old.md"));
        assert_eq!(steps, vec![expected]);
    }

    #[test]
    fn should_not_pair_when_content_differs() {
        let steps = run(
            vec![present("new.md", "y")],
            vec![note('a', "old.md", "x")],
            &base(&[("old.md", "x")]),
            &[],
        );
        let tombstone = DocAction::Tombstone { id: id('a') };
        let expected = vec![
            step("new.md", create_doc("new.md", "y"), None, set("y")),
            step("old.md", Some(tombstone), None, BaseChange::Clear),
        ];
        assert_eq!(steps, expected);
    }

    #[test]
    fn should_pair_each_key_once_in_key_order() {
        let steps = run(
            vec![present("c.md", "x"), present("d.md", "x")],
            vec![note('a', "a.md", "x"), note('b', "b.md", "x")],
            &base(&[("a.md", "x"), ("b.md", "x")]),
            &[],
        );
        let moved = |new: &str, old: &str, c: char| {
            let set_path = DocAction::SetPath {
                id: id(c),
                path: npath(new),
            };
            let mut expected = step(new, Some(set_path), None, set("x"));
            expected.clear_base = Some(key(old));
            expected
        };
        assert_eq!(
            steps,
            vec![moved("c.md", "a.md", 'a'), moved("d.md", "b.md", 'b')]
        );
    }

    #[test]
    fn should_not_pair_a_rename_with_a_both_changed_row() {
        // Local side: "new.md" was edited locally against a remote delete (base present).
        let local = run(
            vec![present("new.md", "x1")],
            vec![note('a', "old.md", "x1")],
            &base(&[("old.md", "x1"), ("new.md", "x")]),
            &[],
        );
        let tombstone = DocAction::Tombstone { id: id('a') };
        let expected_local = vec![
            step("new.md", create_doc("new.md", "x1"), None, set("x1")),
            step("old.md", Some(tombstone), None, BaseChange::Clear),
        ];
        assert_eq!(local, expected_local);
        // Remote side: "new.md" was deleted locally against a remote edit (base present).
        let remote = run(
            vec![present("old.md", "x2")],
            vec![note('b', "new.md", "x2")],
            &base(&[("old.md", "x2"), ("new.md", "x")]),
            &[],
        );
        let trash = DiskAction::Trash {
            path: npath("old.md"),
            expect: h("x2"),
        };
        let expected_remote = vec![
            step("new.md", None, create_file("new.md", "x2"), set("x2")),
            step("old.md", None, Some(trash), BaseChange::Clear),
        ];
        assert_eq!(remote, expected_remote);
    }

    #[test]
    fn should_keep_the_disk_spelling_when_replacing() {
        let steps = run(
            vec![present("Notes/Idea.md", "old")],
            vec![note('a', "notes/idea.md", "new")],
            &base(&[("notes/idea.md", "old")]),
            &[],
        );
        let replace = DiskAction::Replace {
            path: npath("Notes/Idea.md"),
            content: "new".to_owned(),
            expect: h("old"),
        };
        assert_eq!(
            steps,
            vec![step("notes/idea.md", None, Some(replace), set("new"))]
        );
    }

    #[test]
    fn should_order_steps_by_key() {
        let steps = run(
            vec![present("a.md", "x"), present("m.md", "y")],
            vec![note('e', "z.md", "x")],
            &base(&[("z.md", "x")]),
            &[],
        );
        let keys: Vec<&str> = steps.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["a.md", "m.md"]);
    }
}
