//! Extraction of links and tags from the text of one note.

use std::collections::BTreeSet;
use std::mem;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use serde::Serialize;

/// How a link was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// `[[Target]]`.
    Wiki,
    /// `![[Target]]`.
    Embed,
    /// `[text](Note.md)`.
    Markdown,
}

/// One link found in a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// Target as written, heading removed, never empty.
    pub target: String,
    /// Heading after the `#`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// Display text after the `|` of a wiki link, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Syntax the link was written in.
    pub kind: LinkKind,
    /// 1-based line of the link.
    pub line: u32,
}

/// File extensions (without the dot) whose links point to attachments, not notes (spec 6.1).
pub const ATTACHMENT_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "avif", "ico", "tif", "tiff", "pdf", "mp3",
    "wav", "ogg", "m4a", "flac", "mp4", "webm", "mov", "mkv", "avi",
];

impl Link {
    /// True when the target's last component ends in `.` plus an attachment extension, ignoring case.
    pub fn is_attachment(&self) -> bool {
        is_attachment_target(&self.target)
    }
}

pub(crate) fn is_attachment_target(target: &str) -> bool {
    let slashed = target.trim().replace('\\', "/");
    let last = slashed.rsplit('/').next().unwrap_or_default();
    let Some((_, extension)) = last.rsplit_once('.') else {
        return false;
    };
    ATTACHMENT_EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

/// Links and tags of one note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Links in document order.
    pub links: Vec<Link>,
    /// Tags lowercased, sorted, unique, without the `#`.
    pub tags: Vec<String>,
}

/// Extracts the links and inline tags of `content`, ignoring code and front matter.
pub fn parse(content: &str) -> Parsed {
    let options = Options::ENABLE_WIKILINKS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut collector = Collector::new(content);
    for (event, range) in Parser::new_ext(content, options).into_offset_iter() {
        collector.accept(event, range.start);
    }
    collector.finish()
}

/// Text of a wiki link still being read: its inner text is the alias, not prose.
struct OpenWiki {
    link_index: Option<usize>,
    has_pothole: bool,
    alias: String,
}

struct Collector<'a> {
    content: &'a str,
    links: Vec<Link>,
    tags: BTreeSet<String>,
    run: String,
    in_code: bool,
    open_wiki: Option<OpenWiki>,
    /// Byte offset and 1-based line of the last link seen; offsets never decrease.
    line_cursor: (usize, u32),
}

impl<'a> Collector<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            content,
            links: Vec::new(),
            tags: BTreeSet::new(),
            run: String::new(),
            in_code: false,
            open_wiki: None,
            line_cursor: (0, 1),
        }
    }

    fn accept(&mut self, event: Event<'_>, offset: usize) {
        match event {
            Event::Start(tag @ (Tag::Link { .. } | Tag::Image { .. })) => {
                self.start_link(&tag, offset);
            }
            Event::End(TagEnd::Link | TagEnd::Image) => self.end_link(),
            Event::Start(Tag::CodeBlock(_) | Tag::MetadataBlock(_)) => self.set_in_code(true),
            Event::End(TagEnd::CodeBlock | TagEnd::MetadataBlock(_)) => self.set_in_code(false),
            Event::Text(text) => self.source_text(&text, offset),
            Event::SoftBreak | Event::HardBreak => self.text("\n"),
            Event::Code(code) => {
                self.flush_run();
                if self.open_wiki.is_some() {
                    self.text(&code);
                }
            }
            _ => self.flush_run(),
        }
    }

    fn start_link(&mut self, tag: &Tag<'_>, offset: usize) {
        let Some((link_type, dest, is_image)) = link_parts(tag) else {
            return;
        };
        self.flush_run();
        let line = self.line_at(offset);
        match link_type {
            LinkType::WikiLink { has_pothole } => {
                let kind = if is_image {
                    LinkKind::Embed
                } else {
                    LinkKind::Wiki
                };
                let link_index = self.push_link(dest, kind, line);
                self.open_wiki = Some(OpenWiki {
                    link_index,
                    has_pothole,
                    alias: String::new(),
                });
            }
            _ if !is_image && is_note_destination(dest) => {
                self.push_link(dest, LinkKind::Markdown, line);
            }
            _ => {}
        }
    }

    fn push_link(&mut self, dest: &str, kind: LinkKind, line: u32) -> Option<usize> {
        let link = build_link(dest, kind, line)?;
        self.links.push(link);
        Some(self.links.len() - 1)
    }

    fn end_link(&mut self) {
        self.flush_run();
        let Some(wiki) = self.open_wiki.take() else {
            return;
        };
        let alias = wiki.alias.trim();
        if let Some(index) = wiki.link_index
            && wiki.has_pothole
            && !alias.is_empty()
        {
            self.links[index].alias = Some(alias.to_owned());
        }
    }

    fn set_in_code(&mut self, in_code: bool) {
        self.flush_run();
        self.in_code = in_code;
    }

    fn line_at(&mut self, offset: usize) -> u32 {
        let (from, base) = if offset >= self.line_cursor.0 {
            self.line_cursor
        } else {
            (0, 1)
        };
        let newlines = self.content.as_bytes()[from..offset]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count();
        let line = base.saturating_add(u32::try_from(newlines).unwrap_or(u32::MAX));
        self.line_cursor = (offset, line);
        line
    }

    /// Adds a text event that starts at `offset`, remembering an escaping backslash the parser dropped.
    fn source_text(&mut self, text: &str, offset: usize) {
        // Only a `#` can start a tag, so only then is the escape worth looking for.
        if text.starts_with('#') && self.open_wiki.is_none() && !self.in_code {
            let backslashes = self.content.as_bytes()[..offset]
                .iter()
                .rev()
                .take_while(|&&byte| byte == b'\\')
                .count();
            if backslashes % 2 == 1 {
                self.run.push('\\');
            }
        }
        self.text(text);
    }

    fn text(&mut self, text: &str) {
        match &mut self.open_wiki {
            Some(wiki) if wiki.has_pothole => wiki.alias.push_str(text),
            Some(_) => {}
            None if self.in_code => {}
            None => self.run.push_str(text),
        }
    }

    fn flush_run(&mut self) {
        let run = mem::take(&mut self.run);
        scan_tags(&run, &mut self.tags);
    }

    fn finish(mut self) -> Parsed {
        self.flush_run();
        Parsed {
            links: self.links,
            tags: self.tags.into_iter().collect(),
        }
    }
}

