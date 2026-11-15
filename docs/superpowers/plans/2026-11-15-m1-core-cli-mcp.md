# M1 — Core, CLI and MCP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust workspace with the vault domain library `sb-core` and the console binary `sb`, giving people and AI agents every note operation through CLI commands and an MCP stdio server.

**Architecture:** `sb-core` owns note paths, vault file operations, Markdown parsing, the in-memory index, link resolution, graph and search, and exposes them as a small set of operations returning serialisable results. `sb-cli` is a thin shell: `clap` commands and an `rmcp` server that both call those operations. The Markdown files are the only state.

**Tech Stack:** Rust 1.99 stable (edition 2024), `pulldown-cmark` 0.13, `thiserror` 2, `serde`/`serde_json` 1, `clap` 4, `rmcp` 3.10, `schemars` 1, `tokio` 1, `tempfile` 3 (tests).

**Spec:** `docs/superpowers/specs/2026-11-15-second-brain-design.md` — sections 5, 6 and 7 are implemented here. Section and rule numbers below refer to it.

## Global Constraints

- Rust only. Edition 2024. No `unsafe` (`unsafe_code = "forbid"`).
- No `unwrap()`, `expect()` or `panic!` outside tests. Errors are values.
- Every public item has a doc comment (`missing_docs` is a warning and warnings are errors).
- Functions stay at or under 30 lines of code and 4 parameters (enforced by clippy configuration from Task 1).
- Before every commit, all three pass with no output other than success:
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Test names follow `should_<expected>_when_<condition>`.
- Commits follow Conventional Commits (`feat(core): …`, `test(cli): …`), one logical change each.
- Note paths use `/` on every platform. File contents are never altered: no line-ending or trailing-newline normalisation.
- Every path that comes from outside the process (CLI argument, MCP argument) is parsed with `NotePath::parse` before any disk access.
- Nothing is ever unlinked directly: deleting moves the file to `.secondbrain/trash/`.
- The MCP server writes only protocol messages to stdout. Diagnostics go to stderr.
- A dependency is added only where this plan names it.
- `README.md` and `CHANGELOG.md` are updated by the task that adds a user-visible feature.

## Review Focus

Input classes the spec implies and that are most likely to bite a user. Each has a test in the task that owns the code.

1. **A note asked for with different letter case than on disk** (`sb read projects/idea.md` for `projects/Idea.md`, on Linux). Expected: found, and written back to the existing file, never to a second file. → Task 2 (`locate`), Task 6.
2. **CRLF files and files without a trailing newline.** Expected: bytes preserved on read, write and append; line numbers and snippets still correct. → Tasks 2, 3, 5.
3. **Arguments of the wrong type or missing from an MCP client.** Expected: a tool result with `isError: true`, the server keeps running. → Task 8.
4. **A vault with thousands of notes.** Expected: building the graph is not quadratic in notes × links; resolution goes through a stem lookup. → Task 4.
5. **Non-ASCII names** (`día/ñandú.md`, emoji). Expected: valid paths, case-insensitive identity by Unicode lowercase, truncation never splits a character. → Tasks 1, 5.

---

### Task 1: Workspace and note paths

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `clippy.toml`, `README.md`, `CHANGELOG.md`
- Create: `crates/sb-core/Cargo.toml`, `crates/sb-core/src/lib.rs`, `crates/sb-core/src/path.rs`

**Interfaces:**
- Consumes: nothing.
- Produces (in `sb_core`, re-exported from `lib.rs`):

```rust
pub struct NotePath(/* private String */);   // Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Display, serde::Serialize (as a plain string)
pub struct NoteKey(/* private String */);    // Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Display

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    Empty, TooLong, Backslash, Absolute, EmptyComponent, DotComponent, Hidden,
    IllegalChar(char), TrailingSpaceOrDot, ReservedName(String), NotMarkdown,
}

impl NotePath {
    pub fn parse(raw: &str) -> Result<NotePath, PathError>;
    pub fn as_str(&self) -> &str;
    pub fn key(&self) -> NoteKey;            // Unicode lowercase of the path
    pub fn stem(&self) -> &str;              // last component without its final ".md" (any case)
    pub fn folder(&self) -> &str;            // "" at the vault root, else "a/b" (no trailing slash)
    pub fn components(&self) -> impl Iterator<Item = &str>;
}
impl NoteKey { pub fn as_str(&self) -> &str; }
```

- [ ] **Step 1: Create the workspace**

Root `Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
publish = false

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tempfile = "3"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
too_many_lines = "warn"
```

`clippy.toml`:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
too-many-lines-threshold = 30
too-many-arguments-threshold = 4
```

`.gitignore`: `/target`, `/dist`, `.claude/worktrees/`, `.worktrees/`, `graphify-out/`.

`crates/sb-core/Cargo.toml`: package `sb-core`, inherits `version`, `edition`, `publish`; `[lints] workspace = true`; dependencies `serde` and `thiserror` from the workspace; dev-dependency `tempfile` from the workspace.

`README.md`: title «Second Brain», one paragraph taken from spec section 1, a «Status» line saying the project is under construction. `CHANGELOG.md`: Keep a Changelog format with an empty `## [Unreleased]` section.

Integration test files (`tests/*.rs`) in every crate start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]`, because clippy's in-test allowances do not reach helper functions there.

- [ ] **Step 2: Write the failing tests** in `crates/sb-core/src/path.rs` (`#[cfg(test)] mod tests`)

```rust
#[test]
fn should_accept_path_when_it_is_valid() {
    for p in ["a.md", "Projects/Idea.md", "a/b/c/Note 1.md", "día/ñandú.md", "x.MD", "notes/v1.2 plan.md", "🧠/idea.md"] {
        assert_eq!(NotePath::parse(p).unwrap().as_str(), p);
    }
}

#[test]
fn should_reject_path_when_a_rule_is_broken() {
    let cases: &[(&str, PathError)] = &[
        ("", PathError::Empty),
        ("a\\b.md", PathError::Backslash),
        ("/a.md", PathError::Absolute),
        ("a//b.md", PathError::EmptyComponent),
        ("a/", PathError::EmptyComponent),
        ("./a.md", PathError::DotComponent),
        ("a/../b.md", PathError::DotComponent),
        (".hidden/a.md", PathError::Hidden),
        ("a/.b.md", PathError::Hidden),
        ("a/.md", PathError::Hidden),
        (".secondbrain/trash/x.md", PathError::Hidden),
        ("C:/a.md", PathError::IllegalChar(':')),
        ("a?.md", PathError::IllegalChar('?')),
        ("a\u{0007}.md", PathError::IllegalChar('\u{0007}')),
        ("dir /a.md", PathError::TrailingSpaceOrDot),
        ("dir./a.md", PathError::TrailingSpaceOrDot),
        ("a.md ", PathError::TrailingSpaceOrDot),
        ("CON.md", PathError::ReservedName("CON".into())),
        ("x/com1.notes.md", PathError::ReservedName("com1".into())),
        ("a.txt", PathError::NotMarkdown),
        ("a", PathError::NotMarkdown),
    ];
    for (raw, expected) in cases {
        assert_eq!(NotePath::parse(raw).unwrap_err(), *expected, "path: {raw:?}");
    }
}

#[test]
fn should_reject_path_when_it_is_too_long() {
    let long_component = format!("{}.md", "a".repeat(253)); // 256 bytes
    assert_eq!(NotePath::parse(&long_component).unwrap_err(), PathError::TooLong);
    let long_path = format!("{}x.md", "ab/".repeat(341)); // 1027 bytes
    assert_eq!(NotePath::parse(&long_path).unwrap_err(), PathError::TooLong);
}

#[test]
fn should_expose_key_stem_and_folder() {
    let p = NotePath::parse("Projects/ÀB Idea.md").unwrap();
    assert_eq!(p.key().as_str(), "projects/àb idea.md");
    assert_eq!(p.stem(), "ÀB Idea");
    assert_eq!(p.folder(), "Projects");
    assert_eq!(p.components().collect::<Vec<_>>(), ["Projects", "ÀB Idea.md"]);
    let root = NotePath::parse("b.c.MD").unwrap();
    assert_eq!(root.stem(), "b.c");
    assert_eq!(root.folder(), "");
}

#[test]
fn should_serialize_as_plain_string() {
    let p = NotePath::parse("a/b.md").unwrap();
    assert_eq!(serde_json::to_string(&p).unwrap(), "\"a/b.md\"");
}
```

