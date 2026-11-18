//! The open vault and open note, with every operation the window needs and no UI in sight.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, io, mem};

use sb_core::{
    CoreError, DEFAULT_SEARCH_LIMIT, Index, NotePath, Result, SearchHit, Vault, new_note_path, ops,
    utc_compact,
};

use super::notice::Notice;
use super::tree::{Folder, build_tree};

/// How long after the last keystroke the open note is saved.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);
/// The most notes `quick_open` returns.
pub const QUICK_OPEN_LIMIT: usize = 50;

const CONFLICT_NAME_ATTEMPTS: usize = 5;
const RECOVERED_STEM_CHARS: usize = 50;

/// What `Workspace::apply_index` did with a rebuilt index.
#[derive(Debug)]
pub enum ApplyOutcome {
    /// The index was swapped in; the notices describe what happened to the open note.
    Applied(Vec<Notice>),
    /// The workspace wrote something since the rebuild started: the index was dropped.
    Stale,
}

/// The note being edited.
#[derive(Debug)]
pub struct OpenNote {
    /// Where the note lives.
    pub path: NotePath,
    /// The text as typed.
    pub buffer: String,
    /// The text as last read from or written to disk.
    pub saved: String,
    /// When the buffer last changed; `Some` exactly while it differs from `saved`.
    pub last_edit: Option<Instant>,
    /// Set by a failed save, cleared by the next edit: autosave waits for a keystroke.
    pub autosave_paused: bool,
    /// Words in `buffer`, recounted whenever the buffer is replaced (never per frame).
    words: usize,
    /// The buffer is already in a conflict note; cleared by the next edit.
    kept: bool,
}

/// What `Workspace::save` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// The note was clean; nothing to write.
    Nothing,
    /// The buffer replaced the file.
    Written,
    /// The file no longer holds what was last read (or is gone): nothing was written and the
    /// buffer stays dirty, for the reload to keep in a conflict note.
    ChangedOnDisk,
}

impl OpenNote {
    /// True when the buffer holds text that is not on disk.
    pub fn is_dirty(&self) -> bool {
        self.last_edit.is_some()
    }
}

/// An open vault, its index and tree, and at most one open note.
pub struct Workspace {
    vault: Vault,
    index: Index,
    tree: Folder,
    open: Option<OpenNote>,
    write_generation: u64,
    index_swaps: u64,
    buffer_generation: u64,
    notices: Vec<Notice>,
}

impl Workspace {
    /// Opens the vault at `root` and indexes it. Blocking: call it off the UI thread.
    ///
    /// # Errors
    /// Whatever `Vault::open` or `Index::build` returns.
    pub fn open(root: &Path) -> Result<Workspace> {
        let vault = Vault::open(root)?;
        let index = Index::build(&vault)?;
        let tree = build_tree(index.notes().map(|entry| &entry.path));
        Ok(Workspace {
            vault,
            index,
            tree,
            open: None,
            write_generation: 0,
            index_swaps: 0,
            buffer_generation: 0,
            notices: Vec::new(),
        })
    }

