<img src="assets/logo/neurospace.svg" alt="Neurospace" width="128">

# Neurospace

A note-taking application in the spirit of Obsidian: a folder of Markdown files, links between notes, a graph of those links, a simple desktop UI. Devices sync directly with each other, with no account and no server, and AI agents are first-class users through a native gateway (MCP server and CLI).

Status: under construction.

## Command line

`ns` works on a vault, a folder of Markdown notes. Pick it with `--vault <DIR>`, else the `NS_VAULT` environment variable, else the current directory. Paths always use `/`. Add `--json` to any command to get one JSON document instead of text.

| Command | What it does | Examples |
| --- | --- | --- |
| `ns list [--folder <FOLDER>]` | List all notes; ignored files are summarised on stderr. | `ns list`, `ns --json list --folder Projects` |
| `ns read <PATH>` | Print a note exactly as stored. | `ns read Home.md`, `ns --vault ~/notes read Projects/Idea.md` |
| `ns write <PATH> [--content <TEXT>]` | Create or replace a note; reads stdin without `--content`. | `ns write Home.md --content "[[Idea]]"`, `echo hi \| ns write log.md` |
| `ns append <PATH> [--content <TEXT>]` | Append to a note, creating it if missing; reads stdin without `--content`. | `ns append log.md --content "done"`, `ns append log.md < more.txt` |
| `ns delete <PATH>` | Move a note to the trash. | `ns delete Old.md`, `ns --json delete Old.md` |
| `ns rename <FROM> <TO>` | Move a note. | `ns rename Home.md Start/Home.md`, `ns --json rename a.md b.md` |
| `ns search <QUERY> [--limit <N>]` | Notes containing every word of the query, best first. | `ns search idea`, `ns --json search "idea body" --limit 5` |
| `ns links <PATH>` | Outgoing links and backlinks of a note. | `ns links Home.md`, `ns --json links Home.md` |
| `ns graph [--around <PATH>] [--depth <N>]` | The link graph, or the part around a note. | `ns graph`, `ns graph --around Home.md --depth 2` |
| `ns tags` | Every tag with its note count. | `ns tags`, `ns --json tags` |
| `ns mcp` | Run the MCP server on stdio until stdin closes. | `ns --vault ~/notes mcp` |
| `ns sync run \| invite \| join <TICKET> \| leave \| status` | Sync the vault with your other devices; see [Sync between devices](#sync-between-devices). | `ns sync run`, `ns sync invite` |

Exit code 0 means success, 1 an operation error (one line on stderr), 2 a usage error.

Deleted notes go to `.neurospace/trash/` inside the vault, named `<UTC timestamp>-<path with / turned into __>`, and can be moved back by hand. To empty the trash, delete the files inside `.neurospace/trash/`; never delete the `.neurospace` folder itself. Renaming a note does not rewrite the links that point to it. A note holds at most 4 MiB; larger files are left alone and not listed.

## Sync between devices

Your devices sync their vaults directly with each other, with no server of yours and no account. It works between Windows and Linux; macOS is not supported. The sync engine runs in the foreground, in a terminal (the desktop app will start it itself later).

**What is synced.** Valid notes: the `.md` files `ns list` shows. Files that `ns list` ignores (not UTF-8, over 4 MiB, invalid names), attachments and other file types are not synced. Edits to the same note on two devices are merged line by line. If two devices end up with different notes at the same path, both are kept and one becomes `<name> (conflict <id>).md` next to the original. A note that another device deleted is moved to `.neurospace/trash/` in your vault, never erased; a file that cannot be read is skipped, never treated as deleted. Syncing is designed for about 10 000 notes and 100 MB of text. The edit history is kept for as long as the group exists and is never compacted: deleting a note removes it from every vault but not from the sync history.

**Pairing, in one direction only.**

1. On the first device: `ns sync run` (leave it running). Its first lines on stderr give the device id and the addresses it listens on.
2. On the same device, in another terminal: `ns sync invite`. It prints a ticket (`ns1/...`). With the engine running, the ticket carries the device's addresses as hints; without it, `invite` still works, but the ticket has none.
3. On the second device: `ns sync join <TICKET>`, then `ns sync run`. The second device dials the first, and from then on both sync whenever both are online.

A ticket is a password: anyone who has it joins the group and receives every note. Share it only over a private channel, and run `ns sync leave` on a device you no longer trust. Do not make two devices join each other's tickets at the same time: each then takes the other's secret and they never sync. If that happened, run `ns sync join` again on one of them with a fresh ticket from the other.

`ns sync join` and `ns sync leave` act at once when no engine runs, and otherwise hand the command to the running engine, which carries it out within a few seconds. `ns sync leave` turns sync off for this vault: the notes stay, the group secret and the sync history on this device are deleted, and a running engine stops. Notes deleted in the group while this device was away come back as new notes if it joins again. Moving or renaming the vault folder (or a drive-letter change for a vault on a removable drive) makes it look unconfigured: `ns sync run` then starts a new group of one and says so on stderr, and the device needs a new ticket (`ns sync join`). `ns sync status` shows the running engine's device id, addresses, relay (in `internet` mode), peers, note count, blocked notes and last error (`--json` for the raw document), or `engine not running`.

**Status file.** `status.json` is rewritten by the engine at every tick and at the end of every round, and at least every 5 seconds while a long round runs (the first round of a large vault can take minutes), so a reader sees a status at most about 5 seconds old; `ns sync status` and the app's sync window show it as it is and never ask the engine. Besides the device id, addresses, peers, note count, blocked notes and relay, it carries `mode` (`internet` or `lan`, the mode the engine was started with: `ns sync status` does not read `config.json`), `busy` (`true` while a round runs; the other fields are then from the moment it began, and `ns sync status` says «a sync round is running»), `notices` (conditions that hold until they end: watching the folder is off, a problem of the last round such as path claims that did not settle, something repaired when the state was opened, a full device list) and `last_error` (the last error only, cleared by the next good round). Notices never contain the group secret or a ticket.

**Modes.** The mode is set in `config.json` in the vault's state directory (see below): `{"mode":"internet"}` (the default) or `{"mode":"lan"}`. Optional keys: `"mdns": false` turns off discovery on the local network, `"bind_port": 4040` fixes the port. `config.json` is read once, when `ns sync run` starts: a change needs `ns sync run` to be restarted.

- `internet`: devices find each other by device id through the public infrastructure of the iroh project: a DNS discovery service (`dns.iroh.link`) and relay servers (`use1-1`, `usw1-1`, `euc1-1` and `aps1-1.relay.n0.iroh.link`) that help to cross NATs. Those servers are run by the iroh project, not by you; they learn device ids and IP addresses, and never receive note content, because every connection is end-to-end encrypted. It also keeps the router port mapping (UPnP, PCP, NAT-PMP) on, which only talks to your own router. Verification status: on the development machine (Windows 11) a bare endpoint with iroh's default preset, run by a test and not by `ns sync run`, came online in three runs on 2026-11-17 (after about 25.6 s, under 15.5 s and 0.7 s); an earlier test on 2026-11-15 never came online, cause unknown. Sync between two devices on different networks through this mode has not been verified.
  Troubleshooting: in `internet` mode, `ns sync status` shows the relay the device reached. If it stays at "none yet", something on this computer or network blocks `ns` from going out (an application firewall, a corporate filter): allow `ns` (and later `neurospace`) outbound HTTPS and UDP. `lan` mode with address hints is not affected.
- `lan`: nothing outside the local network is contacted, and the router port mapping is off. Devices find each other by the address hints in the ticket. Discovery between two real machines is not verified; the tests pair two processes on one machine through loopback.
- In both modes, devices on the same network also find each other by mDNS, unless `"mdns": false`. Where mDNS does not work (turned off, across subnets, some Wi-Fi), set the same fixed `bind_port` on every device, or paired devices will not find each other after a restart: the address hints in a ticket only name the port the engine had then.

**State directory.** The engine keeps its state outside the vault, in `<base>/vaults/<one folder per vault>`, where `<base>` is `%LOCALAPPDATA%\neurospace` on Windows, `$XDG_DATA_HOME/neurospace` or `~/.local/share/neurospace` on Linux, or the folder in the `NS_STATE_DIR` environment variable. Each of these folders holds a `vault-path.txt` that names its vault, so you can tell them apart. A folder holds the device key (`device.key`), the group secret (`secret`), the sync history and `status.json`, created readable by their owner only on Linux (`ns sync run` and `ns sync invite` also create `device.key` and `secret` when they are missing; `ns sync join` writes the group's `secret` and does not create `device.key`), and `config.json`, which you write yourself. Only one engine can run per vault: a second `ns sync run` fails with "already running".

`ns sync run` writes its log to stderr (never the ticket or the secret) and nothing to stdout. Ctrl+C (and SIGTERM on Linux) stops it cleanly with exit code 0, as does `ns sync leave` from another terminal (tested on Linux with SIGTERM; on Windows the same shutdown path runs, but Ctrl+C itself was not tested).

## Desktop app

`neurospace [VAULT_DIR]` opens the desktop window (`cargo run -p ns-app`). Without an argument it reopens the last vault, or shows a welcome screen where you pick a folder.

**Layout.** A top bar (sidebar toggle, New, Quick open, Graph, Preview/Edit, vault name, Sync, the menu, inspector toggle), the file tree and search on the left, the open note in the middle on a solid surface, backlinks, outgoing links and tags on the right, and a status strip at the bottom. The note is shown in a text editor with light colouring of headings, links and code, or as rendered Markdown where `[[wikilinks]]` are clickable; a link to a note that does not exist yet creates it, a link to an attachment (`![[image.png]]`, `[[report.pdf]]`) never creates a note and is shown as plain text. Only web links (`http`, `https`) open outside the app; in the rendered note every other link (a file, a network share, an unknown scheme, a footnote, table or list link included) is not opened and the status strip says so. The line width of a note is capped at 760 points.

**Shortcuts.** Ctrl+N new note, Ctrl+O quick open (Up and Down choose, Enter opens, Escape closes), Ctrl+E switch between editor and preview, Ctrl+G graph, Ctrl+F search, Ctrl+Plus, Ctrl+Minus and Ctrl+0 for the text size. Every control can also be reached with Tab.

**Graph.** Ctrl+G or «Graph» replaces the note with a force-directed graph of the whole vault: notes are filled discs sized by number of links, a link to a note that does not exist yet is a dimmer disc with an outline (its label says «not created yet»), and the open note has a ring. Drag the background to pan, scroll to zoom about the cursor, drag a node to move it; hovering a node shows its label and dims what is not linked to it, and every label shows when zoomed in (from 150 %; «Fit graph» zooms in up to 200 %, so a small graph shows its labels). Clicking a note opens it in the editor; clicking a not-yet-created note creates it with the letter case the link wrote. «Fit graph» (or Home while the graph has the focus) brings every node into view. «List view» (or Enter on the focused graph) opens quick open, so every note can be reached without a mouse; opening any note ends the graph view. The layout is simulated (springs and repulsion) and is at rest when idle; «Reduce motion» in the menu runs it to rest at once instead of animating. Repulsion uses a Barnes–Hut quadtree, and edges outside the view are not drawn. Above 10 000 notes the simulation is skipped (it would stall the window): the notes stay on a deterministic spiral, still pannable, zoomable and clickable, and the pane says «Large vault: simplified layout».

**Saving.** There is no save button: the note is saved 500 ms after the last keystroke, when the window loses focus, when you open another note, when you open another vault and when the application closes (a failure while quitting is written to stderr). If the file changed on disk while you had unsaved edits (another device, an agent, another editor), nothing is overwritten: your text is written to `<name> (conflict <id>).md` next to the note and the note reloads with the file's text. If the note was removed on disk, your text is kept the same way. A save that fails (a read-only note, a full disk) keeps your text in the editor and shows the error; when you leave the note (another note, create, rename, delete, a link, another vault, closing the window) the text goes to a conflict note and the status strip says where. Only if that fails too (the folder is gone) do you stay on the note, and closing the window asks you to close again; the second request writes the text to `recovered/<time> <name>.md` in the state folder, prints that path to stderr and quits.

**Sync window.** «Sync» in the top bar, or the sync summary at the right of the status strip («Sync off», «Sync: 2 of 3 peers online», …), opens the window; Escape closes it. It shows whether sync is on, this device's id, «Create invite» (the ticket appears in a read-only field with «Copy»; it is a password for your notes, share it privately), «Join» with a pasted ticket, «Turn sync off», the network mode (Internet or LAN only; changing it restarts the engine), the devices with the words «online» or «offline», notes the engine cannot sync marked «blocked» with the reason, and the relay reached. Starting the engine, creating an invite, joining and leaving run in the background: the window shows what is in progress and never freezes.

The app starts the sync engine for a vault only when that vault already has a sync group, or when you create an invite or join from this window. **A vault that never used sync never contacts the network.** If another process runs the engine for the vault (for example `ns sync run`), the window says so and shows its status read-only; the app does not start a second engine. Switching vaults and quitting stop the engine first (waiting up to 10 seconds).

**Look.** The tool areas are glass over a neutral backdrop (the Windows Mica material where the system accepts it, a simulated glass elsewhere); «Reduce transparency» in the menu makes everything opaque, and the menu also sets the theme (System, Light, Dark) and the text size (75 % to 200 %). The font is Inter, used under the SIL Open Font License 1.1 (`crates/ns-app/assets/Inter-LICENSE.txt`).

**Requirements.** Windows (tried on Windows 11 only) or a Linux desktop (X11 or Wayland) with a working OpenGL 2.1 driver. The window is drawn with OpenGL (`eframe` with `glow`), so a machine without one, such as a bare virtual machine or a remote session without 3D, may fail to open it. The Windows Mica backdrop needs Windows 11; elsewhere the glass is simulated. macOS is not supported.

**Verified, and not.** Verified: the unit tests of the state module and the graph layout, and the headless UI tests (`egui_kittest`), which drive the real UI code from the welcome screen through creating, editing, renaming, deleting and following notes, search, the graph, an external edit, the sync window and the exit path; the contrast of every colour pair (WCAG 2, at least 4.5 : 1 for text); the whole workspace test suite in a Linux container (`rust:1-bullseye`, run by the orchestrator on 2026-11-18, all green). Not verified: no person has used the GUI; the Linux GUI was not started in M3 (only its test suite ran, in that container, with no display); the folder dialog («Open a folder») was compiled but never opened, and on Linux it uses the xdg desktop portal, so without a portal there is no folder dialog and typing the path still works; the Mica backdrop was not tried on Windows 10 or with a person dragging the window; sync from the GUI between two real machines was not tried (the GUI tests use a fake sync control and never start the engine or touch the network).

## Use with AI agents

`ns mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio. It lets an AI agent such as Claude Code or Claude Desktop read, search, write and navigate a vault with the same operations as the command line; `list_notes` reports the number of ignored files in `ignored`, while `ns list` names them on stderr. Stdout carries protocol messages only; a failed operation (bad path, missing note) comes back as a tool result with `isError: true`.

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
claude mcp add neurospace -- ns --vault /path/to/vault mcp
```

Or in Claude Desktop, in `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "neurospace": {
      "command": "ns",
      "args": ["--vault", "/path/to/vault", "mcp"]
    }
  }
}
```