(`serde_json` is a dev-dependency of `sb-core` for this test.)

- [ ] **Step 3: Run the tests and see them fail**

Run: `cargo test -p sb-core path::`
Expected: compilation fails, `NotePath` not found.

- [ ] **Step 4: Implement `NotePath`, `NoteKey`, `PathError`**

Rules are spec section 5.2, checked in this order so the error is deterministic: `Empty`; whole path longer than 1024 bytes → `TooLong`; any `\` → `Backslash`; leading `/` → `Absolute`; then for each component in order: empty → `EmptyComponent`; `.` or `..` → `DotComponent`; starts with `.` → `Hidden`; longer than 255 bytes → `TooLong`; contains one of `< > : " | ? *` or a control character (U+0000–U+001F, U+007F) → `IllegalChar(c)` with the first offender; ends with a space or a dot → `TrailingSpaceOrDot`; the part before its first `.` equals a Windows device name ASCII-case-insensitively (`CON PRN AUX NUL COM1`–`COM9 LPT1`–`LPT9`) → `ReservedName` carrying that part as written; finally the last component must end in `.md` ASCII-case-insensitively → else `NotMarkdown`.

`PathError` messages (via `thiserror`) are lower-case phrases a user can act on, for example `"path is empty"`, `"character `:` is not allowed"`, `"name `CON` is reserved on Windows"`, `"note paths must end with .md"`.

- [ ] **Step 5: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore clippy.toml README.md CHANGELOG.md crates/sb-core
git commit -m "feat(core): add workspace and validated note paths"
```

---

### Task 2: Vault file operations

**Files:**
- Create: `crates/sb-core/src/error.rs`, `crates/sb-core/src/vault.rs`, `crates/sb-core/src/time.rs`
- Modify: `crates/sb-core/src/lib.rs` (modules and re-exports)

**Interfaces:**
- Consumes: `NotePath`, `NoteKey`, `PathError` (Task 1).
- Produces:

```rust
// error.rs
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid note path `{path}`: {source}")] InvalidPath { path: String, source: PathError },
    #[error("note not found: {0}")] NotFound(String),
    #[error("a note already exists at {0}")] AlreadyExists(String),
    #[error("{} is not a directory", .0.display())] NotADirectory(std::path::PathBuf),
    #[error("a symbolic link is in the way of {0}")] SymlinkInPath(String),
    #[error("{0} is not valid UTF-8")] NotUtf8(String),
    #[error("invalid input: {0}")] InvalidInput(String),
    #[error("I/O error on {}: {source}", .path.display())] Io { path: std::path::PathBuf, source: std::io::Error },
}
pub type Result<T> = std::result::Result<T, CoreError>;

// vault.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason { InvalidPath, CaseCollision, NotUtf8 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored { pub path: String, pub reason: IgnoreReason }
#[derive(Debug)]
pub struct Loaded { pub notes: Vec<(NotePath, String)>, pub ignored: Vec<Ignored> } // notes ordered by key

#[derive(Debug, Clone)]
pub struct Vault { /* root: PathBuf */ }
impl Vault {
    pub fn open(root: impl Into<std::path::PathBuf>) -> Result<Vault>;   // NotADirectory if missing or not a dir
    pub fn root(&self) -> &std::path::Path;
    pub fn load(&self) -> Result<Loaded>;                                // scan (spec 5.3) and read every note
    pub fn locate(&self, path: &NotePath) -> Result<Option<NotePath>>;   // the on-disk spelling of this key, if present
    pub fn read(&self, path: &NotePath) -> Result<String>;               // NotFound, NotUtf8
    pub fn write(&self, path: &NotePath, content: &str) -> Result<bool>; // atomic; true when the file did not exist
    pub fn append(&self, path: &NotePath, text: &str) -> Result<bool>;   // true when created
    pub fn delete(&self, path: &NotePath) -> Result<()>;                 // move to trash; NotFound
    pub fn rename(&self, from: &NotePath, to: &NotePath) -> Result<()>;  // NotFound, AlreadyExists
    pub fn trash_dir(&self) -> std::path::PathBuf;                       // <root>/.secondbrain/trash
}

// time.rs
pub fn utc_compact(t: std::time::SystemTime) -> String;                  // "20251115T142233Z"
```

`read`, `write`, `append`, `delete`, `rename` act on the exact spelling they are given. Case-insensitive lookup is `locate`, used by Task 6.

- [ ] **Step 1: Write the failing tests** in `vault.rs` and `time.rs`

`time.rs`:

```rust
#[test]
fn should_format_utc_compact() {
    use std::time::{Duration, UNIX_EPOCH};
    assert_eq!(utc_compact(UNIX_EPOCH), "19700101T000000Z");
    assert_eq!(utc_compact(UNIX_EPOCH + Duration::from_secs(951_782_400)), "20000229T000000Z");
    assert_eq!(utc_compact(UNIX_EPOCH + Duration::from_secs(1_763_216_553)), "20251115T142233Z");
}
```

`vault.rs` — each test builds a vault in `tempfile::tempdir()`; `p("a.md")` is a helper for `NotePath::parse("a.md").unwrap()`:

```rust
#[test] fn should_fail_to_open_when_root_is_missing()            // Vault::open(tmp/"nope") is Err(NotADirectory)
#[test] fn should_round_trip_bytes_when_writing_and_reading()    // write "a\r\nb" (no trailing newline) to "deep/dir/a.md": returns Ok(true); read == "a\r\nb"; second write returns Ok(false)
#[test] fn should_leave_no_temp_files_when_write_succeeds()      // after two writes, the note's directory holds exactly one entry
#[test] fn should_append_with_single_separator()                 // missing + "x" -> "x" (created = true); "abc" + "def" -> "abc\ndef"; "abc\n" + "def" -> "abc\ndef"; "" + "x" -> "x"; "a\r\n" + "b" -> "a\r\nb"
#[test] fn should_move_note_to_trash_when_deleted()              // file gone; trash_dir has exactly 1 file; its name is 16 chars of timestamp, then "-notes__a.md" for path "notes/a.md"; its content is the note's
#[test] fn should_keep_both_when_same_path_deleted_twice()       // write, delete, write, delete -> 2 files in trash_dir
#[test] fn should_report_not_found_when_deleting_missing_note()
#[test] fn should_rename_and_create_parent_directories()         // "a.md" -> "x/y/b.md"; content preserved; "a.md" gone
#[test] fn should_refuse_rename_when_destination_exists()        // AlreadyExists; both files unchanged
#[test] fn should_allow_case_only_rename()                       // "a.md" -> "A.md" is Ok; locate(p("a.md")) == Some(p("A.md"))
#[test] fn should_locate_note_ignoring_case()                    // "Projects/Idea.md" on disk; locate(p("projects/idea.md")) == Some(p("Projects/Idea.md")); locate(p("nope.md")) == None
#[test] fn should_load_only_valid_notes()                        // files: "b.md", "A.md", ".obsidian/x.md", ".hidden.md", "img.png", "sub/c.md", "bin.md" containing bytes [0xff, 0xfe]
                                                                 // notes == [("A.md", ..), ("b.md", ..), ("sub/c.md", ..)] in that order; ignored == [Ignored { path: "bin.md", reason: NotUtf8 }]
