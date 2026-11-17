//! In-memory index of a vault's notes and the rule that decides where a link points (spec 6.2).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::markdown::{self, Link, is_attachment_target};
use crate::path::{NoteKey, NotePath};
use crate::vault::{Ignored, Vault};

const MARKDOWN_SUFFIX: &str = ".md";
const MAX_CONTEXT_CHARS: usize = 200;

/// One note held by the index, with what was extracted from its text.
#[derive(Debug, Clone)]
pub struct NoteEntry {
    /// Path as written on disk.
    pub path: NotePath,
    /// Full text of the note.
    pub content: String,
    /// Links in document order.
    pub links: Vec<Link>,
    /// Tags lowercased, sorted, unique.
    pub tags: Vec<String>,
}

/// A link together with the note it points to, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutLink {
    /// The link as extracted from the note.
    #[serde(flatten)]
    pub link: Link,
    /// The note the link resolves to; absent when unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<NotePath>,
}

/// A line of another note that links to a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Backlink {
    /// The note holding the link.
    pub from: NotePath,
    /// 1-based line of the link.
    pub line: u32,
    /// That line, trimmed, at most 200 characters.
    pub context: String,
}

/// How many notes carry a tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagCount {
    /// Tag, lowercase, without `#`.
    pub tag: String,
    /// Number of notes carrying the tag.
    pub count: usize,
}

/// The notes of a vault, keyed for link resolution.
#[derive(Debug, Default)]
pub struct Index {
    notes: BTreeMap<NoteKey, NoteEntry>,
    by_stem: HashMap<String, BTreeSet<NoteKey>>,
    ignored: Vec<Ignored>,
}

impl Index {
    /// Loads every note of `vault` and indexes it.
    ///
    /// # Errors
    /// Whatever `Vault::load` returns.
    pub fn build(vault: &Vault) -> Result<Index> {
        let loaded = vault.load()?;
        let mut index = Index::from_notes(loaded.notes);
        index.ignored = loaded.ignored;
        Ok(index)
    }

    /// Indexes the given notes; a later note replaces an earlier one with the same key.
    pub fn from_notes(notes: Vec<(NotePath, String)>) -> Index {
        let mut index = Index::default();
        for (path, content) in notes {
            index.upsert(path, content);
        }
        index
    }

    /// Adds a note or replaces the one with the same key.
    pub fn upsert(&mut self, path: NotePath, content: String) {
        let parsed = markdown::parse(&content);
        let key = path.key();
        self.by_stem
            .entry(stem_of_key(&key).to_owned())
            .or_default()
            .insert(key.clone());
        let entry = NoteEntry {
            path,
            content,
            links: parsed.links,
            tags: parsed.tags,
        };
        self.notes.insert(key, entry);
    }

    /// Removes a note; false when it was not indexed.
    pub fn remove(&mut self, key: &NoteKey) -> bool {
        if self.notes.remove(key).is_none() {
            return false;
        }
        let stem = stem_of_key(key);
        if let Some(keys) = self.by_stem.get_mut(stem) {
            keys.remove(key);
            if keys.is_empty() {
                self.by_stem.remove(stem);
            }
        }
        true
    }

    /// The note with this key.
    pub fn get(&self, key: &NoteKey) -> Option<&NoteEntry> {
        self.notes.get(key)
    }

    /// Every note, ordered by key.
    pub fn notes(&self) -> impl Iterator<Item = &NoteEntry> {
        self.notes.values()
    }

    /// Number of notes.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// True when no note is indexed.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Files the last `build` skipped.
    pub fn ignored(&self) -> &[Ignored] {
        &self.ignored
    }

    /// The note that `target`, written in note `from`, points to (spec 6.2).
    pub fn resolve(&self, target: &str, from: &NotePath) -> Option<&NotePath> {
        let key = self.resolve_key(target, from.key().as_str())?;
        self.notes.get(key).map(|entry| &entry.path)
    }

    /// A leading `/` anchors the target at the vault root and a leading `./` at the source
    /// folder; both match exactly or not at all. Other targets follow rules 1 and 2 of spec 6.2.
    fn resolve_key(&self, target: &str, from_key: &str) -> Option<&NoteKey> {
        let slashed = target.trim().replace('\\', "/");
        if let Some(rest) = slashed.strip_prefix('/') {
            return self.find_exact(&strip_extension_lowercase(rest));
        }
        if let Some(rest) = slashed.strip_prefix("./") {
            let relative = join_relative(folder_of(from_key), &strip_extension_lowercase(rest))?;
            return self.find_exact(&relative);
        }
        let target = normalize_target(&slashed);
        let stem = target.rsplit('/').next().unwrap_or_default();
        let candidates = self.by_stem.get(stem)?;
        if target.contains('/') {
            pick_by_path(&target, from_key, candidates)
        } else {
            best_candidate(candidates.iter(), from_key)
        }
    }

