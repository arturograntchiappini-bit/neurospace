//! Domain library of Second Brain.

mod error;
mod index;
mod markdown;
mod path;
mod time;
mod vault;

pub use error::{CoreError, Result};
pub use index::{Backlink, Index, NoteEntry, OutLink, TagCount};
pub use markdown::{Link, LinkKind, Parsed, parse};
pub use path::{NoteKey, NotePath, PathError};
pub use time::utc_compact;
pub use vault::{IgnoreReason, Ignored, Loaded, MAX_NOTE_BYTES, Vault};