#[test] fn should_skip_trash_when_loading()                      // a deleted note does not come back from load()
#[cfg(unix)] #[test] fn should_ignore_invalid_names_and_case_collisions()
                                                                 // files "a:b.md", "Note.md", "note.md": notes == ["Note.md"]; ignored holds ("a:b.md", InvalidPath) and ("note.md", CaseCollision)
#[cfg(unix)] #[test] fn should_not_follow_symlinks()             // a symlinked directory "link" -> outside dir holding "x.md": load() does not list it; write(p("link/y.md")) is Err(SymlinkInPath); nothing is created outside
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-core vault:: time::`
Expected: compilation fails, `Vault` not found.

- [ ] **Step 3: Implement `time::utc_compact`**

Civil date from days since 1970-01-01 with integer arithmetic only (no date crate). Times before the epoch are formatted as the epoch.

- [ ] **Step 4: Implement `Vault`**

- `load`: walk recursively with `std::fs::read_dir` and `DirEntry::file_type` (which does not follow links). Skip names starting with `.`; skip symbolic links; recurse into directories; consider files whose extension is `md` ASCII-case-insensitively. Build the relative path with `/`. A name that is not valid Unicode, or a path `NotePath::parse` rejects → `Ignored { InvalidPath }` with the lossy path. Among paths sharing one key keep the bytewise smallest; the others → `CaseCollision`. Read each kept file; invalid UTF-8 → `NotUtf8`. `ignored` is ordered by path.
- `locate`: walk the components from the root; in each directory find the entry whose name lowercases (`str::to_lowercase`) to the component's lowercase, preferring an exact match. Never use `Path::exists` for this: on Windows it would return the requested spelling instead of the stored one.
- `write`: check confinement (spec 5.4: every existing ancestor below the root must be a directory and not a symbolic link, else `SymlinkInPath`), create parent directories, write a sibling file `.sb-<process id>-<counter>-<nanoseconds>.tmp`, `sync_all`, then `std::fs::rename` over the target. Remove the temporary file if any step fails.
- `append`: read (missing means empty and created), apply the separator rule of spec 5.4, `write`.
- `delete`: `NotFound` if missing; create `trash_dir()`; target name `<utc_compact(now)>-<path with "/" replaced by "__">`; if taken, add `-1`, `-2`, …; `std::fs::rename`.
- `rename`: `NotFound` if `from` is missing; `AlreadyExists` if `locate(to)` finds a note whose key differs from `from.key()`; confinement check and parent creation for `to`; `std::fs::rename`.
- Every `std::io::Error` becomes `CoreError::Io` carrying the path it happened on.

- [ ] **Step 5: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/sb-core
git commit -m "feat(core): add vault file operations with atomic writes and trash"
```

---

### Task 3: Markdown links and tags

**Files:**
- Create: `crates/sb-core/src/markdown.rs`
- Modify: `crates/sb-core/Cargo.toml` (add `pulldown-cmark = { version = "0.13", default-features = false }`), `crates/sb-core/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind { Wiki, Embed, Markdown }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Link {
    pub target: String,                                            // as written, heading removed, never empty
    #[serde(skip_serializing_if = "Option::is_none")] pub heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub alias: Option<String>,
    pub kind: LinkKind,
    pub line: u32,                                                 // 1-based
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed { pub links: Vec<Link>, pub tags: Vec<String> } // links in document order; tags lowercased, sorted, unique

pub fn parse(content: &str) -> Parsed;
```

- [ ] **Step 1: Write the failing tests** in `markdown.rs`

```rust
fn link(target: &str, heading: Option<&str>, alias: Option<&str>, kind: LinkKind, line: u32) -> Link { /* builds a Link */ }

#[test]
fn should_extract_links_of_every_kind() {
    let md = "# Title\n\
See [[Note A]] and [[Folder/Note B|B alias]] and [[Note C#Section]].\n\
Embed: ![[Diagram]]\n\
MD: [text](other/Note%20D.md#h) and [ext](https://example.com/x.md) and [img](pic.png)\n";
    assert_eq!(parse(md).links, vec![
        link("Note A", None, None, LinkKind::Wiki, 2),
        link("Folder/Note B", None, Some("B alias"), LinkKind::Wiki, 2),
        link("Note C", Some("Section"), None, LinkKind::Wiki, 2),
        link("Diagram", None, None, LinkKind::Embed, 3),
        link("other/Note D.md", Some("h"), None, LinkKind::Markdown, 4),
    ]);
}

#[test]
fn should_ignore_links_inside_code() {
    let md = "`[[inline code]]`\n\n```\n[[fenced]]\n```\n\n    [[indented]]\n";
    assert!(parse(md).links.is_empty());
}

#[test]
fn should_drop_links_without_target() {
    assert!(parse("[[#Only heading]] and [[]]").links.is_empty());
}

#[test]
fn should_count_lines_with_crlf() {
    assert_eq!(parse("one\r\ntwo\r\n[[X]]\r\n").links, vec![link("X", None, None, LinkKind::Wiki, 3)]);
}

#[test]
fn should_extract_inline_tags() {
    let md = "#tag1 text #Tag-Two/sub (#paren) #123 a#nottag #snake_case\n`#code`\n";
    assert_eq!(parse(md).tags, ["paren", "snake_case", "tag-two/sub", "tag1"]);
}

#[test]
fn should_ignore_front_matter() {
    let md = "---\ntitle: x #fm\n---\nBody #real [[L]]\n";
    let parsed = parse(md);
    assert_eq!(parsed.tags, ["real"]);
    assert_eq!(parsed.links, vec![link("L", None, None, LinkKind::Wiki, 4)]);
}

