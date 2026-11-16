//! Carrying out a reconciliation (spec section 8.5): one replica owns a vault, its state
//! directory, the sync document, the base and the store.

use std::collections::BTreeSet;

use sb_core::{CoreError, NotePath, Vault};

use crate::doc::{Hash32, SyncDoc, content_hash};
use crate::error::{Result, SyncError};
use crate::plan::{
    BaseChange, DiskAction, DiskEntry, DiskScan, DocAction, PlanInput, Step, plan, scan,
};
use crate::state::{
    BlockedNote, StateDir, from_hex, io_error, read_optional, remove_if_exists, to_hex,
    write_atomic,
};
use crate::store::{Base, COMPACT_LOG_BYTES, DocStore};

/// The most document-plus-disk passes one `reconcile` call makes.
pub const MAX_PASSES: u8 = 8;

const MARKER_DIR: &str = ".secondbrain";
const BASE_VERSION_FILE: &str = "base.version";
const CHANGED_WHILE_SYNCING: &str = "changed while syncing";

/// The result of one file action: the reason when it was refused.
type FileResult = std::result::Result<(), String>;

/// What a `reconcile` call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the document changed.
    pub doc_changed: bool,
    /// Whether any file was created, replaced, renamed or moved to the trash.
    pub disk_changed: bool,
    /// Notes that could not be synced in the last pass, each path once.
    pub blocked: Vec<BlockedNote>,
    /// How many passes ran.
    pub passes: u8,
}

impl Outcome {
    fn absorb(&mut self, pass: Outcome) {
        self.doc_changed |= pass.doc_changed;
        self.disk_changed |= pass.disk_changed;
        self.blocked = pass.blocked;
        self.passes += pass.passes;
    }
}

/// What the last document stage leaves for the disk stage.
#[derive(Default)]
struct StageRecord {
    /// The document version right after the stage was persisted.
    version: Vec<u8>,
    doc_changed: bool,
    scan_blocked: Vec<BlockedNote>,
}

/// A vault together with everything needed to reconcile it with the sync document.
pub struct LocalReplica {
    /// The vault.
    pub vault: Vault,
    /// The state directory.
    pub state: StateDir,
    /// The sync document.
    pub doc: SyncDoc,
    /// What disk and document last agreed on.
    pub base: Base,
    /// The document's durable storage.
    pub store: DocStore,
    stage: StageRecord,
}

impl LocalReplica {
    /// Opens the replica: loads document and base, drops a base whose document lost history
    /// (spec section 8.3), and creates the vault's `.secondbrain` folder.
    ///
    /// # Errors
    /// `VaultMarkerMissing` when sync state exists but the vault's `.secondbrain` folder is gone.
    pub fn open(vault: Vault, state: StateDir) -> Result<LocalReplica> {
        ensure_marker(&vault, &state)?;
        let (store, doc) = DocStore::open(&state)?;
        let mut base = Base::load(&state)?;
        drop_base_without_history(&state, &doc, &mut base)?;
        Ok(LocalReplica {
            vault,
            state,
            doc,
            base,
            store,
            stage: StageRecord::default(),
        })
    }

    /// Makes disk and document agree, repeating until nothing changes or `MAX_PASSES` is
    /// reached (the next call then continues). A file that cannot be handled is reported in
    /// `Outcome::blocked`; only failures of the state directory or the document are errors.
    pub fn reconcile(&mut self) -> Result<Outcome> {
        let mut total = Outcome::default();
        for _ in 0..MAX_PASSES {
            let steps = self.run_doc_stage()?;
            let pass = self.run_disk_stage(steps)?;
            let quiet = !pass.doc_changed && !pass.disk_changed;
            total.absorb(pass);
            if quiet {
                break;
            }
        }
        Ok(total)
    }

