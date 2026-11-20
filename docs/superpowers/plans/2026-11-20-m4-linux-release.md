# M4 — Linux and release — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Neurospace 0.1.0: a Linux release tarball built on old glibc, a graph view that comes to rest and paints cheaply at 10 000 notes, the cause of the slow first sync round found, and complete README/CHANGELOG.

**Architecture:** No new crate. Task 1 adds a build script and a `.desktop` file; Tasks 2–3 change the graph layout (`ns-app/src/state/graph_layout.rs`) and the graph view (`ns-app/src/ui/graph_view.rs`); Task 4 is a measured diagnosis in `ns-sync` with a fix only where the measurement points; Task 5 writes the release documents after the orchestrator's checks.

**Tech Stack:** Rust 1.98+, eframe/egui 0.37 (glow), loro 1.17, iroh 1.4; Docker images `rust:1-bullseye` (build) and `debian:bookworm` (GUI start check).

**Spec:** `docs/superpowers/specs/2026-11-15-second-brain-design.md` — sections 9.4 («Convergence at scale (M4)»), 9.6, 10, 11.2 «M4».

## Global Constraints

- All Rust in the product; no web/JS. Build and packaging scripts are POSIX `sh` and are tooling, not product.
- `.md` files on disk are the source of truth; no silent data loss (trash, conflict copies, «blocked», never «absent»).
- UI: simple, minimal, very legible and accessible, macOS-like glass, no coloured gradients; colour tokens of spec 9.6 are fixed; keep AccessKit roles/labels; respect «Reduce motion».
- `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; doc-comments on public APIs; no `unsafe`; no `unwrap()`/`expect()` outside tests; functions within 30 lines and 4 parameters; Conventional Commits.
- The graph layout module stays free of clocks and UI-toolkit types.
- Tests that measure time are `#[ignore = "... run in release"]` and print their numbers; they never sleep a fixed time.
- Never put a ticket or group secret in logs, errors, `status.json` or `Debug`.
- One git command per shell call; commits end with `Co-Authored-By: Claude <noreply@anthropic.com>`; never push; never bare `git stash`; do not dispatch subagents.
- Disk is tight (about 8 GB free on C:): at most one extra release build per task; no second target directory.

## Review Focus

1. A graph that is mid-drag when the new cooling would end the wake: the dragged node must still follow the pointer and the layout must wake again afterwards (Task 2 test).
2. A small vault (2–30 notes) after the physics change: it must still come to rest quickly and look as before (existing tests, plus Task 2 keeps `should_…` small-graph tests green).
3. Edges hidden or merged for speed must not hide the only link of a node the user is hovering: hover highlighting still draws that node's edges (Task 3 test).
4. A Linux user whose desktop has no xdg portal: the welcome screen's typed path keeps working (already true; Task 5 README says so).
5. The release script run twice in a row, or after a failed run: it overwrites `dist/` output cleanly and never packages a stale binary (Task 1: build with `--locked`, copy from the fresh target, fail on any error with `set -eu`).

---

### Task 1: Linux release script, `.desktop` file, app-id pin

**Files:**
- Create: `scripts/linux-release.sh`, `packaging/linux/neurospace.desktop`
- Modify: `crates/ns-app/src/main.rs` (use a shared app-id constant), `crates/ns-app/src/lib.rs` or `crates/ns-app/src/app.rs` (define it)
- Test: a unit test in `crates/ns-app` (where the constant lives)

**Interfaces:**
- Produces: `pub const APP_ID: &str = "neurospace";` in `ns_app` (public, documented); `main.rs` passes it to `with_app_id`.
- Produces: `dist/neurospace-<version>-linux-x86_64.tar.gz` (the orchestrator uses its `ns` for the Windows↔Linux check and its `neurospace` for the GUI start check).

