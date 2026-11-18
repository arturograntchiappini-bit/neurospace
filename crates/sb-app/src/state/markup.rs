//! Markdown colouring for the editor and wikilink rewriting for the preview. No UI types.

use std::collections::HashSet;
use std::ops::Range;

/// What a stretch of the note's text is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    /// Ordinary text.
    Plain,
    /// A `#` heading line.
    Heading,
    /// A wikilink or Markdown link.
    Link,
    /// An inline code span or a fenced code block.
    Code,
}

/// The scheme of the destinations that `wikilinks_to_markdown` writes.
pub const NOTE_LINK_SCHEME: &str = "sb-note:";

/// Splits `text` into contiguous byte ranges that cover all of it, each with its kind.
///
/// shortcut: an inline code span never crosses a line and a fence closes on any line starting
/// with the same three characters; upgrade to `pulldown-cmark` offsets if that ever misleads.
pub fn highlight(text: &str) -> Vec<(Range<usize>, SpanKind)> {
    let mut spans = Vec::new();
    for (range, is_code) in segments(text) {
        if is_code {
            push_span(&mut spans, range, SpanKind::Code);
        } else {
            plain_spans(text, range, &mut spans);
        }
    }
    spans
}

/// Rewrites wikilinks outside code into Markdown links to `sb-note:` destinations.
///
/// `[[T|A]]` becomes the link `A` to `T`; a heading in `T` is dropped; an embed `![[x]]` and a
/// link to the note itself (`[[#H]]`) become plain text.
pub fn wikilinks_to_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (range, is_code) in segments(text) {
        let chunk = &text[range];
        if is_code {
            out.push_str(chunk);
            continue;
        }
        for line in chunk.split_inclusive('\n') {
            rewrite_line(line, &mut out);
        }
    }
    out
}

/// The `sb-note:` destination that names `target`, percent-encoded.
pub fn note_link_destination(target: &str) -> String {
    let mut dest = String::from(NOTE_LINK_SCHEME);
    for byte in target.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
            dest.push(char::from(byte));
        } else {
            dest.push_str(&format!("%{byte:02X}"));
        }
    }
    dest
}

/// The note a link destination names, or `None` for any other destination: a `sb-note:`
/// destination (a rewritten wikilink) or a Markdown link to a `.md` file (spec 6.1).
pub fn note_link_target(dest: &str) -> Option<String> {
    match dest.strip_prefix(NOTE_LINK_SCHEME) {
        Some(encoded) => decode_note_target(encoded),
        None => sb_core::markdown_note_target(dest),
    }
}

fn decode_note_target(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|&b| char::from(b).to_digit(16));
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            decoded.push(u8::try_from(hex(at + 1)? * 16 + hex(at + 2)?).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Every destination in `markdown` that names a note, each once, in order of appearance.
pub fn note_link_destinations(markdown: &str) -> Vec<String> {
    let mut destinations = link_destinations(markdown);
    destinations.retain(|dest| note_link_target(dest).is_some());
    destinations
}

/// The destinations of the links in `markdown` that must not open from a note.
pub fn blocked_link_destinations(markdown: &str) -> Vec<String> {
    let mut destinations = link_destinations(markdown);
    destinations.retain(|dest| !is_openable(dest));
    destinations
}

/// The options the preview renderer (the Markdown viewer crate, 0.26.0) parses with: its backend
/// crate, `src/pulldown.rs`, `parser_options`, line 543. The link hooks come from the same parse
/// the preview draws, so every link it makes clickable is seen here.
const PREVIEW_OPTIONS: pulldown_cmark::Options = pulldown_cmark::Options::ENABLE_TABLES
    .union(pulldown_cmark::Options::ENABLE_TASKLISTS)
    .union(pulldown_cmark::Options::ENABLE_STRIKETHROUGH)
    .union(pulldown_cmark::Options::ENABLE_FOOTNOTES)
    .union(pulldown_cmark::Options::ENABLE_DEFINITION_LIST);

/// The destination of every link and image in `markdown`, each once, in order of appearance.
fn link_destinations(markdown: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    pulldown_cmark::Parser::new_ext(markdown, PREVIEW_OPTIONS)
        .filter_map(|event| match event {
            pulldown_cmark::Event::Start(
                pulldown_cmark::Tag::Link { dest_url, .. }
                | pulldown_cmark::Tag::Image { dest_url, .. },
            ) => Some(dest_url.into_string()),
            _ => None,
        })
        .filter(|dest| seen.insert(dest.clone()))
        .collect()
}

/// Only web links open outside the app, and note links open inside it; `sb-note:` is the exact
/// scheme our own rewrite writes.
fn is_openable(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || note_link_target(dest).is_some()
}

/// Alternating stretches that cover `text`: `true` for code (fenced blocks and inline spans).
fn segments(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for code in code_ranges(text) {
        if code.start > at {
            out.push((at..code.start, false));
        }
        at = code.end;
        out.push((code, true));
    }
    if at < text.len() {
        out.push((at..text.len(), false));
    }
    out
}

fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut fence: Option<u8> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let range = offset..offset + line.len();
        offset = range.end;
        match (fence, fence_marker(line)) {
            (Some(open), marker) => {
                push_range(&mut ranges, range);
                fence = if marker == Some(open) { None } else { fence };
            }
            (None, Some(marker)) => {
                fence = Some(marker);
                push_range(&mut ranges, range);
            }
            (None, None) => inline_code(line, range.start, &mut ranges),
        }
    }
    ranges
}

