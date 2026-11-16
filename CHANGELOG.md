# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `sb` command line: `list`, `read`, `write`, `append`, `delete`, `rename`, `search`, `links`, `graph` and `tags`, with text or `--json` output.
- `sb mcp`: an MCP stdio server exposing the note operations as ten tools (`list_notes`, `read_note`, `write_note`, `append_to_note`, `delete_note`, `rename_note`, `search_notes`, `get_links`, `get_graph`, `list_tags`) for AI agents.