    /// Scans, resolves path claims, plans, applies the document actions and persists the document.
    pub(crate) fn run_doc_stage(&mut self) -> Result<Vec<Step>> {
        let before = self.doc.version();
        let scan = scan(&self.vault)?;
        self.doc.resolve_claims()?;
        let steps = self.plan_steps(&scan);
        for action in steps.iter().filter_map(|step| step.doc.as_ref()) {
            apply_doc_action(&mut self.doc, action)?;
        }
        self.store.persist(&self.doc)?;
        if self.store.log_len()? > COMPACT_LOG_BYTES {
            self.store.compact(&self.doc)?;
        }
        let version = self.doc.version();
        self.stage = StageRecord {
            doc_changed: version != before,
            version,
            scan_blocked: blocked_notes(&scan),
        };
        Ok(steps)
    }

    /// Carries the file actions out, applies the base changes of the steps that succeeded and
    /// saves the base, then `base.version`.
    pub(crate) fn run_disk_stage(&mut self, steps: Vec<Step>) -> Result<Outcome> {
        let mut outcome = Outcome {
            doc_changed: self.stage.doc_changed,
            passes: 1,
            blocked: self.stage.scan_blocked.clone(),
            ..Outcome::default()
        };
        for step in &steps {
            match self.carry_out(step) {
                Ok(changed) => {
                    outcome.disk_changed |= changed;
                    self.apply_base(step);
                }
                Err(blocked) => outcome.blocked.push(blocked),
            }
        }
        self.base.save(&self.state)?;
        let version_file = self.state.file(BASE_VERSION_FILE);
        write_atomic(&version_file, to_hex(&self.stage.version).as_bytes())?;
        outcome.blocked = unique_by_path(outcome.blocked);
        Ok(outcome)
    }

    fn plan_steps(&self, scan: &DiskScan) -> Vec<Step> {
        let view = self.doc.view();
        let doc = &self.doc;
        plan(&PlanInput {
            disk: &scan.disk,
            view: &view,
            base: &self.base,
            tombstoned: &|id, hash| doc.is_tombstoned(id, hash),
            blocked_folders: &scan.blocked_folders,
        })
    }

    /// Whether a file was changed; a refused action becomes a blocked note.
    fn carry_out(&self, step: &Step) -> std::result::Result<bool, BlockedNote> {
        let Some(action) = &step.disk else {
            return Ok(false);
        };
        apply_disk_action(&self.vault, action)
            .map(|()| true)
            .map_err(|reason| BlockedNote {
                path: action_path(action).to_string(),
                reason,
            })
    }

    fn apply_base(&mut self, step: &Step) {
        match step.base {
            BaseChange::Set(hash) => self.base.set(step.key.clone(), hash),
            BaseChange::Clear => self.base.remove(&step.key),
        }
        if let Some(old) = &step.clear_base {
            self.base.remove(old);
        }
    }
}

/// Without the folder, a vault that has sync state is not the vault that was synced.
fn ensure_marker(vault: &Vault, state: &StateDir) -> Result<()> {
    let marker = vault.root().join(MARKER_DIR);
    if marker.is_dir() {
        return Ok(());
    }
    let has_state = ["doc.snap", "doc.log", "base.json"]
        .iter()
        .any(|name| state.file(name).exists());
    if has_state {
        return Err(SyncError::VaultMarkerMissing(marker));
    }
    std::fs::create_dir_all(&marker).map_err(io_error(&marker))
}

/// A base the loaded document cannot vouch for would turn every note into a remote deletion.
fn drop_base_without_history(state: &StateDir, doc: &SyncDoc, base: &mut Base) -> Result<()> {
    if base.is_empty() || base_version_is_known(state, doc)? {
        return Ok(());
    }
    base.clear();
    base.save(state)?;
    remove_if_exists(&state.file(BASE_VERSION_FILE))
}

fn base_version_is_known(state: &StateDir, doc: &SyncDoc) -> Result<bool> {
    let Some(bytes) = read_optional(&state.file(BASE_VERSION_FILE))? else {
        return Ok(false);
    };
    let version = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| from_hex(text.trim()))
        .filter(|version| !version.is_empty());
    Ok(version.is_some_and(|version| doc.includes_version(&version).unwrap_or(false)))
}

fn apply_doc_action(doc: &mut SyncDoc, action: &DocAction) -> Result<()> {
    match action {
        DocAction::Create { path, content } => doc.create_note(path, content).map(drop),
        DocAction::Update { id, content } => doc.update_note(id, content),
        DocAction::Tombstone { id } => doc.tombstone(id),
        DocAction::SetPath { id, path } => doc.set_path(id, path),
        DocAction::Supersede { old, path, content } => {
            doc.tombstone(old)?;
            doc.create_note(path, content).map(drop)
        }
    }
}

