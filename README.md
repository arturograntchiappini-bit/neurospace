<h1 align="center">
  <img src="docs/images/pixel/banner.svg" alt="Neurospace: a pixel-art brain, the word NEUROSPACE and the tag LEVEL 1, EXPERIMENTAL" width="900">
</h1>

<p align="center">
  <strong>Markdown notes, a link graph, serverless peer-to-peer sync and a native gateway for AI agents. Written entirely in Rust.</strong>
</p>

<p align="center">
  <img alt="Status: experimental, work in progress" src="https://img.shields.io/badge/status-experimental%20%C2%B7%20work%20in%20progress-orange">
  <a href="https://github.com/arturograntchiappini-bit/neurospace/releases/tag/v0.1.0"><img alt="Version 0.1.0" src="https://img.shields.io/badge/version-0.1.0-blue"></a>
  <a href="LICENSE"><img alt="Licence MIT" src="https://img.shields.io/badge/licence-MIT-green"></a>
  <img alt="Rust 1.98 or newer" src="https://img.shields.io/badge/rust-1.98%2B-orange">
  <img alt="Platforms: Windows and Linux" src="https://img.shields.io/badge/platforms-Windows%20%7C%20Linux-lightgrey">
  <a href="https://github.com/arturograntchiappini-bit/neurospace/actions/workflows/ci.yml"><img alt="CI status" src="https://github.com/arturograntchiappini-bit/neurospace/actions/workflows/ci.yml/badge.svg"></a>
</p>

> [!WARNING]
> **Experimental — work in progress.** Neurospace 0.1 is the first level of its architecture: the core, sync, the desktop app and the agent gateway. Several more levels are coming, and they may change the vault layout, the sync protocol and the command line without a migration path. Keep a backup of your notes.

<p align="center">
  <img src="docs/images/graph-dark.png" alt="The Neurospace desktop app showing the link graph of the demo vault in the dark theme" width="900">
</p>

Neurospace is a note-taking application in the spirit of Obsidian: a folder of Markdown files, links between notes, a graph of those links and a simple desktop UI. Your devices sync directly with each other, with no account and no server, and AI agents are first-class users through a native gateway (an MCP server and a command line).