- [ ] **Step 1: Write the failing test** `should_match_the_desktop_file_to_the_window_app_id`: `include_str!` of `packaging/linux/neurospace.desktop` (path relative to the crate); assert it contains the lines `[Desktop Entry]`, `Type=Application`, `Name=Neurospace`, `Exec=neurospace %f`, `Icon=neurospace`, `Terminal=false`, `Categories=Office;Utility;`, and `format!("StartupWMClass={APP_ID}")`.
- [ ] **Step 2: Run it** — `cargo test -p ns-app should_match_the_desktop_file` — expected: compile failure (no `APP_ID`, no file).
- [ ] **Step 3: Create the `.desktop` file** with exactly the keys of spec 10 («`.desktop` file»), including `Comment=Markdown notes with links, graph and peer-to-peer sync`; add `APP_ID` and use it in `main.rs`.
- [ ] **Step 4: Run it** — expected: PASS.
- [ ] **Step 5: Write `scripts/linux-release.sh`** (`#!/bin/sh`, `set -eu`), run from the repository root inside `rust:1-bullseye`:
  - read the version from the workspace `Cargo.toml` (`[workspace.package] version`);
  - `cargo build --release --locked -p ns-cli -p ns-app` with `CARGO_TARGET_DIR` honoured if set (default `target`);
  - for each of `ns` and `neurospace`: list the `GLIBC_x.y` versions it needs (`objdump -T`), and exit non-zero with a message naming the symbol version if any is newer than 2.31;
  - stage into a fresh temporary folder `neurospace-<version>-linux-x86_64/`: `neurospace`, `ns`, `neurospace.desktop`, `neurospace.png` (copy of `assets/logo/neurospace-256.png`), `README.md`, `Inter-LICENSE.txt` (from `crates/ns-app/assets/`); `tar -czf dist/neurospace-<version>-linux-x86_64.tar.gz` (create `dist/`, replace an existing file); print the tarball path and the highest glibc version found.
  - Add `dist/` to `.gitignore` if it is not already ignored.
- [ ] **Step 6: Run it for real** (PowerShell, from the repo root; named volumes keep the build off the Windows bind mount):
  `docker run --rm -v "${PWD}:/src" -w /src -v ns-release-target:/target -v ns-release-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target rust:1-bullseye sh scripts/linux-release.sh`
  Expected: the tarball exists; `tar -tzf` lists the six files; the reported highest glibc is ≤ 2.31. Run it a second time: same result, the tarball is replaced.
- [ ] **Step 7: Gates** — fmt, clippy, `cargo test -p ns-app`.
- [ ] **Step 8: Commit** — `feat(release): add the Linux release script and desktop entry`.

### Task 2: Graph layout comes to rest at 10 000 nodes

**Files:**
- Modify: `crates/ns-app/src/state/graph_layout.rs` (constants, `step`/`advance`/`repulsion`/`is_settled`, tests)
- Test: same file (`mod tests`); `crates/ns-app/tests/ui_graph.rs` must stay green unchanged

**Interfaces:**
- Consumes: `Layout::{new, update, step, is_settled, rest, set_position, is_simplified}`, `MAX_STEPS_PER_WAKE`, `SIMULATION_NODE_LIMIT` (unchanged public API).
- Produces: the same public API. A new public constant is allowed only if `graph_view.rs` needs it.

Facts measured at the end of M3b (release, headless, 10 000 nodes): a step costs ~15 ms; at every sampled step the largest, p99 and p90 moves sit at `MAX_STEP` = 30 and the mean is 21–29, for 6, 20 and 40 links per node; repulsion uses `MIN_DISTANCE_SQUARED` = 1 with force `REPULSION·mass/d²`, so close pairs get pushes far above the speed limit; springs are `SPRING_STIFFNESS` = 0.02 per link, unnormalised by degree; there is no cooling. In the release GUI a frame costs 53–74 ms with about one step per frame, so ~150 steps fit in the 10 s wake cap today (Task 3 lowers the frame cost).

- [ ] **Step 1: Measure first** (no code change): add the two generators below to `mod tests` and an ignored test `measure_large_layout` that prints, for each graph, the step time and the mean/p90/max move at steps 1, 50, 150, 300, 600. Run `cargo test --release -p ns-app --lib measure_large_layout -- --ignored --nocapture`; put the table in the report.
  - `clustered_graph(clusters: usize, size: usize, seed: u64) -> Graph`: node ids `c{k}/n{i}.md`; each node links to 4 random other nodes of its cluster; one node in five also links to one random node of another cluster.
  - `random_graph_with_degree(count: usize, links_per_node: usize, seed: u64) -> Graph`.
  Use the deterministic generator style of the existing `random_graph` (an LCG on `u64`).
- [ ] **Step 2: Write the failing tests** (ignored, release):
  - `should_rest_by_its_own_rule_on_a_clustered_graph_of_ten_thousand`: `Layout::new(&clustered_graph(100, 100, 11))`; step until `is_settled()`; assert `steps < MAX_STEPS_PER_WAKE` (it rested by its own rule) and `steps <= STEPS_WITHIN_WAKE_CAP`, a constant in the test whose comment states the arithmetic (10 s ÷ the frame cost you expect after Task 3, at one step per frame; use 25 ms per frame unless your measurement says otherwise); then assert `mean distance node→own centroid <= 0.5 × mean distance from each centroid to its nearest other centroid`.
  - `should_rest_by_its_own_rule_on_a_random_graph_of_ten_thousand`: `random_graph_with_degree(10_000, 6, 5)`; same step assertions.
  Run them; expected: FAIL (they hit the step cap).