#[test]
fn should_not_treat_heading_marker_as_tag() {
    assert!(parse("# Heading\n## Another\n").tags.is_empty());
}
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-core markdown::`
Expected: compilation fails, `parse` not found.

- [ ] **Step 3: Implement `parse`**

Use `pulldown_cmark::Parser::new_ext(content, options).into_offset_iter()` with `ENABLE_WIKILINKS`, `ENABLE_YAML_STYLE_METADATA_BLOCKS`, `ENABLE_TABLES`, `ENABLE_STRIKETHROUGH`, `ENABLE_TASKLISTS`, `ENABLE_FOOTNOTES`. Read the crate's documentation for 0.13 in `~/.cargo/registry/src/` to confirm how wikilinks are reported (a `LinkType` variant on `Tag::Link` and `Tag::Image`, with the destination holding the target and the inner text holding the alias when a `|` is present). The tests above define the behaviour; where the parser reports something slightly different (surrounding spaces, for instance), adapt the code, not the tests.

- Wiki link → `Wiki`; wiki image → `Embed`; any other link whose destination has no URL scheme (`^[A-Za-z][A-Za-z0-9+.-]*:`), is percent-decoded, and ends with `.md` ASCII-case-insensitively before any `#` → `Markdown`. Everything else is not a link.
- Split the destination at the first `#` into target and heading; trim both; drop the link if the target is empty.
- `line` = 1 + number of `\n` bytes before the event's start offset.
- Tags: gather consecutive text events of one run into a single string before scanning (the parser splits text around characters such as `_`), treating soft and hard breaks as whitespace. Rule: spec section 6.4. Text inside code spans, code blocks and metadata blocks is not scanned.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sb-core Cargo.lock
git commit -m "feat(core): extract links and tags from markdown"
```

---

### Task 4: Index and link resolution

**Files:**
- Create: `crates/sb-core/src/index.rs`
- Modify: `crates/sb-core/src/lib.rs`

**Interfaces:**
- Consumes: `NotePath`, `NoteKey` (Task 1); `Vault::load`, `Ignored`, `Result` (Task 2); `markdown::{parse, Link}` (Task 3).
- Produces:

```rust
#[derive(Debug, Clone)]
pub struct NoteEntry { pub path: NotePath, pub content: String, pub links: Vec<Link>, pub tags: Vec<String> }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OutLink {
    #[serde(flatten)] pub link: Link,
    #[serde(skip_serializing_if = "Option::is_none")] pub resolved: Option<NotePath>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Backlink { pub from: NotePath, pub line: u32, pub context: String }   // context: that line, trimmed, at most 200 chars
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TagCount { pub tag: String, pub count: usize }                       // count = notes carrying the tag

#[derive(Debug, Default)]
pub struct Index { /* notes: BTreeMap<NoteKey, NoteEntry>, by_stem: HashMap<String, BTreeSet<NoteKey>>, ignored: Vec<Ignored> */ }
impl Index {
    pub fn build(vault: &Vault) -> Result<Index>;
    pub fn from_notes(notes: Vec<(NotePath, String)>) -> Index;
    pub fn upsert(&mut self, path: NotePath, content: String);
    pub fn remove(&mut self, key: &NoteKey) -> bool;
    pub fn get(&self, key: &NoteKey) -> Option<&NoteEntry>;
    pub fn notes(&self) -> impl Iterator<Item = &NoteEntry>;                    // ordered by key
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn ignored(&self) -> &[Ignored];
    pub fn resolve(&self, target: &str, from: &NotePath) -> Option<&NotePath>;
    pub fn outgoing(&self, key: &NoteKey) -> Vec<OutLink>;                      // document order; empty if the note is unknown
    pub fn backlinks(&self, key: &NoteKey) -> Vec<Backlink>;                    // unique per (from, line); ordered by from key, then line
    pub fn tags(&self) -> Vec<TagCount>;                                        // count descending, then tag ascending
}
pub(crate) fn truncate_chars(s: &str, max: usize) -> &str;                      // never splits a character
pub(crate) fn normalize_target(target: &str) -> String;                         // spec 6.2 normalisation
```

- [ ] **Step 1: Write the failing tests** in `index.rs`

Shared fixture:

```rust
fn fixture() -> Index {
    let n = |p: &str, c: &str| (NotePath::parse(p).unwrap(), c.to_string());
    Index::from_notes(vec![
        n("Home.md", "[[Idea]] [[projects/Plan]] [[Missing Note]] [[idea|again]]"),
        n("projects/Idea.md", "#work [[Home]]"),
        n("projects/Plan.md", "[[Idea]] [[../Home]]"),
        n("archive/Idea.md", "#work #old"),
        n("archive/deep/Idea.md", ""),
    ])
}
fn p(s: &str) -> NotePath { NotePath::parse(s).unwrap() }
```

```rust
#[test]
fn should_resolve_links_by_spec_rules() {
    let ix = fixture();
    let r = |t: &str, from: &str| ix.resolve(t, &p(from)).map(|x| x.as_str().to_string());
    assert_eq!(r("Idea", "Home.md").as_deref(), Some("archive/Idea.md"));            // tie on depth -> smallest key
    assert_eq!(r("Idea", "projects/Plan.md").as_deref(), Some("projects/Idea.md"));  // same folder first
    assert_eq!(r("projects/Plan", "Home.md").as_deref(), Some("projects/Plan.md"));  // exact path
    assert_eq!(r("../Home", "projects/Plan.md").as_deref(), Some("Home.md"));        // relative to the source folder
    assert_eq!(r("deep/Idea", "Home.md").as_deref(), Some("archive/deep/Idea.md"));  // suffix match
    assert_eq!(r("IDEA.md", "archive/Idea.md").as_deref(), Some("archive/Idea.md")); // case and extension ignored
    assert_eq!(r(" .\\projects\\plan ", "Home.md").as_deref(), Some("projects/Plan.md"));
    assert_eq!(r("Missing Note", "Home.md"), None);
    assert_eq!(r("../../etc/passwd", "Home.md"), None);
}

#[test]
fn should_list_outgoing_links_with_resolution() {
    let ix = fixture();
    let out = ix.outgoing(&p("home.md").key());
    let resolved: Vec<_> = out.iter().map(|o| o.resolved.as_ref().map(|x| x.as_str())).collect();
    assert_eq!(resolved, [Some("archive/Idea.md"), Some("projects/Plan.md"), None, Some("archive/Idea.md")]);
}

#[test]
fn should_list_backlinks_once_per_line() {
    let ix = fixture();
    assert_eq!(ix.backlinks(&p("archive/Idea.md").key()), vec![Backlink {
        from: p("Home.md"), line: 1,
        context: "[[Idea]] [[projects/Plan]] [[Missing Note]] [[idea|again]]".into(),
    }]);
    let home: Vec<_> = ix.backlinks(&p("Home.md").key()).into_iter().map(|b| b.from.as_str().to_string()).collect();
    assert_eq!(home, ["projects/Idea.md", "projects/Plan.md"]);
}

#[test]
fn should_count_tags_by_note() {
    let tags = fixture().tags();
    assert_eq!(tags, vec![TagCount { tag: "work".into(), count: 2 }, TagCount { tag: "old".into(), count: 1 }]);
}

#[test]
fn should_follow_upsert_and_remove() {
    let mut ix = fixture();
    assert!(ix.remove(&p("projects/Idea.md").key()));
    assert!(!ix.remove(&p("projects/Idea.md").key()));
    assert_eq!(ix.resolve("Idea", &p("projects/Plan.md")).map(|x| x.as_str()), Some("archive/Idea.md"));
    ix.upsert(p("Missing Note.md"), "#new".into());
    assert_eq!(ix.resolve("missing note", &p("Home.md")).map(|x| x.as_str()), Some("Missing Note.md"));
    ix.upsert(p("Home.md"), "no links".into());
    assert!(ix.outgoing(&p("Home.md").key()).is_empty());
    assert_eq!(ix.len(), 5);
}

#[test]
fn should_replace_entry_when_upserting_with_different_case() {
    let mut ix = fixture();
    ix.upsert(p("HOME.md"), "x".into());
    assert_eq!(ix.len(), 5);
    assert_eq!(ix.get(&p("home.md").key()).unwrap().path.as_str(), "HOME.md");
}

#[test]
fn should_truncate_on_character_boundaries() {
    assert_eq!(truncate_chars("añb", 2), "añ");
    assert_eq!(truncate_chars("🧠🧠🧠", 2), "🧠🧠");
    assert_eq!(truncate_chars("ab", 5), "ab");
}

#[test]
fn should_resolve_quickly_in_a_large_vault() {
    let notes: Vec<_> = (0..5000).map(|i| (p(&format!("f{}/n{i}.md", i % 50)), format!("[[n{}]] [[n{}]]", (i + 1) % 5000, (i + 7) % 5000))).collect();
    let ix = Index::from_notes(notes);
    let started = std::time::Instant::now();
    let resolved = ix.notes().flat_map(|n| ix.outgoing(&n.path.key())).filter(|o| o.resolved.is_some()).count();
    assert_eq!(resolved, 10_000);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "resolution must not scan every note per link");
}
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-core index::`
Expected: compilation fails, `Index` not found.

- [ ] **Step 3: Implement `Index`**

- `upsert` parses the content with `markdown::parse`, replaces any entry with the same key and keeps `by_stem` (lowercased stem → keys) in step. `remove` does the reverse.
- `resolve` follows spec section 6.2 exactly. Candidates always come from `by_stem[last component of the normalised target]`, then are filtered by the rule in force (exact, relative, suffix, bare name), so no rule scans all notes. Tie-break: same folder as `from` (compared by key), then fewest components, then smallest key.
- `backlinks` scans the links of all notes, resolving each from its own note; `context` is the linking line (split on `\n`, a trailing `\r` removed), trimmed and cut with `truncate_chars(…, 200)`.
- `build` is `Vault::load` followed by `from_notes`, keeping the `ignored` list.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sb-core
git commit -m "feat(core): add note index with link resolution, backlinks and tags"
```

