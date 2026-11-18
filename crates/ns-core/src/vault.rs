use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{CoreError, Result};
use crate::path::NotePath;
use crate::time::utc_compact;

/// Largest note the vault reads or writes, measured in bytes of content.
pub const MAX_NOTE_BYTES: usize = 4 * 1024 * 1024;

const TRASH_RELATIVE: &str = ".neurospace/trash";
const MARKDOWN_EXTENSION: &str = "md";
const MAX_FILE_NAME_BYTES: usize = 255;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Why a file in the vault is not listed as a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// The relative path breaks a note path rule.
    InvalidPath,
    /// Another file with the same note key sorts first.
    CaseCollision,
    /// The bytes are not valid UTF-8.
    NotUtf8,
    /// The file is larger than `MAX_NOTE_BYTES`.
    TooLarge,
    /// The file could not be read (locked, no permission), or the folder could not be listed.
    Unreadable,
}

/// A `.md` file the scan did not turn into a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// Vault-relative path, `/`-separated (lossy for names that are not Unicode). A trailing
    /// `/` marks a folder whose notes could not be listed.
    pub path: String,
    /// Why it was left out.
    pub reason: IgnoreReason,
}

/// Result of scanning a vault.
#[derive(Debug)]
pub struct Loaded {
    /// Every valid note with its content, ordered by note key.
    pub notes: Vec<(NotePath, String)>,
    /// Files that were skipped, ordered by path.
    pub ignored: Vec<Ignored>,
}

