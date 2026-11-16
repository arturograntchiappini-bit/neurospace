//! Full-text search over the notes held in memory (spec 6.5).

use serde::Serialize;

use crate::index::{Index, NoteEntry, truncate_chars};
use crate::path::NotePath;

/// Number of hits returned when the caller states no limit.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;
/// Largest number of hits `search` returns.
pub const MAX_SEARCH_LIMIT: usize = 100;
const TITLE_TERM_SCORE: u32 = 10;
const MAX_COUNTED_OCCURRENCES: usize = 5;
const MAX_SNIPPETS: usize = 3;
const MAX_SNIPPET_CHARS: usize = 200;

/// A line of a note that contains a search term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    /// 1-based line number.
    pub line: u32,
    /// The line, trimmed, at most 200 characters.
    pub text: String,
}

/// A note that matches every term of a query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    /// The matching note.
    pub path: NotePath,
    /// File stem of the note.
    pub title: String,
    /// 10 per term in the title plus content occurrences (at most 5 per term).
    pub score: u32,
    /// Up to 3 first lines containing a term.
    pub snippets: Vec<Snippet>,
}

impl Index {
    /// Notes containing every whitespace-separated term, best first.
    /// `limit` is clamped to `1..=MAX_SEARCH_LIMIT`; a blank query finds nothing.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = self
            .notes()
            .filter_map(|entry| match_note(entry, &terms))
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        hits.truncate(limit.clamp(1, MAX_SEARCH_LIMIT));
        hits
    }
}

fn match_note(entry: &NoteEntry, terms: &[&str]) -> Option<SearchHit> {
    let title = entry.path.stem();
    let title_lower = title.to_lowercase();
    let content_lower = entry.content.to_lowercase();
    let mut score = 0;
    for term in terms {
        let in_title = title_lower.contains(term);
        let occurrences: u32 = content_lower
            .matches(term)
            .take(MAX_COUNTED_OCCURRENCES)
            .map(|_| 1)
            .sum();
        if !in_title && occurrences == 0 {
            return None;
        }
        score += occurrences + if in_title { TITLE_TERM_SCORE } else { 0 };
    }
    Some(SearchHit {
        path: entry.path.clone(),
        title: title.to_owned(),
        score,
        snippets: snippets(&entry.content, terms),
    })
}

fn snippets(content: &str, terms: &[&str]) -> Vec<Snippet> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let lower = line.to_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .take(MAX_SNIPPETS)
        .map(|(index, line)| Snippet {
            line: u32::try_from(index + 1).unwrap_or(u32::MAX),
            text: truncate_chars(line.replace('\r', "").trim(), MAX_SNIPPET_CHARS).to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::NotePath;

    fn fixture() -> Index {
        let n = |p: &str, c: &str| (NotePath::parse(p).unwrap(), c.to_string());
        Index::from_notes(vec![
            n(
                "Rust Notes.md",
                "Rust is fast.\r\nrust RUST rust rust rust rust\r\nOther line",
            ),
            n("cooking.md", "I like rust-colored pans.\nAlso pasta."),
            n("empty.md", ""),
        ])
    }

    #[test]
    fn should_rank_title_matches_first() {
        let hits = fixture().search("rust", 20);
        let got: Vec<_> = hits
            .iter()
            .map(|h| (h.path.as_str(), h.title.as_str(), h.score))
            .collect();
        assert_eq!(
            got,
            [
                ("Rust Notes.md", "Rust Notes", 15),
                ("cooking.md", "cooking", 1)
            ]
        );
        assert_eq!(
            hits[0].snippets,
            vec![
                Snippet {
                    line: 1,
                    text: "Rust is fast.".into()
                },
                Snippet {
                    line: 2,
                    text: "rust RUST rust rust rust rust".into()
                },
            ]
        );
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
        assert_eq!(
            (hits[0].path.as_str(), hits[0].score, hits[0].snippets.len()),
            ("Rust Notes.md", 10, 0)
        );
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
        assert_eq!(
            hit.snippets[1],
            Snippet {
                line: 3,
                text: "ñ rust".into()
            }
        );
    }
}
