//! Domain library of Second Brain.

mod error;
mod path;
mod time;
mod vault;

pub use error::{CoreError, Result};
pub use path::{NoteKey, NotePath, PathError};
pub use time::utc_compact;
pub use vault::{IgnoreReason, Ignored, Loaded, MAX_NOTE_BYTES, Vault};
