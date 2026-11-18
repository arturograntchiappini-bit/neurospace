//! The `ns` command line, as declared to `clap`.

use std::convert::Infallible;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};

/// Neurospace: read and change a vault of Markdown notes.
#[derive(Debug, Parser)]
#[command(name = "ns", version)]
pub struct Cli {
    /// The vault directory; defaults to `NS_VAULT`, then the current directory.
    #[arg(long, global = true, env = "NS_VAULT", value_name = "DIR")]
    pub vault: Option<PathBuf>,
    /// Print one JSON document instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// One `ns` command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List all notes; ignored files are summarised on stderr.
    List {
        /// Only the notes inside this folder.
        #[arg(long)]
        folder: Option<String>,
    },
    /// Print a note.
    Read {
        /// Note path.
        path: String,
    },
    /// Create or replace a note; reads stdin without `--content`.
    Write {
        /// Note path.
        path: String,
        /// The text to write.
        #[arg(long)]
        content: Option<String>,
    },
    /// Append to a note; reads stdin without `--content`.
    Append {
        /// Note path.
        path: String,
        /// The text to append.
        #[arg(long)]
        content: Option<String>,
    },
    /// Move a note to the trash.
    Delete {
        /// Note path.
        path: String,
    },
    /// Move a note.
    Rename {
        /// Current note path.
        from: String,
        /// New note path.
        to: String,
    },
    /// Full-text search.
    Search {
        /// Words that must all appear in the note.
        query: String,
        /// Most hits to return.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Outgoing links and backlinks of a note.
    Links {
        /// Note path.
        path: String,
    },
    /// The link graph.
    Graph {
        /// Only the part around this note.
        #[arg(long)]
        around: Option<String>,
        /// How many steps around the note.
        #[arg(long)]
        depth: Option<u8>,
    },
    /// Every tag with its note count.
    Tags,
    /// Run the MCP server on stdio.
    Mcp,
    /// Sync the vault with your other devices, peer to peer.
    Sync {
        /// What to do.
        #[command(subcommand)]
        action: SyncAction,
    },
}

/// One `ns sync` command.
#[derive(Debug, Subcommand)]
pub enum SyncAction {
    /// Run the sync engine in the foreground until Ctrl+C or until the vault leaves its group.
    Run,
    /// Print a ticket that lets another device join this vault's group; the ticket is a password.
    Invite,
    /// Join the group of a ticket from another device.
    Join {
        /// The ticket printed by `ns sync invite`.
        ticket: TicketArg,
    },
    /// Leave the group; the notes stay where they are.
    Leave,
    /// Show the running engine's status.
    Status,
}

/// The ticket argument as typed. Its `Debug` hides it, because a ticket contains the group secret. It is
/// parsed later, by `ns sync join`, so that a bad ticket is an operation error (exit 1) and not a usage error.
#[derive(Clone)]
pub struct TicketArg(pub String);

impl FromStr for TicketArg {
    type Err = Infallible;

    fn from_str(text: &str) -> Result<Self, Infallible> {
        Ok(TicketArg(text.to_owned()))
    }
}

impl fmt::Debug for TicketArg {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("ns1/…/<redacted>")
    }
}