/// `` ` `` or `~` when `line` opens or closes a fenced block (up to 3 spaces of indent).
fn fence_marker(line: &str) -> Option<u8> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    (*b"`~")
        .into_iter()
        .find(|&marker| trimmed.bytes().take(3).filter(|&b| b == marker).count() == 3)
}

fn inline_code(line: &str, base: usize, ranges: &mut Vec<Range<usize>>) {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let run = backtick_run(bytes, at);
        match closing_run(bytes, at + run, run) {
            Some(end) => {
                push_range(ranges, base + at..base + end);
                at = end;
            }
            None => at += run,
        }
    }
}

fn backtick_run(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|&&b| b == b'`').count()
}

/// The end of the first run of exactly `run` backticks at or after `from`.
fn closing_run(bytes: &[u8], from: usize, run: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let found = backtick_run(bytes, at);
        if found == run {
            return Some(at + found);
        }
        at += found;
    }
    None
}

fn push_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    match ranges.last_mut() {
        Some(last) if last.end == range.start => last.end = range.end,
        _ => ranges.push(range),
    }
}

fn push_span(spans: &mut Vec<(Range<usize>, SpanKind)>, range: Range<usize>, kind: SpanKind) {
    if range.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some((last, last_kind)) if *last_kind == kind => last.end = range.end,
        _ => spans.push((range, kind)),
    }
}

/// Splits a stretch outside code into heading lines, links and plain text.
fn plain_spans(text: &str, range: Range<usize>, spans: &mut Vec<(Range<usize>, SpanKind)>) {
    let mut start = range.start;
    for line in text[range].split_inclusive('\n') {
        let at_line_start = start == 0 || text.as_bytes()[start - 1] == b'\n';
        if at_line_start && is_heading(line) {
            let content = line.trim_end_matches('\n').len();
            push_span(spans, start..start + content, SpanKind::Heading);
            push_span(spans, start + content..start + line.len(), SpanKind::Plain);
        } else {
            line_spans(line, start, spans);
        }
        start += line.len();
    }
}

fn line_spans(line: &str, base: usize, spans: &mut Vec<(Range<usize>, SpanKind)>) {
    let mut at = 0;
    while let Some(link) = next_link(line, at) {
        push_span(spans, base + at..base + link.start, SpanKind::Plain);
        push_span(spans, base + link.start..base + link.end, SpanKind::Link);
        at = link.end;
    }
    push_span(spans, base + at..base + line.len(), SpanKind::Plain);
}

fn is_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ')
}

