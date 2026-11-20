# GitHub release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish Neurospace 0.1.0 as a private GitHub repository with a precise, professional README full of real screenshots and diagrams, an MIT licence, CI, topics and a GitHub Release.

**Architecture:** Documentation, licence, a CI workflow, a demo vault with a reproducible screenshot script (run in a Linux container under Xvfb, so no window ever appears on the user's screen), then the publication itself (repository, push, tag, release, topics) run by the orchestrator once the user has logged in with `gh`.

**Tech Stack:** Markdown (GitHub-flavoured, Mermaid), GitHub Actions, Docker (`debian:bookworm`, Xvfb, Mesa, `xdotool`), `gh`.

**Spec:** `docs/superpowers/specs/2026-11-15-second-brain-design.md` (sections 1–3 for what the product is, 10 for the release); the user's request in CLAUDE.md V.1 (2026-11-20) and the rulings in V.8 «Rulings della pubblicazione su GitHub».

Runs after M4 is merged into `main` and tagged `v0.1.0` locally, on a branch `docs/github-release` from `main`.

## Global Constraints

- Licence: MIT, copyright line `Copyright (c) 2026 artur` (the repository's git author name).
- Version 0.1.0; Rust floor `rust-version = 1.98`; platforms Windows and Linux; macOS not supported. Say this in exactly these terms.
- No claim in the README that the code, the tests or the M4 ledger contradict. Every number is copied from the current README (M1–M4 measurements); none invented. The «Verified, and not» content stays complete.
- No secret, ticket, personal path or the user's email in any committed file or image.
- Images: PNG, at most about 300 KB each, under `docs/images/`.
- No new dependency in any crate.

## Review Focus

1. Badges that do not render for a private repository (shields.io dynamic badges cannot read private repos): only static badges plus GitHub's own Actions badge.
2. Mermaid blocks that GitHub refuses to render (syntax errors show as an error box): each diagram checked against Mermaid syntax.
3. Screenshots that leak the container's paths or a test vault name; the window title and vault name in every capture must be the demo vault's.
4. CI that fails on GitHub runners for reasons local runs hide (missing Linux packages for `eframe`, Windows long paths, ignored release tests accidentally run).
5. Install commands in the README that do not match the release asset names.

---

### Task 1: Licence and CI

**Files:** Create `LICENSE`, `.github/workflows/ci.yml`; modify workspace `Cargo.toml` (`license = "MIT"` in `[workspace.package]`, inherited by every crate with `license.workspace = true`).

- [ ] `LICENSE`: the standard MIT text with the copyright line above.
- [ ] `ci.yml`: on push and pull request to `main`; jobs on `ubuntu-22.04` and `windows-latest`; steps: checkout, Rust 1.98 toolchain with rustfmt and clippy, cargo cache, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. On Ubuntu install the packages `eframe`/`glow` needs to build (find them from the crates' build scripts; nothing for a display: the UI tests are headless). Ignored tests stay ignored.
- [ ] Check: `cargo metadata` shows `MIT` for all four crates; the workflow is valid YAML (parse it); the Ubuntu package list is justified in the report.
- [ ] Commit `chore: MIT licence and CI workflow`.

### Task 2: Demo vault and screenshots

**Files:** Create `docs/demo-vault/` (about 30–45 short notes in English with wikilinks forming 3–4 visible clusters, a few tags, one folder level, one note linking to a not-yet-created note), `scripts/screenshots.sh`, `docs/images/*.png`.

- [ ] Demo vault: realistic knowledge notes (for example a reading list, a project, a trip, a cooking cluster), each note 3–15 lines, no personal data.
- [ ] `scripts/screenshots.sh`: runs inside `debian:bookworm` with the release tarball mounted at `/dist` and the repository at `/src`; installs Xvfb, Mesa, `xdotool`, `netpbm` (or `imagemagick`) in the container; starts `neurospace` on a copy of the demo vault under Xvfb (1440×900, `LIBGL_ALWAYS_SOFTWARE=1`, `NS_STATE_DIR` in `/tmp`, theme set through the app's settings file); drives it only with `xdotool` inside the container (shortcuts first: Ctrl+O, Ctrl+E, Ctrl+G); captures the app window; header comment with the `docker run` line for PowerShell and for a Linux shell.
- [ ] Captures (each in light and dark where it says so): `welcome.png`; `editor-dark.png` (a note open with backlinks and outgoing links in the side panel); `preview-light.png`; `graph-dark.png` and `graph-light.png` (the demo vault's graph at rest, labels visible); `quick-open.png`; `sync-window.png` (the sync window open, never an invite created: no ticket on screen).
- [ ] Check: look at every image; vault name is the demo vault's; no ticket, path or email visible; sizes within the limit.
- [ ] Commit `docs: demo vault and app screenshots`.

### Task 3: README for GitHub

**Files:** Modify `README.md`, `CHANGELOG.md` (compare links at the bottom).

- [ ] Structure: centred logo (`assets/logo/neurospace.svg`) and one-line tagline; static badges (version 0.1.0, licence MIT, Rust 1.98+, platforms Windows | Linux) and the GitHub Actions CI badge (`../../actions/workflows/ci.yml/badge.svg`); a hero screenshot; a table of contents.
- [ ] Sections: Highlights (short bullets, each backed by a feature that exists); Screenshots (a grid/table of the Task 2 images with captions); Quick start (download the release, or build); Install and build (the current README text, with the release asset names of Task 4); Using the app; Command line (a command table); AI agents (MCP setup snippets for Claude Code and Claude Desktop, the tool list as a table); Sync between devices (the current text, plus a Mermaid sequence diagram of pairing and a change flowing between two devices); Architecture (a Mermaid diagram of the four crates and their dependencies, the stack table, the source-of-truth rule); Performance (a table of the measured numbers already in the README); Verified, and not; Not in version 1; Roadmap (only items already named as limits or non-goals, phrased as possibilities, no promises); Contributing (fmt, clippy, tests, Conventional Commits); Licence.
- [ ] Every command, path, shortcut, number and file name checked against the repository and the current README; the report lists them. Known fix: «installed with the four commands above» becomes «with the commands above» (the install block has five lines since the `sed` line).
- [ ] Commit `docs: README for GitHub`.

### Task 4: Publication (orchestrator, after the user's `gh auth login`)

- [ ] Merge `docs/github-release` into `main`; move the local tag only if the release content changed (decide and record).
- [ ] Release assets: `neurospace-0.1.0-linux-x86_64.tar.gz` (from `scripts/linux-release.sh` on the final `main`) and `neurospace-0.1.0-windows-x86_64.zip` (`neurospace.exe`, `ns.exe`, `README.md`, `LICENSE`, `Inter-LICENSE.txt`, from `cargo build --release --locked`), with SHA-256 sums in the release notes.
- [ ] `gh repo create neurospace --private --source . --remote origin`, push `main` and the tag, `gh release create v0.1.0` with the CHANGELOG 0.1.0 section as notes and both assets, `gh repo edit --description ... --add-topic rust,markdown,note-taking,notes-app,obsidian-alternative,knowledge-graph,local-first,p2p,sync,crdt,egui,mcp,linux,windows`.
- [ ] Check: the README renders on GitHub (images, Mermaid, badges), the release has both assets, CI runs green on `main`.