    /// The note whose key is `path` plus `.md`.
    fn find_exact(&self, path: &str) -> Option<&NoteKey> {
        let stem = path.rsplit('/').next().unwrap_or_default();
        find_key(self.by_stem.get(stem)?, &format!("{path}{MARKDOWN_SUFFIX}"))
    }

    /// Links of a note in document order, each with its resolution; empty if the note is unknown.
    pub fn outgoing(&self, key: &NoteKey) -> Vec<OutLink> {
        let Some(entry) = self.notes.get(key) else {
            return Vec::new();
        };
        entry
            .links
            .iter()
            .map(|link| OutLink {
                link: link.clone(),
                resolved: self.resolve(&link.target, &entry.path).cloned(),
            })
            .collect()
    }

    /// Lines linking to a note, once per (note, line), ordered by note key then line.
    pub fn backlinks(&self, key: &NoteKey) -> Vec<Backlink> {
        let mut backlinks = Vec::new();
        for entry in self.notes.iter().filter(|(k, _)| *k != key).map(|(_, e)| e) {
            let lines: BTreeSet<u32> = entry
                .links
                .iter()
                .filter(|link| {
                    self.resolve(&link.target, &entry.path)
                        .map(NotePath::key)
                        .as_ref()
                        == Some(key)
                })
                .map(|link| link.line)
                .collect();
            backlinks.extend(lines.into_iter().map(|line| Backlink {
                from: entry.path.clone(),
                line,
                context: line_context(&entry.content, line),
            }));
        }
        backlinks
    }

    /// Tags with the number of notes carrying them: count descending, then tag ascending.
    pub fn tags(&self) -> Vec<TagCount> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for tag in self.notes.values().flat_map(|entry| &entry.tags) {
            *counts.entry(tag).or_default() += 1;
        }
        let mut tags: Vec<TagCount> = counts
            .into_iter()
            .map(|(tag, count)| TagCount {
                tag: tag.to_owned(),
                count,
            })
            .collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
        tags
    }
}

/// The longest prefix of `s` with at most `max` characters.
pub(crate) fn truncate_chars(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(end, _)| &s[..end])
}

/// The note that clicking an unresolved link `target`, written in note `from`, creates (spec 6.6).
///
/// # Errors
/// `InvalidInput` when the target is an attachment or a `./` path leaving the vault,
/// `InvalidPath` when the result is not a valid note path.
pub fn new_note_path(target: &str, from: &NotePath) -> Result<NotePath> {
    if is_attachment_target(target) {
        return Err(CoreError::InvalidInput("attachments are not notes".into()));
    }
    let slashed = target.trim().replace('\\', "/");
    let mut path = match slashed.strip_prefix("./") {
        Some("") => String::new(),
        Some(rest) => join_relative(folder_of(from.as_str()), rest)
            .ok_or_else(|| CoreError::InvalidInput("the link leaves the vault".into()))?,
        None => slashed.strip_prefix('/').unwrap_or(&slashed).to_owned(),
    };
    if !path.to_ascii_lowercase().ends_with(MARKDOWN_SUFFIX) {
        path.push_str(MARKDOWN_SUFFIX);
    }
    NotePath::parse(&path).map_err(|source| CoreError::InvalidPath {
        path: target.to_owned(),
        source,
    })
}

/// Spec 6.2 normalisation: trim, `\` to `/`, drop a leading `./` or `/` and a trailing `.md`, lowercase.
pub(crate) fn normalize_target(target: &str) -> String {
    let slashed = target.trim().replace('\\', "/");
    let without_dot = slashed.strip_prefix("./").unwrap_or(&slashed);
    let without_dot = without_dot.strip_prefix('/').unwrap_or(without_dot);
    strip_extension_lowercase(without_dot)
}

fn strip_extension_lowercase(text: &str) -> String {
    let split = text.len().saturating_sub(MARKDOWN_SUFFIX.len());
    let without_extension = match text.get(split..) {
        Some(tail) if tail.eq_ignore_ascii_case(MARKDOWN_SUFFIX) => &text[..split],
        _ => text,
    };
    without_extension.to_lowercase()
}

fn find_key<'a>(candidates: &'a BTreeSet<NoteKey>, wanted: &str) -> Option<&'a NoteKey> {
    candidates.iter().find(|key| key.as_str() == wanted)
}