    /// What the workspace has to tell the user about things it did on its own (text moved to a
    /// conflict note); the caller shows them.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        mem::take(&mut self.notices)
    }

    /// The vault on disk.
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// The index of every note.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The folder tree.
    pub fn tree(&self) -> &Folder {
        &self.tree
    }

    /// The note being edited, if any.
    pub fn open_note(&self) -> Option<&OpenNote> {
        self.open.as_ref()
    }

    /// Counts every write the workspace makes (saves, creates, renames, deletes, conflict notes)
    /// and every read that replaces the open note's saved text. An index built before one of
    /// them is stale.
    pub fn write_generation(&self) -> u64 {
        self.write_generation
    }

    /// Changes whenever the open note's buffer is replaced from outside the editor (a note
    /// opened, the file's text adopted, a conflict note taking over). An editor that keys its
    /// undo history on it cannot bring back text from before the replacement.
    pub fn buffer_generation(&self) -> u64 {
        self.buffer_generation
    }

    /// Changes whenever the index content may have changed: every local write and every swapped-in
    /// rebuild. Both parts only grow, so the sum differs from any earlier value. Panels that
    /// derive data from the index key their caches on it.
    pub fn index_revision(&self) -> u64 {
        self.write_generation + self.index_swaps
    }

    /// Opens a note, saving a dirty open one first (or keeping its text in a conflict note, see
    /// `save_or_keep`).
    ///
    /// # Errors
    /// The `save_or_keep` error, or the read error for `path`.
    pub fn select(&mut self, path: &NotePath) -> Result<()> {
        self.save_or_keep()?;
        let text = ops::read_note(&self.vault, path.as_str())?;
        // This read is newer than any rebuild already running.
        self.write_generation += 1;
        self.buffer_generation += 1;
        self.open = Some(clean_note(path.clone(), text));
        Ok(())
    }

    /// Replaces the buffer of the open note; does nothing when no note is open.
    pub fn edit(&mut self, text: String, now: Instant) {
        let Some(open) = self.open.as_mut().filter(|open| open.buffer != text) else {
            return;
        };
        open.last_edit = (text != open.saved).then_some(now);
        open.autosave_paused = false;
        open.kept = false;
        open.words = count_words(&text);
        open.buffer = text;
    }

    /// True when the open note is dirty, `AUTOSAVE_DELAY` has passed since the last edit and no
    /// failed save is waiting for the next edit.
    pub fn autosave_due(&self, now: Instant) -> bool {
        let Some(open) = self.open.as_ref().filter(|open| !open.autosave_paused) else {
            return false;
        };
        open.last_edit
            .is_some_and(|at| now.saturating_duration_since(at) >= AUTOSAVE_DELAY)
    }

    /// Writes a dirty buffer over the file, but only if the file still holds the text last read
    /// or written (compare-and-swap): an external change is never replaced and a removed note is
    /// never recreated. On `ChangedOnDisk` or an error nothing is written, the buffer and dirty
    /// flag are untouched and autosave pauses until the next edit.
    ///
    /// # Errors
    /// Whatever `Vault::locate` or `Vault::replace_if` returns, except `NotFound`.
    pub fn save(&mut self) -> Result<Saved> {
        let Some(open) = self.open.as_mut().filter(|open| open.is_dirty()) else {
            return Ok(Saved::Nothing);
        };
        let stored = match swap_on_disk(&self.vault, open) {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                open.autosave_paused = true;
                return Ok(Saved::ChangedOnDisk);
            }
            Err(error) => {
                open.autosave_paused = true;
                return Err(error);
            }
        };
        open.saved.clone_from(&open.buffer);
        open.last_edit = None;
        open.path = stored.clone();
        self.index.upsert(stored, open.buffer.clone());
        self.write_generation += 1;
        Ok(Saved::Written)
    }

    /// Saves the open note before it is left. When the file cannot be written, or changed on
    /// disk, the typed text goes to a conflict note instead (see `take_notices`) and the caller
    /// carries on: typed text is never lost, nor is the user trapped on a note.
    ///
    /// # Errors
    /// `InvalidInput` when the note can be neither saved nor kept in a conflict note; the buffer
    /// stays open and dirty and the caller must stop.
    pub fn save_or_keep(&mut self) -> Result<()> {
        let failure = match self.save() {
            Ok(Saved::Nothing | Saved::Written) => return Ok(()),
            Ok(Saved::ChangedOnDisk) => None,
            Err(error) => Some(error),
        };
        let path = self.open.as_ref().map(|open| open.path.to_string());
        let path = path.unwrap_or_default();
        match self.keep_unsaved_as_conflict() {
            Ok(Some(conflict)) => {
                self.notices.push(kept_notice(&path, failure, &conflict));
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(kept) => {
                let cause =
                    failure.map_or_else(|| format!("{path} changed on disk"), |e| e.to_string());
                Err(CoreError::InvalidInput(format!(
                    "{cause}; your text could not be kept in a conflict note either ({kept})"
                )))
            }
        }
    }

    /// Creates an empty note called `name` (`.md` added when missing) in `folder` or the vault
    /// root, and opens it.
    ///
    /// # Errors
    /// `InvalidInput` for a blank name, `AlreadyExists`, `InvalidPath`, the `save_or_keep` error
    /// or a write error.
    pub fn create(&mut self, name: &str, folder: Option<&str>) -> Result<NotePath> {
        let file = with_markdown_extension(name)?;
        let folder = folder
            .map(|folder| folder.trim().trim_matches('/'))
            .filter(|folder| !folder.is_empty());
        match folder {
            Some(folder) => self.create_at(&format!("{folder}/{file}")),
            None => self.create_at(&file),
        }
    }

    fn create_at(&mut self, path: &str) -> Result<NotePath> {
        self.save_or_keep()?;
        let created = ops::create_note(&self.vault, path, "")?.path;
        self.write_generation += 1;
        self.index.upsert(created.clone(), String::new());
        self.refresh_tree();
        self.select(&created)?;
        Ok(created)
    }

    /// Moves `from` to `to` (`.md` added when missing). An open note keeps its buffer and follows.
    ///
    /// # Errors
    /// The `save_or_keep` error, `InvalidInput` for a blank name, or what `ops::rename_note`
    /// returns.
    pub fn rename(&mut self, from: &NotePath, to: &str) -> Result<NotePath> {
        let to = with_markdown_extension(to)?;
        if self.is_open(from) {
            self.save_or_keep()?;
        }
        let outcome = ops::rename_note(&self.vault, from.as_str(), &to)?;
        self.write_generation += 1;
        self.index.remove(&outcome.from.key());
        // An unreadable moved note stays out of the index; the next reload re-indexes it.
        if let Ok(text) = self.vault.read(&outcome.to) {
            self.index.upsert(outcome.to.clone(), text);
        }
        if let Some(open) = self
            .open
            .as_mut()
            .filter(|open| open.path.key() == from.key())
        {
            open.path = outcome.to.clone();
        }
        self.refresh_tree();
        Ok(outcome.to)
    }

    /// Moves a note to the vault's trash. An open copy is saved first, so the trash holds the
    /// typed text, and then closed.
    ///
    /// # Errors
    /// The `save_or_keep` error, or whatever `ops::delete_note` returns; then nothing is deleted.
    pub fn delete(&mut self, path: &NotePath) -> Result<()> {
        if self.is_open(path) {
            self.save_or_keep()?;
        }
        let outcome = ops::delete_note(&self.vault, path.as_str())?;
        self.write_generation += 1;
        self.index.remove(&outcome.path.key());
        if self.is_open(path) {
            self.open = None;
        }
        self.refresh_tree();
        Ok(())
    }

    /// Follows a link written in the open note: opens the note it resolves to, or creates it
    /// first (spec 6.6). Attachments and invalid targets create nothing and return an error.
    ///
    /// # Errors
    /// `InvalidInput` when no note is open or the link cannot name a note, `InvalidPath`, or any
    /// error of `select` and `create`.
    pub fn follow_link(&mut self, target: &str) -> Result<NotePath> {
        let Some(from) = self.open.as_ref().map(|open| open.path.clone()) else {
            return Err(CoreError::InvalidInput("no note is open".into()));
        };
        if let Some(existing) = self.index.resolve(target, &from).cloned() {
            self.select(&existing)?;
            return Ok(existing);
        }
        let path = new_note_path(target, &from)?;
        self.create_at(path.as_str())
    }

    /// Creates and opens the note a graph ghost node stands for. The ghost id is lowercase, so the
    /// path comes from the first link (in key order) that produces that id: this keeps the letter
    /// case as the link wrote it (spec 6.3, 6.6).
    ///
    /// # Errors
    /// `NotFound` when no link has that ghost id, or any error of `create_at` (such as
    /// `AlreadyExists`: nothing is replaced).
    pub fn create_from_ghost(&mut self, ghost_id: &str) -> Result<NotePath> {
        let path = self
            .ghost_target(ghost_id)
            .ok_or_else(|| CoreError::NotFound(ghost_id.to_owned()))?;
        self.create_at(path.as_str())
    }

    fn ghost_target(&self, ghost_id: &str) -> Option<NotePath> {
        let wants = ghost_id.strip_prefix('?')?;
        self.index.notes().find_map(|entry| {
            let links = entry.links.iter().filter(|link| !link.is_attachment());
            links
                .filter_map(|link| new_note_path(&link.target, &entry.path).ok())
                .find(|path| path.key().as_str().strip_suffix(".md") == Some(wants))
        })
    }

    /// Notes whose path contains `query` (ignoring case), those whose name starts with it first.
    pub fn quick_open(&self, query: &str) -> Vec<NotePath> {
        if query.is_empty() {
            return Vec::new();
        }
        let query = query.to_lowercase();
        // `index.notes()` is ordered by key, and the sort is stable: ties stay in path order.
        let mut matches: Vec<(bool, &NotePath)> = self
            .index
            .notes()
            .map(|entry| &entry.path)
            .filter(|path| path.key().as_str().contains(&query))
            .map(|path| (!path.stem().to_lowercase().starts_with(&query), path))
            .collect();
        matches.sort_by_key(|(stem_miss, _)| *stem_miss);
        matches
            .into_iter()
            .take(QUICK_OPEN_LIMIT)
            .map(|(_, path)| path.clone())
            .collect()
    }

    /// Full-text search over the index.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        self.index.search(query, DEFAULT_SEARCH_LIMIT)
    }

    /// Whitespace-separated words in the open buffer; 0 when no note is open.
    pub fn word_count(&self) -> usize {
        self.open.as_ref().map_or(0, |open| open.words)
    }

    /// Swaps in an index rebuilt from disk and reconciles the open note with it. `Stale` (nothing
    /// changed) when the workspace wrote something since `started_at_write` (see
    /// `write_generation`): that index may predate our own write, so the caller must rebuild
    /// again.
    pub fn apply_index(&mut self, index: Index, started_at_write: u64) -> ApplyOutcome {
        if self.write_generation != started_at_write {
            return ApplyOutcome::Stale;
        }
        self.index = index;
        self.index_swaps += 1;
        self.refresh_tree();
        ApplyOutcome::Applied(self.reconcile_open_note().into_iter().collect())
    }

    fn reconcile_open_note(&mut self) -> Option<Notice> {
        let open = self.open.as_ref()?;
        let on_disk = self
            .index
            .get(&open.path.key())
            .map(|entry| (entry.path.clone(), entry.content.clone()));
        match on_disk {
            Some((path, text)) if text == open.saved => {
                self.open.as_mut()?.path = path;
                None
            }
            Some((path, text)) => self.adopt_disk_text(path, text),
            None => self.handle_note_gone(),
        }
    }

    /// The note on disk differs from what was last read: the file wins, unsaved edits survive in
    /// a conflict note, unless the buffer already holds exactly what is on disk.
    fn adopt_disk_text(&mut self, path: NotePath, text: String) -> Option<Notice> {
        if self.open.as_ref().is_some_and(|open| open.buffer == text) {
            self.open = Some(clean_note(path, text));
            return None;
        }
        let kept_in = match self.keep_unsaved_edits() {
            Ok(kept_in) => kept_in,
            Err(notice) => return Some(notice),
        };
        self.buffer_generation += 1;
        self.open = Some(clean_note(path.clone(), text));
        kept_in.map(|conflict| {
            Notice::info(format!(
                "Your unsaved edits to {path} were kept in {conflict}"
            ))
        })
    }

    /// The open note is no longer in the index: close it, or when it has unsaved edits open the
    /// conflict note that keeps them.
    fn handle_note_gone(&mut self) -> Option<Notice> {
        let path = self.open.as_ref()?.path.clone();
        let kept_in = match self.keep_unsaved_edits() {
            Ok(kept_in) => kept_in,
            Err(notice) => return Some(notice),
        };
        let Some(conflict) = kept_in else {
            self.open = None;
            return Some(Notice::info(format!(
                "{path} was removed or can no longer be read"
            )));
        };
        let buffer = self.open.take()?.buffer;
        self.buffer_generation += 1;
        self.open = Some(clean_note(conflict.clone(), buffer));
        Some(Notice::info(format!(
            "{path} was removed or can no longer be read; your unsaved edits are in {conflict}"
        )))
    }

    /// Writes a dirty buffer to a conflict note; `None` when the note is clean. On failure the
    /// buffer stays open and dirty and the error notice says so.
    fn keep_unsaved_edits(&mut self) -> std::result::Result<Option<NotePath>, Notice> {
        let path = self.open.as_ref().map(|open| open.path.clone());
        self.keep_unsaved_as_conflict().map_err(|error| {
            let path = path.map(|path| path.to_string()).unwrap_or_default();
            Notice::error(format!(
                "Could not keep your unsaved edits to {path} in a conflict note ({error}); \
                 they stay in the editor"
            ))
        })
    }

    /// Writes the unsaved buffer to `<stem> (conflict <id>).md` next to the note and leaves the
    /// open note as it is. `None` when nothing is unsaved or this text was kept already (a window
    /// close and the exit after it keep it once). For a window that is closing and can no longer
    /// save over a file that changed.
    ///
    /// # Errors
    /// Whatever creating the conflict note returns.
    pub fn keep_unsaved_as_conflict(&mut self) -> Result<Option<NotePath>> {
        let Some(open) = self.open.as_mut().filter(|o| o.is_dirty() && !o.kept) else {
            return Ok(None);
        };
        let (path, buffer) = (open.path.clone(), open.buffer.clone());
        let created = create_conflict_note(&self.vault, &path, &buffer)?;
        open.kept = true;
        self.write_generation += 1;
        self.index.upsert(created.clone(), buffer);
        self.refresh_tree();
        Ok(Some(created))
    }

    /// Last resort for a buffer that can neither be saved nor kept in a conflict note: writes it
    /// to `<state_base>/recovered/<utc time> <stem>.md`. `None` when nothing is unsaved. The stem
    /// is cut so the name fits the file system, and a second write in the same second replaces
    /// the first (same note, newer text).
    pub fn write_recovered(&self, state_base: &Path) -> Option<io::Result<PathBuf>> {
        let open = self.open.as_ref().filter(|open| open.is_dirty())?;
        let folder = state_base.join("recovered");
        let stem: String = open
            .path
            .stem()
            .chars()
            .take(RECOVERED_STEM_CHARS)
            .collect();
        let file = folder.join(format!("{} {stem}.md", utc_compact(SystemTime::now())));
        let written = fs::create_dir_all(&folder).and_then(|()| fs::write(&file, &open.buffer));
        Some(written.map(|()| file))
    }

    fn is_open(&self, path: &NotePath) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.path.key() == path.key())
    }

    fn refresh_tree(&mut self) {
        self.tree = build_tree(self.index.notes().map(|entry| &entry.path));
    }
}