/// A folder of Markdown notes and the operations that touch it on disk.
#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    /// Opens an existing directory as a vault.
    ///
    /// # Errors
    /// `NotADirectory` when `root` is missing or is not a directory.
    pub fn open(root: impl Into<PathBuf>) -> Result<Vault> {
        let root = root.into();
        if !root.is_dir() {
            return Err(CoreError::NotADirectory(root));
        }
        Ok(Vault { root })
    }

    /// The vault's root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where deleted notes go: `<root>/.neurospace/trash`.
    pub fn trash_dir(&self) -> PathBuf {
        self.root.join(TRASH_RELATIVE)
    }

    /// Scans the vault (spec 5.3) and reads every note.
    ///
    /// # Errors
    /// `Io` when the vault root cannot be listed. An unreadable note or sub-folder is
    /// reported as ignored.
    pub fn load(&self) -> Result<Loaded> {
        let mut scan = Scan::default();
        scan.walk(&self.root, "", true)
            .map_err(io_error(&self.root))?;
        Ok(scan.read_notes())
    }

    /// The on-disk spelling of `path`, matched case-insensitively, if present.
    ///
    /// # Errors
    /// `SymlinkInPath` when a symbolic link stands in for a directory on the way.
    pub fn locate(&self, path: &NotePath) -> Result<Option<NotePath>> {
        let depth = path.components().count();
        let mut dir = self.root.clone();
        let mut spelled = Vec::with_capacity(depth);
        for (index, component) in path.components().enumerate() {
            let Some((name, kind)) = find_entry(&dir, component)? else {
                return Ok(None);
            };
            if index + 1 < depth {
                if kind.is_symlink() {
                    return Err(CoreError::SymlinkInPath(path.to_string()));
                }
                if !kind.is_dir() {
                    return Ok(None);
                }
            }
            dir.push(&name);
            spelled.push(name);
        }
        let joined = spelled.join("/");
        NotePath::parse(&joined)
            .map(Some)
            .map_err(|source| CoreError::InvalidPath {
                path: joined,
                source,
            })
    }

    /// Rewrites each folder of `path` to the spelling of an existing folder that matches it
    /// ignoring case, so creating a note never makes a second folder differing only in case.
    /// Folders that do not exist yet, and the file name, keep the given spelling.
    ///
    /// # Errors
    /// `SymlinkInPath` when a symbolic link stands in for a folder, `NotADirectory` when a
    /// file does.
    pub fn adopt_folders(&self, path: &NotePath) -> Result<NotePath> {
        let mut dir = self.root.clone();
        let mut spelled: Vec<String> = Vec::new();
        let mut folders = path.folder().split('/').filter(|folder| !folder.is_empty());
        for folder in folders.by_ref() {
            let Some((name, kind)) = find_entry(&dir, folder)? else {
                spelled.push(folder.to_owned());
                break;
            };
            if kind.is_symlink() {
                return Err(CoreError::SymlinkInPath(path.to_string()));
            }
            dir.push(&name);
            if !kind.is_dir() {
                return Err(CoreError::NotADirectory(dir));
            }
            spelled.push(name);
        }
        spelled.extend(folders.map(str::to_owned));
        spelled.extend(path.components().last().map(str::to_owned));
        let joined = spelled.join("/");
        NotePath::parse(&joined).map_err(|source| CoreError::InvalidPath {
            path: joined,
            source,
        })
    }

    /// Reads the note stored under exactly this spelling.
    ///
    /// # Errors
    /// `NotFound` when no regular file is there, `NotUtf8` when its bytes are not text,
    /// `InvalidInput` when the file is over `MAX_NOTE_BYTES` (it is not read).
    pub fn read(&self, path: &NotePath) -> Result<String> {
        let full = self.require_file(path)?;
        let size = fs::metadata(&full).map_err(io_error(&full))?.len();
        if size > MAX_NOTE_BYTES as u64 {
            return Err(CoreError::InvalidInput(format!(
                "{path} is larger than 4 MiB and is not treated as a note"
            )));
        }
        let bytes = fs::read(&full).map_err(io_error(&full))?;
        String::from_utf8(bytes).map_err(|_| CoreError::NotUtf8(path.to_string()))
    }

    /// Creates or atomically replaces a note; true when it did not exist before. The caller
    /// must first resolve the spelling of `path` with `locate` (existing note) or
    /// `adopt_folders` (new note): this method uses the path as given.
    ///
    /// # Errors
    /// `InvalidInput` over `MAX_NOTE_BYTES`, `SymlinkInPath` when the note itself or a
    /// folder on the way is a link, `Io` on file system trouble.
    pub fn write(&self, path: &NotePath, content: &str) -> Result<bool> {
        check_size(content.len())?;
        let target = self.prepare_target(path)?;
        let existing = lstat(&target)?;
        if existing.as_ref().is_some_and(fs::Metadata::is_symlink) {
            return Err(CoreError::SymlinkInPath(path.to_string()));
        }
        write_atomically(&target, content.as_bytes())?;
        Ok(existing.is_none())
    }

    /// Replaces an existing note only if `still_expected` accepts the bytes on disk at the moment
    /// of the swap. The new content is fully written to a temporary file before the check, so the
    /// window between check and rename is a few system calls. Returns whether it replaced.
    /// The caller must first resolve the spelling of `path` with `locate`.
    ///
    /// # Errors
    /// `NotFound` when no regular file is there, `InvalidInput` over `MAX_NOTE_BYTES`,
    /// `SymlinkInPath`, or `Io` (a read-only note is refused with `PermissionDenied`).
    pub fn replace_if(
        &self,
        path: &NotePath,
        content: &str,
        still_expected: &dyn Fn(&[u8]) -> bool,
    ) -> Result<bool> {
        check_size(content.len())?;
        let target = self.require_file(path)?;
        refuse_read_only(&target)?;
        let temp = temp_sibling(&target);
        write_new_file(&temp, content.as_bytes()).map_err(io_error(&temp))?;
        let swapped = swap_if(&temp, &target, path, still_expected);
        if !matches!(swapped, Ok(true)) {
            let _ = fs::remove_file(&temp);
        }
        swapped
    }

    /// Creates a note and never replaces anything. The caller must first resolve the spelling
    /// of `path` with `adopt_folders`: this method uses the path as given.
    ///
    /// # Errors
    /// `AlreadyExists` when anything occupies the location, plus the errors of `write`.
    pub fn create(&self, path: &NotePath, content: &str) -> Result<()> {
        check_size(content.len())?;
        let target = self.prepare_target(path)?;
        // Windows reports a directory in the way as access denied, not as "exists".
        if lstat(&target)?.is_some() {
            return Err(CoreError::AlreadyExists(path.to_string()));
        }
        create_atomically(&target, content.as_bytes()).map_err(|source| match source.kind() {
            io::ErrorKind::AlreadyExists => CoreError::AlreadyExists(path.to_string()),
            _ => CoreError::Io {
                path: target.clone(),
                source,
            },
        })
    }

    /// Appends `text`, inserting one `\n` first when the note does not end with one.
    /// Creates the note if missing; true when it was created.
    ///
    /// # Errors
    /// The errors of `read`, `write` and `create`.
    pub fn append(&self, path: &NotePath, text: &str) -> Result<bool> {
        match self.read(path) {
            Ok(existing) => {
                self.write(path, &join_with_separator(existing, text))?;
                Ok(false)
            }
            Err(CoreError::NotFound(_)) => self.create(path, text).map(|()| true),
            Err(other) => Err(other),
        }
    }

    /// Moves a note into the trash; nothing is ever unlinked.
    ///
    /// # Errors
    /// `NotFound` when there is no such note, `SymlinkInPath` or `Io` otherwise.
    pub fn delete(&self, path: &NotePath) -> Result<()> {
        self.delete_with_stamp(path, &utc_compact(SystemTime::now()))
    }

    fn delete_with_stamp(&self, path: &NotePath, stamp: &str) -> Result<()> {
        let source = self.require_file(path)?;
        self.confine(TRASH_RELATIVE.split('/'), TRASH_RELATIVE)?;
        let trash = self.trash_dir();
        fs::create_dir_all(&trash).map_err(io_error(&trash))?;
        let destination = claim_trash_path(&trash, path, stamp)?;
        fs::rename(&source, &destination).map_err(|error| {
            let _ = fs::remove_file(&destination);
            io_error(&source)(error)
        })
    }

    /// Moves a note, creating the destination's folders.
    ///
    /// # Errors
    /// `NotFound` for a missing source, `AlreadyExists` when the destination key is taken
    /// by anything but the source itself (a case-only rename of the source is allowed).
    pub fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()> {
        let source = self.require_file(from)?;
        if self.locate(to)?.is_some_and(|found| found != *from) {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        let destination = self.prepare_target(to)?;
        // `fs::rename` replaces its target; `locate` misses aliases such as Windows 8.3 names.
        if to.key() != from.key() && lstat(&destination)?.is_some() {
            return Err(CoreError::AlreadyExists(to.to_string()));
        }
        fs::rename(&source, &destination).map_err(io_error(&destination))
    }

    fn full_path(&self, path: &NotePath) -> PathBuf {
        path.components()
            .fold(self.root.clone(), |dir, component| dir.join(component))
    }

    /// Confines the path, then returns it only if a regular file (not a link) is there.
    fn require_file(&self, path: &NotePath) -> Result<PathBuf> {
        self.confine_note(path)?;
        let full = self.full_path(path);
        match lstat(&full)? {
            Some(meta) if meta.is_file() => Ok(full),
            _ => Err(CoreError::NotFound(path.to_string())),
        }
    }

    fn confine_note(&self, path: &NotePath) -> Result<()> {
        self.confine(path.folder().split('/'), path.as_str())
    }

    /// Spec 5.4 confinement: every existing directory on the way must be a real directory.
    fn confine<'a>(&self, dirs: impl Iterator<Item = &'a str>, subject: &str) -> Result<()> {
        let mut current = self.root.clone();
        for dir in dirs.filter(|dir| !dir.is_empty()) {
            current.push(dir);
            let Some(meta) = lstat(&current)? else {
                return Ok(());
            };
            if meta.file_type().is_symlink() {
                return Err(CoreError::SymlinkInPath(subject.to_owned()));
            }
            if !meta.is_dir() {
                return Err(CoreError::NotADirectory(current));
            }
        }
        Ok(())
    }

    fn prepare_target(&self, path: &NotePath) -> Result<PathBuf> {
        self.confine_note(path)?;
        let full = self.full_path(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(io_error(parent))?;
        }
        Ok(full)
    }
}