---

### Task 5: Graph and search

**Files:**
- Create: `crates/sb-core/src/graph.rs`, `crates/sb-core/src/search.rs`
- Modify: `crates/sb-core/src/lib.rs`

**Interfaces:**
- Consumes: `Index`, `NoteEntry`, `truncate_chars`, `normalize_target` (Task 4); `NotePath`, `NoteKey` (Task 1).
- Produces:

```rust
// graph.rs
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GraphNode { pub id: String, pub label: String, pub exists: bool }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GraphEdge { pub from: String, pub to: String }
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Graph { pub nodes: Vec<GraphNode>, pub edges: Vec<GraphEdge> }
pub const MAX_GRAPH_DEPTH: u8 = 5;
impl Index {
    pub fn graph(&self) -> Graph;
    pub fn graph_around(&self, key: &NoteKey, depth: u8) -> Option<Graph>;   // None when the note is unknown; depth clamped to 1..=MAX_GRAPH_DEPTH
}

// search.rs
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Snippet { pub line: u32, pub text: String }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchHit { pub path: NotePath, pub title: String, pub score: u32, pub snippets: Vec<Snippet> }
pub const DEFAULT_SEARCH_LIMIT: usize = 20;
pub const MAX_SEARCH_LIMIT: usize = 100;
impl Index {
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit>;       // limit clamped to 1..=MAX_SEARCH_LIMIT
}
```

Ordering, fixed so output is deterministic: note nodes ordered by key, then ghost nodes ordered by id; edges ordered bytewise by `(from, to)`.

- [ ] **Step 1: Write the failing tests**

`graph.rs`, using the same `fixture()` and `p()` as Task 4 (copy them into this test module):

```rust
#[test]
fn should_build_whole_graph_with_ghost_nodes() {
    let g = fixture().graph();
    let ids: Vec<_> = g.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str(), n.exists)).collect();
    assert_eq!(ids, [
        ("archive/deep/Idea.md", "Idea", true),
        ("archive/Idea.md", "Idea", true),
        ("Home.md", "Home", true),
        ("projects/Idea.md", "Idea", true),
        ("projects/Plan.md", "Plan", true),
        ("?missing note", "Missing Note", false),
    ]);
    let edges: Vec<_> = g.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
    assert_eq!(edges, [
        ("Home.md", "?missing note"),
        ("Home.md", "archive/Idea.md"),
        ("Home.md", "projects/Plan.md"),
        ("projects/Idea.md", "Home.md"),
        ("projects/Plan.md", "Home.md"),
        ("projects/Plan.md", "projects/Idea.md"),
    ]);
}

#[test]
fn should_ignore_self_links_and_duplicates() {
    let ix = Index::from_notes(vec![(p("a.md"), "[[a]] [[b]] [[b]] [[B]]".into()), (p("b.md"), String::new())]);
    let g = ix.graph();
    assert_eq!(g.edges, vec![GraphEdge { from: "a.md".into(), to: "b.md".into() }]);
}

#[test]
fn should_return_neighbourhood_by_depth() {
    let ix = fixture();
    let ids = |g: Graph| g.nodes.into_iter().map(|n| n.id).collect::<Vec<_>>();
    let near = ix.graph_around(&p("archive/Idea.md").key(), 1).unwrap();
    assert_eq!(near.edges, vec![GraphEdge { from: "Home.md".into(), to: "archive/Idea.md".into() }]);
    assert_eq!(ids(near), ["archive/Idea.md", "Home.md"]);
    let wide = ix.graph_around(&p("archive/Idea.md").key(), 2).unwrap();
    assert_eq!(ids(wide), ["archive/Idea.md", "Home.md", "projects/Idea.md", "projects/Plan.md", "?missing note"]);
    let alone = ix.graph_around(&p("archive/deep/Idea.md").key(), 9).unwrap();
    assert_eq!(ids(alone), ["archive/deep/Idea.md"]);
    assert!(ix.graph_around(&p("nope.md").key(), 1).is_none());
}
```

`search.rs`:

