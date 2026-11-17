//! The vault's folder tree, derived from the note paths.

use sb_core::NotePath;

/// A folder with its sub-folders and the notes directly inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    /// Last path component; empty for the root.
    pub name: String,
    /// Vault-relative path without trailing `/`; empty for the root.
    pub path: String,
    /// Sub-folders, sorted by lowercase name, then bytewise.
    pub folders: Vec<Folder>,
    /// Notes in this folder, sorted by lowercase file name, then bytewise.
    pub notes: Vec<NotePath>,
}

/// Builds the tree holding every given note.
pub fn build_tree<'a>(paths: impl IntoIterator<Item = &'a NotePath>) -> Folder {
    let mut root = Folder::default();
    for path in paths {
        insert(&mut root, path);
    }
    sort(&mut root);
    root
}

fn insert(root: &mut Folder, note: &NotePath) {
    let mut folder = root;
    for name in note.folder().split('/').filter(|name| !name.is_empty()) {
        let position = folder.folders.iter().position(|child| child.name == name);
        let position = position.unwrap_or_else(|| {
            let path = match folder.path.as_str() {
                "" => name.to_owned(),
                parent => format!("{parent}/{name}"),
            };
            let name = name.to_owned();
            folder.folders.push(Folder {
                name,
                path,
                ..Folder::default()
            });
            folder.folders.len() - 1
        });
        folder = &mut folder.folders[position];
    }
    folder.notes.push(note.clone());
}

fn sort(folder: &mut Folder) {
    folder
        .folders
        .sort_by_cached_key(|child| (child.name.to_lowercase(), child.name.clone()));
    folder.notes.sort_by_cached_key(|note| {
        let name = file_name(note);
        (name.to_lowercase(), name.to_owned())
    });
    folder.folders.iter_mut().for_each(sort);
}

fn file_name(note: &NotePath) -> &str {
    note.as_str().rsplit('/').next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use sb_core::NotePath;

    use super::*;

    #[test]
    fn should_nest_folders_and_sort_case_insensitively() {
        let paths: Vec<NotePath> = ["b.md", "A.md", "z/x.md", "Z2/y.md", "z/sub/w.md"]
            .iter()
            .map(|raw| NotePath::parse(raw).unwrap())
            .collect();
        let root = build_tree(&paths);
        let notes: Vec<&str> = root.notes.iter().map(NotePath::as_str).collect();
        assert_eq!(notes, ["A.md", "b.md"]);
        let folders: Vec<&str> = root.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(folders, ["z", "Z2"]);
        assert_eq!((root.name.as_str(), root.path.as_str()), ("", ""));
        let z = &root.folders[0];
        assert_eq!(z.notes[0].as_str(), "z/x.md");
        assert_eq!(z.folders[0].path, "z/sub");
        assert_eq!(z.folders[0].notes[0].as_str(), "z/sub/w.md");
    }
}
