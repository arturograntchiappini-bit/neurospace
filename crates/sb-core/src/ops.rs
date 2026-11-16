//! Agent-facing note operations: the one entry point of the CLI, the MCP server and the GUI.
//!
//! Every `&str` path is untrusted: it is parsed with `NotePath::parse` before any disk access.
//! An existing note is found with `Vault::locate` and acted on under the spelling stored on
//! disk; a note that `locate` does not see is new and is made with `Vault::create`.

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::graph::Graph;
use crate::index::{Backlink, Index, OutLink, TagCount};
use crate::path::NotePath;
use crate::search::{DEFAULT_SEARCH_LIMIT, SearchHit};
use crate::vault::Vault;

const DEFAULT_GRAPH_DEPTH: u8 = 1;

/// A note in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteSummary {
    /// Path as stored on disk.
    pub path: NotePath,
    /// The file name without `.md`.
    pub title: String,
}

/// The notes of a vault or folder, plus how many files the scan ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteList {
    /// Notes ordered by note key.
    pub notes: Vec<NoteSummary>,
    /// Number of ignored files.
    pub ignored: usize,
}

/// Result of `write_note` and `append_to_note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WriteOutcome {
    /// The path acted on: the stored spelling for an existing note, else as given.
    pub path: NotePath,
    /// True when the note did not exist before.
    pub created: bool,
}

/// Result of `delete_note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteOutcome {
    /// The stored spelling of the deleted note.
    pub path: NotePath,
    /// Always true: deleting moves the note to the trash.
    pub trashed: bool,
}

/// Result of `rename_note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenameOutcome {
    /// The stored spelling of the moved note.
    pub from: NotePath,
    /// The destination as given.
    pub to: NotePath,
}

/// Result of `search_notes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResults {
    /// Best match first.
    pub hits: Vec<SearchHit>,
}

/// Result of `get_links`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinksReport {
    /// Links of the note in document order.
    pub outgoing: Vec<OutLink>,
    /// Lines of other notes that link to it.
    pub backlinks: Vec<Backlink>,
}

/// Result of `list_tags`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagList {
    /// Tags with the number of notes carrying them.
    pub tags: Vec<TagCount>,
}

fn parse_path(raw: &str) -> Result<NotePath> {
    NotePath::parse(raw).map_err(|source| CoreError::InvalidPath {
        path: raw.to_owned(),
        source,
    })
}

/// Makes a `NotFound` from the vault carry the path as the caller wrote it.
fn as_written(raw: &str) -> impl FnOnce(CoreError) -> CoreError + '_ {
    move |error| match error {
        CoreError::NotFound(_) => CoreError::NotFound(raw.to_owned()),
        other => other,
    }
}

/// Parses `raw` and returns the stored spelling of that note.
fn locate_existing(vault: &Vault, raw: &str) -> Result<NotePath> {
    vault
        .locate(&parse_path(raw)?)?
        .ok_or_else(|| CoreError::NotFound(raw.to_owned()))
}

/// The lowercase key prefix (`projects/`) that selects a folder; `None` selects the whole vault.
fn folder_prefix(folder: Option<&str>) -> Result<Option<String>> {
    let raw = folder.unwrap_or_default();
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed.is_empty() {
        return Ok(None);
    }
    let probe =
        NotePath::parse(&format!("{trimmed}/x.md")).map_err(|source| CoreError::InvalidPath {
            path: raw.to_owned(),
            source,
        })?;
    Ok(probe.key().as_str().strip_suffix("x.md").map(str::to_owned))
}

/// Lists the notes, optionally only those inside `folder` (a folder, not a text prefix;
/// matched ignoring case, one trailing `/` allowed, empty means the whole vault).
///
/// # Errors
/// `InvalidPath` when `folder` could not be a folder of notes (`..`, a backslash, a hidden
/// folder), or whatever loading the vault returns.
pub fn list_notes(vault: &Vault, folder: Option<&str>) -> Result<NoteList> {
    let prefix = folder_prefix(folder)?;
    let index = Index::build(vault)?;
    let notes = index
        .notes()
        .filter(|entry| {
            prefix
                .as_ref()
                .is_none_or(|p| entry.path.key().as_str().starts_with(p))
        })
        .map(|entry| NoteSummary {
            path: entry.path.clone(),
            title: entry.path.stem().to_owned(),
        })
        .collect();
    Ok(NoteList {
        notes,
        ignored: index.ignored().len(),
    })
}

