//! What the file panel, the links panel, the note dialogs and the status strip show, and what
//! their actions do. No UI here: the UI draws these values and forwards input.

use std::mem;
use std::time::{Duration, Instant};

use ns_core::{Backlink, IgnoreReason, Ignored, NotePath, OutLink, SearchHit};

use super::notice::Notice;
use super::workspace::Workspace;

/// How long the search text must stay unchanged before the query runs.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);
/// The most ignored files the status strip lists; the rest are counted.
pub const MAX_IGNORED_LISTED: usize = 20;

/// What the user asked for in a side panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// Open this note.
    Open(NotePath),
    /// Open the note a link written in the open note points to, creating it if needed.
    Follow(String),
    /// Create and open the note a graph ghost node (this id) stands for.
    CreateFromGhost(String),
    /// Ask for a new name for this note.
    Rename(NotePath),
    /// Move this note to the trash.
    Delete(NotePath),
    /// Ask for the name of a new note in this folder.
    NewIn(String),
    /// Tell the user this, and do nothing else.
    Inform(String),
}

impl NoteAction {
    /// True for the actions that put a note in front of the user: they end the graph view.
    pub fn navigates(&self) -> bool {
        matches!(
            self,
            NoteAction::Open(_) | NoteAction::Follow(_) | NoteAction::CreateFromGhost(_)
        )
    }

    /// Runs the action on `workspace`. Actions that need a name open `dialog`. The returned
    /// notice tells the user what happened or went wrong.
    pub fn run(self, workspace: &mut Workspace, dialog: &mut Option<NoteDialog>) -> Option<Notice> {
        match self {
            NoteAction::Open(path) => failure(workspace.select(&path)),
            NoteAction::Follow(target) => failure(workspace.follow_link(&target)),
            NoteAction::CreateFromGhost(id) => failure(workspace.create_from_ghost(&id)),
            NoteAction::Delete(path) => match workspace.delete(&path) {
                Ok(()) => Some(Notice::info(format!("Moved to trash: {path}"))),
                Err(error) => Some(Notice::error(error.to_string())),
            },
            NoteAction::Rename(path) => open_dialog(dialog, NoteDialog::rename(path)),
            NoteAction::NewIn(folder) => open_dialog(dialog, NoteDialog::new_note(Some(folder))),
            NoteAction::Inform(text) => Some(Notice::info(text)),
        }
    }
}

fn failure<T>(result: ns_core::Result<T>) -> Option<Notice> {
    result.err().map(|error| Notice::error(error.to_string()))
}

fn open_dialog(slot: &mut Option<NoteDialog>, dialog: NoteDialog) -> Option<Notice> {
    *slot = Some(dialog);
    None
}

/// The search field and its debounced results.
#[derive(Debug, Default)]
pub struct SearchBox {
    /// What the user has typed.
    pub text: String,
    seen: String,
    changed_at: Option<Instant>,
    searched: Option<(String, u64)>,
    hits: Vec<SearchHit>,
}

impl SearchBox {
    /// Runs the query when the text has been unchanged for `SEARCH_DEBOUNCE`, and again when the
    /// index changed under the same text. Returns how long to wait before calling again, if the
    /// query is still pending.
    pub fn update(&mut self, workspace: &Workspace, now: Instant) -> Option<Duration> {
        if self.text.is_empty() {
            *self = SearchBox::default();
            return None;
        }
        if self.text != self.seen {
            self.seen.clone_from(&self.text);
            self.changed_at = Some(now);
        }
        let revision = workspace.cross_note_revision();
        if self.is_settled(revision) {
            return None;
        }
        let waited = now.saturating_duration_since(self.changed_at.unwrap_or(now));
        if waited < SEARCH_DEBOUNCE {
            return Some(SEARCH_DEBOUNCE - waited);
        }
        self.hits = workspace.search(&self.text);
        self.searched = Some((self.text.clone(), revision));
        None
    }

    fn is_settled(&self, revision: u64) -> bool {
        let done = self.searched.as_ref();
        done.is_some_and(|(query, at)| *query == self.text && *at == revision)
    }

    /// The hits of the last query that ran.
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    /// True once the hits answer the text as it is now.
    pub fn has_answer(&self) -> bool {
        let done = self.searched.as_ref();
        done.is_some_and(|(query, _)| *query == self.text)
    }
}

/// Everything the links panel lists for the open note.
#[derive(Debug, Default)]
pub struct InspectorData {
    /// Lines of other notes that link here.
    pub backlinks: Vec<Backlink>,
    /// Links written in the open note, with what they resolve to.
    pub outgoing: Vec<OutLink>,
    /// Tags of the open note, lowercase, without `#`.
    pub tags: Vec<String>,
}