```rust
fn fixture() -> Index {
    let n = |p: &str, c: &str| (NotePath::parse(p).unwrap(), c.to_string());
    Index::from_notes(vec![
        n("Rust Notes.md", "Rust is fast.\r\nrust RUST rust rust rust rust\r\nOther line"),
        n("cooking.md", "I like rust-colored pans.\nAlso pasta."),
        n("empty.md", ""),
    ])
}

#[test]
fn should_rank_title_matches_first() {
    let hits = fixture().search("rust", 20);
    let got: Vec<_> = hits.iter().map(|h| (h.path.as_str(), h.title.as_str(), h.score)).collect();
    assert_eq!(got, [("Rust Notes.md", "Rust Notes", 15), ("cooking.md", "cooking", 1)]);
    assert_eq!(hits[0].snippets, vec![
        Snippet { line: 1, text: "Rust is fast.".into() },
        Snippet { line: 2, text: "rust RUST rust rust rust rust".into() },
    ]);
}

#[test]
fn should_require_every_term() {
    let hits = fixture().search("  RUST   pasta ", 20);
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].path.as_str(), hits[0].score), ("cooking.md", 2));
}

#[test]
fn should_match_on_title_alone() {
    let hits = fixture().search("NOTES", 20);
    assert_eq!((hits[0].path.as_str(), hits[0].score, hits[0].snippets.len()), ("Rust Notes.md", 10, 0));
}

#[test]
fn should_return_nothing_for_blank_query() {
    assert!(fixture().search("", 20).is_empty());
    assert!(fixture().search("   ", 20).is_empty());
}

#[test]
fn should_clamp_limit() {
    assert_eq!(fixture().search("rust", 1).len(), 1);
    assert_eq!(fixture().search("rust", 0).len(), 1);
}

#[test]
fn should_cap_snippets_at_three_lines_of_200_chars() {
    let body = format!("{}\nx\nñ rust\nrust 2\nrust 3\nrust 4", "rust ".repeat(100));
    let ix = Index::from_notes(vec![(NotePath::parse("a.md").unwrap(), body)]);
    let hit = &ix.search("rust", 20)[0];
    assert_eq!(hit.snippets.len(), 3);
    assert_eq!(hit.snippets[0].text.chars().count(), 200);
    assert_eq!(hit.snippets[1], Snippet { line: 3, text: "ñ rust".into() });
}
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-core graph:: search::`
Expected: compilation fails, `graph` not found.

- [ ] **Step 3: Implement `graph`, `graph_around`, `search`**

- Graph: spec section 6.3. Ghost id is `?` + `normalize_target(target)`; its label is the target as written by the first link (in key order of the source notes) that mentions it.
- Neighbourhood: breadth-first over the whole graph treating edges as undirected; keep the nodes within `depth` steps and the edges whose both ends are kept; same ordering as the whole graph.
- Search: spec section 6.5. Title is the note's stem. A snippet line is the line with any `\r` removed, trimmed, cut with `truncate_chars(…, 200)`.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sb-core
git commit -m "feat(core): add link graph and full-text search"
```

---

### Task 6: Agent-facing operations

**Files:**
- Create: `crates/sb-core/src/ops.rs`
- Modify: `crates/sb-core/src/lib.rs`

**Interfaces:**
- Consumes: everything public from Tasks 1–5.
- Produces (`sb_core::ops`), the single entry point for the CLI, the MCP server and later the GUI. Every `&str` path is untrusted and is parsed here:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct NoteSummary { pub path: NotePath, pub title: String }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct NoteList { pub notes: Vec<NoteSummary>, pub ignored: usize }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct WriteOutcome { pub path: NotePath, pub created: bool }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct DeleteOutcome { pub path: NotePath, pub trashed: bool }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct RenameOutcome { pub from: NotePath, pub to: NotePath }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct SearchResults { pub hits: Vec<SearchHit> }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct LinksReport { pub outgoing: Vec<OutLink>, pub backlinks: Vec<Backlink> }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)] pub struct TagList { pub tags: Vec<TagCount> }

pub fn list_notes(vault: &Vault, folder: Option<&str>) -> Result<NoteList>;
pub fn read_note(vault: &Vault, path: &str) -> Result<String>;
pub fn write_note(vault: &Vault, path: &str, content: &str) -> Result<WriteOutcome>;
pub fn append_to_note(vault: &Vault, path: &str, content: &str) -> Result<WriteOutcome>;
pub fn delete_note(vault: &Vault, path: &str) -> Result<DeleteOutcome>;
pub fn rename_note(vault: &Vault, from: &str, to: &str) -> Result<RenameOutcome>;
pub fn search_notes(vault: &Vault, query: &str, limit: Option<usize>) -> Result<SearchResults>;
pub fn get_links(vault: &Vault, path: &str) -> Result<LinksReport>;
pub fn get_graph(vault: &Vault, around: Option<&str>, depth: Option<u8>) -> Result<Graph>;
pub fn list_tags(vault: &Vault) -> Result<TagList>;
```

- [ ] **Step 1: Write the failing tests** in `ops.rs`

Each test uses a temporary vault seeded through `Vault::write` with `Home.md` = `"[[Idea]] #start"`, `Projects/Idea.md` = `"Idea body [[Home]]"`.

```rust
#[test] fn should_list_notes_with_optional_folder_filter()
    // list_notes(None): paths ["Home.md", "Projects/Idea.md"], titles ["Home", "Idea"], ignored 0
    // list_notes(Some("projects")) and Some("Projects/"): ["Projects/Idea.md"];  Some("proj"): []  (a folder, not a text prefix)
#[test] fn should_read_note_ignoring_case()
    // read_note("projects/idea.md") == "Idea body [[Home]]"
#[test] fn should_write_to_existing_spelling_when_case_differs()
    // write_note("PROJECTS/IDEA.md", "new") == WriteOutcome { path: "Projects/Idea.md", created: false }
    // afterwards list_notes(None) still has 2 notes and read_note("Projects/Idea.md") == "new"
#[test] fn should_create_note_when_writing_new_path()
    // write_note("new/n.md", "x") == WriteOutcome { path: "new/n.md", created: true }
#[test] fn should_append_to_note()
    // append_to_note("home.md", "more") -> created false; read_note("Home.md") == "[[Idea]] #start\nmore"
    // append_to_note("log.md", "first") -> created true
#[test] fn should_delete_note_to_trash()
    // delete_note("home.md") == DeleteOutcome { path: "Home.md", trashed: true }; then read_note("Home.md") is Err(NotFound)
#[test] fn should_rename_note()
    // rename_note("home.md", "Start/Home.md") == RenameOutcome { from: "Home.md", to: "Start/Home.md" }
    // rename_note("Projects/Idea.md", "start/home.md") is Err(AlreadyExists)
    // rename_note("missing.md", "x.md") is Err(NotFound)
#[test] fn should_report_links_both_ways()
    // get_links("projects/idea.md"): outgoing[0].resolved == Some("Home.md"); backlinks == [Backlink { from: "Home.md", line: 1, context: "[[Idea]] #start" }]
#[test] fn should_return_graph_whole_and_around()
    // get_graph(None, None): 2 nodes, 2 edges
    // get_graph(Some("home.md"), Some(1)): 2 nodes;  get_graph(Some("missing.md"), None) is Err(NotFound)
#[test] fn should_search_and_list_tags()
    // search_notes("body", None).hits[0].path == "Projects/Idea.md";  list_tags().tags == [TagCount { tag: "start", count: 1 }]
#[test] fn should_reject_paths_that_leave_the_vault()
    // for raw in ["../x.md", "/etc/x.md", "a/../../x.md", ".secondbrain/trash/x.md", "C:\\x.md", "x.txt"]:
    //   read_note, write_note(.., "x"), append_to_note(.., "x"), delete_note, get_links and rename_note(raw, "ok.md"), rename_note("Home.md", raw)
    //   all return Err(CoreError::InvalidPath { .. }), and the directory tree outside and inside the vault is unchanged
#[test] fn should_report_not_found_for_missing_notes()
    // read_note("nope.md"), delete_note("nope.md"), get_links("nope.md") are Err(NotFound("nope.md"))
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-core ops::`
Expected: compilation fails, `ops` not found.

- [ ] **Step 3: Implement the operations**