- [ ] **Step 3: Unit tests that run in debug** (not ignored):
  - `should_rest_by_its_own_rule_within_the_step_cap`: a 300-node clustered graph (`clustered_graph(10, 30, 3)`) rests with `steps < MAX_STEPS_PER_WAKE`.
  - `should_keep_following_a_drag_after_cooling`: run a 200-node graph to rest, then `set_position(0, [500.0, 500.0])` repeatedly while stepping: node 0 sits at the dragged point each step, and the layout is not settled until the drag stops and the rest of the graph settles again.
  - `should_not_push_a_coincident_pair_beyond_the_speed_limit`: two unlinked nodes at the same point move apart, each by at most the current move cap, and come to rest.
- [ ] **Step 4: Implement** in `graph_layout.rs`, smallest change the measurements support: a cooling move cap (temperature that starts at `MAX_STEP` on every wake and decays per step to a floor; the layout is settled when the temperature has reached the floor and the largest move is under it), a softened repulsion (minimum distance of the order of `SPRING_REST_LENGTH`/2), and spring force divided by the node's degree where that is what the numbers need. Tune only the constants spec 12 leaves to the builder. A wake (`update`, `set_position`, explicit wake) resets the temperature.
- [ ] **Step 5: Run** the debug tests, then the ignored release tests; the release tests PASS; re-run `measure_large_layout` and put the after-table in the report.
- [ ] **Step 6: Gates** — fmt, clippy, `cargo test -p ns-app` (every existing test in `graph_layout.rs`, `graph_view.rs`, `tests/ui_graph.rs` green; if a test pins an old constant, update it and say why in the report).
- [ ] **Step 7: Commit** — `perf(app): let the graph layout come to rest at ten thousand notes`.

### Task 3: Graph painting at 10 000 nodes: frame cost and memory

**Files:**
- Modify: `crates/ns-app/src/ui/graph_view.rs` (painting); `crates/ns-app/src/app.rs` only for a test hook if needed
- Test: `crates/ns-app/tests/ui_graph.rs` (new tests), an ignored release measurement test there or in `graph_view.rs`

**Interfaces:**
- Consumes: Task 2's `Layout` (unchanged API); existing `GraphView` hooks (`set_step_budgets`, `set_wake_cap`, `pane_rect`).
- Produces: no new public API beyond test hooks.

Facts: with the graph of a 10 000-note vault open (about 100 000 edges) a frame costs 53–74 ms, of which a layout step is ~15 ms; the process grows from 228 MB to 0.8–1.15 GB while the graph is open. Each edge and node is painted as its own shape today; edges whose bounding box misses the view are already skipped (M3b).

- [ ] **Step 1: Measure first:** an ignored release test that builds the view over a 10 000-node graph shaped like Task 2's `clustered_graph(100, 100, seed)` (that generator is private to `graph_layout.rs`'s tests; a copy in this test file is fine), through the app's normal path or the `GraphView` directly, runs one frame at the «Fit graph» zoom, and prints the number of shapes, the tessellated vertex/index counts (`Context::tessellate` on the frame's shapes) and the CPU time of painting plus tessellation. Put the numbers in the report, and say which part dominates.
- [ ] **Step 2: Write the failing tests:**
  - the ignored release test asserts painting plus tessellation at 10 000 nodes under 8 ms and the tessellated vertex count under a bound you derive from Step 1 (state it);
  - `should_draw_the_edges_of_the_hovered_node_at_any_zoom` (headless, not ignored): in a graph where edges would be hidden or merged at the current zoom, hovering a node still draws that node's edges (assert through the painted shapes or a test hook, as the existing hover tests do).
- [ ] **Step 3: Implement** the smallest change the numbers support (for example: edges batched into one mesh of thin quads instead of one shape each; edges below a zoom level drawn only for the hovered/open node, with a small hint in the pane; nodes as one mesh). Colours, radii and highlight rules of spec 9.4/9.6 unchanged; «Reduce motion» unchanged.
- [ ] **Step 4: Run** the tests; PASS; re-run the measurement and report before/after.
- [ ] **Step 5: Gates** — fmt, clippy, `cargo test -p ns-app`.
- [ ] **Step 6: Commit** — `perf(app): paint the graph of a large vault in a few milliseconds`.

### Task 4: Engine first round at 10 000 notes — measure, then fix where the measurement points

**Files:**
- Read first: `crates/ns-sync/src/engine.rs`, `local.rs`, `store.rs`, `state.rs`, `crates/ns-sync/tests/scale.rs`
- Modify: only where the measurement points; tests next to the code changed

