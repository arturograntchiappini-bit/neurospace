//! Domain library of Neurospace.

mod error;
mod graph;
mod index;
mod load_cache;
mod markdown;
pub mod ops;
mod path;
mod search;
mod time;
mod vault;
mod watch;

pub use error::{CoreError, Result};
pub use graph::{Graph, GraphEdge, GraphNode, MAX_GRAPH_DEPTH};
pub use index::{Backlink, Index, NoteEntry, OutLink, TagCount, new_note_path};
pub use load_cache::{LoadCache, LoadDelta, RACY_WINDOW};
pub use markdown::{
    ATTACHMENT_EXTENSIONS, Link, LinkKind, Parsed, is_attachment_target, markdown_note_target,
    parse,
};
pub use path::{NoteKey, NotePath, PathError};
pub use search::{DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT, SearchHit, Snippet};
pub use time::utc_compact;
pub use vault::{IgnoreReason, Ignored, Loaded, MAX_NOTE_BYTES, Vault};
pub use watch::{VaultWatcher, watch};
