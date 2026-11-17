# Second Brain — Design

Date: 2026-11-15
Status: approved under the user's delegation of 2026-11-15 (see `CLAUDE.md`, «Verità assoluta»). Every choice below that the user did not state is the orchestrator's call and is marked **(call)** where it first appears.

## 1. Intent

A note-taking application in the spirit of Obsidian: a folder of Markdown files, links between notes, a graph of those links, a simple desktop UI. Two things set it apart:

- **Devices sync directly with each other.** No account, no server the user runs, no server that stores notes. It must work between Windows and Linux.
- **AI agents are first-class users.** An agent reads, searches, writes and navigates the vault through a native gateway (MCP server and CLI), not by scraping a UI.

The user's words: «un'applicazione per prendere appunti, come obsidian, su base markdown e grafi, sincronizzazione p2p serverless tra dispositivi (anche tra os diversi come windows e linux (quasi ogni distro), ma niente mac), come ui per ora falla simile a obsidian non troppo complicata. crea l'applicazione in full rust, e con una porta di accesso nativa per agenti ai, mcp o cli o connettori custom per claude.»

## 2. Goals, non-goals, anti-goals

### Goals

1. A vault is a plain folder of `.md` files. An existing Obsidian vault opens as is.
2. `[[wikilinks]]`, backlinks, tags, full-text search and a graph view.
3. A desktop GUI for Windows and Linux with the familiar three-pane layout.
4. Peer-to-peer sync between the user's devices, across Windows and Linux, with no silent data loss.
5. A console binary giving agents and scripts the same operations as the GUI: CLI commands and an MCP stdio server.
6. Everything written in Rust.

### Non-goals (version 1)

Named so nobody builds them by accident:

- macOS, mobile, web.
- Syncing attachments (images, PDFs) and empty folders. Only `.md` notes sync.
- Rendering images in the preview.
- Plugins, themes, canvas, live-preview (WYSIWYG) editing, multiple tabs or panes, vim mode.
- Rewriting links when a note is renamed.
- Tags declared in YAML front matter (only inline `#tags` are indexed).
- Propagating case-only renames (`note.md` → `Note.md`) between devices.
- Removing another device from a sync group, or rotating the group secret. A device can only leave by itself.
- Compacting the sync history, and syncing vaults beyond the size stated in section 8.1.
- A remote (HTTP) MCP transport and claude.ai «custom connectors», which need a public URL and so contradict the serverless goal. The stdio server covers Claude Code and Claude Desktop.
- Installers, AppImage, auto-update, CI pipelines.
- End-to-end encryption at rest. Notes are plain files on disk, as in Obsidian.

### Anti-goals

Things that would make the product a failure even if it «works»:

- A sync that can silently lose or duplicate text.
- A vault format that only this application can read.
- An agent gateway that can read or write outside the vault folder.
- A GUI that holds logic the CLI and MCP server cannot reach.

## 3. Constraints

- **Language:** Rust only, stable toolchain (developed on 1.99). No `unsafe` in project code.
- **Platforms:** Windows 10/11 x86_64 and Linux x86_64 (glibc ≥ 2.31, X11 or Wayland). Not macOS.
- **Process:** workflow of `CLAUDE.md` §11. Implementers are Sonnet or Haiku subagents; the orchestrator writes no code.
- **Code standards:** `CLAUDE.md` §1, §5, §9. Concretely: `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean; doc comments on every public item; no `unwrap()`/`expect()`/`panic!` outside tests; functions of about 20 lines; Conventional Commits.
- **Dependencies:** as few as the job needs. A crate is added only when it replaces more than a few lines of code we would otherwise own.

## 4. Architecture

### 4.1 Process model (call)

**The files on disk are the truth.** Index, graph and sync state are derived from them and can be rebuilt.

That one rule lets every actor coexist without talking to each other:

```
 GUI (secondbrain)      CLI / MCP (sb)        any editor, any agent
        │                     │                        │
        └──── read / write plain .md files in the vault folder ────┘
                              ▲
                              │ watches the folder, reconciles
                     sync engine (exactly one per vault)
                              │
                     other devices (peer-to-peer)
```

- The GUI, the CLI, the MCP server and external editors only ever read and write Markdown files.
- The **sync engine** is the only writer of sync state. It runs inside the GUI process, or headless as `sb sync run`. A lock file guarantees one engine per vault.
- The engine is controlled through files in its state directory (section 8.3), so the CLI needs no IPC to a running engine.

### 4.2 Workspace layout (call)

```
Cargo.toml                 workspace
crates/
  sb-core/                 lib: note paths, vault file operations, Markdown parsing,
                           index, link resolution, graph, search, folder watching.
                           No GUI, no network.
  sb-sync/                 lib: sync document, reconciliation, peer protocol, engine,
                           state directory, tickets, network adapter.
  sb-cli/                  bin `sb`: CLI commands, MCP stdio server, headless sync.
  sb-app/                  bin `secondbrain`: desktop GUI.
docs/                      spec, plans
scripts/                   Linux build and cross-OS sync check (Docker)
README.md  CHANGELOG.md
```

Two binaries because one Windows executable cannot cleanly be both a console program and a GUI program.

`sb-core` is the domain core together with `sb-sync`'s reconciliation, protocol and authentication modules (section 8). Changes there get the orchestrator's own diff review.

### 4.3 Technology (call)

| Concern | Choice | Why |
| --- | --- | --- |
| GUI | `eframe`/`egui` | Pure Rust, one self-contained binary, same code on Windows and Linux, no web view to depend on. |
| Markdown parsing | `pulldown-cmark` | Standard parser; version 0.13 understands wikilinks natively. |
| Markdown preview | `egui_commonmark` | Renders CommonMark inside egui. |
| Sync document | `loro` (CRDT) | Merges concurrent text edits without conflict handling of our own; sync reduces to exchanging missing updates. |
| Peer transport | `iroh`, `iroh-mdns-address-lookup` | Pure-Rust QUIC connections dialed by public key, encrypted and authenticated, with NAT traversal; the second crate adds LAN discovery. |
| Folder watching | `notify` | The standard cross-platform watcher. |
| Windows glass material | `window-vibrancy` (Windows only) | Applies the system Mica backdrop to the eframe window through its raw handle; no own unsafe code (appendix A.5). |
| Folder picker | `rfd` | Native dialog; on Linux through the desktop portal, with no GTK dependency. |
| CLI parsing | `clap` | Standard. |
| MCP server | `rmcp` | The official SDK. It already serves both the `initialize` handshake and its replacement in the 2026-07-28 revision of the protocol; a hand-written loop would have to chase the specification. |
| Hashing | `blake3` | Content hashes and the peer authentication proof. |

Exact versions, cargo features and verified API facts are in appendix A.

## 5. Vault and note model

### 5.1 Vault

A vault is any existing directory. Notes are the `.md` files below it, at any depth. Nothing has to be initialised.

The application owns one directory inside the vault, `.secondbrain/`, used only for `trash/` (section 5.5). It is hidden and never synced or indexed.

### 5.2 Note path and note key

- A **note path** is the vault-relative path of a note, with `/` as separator on every platform, case preserved, for example `projects/Idea.md`. It is the identifier used by the CLI, MCP, the GUI and the wire protocol.
- A **note key** is the note path in Unicode lowercase (`str::to_lowercase`). Two paths with the same key are the same note everywhere. This makes identity case-insensitive, as it already is on Windows. **(call)**

A note path is valid when all of the following hold. The same validation runs on every path that arrives from the CLI, MCP or a peer, before any disk access:

1. Not empty; at most 1024 bytes; each component at most 255 bytes.
2. Relative: no leading `/`, no drive letter, no `\`.
3. No empty component, no `.` or `..` component.
4. No component starts with `.` (hidden files and directories are never notes).
5. No component contains `< > : " | ? *` or a control character (U+0000–U+001F, U+007F).
6. No component ends with a space or a dot.
7. No component's stem (the part before the first dot, with trailing spaces removed) is a Windows device name, compared case-insensitively: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`–`COM9`, `LPT0`–`LPT9`, and `COM` or `LPT` followed by one of the superscript digits `¹`, `²`, `³`. (Widened in M2, before paths arrive from peers: a Linux peer may legally hold `CON .md`, which older Windows versions treat as the device.)
8. The last component ends with `.md`, compared ASCII case-insensitively, and has a non-empty stem.