fn blocked_notes(scan: &DiskScan) -> Vec<BlockedNote> {
    let files = scan.disk.values().filter_map(|entry| match entry {
        DiskEntry::Blocked { path, reason } => Some(BlockedNote {
            path: path.clone(),
            reason: reason.clone(),
        }),
        DiskEntry::Present { .. } => None,
    });
    let folders = scan.blocked_folders.iter().map(|folder| BlockedNote {
        path: folder.clone(),
        reason: "folder cannot be listed".to_owned(),
    });
    files.chain(folders).collect()
}

fn unique_by_path(blocked: Vec<BlockedNote>) -> Vec<BlockedNote> {
    let mut seen = BTreeSet::new();
    blocked
        .into_iter()
        .filter(|note| seen.insert(note.path.clone()))
        .collect()
}

fn action_path(action: &DiskAction) -> &NotePath {
    match action {
        DiskAction::Replace { path, .. }
        | DiskAction::Create { path, .. }
        | DiskAction::Trash { path, .. } => path,
        DiskAction::Rename { from, .. } => from,
    }
}

fn describe(error: CoreError) -> String {
    error.to_string()
}

fn apply_disk_action(vault: &Vault, action: &DiskAction) -> FileResult {
    match action {
        DiskAction::Replace {
            path,
            content,
            expect,
        } => replace_file(vault, path, content, expect),
        DiskAction::Create { path, content } => create_file(vault, path, content),
        DiskAction::Trash { path, expect } => trash_file(vault, path, expect),
        DiskAction::Rename { from, to, expect } => rename_file(vault, from, to, expect),
    }
}

fn replace_file(vault: &Vault, path: &NotePath, content: &str, expect: &Hash32) -> FileResult {
    let unchanged = |bytes: &[u8]| blake3::hash(bytes).as_bytes() == expect;
    match vault.replace_if(path, content, &unchanged) {
        Ok(true) => Ok(()),
        Ok(false) => Err(CHANGED_WHILE_SYNCING.to_owned()),
        Err(error) => Err(describe(error)),
    }
}

fn create_file(vault: &Vault, path: &NotePath, content: &str) -> FileResult {
    let spelled = vault.adopt_folders(path).map_err(describe)?;
    vault.create(&spelled, content).map_err(describe)
}

/// Any failure to read the file counts as a change: the file is not what the scan saw.
fn still_has_content(vault: &Vault, path: &NotePath, expect: &Hash32) -> FileResult {
    match vault.read(path) {
        Ok(content) if content_hash(&content) == *expect => Ok(()),
        _ => Err(CHANGED_WHILE_SYNCING.to_owned()),
    }
}

fn trash_file(vault: &Vault, path: &NotePath, expect: &Hash32) -> FileResult {
    still_has_content(vault, path, expect)?;
    vault.delete(path).map_err(describe)?;
    remove_empty_folders(vault, path);
    Ok(())
}

fn rename_file(vault: &Vault, from: &NotePath, to: &NotePath, expect: &Hash32) -> FileResult {
    still_has_content(vault, from, expect)?;
    let spelled = vault.adopt_folders(to).map_err(describe)?;
    vault.rename(from, &spelled).map_err(describe)?;
    remove_empty_folders(vault, from);
    Ok(())
}

