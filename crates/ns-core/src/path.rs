use std::fmt;

use serde::Serialize;

const MAX_PATH_BYTES: usize = 1024;
const MAX_COMPONENT_BYTES: usize = 255;
const MARKDOWN_EXTENSION: &str = ".md";
const ILLEGAL_CHARS: &str = "<>:\"|?*";

/// Reason a raw string is not a valid note path (spec section 5.2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The path is the empty string.
    #[error("path is empty")]
    Empty,
    /// The path or one of its components exceeds the byte limit.
    #[error("path is too long (at most 1024 bytes, 255 per component)")]
    TooLong,
    /// The path contains a backslash; use `/` as separator.
    #[error("use `/` as separator, not `\\`")]
    Backslash,
    /// The path starts with `/`.
    #[error("path must be relative to the vault")]
    Absolute,
    /// Two separators in a row, or a trailing separator.
    #[error("path has an empty component")]
    EmptyComponent,
    /// A component is `.` or `..`.
    #[error("`.` and `..` are not allowed")]
    DotComponent,
    /// A component starts with `.`.
    #[error("names starting with `.` are hidden and cannot be notes")]
    Hidden,
    /// A component contains a character that is illegal on some platform.
    #[error("character `{}` is not allowed", .0.escape_debug())]
    IllegalChar(char),
    /// A component ends with a space or a dot.
    #[error("names cannot end with a space or a dot")]
    TrailingSpaceOrDot,
    /// A component's stem is a Windows device name.
    #[error("name `{0}` is reserved on Windows")]
    ReservedName(String),
    /// The last component does not end with `.md`.
    #[error("note paths must end with .md")]
    NotMarkdown,
}

/// Validated vault-relative path of a note, `/`-separated, case preserved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct NotePath(String);

/// Identity of a note: its path in Unicode lowercase.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NoteKey(String);

impl NotePath {
    /// Validates `raw` against every rule of spec section 5.2.
    ///
    /// # Errors
    /// Returns the first rule the path breaks, checked in a fixed order.
    pub fn parse(raw: &str) -> Result<NotePath, PathError> {
        if raw.is_empty() {
            return Err(PathError::Empty);
        }
        if raw.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if raw.contains('\\') {
            return Err(PathError::Backslash);
        }
        if raw.starts_with('/') {
            return Err(PathError::Absolute);
        }
        raw.split('/').try_for_each(validate_component)?;
        let last = raw.rsplit('/').next().unwrap_or(raw);
        strip_markdown_extension(last).ok_or(PathError::NotMarkdown)?;
        Ok(NotePath(raw.to_owned()))
    }

    /// The path as written, with `/` separators.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The note key: this path in Unicode lowercase.
    pub fn key(&self) -> NoteKey {
        NoteKey(self.0.to_lowercase())
    }

    /// The last component without its final `.md` (any case).
    pub fn stem(&self) -> &str {
        let last = self.0.rsplit('/').next().unwrap_or(&self.0);
        strip_markdown_extension(last).unwrap_or(last)
    }

    /// The containing folder: empty at the vault root, else `a/b` without trailing slash.
    pub fn folder(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(folder, _)| folder)
    }

    /// The path components, in order.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl NoteKey {
    /// The key as a lowercase string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for NoteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn strip_markdown_extension(name: &str) -> Option<&str> {
    let split = name.len().checked_sub(MARKDOWN_EXTENSION.len())?;
    let (stem, extension) = name.split_at_checked(split)?;
    extension
        .eq_ignore_ascii_case(MARKDOWN_EXTENSION)
        .then_some(stem)
}