fn link_parts<'t>(tag: &'t Tag<'_>) -> Option<(LinkType, &'t str, bool)> {
    match tag {
        Tag::Link {
            link_type,
            dest_url,
            ..
        } => Some((*link_type, dest_url, false)),
        Tag::Image {
            link_type,
            dest_url,
            ..
        } => Some((*link_type, dest_url, true)),
        _ => None,
    }
}

fn build_link(dest: &str, kind: LinkKind, line: u32) -> Option<Link> {
    // The parser keeps the backslash of an escaped pipe (`[[A#H\|B]]`) in a wiki destination.
    let dest = if kind == LinkKind::Markdown {
        dest
    } else {
        dest.strip_suffix('\\').unwrap_or(dest)
    };
    let (target, heading) = dest.split_once('#').unwrap_or((dest, ""));
    let (target, heading) = if kind == LinkKind::Markdown {
        (percent_decode(target), percent_decode(heading))
    } else {
        (target.to_owned(), heading.to_owned())
    };
    let target = target.trim();
    let heading = heading.trim();
    if target.is_empty() {
        return None;
    }
    Some(Link {
        target: target.to_owned(),
        heading: (!heading.is_empty()).then(|| heading.to_owned()),
        alias: None,
        kind,
        line,
    })
}

fn is_note_destination(dest: &str) -> bool {
    let path = dest.split_once('#').map_or(dest, |(path, _)| path);
    !has_url_scheme(dest) && percent_decode(path).to_ascii_lowercase().ends_with(".md")
}

