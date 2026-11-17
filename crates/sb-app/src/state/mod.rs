//! Application behaviour with no dependency on any UI crate.

pub mod notice;
pub mod tree;
pub mod workspace;

pub use notice::{Notice, NoticeKind};
pub use tree::{Folder, build_tree};
pub use workspace::{AUTOSAVE_DELAY, OpenNote, QUICK_OPEN_LIMIT, Workspace};