fn validate_component(component: &str) -> Result<(), PathError> {
    if component.is_empty() {
        return Err(PathError::EmptyComponent);
    }
    if component == "." || component == ".." {
        return Err(PathError::DotComponent);
    }
    if component.starts_with('.') {
        return Err(PathError::Hidden);
    }
    if component.len() > MAX_COMPONENT_BYTES {
        return Err(PathError::TooLong);
    }
    if let Some(c) = component.chars().find(|&c| is_illegal_char(c)) {
        return Err(PathError::IllegalChar(c));
    }
    if component.ends_with([' ', '.']) {
        return Err(PathError::TrailingSpaceOrDot);
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    if is_windows_device_name(stem) {
        return Err(PathError::ReservedName(stem.to_owned()));
    }
    Ok(())
}

fn is_windows_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let numbered = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"));
    numbered.is_some_and(|suffix| {
        let mut chars = suffix.chars();
        matches!(chars.next(), Some('0'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
    })
}

fn is_illegal_char(c: char) -> bool {
    ILLEGAL_CHARS.contains(c) || c.is_ascii_control()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_path_when_it_is_valid() {
        for p in [
            "a.md",
            "Projects/Idea.md",
            "a/b/c/Note 1.md",
            "día/ñandú.md",
            "x.MD",
            "notes/v1.2 plan.md",
            "🧠/idea.md",
        ] {
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
            (".neurospace/trash/x.md", PathError::Hidden),
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
            assert_eq!(
                NotePath::parse(raw).unwrap_err(),
                *expected,
                "path: {raw:?}"
            );
        }
    }

    #[test]
    fn should_reject_device_names_in_every_spelling() {
        for raw in [
            "CON .md",
            "nul  .md",
            "a/AUX .md",
            "prn .notes.md",
            "COM0.md",
            "lpt0.md",
            "COM¹.md",
            "LPT².md",
            "com³.md",
            "CONIN$.md",
            "conout$.md",
        ] {
            assert!(
                matches!(
                    NotePath::parse(raw).unwrap_err(),
                    PathError::ReservedName(_)
                ),
                "path: {raw:?}"
            );
        }
    }

    #[test]
    fn should_still_accept_names_that_only_resemble_device_names() {
        for raw in [
            "CONSOLE.md",
            "COM10.md",
            "COM.md",
            "LPT.md",
            "CON-notes.md",
            "COM⁴.md",
            "CONIN.md",
            "my CON.md",
        ] {
            assert!(NotePath::parse(raw).is_ok(), "path: {raw:?}");
        }
    }

    #[test]
    fn should_reject_path_when_it_is_too_long() {
        let long_component = format!("{}.md", "a".repeat(253)); // 256 bytes
        assert_eq!(
            NotePath::parse(&long_component).unwrap_err(),
            PathError::TooLong
        );
        let long_path = format!("{}x.md", "ab/".repeat(341)); // 1027 bytes
        assert_eq!(NotePath::parse(&long_path).unwrap_err(), PathError::TooLong);
    }

    #[test]
    fn should_accept_paths_at_the_length_limits() {
        let ascii = format!("{}.md", "a".repeat(252));
        let accented = format!("{}.md", "é".repeat(126));
        let path = format!("{}x.md", "ab/".repeat(340));
        assert_eq!((ascii.len(), accented.len(), path.len()), (255, 255, 1024));
        for exact in [&ascii, &accented, &path] {
            assert!(NotePath::parse(exact).is_ok(), "{exact}");
        }
        let over_ascii = format!("a{ascii}");
        let over_accented = format!("{}a.md", "é".repeat(126));
        let over_path = format!("{}xy.md", "ab/".repeat(340));
        for over in [&over_ascii, &over_accented, &over_path] {
            assert_eq!(NotePath::parse(over).unwrap_err(), PathError::TooLong);
        }
    }

    #[test]
    fn should_expose_key_stem_and_folder() {
        let p = NotePath::parse("Projects/ÀB Idea.md").unwrap();
        assert_eq!(p.key().as_str(), "projects/àb idea.md");
        assert_eq!(p.stem(), "ÀB Idea");
        assert_eq!(p.folder(), "Projects");
        assert_eq!(
            p.components().collect::<Vec<_>>(),
            ["Projects", "ÀB Idea.md"]
        );
        let root = NotePath::parse("b.c.MD").unwrap();
        assert_eq!(root.stem(), "b.c");
        assert_eq!(root.folder(), "");
    }

    #[test]
    fn should_serialize_as_plain_string() {
        let p = NotePath::parse("a/b.md").unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"a/b.md\"");
    }
}