/// `InspectorData`: the backlinks are recomputed only when the open note changes or another note
/// may have changed (they cost a scan of every note); the open note's own links and tags are
/// refreshed after each save of it.
#[derive(Debug, Default)]
pub struct InspectorCache {
    key: Option<InspectorKey>,
    data: InspectorData,
    computed: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct InspectorKey {
    path: NotePath,
    cross_note: u64,
    revision: u64,
}

impl InspectorCache {
    /// The lists for the open note of `workspace`; empty when no note is open.
    pub fn data(&mut self, workspace: &Workspace) -> &InspectorData {
        let Some(open) = workspace.open_note() else {
            self.key = None;
            self.data = InspectorData::default();
            return &self.data;
        };
        let key = InspectorKey {
            path: open.path.clone(),
            cross_note: workspace.cross_note_revision(),
            revision: workspace.index_revision(),
        };
        match self.key.as_ref() {
            Some(seen) if *seen == key => {}
            Some(seen) if seen.path == key.path && seen.cross_note == key.cross_note => {
                let (outgoing, tags) = own_links(workspace, &key.path);
                let backlinks = mem::take(&mut self.data.backlinks);
                self.data = InspectorData {
                    backlinks,
                    outgoing,
                    tags,
                };
            }
            _ => {
                self.data = compute(workspace, &key.path);
                self.computed += 1;
            }
        }
        self.key = Some(key);
        &self.data
    }

    /// How many times the backlinks were computed; stays put while the open note and the other
    /// notes do.
    pub fn computations(&self) -> usize {
        self.computed
    }
}

fn compute(workspace: &Workspace, path: &NotePath) -> InspectorData {
    let (outgoing, tags) = own_links(workspace, path);
    InspectorData {
        backlinks: workspace.index().backlinks(&path.key()),
        outgoing,
        tags,
    }
}

fn own_links(workspace: &Workspace, path: &NotePath) -> (Vec<OutLink>, Vec<String>) {
    let (index, key) = (workspace.index(), path.key());
    let tags = index.get(&key).map(|entry| entry.tags.clone());
    (index.outgoing(&key), tags.unwrap_or_default())
}

/// What a note dialog does when confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    /// Create a note in this folder (the vault root when `None`).
    NewNote(Option<String>),
    /// Move this note to the typed path.
    Rename(NotePath),
}

/// The «New note» or «Rename note» dialog.
#[derive(Clone, Debug)]
pub struct NoteDialog {
    /// What confirming does.
    pub kind: DialogKind,
    /// The text of the name field.
    pub name: String,
    /// Why the last attempt failed.
    pub error: Option<String>,
    /// Set once the name field has taken the keyboard focus.
    pub focused: bool,
}

impl NoteDialog {
    /// A dialog that creates a note in `folder` (the vault root when `None`).
    pub fn new_note(folder: Option<String>) -> NoteDialog {
        NoteDialog::with(DialogKind::NewNote(folder), String::new())
    }

    /// A dialog that renames `from`, prefilled with its current path.
    pub fn rename(from: NotePath) -> NoteDialog {
        let name = from.to_string();
        NoteDialog::with(DialogKind::Rename(from), name)
    }

    fn with(kind: DialogKind, name: String) -> NoteDialog {
        NoteDialog {
            kind,
            name,
            error: None,
            focused: false,
        }
    }

    /// The dialog's heading.
    pub fn title(&self) -> &'static str {
        match self.kind {
            DialogKind::NewNote(_) => "New note",
            DialogKind::Rename(_) => "Rename note",
        }
    }

    /// The hint and accessible name of the name field.
    pub fn field_name(&self) -> &'static str {
        match self.kind {
            DialogKind::NewNote(_) => "Note name",
            DialogKind::Rename(_) => "New name",
        }
    }

    /// The text of the confirm button.
    pub fn confirm_label(&self) -> &'static str {
        match self.kind {
            DialogKind::NewNote(_) => "Create",
            DialogKind::Rename(_) => "Rename",
        }
    }

    /// Does what the dialog is for. True when it succeeded and the dialog should close; on
    /// failure the reason is kept in `error` and nothing changes on disk.
    pub fn confirm(&mut self, workspace: &mut Workspace) -> bool {
        let done = match &self.kind {
            DialogKind::NewNote(folder) => workspace.create(&self.name, folder.as_deref()),
            DialogKind::Rename(from) => workspace.rename(from, &self.name),
        };
        self.error = done.as_ref().err().map(ToString::to_string);
        // Enter took the focus off the field: take it back so the name can be retyped at once.
        self.focused = false;
        done.is_ok()
    }
}

