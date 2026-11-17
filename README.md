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
| `sb mcp` | Run the MCP server on stdio until stdin closes. | `sb --vault ~/notes mcp` |
| `sb sync run \| invite \| join <TICKET> \| leave \| status` | Sync the vault with your other devices; see [Sync between devices](#sync-between-devices). | `sb sync run`, `sb sync invite` |

Exit code 0 means success, 1 an operation error (one line on stderr), 2 a usage error.

Deleted notes go to `.secondbrain/trash/` inside the vault, named `<UTC timestamp>-<path with / turned into __>`, and can be moved back by hand. To empty the trash, delete the files inside `.secondbrain/trash/`; never delete the `.secondbrain` folder itself. Renaming a note does not rewrite the links that point to it. A note holds at most 4 MiB; larger files are left alone and not listed.

## Sync between devices

Your devices sync their vaults directly with each other, with no server of yours and no account. It works between Windows and Linux; macOS is not supported. The sync engine runs in the foreground, in a terminal (the desktop app will start it itself later).

**What is synced.** Valid notes: the `.md` files `sb list` shows. Files that `sb list` ignores (not UTF-8, over 4 MiB, invalid names), attachments and other file types are not synced. Edits to the same note on two devices are merged line by line. If two devices end up with different notes at the same path, both are kept and one becomes `<name> (conflict <id>).md` next to the original. A note that another device deleted is moved to `.secondbrain/trash/` in your vault, never erased; a file that cannot be read is skipped, never treated as deleted. Syncing is designed for about 10 000 notes and 100 MB of text. The edit history is kept for as long as the group exists and is never compacted: deleting a note removes it from every vault but not from the sync history.

**Pairing, in one direction only.**

1. On the first device: `sb sync run` (leave it running). Its first lines on stderr give the device id and the addresses it listens on.
2. On the same device, in another terminal: `sb sync invite`. It prints a ticket (`sb1/...`). With the engine running, the ticket carries the device's addresses as hints; without it, `invite` still works, but the ticket has none.
3. On the second device: `sb sync join <TICKET>`, then `sb sync run`. The second device dials the first, and from then on both sync whenever both are online.

A ticket is a password: anyone who has it joins the group and receives every note. Share it only over a private channel, and run `sb sync leave` on a device you no longer trust. Do not make two devices join each other's tickets at the same time: each then takes the other's secret and they never sync. If that happened, run `sb sync join` again on one of them with a fresh ticket from the other.

`sb sync join` and `sb sync leave` act at once when no engine runs, and otherwise hand the command to the running engine, which carries it out within a few seconds. `sb sync leave` turns sync off for this vault: the notes stay, the group secret and the sync history on this device are deleted, and a running engine stops. Notes deleted in the group while this device was away come back as new notes if it joins again. Moving or renaming the vault folder (or a drive-letter change for a vault on a removable drive) makes it look unconfigured: `sb sync run` then starts a new group of one and says so on stderr, and the device needs a new ticket (`sb sync join`). `sb sync status` shows the running engine's device id, addresses, relay (in `internet` mode), peers, note count, blocked notes and last error (`--json` for the raw document), or `engine not running`.

**Modes.** The mode is set in `config.json` in the vault's state directory (see below): `{"mode":"internet"}` (the default) or `{"mode":"lan"}`. Optional keys: `"mdns": false` turns off discovery on the local network, `"bind_port": 4040` fixes the port. `config.json` is read once, when `sb sync run` starts: a change needs `sb sync run` to be restarted.

- `internet`: devices find each other by device id through the public infrastructure of the iroh project: a DNS discovery service (`dns.iroh.link`) and relay servers (`use1-1`, `usw1-1`, `euc1-1` and `aps1-1.relay.n0.iroh.link`) that help to cross NATs. Those servers are run by the iroh project, not by you; they learn device ids and IP addresses, and never receive note content, because every connection is end-to-end encrypted. It also keeps the router port mapping (UPnP, PCP, NAT-PMP) on, which only talks to your own router. Verification status: on the development machine (Windows 11) a bare endpoint with iroh's default preset, run by a test and not by `sb sync run`, came online in three runs on 2026-11-17 (after about 25.6 s, under 15.5 s and 0.7 s); an earlier test on 2026-11-15 never came online, cause unknown. Sync between two devices on different networks through this mode has not been verified.
  Troubleshooting: in `internet` mode, `sb sync status` shows the relay the device reached. If it stays at "none yet", something on this computer or network blocks `sb` from going out (an application firewall, a corporate filter): allow `sb` (and later `secondbrain`) outbound HTTPS and UDP. `lan` mode with address hints is not affected.
- `lan`: nothing outside the local network is contacted, and the router port mapping is off. Devices find each other by the address hints in the ticket. Discovery between two real machines is not verified; the tests pair two processes on one machine through loopback.
- In both modes, devices on the same network also find each other by mDNS, unless `"mdns": false`. Where mDNS does not work (turned off, across subnets, some Wi-Fi), set the same fixed `bind_port` on every device, or paired devices will not find each other after a restart: the address hints in a ticket only name the port the engine had then.

**State directory.** The engine keeps its state outside the vault, in `<base>/vaults/<one folder per vault>`, where `<base>` is `%LOCALAPPDATA%\secondbrain` on Windows, `$XDG_DATA_HOME/secondbrain` or `~/.local/share/secondbrain` on Linux, or the folder in the `SB_STATE_DIR` environment variable. Each of these folders holds a `vault-path.txt` that names its vault, so you can tell them apart. A folder holds the device key (`device.key`), the group secret (`secret`), the sync history and `status.json`, created readable by their owner only on Linux (`sb sync run` and `sb sync invite` also create `device.key` and `secret` when they are missing; `sb sync join` writes the group's `secret` and does not create `device.key`), and `config.json`, which you write yourself. Only one engine can run per vault: a second `sb sync run` fails with "already running".

`sb sync run` writes its log to stderr (never the ticket or the secret) and nothing to stdout. Ctrl+C (and SIGTERM on Linux) stops it cleanly with exit code 0, as does `sb sync leave` from another terminal (tested on Linux with SIGTERM; on Windows the same shutdown path runs, but Ctrl+C itself was not tested).

## Desktop app

`secondbrain [VAULT_DIR]` opens the desktop window (`cargo run -p sb-app`). Without an argument it reopens the last vault, or shows a welcome screen where you pick a folder.

**Layout.** A top bar (sidebar toggle, New, Quick open, Graph, Preview/Edit, vault name, Sync, the menu, inspector toggle), the file tree and search on the left, the open note in the middle on a solid surface, backlinks, outgoing links and tags on the right, and a status strip at the bottom. The note is shown in a text editor with light colouring of headings, links and code, or as rendered Markdown where `[[wikilinks]]` are clickable; a link to a note that does not exist yet creates it, an attachment link (`![[image.png]]`) never does and is shown as plain text. The line width of a note is capped at 760 points.

**Shortcuts.** Ctrl+N new note, Ctrl+O quick open (Up and Down choose, Enter opens, Escape closes), Ctrl+E switch between editor and preview, Ctrl+G graph, Ctrl+F search, Ctrl+Plus, Ctrl+Minus and Ctrl+0 for the text size. Every control can also be reached with Tab.

**Saving.** There is no save button: the note is saved 500 ms after the last keystroke, when the window loses focus, when you open another note or vault, and when the application closes. If the file changed on disk while you had unsaved edits (another device, an agent, another editor), nothing is overwritten: your text is written to `<name> (conflict <id>).md` next to the note and the note reloads with the file's text. If the note was removed on disk, your text is kept the same way. A failed save keeps your text in the editor and shows the error in the status strip.

**Look.** The tool areas are glass over a neutral backdrop (the Windows Mica material where the system accepts it, a simulated glass elsewhere); «Reduce transparency» in the menu makes everything opaque, and the menu also sets the theme (System, Light, Dark) and the text size (75 % to 200 %). The font is Inter, used under the SIL Open Font License 1.1 (`crates/sb-app/assets/Inter-LICENSE.txt`).

**Testing.** The GUI is tested headless (`egui_kittest`) and by starting the binary; it has not been tried interactively by a person, and nothing was run on Linux.

## Use with AI agents

`sb mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio. It lets an AI agent such as Claude Code or Claude Desktop read, search, write and navigate a vault with the same operations as the command line; `list_notes` reports the number of ignored files in `ignored`, while `sb list` names them on stderr. Stdout carries protocol messages only; a failed operation (bad path, missing note) comes back as a tool result with `isError: true`.

| Tool | Arguments | Result |
| --- | --- | --- |
| `list_notes` | `folder?` | `{ notes: [{ path, title }], ignored }` |
| `read_note` | `path` | the note's Markdown text |
| `write_note` | `path`, `content` | `{ path, created }`; creates or replaces the whole note |
| `append_to_note` | `path`, `content` | `{ path, created }` |
| `delete_note` | `path` | `{ path, trashed: true }`; moves the note to the trash |
| `rename_note` | `from`, `to` | `{ from, to }` |
| `search_notes` | `query`, `limit?` | `{ hits: [{ path, title, score, snippets }] }` |
| `get_links` | `path` | `{ outgoing, backlinks }` |
| `get_graph` | `around?`, `depth?` | `{ nodes, edges }` |
| `list_tags` | none | `{ tags: [{ tag, count }] }` |

Register it in Claude Code:

```sh
claude mcp add second-brain -- sb --vault /path/to/vault mcp
```

Or in Claude Desktop, in `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "second-brain": {
      "command": "sb",
      "args": ["--vault", "/path/to/vault", "mcp"]
    }
  }
}
```