fn kept_notice(path: &str, failure: Option<CoreError>, conflict: &NotePath) -> Notice {
    match failure {
        None => Notice::info(format!(
            "Your unsaved edits to {path} were kept in {conflict}"
        )),
        Some(error) => Notice::error(format!(
            "{path} could not be saved ({error}); your text is in {conflict}"
        )),
    }
}

fn clean_note(path: NotePath, text: String) -> OpenNote {
    OpenNote {
        path,
        words: count_words(&text),
        buffer: text.clone(),
        saved: text,
        last_edit: None,
        autosave_paused: false,
        kept: false,
    }
}

fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Creates `<stem> (conflict <8 hex>).md` next to `path` holding `text`. A name that exists is
/// never replaced: new hex is drawn.
fn create_conflict_note(vault: &Vault, path: &NotePath, text: &str) -> Result<NotePath> {
    let folder = match path.folder() {
        "" => String::new(),
        folder => format!("{folder}/"),
    };
    for _ in 0..CONFLICT_NAME_ATTEMPTS {
        let hex = sb_sync::random_bytes::<4>()
            .map(|bytes| sb_sync::to_hex(&bytes))
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let name = format!("{folder}{} (conflict {hex}).md", path.stem());
        match ops::create_note(vault, &name, text) {
            Err(CoreError::AlreadyExists(_)) => {}
            created => return created.map(|outcome| outcome.path),
        }
    }
    Err(CoreError::AlreadyExists(format!(
        "{folder}{} (conflict …).md",
        path.stem()
    )))
}

