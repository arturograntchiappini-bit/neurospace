# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `sb` command line: `list`, `read`, `write`, `append`, `delete`, `rename`, `search`, `links`, `graph` and `tags`, with text or `--json` output.
- `sb mcp`: an MCP stdio server exposing the note operations as ten tools (`list_notes`, `read_note`, `write_note`, `append_to_note`, `delete_note`, `rename_note`, `search_notes`, `get_links`, `get_graph`, `list_tags`) for AI agents.

### Changed

- Reading a note larger than 4 MiB is refused instead of loading it; writing over a note that is a symbolic link is refused; replacing a note keeps its file permissions.
- The folder filter of `list` and `list_notes` is validated like a note path (`Projects\Sub`, `../x`, `./projects` and hidden folders are errors) and accepts one trailing `/`.
- Note paths reject more Windows device names (`CON .md` with trailing spaces, `COM0`, `LPT0`, `COM¹`–`COM³`, `LPT¹`–`LPT³`, `CONIN$`, `CONOUT$`), because a Linux peer may legally hold such names.
- Creating a note writes a temporary file and links it into place, so a crash never leaves a half-written file under the note's name.
- Writing or appending to a read-only note is refused with a permission error on every platform; it used to replace the file on Linux.
- MCP tool descriptions now state limits, result shapes and that rename does not rewrite links; read-only and destructive tools carry annotations.

### Fixed

- Renaming a note onto an existing file could replace it without a trace (on Windows, by naming it with its 8.3 short name); rename now refuses whenever anything is at the destination.
- One folder that could not be listed (access denied) made list, search, links, graph and tags fail; it is now reported as ignored and the scan goes on.
- Two MCP tool calls at the same moment could lose text (`append_to_note` on the same note); tool calls now run one at a time.
- A `[[Note#Heading\|Alias]]` link inside a table kept a backslash in its heading.