Rules 5–7 make every synced path valid on both operating systems.

### 5.3 Scanning

Scanning walks the vault and returns the notes plus a list of ignored files with a reason:

- Entries whose name starts with `.` are skipped silently, directories included.
- Symbolic links are skipped silently, files and directories alike. They are never followed.
- A `.md` file whose relative path fails validation is ignored with reason `InvalidPath`.
- When several files share one note key (possible on Linux only), the path that sorts first bytewise is the note; the others are ignored with reason `CaseCollision`.
- A `.md` file that is not valid UTF-8 is ignored with reason `NotUtf8`.
- A `.md` file that cannot be read (in use by another program, no permission) is ignored with reason `Unreadable`; the scan goes on.
- A folder below the vault root that cannot be listed (no permission, for example `System Volume Information` or `lost+found`) is ignored with reason `Unreadable`, its path written with a trailing `/`; the scan goes on. For the sync engine every note key under such a folder is *blocked*, never absent. Only a vault root that cannot be listed fails the scan.
- A `.md` file larger than 4 MiB is ignored with reason `TooLarge`. Writing a note larger than that through the CLI, MCP or the GUI is refused, and so is reading one: a file over the limit is not treated as a note by any operation.
- Files without the `.md` extension are not notes and are not reported.

Ignored files are never indexed and never synced, and the sync engine never modifies or replaces them. The GUI and `sb list` show how many were ignored and why.

### 5.4 Reading and writing

- Content is UTF-8 text. Bytes are preserved exactly: no line-ending normalisation, no trailing-newline fixing.
- **Replacing a note is atomic:** write a hidden temporary file `.sb-<random>.tmp` in the destination directory, then rename it over the target. The new file keeps the permissions of the one it replaces. A symbolic link standing where the note should be is never replaced: the write fails with `SymlinkInPath`.
- **Creating a note never replaces anything.** If something already exists at the location, including a file the scan ignores or a file the file system considers the same name, creation fails with `AlreadyExists`. Parent directories are created as needed. From M2 on, the content is written to a temporary file first and then linked into place, so a crash never leaves a half-written note under its real name; on file systems without hard links (FAT, exFAT) the file is created directly.
- **A read-only note is never replaced** (from M2 on): writing or appending to a note whose permissions forbid writing fails, on Windows and Linux alike. Without this rule the atomic replace would overwrite it on Linux, where only the folder's permissions count.
- **A new note adopts the spelling of existing folders.** Each folder of its path that already exists under another letter case keeps the spelling on disk; only folders that do not exist yet, and the file name, are spelled as given. So `projects/new.md` next to an existing `Projects/` becomes `Projects/new.md`, and a vault never grows two folders that differ only in case.
- **Confinement:** before any write, delete or rename, every existing ancestor directory between the vault root and the target must be a real directory. A symbolic link there fails the operation with `SymlinkInPath`; anything else that is not a directory, with `NotADirectory`.
- **Rename never replaces anything.** It fails with `AlreadyExists` if the destination key is already taken by a different note, and also when the file system finds anything at the destination under another name for it (a Windows 8.3 short name such as `NOTEBO~1.md`, a folder, a file the scan ignores). Only a change of letter case of the note itself is allowed onto its own location. It does not rewrite links.
- **Append** adds text at the end; if the note is non-empty and does not end with a newline, one `\n` is inserted first. It creates the note if missing.

### 5.5 Trash

Deleting a note moves it to `.secondbrain/trash/<UTC timestamp>-<flattened path>`, where the timestamp is `YYYYMMDDThhmmssZ` and the flattened path replaces `/` with `__`. A trash file name never exceeds 255 bytes: when it would, the flattened path is shortened from the front, at a character boundary, keeping the timestamp and the end of the path. If the name is taken, `-1`, `-2`, … is appended. Nothing is ever unlinked directly. This applies to deletions by the user, by an agent and by the sync engine. The trash is never emptied automatically.

## 6. Markdown model

### 6.1 Links

Extracted with `pulldown-cmark` so that links inside code spans and code blocks are not links. Three kinds:

| Kind | Syntax | Notes |
| --- | --- | --- |
| Wiki | `[[Target]]`, `[[Target\|Alias]]`, `[[Target#Heading]]`, `[[Target#Heading\|Alias]]` | |
| Embed | `![[Target]]` | Treated as a link. |
| Markdown | `[text](relative/path.md)`, `[text](Note.md#heading)` | Only destinations without a URL scheme that end in `.md` (before any `#`), percent-decoded. |

Each link records: target text as written (without heading), optional heading, optional alias, kind, and 1-based line number.

**Attachment links** (from M3). A link whose target's last component ends, ASCII case-insensitively, with `.` followed by one of `png jpg jpeg gif bmp svg webp avif ico tif tiff pdf mp3 wav ogg m4a flac mp4 webm mov mkv avi` points to an attachment, not a note: `![[image.png]]`, `[[report.pdf]]`. It stays in the note's link list (the CLI and MCP still report it, unresolved), but it produces no graph node and no edge, and the GUI never offers to create a note for it. Version 1 does not display attachments.

### 6.2 Link resolution (call)

