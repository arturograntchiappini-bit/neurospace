//! Domain library of Second Brain.

mod error;
mod graph;
mod index;
mod markdown;
pub mod ops;
mod path;
mod search;
mod time;
mod vault;
mod watch;

pub use error::{CoreError, Result};
pub use graph::{Graph, GraphEdge, GraphNode, MAX_GRAPH_DEPTH};
pub use index::{Backlink, Index, NoteEntry, OutLink, TagCount};
pub use markdown::{Link, LinkKind, Parsed, parse};
pub use path::{NoteKey, NotePath, PathError};
pub use search::{DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT, SearchHit, Snippet};
pub use time::utc_compact;
pub use vault::{IgnoreReason, Ignored, Loaded, MAX_NOTE_BYTES, Vault};
pub use watch::{VaultWatcher, watch};