/// `n` followed by `one` when `n` is 1 and by `many` otherwise.
pub fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One line per ignored file, `path: reason`, at most `MAX_IGNORED_LISTED`, then «and k more».
pub fn ignored_lines(ignored: &[Ignored]) -> Vec<String> {
    let mut lines: Vec<String> = ignored
        .iter()
        .take(MAX_IGNORED_LISTED)
        .map(|file| format!("{}: {}", file.path, ignore_reason_text(file.reason)))
        .collect();
    if ignored.len() > MAX_IGNORED_LISTED {
        lines.push(format!("and {} more", ignored.len() - MAX_IGNORED_LISTED));
    }
    lines
}

/// Why a file was left out of the index, in words.
pub fn ignore_reason_text(reason: IgnoreReason) -> &'static str {
    match reason {
        IgnoreReason::InvalidPath => "name not allowed",
        IgnoreReason::CaseCollision => "same name as another note apart from letter case",
        IgnoreReason::NotUtf8 => "not UTF-8 text",
        IgnoreReason::TooLarge => "larger than 4 MiB",
        IgnoreReason::Unreadable => "cannot be read",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn p(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    fn workspace(files: &[(&str, &str)]) -> (TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            fs::write(dir.path().join(name), text).unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    fn searching(text: &str) -> SearchBox {
        SearchBox {
            text: text.into(),
            ..SearchBox::default()
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn should_run_search_only_after_text_is_unchanged_for_debounce() {
        let (_dir, ws) = workspace(&[("a.md", "needle here")]);
        let t0 = Instant::now();
        let mut search = searching("needle");
        assert_eq!(search.update(&ws, t0), Some(SEARCH_DEBOUNCE));
        assert_eq!(search.update(&ws, t0 + ms(100)), Some(ms(50)));
        assert!(search.hits().is_empty() && !search.has_answer());
        search.text = "needle h".into();
        assert_eq!(search.update(&ws, t0 + ms(120)), Some(SEARCH_DEBOUNCE));
        assert_eq!(search.update(&ws, t0 + ms(270)), None);
        assert_eq!(search.hits()[0].path, p("a.md"));
        assert!(search.has_answer());
    }

    #[test]
    fn should_clear_results_at_once_when_text_is_emptied() {
        let (_dir, ws) = workspace(&[("a.md", "needle")]);
        let t0 = Instant::now();
        let mut search = searching("needle");
        search.update(&ws, t0);
        search.update(&ws, t0 + ms(150));
        assert_eq!(search.hits().len(), 1);
        search.text.clear();
        assert_eq!(search.update(&ws, t0 + ms(151)), None);
        assert!(search.hits().is_empty());
    }

    #[test]
    fn should_search_again_when_index_changed_under_same_text() {
        let (_dir, mut ws) = workspace(&[("a.md", "needle")]);
        let t0 = Instant::now();
        let mut search = searching("needle");
        search.update(&ws, t0);
        search.update(&ws, t0 + ms(150));
        ws.rename(&p("a.md"), "z").unwrap();
        assert_eq!(search.update(&ws, t0 + ms(160)), None);
        assert_eq!(search.hits()[0].path, p("z.md"));
    }

    #[test]
    fn should_list_links_of_open_note_and_recompute_only_on_change() {
        let (_dir, mut ws) = workspace(&[
            ("a.md", "to [[b]] and [[ghost]] #tag"),
            ("b.md", "back to [[a]]"),
        ]);
        let mut cache = InspectorCache::default();
        assert!(cache.data(&ws).backlinks.is_empty());
        ws.select(&p("a.md")).unwrap();
        let data = cache.data(&ws);
        assert_eq!(data.tags, ["tag"]);
        assert_eq!(data.outgoing.len(), 2);
        assert_eq!(data.outgoing[0].resolved, Some(p("b.md")));
        assert_eq!(data.outgoing[1].resolved, None);
        assert_eq!(data.backlinks[0].from, p("b.md"));
        let computed = cache.computations();
        cache.data(&ws);
        assert_eq!(cache.computations(), computed);
        ws.rename(&p("b.md"), "c").unwrap();
        let data = cache.data(&ws);
        assert_eq!(data.backlinks[0].from, p("c.md"));
        assert_eq!(cache.computations(), computed + 1);
    }

    #[test]
    fn should_not_recompute_backlinks_when_the_open_note_is_saved() {
        let (_dir, mut ws) = workspace(&[("a.md", "[[b]] #old"), ("b.md", "back to [[a]]")]);
        let mut cache = InspectorCache::default();
        ws.select(&p("a.md")).unwrap();
        cache.data(&ws);
        let computed = cache.computations();
        ws.edit("[[b]] [[ghost]] #new".into(), Instant::now());
        ws.save().unwrap();
        let data = cache.data(&ws);
        assert_eq!(data.tags, ["new"]);
        assert_eq!(data.outgoing.len(), 2);
        assert_eq!(data.backlinks[0].from, p("b.md"));
        assert_eq!(cache.computations(), computed);
    }

    #[test]
    fn should_search_again_only_when_another_note_may_have_changed() {
        let (_dir, mut ws) = workspace(&[("a.md", "needle")]);
        let t0 = Instant::now();
        let mut search = searching("needle");
        ws.select(&p("a.md")).unwrap();
        search.update(&ws, t0);
        search.update(&ws, t0 + ms(150));
        ws.edit("other".into(), t0);
        ws.save().unwrap();
        search.update(&ws, t0 + ms(160));
        assert_eq!(
            search.hits().len(),
            1,
            "an own save must not rerun the search"
        );
        ws.create("z", None).unwrap();
        search.update(&ws, t0 + ms(170));
        assert!(search.hits().is_empty());
    }

    #[test]
    fn should_report_name_taken_in_dialog_without_closing() {
        let (dir, mut ws) = workspace(&[("a.md", "keep")]);
        let mut dialog = NoteDialog::new_note(None);
        dialog.name = "a".into();
        dialog.focused = true;
        assert!(!dialog.confirm(&mut ws));
        assert!(dialog.error.as_deref().unwrap().contains("already exists"));
        assert!(!dialog.focused, "the field must take the focus again");
        assert_eq!(fs::read_to_string(dir.path().join("a.md")).unwrap(), "keep");
        dialog.name = "b".into();
        assert!(dialog.confirm(&mut ws));
        assert!(dialog.error.is_none());
        assert_eq!(ws.open_note().unwrap().path, p("b.md"));
    }

    #[test]
    fn should_prefill_rename_dialog_with_current_path() {
        let dialog = NoteDialog::rename(p("sub/x.md"));
        assert_eq!(dialog.name, "sub/x.md");
        assert_eq!(dialog.field_name(), "New name");
        assert_eq!(NoteDialog::new_note(None).field_name(), "Note name");
    }

    #[test]
    fn should_trash_note_and_say_so_when_delete_action_runs() {
        let (dir, mut ws) = workspace(&[("a.md", "x")]);
        let mut dialog = None;
        let notice = NoteAction::Delete(p("a.md")).run(&mut ws, &mut dialog);
        assert_eq!(notice, Some(Notice::info("Moved to trash: a.md")));
        assert!(!dir.path().join("a.md").exists());
        let notice = NoteAction::Open(p("a.md")).run(&mut ws, &mut dialog);
        assert!(notice.is_some());
        assert!(dialog.is_none());
    }

    #[test]
    fn should_only_report_when_an_inform_action_runs() {
        let (_dir, mut ws) = workspace(&[("a.md", "x")]);
        let mut dialog = None;
        let action = NoteAction::Inform("hello".into());
        assert!(!action.navigates());
        assert_eq!(
            action.run(&mut ws, &mut dialog),
            Some(Notice::info("hello"))
        );
    }

    #[test]
    fn should_open_dialogs_for_actions_that_need_a_name() {
        let (_dir, mut ws) = workspace(&[("a.md", "x")]);
        let mut dialog = None;
        assert!(
            NoteAction::NewIn("sub".into())
                .run(&mut ws, &mut dialog)
                .is_none()
        );
        assert_eq!(
            dialog.as_ref().unwrap().kind,
            DialogKind::NewNote(Some("sub".into()))
        );
        NoteAction::Rename(p("a.md")).run(&mut ws, &mut dialog);
        assert_eq!(dialog.unwrap().kind, DialogKind::Rename(p("a.md")));
    }

    #[test]
    fn should_cap_ignored_list_and_count_the_rest() {
        let files: Vec<Ignored> = (0..23)
            .map(|n| Ignored {
                path: format!("f{n}.md"),
                reason: IgnoreReason::NotUtf8,
            })
            .collect();
        let lines = ignored_lines(&files);
        assert_eq!(lines.len(), MAX_IGNORED_LISTED + 1);
        assert_eq!(lines[0], "f0.md: not UTF-8 text");
        assert_eq!(lines[MAX_IGNORED_LISTED], "and 3 more");
        assert_eq!(ignored_lines(&files[..2]).len(), 2);
    }

    #[test]
    fn should_pick_singular_only_for_one() {
        assert_eq!(counted(1, "word", "words"), "1 word");
        assert_eq!(counted(0, "word", "words"), "0 words");
        assert_eq!(
            counted(2, "file ignored", "files ignored"),
            "2 files ignored"
        );
    }
}