/// Replaces the file with the buffer if it still holds `saved`; `Some` is the stored path
/// written to, `None` means it changed or vanished.
fn swap_on_disk(vault: &Vault, open: &OpenNote) -> Result<Option<NotePath>> {
    let Some(stored) = vault.locate(&open.path)? else {
        return Ok(None);
    };
    let expected = open.saved.as_bytes();
    match vault.replace_if(&stored, &open.buffer, &|bytes| bytes == expected) {
        Ok(true) => Ok(Some(stored)),
        Ok(false) | Err(CoreError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn with_markdown_extension(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidInput("a note needs a name".into()));
    }
    if name.to_ascii_lowercase().ends_with(".md") {
        return Ok(name.to_owned());
    }
    Ok(format!("{name}.md"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use sb_core::{CoreError, NotePath};
    use tempfile::TempDir;

    use super::*;

    fn p(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    fn workspace(files: &[(&str, &str)]) -> (TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let file = dir.path().join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    fn on_disk(dir: &TempDir, name: &str) -> String {
        fs::read_to_string(dir.path().join(name)).unwrap()
    }

    fn set_readonly(file: &Path, readonly: bool) {
        let mut permissions = fs::metadata(file).unwrap().permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(file, permissions).unwrap();
    }

    /// Makes `file` read-only; false (and the file writable again) when this user can write to
    /// it anyway, as root can.
    fn lock_note(file: &Path) -> bool {
        set_readonly(file, true);
        if fs::OpenOptions::new().append(true).open(file).is_err() {
            return true;
        }
        set_readonly(file, false);
        eprintln!("skipped: read-only files are not enforced for this user");
        false
    }

    fn conflict_names(dir: &TempDir) -> Vec<String> {
        let names = fs::read_dir(dir.path()).unwrap();
        let names = names.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned());
        names.filter(|name| name.contains("(conflict ")).collect()
    }

    fn open_dirty(files: &[(&str, &str)], open: &str, typed: &str) -> (TempDir, Workspace) {
        let (dir, mut ws) = workspace(files);
        ws.select(&p(open)).unwrap();
        ws.edit(typed.into(), Instant::now());
        (dir, ws)
    }

    fn note_names(workspace: &Workspace) -> Vec<String> {
        workspace
            .index()
            .notes()
            .map(|e| e.path.to_string())
            .collect()
    }

    #[test]
    fn should_open_vault_and_index_notes() {
        let (_dir, ws) = workspace(&[("a.md", "x"), ("d/b.md", "y")]);
        assert_eq!(ws.index().len(), 2);
        assert_eq!(ws.tree().notes, [p("a.md")]);
        assert_eq!(ws.tree().folders[0].name, "d");
        assert!(ws.open_note().is_none());
        assert_eq!(ws.write_generation(), 0);
    }

    #[test]
    fn should_change_index_revision_on_every_index_change() {
        let (_dir, mut ws) = workspace(&[("a.md", "x")]);
        let start = ws.index_revision();
        ws.create("b", None).unwrap();
        let after_create = ws.index_revision();
        assert_ne!(after_create, start);
        let rebuilt = Index::from_notes(vec![(p("a.md"), "x".into())]);
        let generation = ws.write_generation();
        assert!(matches!(
            ws.apply_index(rebuilt, generation),
            ApplyOutcome::Applied(_)
        ));
        assert_ne!(ws.index_revision(), after_create);
    }

    #[test]
    fn should_save_after_autosave_delay_only() {
        let (dir, mut ws) = workspace(&[("a.md", "old")]);
        ws.select(&p("a.md")).unwrap();
        let before = ws.write_generation();
        let t0 = Instant::now();
        ws.edit("new text".into(), t0);
        assert!(!ws.autosave_due(t0 + Duration::from_millis(499)));
        assert!(ws.autosave_due(t0 + Duration::from_millis(500)));
        assert_eq!(ws.save().unwrap(), Saved::Written);
        assert_eq!(on_disk(&dir, "a.md"), "new text");
        assert!(!ws.open_note().unwrap().is_dirty());
        assert!(!ws.autosave_due(t0 + Duration::from_secs(60)));
        assert_eq!(ws.write_generation(), before + 1);
        assert_eq!(ws.save().unwrap(), Saved::Nothing);
        let indexed = ws.index().get(&p("a.md").key()).unwrap();
        assert_eq!(indexed.content, "new text");
    }

    #[test]
    fn should_become_clean_when_edit_returns_to_saved_text() {
        let (_dir, mut ws) = workspace(&[("a.md", "old")]);
        ws.select(&p("a.md")).unwrap();
        ws.edit("changed".into(), Instant::now());
        assert!(ws.open_note().unwrap().is_dirty());
        ws.edit("old".into(), Instant::now());
        assert!(!ws.open_note().unwrap().is_dirty());
    }

    #[test]
    fn should_keep_buffer_dirty_when_save_fails() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "typed");
        let before = ws.write_generation();
        let file = dir.path().join("a.md");
        set_readonly(&file, true);
        let failed = ws.save();
        set_readonly(&file, false);
        assert!(failed.is_err());
        let open = ws.open_note().unwrap();
        assert_eq!(open.buffer, "typed");
        assert!(open.is_dirty());
        assert_eq!(ws.write_generation(), before);
        assert_eq!(ws.save().unwrap(), Saved::Written);
        assert_eq!(on_disk(&dir, "a.md"), "typed");
    }

    #[test]
    fn should_not_overwrite_external_change_when_saving() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "mine");
        let before = ws.write_generation();
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        assert_eq!(ws.save().unwrap(), Saved::ChangedOnDisk);
        assert_eq!(on_disk(&dir, "a.md"), "theirs");
        let open = ws.open_note().unwrap();
        assert_eq!(open.buffer, "mine");
        assert!(open.is_dirty());
        assert_eq!(ws.write_generation(), before);
    }

    #[test]
    fn should_not_recreate_note_removed_on_disk() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "mine");
        fs::remove_file(dir.path().join("a.md")).unwrap();
        assert_eq!(ws.save().unwrap(), Saved::ChangedOnDisk);
        assert!(!dir.path().join("a.md").exists());
        assert!(ws.open_note().unwrap().is_dirty());
    }

    #[test]
    fn should_pause_autosave_after_failed_save_until_next_edit() {
        let t = Instant::now();
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "typed");
        let file = dir.path().join("a.md");
        set_readonly(&file, true);
        let failed = ws.save();
        set_readonly(&file, false);
        assert!(failed.is_err());
        assert!(!ws.autosave_due(t + Duration::from_secs(10)));
        assert!(ws.open_note().unwrap().is_dirty());
        let t2 = t + Duration::from_secs(11);
        ws.edit("typed more".into(), t2);
        assert!(ws.autosave_due(t2 + AUTOSAVE_DELAY));
    }

    #[test]
    fn should_pause_autosave_when_note_changed_on_disk() {
        let t = Instant::now();
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "mine");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        assert_eq!(ws.save().unwrap(), Saved::ChangedOnDisk);
        assert!(!ws.autosave_due(t + Duration::from_secs(10)));
    }

    #[test]
    fn should_keep_text_in_a_conflict_note_and_switch_when_open_note_changed_on_disk() {
        let files = [("a.md", "old"), ("b.md", "other")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "mine");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        ws.select(&p("b.md")).unwrap();
        assert_eq!(ws.open_note().unwrap().path, p("b.md"));
        assert_eq!(on_disk(&dir, "a.md"), "theirs");
        let notices = ws.take_notices();
        let kept = conflict_names(&dir).pop().unwrap();
        assert_eq!(on_disk(&dir, &kept), "mine");
        let text = format!("Your unsaved edits to a.md were kept in {kept}");
        assert_eq!(notices, [Notice::info(text)]);
        assert!(ws.take_notices().is_empty());
    }

    #[test]
    fn should_keep_text_in_a_conflict_note_and_switch_when_the_save_fails() {
        let files = [("a.md", "old"), ("b.md", "other")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "typed");
        let file = dir.path().join("a.md");
        if !lock_note(&file) {
            return;
        }
        let switched = ws.select(&p("b.md"));
        set_readonly(&file, false);
        switched.unwrap();
        assert_eq!(ws.open_note().unwrap().path, p("b.md"));
        let kept = conflict_names(&dir).pop().unwrap();
        assert_eq!(on_disk(&dir, &kept), "typed");
        let notices = ws.take_notices();
        assert_eq!(notices.len(), 1);
        let text = &notices[0].text;
        assert!(text.starts_with("a.md could not be saved ("), "{text}");
        assert!(
            text.ends_with(&format!("); your text is in {kept}")),
            "{text}"
        );
    }

    #[test]
    fn should_stop_every_leaving_action_when_neither_save_nor_conflict_note_works() {
        let stem = "n".repeat(245);
        let name = format!("{stem}.md");
        let files = [(name.as_str(), "[[b]]"), ("b.md", "")];
        let (dir, mut ws) = open_dirty(&files, &name, "typed");
        let file = dir.path().join(&name);
        if !lock_note(&file) {
            return;
        }
        let from = p(&name);
        let results = [
            ws.select(&p("b.md")),
            ws.create("New", None).map(drop),
            ws.follow_link("b").map(drop),
            ws.follow_link("missing").map(drop),
            ws.rename(&from, "z").map(drop),
            ws.delete(&from),
        ];
        set_readonly(&file, false);
        assert!(results.iter().all(Result::is_err), "{results:?}");
        let message = results[0].as_ref().unwrap_err().to_string();
        assert!(message.contains("conflict note either"), "{message}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        let open = ws.open_note().unwrap();
        let shown = (open.path.as_str(), open.buffer.as_str());
        assert_eq!(shown, (name.as_str(), "typed"));
        assert!(open.is_dirty() && ws.take_notices().is_empty());
    }

    #[test]
    fn should_not_stop_when_the_text_was_already_kept() {
        let files = [("a.md", "old"), ("b.md", "other")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "typed");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        ws.keep_unsaved_as_conflict().unwrap();
        ws.select(&p("b.md")).unwrap();
        assert_eq!(conflict_names(&dir).len(), 1);
        assert!(ws.take_notices().is_empty());
    }

    #[test]
    fn should_keep_open_note_when_rename_target_exists() {
        let files = [("a.md", "old"), ("b.md", "")];
        let (_dir, mut ws) = open_dirty(&files, "a.md", "typed");
        let refused = ws.rename(&p("a.md"), "b");
        assert!(matches!(refused, Err(CoreError::AlreadyExists(_))));
        let open = ws.open_note().unwrap();
        assert_eq!(
            (open.path.as_str(), open.buffer.as_str()),
            ("a.md", "typed")
        );
    }

    #[test]
    fn should_keep_dirty_buffer_when_deleting_another_note() {
        let files = [("a.md", "old"), ("b.md", "")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "typed");
        ws.delete(&p("b.md")).unwrap();
        assert!(!dir.path().join("b.md").exists());
        let open = ws.open_note().unwrap();
        assert_eq!(
            (open.path.as_str(), open.buffer.as_str()),
            ("a.md", "typed")
        );
        assert!(open.is_dirty());
    }

    #[test]
    fn should_save_before_switching_notes() {
        let (dir, mut ws) = workspace(&[("a.md", "a"), ("b.md", "b")]);
        ws.select(&p("a.md")).unwrap();
        ws.edit("a edited".into(), Instant::now());
        ws.select(&p("b.md")).unwrap();
        assert_eq!(on_disk(&dir, "a.md"), "a edited");
        assert_eq!(ws.open_note().unwrap().buffer, "b");
        ws.edit("b edited".into(), Instant::now());
        ws.save().unwrap();
        assert_eq!(on_disk(&dir, "b.md"), "b edited");
    }

    #[test]
    fn should_create_note_in_folder_and_select_it() {
        let (dir, mut ws) = workspace(&[("a.md", "a")]);
        let created = ws.create("  Idea ", Some("projects")).unwrap();
        assert_eq!(created, p("projects/Idea.md"));
        assert_eq!(ws.open_note().unwrap().path, created);
        assert_eq!(ws.open_note().unwrap().buffer, "");
        assert_eq!(on_disk(&dir, "projects/Idea.md"), "");
        assert_eq!(ws.tree().folders[0].notes, [created]);
        assert_eq!(ws.create("Top.MD", None).unwrap(), p("Top.MD"));
    }

    #[test]
    fn should_refuse_create_when_name_taken() {
        let (dir, mut ws) = workspace(&[("Idea.md", "keep me")]);
        let result = ws.create("idea", None);
        assert!(matches!(result, Err(CoreError::AlreadyExists(_))));
        assert_eq!(on_disk(&dir, "Idea.md"), "keep me");
        let blank = ws.create("  ", None);
        assert!(matches!(blank, Err(CoreError::InvalidInput(_))));
    }

    #[test]
    fn should_rename_open_note_and_keep_buffer() {
        let (dir, mut ws) = workspace(&[("a.md", "old")]);
        ws.select(&p("a.md")).unwrap();
        ws.edit("typed".into(), Instant::now());
        let renamed = ws.rename(&p("a.md"), "sub/b").unwrap();
        assert_eq!(renamed, p("sub/b.md"));
        assert_eq!(on_disk(&dir, "sub/b.md"), "typed");
        assert!(!dir.path().join("a.md").exists());
        let open = ws.open_note().unwrap();
        assert_eq!(open.path, renamed);
        assert_eq!(open.buffer, "typed");
        assert!(!open.is_dirty());
        assert_eq!(note_names(&ws), ["sub/b.md"]);
        assert_eq!(ws.tree().folders[0].notes[0], p("sub/b.md"));
    }

    #[test]
    fn should_delete_note_to_trash_and_close_it() {
        let (dir, mut ws) = workspace(&[("a.md", "bye"), ("b.md", "stay")]);
        ws.select(&p("a.md")).unwrap();
        ws.edit("unsaved".into(), Instant::now());
        ws.delete(&p("a.md")).unwrap();
        assert!(!dir.path().join("a.md").exists());
        let trash = dir.path().join(".secondbrain/trash");
        let trashed: Vec<_> = fs::read_dir(trash).unwrap().collect();
        assert_eq!(trashed.len(), 1);
        let kept = fs::read_to_string(trashed[0].as_ref().unwrap().path()).unwrap();
        assert_eq!(kept, "unsaved");
        assert!(ws.open_note().is_none());
        assert_eq!(note_names(&ws), ["b.md"]);
        assert_eq!(ws.tree().notes, [p("b.md")]);
    }

    #[test]
    fn should_trash_the_new_file_and_keep_the_text_when_deleting_a_note_changed_on_disk() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "mine");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        ws.delete(&p("a.md")).unwrap();
        assert!(ws.open_note().is_none());
        let kept = conflict_names(&dir).pop().unwrap();
        assert_eq!(on_disk(&dir, &kept), "mine");
        assert!(!dir.path().join("a.md").exists());
    }

    #[test]
    fn should_follow_resolved_link_to_existing_note() {
        let (_dir, mut ws) = workspace(&[("src.md", "[[Target]]"), ("deep/Target.md", "t")]);
        ws.select(&p("src.md")).unwrap();
        let reached = ws.follow_link("Target").unwrap();
        assert_eq!(reached, p("deep/Target.md"));
        assert_eq!(ws.open_note().unwrap().path, reached);
        assert_eq!(ws.index().len(), 2);
    }

    #[test]
    fn should_create_note_when_following_unresolved_link() {
        let (dir, mut ws) = workspace(&[("a/src.md", "[[./new]]")]);
        ws.select(&p("a/src.md")).unwrap();
        let created = ws.follow_link("./new").unwrap();
        assert_eq!(created, p("a/new.md"));
        assert_eq!(on_disk(&dir, "a/new.md"), "");
        assert_eq!(ws.open_note().unwrap().path, created);
        assert!(ws.index().get(&created.key()).is_some());
    }

    #[test]
    fn should_create_nothing_when_link_is_invalid() {
        let (dir, mut ws) = workspace(&[("a/src.md", "x")]);
        ws.select(&p("a/src.md")).unwrap();
        for target in ["a:b", "./../../x", "CON", "photo.png"] {
            assert!(ws.follow_link(target).is_err(), "{target}");
        }
        assert_eq!(note_names(&ws), ["a/src.md"]);
        assert_eq!(ws.open_note().unwrap().path, p("a/src.md"));
        assert_eq!(fs::read_dir(dir.path().join("a")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn should_rank_stem_prefix_first_in_quick_open() {
        let (_dir, ws) = workspace(&[("Idea.md", ""), ("notes/valid.md", ""), ("other.md", "")]);
        assert_eq!(ws.quick_open("id"), [p("Idea.md"), p("notes/valid.md")]);
        assert!(ws.quick_open("").is_empty());
        assert!(ws.quick_open("zzz").is_empty());
    }

    #[test]
    fn should_limit_quick_open_results() {
        let names: Vec<String> = (0..QUICK_OPEN_LIMIT + 5)
            .map(|n| format!("n{n:03}.md"))
            .collect();
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "")).collect();
        let (_dir, ws) = workspace(&files);
        assert_eq!(ws.quick_open("n").len(), QUICK_OPEN_LIMIT);
    }

    #[test]
    fn should_find_notes_by_content_search() {
        let (_dir, ws) = workspace(&[("a.md", "needle here"), ("b.md", "hay")]);
        let hits = ws.search("needle");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, p("a.md"));
    }

    #[test]
    fn should_count_words_of_open_buffer() {
        let (_dir, mut ws) = workspace(&[("a.md", "one two\nthree")]);
        assert_eq!(ws.word_count(), 0);
        ws.select(&p("a.md")).unwrap();
        assert_eq!(ws.word_count(), 3);
        ws.edit("  a  b ".into(), Instant::now());
        assert_eq!(ws.word_count(), 2);
    }

    #[test]
    fn should_recount_words_when_a_reload_replaces_the_buffer() {
        let (dir, mut ws) = workspace(&[("a.md", "one two")]);
        ws.select(&p("a.md")).unwrap();
        fs::write(dir.path().join("a.md"), "one two three four").unwrap();
        let rebuilt = sb_core::Index::build(ws.vault()).unwrap();
        let tag = ws.write_generation();
        assert!(matches!(
            ws.apply_index(rebuilt, tag),
            ApplyOutcome::Applied(_)
        ));
        assert_eq!(ws.word_count(), 4);
    }

    #[test]
    fn should_change_buffer_generation_only_when_the_buffer_is_replaced_from_outside() {
        let (dir, mut ws) = workspace(&[("a.md", "one"), ("b.md", "two")]);
        let start = ws.buffer_generation();
        ws.select(&p("a.md")).unwrap();
        let selected = ws.buffer_generation();
        assert_ne!(selected, start);
        ws.edit("one!".into(), Instant::now());
        assert_eq!(ws.buffer_generation(), selected);
        fs::write(dir.path().join("a.md"), "remote").unwrap();
        let rebuilt = sb_core::Index::build(ws.vault()).unwrap();
        let tag = ws.write_generation();
        ws.apply_index(rebuilt, tag);
        assert_eq!(ws.open_note().unwrap().buffer, "remote");
        assert_ne!(ws.buffer_generation(), selected);
    }

    #[test]
    fn should_create_and_select_note_with_the_case_as_written_when_ghost_found() {
        let (dir, mut ws) = workspace(&[("a.md", "[[Ghost]]"), ("sub/b.md", "[[./Next]]")]);
        let graph = ws.index().graph();
        let ghosts: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| !n.exists)
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(
            ghosts,
            ["?ghost", "?sub/next"],
            "ids as the graph builds them"
        );
        let created = ws.create_from_ghost("?ghost").unwrap();
        assert_eq!(created, p("Ghost.md"));
        assert_eq!(ws.open_note().unwrap().path, p("Ghost.md"));
        assert!(dir.path().join("Ghost.md").is_file());
        let anchored = ws.create_from_ghost("?sub/next").unwrap();
        assert_eq!(anchored, p("sub/Next.md"));
    }

    #[test]
    fn should_report_not_found_when_no_link_has_the_ghost_id() {
        let (_dir, mut ws) = workspace(&[("a.md", "[[Ghost]] ![[p.png]]")]);
        for id in ["?other", "?p.png", "?p"] {
            let error = ws.create_from_ghost(id).unwrap_err();
            assert!(
                matches!(&error, CoreError::NotFound(found) if found == id),
                "{id}"
            );
        }
        assert!(ws.open_note().is_none());
    }

    #[test]
    fn should_not_replace_existing_note_when_ghost_path_is_taken() {
        let (dir, mut ws) = workspace(&[("a.md", "[[Ghost]]")]);
        fs::write(dir.path().join("Ghost.md"), "precious").unwrap();
        let error = ws.create_from_ghost("?ghost").unwrap_err();
        assert!(matches!(error, CoreError::AlreadyExists(_)));
        assert_eq!(on_disk(&dir, "Ghost.md"), "precious");
    }

    #[test]
    fn should_keep_typed_text_in_a_conflict_note_on_demand() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "typed");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        let kept = ws.keep_unsaved_as_conflict().unwrap().unwrap();
        assert!(kept.as_str().starts_with("a (conflict "), "{kept}");
        assert_eq!(on_disk(&dir, kept.as_str()), "typed");
        assert_eq!(on_disk(&dir, "a.md"), "theirs");
        assert!(ws.index().get(&kept.key()).is_some());
    }

    #[test]
    fn should_keep_the_same_text_once_and_a_later_edit_again() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "typed");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        assert!(ws.keep_unsaved_as_conflict().unwrap().is_some());
        assert_eq!(ws.keep_unsaved_as_conflict().unwrap(), None);
        assert_eq!(ws.save().unwrap(), Saved::ChangedOnDisk);
        assert_eq!(ws.keep_unsaved_as_conflict().unwrap(), None);
        assert_eq!(conflict_count(&dir), 1);
        ws.edit("typed more".into(), Instant::now());
        assert!(ws.keep_unsaved_as_conflict().unwrap().is_some());
        assert_eq!(conflict_count(&dir), 2);
    }

    fn conflict_count(dir: &TempDir) -> usize {
        conflict_names(dir).len()
    }

    #[test]
    fn should_keep_nothing_when_the_note_is_clean_or_closed() {
        let (dir, mut ws) = workspace(&[("a.md", "old")]);
        assert_eq!(ws.keep_unsaved_as_conflict().unwrap(), None);
        ws.select(&p("a.md")).unwrap();
        assert_eq!(ws.keep_unsaved_as_conflict().unwrap(), None);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn should_save_the_open_note_before_creating_from_a_ghost() {
        let (dir, mut ws) = open_dirty(&[("a.md", "[[Ghost]]")], "a.md", "[[Ghost]] typed");
        ws.create_from_ghost("?ghost").unwrap();
        assert_eq!(on_disk(&dir, "a.md"), "[[Ghost]] typed");
    }
}