/// The first link in `line` that starts at or after `from`: `[[..]]` or `[text](dest)`, with a
/// leading `!` included.
fn next_link(line: &str, from: usize) -> Option<Range<usize>> {
    let mut search = from;
    while let Some(offset) = line[search..].find('[') {
        let open = search + offset;
        if let Some(end) = link_end(line, open) {
            let embed = open > from && line.as_bytes()[open - 1] == b'!';
            return Some(if embed { open - 1 } else { open }..end);
        }
        search = open + 1;
    }
    None
}

fn link_end(line: &str, open: usize) -> Option<usize> {
    let rest = &line[open..];
    if let Some(inner) = rest.strip_prefix("[[") {
        let length = inner.find("]]").filter(|&length| length > 0)?;
        return Some(open + 2 + length + 2);
    }
    let text = rest[1..].find(']')?;
    let after = &rest[text + 2..];
    let dest = after.strip_prefix('(')?.find(')')?;
    Some(open + text + 2 + 1 + dest + 1)
}

fn rewrite_line(line: &str, out: &mut String) {
    let mut at = 0;
    while let Some(link) = next_link(line, at) {
        out.push_str(&line[at..link.start]);
        out.push_str(&rewrite_link(&line[link.start..link.end]));
        at = link.end;
    }
    out.push_str(&line[at..]);
}

/// A wikilink as Markdown: plain text for embeds, attachments and self links, the literal text
/// for a blank one; any other link as it is.
fn rewrite_link(link: &str) -> String {
    let embed = link.starts_with('!');
    let Some(inner) = link
        .trim_start_matches('!')
        .strip_prefix("[[")
        .and_then(|inner| inner.strip_suffix("]]"))
    else {
        return link.to_owned();
    };
    let (written, alias) = inner.split_once('|').unwrap_or((inner, ""));
    let written = written.trim_end_matches('\\').trim();
    let target = written
        .split_once('#')
        .map_or(written, |(target, _)| target);
    if written.is_empty() {
        return link.to_owned();
    }
    let label = match (alias.trim(), target) {
        ("", "") => written,
        ("", target) => target,
        (alias, _) => alias,
    };
    if embed || target.is_empty() || sb_core::is_attachment_target(target) {
        return escape_label(label);
    }
    format!(
        "[{}]({})",
        escape_label(label),
        note_link_destination(target)
    )
}

