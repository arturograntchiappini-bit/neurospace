//! In-memory index of a vault's notes and the rule that decides where a link points (spec 6.2).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::error::Result;
use crate::markdown::{self, Link};
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
        let target = normalize_target(target);
        let stem = target.rsplit('/').next().unwrap_or_default();
        let candidates = self.by_stem.get(stem)?;
        let from_key = from.key();
        let chosen = if target.contains('/') {
            pick_by_path(&target, from_key.as_str(), candidates)
        } else {
            best_candidate(candidates.iter(), from_key.as_str())
        };
        chosen.and_then(|key| self.notes.get(key)).map(|e| &e.path)
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
        for entry in self.notes.values() {
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

/// Spec 6.2 normalisation: trim, `\` to `/`, drop a leading `./` and a trailing `.md`, lowercase.
pub(crate) fn normalize_target(target: &str) -> String {
    let slashed = target.trim().replace('\\', "/");
    let without_dot = slashed.strip_prefix("./").unwrap_or(&slashed);
    let split = without_dot.len().saturating_sub(MARKDOWN_SUFFIX.len());
    let without_extension = match without_dot.get(split..) {
        Some(tail) if tail.eq_ignore_ascii_case(MARKDOWN_SUFFIX) => &without_dot[..split],
        _ => without_dot,
    };
    without_extension.to_lowercase()
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
    let find = |wanted: &str| candidates.iter().find(|key| key.as_str() == wanted);
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
            started.elapsed() < std::time::Duration::from_secs(2),
            "resolution must not scan every note per link"
        );
    }
}