/// A `.md` file found by the walk, with the metadata of its directory entry when available.
pub(crate) struct Candidate {
    pub(crate) path: NotePath,
    pub(crate) full: PathBuf,
    pub(crate) meta: Option<fs::Metadata>,
}

#[derive(Default)]
pub(crate) struct Scan {
    files: Vec<Candidate>,
    pub(crate) ignored: Vec<Ignored>,
}

impl Scan {
    /// `prefix` is the relative path of `dir` with a trailing `/`; `unicode` is false
    /// when any name on the way was not valid Unicode. Fails when `dir` or one of its
    /// entries cannot be listed.
    pub(crate) fn walk(&mut self, dir: &Path, prefix: &str, unicode: bool) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            self.visit(&entry?, prefix, unicode);
        }
        Ok(())
    }

    fn visit(&mut self, entry: &fs::DirEntry, prefix: &str, unicode: bool) {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            return;
        }
        let relative = format!("{prefix}{name}");
        let unicode = unicode && file_name.to_str().is_some();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => self.walk_folder(&entry.path(), relative, unicode),
            Ok(kind) if kind.is_file() && has_markdown_extension(&file_name) => {
                self.classify(entry, relative, unicode);
            }
            Err(_) if has_markdown_extension(&file_name) => self.ignore_unreadable(relative),
            _ => {}
        }
    }

    /// A folder that cannot be listed is recorded with a trailing `/` and the scan goes on.
    fn walk_folder(&mut self, dir: &Path, mut relative: String, unicode: bool) {
        relative.push('/');
        if self.walk(dir, &relative, unicode).is_err() {
            self.ignore_unreadable(relative);
        }
    }

    fn ignore_unreadable(&mut self, path: String) {
        self.ignored.push(Ignored {
            path,
            reason: IgnoreReason::Unreadable,
        });
    }

    fn classify(&mut self, entry: &fs::DirEntry, relative: String, unicode: bool) {
        match NotePath::parse(&relative) {
            Ok(path) if unicode => self.files.push(Candidate {
                path,
                full: entry.path(),
                meta: entry.metadata().ok(),
            }),
            _ => self.ignored.push(Ignored {
                path: relative,
                reason: IgnoreReason::InvalidPath,
            }),
        }
    }

    /// One candidate per note key, ordered by path; the others are ignored as `CaseCollision`.
    pub(crate) fn winners(&mut self) -> Vec<Candidate> {
        let mut seen = HashSet::new();
        let mut winners = Vec::new();
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        for candidate in std::mem::take(&mut self.files) {
            if seen.insert(candidate.path.key()) {
                winners.push(candidate);
            } else {
                self.ignored.push(Ignored {
                    path: candidate.path.to_string(),
                    reason: IgnoreReason::CaseCollision,
                });
            }
        }
        winners
    }

    fn read_notes(mut self) -> Loaded {
        let mut notes = Vec::new();
        for Candidate { path, full, meta } in self.winners() {
            match read_content(&full, meta.as_ref().map(fs::Metadata::len)) {
                ReadOutcome::Content(content) => notes.push((path, content)),
                ReadOutcome::Ignore(reason) => self.ignored.push(Ignored {
                    path: path.to_string(),
                    reason,
                }),
                ReadOutcome::Vanished => {}
            }
        }
        notes.sort_by_key(|(path, _)| path.key());
        self.ignored.sort_by(|a, b| a.path.cmp(&b.path));
        Loaded {
            notes,
            ignored: self.ignored,
        }
    }
}

pub(crate) fn io_error(path: &Path) -> impl FnOnce(io::Error) -> CoreError + '_ {
    move |source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Metadata of the entry itself (a symbolic link is not followed); `None` when missing.
fn lstat(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path)(error)),
    }
}

fn has_markdown_extension(file_name: &std::ffi::OsStr) -> bool {
    Path::new(file_name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(MARKDOWN_EXTENSION))
}

/// What reading one scanned file yielded.
pub(crate) enum ReadOutcome {
    Content(String),
    Ignore(IgnoreReason),
    /// The file disappeared between the walk and the read.
    Vanished,
}

/// Reads a scanned file; `listed_size` is the size the directory listing reported, if any.
pub(crate) fn read_content(full: &Path, listed_size: Option<u64>) -> ReadOutcome {
    let size = match listed_size.map_or_else(|| fs::metadata(full).map(|meta| meta.len()), Ok) {
        Ok(size) => size,
        Err(error) => return read_failure(&error),
    };
    if size > MAX_NOTE_BYTES as u64 {
        return ReadOutcome::Ignore(IgnoreReason::TooLarge);
    }
    match fs::read(full) {
        Ok(bytes) if bytes.len() > MAX_NOTE_BYTES => ReadOutcome::Ignore(IgnoreReason::TooLarge),
        Ok(bytes) => String::from_utf8(bytes).map_or(
            ReadOutcome::Ignore(IgnoreReason::NotUtf8),
            ReadOutcome::Content,
        ),
        Err(error) => read_failure(&error),
    }
}