fn stem_of_key(key: &NoteKey) -> &str {
    let last = key.as_str().rsplit('/').next().unwrap_or_default();
    last.strip_suffix(MARKDOWN_SUFFIX).unwrap_or(last)
}

fn folder_of(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// Rules 1.1 to 1.3 for a target that contains `/`.
fn pick_by_path<'a>(
    target: &str,
    from_key: &str,
    candidates: &'a BTreeSet<NoteKey>,
) -> Option<&'a NoteKey> {
    let find = |wanted: &str| find_key(candidates, wanted);
    let exact = format!("{target}{MARKDOWN_SUFFIX}");
    find(&exact)
        .or_else(|| {
            let relative = join_relative(folder_of(from_key), target)?;
            find(&format!("{relative}{MARKDOWN_SUFFIX}"))
        })
        .or_else(|| {
            let suffix = format!("/{exact}");
            let matching = candidates
                .iter()
                .filter(|key| key.as_str().ends_with(&suffix));
            best_candidate(matching, from_key)
        })
}

/// `target` resolved against `folder`; `None` when `..` climbs out of the vault.
fn join_relative(folder: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for part in target.split('/') {
        match part {
            ".." => {
                parts.pop()?;
            }
            "." => {}
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Tie-break: same folder as the source, then fewest components, then smallest key.
fn best_candidate<'a>(
    candidates: impl Iterator<Item = &'a NoteKey>,
    from_key: &str,
) -> Option<&'a NoteKey> {
    let from_folder = folder_of(from_key);
    candidates.min_by_key(|key| {
        let text = key.as_str();
        (
            folder_of(text) != from_folder,
            text.split('/').count(),
            *key,
        )
    })
}

fn line_context(content: &str, line: u32) -> String {
    let index = usize::try_from(line).map_or(0, |n| n.saturating_sub(1));
    let text = content.lines().nth(index).unwrap_or_default();
    truncate_chars(text.trim(), MAX_CONTEXT_CHARS).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Index {
        let n = |p: &str, c: &str| (NotePath::parse(p).unwrap(), c.to_string());
        Index::from_notes(vec![
            n(
                "Home.md",
                "[[Idea]] [[projects/Plan]] [[Missing Note]] [[idea|again]]",
            ),
            n("projects/Idea.md", "#work [[Home]]"),
            n("projects/Plan.md", "[[Idea]] [[../Home]]"),
            n("archive/Idea.md", "#work #old"),
            n("archive/deep/Idea.md", ""),
        ])
    }

    fn p(s: &str) -> NotePath {
        NotePath::parse(s).unwrap()
    }

    #[test]
    fn should_resolve_links_by_spec_rules() {
        let ix = fixture();
        let r = |t: &str, from: &str| ix.resolve(t, &p(from)).map(|x| x.as_str().to_string());
        assert_eq!(r("Idea", "Home.md").as_deref(), Some("archive/Idea.md")); // tie on depth -> smallest key
        assert_eq!(
            r("Idea", "projects/Plan.md").as_deref(),
            Some("projects/Idea.md")
        ); // same folder first
        assert_eq!(
            r("projects/Plan", "Home.md").as_deref(),
            Some("projects/Plan.md")
        ); // exact path
        assert_eq!(r("../Home", "projects/Plan.md").as_deref(), Some("Home.md")); // relative to the source folder
        assert_eq!(
            r("deep/Idea", "Home.md").as_deref(),
            Some("archive/deep/Idea.md")
        ); // suffix match
        assert_eq!(
            r("IDEA.md", "archive/Idea.md").as_deref(),
            Some("archive/Idea.md")
        ); // case and extension ignored
        assert_eq!(
            r(" .\\projects\\plan ", "Home.md").as_deref(),
            Some("projects/Plan.md")
        );
        assert_eq!(r("Missing Note", "Home.md"), None);
        assert_eq!(r("../../etc/passwd", "Home.md"), None);
    }

    #[test]
    fn should_list_outgoing_links_with_resolution() {
        let ix = fixture();
        let out = ix.outgoing(&p("home.md").key());
        let resolved: Vec<_> = out
            .iter()
            .map(|o| o.resolved.as_ref().map(|x| x.as_str()))
            .collect();
        assert_eq!(
            resolved,
            [
                Some("archive/Idea.md"),
                Some("projects/Plan.md"),
                None,
                Some("archive/Idea.md")
            ]
        );
    }

    #[test]
    fn should_list_backlinks_once_per_line() {
        let ix = fixture();
        assert_eq!(
            ix.backlinks(&p("archive/Idea.md").key()),
            vec![Backlink {
                from: p("Home.md"),
                line: 1,
                context: "[[Idea]] [[projects/Plan]] [[Missing Note]] [[idea|again]]".into(),
            }]
        );
        let home: Vec<_> = ix
            .backlinks(&p("Home.md").key())
            .into_iter()
            .map(|b| b.from.as_str().to_string())
            .collect();
        assert_eq!(home, ["projects/Idea.md", "projects/Plan.md"]);
    }

    #[test]
    fn should_count_tags_by_note() {
        let tags = fixture().tags();
        assert_eq!(
            tags,
            vec![
                TagCount {
                    tag: "work".into(),
                    count: 2
                },
                TagCount {
                    tag: "old".into(),
                    count: 1
                }
            ]
        );
    }

    #[test]
    fn should_follow_upsert_and_remove() {
        let mut ix = fixture();
        assert!(ix.remove(&p("projects/Idea.md").key()));
        assert!(!ix.remove(&p("projects/Idea.md").key()));
        assert_eq!(
            ix.resolve("Idea", &p("projects/Plan.md"))
                .map(|x| x.as_str()),
            Some("archive/Idea.md")
        );
        ix.upsert(p("Missing Note.md"), "#new".into());
        assert_eq!(
            ix.resolve("missing note", &p("Home.md"))
                .map(|x| x.as_str()),
            Some("Missing Note.md")
        );
        ix.upsert(p("Home.md"), "no links".into());
        assert!(ix.outgoing(&p("Home.md").key()).is_empty());
        assert_eq!(ix.len(), 5);
    }

    #[test]
    fn should_replace_entry_when_upserting_with_different_case() {
        let mut ix = fixture();
        ix.upsert(p("HOME.md"), "x".into());
        assert_eq!(ix.len(), 5);
        assert_eq!(
            ix.get(&p("home.md").key()).unwrap().path.as_str(),
            "HOME.md"
        );
    }

    #[test]
    fn should_truncate_on_character_boundaries() {
        assert_eq!(truncate_chars("añb", 2), "añ");
        assert_eq!(truncate_chars("🧠🧠🧠", 2), "🧠🧠");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }

    #[test]
    fn should_resolve_quickly_in_a_large_vault() {
        let notes: Vec<_> = (0..5000)
            .map(|i| {
                (
                    p(&format!("f{}/n{i}.md", i % 50)),
                    format!("[[n{}]] [[n{}]]", (i + 1) % 5000, (i + 7) % 5000),
                )
            })
            .collect();
        let ix = Index::from_notes(notes);
        let started = std::time::Instant::now();
        let resolved = ix
            .notes()
            .flat_map(|n| ix.outgoing(&n.path.key()))
            .filter(|o| o.resolved.is_some())
            .count();
        assert_eq!(resolved, 10_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "resolution must not scan every note per link"
        );
    }

    fn index_of(notes: &[(&str, &str)]) -> Index {
        Index::from_notes(
            notes
                .iter()
                .map(|(n, c)| (p(n), (*c).to_string()))
                .collect(),
        )
    }

    fn resolved(ix: &Index, target: &str, from: &str) -> Option<String> {
        ix.resolve(target, &p(from)).map(|x| x.as_str().to_string())
    }

    #[test]
    fn should_not_list_a_note_as_its_own_backlink() {
        let ix = index_of(&[("a.md", "[[a]] [[b]]"), ("b.md", "[[a]]")]);
        let from: Vec<_> = ix
            .backlinks(&p("a.md").key())
            .into_iter()
            .map(|b| b.from.as_str().to_string())
            .collect();
        assert_eq!(from, ["b.md"]);
        assert_eq!(ix.outgoing(&p("a.md").key()).len(), 2);
        assert!(ix.outgoing(&p("a.md").key())[0].resolved.is_some());
    }

    #[test]
    fn should_treat_leading_slash_as_vault_root() {
        let ix = fixture();
        assert_eq!(
            resolved(&ix, "/projects/Plan", "archive/Idea.md").as_deref(),
            Some("projects/Plan.md")
        );
        assert_eq!(
            resolved(&ix, "/Home", "projects/Plan.md").as_deref(),
            Some("Home.md")
        );
        assert_eq!(normalize_target("/A/B.md"), "a/b");
        assert_eq!(normalize_target("./x"), normalize_target("/x"));
    }

    #[test]
    fn should_anchor_slash_and_dot_slash_targets_without_fallback() {
        let ix = index_of(&[
            ("Home.md", ""),
            ("projects/Home.md", ""),
            ("projects/Plan.md", ""),
            ("archive/x.md", ""),
        ]);
        let r = |t: &str, from: &str| resolved(&ix, t, from);
        assert_eq!(r("/Home", "projects/Plan.md").as_deref(), Some("Home.md"));
        assert_eq!(
            r("Home", "projects/Plan.md").as_deref(),
            Some("projects/Home.md")
        );
        assert_eq!(
            r("./Home", "projects/Plan.md").as_deref(),
            Some("projects/Home.md")
        );
        assert_eq!(r("./Home", "archive/x.md"), None);
        assert_eq!(r("/Plan", "Home.md"), None);
        assert_eq!(r("./Plan", "archive/x.md"), None);
    }

    #[test]
    fn should_resolve_anchored_paths_exactly() {
        let ix = index_of(&[("a/x/n.md", ""), ("b/x/n.md", ""), ("b/s.md", "")]);
        let r = |t: &str| resolved(&ix, t, "b/s.md");
        assert_eq!(r("/x/n"), None);
        assert_eq!(r("/a/x/n").as_deref(), Some("a/x/n.md"));
        assert_eq!(r("./x/n").as_deref(), Some("b/x/n.md"));
        assert_eq!(r("./../a/x/n").as_deref(), Some("a/x/n.md"));
        assert_eq!(r("./../../x/n"), None);
    }

    #[test]
    fn should_resolve_nothing_for_degenerate_anchors() {
        let ix = index_of(&[("x.md", ""), ("s.md", "")]);
        for target in ["/", "./", "//x", ".//x"] {
            assert_eq!(resolved(&ix, target, "s.md"), None, "target: {target:?}");
        }
    }

    #[test]
    fn should_prefer_relative_match_over_suffix_match() {
        let ix = index_of(&[("a/x/n.md", ""), ("b/x/n.md", ""), ("b/s.md", "")]);
        assert_eq!(resolved(&ix, "x/n", "b/s.md").as_deref(), Some("b/x/n.md"));
        assert_eq!(resolved(&ix, "x/n", "c/s.md").as_deref(), Some("a/x/n.md"));
        assert_eq!(resolved(&ix, "../../x/n", "b/s.md"), None);
    }

    #[test]
    fn should_prefer_same_folder_over_shallower_note() {
        let ix = index_of(&[("n.md", ""), ("deep/er/n.md", ""), ("deep/er/s.md", "")]);
        assert_eq!(
            resolved(&ix, "n", "deep/er/s.md").as_deref(),
            Some("deep/er/n.md")
        );
    }

    #[test]
    fn should_normalize_and_reject_degenerate_targets() {
        assert_eq!(normalize_target("X.MD"), "x");
        assert_eq!(normalize_target(".md"), "");
        assert_eq!(normalize_target(""), "");
        let ix = fixture();
        assert_eq!(resolved(&ix, "", "Home.md"), None);
        assert_eq!(resolved(&ix, ".md", "Home.md"), None);
    }

    #[test]
    fn should_place_new_note_by_spec_6_6_rules() {
        let from = p("a/b/src.md");
        let at = |t: &str| new_note_path(t, &from).unwrap().as_str().to_owned();
        assert_eq!(at("Idea"), "Idea.md");
        assert_eq!(at("sub/Idea"), "sub/Idea.md");
        assert_eq!(at("/top"), "top.md");
        assert_eq!(at("./near"), "a/b/near.md");
        assert_eq!(at("./../up"), "a/up.md");
        assert_eq!(at(" Idea.MD "), "Idea.MD");
        assert_eq!(at("x\\y"), "x/y.md");
    }

    #[test]
    fn should_refuse_new_note_when_target_leaves_vault_or_is_invalid() {
        let from = p("a/src.md");
        let refused = |t: &str| new_note_path(t, &from).unwrap_err();
        assert!(matches!(refused("./../../x"), CoreError::InvalidInput(_)));
        assert!(matches!(refused("photo.png"), CoreError::InvalidInput(_)));
        for target in ["a:b", "CON", ""] {
            assert!(
                matches!(refused(target), CoreError::InvalidPath { .. }),
                "target: {target:?}"
            );
        }
    }

    #[test]
    fn should_keep_ignored_files_when_building_from_a_vault() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "[[b]]").unwrap();
        std::fs::write(dir.path().join("b.md"), "").unwrap();
        std::fs::write(dir.path().join("bad.md"), [0xff, 0xfe, 0xfd]).unwrap();
        let ix = Index::build(&Vault::open(dir.path()).unwrap()).unwrap();
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.ignored().len(), 1);
        assert_eq!(ix.ignored()[0].path, "bad.md");
    }
}
