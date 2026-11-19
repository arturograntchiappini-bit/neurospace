//! The rows the file tree shows: the tree flattened to the folders that are open.

use std::collections::BTreeSet;

use ns_core::NotePath;

use super::tree::Folder;

/// What one row of the file tree stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A folder; `expanded` says whether its children follow it.
    Folder {
        /// Vault-relative path without trailing `/`.
        path: String,
        /// Last path component.
        name: String,
        /// True when the user opened it.
        expanded: bool,
    },
    /// A note.
    Note(NotePath),
}

/// One row of the file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// Nesting level; 0 for the vault root's children.
    pub depth: usize,
    /// What the row stands for.
    pub kind: RowKind,
}

/// Folders first, then notes, each level in the tree's order; children only under expanded folders.
pub fn visible_rows(tree: &Folder, expanded: &BTreeSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    push_level(tree, expanded, 0, &mut rows);
    rows
}

fn push_level(folder: &Folder, expanded: &BTreeSet<String>, depth: usize, rows: &mut Vec<TreeRow>) {
    for child in &folder.folders {
        let open = expanded.contains(&child.path);
        let kind = RowKind::Folder {
            path: child.path.clone(),
            name: child.name.clone(),
            expanded: open,
        };
        rows.push(TreeRow { depth, kind });
        if open {
            push_level(child, expanded, depth + 1, rows);
        }
    }
    let notes = folder.notes.iter().cloned();
    rows.extend(notes.map(|note| TreeRow {
        depth,
        kind: RowKind::Note(note),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tree::build_tree;

    fn tree(paths: &[&str]) -> Folder {
        let notes: Vec<NotePath> = paths.iter().map(|p| NotePath::parse(p).unwrap()).collect();
        build_tree(&notes)
    }

    fn open(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    fn describe(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match &row.kind {
                RowKind::Folder { path, expanded, .. } => {
                    format!("{}{path}/ {expanded}", row.depth)
                }
                RowKind::Note(note) => format!("{}{note}", row.depth),
            })
            .collect()
    }

    #[test]
    fn should_hide_children_of_collapsed_folders() {
        let tree = tree(&["a/x.md", "a/b/y.md", "top.md"]);
        let rows = visible_rows(&tree, &BTreeSet::new());
        assert_eq!(describe(&rows), ["0a/ false", "0top.md"]);
    }

    #[test]
    fn should_list_subfolders_before_notes_with_depth() {
        let tree = tree(&["a/x.md", "a/b/y.md", "a/c/z.md", "top.md"]);
        let rows = visible_rows(&tree, &open(&["a"]));
        assert_eq!(
            describe(&rows),
            [
                "0a/ true",
                "1a/b/ false",
                "1a/c/ false",
                "1a/x.md",
                "0top.md"
            ]
        );
    }

    #[test]
    fn should_show_nested_rows_only_when_every_ancestor_is_expanded() {
        let tree = tree(&["a/b/y.md", "a/x.md"]);
        let inner_only = visible_rows(&tree, &open(&["a/b"]));
        assert_eq!(describe(&inner_only), ["0a/ false"]);
        let both = visible_rows(&tree, &open(&["a", "a/b"]));
        assert_eq!(
            describe(&both),
            ["0a/ true", "1a/b/ true", "2a/b/y.md", "1a/x.md"]
        );
    }
}
