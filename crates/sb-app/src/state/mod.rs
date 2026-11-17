//! Application behaviour with no dependency on any UI crate.

pub mod jobs;
pub mod notice;
pub mod panels;
pub mod reload;
pub mod settings;
pub mod tree;
pub mod welcome;
pub mod workspace;

pub use jobs::{Job, Poll};
pub use notice::{INFO_NOTICE_TTL, Notice, NoticeBoard, NoticeKind};
pub use panels::{
    DialogKind, InspectorCache, InspectorData, MAX_IGNORED_LISTED, NoteAction, NoteDialog,
    SEARCH_DEBOUNCE, SearchBox, counted, ignore_reason_text, ignored_lines,
};
pub use reload::{RELOAD_DEBOUNCE, Reloader};
pub use settings::{Settings, ThemeChoice};
pub use tree::{Folder, build_tree};
pub use workspace::{AUTOSAVE_DELAY, ApplyOutcome, OpenNote, QUICK_OPEN_LIMIT, Saved, Workspace};
