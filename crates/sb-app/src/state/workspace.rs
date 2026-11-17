//! The open vault and open note, with every operation the window needs and no UI in sight.

use std::path::Path;
use std::time::{Duration, Instant};

use sb_core::{
    CoreError, DEFAULT_SEARCH_LIMIT, Index, NotePath, Result, SearchHit, Vault, new_note_path, ops,
};

use super::tree::{Folder, build_tree};

/// How long after the last keystroke the open note is saved.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);
/// The most notes `quick_open` returns.
pub const QUICK_OPEN_LIMIT: usize = 50;

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
    save_generation: u64,
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
            save_generation: 0,
        })
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

    /// Counts successful saves, so the window can tell its own writes from foreign ones.
    pub fn save_generation(&self) -> u64 {
        self.save_generation
    }

    /// Opens a note, saving a dirty open one first; a failed save keeps the current note open.
    ///
    /// # Errors
    /// The save error, `InvalidInput` when the open note changed on disk, or the read error for
    /// `path`.
    pub fn select(&mut self, path: &NotePath) -> Result<()> {
        self.save_or_stop()?;
        let text = ops::read_note(&self.vault, path.as_str())?;
        self.open = Some(OpenNote {
            path: path.clone(),
            buffer: text.clone(),
            saved: text,
            last_edit: None,
            autosave_paused: false,
        });
        Ok(())
    }

    /// Saves a dirty open note, then closes it; a failed save keeps it open.
    ///
    /// # Errors
    /// The save error, or `InvalidInput` when the open note changed on disk.
    pub fn close(&mut self) -> Result<()> {
        self.save_or_stop()?;
        self.open = None;
        Ok(())
    }

    /// Replaces the buffer of the open note; does nothing when no note is open.
    pub fn edit(&mut self, text: String, now: Instant) {
        let Some(open) = self.open.as_mut().filter(|open| open.buffer != text) else {
            return;
        };
        open.last_edit = (text != open.saved).then_some(now);
        open.autosave_paused = false;
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
        self.save_generation += 1;
        Ok(Saved::Written)
    }

    /// Saves, and turns `ChangedOnDisk` into an error so the caller stops what it was doing.
    fn save_or_stop(&mut self) -> Result<()> {
        if self.save()? != Saved::ChangedOnDisk {
            return Ok(());
        }
        let path = self.open.as_ref().map(|open| open.path.to_string());
        Err(CoreError::InvalidInput(format!(
            "{} changed on disk; it will reload in a moment",
            path.unwrap_or_default()
        )))
    }

    /// Creates an empty note called `name` (`.md` added when missing) in `folder` or the vault
    /// root, and opens it.
    ///
    /// # Errors
    /// `InvalidInput` for a blank name, `AlreadyExists`, `InvalidPath`, or a save or write error.
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
        self.save_or_stop()?;
        let created = ops::create_note(&self.vault, path, "")?.path;
        self.index.upsert(created.clone(), String::new());
        self.refresh_tree();
        self.select(&created)?;
        Ok(created)
    }

    /// Moves `from` to `to` (`.md` added when missing). An open note keeps its buffer and follows.
    ///
    /// # Errors
    /// The save error, `InvalidInput` for a blank name, or what `ops::rename_note` returns.
    pub fn rename(&mut self, from: &NotePath, to: &str) -> Result<NotePath> {
        let to = with_markdown_extension(to)?;
        if self.is_open(from) {
            self.save_or_stop()?;
        }
        let outcome = ops::rename_note(&self.vault, from.as_str(), &to)?;
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

    /// Moves a note to the vault's trash. An open copy is closed without saving.
    ///
    /// # Errors
    /// Whatever `ops::delete_note` returns; then nothing changes.
    pub fn delete(&mut self, path: &NotePath) -> Result<()> {
        let outcome = ops::delete_note(&self.vault, path.as_str())?;
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
        self.open
            .as_ref()
            .map_or(0, |open| open.buffer.split_whitespace().count())
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
        assert_eq!(ws.save_generation(), 0);
    }

    #[test]
    fn should_save_after_autosave_delay_only() {
        let (dir, mut ws) = workspace(&[("a.md", "old")]);
        ws.select(&p("a.md")).unwrap();
        let t0 = Instant::now();
        ws.edit("new text".into(), t0);
        assert!(!ws.autosave_due(t0 + Duration::from_millis(499)));
        assert!(ws.autosave_due(t0 + Duration::from_millis(500)));
        assert_eq!(ws.save().unwrap(), Saved::Written);
        assert_eq!(on_disk(&dir, "a.md"), "new text");
        assert!(!ws.open_note().unwrap().is_dirty());
        assert!(!ws.autosave_due(t0 + Duration::from_secs(60)));
        assert_eq!(ws.save_generation(), 1);
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
        let file = dir.path().join("a.md");
        set_readonly(&file, true);
        let failed = ws.save();
        let switched = ws.select(&p("a.md"));
        let closed = ws.close();
        set_readonly(&file, false);
        assert!(failed.is_err() && switched.is_err() && closed.is_err());
        let open = ws.open_note().unwrap();
        assert_eq!(open.buffer, "typed");
        assert!(open.is_dirty());
        assert_eq!(ws.save_generation(), 0);
        assert_eq!(ws.save().unwrap(), Saved::Written);
        assert_eq!(on_disk(&dir, "a.md"), "typed");
    }

    #[test]
    fn should_not_overwrite_external_change_when_saving() {
        let (dir, mut ws) = open_dirty(&[("a.md", "old")], "a.md", "mine");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        assert_eq!(ws.save().unwrap(), Saved::ChangedOnDisk);
        assert_eq!(on_disk(&dir, "a.md"), "theirs");
        let open = ws.open_note().unwrap();
        assert_eq!(open.buffer, "mine");
        assert!(open.is_dirty());
        assert_eq!(ws.save_generation(), 0);
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
    fn should_refuse_switch_when_open_note_changed_on_disk() {
        let files = [("a.md", "old"), ("b.md", "other")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "mine");
        fs::write(dir.path().join("a.md"), "theirs").unwrap();
        let refused = ws.select(&p("b.md"));
        assert!(matches!(refused, Err(CoreError::InvalidInput(_))));
        let open = ws.open_note().unwrap();
        assert_eq!((open.path.as_str(), open.buffer.as_str()), ("a.md", "mine"));
        assert!(open.is_dirty());
        assert!(matches!(ws.close(), Err(CoreError::InvalidInput(_))));
        assert!(ws.open_note().is_some());
    }

    #[test]
    fn should_stop_create_follow_and_rename_when_save_fails() {
        let files = [("a.md", "[[b]]"), ("b.md", "")];
        let (dir, mut ws) = open_dirty(&files, "a.md", "typed");
        let file = dir.path().join("a.md");
        set_readonly(&file, true);
        let results = [
            ws.create("New", None).map(drop),
            ws.follow_link("b").map(drop),
            ws.follow_link("missing").map(drop),
            ws.rename(&p("a.md"), "z").map(drop),
        ];
        set_readonly(&file, false);
        assert!(results.iter().all(Result::is_err), "{results:?}");
        assert_eq!(note_names(&ws), ["a.md", "b.md"]);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        let open = ws.open_note().unwrap();
        assert_eq!(
            (open.path.as_str(), open.buffer.as_str()),
            ("a.md", "typed")
        );
        assert!(open.is_dirty());
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
        ws.close().unwrap();
        assert_eq!(on_disk(&dir, "b.md"), "b edited");
        assert!(ws.open_note().is_none());
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
        assert_eq!(fs::read_dir(trash).unwrap().count(), 1);
        assert!(ws.open_note().is_none());
        assert_eq!(note_names(&ws), ["b.md"]);
        assert_eq!(ws.tree().notes, [p("b.md")]);
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
}
