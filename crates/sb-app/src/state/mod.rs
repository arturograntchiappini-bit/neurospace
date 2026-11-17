//! Application behaviour with no dependency on any UI crate.

pub mod jobs;
pub mod notice;
pub mod reload;
pub mod tree;
pub mod workspace;

pub use jobs::{Job, Poll};
pub use notice::{Notice, NoticeKind};
pub use reload::{RELOAD_DEBOUNCE, Reloader};
pub use tree::{Folder, build_tree};
pub use workspace::{AUTOSAVE_DELAY, ApplyOutcome, OpenNote, QUICK_OPEN_LIMIT, Saved, Workspace};