Version 0.1.0, the first release. It runs on Windows and on Linux (macOS is not supported). The command line, the MCP server and sync are tested against real vaults; no person has used the desktop app interactively yet (see [Verified, and not](#verified-and-not)).

## Contents

- [Highlights](#highlights)
- [Screenshots](#screenshots)
- [Quick start](#quick-start)
- [Install and build](#install-and-build)
- [Using the app](#using-the-app)
- [Command line](#command-line)
- [AI agents](#ai-agents)
- [Sync between devices](#sync-between-devices)
- [Architecture](#architecture)
- [Performance](#performance)
- [Verified, and not](#verified-and-not)
- [Not in version 1](#not-in-version-1)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [Licence](#licence)

## Highlights

<table>
  <tr>
    <td align="center" width="25%"><img src="docs/images/pixel/note.svg" alt="Pixel icon of a note page with a folded corner" width="48"><br><b>Plain Markdown</b><br><sub>A vault is a folder of <code>.md</code> files.</sub></td>
    <td align="center" width="25%"><img src="docs/images/pixel/graph.svg" alt="Pixel icon of three linked nodes" width="48"><br><b>Link graph</b><br><sub>Every <code>[[wikilink]]</code> becomes an edge.</sub></td>
    <td align="center" width="25%"><img src="docs/images/pixel/sync.svg" alt="Pixel icon of two devices with arrows between them" width="48"><br><b>P2P sync</b><br><sub>Windows and Linux, no server, no account.</sub></td>
    <td align="center" width="25%"><img src="docs/images/pixel/agent.svg" alt="Pixel icon of a robot head" width="48"><br><b>Agent gateway</b><br><sub>An MCP server and a CLI with <code>--json</code>.</sub></td>
  </tr>
</table>

- **Plain Markdown files.** A vault is a folder of `.md` files that stay readable by any other tool. `[[wikilinks]]`, backlinks and inline `#tags` are indexed; the files on disk are the source of truth, and the index, the graph and the sync state are derived from them.
- **A link graph.** A force-directed graph of the whole vault with pan, zoom, drag, hover highlighting and a keyboard-reachable list view; links to notes that do not exist yet appear as dimmer discs and a click creates the note.
- **Serverless P2P sync between Windows and Linux.** Devices pair with a ticket and sync directly over an end-to-end encrypted [iroh](https://www.iroh.computer) connection. Edits to the same note merge line by line, divergences become conflict copies, and nothing the sync removes is erased: it goes to the vault's trash.
- **Built for AI agents.** `ns mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdio with ten tools, and `ns` offers the same operations on the command line with `--json` output.
- **A calm, legible desktop app.** Glass tool areas over a neutral backdrop, a solid surface for the note, light and dark themes, text sizes from 75 % to 200 %, a visible focus ring, an accessible name on every field and symbol button and a test that every text colour reaches a WCAG 2 contrast of at least 4.5 : 1.
- **No silent data loss.** Creating a note never replaces an existing file, a file that cannot be read is «blocked» and never treated as deleted, and every path from the command line, MCP or a peer is validated and confined to the vault.
- **Two native programs.** `neurospace` (the app) and `ns` (the console tool), in four Rust crates. No web frontend, no other language in the product.
- **Designed for about 10 000 notes and 100 MB of text.** See [Performance](#performance) for the measurements and their conditions.

<p align="center">
  <img src="docs/images/pixel/divider.svg" alt="" width="640">
</p>

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/images/editor-dark.png" alt="The editor in the dark theme with the note Zettelkasten Method open, and its backlinks, outgoing links and tags on the right"></td>
    <td width="50%"><img src="docs/images/preview-light.png" alt="The rendered preview of the note Project Roadmap in the light theme, with checkboxes and links"></td>
  </tr>
  <tr>
    <td><sub><b>Editor</b> (dark). The note «Zettelkasten Method» with its backlinks, outgoing links and tags.</sub></td>
    <td><sub><b>Preview</b> (light). Rendered Markdown with checkboxes and clickable links.</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/images/graph-light.png" alt="The link graph of the demo vault in the light theme, four clusters of notes"></td>
    <td width="50%"><img src="docs/images/quick-open.png" alt="Quick open with the query re"></td>
  </tr>
  <tr>
    <td><sub><b>Graph</b> (light). The demo vault at rest after «Fit graph»: four clusters, and the grey, not-yet-created «Solar Panel Upgrade». The dark version is at the top of this page.</sub></td>
    <td><sub><b>Quick open</b> (Ctrl+O) filtering notes for «re».</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/images/sync-window.png" alt="The sync window in the dark theme, showing that sync is off"></td>
    <td width="50%"><img src="docs/images/welcome.png" alt="The welcome card in the light theme"></td>
  </tr>
  <tr>
    <td><sub><b>Sync window</b> (dark). «Sync is off», with no invite created.</sub></td>
    <td><sub><b>Welcome</b> (light). Where you pick a folder.</sub></td>
  </tr>
</table>

All images are real captures of the Linux release build running under Xvfb in a container, so the glass is the simulated one: the Windows Mica backdrop exists only on Windows and is not shown here. They show the demo vault in [`docs/demo-vault`](docs/demo-vault) (40 notes); [`scripts/screenshots.sh`](scripts/screenshots.sh) regenerates them (the command is in the script's header).

## Quick start

1. Download the release asset for your system from the [v0.1.0 release page](https://github.com/arturograntchiappini-bit/neurospace/releases/tag/v0.1.0) (while the repository is private, this needs a signed-in account with access):

   | System | Asset | Contents |
   | --- | --- | --- |
   | Windows | `neurospace-0.1.0-windows-x86_64.zip` | `neurospace.exe`, `ns.exe`, `README.md`, `LICENSE`, `Inter-LICENSE.txt` |
   | Linux | `neurospace-0.1.0-linux-x86_64.tar.gz` | one folder with `neurospace`, `ns`, `neurospace.desktop`, `neurospace.png`, `README.md`, `LICENSE`, `Inter-LICENSE.txt` |

2. **Windows:** unzip it, run `neurospace.exe`, and put `ns.exe` somewhere on your `PATH`. **Linux:** unpack it (`tar -xzf`) and run the install commands under [Install and build](#install-and-build).
3. Run `neurospace` and pick a folder of Markdown notes (or run `neurospace <VAULT_DIR>`). To look around first, open a copy of [`docs/demo-vault`](docs/demo-vault) from a checkout of this repository.
4. From a terminal, `ns --vault <VAULT_DIR> list` lists the notes of the same vault. To sync it with another device, see [Sync between devices](#sync-between-devices); to give an agent access, see [AI agents](#ai-agents).

Prefer to build it yourself? See below.

## Install and build

Building needs Rust 1.98 or newer (`rust-version = "1.98"` in `Cargo.toml`, the oldest toolchain tested).

**Windows.** Build from the repository root with `cargo build --release`. The two programs land in `target\release\`: `neurospace.exe` (the desktop app) and `ns.exe` (the command line, MCP server and headless sync). Put them anywhere on your `PATH`.

**Linux.** A script builds both programs in a Debian 11 container, so they run on any distribution with glibc 2.31 or newer (Debian 11 and later, Ubuntu 20.04 and later, Fedora 32 and later, and similar). From the repository root, in a Linux shell:

```sh
docker run --rm -v "$PWD:/src" -w /src -v ns-release-target:/target -v ns-release-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target rust:1-bullseye sh scripts/linux-release.sh
```

or in Windows PowerShell:

```powershell
docker run --rm -v "${PWD}:/src" -w /src -v ns-release-target:/target -v ns-release-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target rust:1-bullseye sh scripts/linux-release.sh
```

It writes `dist/neurospace-0.1.0-linux-x86_64.tar.gz` (about 29 MB) holding `neurospace`, `ns`, `neurospace.desktop`, `neurospace.png`, `README.md`, `LICENSE` and `Inter-LICENSE.txt`. The script refuses to pack a binary that needs a newer glibc than 2.31. To install for your user, with no root, unpack the tarball and run these from inside the unpacked folder:

```sh
install -Dm755 neurospace ~/.local/bin/neurospace
install -Dm755 ns ~/.local/bin/ns
mkdir -p ~/.local/share/applications
sed "s|^Exec=neurospace|Exec=$HOME/.local/bin/neurospace|" neurospace.desktop > ~/.local/share/applications/neurospace.desktop
install -Dm644 neurospace.png ~/.local/share/icons/hicolor/256x256/apps/neurospace.png
```

The `sed` line gives the menu entry the full path of the program, because a desktop session often does not have `~/.local/bin` on its `PATH` (on Ubuntu it only does after the first login that finds the folder). Make sure `~/.local/bin` is on your shell's `PATH` too. There is no installer and no `.deb`, AppImage or Flatpak package in version 1.

**What the Linux desktop app needs at run time.** An X11 or Wayland session and a working OpenGL 2.1 driver. «Open a folder» uses the xdg desktop portal; without one there is no folder dialog, and you can type the folder path instead.

## Using the app

`neurospace [VAULT_DIR]` opens the desktop window (`cargo run -p ns-app` from a checkout). Without an argument it reopens the last vault, or shows a welcome screen where you pick a folder.

### Layout

A top bar (sidebar toggle, New, Quick open, Graph, Preview/Edit, vault name, Sync, the menu, inspector toggle), the file tree and search on the left, the open note in the middle on a solid surface, backlinks, outgoing links and tags on the right, and a status strip at the bottom. The note is shown in a text editor with light colouring of headings, links and code, or as rendered Markdown where `[[wikilinks]]` are clickable; a link to a note that does not exist yet creates it, a link to an attachment (`![[image.png]]`, `[[report.pdf]]`) never creates a note and is shown as plain text. Only web links (`http`, `https`) open outside the app; in the rendered note every other link (a file, a network share, an unknown scheme, a footnote, table or list link included) is not opened and the status strip says so. The line width of a note is capped at 760 points.

### Shortcuts

| Keys | Action |
| --- | --- |
| Ctrl+N | New note |
| Ctrl+O | Quick open (Up and Down choose, Enter opens, Escape closes) |
| Ctrl+E | Switch between editor and preview |
| Ctrl+G | Graph |
| Ctrl+F | Search |
| Ctrl+Plus, Ctrl+Minus, Ctrl+0 | Text size |
| Home | «Fit graph», while the graph has the focus |
| Enter | «List view» (opens quick open), while the graph has the focus |
| Escape | Close the sync window |
| Tab | Reach every control |

### Graph

Ctrl+G or «Graph» replaces the note with a force-directed graph of the whole vault: notes are filled discs sized by number of links, a link to a note that does not exist yet is a dimmer disc with an outline (its label says «not created yet»), and the open note has a ring. Drag the background to pan, scroll to zoom about the cursor, drag a node to move it; hovering a node shows its label and dims what is not linked to it, and every label shows when zoomed in (from 150 %, as long as at most 500 notes are in view; «Fit graph» zooms in up to 200 %, so a small graph shows its labels). Clicking a note opens it in the editor; clicking a not-yet-created note creates it with the letter case the link wrote. «Fit graph» (or Home while the graph has the focus) brings every node into view. «List view» (or Enter on the focused graph) opens quick open, so every note can be reached without a mouse; opening any note ends the graph view. The layout is simulated (springs and repulsion); after a wake it animates for at most 10 seconds, then stays still until the next wake (a drag or a change of the vault); «Reduce motion» in the menu runs it to rest at once instead of animating.

<details>
<summary>Large vaults: how the graph behaves at thousands of notes</summary>

Repulsion uses a Barnes–Hut quadtree, and edges outside the view are not drawn. Above 10 000 notes the simulation is skipped (it would stall the window): the notes stay on a deterministic spiral, still pannable, zoomable and clickable, and the pane says «Large vault: simplified layout». The layout cools down on a fixed schedule and comes to rest by its own rule, and the view keeps fitting every note while it settles unless you have moved it. A vault whose notes form clusters separates into visible groups; a vault with no structure stays a round mass, but it stops moving. When more than 7 000 links would be on screen, only the links of the hovered note and of the open note are drawn and the pane says «Zoom in to see every link»; zooming in brings every link back. The measurements are under [Performance](#performance).

</details>

### Saving

There is no save button: the note is saved 500 ms after the last keystroke, when the window loses focus, when you open another note, when you open another vault and when the application closes (a failure while quitting is written to stderr). If the file changed on disk while you had unsaved edits (another device, an agent, another editor), nothing is overwritten: your text is written to `<name> (conflict <id>).md` next to the note and the note reloads with the file's text. If the note was removed on disk, your text is kept the same way. A save that fails (a read-only note, a full disk) keeps your text in the editor and shows the error; when you leave the note (another note, create, rename, delete, a link, another vault, closing the window) the text goes to a conflict note and the status strip says where. Only if that fails too (the folder is gone) do you stay on the note, and closing the window asks you to close again; the second request writes the text to `recovered/<time> <name>.md` in the state folder, prints that path to stderr and quits.

### Sync window

«Sync» in the top bar, or the sync summary at the right of the status strip («Sync off», «Sync: 2 of 3 peers online», …), opens the window; Escape closes it. It shows whether sync is on, this device's id, «Create invite» (the ticket appears in a read-only field with «Copy»; it is a password for your notes, share it privately), «Join» with a pasted ticket, «Turn sync off», the network mode (Internet or LAN only; changing it restarts the engine), the devices with the words «online» or «offline», notes the engine cannot sync marked «blocked» with the reason, and the relay reached. Starting the engine, creating an invite, joining and leaving run in the background: the window shows what is in progress and never freezes.

The app starts the sync engine for a vault only when that vault already has a sync group, or when you create an invite or join from this window. **A vault that never used sync never contacts the network.** If another process runs the engine for the vault (for example `ns sync run`), the window says so and shows its status read-only; the app does not start a second engine. Switching vaults and quitting stop the engine first (waiting up to 10 seconds).

### Look

The tool areas are glass over a neutral backdrop (the Windows Mica material where the system accepts it, a simulated glass elsewhere); «Reduce transparency» in the menu makes everything opaque, and the menu also sets the theme (System, Light, Dark) and the text size (75 % to 200 %). The font is Inter, used under the SIL Open Font License 1.1 ([`crates/ns-app/assets/Inter-LICENSE.txt`](crates/ns-app/assets/Inter-LICENSE.txt)).

### Requirements

Windows (tried on Windows 11 only) or a Linux desktop (X11 or Wayland) with a working OpenGL 2.1 driver and, for «Open a folder», an xdg desktop portal. The window is drawn with OpenGL (`eframe` with `glow`), so a machine without one, such as a bare virtual machine or a remote session without 3D, may fail to open it. The Windows Mica backdrop needs Windows 11; elsewhere the glass is simulated. macOS is not supported.

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

## AI agents

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

**Claude Code:**

```sh
claude mcp add neurospace -- ns --vault /path/to/vault mcp
```

**Claude Desktop**, in `claude_desktop_config.json`:

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

The two snippets follow each client's documented format; neither client has been tried against `ns mcp` yet. The server is a local stdio process, so it needs no public address and no account. A remote (HTTP) server and claude.ai custom connectors are not offered (see [Not in version 1](#not-in-version-1)).

## Sync between devices

Your devices sync their vaults directly with each other, with no server of yours and no account. It works between Windows and Linux; macOS is not supported. The sync engine runs in the foreground in a terminal with `ns sync run`; the desktop app also starts it for a vault that has a sync group (see [Sync window](#sync-window)).

```mermaid
sequenceDiagram
    autonumber
    participant A as Device A
    participant B as Device B
    Note over A: ns sync run, then ns sync invite prints a ticket
    A-->>B: ticket, shared over a private channel
    Note over B: ns sync join TICKET, then ns sync run
    B->>A: dial by device id or by the address hints in the ticket
    A->>B: proof that it knows the group secret
    B->>A: proof that it knows the group secret
    Note over A: you save a note
    A->>A: folder watcher wakes the engine
    A->>A: round reconciles the disk with the sync document, a loro CRDT
    A->>B: sync document updates over the iroh connection
    B->>B: merge into its own sync document
    B->>B: reconcile to disk
    Note over B: note updated, or a conflict copy, or the old text goes to .neurospace/trash
```

### What is synced

Valid notes: the `.md` files `ns list` shows. Files that `ns list` ignores (not UTF-8, over 4 MiB, invalid names), attachments and other file types are not synced. Edits to the same note on two devices are merged line by line. If two devices end up with different notes at the same path, both are kept and one becomes `<name> (conflict <id>).md` next to the original. A note that another device deleted is moved to `.neurospace/trash/` in your vault, never erased; a file that cannot be read is skipped, never treated as deleted. Syncing is designed for about 10 000 notes and 100 MB of text; the measurements, including Windows to Linux, are under [Performance](#performance). The edit history is kept for as long as the group exists and is never compacted: deleting a note removes it from every vault but not from the sync history.

### Pairing, in one direction only

1. On the first device: `ns sync run` (leave it running). Its first lines on stderr give the device id and the addresses it listens on.
2. On the same device, in another terminal: `ns sync invite`. It prints a ticket (`ns1/...`). With the engine running, the ticket carries the device's addresses as hints; without it, `invite` still works, but the ticket has none.
3. On the second device: `ns sync join <TICKET>`, then `ns sync run`. The second device dials the first, and from then on both sync whenever both are online.

A ticket is a password: anyone who has it joins the group and receives every note. Share it only over a private channel, and run `ns sync leave` on a device you no longer trust. Do not make two devices join each other's tickets at the same time: each then takes the other's secret and they never sync. If that happened, run `ns sync join` again on one of them with a fresh ticket from the other.

`ns sync join` and `ns sync leave` act at once when no engine runs, and otherwise hand the command to the running engine, which carries it out within a few seconds. `ns sync leave` turns sync off for this vault: the notes stay, the group secret and the sync history on this device are deleted, and a running engine stops. Notes deleted in the group while this device was away come back as new notes if it joins again. Moving or renaming the vault folder (or a drive-letter change for a vault on a removable drive) makes it look unconfigured: `ns sync run` then starts a new group of one and says so on stderr, and the device needs a new ticket (`ns sync join`). `ns sync status` shows the running engine's device id, addresses, relay (in `internet` mode), peers, note count, blocked notes and last error (`--json` for the raw document), or `engine not running`.

### Modes

The mode is set in `config.json` in the vault's state directory (see below): `{"mode":"internet"}` (the default) or `{"mode":"lan"}`. Optional keys: `"mdns": false` turns off discovery on the local network, `"bind_port": 4040` fixes the port. `config.json` is read once, when `ns sync run` starts: a change needs `ns sync run` to be restarted.

| Mode | How devices find each other | What leaves your network |
| --- | --- | --- |
| `internet` (default) | By device id, through the public infrastructure of the iroh project: a DNS discovery service (`dns.iroh.link`) and relay servers (`use1-1`, `usw1-1`, `euc1-1` and `aps1-1.relay.n0.iroh.link`) that help to cross NATs. The router port mapping (UPnP, PCP, NAT-PMP) is on; it only talks to your own router. | Device ids and IP addresses are visible to the iroh servers, run by the iroh project and not by you. Note content never is: every connection is end-to-end encrypted. |
| `lan` | By the address hints in the ticket. The router port mapping is off. | Nothing outside the local network is contacted. |

In both modes, devices on the same network also find each other by mDNS, unless `"mdns": false`. Where mDNS does not work (turned off, across subnets, some Wi-Fi), set the same fixed `bind_port` on every device, or paired devices will not find each other after a restart: the address hints in a ticket only name the port the engine had then.

Verification status of `internet`: on the development machine (Windows 11) a bare endpoint with iroh's default preset, run by a test and not by `ns sync run`, came online in three runs on 2026-11-17 (after about 25.6 s, under 15.5 s and 0.7 s); an earlier test on 2026-11-15 never came online, cause unknown. Sync between two devices on different networks through this mode has not been verified. Verification status of `lan`: discovery between two real machines is not verified; the tests pair two processes on one machine through loopback.

**Troubleshooting.** In `internet` mode, `ns sync status` shows the relay the device reached. If it stays at "none yet", something on this computer or network blocks `ns` from going out (an application firewall, a corporate filter): allow `ns` (and later `neurospace`) outbound HTTPS and UDP. `lan` mode with address hints is not affected.

### State directory

The engine keeps its state outside the vault, in `<base>/vaults/<one folder per vault>`, where `<base>` is `%LOCALAPPDATA%\neurospace` on Windows, `$XDG_DATA_HOME/neurospace` or `~/.local/share/neurospace` on Linux, or the folder in the `NS_STATE_DIR` environment variable. Each of these folders holds a `vault-path.txt` that names its vault, so you can tell them apart. A folder holds the device key (`device.key`), the group secret (`secret`), the sync history and `status.json`, created readable by their owner only on Linux (`ns sync run` and `ns sync invite` also create `device.key` and `secret` when they are missing; `ns sync join` writes the group's `secret` and does not create `device.key`), and `config.json`, which you write yourself. Only one engine can run per vault: a second `ns sync run` fails with "already running".

`ns sync run` writes its log to stderr (never the ticket or the secret) and nothing to stdout. Ctrl+C (and SIGTERM on Linux) stops it cleanly with exit code 0, as does `ns sync leave` from another terminal (tested on Linux with SIGTERM; on Windows the same shutdown path runs, but Ctrl+C itself was not tested).

<details>
<summary>The status file (<code>status.json</code>)</summary>

`status.json` is rewritten by the engine at every tick and at the end of every round, and at least every 5 seconds while a long round runs (the first round of a large vault can take a while), so a reader sees a status at most about 5 seconds old; `ns sync status` and the app's sync window show it as it is and never ask the engine. Besides the device id, addresses, peers, note count, blocked notes and relay, it carries `mode` (`internet` or `lan`, the mode the engine was started with: `ns sync status` does not read `config.json`), `busy` (`true` while a round runs; the other fields are then from the moment it began, and `ns sync status` says «a sync round is running»), `notices` (conditions that hold until they end: watching the folder is off, a problem of the last round such as path claims that did not settle, something repaired when the state was opened, a full device list) and `last_error` (the last error only, cleared by the next good round). Notices never contain the group secret or a ticket.

</details>

<p align="center">
  <img src="docs/images/pixel/divider.svg" alt="" width="640">
</p>

## Architecture

```mermaid
flowchart LR
    app["ns-app<br/>neurospace, the desktop app"]
    cli["ns-cli<br/>ns, CLI, MCP server and headless sync"]
    sync["ns-sync<br/>peer-to-peer sync engine"]
    core["ns-core<br/>vault, index, link graph, operations"]
    app --> core
    app --> sync
    cli --> core
    cli --> sync
    sync --> core
```

| Crate | Produces | Role |
| --- | --- | --- |
| `ns-core` | library | Vault operations on disk, path validation and confinement, the trash, the Markdown and link index, the graph and search, and the operations shared by the CLI, MCP and the app. |
| `ns-sync` | library | The sync engine: the CRDT sync document and its rules for notes that are alive or deleted, the planner that reconciles disk and document, the authenticated protocol between peers, and the `iroh` transport. |
| `ns-cli` | `ns` | The console program: note commands, the MCP server and headless sync. |
| `ns-app` | `neurospace` | The desktop app. |

| Concern | Library |
| --- | --- |
| Desktop UI | `eframe` and `egui` 0.37 with the OpenGL renderer (`glow`) |
| Markdown preview | `egui_commonmark` 0.26 |
| Markdown parsing | `pulldown-cmark` 0.13 |
| Sync document (CRDT, line-by-line merge) | `loro` 1.17 |
| Transport | `iroh` 1.4, with `iroh-mdns-address-lookup` for discovery on the local network |
| Folder watching | `notify` 8.2 |
| MCP server | `rmcp` 3.10 |
| Command line | `clap` 4 |

**Source of truth.** The `.md` files on disk are the truth. The index, the graph and the sync state are derived from them and can be rebuilt. The sync state lives outside the vault, in a per-user data folder (see [State directory](#state-directory)); inside the vault only `.neurospace/trash/` is kept. One process per vault runs the sync engine (a lock file enforces it), and the note commands and MCP tools only read and write note files. Every path that arrives from the command line, MCP or a peer is validated and confined to the vault root before it touches the disk. Nothing is lost silently: what sync removes goes to the trash, a divergence becomes a conflict copy, an unreadable file is blocked and never read as absent, and creating a note never replaces an existing file.

## Performance

Neurospace is designed for a vault of about 10 000 notes and 100 MB of text; a note larger than 4 MiB is ignored. The numbers below were measured, with the conditions that matter. They are not a promise for other machines.

**Sync** (10 000 notes, about 100 MB):

| What | Result | Conditions |
| --- | --- | --- |
| One change reaching the other device | 0.66 s median of 12, at most 0.72 s (about 4.2 s before the sync engine was made incremental) | two release `ns sync run` processes, `lan` mode, Windows, 102.9 MB |
| A change between Windows and Linux | 0.21 to 0.36 s from Linux to Windows, 0.46 to 0.51 s from Windows to Linux; a note deleted on Windows landed in the Linux vault's `.neurospace/trash/` | `lan` mode, five edits each way |
| First round on a vault Windows has already scanned | 4.6 to 5.6 s | `ns sync run` |
| First round on a vault just written, copied or restored | about 2 minutes | Windows Defender scans each file at its first read, about 12 to 15 ms per note; this cost is outside Neurospace and reading in parallel does not help |
| Copy of the whole vault to an empty device | 35 s | two release processes, `lan` mode, Windows, 102.9 MB |
| A round with nothing to do, one note changed, a round applying a remote change | 56 to 58 ms, 128 ms, 207 ms | timing test in release, about 100 MB, machine under load |
| CPU at rest | 0.00 to 0.02 s of CPU over 30 s | two release processes, `lan` mode |
| Sync state on disk | 108 MB per device | same run |

**Desktop app, Windows release build, 10 000 notes:**

| What | Result |
| --- | --- |
| Opening the vault (with a folder of 5 000 notes) | about 0.3 s |
| Scrolling the folder of 5 000 notes | about 10 ms per mouse-wheel step, at most 70 ms |

**Graph at 10 000 notes** (about 20 random links per note, the headless release tests; a random graph cannot be made readable by any layout, so the claim is only that it comes to rest):

| What | Result |
| --- | --- |
| Layout comes to rest by its own rule | after about 250 steps |
| Frame time while it settles, layout step and painting together | median 14 to 23 ms; the first frame takes up to about 0.8 s while the graph is built |
| Painting one frame | about 3 ms (it was 22 ms) |
| Memory with the graph open | about 280 MB resident in the Linux release app under Xvfb with Mesa software OpenGL, which did not grow while the layout animated |

On Windows the working set of the graph was last measured before node drawing was cut from about 12.7 MB to about 1 MB of mesh data per frame: it peaked around 760 MB while animating, and it has not been measured again since. Graph clusters (a vault whose notes form groups) separate into visible groups. The 10-second animation cap, the simplified layout above 10 000 notes and the link thinning above 7 000 links are described under [Graph](#graph).

All of these were taken on a development machine with other applications running (load up to 79 %), not on an idle one.

## Verified, and not

**Verified**

- The unit tests of the state module and the graph layout, and the headless UI tests (`egui_kittest`), which drive the real UI code from the welcome screen through creating, editing, renaming, deleting and following notes, search, the graph, an external edit, the sync window and the exit path.
- The contrast of every colour pair (WCAG 2, at least 4.5 : 1 for text).
- The whole workspace test suite on Windows 11 and in a Linux container (`rust:1-bullseye`, rustc 1.98), all green, with the permission-sensitive tests also run as an unprivileged user.
- The Linux release tarball was installed with the commands above on `debian:bookworm` (glibc 2.36): `desktop-file-validate` accepts the `.desktop` file, `ns --version` prints `ns 0.1.0`, and `neurospace` started under Xvfb with Mesa software OpenGL, opened a three-note vault with the simulated glass, wrote nothing to stderr and stopped on SIGTERM.
- Windows and Linux synced both ways in the test described under [Performance](#performance).

**Not verified**

- No person has used the GUI interactively: dragging, the feel of the Mica material, a Linux desktop with a real display.
- The Linux GUI ran only under Xvfb with software OpenGL, not on a physical display, under Wayland or from a desktop menu.
- The folder dialog («Open a folder») was compiled but never opened.
- Windows 10 (only Windows 11 was tried).
- The MCP server with the real Claude Code and Claude Desktop clients: it was tested with hand-written JSON-RPC requests and with the `rmcp` test client.
- The `internet` mode between two real different networks, and mDNS between two real machines.
- Sync started from the GUI between two real machines: the GUI tests use a fake sync control and never start the engine or touch the network.
- The graph's memory on Windows after the node drawing change, and its rest by the layout's own rule in a GUI with a hardware GPU (shown only by the headless release tests; under Linux software OpenGL the 10-second cap ended the animation).
- On Windows the app's first «Opening the vault…» on a vault just written or copied, or right after installing a new build, can take a minute or more (seen, not proven to be Defender).

## Not in version 1

These are left out on purpose:

- macOS, mobile and web versions.
- Syncing attachments (images, PDFs) and empty folders: only `.md` notes sync. The preview does not show images.
- Plugins, themes, canvas, live-preview (WYSIWYG) editing, several tabs or panes, vim mode.
- Rewriting links when a note is renamed.
- Tags written in YAML front matter: only inline `#tags` are indexed.
- Carrying a case-only rename (`note.md` to `Note.md`) to other devices.
- Removing another device from a sync group, or changing the group secret: a device can only leave by itself.
- Shrinking the sync history, and vaults much larger than about 10 000 notes and 100 MB.
- A remote (HTTP) MCP server and claude.ai custom connectors, which need a public address and so contradict the serverless goal; the stdio server covers Claude Code and Claude Desktop.
- Installers, AppImage, auto-update.
- Encryption of notes on disk: they are plain files, as in Obsidian.

<p align="center">
  <img src="docs/images/pixel/divider.svg" alt="" width="640">
</p>

## Roadmap

<p align="center">
  <img src="docs/images/pixel/level.svg" alt="Progress bar: level 1 reached, further levels still to come" width="420">
</p>

This is level 1. Further levels of the architecture are planned; what is listed below is what level 1 left open.

There is no schedule and no promise. The limits above are where a later version could go if there is a reason to:

- Packaging for Linux beyond the tarball (`.deb`, AppImage, Flatpak) and an installer for Windows.
- Syncing attachments and empty folders, and showing images in the preview.
- Rewriting links on rename, and reading tags from YAML front matter.
- Removing a device from a group and rotating the group secret; compacting the sync history.
- Testing what is still open: the app on a physical display and under Wayland, Windows 10, `internet` mode and mDNS between real machines.

## Contributing

Keep the three checks below green; CI runs them on Ubuntu and Windows with Rust 1.98.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The standards the code follows:

- No `unsafe`; no `unwrap()`, `expect()` or `panic!` outside tests (the workspace lints deny them).
- Doc comments on public items (`missing_docs`); functions of at most 30 lines and 4 parameters (`clippy.toml`).
- Test names say the behaviour: `should_return_error_when_input_is_empty`.
- Commits follow [Conventional Commits](https://www.conventionalcommits.org): `<type>(<scope>): <description>` with `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore` or `ci`, in the imperative mood.
- Every change that users can see goes in [`CHANGELOG.md`](CHANGELOG.md) under `[Unreleased]`, in the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Licence

Neurospace is released under the [MIT licence](LICENSE). The Inter font embedded in the app is used under the SIL Open Font License 1.1; its text is in [`crates/ns-app/assets/Inter-LICENSE.txt`](crates/ns-app/assets/Inter-LICENSE.txt) and ships in both release archives; the MIT `LICENSE` ships in both release archives as well.

<p align="center">
  <img src="docs/images/pixel/divider.svg" alt="" width="640">
  <br>
  <sub>Made with Rust · MIT</sub>
</p>