Resolving target `T` written in note `S`. First trim and replace `\` with `/`. Two prefixes anchor the target and switch the general rules off:

- a target starting with `/` is anchored at the vault root: it resolves only to the note at exactly that path, otherwise it is unresolved;
- a target starting with `./` is relative to the folder of `S` and nothing else, `..` components allowed; a path leaving the vault matches nothing.

Otherwise normalise (drop a trailing `.md`, ASCII case-insensitive; lowercase) and apply, in order:

1. **`T` contains `/`:**
   1. exact vault-relative match: the note whose key is `T.md`;
   2. relative to the folder of `S`, resolving `..` components; a path escaping the vault matches nothing;
   3. suffix match: notes whose key ends with `/T.md`, chosen by the tie-break below.
2. **`T` is a bare name:** notes whose file stem, lowercased, equals `T`, chosen by the tie-break.
3. Otherwise the link is **unresolved**.

Tie-break among several candidates: a note in the same folder as `S` first; then fewest path components; then smallest key.

An empty target (`[[#Heading]]`) refers to `S` itself and is not a link between notes. A note that links to itself is not listed among its own backlinks.

### 6.3 Graph

- A node for every note: id = note path, label = file stem, `exists = true`.
- A **ghost node** for every distinct unresolved target that is not an attachment: id = `?` followed by the key of the note that clicking it would create (section 6.6) without the trailing `.md`, label = the target as first written, `exists = false`. For an unanchored target this is the normalised target, as before M3; for `./x` written in `a/b.md` it is `?a/x`, so anchored targets in different folders are different ghosts. A target for which section 6.6 yields no valid path produces no ghost and no edge.
- A directed edge for each (source note, target node) pair, at most one per pair. Self-links produce no edge.
- **Neighbourhood** of a note at depth `d` (1 ≤ d ≤ 5): the subgraph reachable in at most `d` steps, treating edges as undirected.

### 6.4 Tags

An inline tag is `#` followed by one or more of: Unicode letters and digits, `_`, `-`, `/`, with at least one non-digit character, where the `#` is at the start of a text run or follows whitespace or `(`. Tags inside code are ignored. Tags are stored lowercased without the `#`.

### 6.5 Search

- The query is split on whitespace into terms and lowercased. An empty query returns nothing.
- A note matches when every term is a substring of its lowercased title or content.
- Score = 10 × (number of terms found in the title) + (number of occurrences of all terms in the content, each term capped at 5).
- Results are ordered by score descending, then path ascending. `limit` defaults to 20, maximum 100.
- Each hit carries up to 3 snippets: the first lines containing any term, with their 1-based line numbers, each trimmed to 200 characters.

This is a linear scan over notes held in memory. It is the right size for vaults of a few thousand notes.

### 6.6 New note from an unresolved link (call, M3)

Clicking an unresolved link (or a ghost node) creates a note at a path computed from the target `T` and the source note `S`. First trim and replace `\` with `/`; then:

- `T` starts with `/`: the rest, from the vault root;
- `T` starts with `./`: relative to the folder of `S`, `..` components resolved; a path leaving the vault yields nothing;
- otherwise: `T` from the vault root, whether bare (`Idea` → `Idea.md`) or with folders (`sub/Idea` → `sub/Idea.md`). A note created there is always what the link resolves to (rule 1.1 or rule 2 of section 6.2).

`.md` is appended unless the last component already ends with it (ASCII case-insensitive); the letter case is kept as written. The result must pass section 5.2, otherwise nothing is created and the GUI says why. Creation follows section 5.4: it never replaces anything and adopts the spelling of existing folders. An attachment link (section 6.1) never creates a note.

## 7. Agent gateway: CLI and MCP

One console binary, `sb`. Both surfaces call the same `sb-core` operations and return the same data.

### 7.1 Vault selection

`--vault <DIR>` wins; otherwise the `SB_VAULT` environment variable; otherwise the current directory. The directory must exist.

### 7.2 CLI

```
sb [--vault <DIR>] [--json] <command>

  list                              all notes; ignored files are summarised on stderr
  read <PATH>                       print the note
  write <PATH> [--content <TEXT>]   create or replace; reads stdin without --content
  append <PATH> [--content <TEXT>]  append; reads stdin without --content
  delete <PATH>                     move to trash
  rename <FROM> <TO>
  search <QUERY> [--limit <N>]
  links <PATH>                      outgoing links and backlinks
  graph [--around <PATH>] [--depth <N>]
  tags
  mcp                               run the MCP server on stdio
  sync run | invite | join <TICKET> | leave | status      (section 8.8)
```

- Without `--json` the output is human-readable text. With `--json`, stdout carries exactly one JSON document with the same shape the matching MCP tool returns; the one exception is `read`, which prints the note as a single JSON string (the MCP tool returns the raw text).
- Exit code 0 on success, 1 on an operation error with a one-line message on stderr, 2 on a usage error.

### 7.3 MCP server

`sb mcp` speaks the Model Context Protocol over stdio. Stdout carries protocol messages only; diagnostics go to stderr. The process exits when stdin closes.

Tools. Every `path` is a note path as in section 5.2.

| Tool | Arguments | Result |
| --- | --- | --- |
| `list_notes` | `folder?` (path prefix) | `{ notes: [{ path, title }], ignored: <count> }` |
| `read_note` | `path` | the note's Markdown text |
| `write_note` | `path`, `content` | `{ path, created }` — creates or replaces |
| `append_to_note` | `path`, `content` | `{ path, created }` |
| `delete_note` | `path` | `{ path, trashed: true }` |
| `rename_note` | `from`, `to` | `{ from, to }` |
| `search_notes` | `query`, `limit?` | `{ hits: [{ path, title, score, snippets: [{ line, text }] }] }` |
| `get_links` | `path` | `{ outgoing: [{ target, heading?, alias?, kind, line, resolved? }], backlinks: [{ from, line, context }] }` |
| `get_graph` | `around?`, `depth?` | `{ nodes: [{ id, label, exists }], edges: [{ from, to }] }` |
| `list_tags` | — | `{ tags: [{ tag, count }] }` |

- Structured results are returned as one JSON text content block; `read_note` returns the raw text.
- An operation error (invalid path, note not found, destination exists) is a tool result with `isError: true` and a message naming the problem. Protocol errors are reserved for malformed requests and unknown tools.
- The server's `instructions` explain, in a few sentences, what a vault is, the path format and that `[[wikilinks]]` connect notes.
- The server rebuilds its index from disk for each tool call that needs it, so it always sees the current files.

## 8. Sync

### 8.1 What syncs

Valid notes (section 5.3) and nothing else. Everything in this section is scoped to one vault on one device, called a **replica**.

Sized for vaults of up to about 10 000 notes and 100 MB of text. Edit history is kept for the life of the sync group and is never compacted, so deleting a note removes it from every vault but not from the sync history. Both facts are stated in the README.

### 8.2 Trust model (call)

- A **sync group** is the set of replicas sharing a 32-byte random **group secret**. The first replica creates it; others receive it inside a **ticket**.
- Each replica has its own **device key** (the transport's key pair). Its public half is the **device id**.
- Membership is knowledge of the secret. On every connection both sides send a proof and verify the other's in constant time before any other message:

  `proof = blake3_keyed_hash(key = secret, data = "sb-sync-auth-v1" ‖ sender device id ‖ receiver device id)`

  The transport already authenticates device ids, so a proof cannot be replayed by a third device. A peer whose proof fails learns nothing and is disconnected.
- A ticket is a password. Anyone holding it can read and write the vault until the group is abandoned.
- Everything a peer sends is untrusted input even after authentication: paths are validated (section 5.2), note ids are checked for their format, sizes are bounded (section 8.7). An entry that fails validation is skipped and reported, never acted on.

### 8.3 State directory (call)

All sync state lives **outside** the vault, so copying or sharing the vault folder never copies a device key, a secret or edit history:

- Base directory: `SB_STATE_DIR` if set; otherwise `%LOCALAPPDATA%\secondbrain` on Windows; otherwise `$XDG_DATA_HOME/secondbrain`, falling back to `$HOME/.local/share/secondbrain`.
- Per replica: `<base>/vaults/<first 16 hex chars of blake3(canonical vault path)>/`. On Windows the canonical path is lowercased before hashing.

| File | Content |
| --- | --- |
| `vault-path.txt` | the canonical vault path, for humans |
| `device.key`, `device.id` | device secret key, and its public id for tools that only read files |
| `secret` | group secret, hex. Its presence means «sync is on». |
| `peers.json` | known peers: device id and optional address hints |
| `config.json` | `{ "mode": "internet" \| "lan", "bind_port": <u16 or null>, "mdns": <bool> }`, defaults `internet`, `null`, `true` |
| `doc.snap`, `doc.log` | the sync document: snapshot plus append-only update log |
| `base.json` | per note key, the content hash at the last point disk and document agreed |
| `base.version` | the document version the base was last saved against, hex. Written after `base.json`. |
| `engine.lock` | exclusive lock held by the running engine |
| `status.json` | written by the engine every tick: process id, device id, addresses, peers and their connection state, note count, blocked notes, last error, timestamp |
| `fresh-join` | marker: this replica joined a group and has not reconciled since (section 8.6) |
| `command.pending` | a command for the running engine: `{"join": "<ticket>"}` or `{"leave": true}` |

Moving the vault folder changes the hash, so the replica looks unconfigured and must join again. That is safe (section 8.6) and visible.

**The base never outlives the document it was saved against.** A base entry says «disk and document agreed on this content»; with the note missing from the document, the engine reads that as «the note was deleted elsewhere» and moves the file to the trash. If the document on disk ever loses history while the base survives (a damaged or deleted `doc.log`, a partly restored state directory), that reading would trash every note. So when the engine opens its state it checks that the loaded document includes the version in `base.version`; if the base is not empty and the file is missing, unreadable or names history the document does not have, the base is cleared. With an empty base nothing can be trashed: files become notes again, and a file that differs from a note becomes a conflict copy.

The files `secret`, `peers.json` and `command.pending` are the engine's **control plane**: the engine re-reads them on every tick. The CLI configures sync by editing them, whether or not an engine is running. `config.json` is read once, when the engine starts: a change to the network mode, port or mDNS takes effect at the next start (ruling of the M2 final review, 2026-11-17).

### 8.4 Sync document (call)

One CRDT document per replica. A note in it is identified by a **note id**, 32 lowercase hexadecimal characters chosen at random by the replica that creates the note. The path is a property of the note, not its identity.

- For each note id, a **text** holding the note's content.
- A map **paths**: note id → display path. Written when the note is created and when it is renamed.
- A map **tombs**: `<note id>:<hex content hash>` → `true`. Written when the note is deleted. Entries are never removed.

A note is **alive** when `paths` holds a valid note path for it and `tombs` has no entry for its id together with the hash of its current text. So:

- Deleting a note records a tombstone for the text as it was. The text is kept.
- If another replica edited the note concurrently, the text no longer matches the tombstone and the note stays alive everywhere. **An edit beats a delete**, deterministically, on every replica.
- If two replicas delete the note at different states of its text, both tombstones exist and the note is dead whichever state the text ends in.
- A note is never brought back: a file that appears at the path of a dead note is a new note with a new id.

**One note per path.** Two alive notes may claim the same note key: two replicas created the same path while apart, or a rename landed on an occupied path. Every replica resolves this the same way, as ordinary document changes, at the start of each reconciliation:

1. Among the alive notes claiming one key, the one with the smallest id keeps the path.
2. Each other note whose text is identical to the keeper's is a duplicate and is tombstoned.
3. Each other note with different text is a **conflict copy**: its path is changed to `<stem> (conflict <first 8 characters of its id>).md` in the same folder. If that name would exceed the limits of section 5.2 the stem is shortened; if the whole path still does not fit, the path is `conflict-<id>.md` in the vault root.
4. Repeat until no key has two claimants.

The outcome depends only on the document's content, so replicas that hold the same document reach the same result without talking about it.

**Merge granularity (call).** A change to a file enters the document as a difference between lines, not between characters. Two replicas that edit different lines of one note merge cleanly. Two replicas that edit the same line keep both versions of that line, one after the other, and the user removes the one they do not want. Character-level merging was measured and rejected: on same-line conflicts it interleaves the two edits into text nobody wrote (appendix A.4).

The editing identity inside the document is a fresh random one each time the document is loaded. Nothing has to be kept unique across copies and restores of the state directory.

### 8.5 Reconciliation

Reconciliation is one idempotent function. It runs at start-up, after watcher events (debounced), after every imported remote update and on a periodic tick. One round has four steps.

**Step 1 — scan.** Read the vault. Every note key is in one of three disk states:

- **present** with its content;
- **absent**: the scan completed and nothing occupies that location;
- **blocked**: something is there that is not a readable note (a file ignored as not UTF-8 or too large), or an operation on it failed in this round (unreadable, in use, something in the way of a new file, changed while being replaced).

A blocked key is left completely alone in this round: no document change, no file change, base untouched. It is reported in `status.json` and retried in the next round. **Blocked is never treated as absent**, so an unreadable file can never turn into a deletion on other devices.

**Step 2 — resolve path claims** (section 8.4). After it, each key has at most one alive note.

**Step 3 — compare.** For each key in the union of disk, document and base:

- `D`: the file's content, or absent;
- `H`: the content of the alive note holding that key, or absent;
- `B`: the base hash, or absent.

| D vs H | D vs B | H vs B | Meaning | Action |
| --- | --- | --- | --- | --- |
| equal | | | in agreement | `B := hash(D)`, or no entry when both are absent |
| differ | changed | same | local change | document follows disk (below). `B := hash(D)` |
| differ | same | changed | remote change | disk follows document (below). `B := hash(H)` |
| differ | changed | changed | both changed | below |

Document follows disk:

- file present, note alive: the note's text is updated from the file. If the new text matches a tombstone of that note, the note is tombstoned as it stands and a new note is created with the file's content, so an edit can never fall into an old deletion.
- file present, no note: a new note is created with a new id, this path and this content.
- file absent, note alive: the note is tombstoned.

Disk follows document:

- note alive, file present: the file is replaced.
- note alive, file absent: the file is created at the note's display path. Creation never replaces anything: if something already exists there, the key is blocked.
- no note, file present: the file is moved to the trash, and now-empty parent directories are removed, best effort.

Both changed:

- both present: the file's content becomes a **new note claiming the same path**, and `B := hash(D)`. The next claim resolution separates the two notes: one keeps the path, the other becomes a conflict copy, or they are recognised as duplicates. Exception: if an alive note that has no file and no base yet already holds exactly the file's content, the content is already captured (an earlier round was interrupted) and only `B := hash(D)` is recorded.
- file present, no note (local edit against remote delete): the local edit wins; a new note is created. `B := hash(D)`.
- file absent, note alive (local delete against remote edit): the remote edit wins; the file is created. `B := hash(H)`.

**Step 4 — pair renames.** Before the actions of step 3 are carried out, moves are recognised so that a rename stays a rename:

- *Local rename:* a key whose file disappeared (note alive, unchanged against base) and a key where a file appeared (no note, no base) with the same content hash. Instead of a tombstone plus a new note, the note's path is changed.
- *Remote rename:* a key whose note went away (file present, unchanged against base) and a key where a note appeared (no file, no base) with the same content hash. Instead of trashing and creating, the file is renamed. The rename never replaces an existing file.

Pairs are formed in key order, each key used once. A rename combined with an edit in the same round is not recognised and is handled as delete plus create.

Because the path is a property of the note, an edit made elsewhere to a renamed note follows the note to its new path, and two concurrent renames of one note end with one note at one path.

Rules that make this safe:

1. **Order of durability.** Document changes are persisted before any file is written and before any update is sent to a peer. The base is persisted last. A crash at any point leaves a state the next round resolves correctly: persisted-but-unwritten remote changes look like «remote change»; written-but-unrecorded files look like «in agreement»; a half-done conflict resolution is redone and finds its own earlier result as a duplicate.
2. **Compare before overwrite.** A file is replaced, renamed or trashed only if, re-read immediately beforehand with the replacement already prepared, it still has the content seen during the scan. Otherwise the key is blocked for this round. A writer that slips in between that check and the rename can still be overwritten; the window is a few system calls and is accepted.
3. **Local first.** When a remote update arrives, the engine reconciles, then imports the update, then reconciles again. Pending local edits therefore enter the document as edits and merge with the remote ones line by line; the «both changed» rows are reached only in crash and race situations and after a join without contact (section 8.6).
4. **Echo.** The engine's own writes trigger the watcher; the resulting round finds everything in agreement and produces nothing.
5. **Missing vault.** If the vault directory does not exist, or `.secondbrain/` has disappeared from a vault that has sync state, nothing is treated as deleted: starting the engine fails with an error, and a running engine abandons the round before touching the document or any file. The check runs at the start of every round, because a drive can be unmounted while the engine runs and leave an empty folder behind. A running engine reports the error, keeps going and tries again at the next round, so sync resumes by itself when the vault is back.
6. New files use the display path from `paths`. A note that already exists locally keeps its local casing.

### 8.6 Joining and leaving

**Joining** a group with a ticket:

- If the ticket's secret is the one the replica already has, only the inviting peer is added to `peers.json`.
- Otherwise the replica's sync state is replaced: secret and peers become the ticket's, the document and the base are reset to empty, every open session is closed, and the `fresh-join` marker is set.

While `fresh-join` is set the engine postpones reconciliation until it has imported a first state from any peer, or until 20 seconds have passed since it started waiting, whichever comes first. Then it clears the marker and reconciles normally. With an empty base that means:

- a local note identical to the group's: in agreement, nothing happens;
- a local note that differs from the group's at the same path: both are kept, one of them as a conflict copy;
- a note only local: created in the document and sent to peers;
- a note only in the group: written to disk.

The wait is an optimisation, not a safety device. If it times out, the replica records its local notes as new notes; when the group's state arrives later, claim resolution removes the duplicates and separates the differences exactly as above. The only cost is that the text of duplicated notes is stored twice in the sync history. There is no state a replica can get stuck in.

Joining never deletes or overwrites a local note. A note that was deleted in the group while this replica was away comes back as a new note; that is accepted.

**Leaving** turns sync off for the replica: `secret`, `peers.json`, the document, the base and the markers are deleted and sessions are closed. The vault's files are untouched. The device key is kept.

### 8.7 Protocol

One bidirectional stream per connection. Frames are a 4-byte big-endian length followed by the payload. The first frame in each direction must be a `Hello` and may be at most 64 bytes; later frames may be at most 1 GiB. A longer frame closes the connection. A replica never sends a frame over the limit: it records the error in `status.json` instead. The first payload byte is the message type:

| Type | Body | Meaning |
| --- | --- | --- |
| `Hello` | protocol version (`u16`, currently 1), proof (32 bytes) | authentication, section 8.2 |
| `Version` | the sender's document version | «send me what I lack» |
| `Updates` | the sender's document version, then document updates (possibly empty) | incremental state |
| `Peers` | list of device ids | peers the sender knows |

Flow:

1. The dialling side sends `Hello`. The accepting side verifies it and answers with its own `Hello`. The dialler verifies. Any failure, or a different protocol version, closes the connection.
2. Both send `Version` and `Peers`.
3. On `Version`, a replica answers with `Updates` holding everything the peer lacks. It always answers, even with no updates; that answer is what ends a fresh-join wait.
4. On `Updates`, a replica imports (section 8.5 rule 3). If the import reports missing dependencies it sends `Version` again.
5. After any change to its document, a replica sends `Updates` to every connected peer that lacks something, based on the last version it knows that peer has. Updates received from one peer therefore reach the others, so replicas need not all be connected to each other.
6. On `Peers`, unknown device ids are added to `peers.json`.

Importing is idempotent, so duplicated or re-ordered updates are harmless.

A replica that is not fresh-joined completes its start-up reconciliation before it answers any `Version`, so the state it serves always reflects its disk.

### 8.8 Connectivity and control

- The engine accepts connections and, on every tick, dials each known peer it has no live session with. Two simultaneous sessions with the same peer are allowed; they are redundant, not harmful.
- **Modes.**
  - `internet` (default) uses the public infrastructure of the iroh project to find peers by device id and to traverse NATs: a DNS service and relay servers (hosts listed in appendix A.2). They learn device ids and network addresses. They never receive note content in readable form, because connections are end-to-end encrypted, and they store nothing.
  - `lan` contacts nothing outside the local network.
  - Both modes find peers on the same network by local discovery (mDNS) and try the address hints in `peers.json`.
  - The README states this plainly, so the serverless claim is honest: no server run by the user, and no server that stores or can read notes.
- **Ticket:** `sb1/<device id hex>/<secret hex>[/<addr>,<addr>…]`, where the optional addresses are `ip:port` hints of the inviting device.

Commands:

| Command | Behaviour |
| --- | --- |
| `sb sync run` | Runs the engine in the foreground until interrupted or until the replica leaves its group. Creates the device key and the secret if missing. Fails if another engine holds the lock. |
| `sb sync invite` | Creates the device key and secret if missing and prints a ticket. Includes address hints when a running engine has published them in `status.json`. |
| `sb sync join <TICKET>` | Validates the ticket, then joins (section 8.6). |
| `sb sync leave` | Leaves the group (section 8.6). |
| `sb sync status` | Prints `status.json`; reports «engine not running» when the file is missing or older than 30 seconds. |

`join` and `leave` act directly when no engine is running (they take the lock for the duration). When one is running they write `command.pending`, and the engine carries the command out on its next tick and removes the file.

Timing: watcher debounce 300 ms; tick every 5 s; full rescan at least every 60 s even without watcher events; dial timeout 15 s; fresh-join wait 20 s.

### 8.9 Engine structure

- One **actor** thread owns the document, the base and the vault handle, and processes commands from a queue: reconcile, remote update, peer version, session opened or closed, tick, join, leave, shutdown. Nothing else touches the document. Its first command after start-up is a reconciliation (or the fresh-join wait).
- Network sessions run on an async runtime and talk to the actor through channels. Session logic is written against generic byte streams, so the whole protocol is tested in memory; the network adapter only opens streams and reports device ids.
- The engine reports events (peer connected, peer disconnected, synced, error) to its host through a channel. The GUI shows them; `sb sync run` logs them to stderr.

## 9. GUI

A single window in the spirit of Obsidian: very simple and minimal, very legible and accessible, polished, a little like a macOS app with a glass effect (user's direction, 2026-11-17; visual rules in section 9.6). It follows the system light/dark setting. All behaviour lives in a state module that has no dependency on the UI toolkit and is unit-tested; the UI code draws state and forwards input.

### 9.1 Start

`secondbrain [VAULT_DIR]`. Without an argument it reopens the last vault; if there is none it shows a welcome screen with a folder picker and a path field (the field is the fallback where no native dialog is available). The last vault is remembered in `<state base>/app.json`.

### 9.2 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ ☰  New   Quick open   Graph   Edit/Preview         vault name   Sync │ top bar
├───────────────┬──────────────────────────────────┬───────────────────┤
│ search…       │                                  │ Backlinks         │
│               │   editor, preview or graph       │  ← Note A: line…  │
│ ▾ projects    │                                  │                   │
│     Idea      │                                  │ Outgoing links    │
│   Inbox       │                                  │  → Note B         │
│               │                                  │  → missing (new)  │
│               │                                  │ Tags              │
├───────────────┴──────────────────────────────────┴───────────────────┤
│ 312 words · 2 files ignored                     Sync: 1 peer online  │ status bar
└──────────────────────────────────────────────────────────────────────┘
```

- **Left:** a search field; below it the folder tree, or the search results while the field is non-empty. Clicking opens a note. A context menu on a note offers rename and delete; on a folder, «new note here».
- **Centre:** the open note in **edit** mode (a plain-text editor with light Markdown colouring of headings, links and code) or **preview** mode (rendered Markdown where wikilinks are clickable), or the **graph**.
- **Right:** backlinks with the linking line, outgoing links (unresolved ones marked; clicking one creates the note, except attachments), and the note's tags. Both side panels can be hidden.
- **Top bar menu (⋯):** theme (System, Light, Dark), «Reduce transparency», text size (smaller, larger, reset; also Ctrl+Plus, Ctrl+Minus, Ctrl+0), «Open another vault». The drawing above is the arrangement, not the look: the look is section 9.6.

### 9.3 Behaviour

- **Autosave:** 500 ms after the last keystroke, and whenever the note is closed, switched or the window loses focus. There is no save button.
- **External change of the open note** (sync, an agent, another editor): if the buffer has no unsaved edits it reloads silently. If it has, the buffer is written to a new note named `<stem> (conflict <8 random hex characters>).md` next to it, and the note reloads. The application's own writes are recognised by content and ignored.
- **The open note disappears from disk** (deleted or renamed elsewhere): with no unsaved edits the note closes and the status bar says it was removed. With unsaved edits the buffer is written to a conflict note as above, which then opens. Typed text is never lost.
- **A save fails** (read-only note, disk full, folder gone): the buffer stays as typed and dirty, the status bar shows the error, and the save is retried at the next autosave trigger.
- **New note:** asks for a name, creates `<name>.md` in the selected folder or the vault root, opens it in edit mode.
- **Clicking a link** opens the resolved note; an unresolved link creates it first.
- **Quick open:** a centred popup that filters note names as the user types; Enter opens the first match, Escape closes.
- **Shortcuts:** Ctrl+N new note, Ctrl+O quick open, Ctrl+E edit/preview, Ctrl+G graph, Ctrl+F focus search.
- The index follows the folder: after a watcher event and 300 ms of quiet, the whole index is rebuilt on a background thread and swapped in; the tree, backlinks and graph follow (call, M3: the watcher gives no paths, and a full rebuild keeps the ignored-file list correct). The application's own saves update the index at once.

### 9.4 Graph view

A force-directed layout of the whole vault graph drawn in the centre pane:

- nodes are circles sized by number of links; ghost nodes are dimmer; the open note is highlighted;
- drag the background to pan, scroll to zoom about the cursor, drag a node to move it;
- hovering a node highlights it and its neighbours and shows its label; labels of all nodes appear when zoomed in;
- clicking a node opens the note (creating it for a ghost node).

The layout is a simple spring-and-repulsion simulation, quadratic in the number of nodes. It is the right size for a few thousand notes.

### 9.5 Sync window

Opened from the top bar: whether sync is on; this device's id; **Create invite** (shows the ticket with a copy button); **Join** (paste a ticket, with a note that where a local note differs from the group's both are kept); **Turn sync off** (leave the group); the list of peers with their state; notes the engine could not sync and why; the mode switch (Internet / LAN only).

If another process holds the engine lock, the window says sync is being handled by that process and shows `status.json` read-only.

**Engine lifecycle in the GUI (call, M3).** The GUI starts the engine for the open vault only when sync is configured for it (its state directory holds a group secret), or when the user creates an invite or joins from this window. A vault that never used sync therefore never contacts the network. «Turn sync off» leaves the group and stops the engine. Changing the mode writes `config.json` and restarts the engine, because the engine reads it only at start (section 8.3). Engine calls that can block (start, invite, join, leave, stop) run on a background thread; the window shows that they are in progress and never freezes. Closing the application saves the open note and stops the engine.

### 9.6 Visual design and accessibility (call, M3)

From the glass spike of 2026-11-17 (appendix A.5), refined after the user's feedback on its first screenshots: no coloured gradients, neutral backdrops only, refined glass.

**Surfaces.** The tool areas are glass: the top bar, the left panel, the right panel and a slim status strip. The open note (editor, preview or graph) sits on a **solid, quiet document surface**. No hard separator lines between panels; regions are separated by 8 px gutters and depth. Corner radius 12 px. Each glass panel has a 1 px hairline border and a 1 px highlight along its top edge. The document surface has a soft shadow and no border. The status strip has no border, radius or shadow.

**Material.** One of three, chosen at start and on a settings change:

| Material | When | Behind the glass |
| --- | --- | --- |
| `Mica` | Windows, «Reduce transparency» off, and the system accepts the Mica backdrop | the Windows Mica material (desktop-tinted, follows light/dark); the window is transparent |
| `Simulated` | Linux; Windows when Mica is refused | a neutral vertical gradient painted by the app |
| `Opaque` | «Reduce transparency» on, any system | the same gradient; every glass panel is drawn fully opaque |

Linux has no portable blur for a window (appendix A.5), so it always gets `Simulated` or `Opaque`.

**Colour tokens.** Hex; a percentage is the alpha. Glass alpha depends on the material because with `Mica` the backdrop is unknown.

| Token | Light | Dark |
| --- | --- | --- |
| Glass fill, `Simulated` | `#FFFFFF` 60 % | `#2C2C30` 60 % |
| Glass fill, `Mica` | `#FFFFFF` 76 % | `#2C2C30` 82 % |
| Glass fill, `Opaque` | `#FFFFFF` 100 % | `#2C2C30` 100 % |
| Document surface | `#FFFFFF` | `#232325` |
| Primary text | `#1C1C1E` | `#F5F5F7` |
| Secondary text | `#3C3C43` | `#C8C8D2` |
| Accent text (links, selected item) | `#0940A0` | `#B8D3FF` |
| Hairline | white 35 % | white 8 % |
| Top highlight | white 50 % | white 10 % |
| Shadow | black 8 %, blur 24, offset y 6 | black 30 %, blur 24, offset y 6 |
| Hover fill | black 6 % | white 6 % |
| Selection pill | `#0A84FF` 18 % | `#0A84FF` 18 % |
| Backdrop gradient (top → bottom) | `#F2F2F4` → `#E9E9EC` | `#1E1E20` → `#18181A` |

**Contrast.** Every text colour on every surface it can appear on has a WCAG 2 contrast ratio of at least 4.5 : 1, computed for the worst case: for `Simulated`, the glass composited over both ends of the backdrop gradient; for `Mica`, the glass composited over pure white and over pure black; for the selected item, the accent text on the selection pill composited over the glass. A unit test computes these ratios from the tokens. If a pair fails, the text colour is darkened (light theme) or lightened (dark theme) until it passes, and the table is corrected.

**Type.** Inter 4.1 (SIL Open Font License 1.1, licence file shipped next to the fonts), Regular and SemiBold, embedded in the binary, first in the proportional family; egui's default fonts stay as fallbacks for symbols and other scripts. Sizes: body 15.5 px, buttons 15 px, small text 13 px, headings 22 px SemiBold, section labels («FILES», «BACKLINKS») 11 px with 1.1 px letter spacing in the secondary colour. Code and the editor use egui's monospace font.

**Theme.** «System» (default) follows the operating system and updates when it changes; where the system reports nothing, dark. «Light» and «Dark» pin it. The native title bar follows the theme where the system allows; this is best-effort.

**Accessibility.**
- Every action is reachable from the keyboard: the shortcuts of section 9.3, Tab through every control, Enter and Space activate, Escape closes popups.
- The keyboard-focused control shows a 2 px ring in the accent colour.
- Every text field has a label or hint text, and every button that shows only a symbol carries an accessible name; screen readers are served through AccessKit (`accesskit` feature).
- Text size scales from 75 % to 200 % (top bar menu, Ctrl+Plus, Ctrl+Minus, Ctrl+0) and is remembered.
- «Reduce transparency» removes all translucency.
- Nothing is conveyed by colour alone: an unresolved link also shows the words «not created yet», a blocked note the word «blocked».

**Settings** are kept in `<state base>/app.json` with the last vault: `{"last_vault": path or null, "theme": "system" | "light" | "dark", "reduce_transparency": bool, "zoom": number}`. A missing, unreadable or invalid file means the defaults (`null`, `"system"`, `false`, `1.0`); a zoom outside 0.75–2.0 is clamped. The application never fails to start because of this file.

## 10. Cross-platform and distribution

- Windows: `cargo build --release` produces `secondbrain.exe` and `sb.exe`.
- Linux: a script builds both binaries in a Docker container based on an old-glibc distribution (Debian 11, glibc 2.31) so that they run on almost every current distribution. The GUI needs a working OpenGL driver and X11 or Wayland at run time.
- The README states the Linux run-time requirements, how to register the MCP server with Claude Code and Claude Desktop, how sync and tickets work, what `internet` mode contacts, and the non-goals.

## 11. Verification

### 11.1 Test strategy

- **Unit tests** for every rule in sections 5, 6 and 8: path validation, scanning, resolution, graph, tags, search, the alive/tombstone rule, each row of the reconciliation table, the frame codec, the authentication proof, ticket parsing.
- **Reconciliation scenario tests** on real temporary directories: two or three replicas wired together in memory. Each scenario asserts the final file contents on every replica and that a further reconciliation changes nothing.
- **Protocol tests** over in-memory streams, including a wrong secret, an oversized frame and a truncated frame.
- **One network test** over real loopback connections with relays and discovery off.
- **CLI and MCP tests** that run the built binary against a temporary vault.
- **GUI:** unit tests of the state module and the graph layout, plus headless UI smoke tests.

### 11.2 Acceptance criteria by milestone

Checks described as «by the orchestrator» are run by the orchestrating session itself, with commands, not by a person at a keyboard. Nothing in this project is verified by a human using the GUI interactively, and the README says so.

**M1 — core, CLI, MCP**
- `cargo test --workspace`, `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass on Windows, and the test suite passes in a Linux container.
- By the orchestrator, against a sample vault: every CLI command gives the expected output with and without `--json`; a path with `..`, an absolute path and a path under `.secondbrain/` are refused.
- By the orchestrator: piping `initialize`, `tools/list` and a `tools/call` for each tool into `sb mcp` returns well-formed responses and nothing else on stdout.

**M2 — sync**
- All tests pass on Windows and in a Linux container, including the scenario suite:
  - concurrent edits of different lines merge; concurrent edits of one line keep both;
  - edit against delete keeps the note; deletes at two different states leave it dead; an edit back to deleted content does not delete the file;
  - delete propagates and lands in the trash;
  - rename propagates as a rename; an edit elsewhere follows the renamed note; two concurrent renames end with one note;
  - the same path created on two replicas: identical content ends as one note, different content as a note plus a conflict copy, with the same result on every replica;
  - a blocked file (not UTF-8, something in the way) is never deleted remotely and never overwritten;
  - join with identical, differing, local-only and remote-only notes, both when the first state arrives in time and when the wait times out;
  - crash windows: document persisted but files not written; files written but base not persisted; conflict resolution interrupted;
  - a wrong secret is rejected; an oversized or truncated frame closes the connection;
  - three replicas in a line converge.
- By the orchestrator, on one machine: two `sb sync run` processes with different vaults and state directories, paired with a ticket, converge within 10 seconds after edits on either side, including an edit made by `sb write` while both run.
- `internet` mode is tried between two processes and its real status is recorded: working, or not verified on this machine and why.

**M3 — GUI**
- All tests pass, including headless UI tests that drive the real UI code: open a vault, create, edit, rename and delete a note, follow a link, search, switch to the graph and click a node, see an external edit appear, open the sync window.
- The contrast test of section 9.6 passes for every token pair.
- By the orchestrator on Windows: the binary starts with a sample vault and stays up without errors; screenshots in light and dark, with `Mica` and with «Reduce transparency», are taken and looked at.
- By the orchestrator on Windows, with a 10 000-note vault: the window opens, search answers while typing, and an external edit of the open note appears.

**M4 — Linux and release**
- Both binaries build in the Linux container and the full test suite passes there.
- A Windows replica and a Linux-container replica sync in both directions, connected through a ticket with address hints and a fixed port.
- The GUI binary starts in the Linux container under a virtual display with software rendering.
- README and CHANGELOG are complete, including what was and was not verified.

## 12. Left to the builder

- Internal module boundaries inside each crate beyond what the plans fix.
- Exact wording of human-readable CLI output and error messages.
- Icon glyphs, and spacing of the GUI not fixed by section 9.6, within «simple, minimal, legible, a little like macOS with glass». The colour tokens of section 9.6 are fixed; a change needs the contrast test to pass and the table corrected.
- Constants of the force layout.

## Appendix A — Verified facts about dependencies

From API spikes run on 2026-11-15 on Windows 11 with Rust 1.99. Each fact was verified by running code (**run**) or by reading the crate source (**source**). Anything not verified says so.

### A.1 MCP: `rmcp` 3.10 (decision: use it)

- The MCP specification revision `2026-07-28` removes `initialize` in favour of `server/discover` and per-request metadata. `rmcp` 3.10 serves both styles (**run**). A hand-written `initialize` loop would stop working with clients that move to the new style, so the SDK is the choice.
- Features for a stdio server: `server`, `macros`, `transport-io`. With `tokio` features `rt`, `macros`, `io-std` on a current-thread runtime (**run**).
- `#[tool_router]` on the impl block, `#[tool(description = …)]` on each method, `#[tool_handler(name, version, instructions)]` on `impl ServerHandler`. Parameters are a `Parameters<T>` where `T: Deserialize + JsonSchema`; doc comments on fields become schema descriptions (**run**).
- A handler returning `Result<String, String>` maps `Err` to a tool result with `isError: true`. Arguments that fail to deserialise are also reported in-band with `isError: true`. An unknown tool is JSON-RPC error `-32602` (**run**).
- Blocking work inside a handler is fine on a current-thread runtime; `spawn_blocking` works too. Stdout carries only protocol messages; the process exits with code 0 when stdin closes (**run**).
- In-process tests: connect a server and a client through `tokio::io::duplex`; the client side is `().serve(io)` with the `client` feature (**run**).
- Over `initialize` the server echoes the client's protocol version for `2024-11-05`, `2025-06-18` and `2025-11-25` (**run**).
- Not verified: behaviour against the real Claude Code and Claude Desktop clients.

### A.2 Transport: `iroh` 1.4

- Names changed from 0.x: `EndpointId` (alias of `PublicKey`), `EndpointAddr`, «address lookup» instead of «discovery». `Endpoint::builder(preset)` takes a preset (**run**).
- Device key: `SecretKey::generate()`, `to_bytes() -> [u8; 32]`, `SecretKey::from_bytes(&[u8; 32])`; `secret.public()` is the `EndpointId`; `id.as_bytes() -> &[u8; 32]`; `Display` is 64 lowercase hex characters and parses back with `str::parse` (**run**).
- Serving: `Router::builder(endpoint).accept(ALPN, handler).spawn()` with `impl ProtocolHandler { async fn accept(&self, conn: Connection) -> Result<(), AcceptError> }`, one task per connection. `conn.remote_id()` is the authenticated peer id. Streams: `conn.open_bi()` / `conn.accept_bi()` (**run**).
- The stream types implement `tokio::io::AsyncRead` and `tokio::io::AsyncWrite` (**source**: `noq-1.4.0/src/recv_stream.rs`, `send_stream.rs`), so protocol code can be written against generic streams.
- Closing without losing bytes: the last writer calls `send.finish()` and then awaits `conn.closed()`; the last reader calls `conn.close(code, reason)`; finally `endpoint.close().await` (**run**).
- **`lan` mode:** `Endpoint::builder(presets::Minimal).portmapper_config(PortmapperConfig::Disabled)` has no relay and no address lookup. In that configuration the process opened no TCP connection and only two unconnected UDP sockets (**run**), and nothing in the source contacts a third party (**source**). LAN discovery comes from the separate crate `iroh-mdns-address-lookup` 0.6: `.address_lookup(MdnsAddressLookup::builder())`. Two endpoints in one process connected by id alone in about 0.3 s (**run**). Discovery between two real machines is not verified.
- Dialling with a known address: `endpoint.connect(EndpointAddr::new(id).with_ip_addr(socket_addr), ALPN)` (**run**). `endpoint.addr()` is valid right after `bind()` in `lan` mode and lists every local interface (**run**).
- **`internet` mode:** `presets::N0`. It publishes the endpoint's relay URL (not its IP addresses) to `https://dns.iroh.link/pkarr`, resolves peers through DNS at `dns.iroh.link`, and uses the relay servers `use1-1`, `usw1-1`, `euc1-1` and `aps1-1.relay.n0.iroh.link` for NAT traversal and as a fallback path (**source**). These servers are operated by the iroh project. They see device ids and network addresses; traffic through them is end-to-end encrypted.
- **Open risk:** on the development machine the `N0` preset never reported itself online: every HTTPS probe failed with «network unreachable» although the same hosts answer over IPv4. The cause is not established. `Endpoint::online()` never completes without a relay, so nothing may wait on it without a timeout. `internet` mode must be re-tested in milestone M2 and its real status reported.
- TLS backend is `ring` (default feature `tls-ring`): needs a C compiler, no cmake or nasm. Declared minimum Rust version 1.91. Cold debug build about 2.5 minutes; about 270 crates (**run**).
- Shutdown: `router.shutdown().await` stops accepting and closes the endpoint; pending `accept()` calls return `None` (**run**).

### A.3 GUI: `eframe` 0.37, `egui_commonmark` 0.26, `egui_kittest` 0.37, `rfd` 0.18

- These four versions share one `egui` 0.37 (**run**: `cargo tree -d` shows no duplicate). `egui` is used as `eframe::egui`, not as a separate dependency.
- Features: `eframe` with `default-features = false` and `default_fonts`, `glow`, `x11`, `wayland`, `clipboard`, `links`, `accesskit`. The renderer is then OpenGL (`glow`), which needs OpenGL 2.1 or newer and works with software rendering (Mesa) on Linux. `egui_commonmark` with `default-features = false` and `pulldown_cmark`. `rfd` with `default-features = false` and `xdg-portal`, `wayland`: no GTK in the dependency graph (**run** on Windows; Linux dependency graph inspected, not built).
- The application trait is `eframe::App` with `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)`; there is no `update`. Start-up is `eframe::run_native(name, NativeOptions, Box::new(|cc| Ok(Box::new(app))))`; `cc.egui_ctx` is the `Context` (**run**).
- Layout uses `egui::Panel::{top, bottom, left, right}(id).show(ui, …)` followed by `egui::CentralPanel::default().show(ui, …)`; the older `SidePanel` and `TopBottomPanel` are gone. Side panels are resizable; `show_collapsible(ui, &mut bool, …)` hides one (**run**).
- Editor: `TextEdit::multiline(&mut String)` inside a `ScrollArea`, with `.layouter(&mut |ui: &Ui, buf: &dyn TextBuffer, wrap_width: f32| -> Arc<Galley>)` building a `LayoutJob` for colouring; `output.response.changed()` reports edits (**run**).
- Preview: `CommonMarkViewer::new().show(ui, &mut cache, markdown)`. A link is intercepted by registering its exact, undecoded destination with `cache.add_link_hook(dest)` and reading `cache.get_link_hook(dest) == Some(true)` right after `show`; the value is true for exactly one frame and no browser is opened. Wikilinks must therefore be rewritten to Markdown links with a private scheme before rendering. Links that are not hooked open in the browser (**run**).
- Graph: `ui.allocate_painter(size, Sense::click_and_drag())`, `painter.circle_filled`, `line_segment`, `text`; dragging through `response.dragged()` and `drag_delta()`, hit-testing the start of a drag at `ui.input(|i| i.pointer.press_origin())`; zoom from `smooth_scroll_delta.y` and `zoom_delta()` (**run**).
- Shortcuts: `ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)))`, consumed before the editor is drawn. Popups: `egui::Modal::new(id).show(ctx, …)`, closing on Escape through `response.should_close()` (**run**).
- Background threads: `egui::Context` is `Clone + Send + Sync`; a thread sends on a channel and calls `ctx.request_repaint()` (**run**).
- Headless tests: `egui_kittest::Harness::builder().build_ui_state(|ui, state| …, state)` drives the real UI code with no GPU: click by label or role, type text, press keys, drag, scroll. It fails a test when a text field has no accessible name, so every field gets a hint text or a label (**run**: 17 tests).
- Release binary of the demo: 7.5 MiB, or 4.8 MiB with size optimisation. Cold debug build about 2.5 minutes (**run**).
- Not verified: any Linux build or run of the GUI; behaviour on machines without OpenGL 2.1; the folder dialog (compiled, never opened).

### A.4 Sync document, folder watching, locking: `loro` 1.17, `notify` 8.2, `std`

**loro**

- Root containers are addressed by name and merge by name: two documents that each write into root text `X` end up with one text holding both writes (**run**). A root name must be non-empty, must not contain `/` or NUL and must not start with `🤝:` (**source**: `loro-common-1.17.0/src/lib.rs`, `check_root_container_name`). Note keys contain `/`, so the text of a note is the root text named `f:` followed by the key with every `/` replaced by `|`, a character that cannot occur in a note key.
- Importing a file: `LoroText::update_by_line(&str, UpdateOptions::default())` turns the difference between the current text and the given text into operations, line by line. `LoroText::update` does the same character by character (**run**).
  - Character-level results on same-line conflicts: both append → the two insertions side by side; one deletes a line while the other edits it → `line t modifiedhree`, text nobody wrote. Line-level results for the same cases: both versions of the line kept as two lines; the edited line survives (**run**).
  - Cost: ordinary edits of a 1 MB text take milliseconds in both modes. A change touching every line of a 1 MB text takes 1–3 s by line and over 100 s by character. Passing a timeout to `update` makes the diff itself about six times slower (**run**).
  - After either call `text.to_string()` equals the input exactly, including CRLF, a lone CR, a BOM, emoji and NUL (**run** for `update`; to be re-asserted for `update_by_line` in the milestone M2 tests).
- Two replicas that create the same text independently with identical content end up with the content twice; with different content, with both, ordered by peer id (**run**). This is why joining resets the local document first (section 8.6) and why the limitation at the end of section 8.5 exists.
- Peer id: a new `LoroDoc` has a random one, also after `LoroDoc::from_snapshot`. Reusing a fixed peer id after restoring an older snapshot corrupted the text on the other replica without any error (**run**). The engine therefore never sets a peer id.
- Versions and updates: `doc.oplog_vv()`, `VersionVector::encode() -> Vec<u8>`, `VersionVector::decode(&[u8])`, `doc.export(ExportMode::updates(&vv))`, `doc.import(&bytes) -> ImportStatus { success, pending }`. Importing the same update twice changes nothing. An update whose dependencies are missing is held as `pending` and applied when they arrive (**run**).
- Maps: `doc.get_map(name)`, `insert`, `delete`, `get`, `for_each`. An entry is a last-writer-wins register, deterministic on every replica. A deleted key is indistinguishable from a key never set, and inserting a value equal to the current one creates no operation (**run**).
- Persistence: `doc.export(ExportMode::Snapshot)` and `LoroDoc::from_snapshot(&bytes)`. A document of 1000 notes of 2.3 KB each is a 1.2 MB snapshot; loading takes about 10 ms. Shallow snapshots are smaller but replicas behind the cut can no longer catch up incrementally, so they are not used (**run**).
- `LoroDoc` is `Send + Sync`. Edits are visible immediately; `export` and `import` commit implicitly (**run**).
- Build: about 160 crates, cold debug build about 3 minutes, needs a C compiler for one build script (**run**).

**notify**

- Latest stable is `notify` 8.2.2 (9.0 is a release candidate) (**run**).
- On Windows the event kinds are unreliable as facts: an atomic replace arrives as `Remove` then `Create` of the destination; renaming or deleting a directory reports the directory only, not the files inside it; one write produces several events (**run**). On Linux a queue overflow is reported as a rescan request, files created in a brand-new directory can be missed, and recursive watches count against `fs.inotify.max_user_watches` (**source**, not run).
- Consequence, already the design in section 8.5: an event is only a hint that something changed. The engine waits for the folder to be quiet for the debounce period and then compares the whole vault. No debouncer crate is needed.

**File locking**

- `std::fs::File::try_lock()` is stable since Rust 1.89. A second process gets `TryLockError::WouldBlock` while the first holds the lock, and the lock is released when the holder is killed (**run** on Windows). The lock file must be a dedicated file opened with read and write access.

### A.5 Glass look: `window-vibrancy` 0.8.1, Inter 4.1 (spike of 2026-11-17, Windows 11 build 26200)

- `window-vibrancy` 0.8.1 shares `raw-window-handle` 0.6.2 with eframe 0.37, egui-winit, winit 0.30.13, glutin and accesskit_winit: one node in `cargo tree -i` (**run**). Its unsafe code stays inside the crate; the calls are safe.
- Real Windows material: `ViewportBuilder::with_transparent(true)`; `eframe::App::clear_color` overridden to return `[0.0; 4]` (the default `rgba(12,12,12,180)` gives a grey wash); `Visuals::panel_fill` transparent and every `Frame` fill with its own alpha. `&CreationContext` and `&eframe::Frame` both implement `HasWindowHandle`, so `window_vibrancy::apply_mica(cc, Some(dark))` works at start and `apply_mica(frame, Some(dark))` / `clear_mica(frame)` work later (**run** at start; later calls **source** only). The native title bar also gets the material (**run**).
- Mica follows the dark flag (measured mean luminance of the bare title-bar row: 242.5 light, 32 dark). Acrylic does not: on Windows 11 22621+ `apply_acrylic` ignores its colour and gives the same darkened blur in both themes (112.6 in both), which made the light theme's gutters grey. Hence Mica, not Acrylic (**run**).
- Window drag and corner resize with Mica: the window moved and resized exactly, the frame during resize was fully laid out, a thin black strip shows at the right and bottom edges during a live drag (**run** with synthetic input). Smoothness as a person feels it is not verified; the crate warns of resize lag with Acrylic on some Windows builds.
- On Linux `window-vibrancy` returns an error («effects are controlled by the compositor»); `winit`'s `set_blur` is KDE-Wayland only and eframe 0.37 does not expose it. Linux uses the simulated glass. Not verified on Linux at all.
- Simulated glass: an opaque window, a neutral gradient `Mesh` on `LayerId::background()`, `egui::Frame` with a fill alpha, 1 px stroke, `corner_radius`, `shadow` and `outer_margin`; the top highlight drawn with `painter.line_segment`. `Panel::show_separator_line(false)` on every panel and `noninteractive.bg_stroke` set to none remove the separator lines (**run**).
- Selected labels: egui colours a selected label's text with `selection.stroke.color`; the spike set the stroke width to 0 and the colour to the text colour (**run**). The same stroke draws the focus outline of text fields, so the M3 plan sets it to the 2 px accent ring of section 9.6.
- Inter 4.1 static TTF from `https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip`, `extras/ttf/Inter-Regular.ttf` (411 640 bytes) and `Inter-SemiBold.ttf` (419 744 bytes), licence `LICENSE.txt` (SIL OFL 1.1). Loaded with `FontData::from_static` wrapped in `Arc`, inserted first in the proportional family, SemiBold as a named family `"bold"`, then `ctx.set_fonts`; sizes through `style.text_styles` (**run**, at 150 % display scale only).
- Theme: `ctx.system_theme() -> Option<Theme>` is filled from winit at start and updated on a system change; `ThemePreference::System` follows it; `ctx.set_theme(Theme::Light | Theme::Dark)` pins it. `ViewportCommand::SetTheme(SystemTheme::…)` sets the native title bar but only when sent from a frame, not from the creation closure, and it was missed once in four runs (**run**). A live system theme switch was not tested.
- Screenshots of a GL window: `BitBlt` with `SRCCOPY | CAPTUREBLT` (`0x40CC0020`); a plain screen copy gives a white client area (**run**).
- Spike code and screenshots (v1 with the rejected gradient, v2 refined) were kept in the session scratchpad only.
