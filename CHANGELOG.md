# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The Neurospace logo (`assets/logo/neurospace.svg`) as the window and taskbar icon, the icon of `neurospace.exe` on Windows and the logo of the welcome screen.
- `ns` command line: `list`, `read`, `write`, `append`, `delete`, `rename`, `search`, `links`, `graph` and `tags`, with text or `--json` output.
- `ns mcp`: an MCP stdio server exposing the note operations as ten tools (`list_notes`, `read_note`, `write_note`, `append_to_note`, `delete_note`, `rename_note`, `search_notes`, `get_links`, `get_graph`, `list_tags`) for AI agents.
- Sync engine (`ns-sync`): peer-to-peer sync of a vault between devices over `iroh`, with no server of its own; a CRDT document merges edits line by line, concurrent differences become conflict copies, removals go to the vault's trash, and a file that cannot be read is blocked instead of deleted. Modes `internet` (default) and `lan`.
- `ns sync run`, `invite`, `join <TICKET>`, `leave` and `status`: pair devices with a ticket, run the engine in the foreground (log on stderr, clean stop on Ctrl+C or SIGTERM) and inspect it, with `--json` output.
- `ns sync status` shows the relay the device reached (`relay: <url>`, or a hint to check the firewall in `internet` mode when there is none yet), and `status.json` and `ns sync status --json` carry the number of rejected entries of the sync document.
- Desktop app `neurospace` (new crate `ns-app`, `eframe`/`egui` with OpenGL): a welcome screen, then a window with a top bar, the file tree (folders start closed and only the rows in view are drawn, so a 10 000-note vault scrolls smoothly; long names are cut short) and search on the left, the open note in the middle, backlinks, outgoing links and tags on the right and a status strip. `neurospace [VAULT_DIR]` reopens the last vault when no folder is given.
- Notes: an editor with Markdown colouring, a preview with clickable wikilinks (a link to a note that does not exist yet creates it, an attachment link never does, only `http` and `https` links open outside the app), quick open, create, rename and delete (to the vault's trash) from the tree, and the shortcuts Ctrl+N, Ctrl+O, Ctrl+E, Ctrl+G, Ctrl+F and Ctrl+Plus, Ctrl+Minus, Ctrl+0.
- Saving: autosave 500 ms after the last keystroke, on focus loss, on switching notes, on opening another vault and on exit. The vault folder is followed, so an external edit of the open note appears shortly after; unsaved edits that meet a changed or removed file go to a `<name> (conflict <id>).md` note, also when the window closes or another vault is opened. A save that fails keeps the text in the editor and shows the error; when the note is left (another note, create, rename, delete, a link, another vault, closing the window) the text goes to a conflict note and the status strip says where, so a read-only note or a full disk never traps you or drops text. If the conflict note cannot be written either, you stay on the note, and a second close writes the text to `recovered/` in the state folder. Text typed into a note that had changed on disk is no longer dropped on exit; it is kept in a conflict note, once.
- Graph view (Ctrl+G or «Graph»): a force-directed layout with pan, zoom about the cursor, node dragging, hover highlighting, labels when zoomed in, «Fit graph» (and Home), «List view» (and Enter) for keyboard users, a click that opens a note for editing or creates a not-yet-created one (creating a note from the New dialog also leaves the graph). The repulsion uses a Barnes–Hut quadtree, edges outside the view are not drawn, and above 10 000 notes the layout is not simulated («Large vault: simplified layout»); the layout stops after 1500 steps at most. Graph links have their own colour token with at least 3 : 1 contrast against the document surface. The «Reduce motion» setting (`reduce_motion` in `app.json`) runs the layout to rest at once.
- Sync window («Sync» in the top bar or the status strip): create an invite (ticket in a read-only field with «Copy»), join a pasted ticket, turn sync off, switch between Internet and LAN only, and see this device's id, the peers, blocked notes with their reason and the relay. The engine runs on a background thread and starts with a vault only when it already has a sync group (or when you invite or join), so a vault that never used sync never contacts the network; when another process runs the engine the window is read-only; switching vault and exit stop the engine (waiting up to 10 seconds). The ticket never appears in notices, logs or `Debug` output. «Turn sync off» never starts the network, a mistyped ticket is rejected before the engine starts, the window notices an engine that ended by itself, a quick vault switch or quit also stops an engine that was still starting, and the busy spinner is skipped with «Reduce motion».
- Look and accessibility: glass tool areas over a neutral backdrop (Windows Mica where the system accepts it, simulated glass elsewhere), a solid document surface, «Reduce transparency», theme System, Light or Dark, text size 75 % to 200 %, Inter 4.1 embedded under the SIL Open Font License 1.1, a visible focus ring, an accessible name on every field and symbol button, nothing conveyed by colour alone, and a test that every text colour has a WCAG 2 contrast of at least 4.5 : 1 on every surface it can meet.
- Quitting and opening another vault run the same steps (save the open note or keep it in a conflict note, save the settings, stop the sync engine); a failure while quitting is written to stderr, a failure of both the save and the conflict note while opening another vault keeps you on the vault.
- `ns_core::ops::create_note` (never replaces a note, adopts existing folder spelling), `new_note_path` (the path a click on an unresolved link creates, spec 6.6), `ATTACHMENT_EXTENSIONS` with `Link::is_attachment` and `is_attachment_target`, and `StateDir::save_config` in `ns-sync`.
- Headless UI tests (`egui_kittest`) that drive the real UI from the welcome screen: note lifecycle, search and graph, external edit, sync window and the exit path.

### Changed

- The project is renamed from Second Brain to Neurospace: binaries `neurospace` and `ns`, crates `ns-*`, vault folder `.neurospace/`, `NS_*` environment variables, tickets `ns1/`.
- Desktop app: a Markdown link to a note (`[text](Other.md#heading)`, percent-decoded) now opens that note from the preview instead of saying only web links open; «Turn sync off» is also offered when the engine failed to start or ended, and the window says «Sync is not running: …»; when another process released the engine (for example `ns sync run` was stopped) the app takes it over at the next status refresh; the folder watcher starts before the index is built, so a change made during opening is not missed.
- Desktop app: saving the open note no longer recomputes its backlinks or reruns the search at once; only the note's own links and tags are refreshed, and backlinks and search results update when another note may have changed (create, rename, delete, conflict note, rebuilt index) or the open note changes. With the folder watcher running they are still recomputed once after the background index rebuild that follows each save (which a later release will remove with an incremental index), so the typing hitch at 10 000 notes is delayed, not gone. Search results for the open note itself are refreshed at the next such change or query.
- Graph view: «Fit graph» zooms in up to 200 % so the labels of a small graph show.
- Graph view: «Fit graph» can zoom out to 5 % (it stopped at 20 %), so every node of a vault of several thousand notes shows; with «Reduce motion» one frame spends at most 200 ms (it was 500 ms) on layout steps before the next frame is drawn.
- Attachment links (`![[image.png]]`, `[[report.pdf]]`) are no longer ghost notes in the graph: they produce no node and no edge.
- The ghost of a `./` link is named by the note a click would create, so `[[./new]]` in two folders gives two ghosts; ghosts of plain names and of `/` links are unchanged. A link for which no valid note path exists produces no ghost.
- Reading a note larger than 4 MiB is refused instead of loading it; writing over a note that is a symbolic link is refused; replacing a note keeps its file permissions.
- The folder filter of `list` and `list_notes` is validated like a note path (`Projects\Sub`, `../x`, `./projects` and hidden folders are errors) and accepts one trailing `/`.
- Note paths reject more Windows device names (`CON .md` with trailing spaces, `COM0`, `LPT0`, `COM¹`–`COM³`, `LPT¹`–`LPT³`, `CONIN$`, `CONOUT$`), because a Linux peer may legally hold such names.
- Creating a note writes a temporary file and links it into place, so a crash never leaves a half-written file under the note's name.
- Writing or appending to a read-only note is refused with a permission error on every platform; it used to replace the file on Linux.
- Sync cost: an update message without updates no longer triggers two full rescans, the note count in the status comes from the last round, and the rename matching of the planner and the claim resolution of the document no longer scan or hash the whole vault for every note.
- Sync robustness: a message over the 1 GiB frame limit is reported instead of silently ending sessions; a failed compaction no longer stops files from being written; the engine starts without a file watcher (and says so) when watching fails, and makes a new one after the vault came back; a new session gets nothing before its `Version` arrived; a failed read of the group secret heals at the next tick; `peers.json` holds at most 256 peers; a version with trailing bytes is rejected.
- Housekeeping: `loro` is pinned to `~1.17`, the workspace declares `rust-version = "1.98"` (the oldest toolchain tested), `ns sync run` retries for one second while `ns sync invite` holds the lock, a stop request no longer waits for the first status, and `ns sync run` says when it creates a new group.
- MCP tool descriptions now state limits, result shapes and that rename does not rewrite links; read-only and destructive tools carry annotations.

### Fixed

- Renaming a note onto an existing file could replace it without a trace (on Windows, by naming it with its 8.3 short name); rename now refuses whenever anything is at the destination.
- One folder that could not be listed (access denied) made list, search, links, graph and tags fail; it is now reported as ignored and the scan goes on.
- Two MCP tool calls at the same moment could lose text (`append_to_note` on the same note); tool calls now run one at a time.
- A `[[Note#Heading\|Alias]]` link inside a table kept a backslash in its heading.
- Sync read a vault that disappeared during a scan (a pulled drive, an unmount) as "every note deleted" and sent the deletions to the other devices; the marker folder is now checked again after the scan. A note whose name differs only in case from another is blocked instead of taking over the synced note.
- `ns sync leave` now works when the vault lost its `.neurospace` folder or a state file is corrupt, an unreadable `base.json` is reset instead of stopping sync, and the "folder is missing" error says how to get out.

### Security

- The sync state files (device key, group secret, history) are readable by their owner only on Linux.
- A sync ticket contains the group secret and is a password: whoever holds it can join the group and read and write every note. `ns sync run` never logs a ticket or the secret.