/// Reads a note, matching its path case-insensitively.
///
/// # Errors
/// `InvalidPath`, `NotFound`, or a read error from the vault.
pub fn read_note(vault: &Vault, path: &str) -> Result<String> {
    let stored = locate_existing(vault, path)?;
    vault.read(&stored).map_err(as_written(path))
}

/// Replaces the note at `path`, or creates it when no spelling of it exists.
///
/// # Errors
/// `InvalidPath`, or a write error from the vault.
pub fn write_note(vault: &Vault, path: &str, content: &str) -> Result<WriteOutcome> {
    let parsed = parse_path(path)?;
    let Some(stored) = vault.locate(&parsed)? else {
        let adopted = vault.adopt_folders(&parsed)?;
        vault.create(&adopted, content)?;
        return Ok(WriteOutcome {
            path: adopted,
            created: true,
        });
    };
    let created = vault.write(&stored, content)?;
    Ok(WriteOutcome {
        path: stored,
        created,
    })
}

/// Appends to the note at `path`, creating it when no spelling of it exists.
///
/// # Errors
/// `InvalidPath`, or a write error from the vault.
pub fn append_to_note(vault: &Vault, path: &str, content: &str) -> Result<WriteOutcome> {
    let parsed = parse_path(path)?;
    // A missing note is created by `Vault::append`, which never replaces anything.
    let target = match vault.locate(&parsed)? {
        Some(stored) => stored,
        None => vault.adopt_folders(&parsed)?,
    };
    let created = vault.append(&target, content)?;
    Ok(WriteOutcome {
        path: target,
        created,
    })
}

/// Moves a note to the vault's trash.
///
/// # Errors
/// `InvalidPath`, `NotFound`, or a file system error.
pub fn delete_note(vault: &Vault, path: &str) -> Result<DeleteOutcome> {
    let stored = locate_existing(vault, path)?;
    vault.delete(&stored).map_err(as_written(path))?;
    Ok(DeleteOutcome {
        path: stored,
        trashed: true,
    })
}

/// Moves a note; a case-only rename of the same note is allowed.
///
/// # Errors
/// `InvalidPath` for either path, `NotFound` for the source, `AlreadyExists` when the
/// destination is held by a different note.
pub fn rename_note(vault: &Vault, from: &str, to: &str) -> Result<RenameOutcome> {
    let requested = parse_path(to)?;
    let source = locate_existing(vault, from)?;
    let destination = vault.adopt_folders(&requested)?;
    vault
        .rename(&source, &destination)
        .map_err(as_written(from))?;
    Ok(RenameOutcome {
        from: source,
        to: destination,
    })
}

/// Full-text search; `limit` defaults to `DEFAULT_SEARCH_LIMIT`.
///
/// # Errors
/// Whatever loading the vault returns.
pub fn search_notes(vault: &Vault, query: &str, limit: Option<usize>) -> Result<SearchResults> {
    let hits = Index::build(vault)?.search(query, limit.unwrap_or(DEFAULT_SEARCH_LIMIT));
    Ok(SearchResults { hits })
}

/// Outgoing links and backlinks of a note.
///
/// # Errors
/// `InvalidPath`, or `NotFound` when the note is not in the index.
pub fn get_links(vault: &Vault, path: &str) -> Result<LinksReport> {
    let key = parse_path(path)?.key();
    let index = Index::build(vault)?;
    if index.get(&key).is_none() {
        return Err(CoreError::NotFound(path.to_owned()));
    }
    Ok(LinksReport {
        outgoing: index.outgoing(&key),
        backlinks: index.backlinks(&key),
    })
}