/// Backslash-escapes what Markdown would read as formatting in link text.
fn escape_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for c in label.chars() {
        if "\\[]*_`<>&".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::SpanKind::{Code, Heading, Link, Plain};
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, SpanKind)> {
        highlight(text)
            .into_iter()
            .map(|(range, kind)| (&text[range], kind))
            .collect()
    }

    #[test]
    fn should_highlight_headings_links_and_code() {
        let text = "# T\nsee [[a]] and `[[b]]`\n```\n[[c]]\n```\n";
        assert_eq!(
            kinds(text),
            [
                ("# T", Heading),
                ("\nsee ", Plain),
                ("[[a]]", Link),
                (" and ", Plain),
                ("`[[b]]`", Code),
                ("\n", Plain),
                ("```\n[[c]]\n```\n", Code),
            ]
        );
    }

    #[test]
    fn should_cover_whole_text_without_gaps() {
        let text = "x [t](d.md) ~~~\n[[u]]\n#nospace\n####### seven\n## two `open\nend";
        let mut at = 0;
        for (range, _) in highlight(text) {
            assert_eq!(range.start, at);
            at = range.end;
        }
        assert_eq!(at, text.len());
        assert!(highlight("").is_empty());
    }

    #[test]
    fn should_classify_edge_cases() {
        assert_eq!(kinds("#nospace"), [("#nospace", Plain)]);
        assert_eq!(kinds("####### x"), [("####### x", Plain)]);
        assert_eq!(kinds("[t](d.md)"), [("[t](d.md)", Link)]);
        assert_eq!(kinds("![[p.png]]"), [("![[p.png]]", Link)]);
        assert_eq!(kinds("a `x\nb`"), [("a `x\nb`", Plain)]);
        assert_eq!(kinds("~~~\n[[u]]"), [("~~~\n[[u]]", Code)]);
        assert_eq!(kinds("`` a ` b ``"), [("`` a ` b ``", Code)]);
        assert_eq!(kinds("[[a\nb]]"), [("[[a\nb]]", Plain)]);
    }

    #[test]
    fn should_rewrite_wikilinks_outside_code() {
        let text = "[[My Note|alias]] [[x#h]] ![[p.png]] `[[y]]`";
        assert_eq!(
            wikilinks_to_markdown(text),
            "[alias](sb-note:My%20Note) [x](sb-note:x) p.png `[[y]]`"
        );
    }

    #[test]
    fn should_keep_fenced_code_and_markdown_links_unchanged() {
        let text = "```\n[[c]]\n```\n[t](d.md)\n[[T#H|A]]";
        assert_eq!(
            wikilinks_to_markdown(text),
            "```\n[[c]]\n```\n[t](d.md)\n[A](sb-note:T)"
        );
    }

    #[test]
    fn should_escape_markdown_in_link_labels_and_show_self_links_as_text() {
        assert_eq!(
            wikilinks_to_markdown("[[a*b]] [[#Head]] [[T\\|x]]"),
            "[a\\*b](sb-note:a%2Ab) #Head [x](sb-note:T)"
        );
    }

    #[test]
    fn should_show_attachment_wikilinks_and_blank_ones_as_text() {
        assert_eq!(
            wikilinks_to_markdown("[[report.pdf]] [[a/b.PNG|pic]] [[x.md]]"),
            "report.pdf pic [x.md](sb-note:x.md)"
        );
        assert_eq!(wikilinks_to_markdown("[[ ]] ![[ ]]"), "[[ ]] ![[ ]]");
    }

    #[test]
    fn should_list_links_that_are_not_notes_or_web_links() {
        let markdown = "[a](https://x.org) [b](HTTP://y.org) [c](file:///z) [d](other.md) \
                        [e](sb-note:n) <mailto:m@x.org> [f](file:///z) `[g](file:///h)`";
        assert_eq!(
            blocked_link_destinations(markdown),
            ["file:///z", "mailto:m@x.org"]
        );
    }

    #[test]
    fn should_list_blocked_links_in_every_syntax_the_preview_renders() {
        let markdown = concat!(
            "text[^1]\n\n",
            "[^1]: [f](file:///foot)\n\n",
            "| h |\n|---|\n| [c](file:///cell) |\n\n",
            "- [ ] [t](file:///task)\n\n",
            "term\n: [d](file:///def)\n\n",
            "[u](SB-NOTE:foo)",
        );
        assert_eq!(
            blocked_link_destinations(markdown),
            [
                "file:///foot",
                "file:///cell",
                "file:///task",
                "file:///def",
                "SB-NOTE:foo"
            ]
        );
    }

    #[test]
    fn should_round_trip_note_link_targets() {
        let target = "a/ü b#?.md";
        let dest = note_link_destination(target);
        assert_eq!(dest, "sb-note:a/%C3%BC%20b%23%3F.md");
        assert_eq!(note_link_target(&dest).as_deref(), Some(target));
        assert_eq!(note_link_target("sb-note:%zz"), None);
        assert_eq!(note_link_target("sb-note:%FF"), None);
        assert_eq!(note_link_target("https://x.org"), None);
    }

    #[test]
    fn should_treat_markdown_links_to_md_files_as_note_links() {
        assert_eq!(
            note_link_target("My%20Note.md#H").as_deref(),
            Some("My Note.md")
        );
        assert_eq!(note_link_target("pic.png"), None);
        let markdown = wikilinks_to_markdown("[a](b.md#h) [[c]] [d](file:///x.md) [e](b.md#h)");
        assert_eq!(note_link_destinations(&markdown), ["b.md#h", "sb-note:c"]);
    }

    #[test]
    fn should_list_each_note_destination_once() {
        let markdown = wikilinks_to_markdown("[[a]] [[b c]] [[a]] [x](https://x.org)");
        assert_eq!(
            note_link_destinations(&markdown),
            ["sb-note:a", "sb-note:b%20c"]
        );
    }
}
