//! Runs one `ns` command: calls the matching `ns_core::ops` function and renders the result.

use std::io::{self, Read};
use std::path::PathBuf;

use ns_core::ops;
use ns_core::{CoreError, Vault};

use crate::cli::{Cli, Command};
use crate::{mcp, render, sync_cmd};

/// Everything that makes a command fail.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// A note operation failed.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Reading stdin or writing stdout failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A result could not be serialised.
    #[error("cannot serialise the result: {0}")]
    Json(#[from] serde_json::Error),
    /// The MCP server stopped on a protocol failure.
    #[error("MCP server failed: {0}")]
    Mcp(String),
    /// Sync failed.
    #[error(transparent)]
    Sync(#[from] ns_sync::SyncError),
}

/// Runs the parsed command line.
///
/// # Errors
/// Any `CliError`; the caller prints it and exits with code 1.
pub fn run(cli: Cli) -> Result<(), CliError> {
    let vault = open_vault(cli.vault)?;
    let json = cli.json;
    match cli.command {
        Command::List { folder } => list(&vault, folder.as_deref(), json),
        Command::Read { path } => read(&vault, &path, json),
        Command::Write { path, content } => write(&vault, &path, content, json),
        Command::Append { path, content } => append(&vault, &path, content, json),
        Command::Delete { path } => delete(&vault, &path, json),
        Command::Rename { from, to } => rename(&vault, &from, &to, json),
        Command::Search { query, limit } => search(&vault, &query, limit, json),
        Command::Links { path } => links(&vault, &path, json),
        Command::Graph { around, depth } => graph(&vault, around.as_deref(), depth, json),
        Command::Tags => render::emit(json, &ops::list_tags(&vault)?, render::tags),
        Command::Mcp => mcp::serve_stdio(vault),
        Command::Sync { action } => sync_cmd::run(&vault, action, json),
    }
}

/// Opens `vault`, or the current directory when it is `None`.
///
/// # Errors
/// `NotADirectory` when the directory is missing, or an I/O error reading the current directory.
pub fn open_vault(vault: Option<PathBuf>) -> Result<Vault, CliError> {
    let root = match vault {
        Some(root) => root,
        None => std::env::current_dir()?,
    };
    Ok(Vault::open(root)?)
}

fn list(vault: &Vault, folder: Option<&str>, json: bool) -> Result<(), CliError> {
    let notes = ops::list_notes(vault, folder)?;
    render::emit(json, &notes, render::note_list)?;
    if notes.ignored > 0 {
        eprint!("{}", render::ignored_summary(&vault.load()?.ignored));
    }
    Ok(())
}

fn read(vault: &Vault, path: &str, json: bool) -> Result<(), CliError> {
    render::emit(json, &ops::read_note(vault, path)?, String::clone)
}

fn write(vault: &Vault, path: &str, content: Option<String>, json: bool) -> Result<(), CliError> {
    let outcome = ops::write_note(vault, path, &content_or_stdin(content)?)?;
    render::emit(json, &outcome, render::written)
}

fn append(vault: &Vault, path: &str, content: Option<String>, json: bool) -> Result<(), CliError> {
    let outcome = ops::append_to_note(vault, path, &content_or_stdin(content)?)?;
    render::emit(json, &outcome, render::appended)
}

fn delete(vault: &Vault, path: &str, json: bool) -> Result<(), CliError> {
    render::emit(json, &ops::delete_note(vault, path)?, render::deleted)
}

fn rename(vault: &Vault, from: &str, to: &str, json: bool) -> Result<(), CliError> {
    render::emit(json, &ops::rename_note(vault, from, to)?, render::renamed)
}

fn search(vault: &Vault, query: &str, limit: Option<usize>, json: bool) -> Result<(), CliError> {
    let results = ops::search_notes(vault, query, limit)?;
    render::emit(json, &results, render::search)
}

fn links(vault: &Vault, path: &str, json: bool) -> Result<(), CliError> {
    render::emit(json, &ops::get_links(vault, path)?, render::links)
}

fn graph(
    vault: &Vault,
    around: Option<&str>,
    depth: Option<u8>,
    json: bool,
) -> Result<(), CliError> {
    let graph = ops::get_graph(vault, around, depth)?;
    render::emit(json, &graph, render::graph)
}

/// The `--content` text, or all of stdin, unmodified.
fn content_or_stdin(content: Option<String>) -> Result<String, CliError> {
    if let Some(content) = content {
        return Ok(content);
    }
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    Ok(text)
}