/// The whole link graph, or the part within `depth` (default 1) steps of `around`.
///
/// # Errors
/// `InvalidPath`, or `NotFound` when `around` is not in the index.
pub fn get_graph(vault: &Vault, around: Option<&str>, depth: Option<u8>) -> Result<Graph> {
    let target = around
        .map(|raw| parse_path(raw).map(|path| (raw, path.key())))
        .transpose()?;
    let index = Index::build(vault)?;
    let Some((raw, key)) = target else {
        return Ok(index.graph());
    };
    index
        .graph_around(&key, depth.unwrap_or(DEFAULT_GRAPH_DEPTH))
        .ok_or_else(|| CoreError::NotFound(raw.to_owned()))
}

/// Every tag with the number of notes carrying it.
///
/// # Errors
/// Whatever loading the vault returns.
pub fn list_tags(vault: &Vault) -> Result<TagList> {
    Ok(TagList {
        tags: Index::build(vault)?.tags(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::{Backlink, CoreError, NotePath, TagCount, Vault};

    fn p(raw: &str) -> NotePath {
        NotePath::parse(raw).unwrap()
    }

    /// Vault inside `<tmp>/vault`, so "outside the vault" is observable in `<tmp>`.
    fn seeded() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("vault");
        fs::create_dir(&root).unwrap();
        let vault = Vault::open(root).unwrap();
        vault.write(&p("Home.md"), "[[Idea]] #start").unwrap();
        vault
            .write(&p("Projects/Idea.md"), "Idea body [[Home]]")
            .unwrap();
        (tmp, vault)
    }

    fn paths(list: &NoteList) -> Vec<&str> {
        list.notes.iter().map(|n| n.path.as_str()).collect()
    }

    fn snapshot(dir: &Path) -> Vec<String> {
        let mut entries = Vec::new();
        collect(dir, dir, &mut entries);
        entries.sort();
        entries
    }

    fn collect(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.push(format!("{relative}/"));
                collect(base, &path, out);
            } else {
                out.push(format!("{relative}={}", fs::read_to_string(&path).unwrap()));
            }
        }
    }

    #[test]
    fn should_list_notes_with_optional_folder_filter() {
        let (_tmp, vault) = seeded();
        let all = list_notes(&vault, None).unwrap();
        assert_eq!(paths(&all), ["Home.md", "Projects/Idea.md"]);
        let titles: Vec<&str> = all.notes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["Home", "Idea"]);
        assert_eq!(all.ignored, 0);
        for folder in ["projects", "Projects/"] {
            let list = list_notes(&vault, Some(folder)).unwrap();
            assert_eq!(paths(&list), ["Projects/Idea.md"]);
        }
        assert!(list_notes(&vault, Some("proj")).unwrap().notes.is_empty());
    }

    #[test]
    fn should_reject_an_invalid_folder_filter() {
        let (_tmp, vault) = seeded();
        for folder in ["Projects\\Sub", "../x", "./projects", ".secondbrain"] {
            let result = list_notes(&vault, Some(folder));
            assert!(
                matches!(&result, Err(CoreError::InvalidPath { path, .. }) if path == folder),
                "{folder}: {result:?}"
            );
        }
    }

    #[test]
    fn should_accept_a_trailing_slash_in_the_folder_filter() {
        let (_tmp, vault) = seeded();
        for folder in ["Projects/", "projects/", ""] {
            let list = list_notes(&vault, Some(folder)).unwrap();
            let expected: &[&str] = if folder.is_empty() {
                &["Home.md", "Projects/Idea.md"]
            } else {
                &["Projects/Idea.md"]
            };
            assert_eq!(paths(&list), expected, "{folder}");
        }
    }

    #[test]
    fn should_read_note_ignoring_case() {
        let (_tmp, vault) = seeded();
        assert_eq!(
            read_note(&vault, "projects/idea.md").unwrap(),
            "Idea body [[Home]]"
        );
    }

    #[test]
    fn should_write_to_existing_spelling_when_case_differs() {
        let (_tmp, vault) = seeded();
        let outcome = write_note(&vault, "PROJECTS/IDEA.md", "new").unwrap();
        assert_eq!(
            outcome,
            WriteOutcome {
                path: p("Projects/Idea.md"),
                created: false
            }
        );
        assert_eq!(list_notes(&vault, None).unwrap().notes.len(), 2);
        assert_eq!(read_note(&vault, "Projects/Idea.md").unwrap(), "new");
        let on_disk = fs::read_to_string(vault.root().join("Projects/Idea.md")).unwrap();
        assert_eq!(on_disk, "new");
    }

    #[test]
    fn should_create_note_when_writing_new_path() {
        let (_tmp, vault) = seeded();
        let outcome = write_note(&vault, "new/n.md", "x").unwrap();
        assert_eq!(
            outcome,
            WriteOutcome {
                path: p("new/n.md"),
                created: true
            }
        );
        assert_eq!(
            fs::read_to_string(vault.root().join("new/n.md")).unwrap(),
            "x"
        );
    }

    #[test]
    fn should_append_to_note() {
        let (_tmp, vault) = seeded();
        let existing = append_to_note(&vault, "home.md", "more").unwrap();
        assert_eq!(existing.path, p("Home.md"));
        assert!(!existing.created);
        assert_eq!(
            read_note(&vault, "Home.md").unwrap(),
            "[[Idea]] #start\nmore"
        );
        let fresh = append_to_note(&vault, "log.md", "first").unwrap();
        assert_eq!(fresh.path, p("log.md"));
        assert!(fresh.created);
    }

    #[test]
    fn should_delete_note_to_trash() {
        let (_tmp, vault) = seeded();
        let outcome = delete_note(&vault, "home.md").unwrap();
        assert_eq!(
            outcome,
            DeleteOutcome {
                path: p("Home.md"),
                trashed: true
            }
        );
        assert!(matches!(
            read_note(&vault, "Home.md"),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn should_rename_note() {
        let (_tmp, vault) = seeded();
        let outcome = rename_note(&vault, "home.md", "Start/Home.md").unwrap();
        assert_eq!(
            outcome,
            RenameOutcome {
                from: p("Home.md"),
                to: p("Start/Home.md")
            }
        );
        assert!(matches!(
            rename_note(&vault, "Projects/Idea.md", "start/home.md"),
            Err(CoreError::AlreadyExists(_))
        ));
        assert!(matches!(
            rename_note(&vault, "missing.md", "x.md"),
            Err(CoreError::NotFound(p)) if p == "missing.md"
        ));
        assert!(!vault.root().join("Home.md").exists());
        let moved = fs::read_to_string(vault.root().join("Start/Home.md")).unwrap();
        assert_eq!(moved, "[[Idea]] #start");
    }

    #[test]
    fn should_rename_note_when_only_the_case_changes() {
        let (_tmp, vault) = seeded();
        let outcome = rename_note(&vault, "Home.md", "home.md").unwrap();
        assert_eq!(outcome.to, p("home.md"));
        let list = list_notes(&vault, None).unwrap();
        assert_eq!(paths(&list), ["home.md", "Projects/Idea.md"]);
        assert_eq!(read_note(&vault, "HOME.md").unwrap(), "[[Idea]] #start");
    }

    #[test]
    fn should_reject_bad_around_path_without_scanning_the_vault() {
        let (tmp, vault) = seeded();
        fs::remove_dir_all(tmp.path().join("vault")).unwrap();
        assert!(matches!(
            get_graph(&vault, Some("../x.md"), None),
            Err(CoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn should_report_links_both_ways() {
        let (_tmp, vault) = seeded();
        let report = get_links(&vault, "projects/idea.md").unwrap();
        assert_eq!(report.outgoing[0].resolved, Some(p("Home.md")));
        assert_eq!(
            report.backlinks,
            [Backlink {
                from: p("Home.md"),
                line: 1,
                context: "[[Idea]] #start".to_owned()
            }]
        );
    }

    #[test]
    fn should_return_graph_whole_and_around() {
        let (_tmp, vault) = seeded();
        let whole = get_graph(&vault, None, None).unwrap();
        assert_eq!((whole.nodes.len(), whole.edges.len()), (2, 2));
        let around = get_graph(&vault, Some("home.md"), Some(1)).unwrap();
        assert_eq!(around.nodes.len(), 2);
        assert!(matches!(
            get_graph(&vault, Some("missing.md"), None),
            Err(CoreError::NotFound(_))
        ));
    }

    #[test]
    fn should_search_and_list_tags() {
        let (_tmp, vault) = seeded();
        let results = search_notes(&vault, "body", None).unwrap();
        assert_eq!(results.hits[0].path, p("Projects/Idea.md"));
        assert_eq!(
            list_tags(&vault).unwrap().tags,
            [TagCount {
                tag: "start".to_owned(),
                count: 1
            }]
        );
    }

    #[test]
    fn should_reject_paths_that_leave_the_vault() {
        let (tmp, vault) = seeded();
        let before = snapshot(tmp.path());
        let raws = [
            "../x.md",
            "/etc/x.md",
            "a/../../x.md",
            ".secondbrain/trash/x.md",
            "C:\\x.md",
            "x.txt",
        ];
        for raw in raws {
            let results = [
                read_note(&vault, raw).map(drop),
                write_note(&vault, raw, "x").map(drop),
                append_to_note(&vault, raw, "x").map(drop),
                delete_note(&vault, raw).map(drop),
                get_links(&vault, raw).map(drop),
                rename_note(&vault, raw, "ok.md").map(drop),
                rename_note(&vault, "Home.md", raw).map(drop),
                get_graph(&vault, Some(raw), None).map(drop),
            ];
            for result in results {
                assert!(
                    matches!(result, Err(CoreError::InvalidPath { .. })),
                    "{raw}: {result:?}"
                );
            }
        }
        assert_eq!(snapshot(tmp.path()), before);
    }

    fn dirs_named_projects(vault: &Vault) -> usize {
        fs::read_dir(vault.root())
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.path().is_dir())
            .filter(|entry| entry.file_name().to_string_lossy().to_lowercase() == "projects")
            .count()
    }

    #[test]
    fn should_adopt_existing_folder_spelling_when_creating() {
        let (_tmp, vault) = seeded();
        let written = write_note(&vault, "projects/new.md", "x").unwrap();
        assert_eq!(written.path, p("Projects/new.md"));
        assert!(written.created);
        let appended = append_to_note(&vault, "PROJECTS/log.md", "x").unwrap();
        assert_eq!(appended.path, p("Projects/log.md"));
        assert!(appended.created);
        let renamed = rename_note(&vault, "Home.md", "projects/Home2.md").unwrap();
        assert_eq!(renamed.to, p("Projects/Home2.md"));
        assert_eq!(dirs_named_projects(&vault), 1);
    }

    #[test]
    fn should_report_not_found_when_a_directory_has_a_note_name() {
        let (_tmp, vault) = seeded();
        fs::create_dir(vault.root().join("x.md")).unwrap();
        for result in [
            read_note(&vault, "X.md").map(drop),
            delete_note(&vault, "X.md").map(drop),
        ] {
            assert!(matches!(result, Err(CoreError::NotFound(p)) if p == "X.md"));
        }
        assert!(matches!(
            rename_note(&vault, "X.md", "y.md"),
            Err(CoreError::NotFound(p)) if p == "X.md"
        ));
    }

    #[test]
    fn should_report_not_found_for_missing_notes() {
        let (_tmp, vault) = seeded();
        let missing = [
            read_note(&vault, "nope.md").map(drop),
            delete_note(&vault, "nope.md").map(drop),
            get_links(&vault, "nope.md").map(drop),
        ];
        for result in missing {
            assert!(matches!(result, Err(CoreError::NotFound(p)) if p == "nope.md"));
        }
    }
}
