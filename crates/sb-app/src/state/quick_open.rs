//! The quick-open popup: what was typed and which match is highlighted.

use sb_core::NotePath;

use super::workspace::Workspace;

/// The most matches the popup lists.
pub const QUICK_OPEN_SHOWN: usize = 10;

/// The state of an open quick-open popup.
#[derive(Debug, Default)]
pub struct QuickOpen {
    /// What the user has typed.
    pub query: String,
    /// True once the field has taken the keyboard focus.
    pub focused: bool,
    highlight: usize,
}

impl QuickOpen {
    /// The notes to list for the current query.
    pub fn matches(&self, workspace: &Workspace) -> Vec<NotePath> {
        let mut found = workspace.quick_open(&self.query);
        found.truncate(QUICK_OPEN_SHOWN);
        found
    }

    /// The index of the highlighted row among `count` rows.
    pub fn highlighted(&self, count: usize) -> usize {
        self.highlight.min(count.saturating_sub(1))
    }

    /// Moves the highlight one row down or up, staying inside `count` rows.
    pub fn step(&mut self, down: bool, count: usize) {
        let current = self.highlighted(count);
        self.highlight = if down {
            (current + 1).min(count.saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
    }

    /// Puts the highlight back on the first row; called when the query changes.
    pub fn reset_highlight(&mut self) {
        self.highlight = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn workspace_with(names: &[String]) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    #[test]
    fn should_list_at_most_ten_matches() {
        let names: Vec<String> = (0..12).map(|n| format!("n{n:02}.md")).collect();
        let (_dir, workspace) = workspace_with(&names);
        let quick = QuickOpen {
            query: "n".into(),
            ..QuickOpen::default()
        };
        assert_eq!(quick.matches(&workspace).len(), QUICK_OPEN_SHOWN);
        assert!(QuickOpen::default().matches(&workspace).is_empty());
    }

    #[test]
    fn should_move_highlight_within_the_rows() {
        let mut quick = QuickOpen::default();
        assert_eq!(quick.highlighted(3), 0);
        quick.step(false, 3);
        assert_eq!(quick.highlighted(3), 0);
        quick.step(true, 3);
        quick.step(true, 3);
        quick.step(true, 3);
        assert_eq!(quick.highlighted(3), 2);
        assert_eq!(quick.highlighted(1), 0);
        quick.reset_highlight();
        assert_eq!(quick.highlighted(3), 0);
        quick.step(true, 0);
        assert_eq!(quick.highlighted(0), 0);
    }
}