fn read_failure(error: &io::Error) -> ReadOutcome {
    match error.kind() {
        io::ErrorKind::NotFound => ReadOutcome::Vanished,
        _ => ReadOutcome::Ignore(IgnoreReason::Unreadable),
    }
}

/// The entry of `dir` whose name matches `component` ignoring case; an exact match wins,
/// otherwise the bytewise smallest.
fn find_entry(dir: &Path, component: &str) -> Result<Option<(String, fs::FileType)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(dir)(error)),
    };
    let wanted = component.to_lowercase();
    let mut best: Option<(String, fs::FileType)> = None;
    for entry in entries {
        let entry = entry.map_err(io_error(dir))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.to_lowercase() != wanted {
            continue;
        }
        let kind = entry.file_type().map_err(io_error(dir))?;
        if name == component {
            return Ok(Some((name, kind)));
        }
        if best.as_ref().is_none_or(|(smallest, _)| name < *smallest) {
            best = Some((name, kind));
        }
    }
    Ok(best)
}

fn check_size(bytes: usize) -> Result<()> {
    if bytes > MAX_NOTE_BYTES {
        return Err(CoreError::InvalidInput(format!(
            "a note holds at most {MAX_NOTE_BYTES} bytes, got {bytes}"
        )));
    }
    Ok(())
}

fn join_with_separator(mut existing: String, text: &str) -> String {
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(text);
    existing
}

/// Writes a new file that must not exist yet; removes it again if writing fails.
fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    written.inspect_err(|_| {
        let _ = fs::remove_file(path);
    })
}

/// Creates `target` complete or not at all: the content is written to a synced temporary sibling
/// and hard-linked into place, which fails when the name is taken.
fn create_atomically(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temp_sibling(target);
    write_new_file(&temp, bytes)?;
    let linked = fs::hard_link(&temp, target);
    let _ = fs::remove_file(&temp);
    match linked {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => write_new_file(target, bytes),
        other => other,
    }
}

/// Replaces `target` through a synced sibling temporary file, so readers never see a partial note.
fn write_atomically(target: &Path, bytes: &[u8]) -> Result<()> {
    refuse_read_only(target)?;
    let temp = temp_sibling(target);
    write_new_file(&temp, bytes).map_err(io_error(&temp))?;
    replace_keeping_permissions(&temp, target).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// A read-only note is refused on every platform, before any temporary file exists.
fn refuse_read_only(target: &Path) -> Result<()> {
    if lstat(target)?.is_some_and(|existing| existing.permissions().readonly()) {
        return Err(io_error(target)(io::ErrorKind::PermissionDenied.into()));
    }
    Ok(())
}

/// Renames `temp` over `target` when `still_expected` accepts the target's current bytes.
fn swap_if(
    temp: &Path,
    target: &Path,
    path: &NotePath,
    still_expected: &dyn Fn(&[u8]) -> bool,
) -> Result<bool> {
    let current = fs::read(target).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => CoreError::NotFound(path.to_string()),
        _ => io_error(target)(error),
    })?;
    if !still_expected(&current) {
        return Ok(false);
    }
    replace_keeping_permissions(temp, target)?;
    Ok(true)
}

fn replace_keeping_permissions(temp: &Path, target: &Path) -> Result<()> {
    if let Some(existing) = lstat(target)? {
        fs::set_permissions(temp, existing.permissions()).map_err(io_error(temp))?;
    }
    fs::rename(temp, target).map_err(io_error(target))
}

fn temp_sibling(target: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".ns-{}-{counter}-{nanos}.tmp", std::process::id()))
}

/// Claims a trash location by creating an empty placeholder file there, so a name is never
/// handed out twice: `<stamp>-<flattened path>`, then `-1`, `-2`, ...
fn claim_trash_path(trash: &Path, path: &NotePath, stamp: &str) -> Result<PathBuf> {
    let flattened = path.as_str().replace('/', "__");
    let mut suffix = String::new();
    let mut counter = 0_u32;
    loop {
        let candidate = trash.join(trash_name(stamp, &flattened, &suffix));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                counter += 1;
                suffix = format!("-{counter}");
            }
            Err(error) => return Err(io_error(&candidate)(error)),
        }
    }
}

