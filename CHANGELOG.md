# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `sb` command line: `list`, `read`, `write`, `append`, `delete`, `rename`, `search`, `links`, `graph` and `tags`, with text or `--json` output.
- `sb mcp`: an MCP stdio server exposing the note operations as ten tools (`list_notes`, `read_note`, `write_note`, `append_to_note`, `delete_note`, `rename_note`, `search_notes`, `get_links`, `get_graph`, `list_tags`) for AI agents.
- Sync engine (`sb-sync`): peer-to-peer sync of a vault between devices over `iroh`, with no server of its own; a CRDT document merges edits line by line, concurrent differences become conflict copies, removals go to the vault's trash, and a file that cannot be read is blocked instead of deleted. Modes `internet` (default) and `lan`.
- `sb sync run`, `invite`, `join <TICKET>`, `leave` and `status`: pair devices with a ticket, run the engine in the foreground (log on stderr, clean stop on Ctrl+C or SIGTERM) and inspect it, with `--json` output.
- `sb sync status` shows the relay the device reached (`relay: <url>`, or a hint to check the firewall in `internet` mode when there is none yet), and the number of rejected entries of the sync document; `status.json` carries both.

### Changed

- Reading a note larger than 4 MiB is refused instead of loading it; writing over a note that is a symbolic link is refused; replacing a note keeps its file permissions.
- The folder filter of `list` and `list_notes` is validated like a note path (`Projects\Sub`, `../x`, `./projects` and hidden folders are errors) and accepts one trailing `/`.
- Note paths reject more Windows device names (`CON .md` with trailing spaces, `COM0`, `LPT0`, `COM¹`–`COM³`, `LPT¹`–`LPT³`, `CONIN$`, `CONOUT$`), because a Linux peer may legally hold such names.
- Creating a note writes a temporary file and links it into place, so a crash never leaves a half-written file under the note's name.
- Writing or appending to a read-only note is refused with a permission error on every platform; it used to replace the file on Linux.
- Sync cost: an update message without updates no longer triggers two full rescans, the note count in the status comes from the last round, and the rename matching of the planner and the claim resolution of the document no longer scan or hash the whole vault for every note.
- Sync robustness: a message over the 1 GiB frame limit is reported instead of silently ending sessions; a failed compaction no longer stops files from being written; the engine starts without a file watcher (and says so) when watching fails, and makes a new one after the vault came back; a new session gets nothing before its `Version` arrived; a failed read of the group secret heals at the next tick; `peers.json` holds at most 256 peers; a version with trailing bytes is rejected.
- Housekeeping: `loro` is pinned to `~1.17`, the workspace declares `rust-version = "1.98"` (the oldest toolchain tested), `sb sync run` retries for one second while `sb sync invite` holds the lock, a stop request no longer waits for the first status, and `sb sync run` says when it creates a new group.
- MCP tool descriptions now state limits, result shapes and that rename does not rewrite links; read-only and destructive tools carry annotations.

### Fixed

- Renaming a note onto an existing file could replace it without a trace (on Windows, by naming it with its 8.3 short name); rename now refuses whenever anything is at the destination.
- One folder that could not be listed (access denied) made list, search, links, graph and tags fail; it is now reported as ignored and the scan goes on.
- Two MCP tool calls at the same moment could lose text (`append_to_note` on the same note); tool calls now run one at a time.
- A `[[Note#Heading\|Alias]]` link inside a table kept a backslash in its heading.
- Sync read a vault that disappeared during a scan (a pulled drive, an unmount) as "every note deleted" and sent the deletions to the other devices; the marker folder is now checked again after the scan. A note whose name differs only in case from another is blocked instead of taking over the synced note.
- `sb sync leave` now works when the vault lost its `.secondbrain` folder or a state file is corrupt, an unreadable `base.json` is reset instead of stopping sync, and the "folder is missing" error says how to get out.

### Security

- The sync state files (device key, group secret, history) are readable by their owner only on Linux.
- A sync ticket contains the group secret and is a password: whoever holds it can join the group and read and write every note. `sb sync run` never logs a ticket or the secret.
