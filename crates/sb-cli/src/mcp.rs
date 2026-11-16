//! The MCP server behind `sb mcp`: the `sb_core::ops` note operations as tools on stdio.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use sb_core::{CoreError, Vault, ops};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::commands::CliError;

/// Arguments of the tools that take only a note path.
#[derive(Deserialize, JsonSchema)]
pub struct PathParams {
    /// Note path relative to the vault root, `/` separated, ending in `.md`; case-insensitive.
    pub path: String,
}

/// Arguments of `list_notes`.
#[derive(Deserialize, JsonSchema)]
pub struct ListNotesParams {
    /// Only list notes inside this folder, for example `projects`; omit for the whole vault.
    pub folder: Option<String>,
}

/// Arguments of `write_note` and `append_to_note`.
#[derive(Deserialize, JsonSchema)]
pub struct WriteParams {
    /// Note path relative to the vault root, `/` separated, ending in `.md`; case-insensitive.
    pub path: String,
    /// The Markdown text to write or append.
    pub content: String,
}

/// Arguments of `rename_note`.
#[derive(Deserialize, JsonSchema)]
pub struct RenameParams {
    /// Current note path.
    pub from: String,
    /// New note path; folders are created as needed.
    pub to: String,
}

/// Arguments of `search_notes`.
#[derive(Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Words that must all appear in a note for it to match.
    pub query: String,
    /// Most hits to return; omit for the default.
    pub limit: Option<usize>,
}

/// Arguments of `get_graph`.
#[derive(Deserialize, JsonSchema)]
pub struct GraphParams {
    /// Note path to centre the graph on; omit for the whole vault graph.
    pub around: Option<String>,
    /// How many link steps around the note to include; omit for 1.
    pub depth: Option<u8>,
}

/// MCP server exposing one vault.
#[derive(Clone)]
pub struct NotesServer {
    vault: Arc<Vault>,
}

impl NotesServer {
    /// A server that reads and changes `vault`.
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: Arc::new(vault),
        }
    }

    /// Runs a blocking vault operation off the async threads; errors become tool error text.
    async fn run<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Vault) -> Result<T, CoreError> + Send + 'static,
    {
        let vault = Arc::clone(&self.vault);
        tokio::task::spawn_blocking(move || operation(&vault))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }

    /// Like `run`, but the result is returned as one JSON document.
    async fn run_json<T, F>(&self, operation: F) -> Result<String, String>
    where
        T: Serialize + Send + 'static,
        F: FnOnce(&Vault) -> Result<T, CoreError> + Send + 'static,
    {
        let outcome = self.run(operation).await?;
        serde_json::to_string(&outcome).map_err(|error| error.to_string())
    }
}

#[tool_router]
impl NotesServer {
    #[tool(
        description = "Lists the notes of the vault, or of one folder, as {notes:[{path,title}],ignored}. `ignored` counts files that are not valid notes."
    )]
    async fn list_notes(
        &self,
        Parameters(params): Parameters<ListNotesParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::list_notes(vault, params.folder.as_deref()))
            .await
    }

    #[tool(
        description = "Returns the raw Markdown text of a note. Fails when the note does not exist."
    )]
    async fn read_note(
        &self,
        Parameters(params): Parameters<PathParams>,
    ) -> Result<String, String> {
        self.run(move |vault| ops::read_note(vault, &params.path))
            .await
    }

    #[tool(
        description = "Creates a note, or replaces the whole text of an existing one, and returns {path,created}. Missing folders are created."
    )]
    async fn write_note(
        &self,
        Parameters(params): Parameters<WriteParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::write_note(vault, &params.path, &params.content))
            .await
    }

    #[tool(
        description = "Adds text to the end of a note on a new line, creating the note when missing, and returns {path,created}."
    )]
    async fn append_to_note(
        &self,
        Parameters(params): Parameters<WriteParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::append_to_note(vault, &params.path, &params.content))
            .await
    }

    #[tool(
        description = "Moves a note to the vault's trash, from where it can be restored, and returns {path,trashed:true}."
    )]
    async fn delete_note(
        &self,
        Parameters(params): Parameters<PathParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::delete_note(vault, &params.path))
            .await
    }

    #[tool(
        description = "Moves a note to a new path and returns {from,to}. Fails when the destination is another existing note."
    )]
    async fn rename_note(
        &self,
        Parameters(params): Parameters<RenameParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::rename_note(vault, &params.from, &params.to))
            .await
    }

    #[tool(
        description = "Full-text search over all notes. Returns {hits:[{path,title,score,snippets:[{line,text}]}]}, best match first."
    )]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::search_notes(vault, &params.query, params.limit))
            .await
    }

    #[tool(
        description = "Returns the links a note contains (outgoing) and the lines of other notes that link to it (backlinks)."
    )]
    async fn get_links(
        &self,
        Parameters(params): Parameters<PathParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::get_links(vault, &params.path))
            .await
    }

    #[tool(
        description = "Returns the link graph as {nodes:[{id,label,exists}],edges:[{from,to}]}: the whole vault, or the part around one note."
    )]
    async fn get_graph(
        &self,
        Parameters(params): Parameters<GraphParams>,
    ) -> Result<String, String> {
        self.run_json(move |vault| ops::get_graph(vault, params.around.as_deref(), params.depth))
            .await
    }

    #[tool(
        description = "Lists every #tag used in the vault with the number of notes carrying it."
    )]
    async fn list_tags(&self) -> Result<String, String> {
        self.run_json(ops::list_tags).await
    }
}

#[tool_handler(
    name = "second-brain",
    version = "0.1.0",
    instructions = "A vault is a folder of Markdown notes. Note paths are relative to the vault root, use `/` separators, end with `.md` and are matched case-insensitively. `[[Note Name]]` in a note links to another note; use get_links and get_graph to navigate those links. write_note replaces the whole note, while append_to_note adds text to the end of it."
)]
impl ServerHandler for NotesServer {}

/// Serves `vault` over stdio until the client closes stdin.
///
/// # Errors
/// An I/O error building the runtime, or a protocol failure while serving.
pub fn serve_stdio(vault: Vault) -> Result<(), CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = NotesServer::new(vault)
            .serve(stdio())
            .await
            .map_err(|error| CliError::Mcp(error.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|error| CliError::Mcp(error.to_string()))?;
        Ok(())
    })
}