/// `<stamp>-<flattened><suffix>` within `MAX_FILE_NAME_BYTES`: the flattened path loses its front first.
fn trash_name(stamp: &str, flattened: &str, suffix: &str) -> String {
    let budget = MAX_FILE_NAME_BYTES - stamp.len() - 1 - suffix.len();
    let mut start = flattened.len().saturating_sub(budget);
    while !flattened.is_char_boundary(start) {
        start += 1;
    }
    format!("{stamp}-{}{suffix}", &flattened[start..])
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn p(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    fn put(dir: &tempfile::TempDir, relative: &str, bytes: &[u8]) {
        let file = dir.path().join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }

    fn entry_count(dir: &Path) -> usize {
        fs::read_dir(dir).unwrap().count()
    }

    fn note_names(loaded: &Loaded) -> Vec<&str> {
        loaded.notes.iter().map(|(path, _)| path.as_str()).collect()
    }

    #[test]
    fn should_adopt_spelling_of_existing_folders() {
        let (dir, vault) = vault();
        fs::create_dir_all(dir.path().join("Projects/Sub")).unwrap();
        let adopt = |raw: &str| vault.adopt_folders(&p(raw)).unwrap();
        assert_eq!(adopt("projects/sub/new.md"), p("Projects/Sub/new.md"));
        assert_eq!(adopt("projects/other/New.md"), p("Projects/other/New.md"));
        assert_eq!(adopt("fresh/Deep/n.md"), p("fresh/Deep/n.md"));
        assert_eq!(adopt("Root.md"), p("Root.md"));
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_to_adopt_through_a_symlinked_folder() {
        let (dir, vault) = vault();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("Link")).unwrap();
        let result = vault.adopt_folders(&p("link/n.md"));
        assert!(matches!(result, Err(CoreError::SymlinkInPath(_))));
    }

    #[test]
    fn should_create_without_leaving_temporary_files() {
        let (dir, vault) = vault();
        let folder = dir.path().join("d");
        vault.create(&p("d/n.md"), "content").unwrap();
        assert_eq!(entry_count(&folder), 1);
        assert_eq!(fs::read_to_string(folder.join("n.md")).unwrap(), "content");

        let again = vault.create(&p("d/n.md"), "other");
        assert!(matches!(again, Err(CoreError::AlreadyExists(_))));
        assert_eq!(fs::read_to_string(folder.join("n.md")).unwrap(), "content");
        assert_eq!(entry_count(&folder), 1);

        fs::create_dir(folder.join("dir.md")).unwrap();
        let blocked = vault.create(&p("d/dir.md"), "x");
        assert!(matches!(blocked, Err(CoreError::AlreadyExists(_))));
        assert_eq!(temporary_entries(&folder), 0);
    }

    fn temporary_entries(folder: &Path) -> usize {
        fs::read_dir(folder)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".ns-"))
            .count()
    }

    #[test]
    fn should_create_the_whole_file_when_the_name_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("n.md");
        create_atomically(&target, b"content").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "content");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn should_not_replace_or_litter_when_the_name_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("n.md");
        fs::write(&file, "old").unwrap();
        let error = create_atomically(&file, b"new").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&file).unwrap(), "old");
        assert_eq!(temporary_entries(dir.path()), 0);

        let folder = dir.path().join("dir.md");
        fs::create_dir(&folder).unwrap();
        let error = create_atomically(&folder, b"new").unwrap_err();
        eprintln!("directory in the way reported {:?}", error.kind());
        assert!(folder.is_dir());
        assert_eq!(entry_count(&folder), 0);
        assert_eq!(temporary_entries(dir.path()), 0);
    }

    #[test]
    fn should_replace_only_when_predicate_accepts_current_bytes() {
        let (dir, vault) = vault();
        vault.write(&p("a.md"), "old").unwrap();
        let replaced = vault.replace_if(&p("a.md"), "new", &|bytes| bytes == b"old");
        assert!(matches!(replaced, Ok(true)));
        assert_eq!(vault.read(&p("a.md")).unwrap(), "new");
        let refused = vault.replace_if(&p("a.md"), "newer", &|bytes| bytes == b"old");
        assert!(matches!(refused, Ok(false)));
        assert_eq!(vault.read(&p("a.md")).unwrap(), "new");
        assert_eq!(temporary_entries(dir.path()), 0);
        let missing = vault.replace_if(&p("missing.md"), "x", &|_| true);
        assert!(matches!(missing, Err(CoreError::NotFound(_))));
        assert_eq!(temporary_entries(dir.path()), 0);
    }

    #[test]
    fn should_refuse_to_replace_a_read_only_note() {
        let (dir, vault) = vault();
        vault.write(&p("n.md"), "old").unwrap();
        let file = dir.path().join("n.md");
        let set_readonly = |readonly: bool| {
            let mut permissions = fs::metadata(&file).unwrap().permissions();
            permissions.set_readonly(readonly);
            fs::set_permissions(&file, permissions).unwrap();
        };
        set_readonly(true);
        let written = vault.write(&p("n.md"), "new");
        let appended = vault.append(&p("n.md"), "x");
        set_readonly(false);
        for result in [written.map(|_| ()), appended.map(|_| ())] {
            match result {
                Err(CoreError::Io { source, .. }) => {
                    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
                }
                other => panic!("expected a permission error, got {other:?}"),
            }
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "old");
        assert_eq!(entry_count(dir.path()), 1);
    }

    #[test]
    fn should_fail_to_open_when_root_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = Vault::open(dir.path().join("nope"));
        assert!(matches!(result, Err(CoreError::NotADirectory(_))));
    }

    #[test]
    fn should_round_trip_bytes_when_writing_and_reading() {
        let (_dir, vault) = vault();
        assert!(vault.write(&p("deep/dir/a.md"), "a\r\nb").unwrap());
        assert_eq!(vault.read(&p("deep/dir/a.md")).unwrap(), "a\r\nb");
        assert!(!vault.write(&p("deep/dir/a.md"), "c").unwrap());
    }

    #[test]
    fn should_leave_no_temp_files_when_write_succeeds() {
        let (dir, vault) = vault();
        vault.write(&p("n/a.md"), "1").unwrap();
        vault.write(&p("n/a.md"), "2").unwrap();
        assert_eq!(entry_count(&dir.path().join("n")), 1);
    }

    #[test]
    fn should_append_with_single_separator() {
        let (_dir, vault) = vault();
        assert!(vault.append(&p("m.md"), "x").unwrap());
        assert_eq!(vault.read(&p("m.md")).unwrap(), "x");
        for (existing, text, expected) in [
            ("abc", "def", "abc\ndef"),
            ("abc\n", "def", "abc\ndef"),
            ("", "x", "x"),
            ("a\r\n", "b", "a\r\nb"),
        ] {
            vault.write(&p("e.md"), existing).unwrap();
            assert!(!vault.append(&p("e.md"), text).unwrap());
            assert_eq!(vault.read(&p("e.md")).unwrap(), expected);
        }
    }

    #[test]
    fn should_move_note_to_trash_when_deleted() {
        let (_dir, vault) = vault();
        vault.write(&p("notes/a.md"), "body").unwrap();
        vault.delete(&p("notes/a.md")).unwrap();
        assert!(matches!(
            vault.read(&p("notes/a.md")),
            Err(CoreError::NotFound(_))
        ));
        let trashed: Vec<_> = fs::read_dir(vault.trash_dir())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(trashed.len(), 1);
        let name = trashed[0].file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 16 + "-notes__a.md".len());
        assert!(name.ends_with("-notes__a.md"));
        assert!(name.as_bytes()[8] == b'T' && name.as_bytes()[15] == b'Z');
        assert_eq!(fs::read_to_string(&trashed[0]).unwrap(), "body");
    }

    #[test]
    fn should_keep_both_when_same_path_deleted_twice() {
        let (_dir, vault) = vault();
        for body in ["one", "two"] {
            vault.write(&p("a.md"), body).unwrap();
            vault.delete(&p("a.md")).unwrap();
        }
        assert_eq!(entry_count(&vault.trash_dir()), 2);
    }

    #[test]
    fn should_report_not_found_when_deleting_missing_note() {
        let (_dir, vault) = vault();
        assert!(matches!(
            vault.delete(&p("ghost.md")),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn should_rename_and_create_parent_directories() {
        let (_dir, vault) = vault();
        vault.write(&p("a.md"), "keep").unwrap();
        vault.rename(&p("a.md"), &p("x/y/b.md")).unwrap();
        assert_eq!(vault.read(&p("x/y/b.md")).unwrap(), "keep");
        assert!(matches!(
            vault.read(&p("a.md")),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn should_refuse_rename_when_destination_exists() {
        let (_dir, vault) = vault();
        vault.write(&p("a.md"), "A").unwrap();
        vault.write(&p("b.md"), "B").unwrap();
        assert!(matches!(
            vault.rename(&p("a.md"), &p("b.md")),
            Err(CoreError::AlreadyExists(_))
        ));
        assert_eq!(vault.read(&p("a.md")).unwrap(), "A");
        assert_eq!(vault.read(&p("b.md")).unwrap(), "B");
    }

    #[test]
    fn should_allow_case_only_rename() {
        let (_dir, vault) = vault();
        vault.write(&p("a.md"), "A").unwrap();
        vault.rename(&p("a.md"), &p("A.md")).unwrap();
        assert_eq!(vault.locate(&p("a.md")).unwrap(), Some(p("A.md")));
    }

    #[test]
    fn should_locate_note_ignoring_case() {
        let (dir, vault) = vault();
        put(&dir, "Projects/Idea.md", b"x");
        assert_eq!(
            vault.locate(&p("projects/idea.md")).unwrap(),
            Some(p("Projects/Idea.md"))
        );
        assert_eq!(vault.locate(&p("nope.md")).unwrap(), None);
    }

    #[test]
    fn should_load_only_valid_notes() {
        let (dir, vault) = vault();
        for name in ["b.md", "A.md", ".obsidian/x.md", ".hidden.md", "sub/c.md"] {
            put(&dir, name, name.as_bytes());
        }
        put(&dir, "img.png", b"png");
        put(&dir, "bin.md", &[0xff, 0xfe]);
        let loaded = vault.load().unwrap();
        assert_eq!(note_names(&loaded), ["A.md", "b.md", "sub/c.md"]);
        assert_eq!(loaded.notes[2].1, "sub/c.md");
        assert_eq!(
            loaded.ignored,
            [Ignored {
                path: "bin.md".into(),
                reason: IgnoreReason::NotUtf8
            }]
        );
    }

    #[test]
    fn should_skip_trash_when_loading() {
        let (_dir, vault) = vault();
        vault.write(&p("gone.md"), "x").unwrap();
        vault.delete(&p("gone.md")).unwrap();
        let loaded = vault.load().unwrap();
        assert!(loaded.notes.is_empty() && loaded.ignored.is_empty());
    }

    #[test]
    fn should_not_replace_anything_when_creating() {
        let (dir, vault) = vault();
        vault.create(&p("n.md"), "1").unwrap();
        assert!(matches!(
            vault.create(&p("n.md"), "2"),
            Err(CoreError::AlreadyExists(_))
        ));
        assert_eq!(vault.read(&p("n.md")).unwrap(), "1");
        fs::create_dir(dir.path().join("d.md")).unwrap();
        assert!(matches!(
            vault.create(&p("d.md"), "x"),
            Err(CoreError::AlreadyExists(_))
        ));
        assert_eq!(entry_count(dir.path()), 2);
        assert_eq!(entry_count(&dir.path().join("d.md")), 0);
    }

    #[test]
    fn should_refuse_notes_over_the_size_limit() {
        let (dir, vault) = vault();
        vault
            .write(&p("max.md"), &"a".repeat(MAX_NOTE_BYTES))
            .unwrap();
        let too_big = "a".repeat(MAX_NOTE_BYTES + 1);
        assert!(matches!(
            vault.write(&p("big.md"), &too_big),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            vault.create(&p("big.md"), &too_big),
            Err(CoreError::InvalidInput(_))
        ));
        assert_eq!(entry_count(dir.path()), 1);
        vault.write(&p("n.md"), "x").unwrap();
        assert!(matches!(
            vault.append(&p("n.md"), &"a".repeat(MAX_NOTE_BYTES)),
            Err(CoreError::InvalidInput(_))
        ));
        assert_eq!(vault.read(&p("n.md")).unwrap(), "x");
    }

    #[test]
    fn should_refuse_to_read_an_oversized_file() {
        let (dir, vault) = vault();
        put(&dir, "big.md", &vec![b'a'; MAX_NOTE_BYTES + 1]);
        assert!(matches!(
            vault.read(&p("big.md")),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            vault.append(&p("big.md"), "x"),
            Err(CoreError::InvalidInput(_))
        ));
        let length = fs::metadata(dir.path().join("big.md")).unwrap().len();
        assert_eq!(length, MAX_NOTE_BYTES as u64 + 1);
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_to_write_over_a_symlinked_note() {
        let (dir, vault) = vault();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("x.md");
        fs::write(&target, "secret").unwrap();
        let link = dir.path().join("note.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(
            vault.write(&p("note.md"), "new"),
            Err(CoreError::SymlinkInPath(_))
        ));
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "secret");
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_permissions_when_replacing() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, vault) = vault();
        vault.write(&p("n.md"), "old").unwrap();
        let file = dir.path().join("n.md");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        vault.write(&p("n.md"), "new").unwrap();
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
    }

    #[test]
    fn should_ignore_oversized_files_when_loading() {
        let (dir, vault) = vault();
        put(&dir, "big.md", &vec![b'a'; MAX_NOTE_BYTES + 1]);
        let loaded = vault.load().unwrap();
        assert!(loaded.notes.is_empty());
        assert_eq!(
            loaded.ignored,
            [Ignored {
                path: "big.md".into(),
                reason: IgnoreReason::TooLarge
            }]
        );
    }

    #[test]
    fn should_delete_note_when_its_trash_name_would_be_too_long() {
        let ascii = vec!["a".repeat(200); 4];
        let mut accented = vec!["é".repeat(120); 3];
        // One ASCII byte at the end makes the 255-byte cut fall inside a 2-byte character.
        accented.push(format!("{}a", "é".repeat(119)));
        for folders in [ascii, accented] {
            let path = p(&format!("{}/n.md", folders.join("/")));
            let (_dir, vault) = vault();
            vault.write(&path, "body").unwrap();
            vault.delete(&path).unwrap();
            assert!(matches!(vault.read(&path), Err(CoreError::NotFound(_))));
            let trashed: Vec<_> = fs::read_dir(vault.trash_dir())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(trashed.len(), 1);
            let name = trashed[0].file_name().unwrap().to_str().unwrap();
            assert!(name.len() <= 255, "{} bytes", name.len());
            assert!(name.as_bytes()[..16].ends_with(b"Z") && name.as_bytes()[16] == b'-');
            assert!(name.ends_with("__n.md"));
            assert_eq!(fs::read_to_string(&trashed[0]).unwrap(), "body");
            vault.write(&path, "again").unwrap();
            vault.delete(&path).unwrap();
            assert_eq!(entry_count(&vault.trash_dir()), 2);
        }
    }

    fn assert_only_locked_note_is_unreadable(vault: &Vault) {
        let loaded = vault.load().unwrap();
        assert_eq!(note_names(&loaded), ["a.md", "z.md"]);
        assert_eq!(
            loaded.ignored,
            [Ignored {
                path: "locked.md".into(),
                reason: IgnoreReason::Unreadable
            }]
        );
    }

    #[cfg(windows)]
    #[test]
    fn should_ignore_unreadable_file_and_keep_loading() {
        use std::os::windows::fs::OpenOptionsExt;

        let (dir, vault) = vault();
        for name in ["a.md", "locked.md", "z.md"] {
            put(&dir, name, b"x");
        }
        let _exclusive = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(dir.path().join("locked.md"))
            .unwrap();
        assert_only_locked_note_is_unreadable(&vault);
    }

    #[cfg(unix)]
    #[test]
    fn should_ignore_unreadable_file_and_keep_loading() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, vault) = vault();
        for name in ["a.md", "locked.md", "z.md"] {
            put(&dir, name, b"x");
        }
        let locked = dir.path().join("locked.md");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            return; // running as root: permissions do not apply
        }
        assert_only_locked_note_is_unreadable(&vault);
    }

    struct OnDrop<F: FnMut()>(F);

    impl<F: FnMut()> Drop for OnDrop<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    fn assert_unlistable_folder_is_ignored(vault: &Vault) {
        let loaded = vault.load().unwrap();
        assert_eq!(note_names(&loaded), ["good.md"]);
        assert_eq!(
            loaded.ignored,
            [Ignored {
                path: "locked/".into(),
                reason: IgnoreReason::Unreadable
            }]
        );
    }

    #[cfg(windows)]
    #[test]
    fn should_ignore_an_unlistable_folder_and_keep_scanning() {
        let (dir, vault) = vault();
        put(&dir, "good.md", b"x");
        put(&dir, "locked/hidden.md", b"x");
        let locked = dir.path().join("locked");
        let user = std::env::var("USERNAME").unwrap();
        let icacls = |flag: &str, rule: &str| {
            let status = std::process::Command::new("icacls")
                .arg(&locked)
                .args([flag, rule])
                .output()
                .unwrap()
                .status;
            assert!(status.success());
        };
        icacls("/deny", &format!("{user}:(RD)"));
        let _restore = OnDrop(|| icacls("/remove:d", &user));
        assert!(fs::read_dir(&locked).is_err());
        assert_unlistable_folder_is_ignored(&vault);
    }

    #[cfg(unix)]
    #[test]
    fn should_ignore_an_unlistable_folder_and_keep_scanning() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, vault) = vault();
        put(&dir, "good.md", b"x");
        put(&dir, "locked/hidden.md", b"x");
        let locked = dir.path().join("locked");
        let set_mode = |mode| fs::set_permissions(&locked, fs::Permissions::from_mode(mode));
        set_mode(0o000).unwrap();
        let _restore = OnDrop(|| {
            let _ = set_mode(0o755);
        });
        if fs::read_dir(&locked).is_ok() {
            eprintln!("running as root: permissions do not apply; skipping");
            return;
        }
        assert_unlistable_folder_is_ignored(&vault);
    }

    #[test]
    fn should_keep_suffix_and_stay_within_limit_when_trash_name_is_shortened() {
        let stamp = "20251115T142233Z";
        let ascii = trash_name(stamp, &"x".repeat(900), "-12");
        assert_eq!(ascii.len(), 255);
        assert!(ascii.starts_with("20251115T142233Z-") && ascii.ends_with("-12"));
        let accented = trash_name(stamp, &"é".repeat(600), "-12");
        assert!(accented.len() <= 255 && accented.ends_with("-12"));
        assert!(accented.starts_with("20251115T142233Z-é"));
    }

    #[test]
    fn should_not_overwrite_an_existing_trash_entry() {
        let (_dir, vault) = vault();
        let stamp = "20251115T142233Z";
        fs::create_dir_all(vault.trash_dir()).unwrap();
        let taken = vault.trash_dir().join(trash_name(stamp, "a.md", ""));
        fs::write(&taken, "old").unwrap();
        vault.write(&p("a.md"), "new").unwrap();
        vault.delete_with_stamp(&p("a.md"), stamp).unwrap();
        assert_eq!(fs::read_to_string(&taken).unwrap(), "old");
        let next = vault.trash_dir().join(trash_name(stamp, "a.md", "-1"));
        assert_eq!(fs::read_to_string(next).unwrap(), "new");
        assert_eq!(entry_count(&vault.trash_dir()), 2);
    }

    #[cfg(unix)]
    #[test]
    fn should_refuse_rename_onto_a_different_spelling_that_exists() {
        let (_dir, vault) = vault();
        vault.write(&p("a.md"), "1").unwrap();
        vault.write(&p("A.md"), "2").unwrap();
        assert!(matches!(
            vault.rename(&p("a.md"), &p("A.md")),
            Err(CoreError::AlreadyExists(_))
        ));
        assert_eq!(vault.read(&p("a.md")).unwrap(), "1");
        assert_eq!(vault.read(&p("A.md")).unwrap(), "2");
    }

    #[test]
    fn should_refuse_rename_onto_anything_the_file_system_finds() {
        let (dir, vault) = vault();
        vault.write(&p("a.md"), "a").unwrap();
        fs::create_dir(dir.path().join("b.md")).unwrap();
        assert!(matches!(
            vault.rename(&p("a.md"), &p("b.md")),
            Err(CoreError::AlreadyExists(_))
        ));
        assert_eq!(vault.read(&p("a.md")).unwrap(), "a");
    }

    #[cfg(windows)]
    fn short_name_of(folder: &Path, long_name: &str) -> Option<String> {
        let listing = std::process::Command::new("cmd")
            .args(["/c", "dir", "/x"])
            .current_dir(folder)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&listing.stdout).into_owned();
        let line = text.lines().find(|line| line.contains(long_name))?;
        line.split_whitespace()
            .find(|token| token.to_uppercase().ends_with("~1.MD"))
            .map(str::to_owned)
    }

    #[cfg(windows)]
    #[test]
    fn should_refuse_rename_onto_a_short_name_alias() {
        let (dir, vault) = vault();
        put(&dir, "Notebook long name.md", b"precious");
        put(&dir, "other.md", b"other");
        let Some(alias) = short_name_of(dir.path(), "Notebook long name.md") else {
            eprintln!("volume has no 8.3 short names; skipping");
            return;
        };
        let result = vault.rename(&p("other.md"), &p(&alias));
        assert!(matches!(result, Err(CoreError::AlreadyExists(_))));
        assert_eq!(vault.read(&p("Notebook long name.md")).unwrap(), "precious");
        assert_eq!(vault.read(&p("other.md")).unwrap(), "other");
    }

    #[cfg(unix)]
    #[test]
    fn should_ignore_invalid_names_and_case_collisions() {
        let (dir, vault) = vault();
        for name in ["a:b.md", "Note.md", "note.md"] {
            put(&dir, name, b"x");
        }
        let loaded = vault.load().unwrap();
        assert_eq!(note_names(&loaded), ["Note.md"]);
        assert_eq!(
            loaded.ignored,
            [
                Ignored {
                    path: "a:b.md".into(),
                    reason: IgnoreReason::InvalidPath
                },
                Ignored {
                    path: "note.md".into(),
                    reason: IgnoreReason::CaseCollision
                },
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn should_refuse_paths_through_a_junction() {
        let (dir, vault) = vault();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "secret").unwrap();
        let link = dir.path().join("link");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        vault.write(&p("a.md"), "a").unwrap();
        let through = p("link/secret.md");
        let refused = |result: Result<()>| matches!(result, Err(CoreError::SymlinkInPath(_)));
        assert!(refused(vault.read(&through).map(drop)));
        assert!(refused(vault.write(&through, "x").map(drop)));
        assert!(refused(vault.append(&through, "x").map(drop)));
        assert!(refused(vault.delete(&through)));
        assert!(refused(vault.rename(&through, &p("b.md"))));
        assert!(refused(vault.rename(&p("a.md"), &p("link/x.md"))));
        assert_eq!(entry_count(outside.path()), 1);
        assert_eq!(
            fs::read_to_string(outside.path().join("secret.md")).unwrap(),
            "secret"
        );
        assert_eq!(vault.read(&p("a.md")).unwrap(), "a");
        fs::remove_dir(&link).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn should_not_follow_symlinks() {
        let (dir, vault) = vault();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x.md"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(vault.load().unwrap().notes.is_empty());
        assert!(matches!(
            vault.write(&p("link/y.md"), "y"),
            Err(CoreError::SymlinkInPath(_))
        ));
        assert_eq!(entry_count(outside.path()), 1);
    }
}