- Parse each path with `NotePath::parse`, mapping the error to `CoreError::InvalidPath { path: raw, source }`.
- `read`, `write`, `append`, `delete`, `rename` use `Vault::locate` to find the stored spelling and act on it; a new note uses the spelling given. `rename` to a key held by a different note is `AlreadyExists`; a case-only rename of the same note is allowed.
- `list`, `search`, `links`, `graph`, `tags` build an `Index` from the vault and answer from it. `get_links` and `get_graph(around)` return `NotFound` when the note is not in the index.
- `list_notes` folder filter: trim a trailing `/`, lowercase, then keep notes whose key starts with that folder followed by `/`. `None` or an empty folder keeps everything. `ignored` is the number of ignored files.
- `search_notes` uses `DEFAULT_SEARCH_LIMIT` when no limit is given; `get_graph` uses depth 1.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sb-core
git commit -m "feat(core): add agent-facing note operations"
```

---

### Task 7: The `sb` command line

**Files:**
- Create: `crates/sb-cli/Cargo.toml`, `crates/sb-cli/src/lib.rs`, `crates/sb-cli/src/main.rs`, `crates/sb-cli/src/cli.rs`, `crates/sb-cli/src/commands.rs`, `crates/sb-cli/src/render.rs`
- Create: `crates/sb-cli/tests/cli.rs`
- Modify: root `Cargo.toml` (add `clap = { version = "4", features = ["derive", "env"] }` to `[workspace.dependencies]`), `README.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `sb_core::ops::*`, `sb_core::{Vault, CoreError}`.
- Produces: the binary `sb` (`[[bin]] name = "sb"`) and the library `sb_cli` (`lib.rs` declares the public modules `cli`, `commands`, `render`; `main.rs` only parses and calls `commands::run`), so integration tests and Task 8 can reach them:

```rust
// cli.rs
#[derive(clap::Parser)] pub struct Cli { /* --vault (global, env SB_VAULT), --json (global), command */ }
#[derive(clap::Subcommand)] pub enum Command { List { folder: Option<String> }, Read { path }, Write { path, content: Option<String> }, Append { path, content: Option<String> },
    Delete { path }, Rename { from, to }, Search { query, limit: Option<usize> }, Links { path }, Graph { around: Option<String>, depth: Option<u8> }, Tags, Mcp }
// commands.rs
pub fn run(cli: Cli) -> Result<(), CliError>;      // CliError wraps CoreError and std::io::Error
pub fn open_vault(vault: Option<std::path::PathBuf>) -> Result<Vault, CliError>;  // --vault / SB_VAULT, else the current directory
```

Command line, exactly as spec section 7.2 (the `sync` subcommands arrive in milestone M2). `list` additionally accepts `--folder <FOLDER>`. `Mcp` returns an error «not available yet» until Task 8.

- [ ] **Step 1: Write the failing tests** in `crates/sb-cli/tests/cli.rs`

Helper: `sb(vault: &Path, args: &[&str], stdin: Option<&str>) -> (i32, String, String)` runs `env!("CARGO_BIN_EXE_sb")` with `--vault <vault>` first, with `SB_VAULT` removed from the environment, feeds stdin, and returns exit code, stdout and stderr.

```rust
#[test] fn should_write_from_flag_and_from_stdin()
    // sb write a.md --content "hello [[b]]" -> exit 0;  sb write b.md with stdin "piped\r\n" -> exit 0
    // sb read a.md -> stdout == "hello [[b]]" (byte for byte, no added newline);  sb read b.md -> stdout == "piped\r\n"
#[test] fn should_list_notes_as_lines_and_as_json()
    // sb list -> stdout == "a.md\nb.md\n"
    // sb --json list -> stdout parses to {"notes":[{"path":"a.md","title":"a"},{"path":"b.md","title":"b"}],"ignored":0}
    // sb list --json (flag after the subcommand) gives the same JSON
#[test] fn should_print_json_for_every_query_command()
    // sb --json links b.md   -> .backlinks[0].from == "a.md"
    // sb --json graph        -> 2 nodes, 1 edge {"from":"a.md","to":"b.md"}
    // sb --json graph --around a.md --depth 1 -> 2 nodes
    // sb --json search hello -> .hits[0].path == "a.md"
    // sb --json tags         -> {"tags":[]}
    // sb --json write c.md --content x -> {"path":"c.md","created":true}
    // sb --json append c.md --content y -> {"path":"c.md","created":false}
    // sb --json rename c.md d/e.md -> {"from":"c.md","to":"d/e.md"}
    // sb --json delete d/e.md -> {"path":"d/e.md","trashed":true}
#[test] fn should_print_readable_text_without_json()
    // sb search hello -> stdout contains "a.md" and "hello [[b]]"
    // sb links b.md   -> stdout contains "a.md"
    // sb graph        -> stdout contains "a.md -> b.md"
#[test] fn should_exit_1_with_message_on_operation_error()
    // sb read ../x.md    -> exit 1, stdout empty, stderr contains "invalid note path"
    // sb read missing.md -> exit 1, stderr contains "note not found: missing.md"
    // sb --vault <tmp>/nope list -> exit 1, stderr contains "is not a directory"
#[test] fn should_exit_2_on_usage_error()
    // sb frobnicate -> exit 2
#[test] fn should_use_sb_vault_environment_variable()
    // run without --vault and with SB_VAULT=<vault>: sb list -> "a.md\nb.md\n"
#[test] fn should_summarise_ignored_files_on_stderr()
    // a file "bin.md" holding bytes [0xff]: sb list -> stdout lists the valid notes, stderr contains "1 file ignored" and "bin.md"
#[test] fn should_handle_empty_stdin()
    // sb write empty.md with empty stdin -> exit 0 and the note exists with length 0
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-cli`
Expected: compilation fails, the `sb` binary does not exist.

- [ ] **Step 3: Implement the CLI**

- `main.rs`: parse with `clap`; call `commands::run`; on error print `error: <message>` to stderr and return exit code 1. `clap` itself exits with 2 on usage errors.
- `commands.rs`: one small function per command; each calls the matching `sb_core::ops` function and hands the result to `render`. Content for `write`/`append` comes from `--content`, otherwise all of stdin, unmodified.
- `render.rs`: with `--json`, `serde_json::to_string(&value)` plus a newline, for every command. Without it: `read` prints the note bytes exactly; `list` one path per line (ignored files summarised on stderr as `1 file ignored:` or `<n> files ignored:` followed by one `path (reason)` line each); the others in a compact readable layout of the implementer's choosing that satisfies the assertions above.
- `README.md`: add a «Command line» section listing every command with a one-line description and two examples. `CHANGELOG.md`: add the CLI under Unreleased → Added.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sb-cli README.md CHANGELOG.md
git commit -m "feat(cli): add sb command line for note operations"
```

---

### Task 8: MCP stdio server

**Files:**
- Create: `crates/sb-cli/src/mcp.rs`, `crates/sb-cli/tests/mcp.rs`
- Modify: `crates/sb-cli/Cargo.toml`, `crates/sb-cli/src/lib.rs`, `crates/sb-cli/src/commands.rs`, `README.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: `sb_core::ops::*`, `sb_core::Vault`; `Command::Mcp`, `open_vault` (Task 7).
- Produces:

```rust
// mcp.rs
#[derive(Clone)] pub struct NotesServer { /* vault: std::sync::Arc<Vault> */ }
impl NotesServer { pub fn new(vault: Vault) -> Self; }
pub fn serve_stdio(vault: Vault) -> Result<(), CliError>;   // builds a current-thread tokio runtime and blocks until stdin closes
```

`lib.rs` gains the public module `mcp`, so integration tests can use `sb_cli::mcp::NotesServer`.

Dependencies, verified by a spike on 2026-11-15 against `rmcp` 3.10.0:

```toml
[dependencies]
rmcp = { version = "3.10", default-features = false, features = ["server", "macros", "transport-io"] }
schemars = "1"
tokio = { version = "1", features = ["rt", "macros", "io-std"] }

[dev-dependencies]
rmcp = { version = "3.10", default-features = false, features = ["server", "macros", "client"] }
tokio = { version = "1", features = ["rt", "macros", "io-util"] }
```

Verified `rmcp` 3.10 shape, to be confirmed against the crate source in `~/.cargo/registry/src/` while implementing:

```rust
use rmcp::{ServerHandler, ServiceExt, handler::server::wrapper::Parameters, tool, tool_handler, tool_router, transport::stdio};

#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct ReadNoteParams {
    /// Doc comments on fields become the parameter descriptions in the tool's JSON schema.
    pub path: String,
}

#[tool_router]
impl NotesServer {
    #[tool(description = "…")]
    async fn read_note(&self, Parameters(p): Parameters<ReadNoteParams>) -> Result<String, String> { /* Err(text) becomes a result with isError: true */ }
}

#[tool_handler(name = "second-brain", version = "0.1.0", instructions = "…")]
impl ServerHandler for NotesServer {}

// serving:            NotesServer::new(vault).serve(stdio()).await?.waiting().await?;
// in-process client:  let (server_io, client_io) = tokio::io::duplex(64 * 1024);
//                     server: NotesServer::new(vault).serve(server_io)      client: ().serve(client_io)
//                     client.list_all_tools(), client.call_tool(CallToolRequestParams::new("read_note").with_arguments(rmcp::object!({ "path": "a.md" })))
//                     result.is_error == Some(true|false); result.content[0].as_text().unwrap().text
```

- [ ] **Step 1: Write the failing tests** in `crates/sb-cli/tests/mcp.rs`

Helper `connect(vault_dir) -> client` wires a `NotesServer` and a client through `tokio::io::duplex`, as above. Helper `call(&client, tool, json_args) -> (bool /* is_error */, String /* first text block */)`. The vault is seeded with `a.md` = `"hello [[b]] #t"` and `B.md` = `"world"`.

```rust
#[tokio::test] async fn should_list_exactly_the_ten_tools()
    // sorted names == ["append_to_note","delete_note","get_graph","get_links","list_notes","list_tags","read_note","rename_note","search_notes","write_note"]
    // every tool has a non-empty description; read_note's input schema has properties.path.description non-empty and required == ["path"]
#[tokio::test] async fn should_report_server_info_and_instructions()
    // server name == "second-brain"; instructions contain "Markdown" and "[[" and ".md"
#[tokio::test] async fn should_read_note_as_raw_text()
    // call read_note {path:"a.md"} -> (false, "hello [[b]] #t")
#[tokio::test] async fn should_return_json_matching_core_operations()
    // for each of: list_notes {}, list_notes {folder:"x"}, search_notes {query:"hello"}, get_links {path:"b.md"}, get_graph {}, get_graph {around:"a.md", depth:1}, list_tags {}
    //   -> is_error false and the text parses to the same serde_json::Value as serde_json::to_value(the matching sb_core::ops call)
#[tokio::test] async fn should_write_append_rename_and_delete()
    // write_note {path:"n/new.md", content:"x"} -> {"path":"n/new.md","created":true}; the file exists with content "x"
    // append_to_note {path:"n/new.md", content:"y"} -> {"path":"n/new.md","created":false}; content "x\ny"
    // rename_note {from:"n/new.md", to:"m.md"} -> {"from":"n/new.md","to":"m.md"}
    // delete_note {path:"m.md"} -> {"path":"m.md","trashed":true}; one file in .secondbrain/trash
#[tokio::test] async fn should_return_tool_error_for_bad_paths_and_missing_notes()
    // read_note {path:"../secret.md"} -> is_error true, text contains "invalid note path"
    // read_note {path:"nope.md"}      -> is_error true, text contains "note not found"
    // write_note {path:".secondbrain/x.md", content:"x"} -> is_error true; nothing created under .secondbrain
#[tokio::test] async fn should_return_tool_error_for_wrong_or_missing_arguments()
    // read_note {} and read_note {path: 5} and search_notes {query:"x", limit:"many"} -> is_error true; a following read_note {path:"a.md"} still succeeds
#[tokio::test] async fn should_carry_large_notes()
    // write_note with 2 MiB of "a…" then read_note returns the same 2 MiB

#[test] fn should_speak_only_protocol_on_stdout()
    // spawn env!("CARGO_BIN_EXE_sb") --vault <vault> mcp; write these lines to stdin, then close it:
    //   {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}
    //   {"jsonrpc":"2.0","method":"notifications/initialized"}
    //   {"jsonrpc":"2.0","id":2,"method":"tools/list"}
    //   {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_note","arguments":{"path":"a.md"}}}
    // exit code 0; every stdout line parses as a JSON object with "jsonrpc":"2.0"; ids 1, 2 and 3 each have a "result";
    // result of id 1 has protocolVersion "2025-06-18" and serverInfo.name "second-brain"; result of id 3 has content[0].text "hello [[b]] #t"
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p sb-cli --test mcp`
Expected: compilation fails, `sb_cli::mcp` not found.

- [ ] **Step 3: Implement the server**

- One parameter struct per tool with the arguments of spec section 7.3; optional arguments are `Option<…>`. Every field has a doc comment: it is what the agent reads.
- One `#[tool]` method per tool. Each calls the matching `sb_core::ops` function inside `tokio::task::spawn_blocking` (the operations do blocking file I/O and may scan the vault), maps `CoreError` to `Err(error.to_string())`, and returns `read_note`'s text as is and every other result as `serde_json::to_string(&outcome)`.
- Tool descriptions say what the tool does, what it returns and any side effect («moves the note to the vault's trash», «replaces the whole note»), in one or two sentences.
- `instructions`: three or four sentences. What a vault is (a folder of Markdown notes); the path format (vault-relative, `/` separators, ends with `.md`, case-insensitive); that `[[Note Name]]` links notes and `get_links`/`get_graph` navigate them; that `write_note` replaces the whole note while `append_to_note` adds to it.
- `serve_stdio`: `tokio::runtime::Builder::new_current_thread().enable_all().build()`, then serve on `rmcp::transport::stdio()` and wait. Print nothing to stdout. `Command::Mcp` calls it.
- `README.md`: add «Use with AI agents» — what the MCP server offers, the tool table, and how to register it: `claude mcp add second-brain -- sb --vault /path/to/vault mcp` for Claude Code, and the equivalent `mcpServers` JSON block for Claude Desktop. `CHANGELOG.md`: add the MCP server under Unreleased → Added.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.lock crates/sb-cli README.md CHANGELOG.md
git commit -m "feat(cli): add MCP stdio server exposing note operations"
```