/// Removes the now-empty folders above a note, up to but not including the vault root.
/// `remove_dir` refuses a folder that holds anything, which ends the walk.
fn remove_empty_folders(vault: &Vault, path: &NotePath) {
    let mut folder = path.folder();
    while !folder.is_empty() {
        let dir = folder
            .split('/')
            .fold(vault.root().to_path_buf(), |dir, part| dir.join(part));
        if std::fs::remove_dir(&dir).is_err() {
            return;
        }
        folder = folder.rsplit_once('/').map_or("", |(parent, _)| parent);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use sb_core::{NotePath, Vault};

    use super::*;
    use crate::doc::NoteView;
    use crate::error::SyncError;

    struct Dirs {
        vault: tempfile::TempDir,
        state: tempfile::TempDir,
    }

    impl Dirs {
        fn new(files: &[(&str, &str)]) -> Dirs {
            let dirs = Dirs {
                vault: tempfile::tempdir().unwrap(),
                state: tempfile::tempdir().unwrap(),
            };
            for (path, content) in files {
                dirs.write(path, content);
            }
            dirs
        }

        fn open(&self) -> LocalReplica {
            let vault = Vault::open(self.vault.path()).unwrap();
            LocalReplica::open(vault, StateDir::at(self.state.path()).unwrap()).unwrap()
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.vault.path().join(relative)
        }

        fn write(&self, relative: &str, content: impl AsRef<[u8]>) {
            let file = self.path(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, content).unwrap();
        }

        fn remove(&self, relative: &str) {
            std::fs::remove_file(self.path(relative)).unwrap();
        }

        fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.path(relative)).unwrap()
        }

        fn state_file(&self, name: &str) -> PathBuf {
            self.state.path().join(name)
        }
    }

    fn np(text: &str) -> NotePath {
        NotePath::parse(text).unwrap()
    }

    fn collect_files(dir: &Path, prefix: &str, found: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{prefix}{name}");
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                collect_files(&entry.path(), &format!("{relative}/"), found);
            } else if name.ends_with(".md") {
                found.insert(relative, std::fs::read_to_string(entry.path()).unwrap());
            }
        }
    }

    fn files(replica: &LocalReplica) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        collect_files(replica.vault.root(), "", &mut found);
        found
    }

    fn trash(replica: &LocalReplica) -> Vec<String> {
        match std::fs::read_dir(replica.vault.trash_dir()) {
            Ok(entries) => entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn note(replica: &LocalReplica, path: &str) -> Option<NoteView> {
        let alive = replica.doc.alive_notes();
        alive.into_iter().find(|note| note.path.as_str() == path)
    }

    fn expect_files(replica: &LocalReplica, expected: &[(&str, &str)]) {
        let expected: BTreeMap<String, String> = expected
            .iter()
            .map(|(path, content)| ((*path).to_owned(), (*content).to_owned()))
            .collect();
        assert_eq!(files(replica), expected);
    }

    fn settled(outcome: &Outcome) -> bool {
        !outcome.doc_changed && !outcome.disk_changed && outcome.passes == 1
    }

    #[test]
    fn should_adopt_existing_notes_and_then_do_nothing() {
        let dirs = Dirs::new(&[("a.md", "A\n"), ("d/b.md", "B\n")]);
        let mut replica = dirs.open();
        let first = replica.reconcile().unwrap();
        assert!(first.doc_changed);
        assert_eq!(note(&replica, "a.md").unwrap().text, "A\n");
        assert_eq!(note(&replica, "d/b.md").unwrap().text, "B\n");
        assert_eq!(replica.doc.alive_notes().len(), 2);
        assert_eq!(replica.base.keys().count(), 2);
        let second = replica.reconcile().unwrap();
        assert!(!second.doc_changed && !second.disk_changed);
        assert_eq!(second.passes, 1);
        assert!(dirs.path(".secondbrain").is_dir());
    }

    #[test]
    fn should_follow_local_edits_creates_and_deletes() {
        let dirs = Dirs::new(&[("a.md", "A\n"), ("d/b.md", "B\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a_id = note(&replica, "a.md").unwrap().id;
        dirs.write("a.md", "A2\n");
        dirs.write("c.md", "C\n");
        dirs.remove("d/b.md");
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        assert_eq!((a.id, a.text.as_str()), (a_id, "A2\n"));
        assert_eq!(note(&replica, "c.md").unwrap().text, "C\n");
        assert!(note(&replica, "d/b.md").is_none());
        assert!(trash(&replica).is_empty());
    }

    #[test]
    fn should_keep_the_note_id_when_a_file_is_renamed() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let old_id = note(&replica, "a.md").unwrap().id;
        dirs.remove("a.md");
        dirs.write("z/renamed.md", "A\n");
        replica.reconcile().unwrap();
        let alive = replica.doc.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].id, old_id);
        assert_eq!(alive[0].path.as_str(), "z/renamed.md");
    }

    #[test]
    fn should_write_remote_changes_to_disk() {
        let dirs = Dirs::new(&[("a.md", "A\n"), ("d/b.md", "B\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let (a, b) = (
            note(&replica, "a.md").unwrap(),
            note(&replica, "d/b.md").unwrap(),
        );
        replica.doc.update_note(&a.id, "remote\n").unwrap();
        replica.doc.create_note(&np("new/n.md"), "n").unwrap();
        replica.doc.tombstone(&b.id).unwrap();
        replica.reconcile().unwrap();
        expect_files(&replica, &[("a.md", "remote\n"), ("new/n.md", "n")]);
        let trashed = trash(&replica);
        assert_eq!(trashed.len(), 1);
        assert!(trashed[0].ends_with("-d__b.md"));
        assert!(!dirs.path("d").exists());
    }

    #[test]
    fn should_rename_the_file_when_the_note_was_renamed_remotely() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.set_path(&a.id, &np("moved/a2.md")).unwrap();
        replica.reconcile().unwrap();
        expect_files(&replica, &[("moved/a2.md", "A\n")]);
        assert!(trash(&replica).is_empty());
    }

    fn both_changed_setup() -> (Dirs, LocalReplica) {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.update_note(&a.id, "theirs\n").unwrap();
        dirs.write("a.md", "mine\n");
        (dirs, replica)
    }

    fn assert_two_versions_kept(replica: &LocalReplica) {
        assert_eq!(replica.doc.alive_notes().len(), 2);
        let found = files(replica);
        assert_eq!(found.len(), 2);
        let copy = found.keys().find(|name| *name != "a.md").unwrap();
        let hex = copy.strip_prefix("a (conflict ").unwrap();
        let hex = hex.strip_suffix(").md").unwrap();
        assert!(hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
        let mut contents: Vec<&str> = found.values().map(String::as_str).collect();
        contents.sort_unstable();
        assert_eq!(contents, ["mine\n", "theirs\n"]);
    }

    #[test]
    fn should_keep_both_versions_when_disk_and_note_changed() {
        let (_dirs, mut replica) = both_changed_setup();
        replica.reconcile().unwrap();
        assert_two_versions_kept(&replica);
        let again = replica.reconcile().unwrap();
        assert!(settled(&again));
        assert_two_versions_kept(&replica);
    }

    #[test]
    fn should_let_a_local_edit_win_over_a_remote_delete() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.tombstone(&a.id).unwrap();
        dirs.write("a.md", "edited\n");
        replica.reconcile().unwrap();
        assert_eq!(dirs.read("a.md"), "edited\n");
        let alive = replica.doc.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].path.as_str(), "a.md");
        assert_ne!(alive[0].id, a.id);
        assert!(trash(&replica).is_empty());
    }

    #[test]
    fn should_restore_a_file_deleted_locally_but_edited_remotely() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.update_note(&a.id, "theirs\n").unwrap();
        dirs.remove("a.md");
        replica.reconcile().unwrap();
        assert_eq!(dirs.read("a.md"), "theirs\n");
    }

    #[test]
    fn should_not_delete_a_file_when_an_edit_returns_to_tombstoned_text() {
        let dirs = Dirs::new(&[("a.md", "v0\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.tombstone(&a.id).unwrap();
        replica.doc.update_note(&a.id, "v1\n").unwrap();
        replica.reconcile().unwrap();
        assert_eq!(dirs.read("a.md"), "v1\n");
        dirs.write("a.md", "v0\n");
        replica.reconcile().unwrap();
        expect_files(&replica, &[("a.md", "v0\n")]);
        assert!(trash(&replica).is_empty());
        let alive = replica.doc.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_ne!(alive[0].id, a.id);
    }

    #[test]
    fn should_never_treat_an_unreadable_file_as_deleted() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        dirs.write("a.md", [0xff, 0xfe]);
        let outcome = replica.reconcile().unwrap();
        assert_eq!(note(&replica, "a.md").unwrap().text, "A\n");
        assert_eq!(std::fs::read(dirs.path("a.md")).unwrap(), [0xff, 0xfe]);
        assert_eq!(outcome.blocked.len(), 1);
        assert_eq!(outcome.blocked[0].path, "a.md");
    }

    #[cfg(unix)]
    #[test]
    fn should_never_treat_notes_under_an_unlistable_folder_as_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let dirs = Dirs::new(&[("Private/a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let folder = dirs.path("Private");
        let set_mode = |mode: u32| {
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        set_mode(0o000);
        if std::fs::read_dir(&folder).is_ok() {
            set_mode(0o755);
            eprintln!("skipped: the folder can still be listed (running as root)");
            return;
        }
        let blocked = replica.reconcile();
        set_mode(0o755);
        let outcome = blocked.unwrap();
        assert!(note(&replica, "Private/a.md").is_some());
        assert_eq!(outcome.blocked.len(), 1);
        assert_eq!(outcome.blocked[0].path, "private/");
        assert!(settled(&replica.reconcile().unwrap()));
        assert!(replica.reconcile().unwrap().blocked.is_empty());
    }

    #[test]
    fn should_create_remote_notes_in_the_existing_folder_spelling() {
        let dirs = Dirs::new(&[("Projects/a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        replica
            .doc
            .create_note(&np("projects/new.md"), "n")
            .unwrap();
        replica.reconcile().unwrap();
        expect_files(
            &replica,
            &[("Projects/a.md", "A\n"), ("Projects/new.md", "n")],
        );
        let names: Vec<String> = std::fs::read_dir(dirs.vault.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|name| name == "Projects"));
        assert!(!names.iter().any(|name| name == "projects"));
    }

    #[test]
    fn should_not_overwrite_what_is_in_the_way_of_a_new_file() {
        let dirs = Dirs::new(&[]);
        std::fs::create_dir(dirs.path("x.md")).unwrap();
        let mut replica = dirs.open();
        replica.doc.create_note(&np("x.md"), "remote").unwrap();
        let outcome = replica.reconcile().unwrap();
        assert!(dirs.path("x.md").is_dir());
        assert_eq!(outcome.blocked.len(), 1);
        assert_eq!(outcome.blocked[0].path, "x.md");
        assert!(note(&replica, "x.md").is_some());
    }

    #[test]
    fn should_block_when_the_file_changed_after_the_scan() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.update_note(&a.id, "remote\n").unwrap();
        let steps = replica.run_doc_stage().unwrap();
        dirs.write("a.md", "late\n");
        let outcome = replica.run_disk_stage(steps).unwrap();
        assert_eq!(dirs.read("a.md"), "late\n");
        assert_eq!(outcome.blocked.len(), 1);
        assert_eq!(outcome.blocked[0].path, "a.md");
        replica.reconcile().unwrap();
        let found = files(&replica);
        assert_eq!(found.len(), 2);
        let mut contents: Vec<&str> = found.values().map(String::as_str).collect();
        contents.sort_unstable();
        assert_eq!(contents, ["late\n", "remote\n"]);
    }

    #[test]
    fn should_finish_after_a_crash_before_files_were_written() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.update_note(&a.id, "remote\n").unwrap();
        replica.run_doc_stage().unwrap();
        drop(replica);
        let mut reopened = dirs.open();
        reopened.reconcile().unwrap();
        assert_eq!(dirs.read("a.md"), "remote\n");
        assert_eq!(reopened.doc.alive_notes().len(), 1);
    }

    #[test]
    fn should_finish_after_a_crash_before_the_base_was_saved() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        let previous_base = std::fs::read(dirs.state_file("base.json")).unwrap();
        let a = note(&replica, "a.md").unwrap();
        replica.doc.update_note(&a.id, "remote\n").unwrap();
        replica.reconcile().unwrap();
        std::fs::write(dirs.state_file("base.json"), previous_base).unwrap();
        drop(replica);
        let mut reopened = dirs.open();
        let outcome = reopened.reconcile().unwrap();
        assert!(!outcome.doc_changed && !outcome.disk_changed);
        expect_files(&reopened, &[("a.md", "remote\n")]);
        let alive = reopened.doc.alive_notes();
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].id, a.id);
    }

    #[test]
    fn should_not_duplicate_a_conflict_copy_after_a_crash() {
        let (dirs, mut replica) = both_changed_setup();
        replica.run_doc_stage().unwrap();
        drop(replica);
        let mut reopened = dirs.open();
        reopened.reconcile().unwrap();
        reopened.reconcile().unwrap();
        assert_eq!(reopened.doc.alive_notes().len(), 2);
        assert_eq!(files(&reopened).len(), 2);
    }

    fn forget_document(dirs: &Dirs) {
        for name in ["doc.snap", "doc.log"] {
            let _ = std::fs::remove_file(dirs.state_file(name));
        }
    }

    #[test]
    fn should_not_trash_notes_when_the_document_lost_history() {
        let dirs = Dirs::new(&[("a.md", "A\n"), ("b.md", "B\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        assert_eq!(replica.base.keys().count(), 2);
        assert!(dirs.state_file("base.version").exists());
        drop(replica);
        forget_document(&dirs);
        let mut reopened = dirs.open();
        reopened.reconcile().unwrap();
        expect_files(&reopened, &[("a.md", "A\n"), ("b.md", "B\n")]);
        assert!(trash(&reopened).is_empty());
        assert_eq!(reopened.doc.alive_notes().len(), 2);
        assert_eq!(reopened.base.keys().count(), 2);
    }

    #[test]
    fn should_not_trash_notes_when_the_base_version_is_missing() {
        let dirs = Dirs::new(&[("a.md", "A\n"), ("b.md", "B\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        drop(replica);
        std::fs::remove_file(dirs.state_file("base.version")).unwrap();
        let mut reopened = dirs.open();
        let outcome = reopened.reconcile().unwrap();
        assert!(!outcome.doc_changed && !outcome.disk_changed);
        expect_files(&reopened, &[("a.md", "A\n"), ("b.md", "B\n")]);
        assert!(trash(&reopened).is_empty());
    }

    #[test]
    fn should_refuse_to_run_when_the_vault_marker_is_gone() {
        let dirs = Dirs::new(&[("a.md", "A\n")]);
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        drop(replica);
        std::fs::remove_dir_all(dirs.path(".secondbrain")).unwrap();
        let vault = Vault::open(dirs.vault.path()).unwrap();
        let reopened = LocalReplica::open(vault, StateDir::at(dirs.state.path()).unwrap());
        assert!(matches!(reopened, Err(SyncError::VaultMarkerMissing(_))));
    }

    fn edit_remotely(replica: &mut LocalReplica, path: &str, text: &str) {
        let id = note(replica, path).unwrap().id;
        replica.doc.update_note(&id, text).unwrap();
    }

    fn disturb(replica: &mut LocalReplica, dirs: &Dirs, index: usize) {
        let path = format!("n{}/{index}.md", index % 7);
        match index % 6 {
            0 => dirs.write(&path, format!("local {index}\n")),
            1 => dirs.remove(&path),
            2 => edit_remotely(replica, &path, &format!("remote {index}\n")),
            3 => {
                let id = note(replica, &path).unwrap().id;
                replica.doc.tombstone(&id).unwrap();
            }
            4 => {
                edit_remotely(replica, &path, &format!("theirs {index}\n"));
                dirs.write(&path, format!("mine {index}\n"));
            }
            _ => {}
        }
    }

    #[test]
    fn should_reach_quiescence_within_the_pass_limit() {
        let dirs = Dirs::new(&[]);
        for index in 0..200 {
            dirs.write(
                &format!("n{}/{index}.md", index % 7),
                format!("text {index}\n"),
            );
        }
        let mut replica = dirs.open();
        replica.reconcile().unwrap();
        for index in 0..200 {
            disturb(&mut replica, &dirs, index);
        }
        dirs.write("brand/new.md", "new\n");
        replica
            .doc
            .create_note(&np("remote/new.md"), "r\n")
            .unwrap();
        let outcome = replica.reconcile().unwrap();
        assert!(outcome.passes <= MAX_PASSES);
        assert!(settled(&replica.reconcile().unwrap()));
    }
}
