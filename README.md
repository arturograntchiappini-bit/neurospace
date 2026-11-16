# Second Brain

A note-taking application in the spirit of Obsidian: a folder of Markdown files, links between notes, a graph of those links, a simple desktop UI. Devices sync directly with each other, with no account and no server, and AI agents are first-class users through a native gateway (MCP server and CLI).

Status: under construction.

## Command line

`sb` works on a vault, a folder of Markdown notes. Pick it with `--vault <DIR>`, else the `SB_VAULT` environment variable, else the current directory. Paths always use `/`. Add `--json` to any command to get one JSON document instead of text.

| Command | What it does | Examples |
| --- | --- | --- |
| `sb list [--folder <FOLDER>]` | List all notes; ignored files are summarised on stderr. | `sb list`, `sb --json list --folder Projects` |
| `sb read <PATH>` | Print a note exactly as stored. | `sb read Home.md`, `sb --vault ~/notes read Projects/Idea.md` |
| `sb write <PATH> [--content <TEXT>]` | Create or replace a note; reads stdin without `--content`. | `sb write Home.md --content "[[Idea]]"`, `echo hi \| sb write log.md` |
| `sb append <PATH> [--content <TEXT>]` | Append to a note, creating it if missing; reads stdin without `--content`. | `sb append log.md --content "done"`, `sb append log.md < more.txt` |
| `sb delete <PATH>` | Move a note to the trash. | `sb delete Old.md`, `sb --json delete Old.md` |
| `sb rename <FROM> <TO>` | Move a note. | `sb rename Home.md Start/Home.md`, `sb --json rename a.md b.md` |
| `sb search <QUERY> [--limit <N>]` | Notes containing every word of the query, best first. | `sb search idea`, `sb --json search "idea body" --limit 5` |
| `sb links <PATH>` | Outgoing links and backlinks of a note. | `sb links Home.md`, `sb --json links Home.md` |
| `sb graph [--around <PATH>] [--depth <N>]` | The link graph, or the part around a note. | `sb graph`, `sb graph --around Home.md --depth 2` |
| `sb tags` | Every tag with its note count. | `sb tags`, `sb --json tags` |
| `sb mcp` | Run the MCP server on stdio (not available yet). | `sb mcp` |

Exit code 0 means success, 1 an operation error (one line on stderr), 2 a usage error.