fn has_url_scheme(dest: &str) -> bool {
    let mut chars = dest.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        match c {
            ':' => return true,
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-') => {}
            _ => return false,
        }
    }
    false
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = bytes
            .get(i + 1..i + 3)
            .filter(|_| bytes[i] == b'%')
            .and_then(hex_pair);
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                i += 3;
            }
            None => {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_pair(digits: &[u8]) -> Option<u8> {
    let [high, low] = digits else {
        return None;
    };
    let value = |digit: &u8| char::from(*digit).to_digit(16);
    u8::try_from(value(high)? * 16 + value(low)?).ok()
}

fn scan_tags(run: &str, tags: &mut BTreeSet<String>) {
    let mut previous: Option<char> = None;
    for (index, c) in run.char_indices() {
        let starts_tag = c == '#' && previous.is_none_or(|p| p.is_whitespace() || p == '(');
        previous = Some(c);
        if starts_tag && let Some(tag) = tag_at(&run[index + 1..]) {
            tags.insert(tag.to_lowercase());
        }
    }
}

fn tag_at(rest: &str) -> Option<&str> {
    let is_tag_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '/');
    let tag = &rest[..rest.find(|c| !is_tag_char(c)).unwrap_or(rest.len())];
    tag.chars().any(|c| !c.is_numeric()).then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn link(
        target: &str,
        heading: Option<&str>,
        alias: Option<&str>,
        kind: LinkKind,
        line: u32,
    ) -> Link {
        Link {
            target: target.to_owned(),
            heading: heading.map(str::to_owned),
            alias: alias.map(str::to_owned),
            kind,
            line,
        }
    }

    #[test]
    fn should_extract_links_of_every_kind() {
        let md = "# Title\n\
See [[Note A]] and [[Folder/Note B|B alias]] and [[Note C#Section]].\n\
Embed: ![[Diagram]]\n\
MD: [text](other/Note%20D.md#h) and [ext](https://example.com/x.md) and [img](pic.png)\n";
        assert_eq!(
            parse(md).links,
            vec![
                link("Note A", None, None, LinkKind::Wiki, 2),
                link("Folder/Note B", None, Some("B alias"), LinkKind::Wiki, 2),
                link("Note C", Some("Section"), None, LinkKind::Wiki, 2),
                link("Diagram", None, None, LinkKind::Embed, 3),
                link("other/Note D.md", Some("h"), None, LinkKind::Markdown, 4),
            ]
        );
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
        assert_eq!(
            parse("one\r\ntwo\r\n[[X]]\r\n").links,
            vec![link("X", None, None, LinkKind::Wiki, 3)]
        );
    }

    #[test]
    fn should_extract_inline_tags() {
        let md = "#tag1 text #Tag-Two/sub (#paren) #123 a#nottag #snake_case\n`#code`\n";
        assert_eq!(
            parse(md).tags,
            ["paren", "snake_case", "tag-two/sub", "tag1"]
        );
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

    #[test]
    fn should_strip_escaped_pipe_backslash_when_wiki_link_has_alias() {
        assert_eq!(
            parse("[[A\\|B]]").links,
            vec![link("A", None, Some("B"), LinkKind::Wiki, 1)]
        );
    }

    #[test]
    fn should_report_wiki_link_once_when_escaped_pipe_is_in_table_cell() {
        let md = "| h | h |\n| - | - |\n| x | [[A\\|B]] |\n";
        assert_eq!(
            parse(md).links,
            vec![link("A", None, Some("B"), LinkKind::Wiki, 3)]
        );
    }

    #[test]
    fn should_strip_escaped_pipe_backslash_from_heading() {
        let expected = vec![link(
            "Note",
            Some("Heading"),
            Some("Alias"),
            LinkKind::Wiki,
            3,
        )];
        let table = "| h | h |\n| - | - |\n| [[Note#Heading\\|Alias]] | x |\n";
        assert_eq!(parse(table).links, expected);
        assert_eq!(
            parse("[[Note#Heading\\|Alias]]").links,
            vec![link(
                "Note",
                Some("Heading"),
                Some("Alias"),
                LinkKind::Wiki,
                1
            )]
        );
    }

    #[test]
    fn should_parse_large_note_in_linear_time() {
        let md = "[[n]]\n".repeat(200_000);
        let started = std::time::Instant::now();
        let links = parse(&md).links;
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(links.len(), 200_000);
        assert_eq!(links.last().map(|l| l.line), Some(200_000));
    }

    #[test]
    fn should_not_treat_escaped_hash_as_tag() {
        assert_eq!(parse("\\#tag and #real").tags, ["real"]);
        assert_eq!(parse("\\\\ #tag").tags, ["tag"]);
    }

    #[test]
    fn should_keep_percent_sign_when_escape_is_not_two_hex_digits() {
        assert_eq!(
            parse("[y](%+1.md)").links,
            vec![link("%+1.md", None, None, LinkKind::Markdown, 1)]
        );
    }

    #[test]
    fn should_decode_percent_escapes_in_heading_of_markdown_link() {
        assert_eq!(
            parse("[z](A.MD#H%23x)").links,
            vec![link("A.MD", Some("H#x"), None, LinkKind::Markdown, 1)]
        );
    }

    #[test]
    fn should_ignore_markdown_links_with_url_scheme() {
        assert!(
            parse("[m](mailto:x.md) [h](http://e.com/a.md)")
                .links
                .is_empty()
        );
    }

    #[test]
    fn should_keep_pipe_text_as_alias_when_embed_has_pipe() {
        assert_eq!(
            parse("![[Img|300]]").links,
            vec![link("Img", None, Some("300"), LinkKind::Embed, 1)]
        );
    }

    #[test]
    fn should_leave_alias_and_heading_empty_when_they_are_blank() {
        assert_eq!(
            parse("[[A|]] [[B#]]").links,
            vec![
                link("A", None, None, LinkKind::Wiki, 1),
                link("B", None, None, LinkKind::Wiki, 1),
            ]
        );
    }

    #[test]
    fn should_not_find_tags_inside_wiki_link_text() {
        assert!(parse("[[#Only heading]] [[Note #x]]").tags.is_empty());
    }

    #[test]
    fn should_mark_attachment_when_extension_is_listed() {
        let md =
            "![[photo.PNG]] [[docs/report.pdf]] [[Idea]] [[v1.2 notes]] [[Meeting 2026.11.17]]";
        let flags: Vec<bool> = parse(md).links.iter().map(Link::is_attachment).collect();
        assert_eq!(flags, [true, true, false, false, false]);
    }
}
