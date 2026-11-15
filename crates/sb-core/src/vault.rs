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

const TRASH_RELATIVE: &str = ".secondbrain/trash";
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
}

/// A `.md` file the scan did not turn into a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// Vault-relative path, `/`-separated (lossy for names that are not Unicode).
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

    /// Where deleted notes go: `<root>/.secondbrain/trash`.
    pub fn trash_dir(&self) -> PathBuf {
        self.root.join(TRASH_RELATIVE)
    }

    /// Scans the vault (spec 5.3) and reads every note.
    ///
    /// # Errors
    /// `Io` when a directory or a listed file cannot be read.
    pub fn load(&self) -> Result<Loaded> {
        let mut scan = Scan::default();
        scan.walk(&self.root, "", true)?;
        scan.read_notes()
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

    /// Reads the note stored under exactly this spelling.
    ///
    /// # Errors
    /// `NotFound` when no regular file is there, `NotUtf8` when its bytes are not text.
    pub fn read(&self, path: &NotePath) -> Result<String> {
        let full = self.require_file(path)?;
        let bytes = fs::read(&full).map_err(io_error(&full))?;
        String::from_utf8(bytes).map_err(|_| CoreError::NotUtf8(path.to_string()))
    }

    /// Creates or atomically replaces a note; true when it did not exist before.
    ///
    /// # Errors
    /// `InvalidInput` over `MAX_NOTE_BYTES`, `SymlinkInPath` or `Io` on file system trouble.
    pub fn write(&self, path: &NotePath, content: &str) -> Result<bool> {
        check_size(content.len())?;
        let target = self.prepare_target(path)?;
        let existed = lstat(&target)?.is_some();
        write_atomically(&target, content.as_bytes())?;
        Ok(!existed)
    }

    /// Creates a note and never replaces anything.
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
        write_new_file(&target, content.as_bytes()).map_err(|source| match source.kind() {
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
        let source = self.require_file(path)?;
        self.confine(TRASH_RELATIVE.split('/'), TRASH_RELATIVE)?;
        let trash = self.trash_dir();
        fs::create_dir_all(&trash).map_err(io_error(&trash))?;
        let destination = free_trash_path(&trash, path)?;
        fs::rename(&source, &destination).map_err(io_error(&source))
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

#[derive(Default)]
struct Scan {
    files: Vec<(NotePath, PathBuf)>,
    ignored: Vec<Ignored>,
}

impl Scan {
    /// `prefix` is the relative path of `dir` with a trailing `/`; `unicode` is false
    /// when any name on the way was not valid Unicode.
    fn walk(&mut self, dir: &Path, prefix: &str, unicode: bool) -> Result<()> {
        for entry in fs::read_dir(dir).map_err(io_error(dir))? {
            let entry = entry.map_err(io_error(dir))?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let full = entry.path();
            let kind = entry.file_type().map_err(io_error(&full))?;
            let unicode = unicode && file_name.to_str().is_some();
            if kind.is_dir() {
                self.walk(&full, &format!("{prefix}{name}/"), unicode)?;
            } else if kind.is_file() && has_markdown_extension(&file_name) {
                self.classify(full, format!("{prefix}{name}"), unicode);
            }
        }
        Ok(())
    }

    fn classify(&mut self, full: PathBuf, relative: String, unicode: bool) {
        match NotePath::parse(&relative) {
            Ok(path) if unicode => self.files.push((path, full)),
            _ => self.ignored.push(Ignored {
                path: relative,
                reason: IgnoreReason::InvalidPath,
            }),
        }
    }

    fn read_notes(self) -> Result<Loaded> {
        let Scan {
            mut files,
            mut ignored,
        } = self;
        files.sort();
        let mut seen = HashSet::new();
        let mut notes = Vec::new();
        for (path, full) in files {
            let reason = if seen.insert(path.key()) {
                match read_content(&full)? {
                    Ok(content) => {
                        notes.push((path.clone(), content));
                        None
                    }
                    Err(reason) => Some(reason),
                }
            } else {
                Some(IgnoreReason::CaseCollision)
            };
            if let Some(reason) = reason {
                ignored.push(Ignored {
                    path: path.to_string(),
                    reason,
                });
            }
        }
        notes.sort_by_key(|(path, _)| path.key());
        ignored.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Loaded { notes, ignored })
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> CoreError + '_ {
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

/// The note's text, or the reason it must be ignored; the outer error is a real failure.
fn read_content(full: &Path) -> Result<std::result::Result<String, IgnoreReason>> {
    let size = fs::metadata(full).map_err(io_error(full))?.len();
    if size > MAX_NOTE_BYTES as u64 {
        return Ok(Err(IgnoreReason::TooLarge));
    }
    let bytes = fs::read(full).map_err(io_error(full))?;
    Ok(String::from_utf8(bytes).map_err(|_| IgnoreReason::NotUtf8))
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

/// Replaces `target` through a synced sibling temporary file, so readers never see a partial note.
fn write_atomically(target: &Path, bytes: &[u8]) -> Result<()> {
    let temp = temp_sibling(target);
    write_new_file(&temp, bytes).map_err(io_error(&temp))?;
    fs::rename(&temp, target).map_err(|source| {
        let _ = fs::remove_file(&temp);
        CoreError::Io {
            path: target.to_path_buf(),
            source,
        }
    })
}

fn temp_sibling(target: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".sb-{}-{counter}-{nanos}.tmp", std::process::id()))
}

/// A trash location that is not taken yet: `<timestamp>-<flattened path>`, then `-1`, `-2`, ...
fn free_trash_path(trash: &Path, path: &NotePath) -> Result<PathBuf> {
    let stamp = utc_compact(SystemTime::now());
    let flattened = path.as_str().replace('/', "__");
    let mut suffix = String::new();
    let mut counter = 0_u32;
    loop {
        let candidate = trash.join(trash_name(&stamp, &flattened, &suffix));
        if lstat(&candidate)?.is_none() {
            return Ok(candidate);
        }
        counter += 1;
        suffix = format!("-{counter}");
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
