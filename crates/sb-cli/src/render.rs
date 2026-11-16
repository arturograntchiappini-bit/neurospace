//! Turns operation results into text or JSON on stdout.

use std::io::{self, Write};

use sb_core::ops::{
    DeleteOutcome, LinksReport, NoteList, RenameOutcome, SearchResults, TagList, WriteOutcome,
};
use sb_core::{Graph, IgnoreReason, Ignored};
use serde::Serialize;

use crate::commands::CliError;

/// Prints `value` as one JSON line when `json` is set, otherwise as the text `to_text` makes of it.
///
/// # Errors
/// A write error on stdout, or a serialisation error.
pub fn emit<T: Serialize>(
    json: bool,
    value: &T,
    to_text: impl FnOnce(&T) -> String,
) -> Result<(), CliError> {
    let output = if json {
        serde_json::to_string(value)? + "\n"
    } else {
        to_text(value)
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn lines(items: impl IntoIterator<Item = String>) -> String {
    items.into_iter().map(|item| item + "\n").collect()
}

/// One path per line.
pub fn note_list(list: &NoteList) -> String {
    lines(list.notes.iter().map(|note| note.path.to_string()))
}

/// The stderr summary of files a scan left out, or nothing when there are none.
pub fn ignored_summary(ignored: &[Ignored]) -> String {
    let header = match ignored {
        [] => return String::new(),
        [_] => "1 file ignored:".to_owned(),
        many => format!("{} files ignored:", many.len()),
    };
    let details = ignored
        .iter()
        .map(|file| format!("  {} ({})", file.path, reason_text(file.reason)));
    lines(std::iter::once(header).chain(details))
}

fn reason_text(reason: IgnoreReason) -> &'static str {
    match reason {
        IgnoreReason::InvalidPath => "invalid note path",
        IgnoreReason::CaseCollision => "differs only in case from another note",
        IgnoreReason::NotUtf8 => "not valid UTF-8",
        IgnoreReason::TooLarge => "too large",
        IgnoreReason::Unreadable => "unreadable",
    }
}

/// `created <path>` or `replaced <path>`.
pub fn written(outcome: &WriteOutcome) -> String {
    let verb = if outcome.created {
        "created"
    } else {
        "replaced"
    };
    format!("{verb} {}\n", outcome.path)
}

/// `created <path>` or `appended to <path>`.
pub fn appended(outcome: &WriteOutcome) -> String {
    let verb = if outcome.created {
        "created"
    } else {
        "appended to"
    };
    format!("{verb} {}\n", outcome.path)
}

/// `deleted <path> (moved to trash)`.
pub fn deleted(outcome: &DeleteOutcome) -> String {
    format!("deleted {} (moved to trash)\n", outcome.path)
}

/// `<from> -> <to>`.
pub fn renamed(outcome: &RenameOutcome) -> String {
    format!("{} -> {}\n", outcome.from, outcome.to)
}

/// Each hit as `path (score)` followed by its matching lines.
pub fn search(results: &SearchResults) -> String {
    lines(results.hits.iter().flat_map(|hit| {
        let snippets = hit
            .snippets
            .iter()
            .map(|snippet| format!("  {}: {}", snippet.line, snippet.text));
        std::iter::once(format!("{} ({})", hit.path, hit.score)).chain(snippets)
    }))
}

/// Outgoing links, then backlinks, each as `line: text`.
pub fn links(report: &LinksReport) -> String {
    let outgoing = report.outgoing.iter().map(|out| {
        let link = &out.link;
        let heading = link
            .heading
            .as_ref()
            .map_or(String::new(), |h| format!("#{h}"));
        let destination = out
            .resolved
            .as_ref()
            .map_or("(unresolved)".to_owned(), |path| format!("-> {path}"));
        format!("  {}: {}{heading} {destination}", link.line, link.target)
    });
    let backlinks = report
        .backlinks
        .iter()
        .map(|back| format!("  {}:{}: {}", back.from, back.line, back.context));
    lines(
        std::iter::once(format!("outgoing ({}):", report.outgoing.len()))
            .chain(outgoing)
            .chain(std::iter::once(format!(
                "backlinks ({}):",
                report.backlinks.len()
            )))
            .chain(backlinks),
    )
}

/// The nodes, then the edges as `from -> to`.
pub fn graph(graph: &Graph) -> String {
    let nodes = graph.nodes.iter().map(|node| match node.exists {
        true => format!("  {}", node.id),
        false => format!("  {} (missing)", node.label),
    });
    let edges = graph
        .edges
        .iter()
        .map(|edge| format!("  {} -> {}", edge.from, edge.to));
    lines(
        std::iter::once(format!("nodes ({}):", graph.nodes.len()))
            .chain(nodes)
            .chain(std::iter::once(format!("edges ({}):", graph.edges.len())))
            .chain(edges),
    )
}

/// One `#tag (count)` per line.
pub fn tags(list: &TagList) -> String {
    lines(
        list.tags
            .iter()
            .map(|entry| format!("#{} ({})", entry.tag, entry.count)),
    )
}