**Interfaces:**
- Consumes: the existing engine, replica and store APIs; `crates/ns-sync/tests/scale.rs` (`should_reconcile_ten_thousand_notes_within_budget`, prints `first=`).
- Produces: unchanged public API unless the fix needs a change (then documented).

Facts: the first round of a release `ns sync run` on a generated vault of 10 000 notes and ~100 MB takes ~119 s (M2: 114 s), against 3.3–6.7 s for `first` in the scale test. Candidates named in the M3b ledger: the vault's texts (random words in the measurement vault vs repetitive texts in the test), compaction or fsync of the first update log, the engine's extra work per round (status keeper, watcher events caused by the round's own writes, per-note persistence), antivirus scanning of the state folder (Windows Defender is on; the measurement vault came from `<scratchpad>\ns_scale.py`, which you may run with `python -I` to generate a vault in your own temporary folder).

- [ ] **Step 1: Reproduce** both numbers in one sitting (release): the scale test's `first`, and `ns sync run` in `lan` mode on a generated vault (`NS_STATE_DIR` in a temporary folder; time from start to the first `status.json` with `busy: false` after the first round). Report both.
- [ ] **Step 2: Bisect the gap** with timings (temporary instrumentation, removed before commit, or `tracing` spans already present): which phase of the engine's first round takes the time. Test the antivirus hypothesis by putting vault and state under a folder excluded from scanning only if one already exists; do not change system settings. Report the phase table.
- [ ] **Step 3: If the cause is in this project's code:** write a failing test that pins the cause (for example a counter of persists or rescans per round on a small vault), fix it, and re-measure: the first round of `ns sync run` is at most 3× the scale test's `first` from the same sitting. **If it is outside the code:** no code change; the report carries the evidence.
- [ ] **Step 4: Gates** — fmt, clippy, `cargo test -p ns-sync` (scenario suite unchanged and green).
- [ ] **Step 5: Commit** (only if code changed) — `perf(sync): <what the measurement found>`.

### Task 5: Release documents

Runs after the orchestrator's milestone checks (Windows↔Linux sync, GUI start on Linux, graph and first-round numbers), whose results the orchestrator hands over in the dispatch.

**Files:**
- Modify: `README.md`, `CHANGELOG.md`

- [ ] **Step 1: README:**
  - «Status: under construction» → version 0.1.0, what it is, and that no person has used the GUI interactively;
  - new section «Install and build»: Windows (`cargo build --release`, where the two `.exe` land); Linux release (the `docker run` line for a Linux shell and for PowerShell, what the tarball holds, glibc 2.31 floor, the three `install -D` commands for `~/.local/bin`, `~/.local/share/applications`, `~/.local/share/icons/hicolor/256x256/apps`); Linux run-time requirements (X11 or Wayland, OpenGL 2.1, an xdg desktop portal for «Open a folder», otherwise type the path);
  - correct the sync section's «the desktop app will start it itself later» (it does, section «Sync window»);
  - «Desktop app → Verified, and not» and the graph paragraph updated with the orchestrator's M4 numbers (handed over in the dispatch);
  - new short section «Not in version 1» with the spec's non-goals (section 2) in plain words.
- [ ] **Step 2: CHANGELOG:** `[Unreleased]` → `[0.1.0] - <date given in the dispatch>`; add the M4 entries (Linux release script and desktop entry, graph convergence and painting, first-round finding) under the right headings; an empty `[Unreleased]` above it.
- [ ] **Step 3: Check** every command and path in the new README text against the repository (script name, file names, docker line) and every number against the dispatch.
- [ ] **Step 4: Commit** — `docs: release 0.1.0`.

---

## Orchestrator checks (milestone validation, not implementer tasks)

1. Windows: fmt, clippy, full suite. Linux (`rust:1-bullseye`): fmt, clippy, full suite as root, permission tests as `nobody`.
2. Tarball from Task 1: contents, glibc floor.
3. GUI start: the tarball's `neurospace` in `debian:bookworm` under Xvfb + Mesa (`LIBGL_ALWAYS_SOFTWARE=1`), a three-note vault, capture, SIGTERM.
4. Windows↔Linux sync: Linux container running the tarball's `ns sync run` in `lan` mode with `bind_port` fixed and published (`-p <port>:<port>/udp`); Windows `ns sync run` in `lan` mode; ticket with hand-written hints (`127.0.0.1:<port>` towards the container); edits on each side arrive on the other; record times.
5. Release GUI on the 10 000-note vault: graph rests before 10 s, median frame < 30 ms, working set < 600 MB.
6. First-round figure re-measured.
7. Final review on `opus`, one fix wave, re-review; `graphify --update`; merge to `main`; tag `v0.1.0`.
