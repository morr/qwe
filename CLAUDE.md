# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

- **MANDATORY — load the `git` skill BEFORE any git operation** — staging, committing, branching, rebasing, history inspection, conflict resolution. This is a blocking prerequisite: do NOT run git commands until the skill is loaded. It contains hard rules (no Claude co-author trailer; never chain `git add` and `git commit` with `&&`; no `--no-verify`) and this repo's commit message conventions.
- **Load the `live-app` skill BEFORE running the app** — `cargo run`, smoke-testing a change in the real app, or querying the running world over the Bevy Remote Protocol on port 15702. It covers background launch, log-based verification, and shutdown.
- **Load the `bevy` skill when writing Bevy code** — ECS/component design, system ordering, UI patterns, common pitfalls.

## Project Overview

QWE is a 2D app built with **Bevy 0.19** (Rust, edition 2024), using Bevy's ECS with a plugin-based modular design.

## Reference Points

Two local checkouts to consult before inventing a pattern or guessing at an API.

### Upstream Bevy examples — `~/develop/bevy/bevy/examples`

A checkout of the Bevy repo at **release 0.19.0**, matching the version this project
builds against. Official, compiling, first-party examples — the authoritative answer for
"how is this API actually used in 0.19". Prefer them over recalled pre-0.19 syntax, which
is frequently wrong.

Relevant subdirectories: `2d/`, `ui/`, `ecs/`, `state/`, `app/`, `input/`, `camera/`,
`picking/`, `remote/` (BRP, incl. `integration_test.rs`), `dev_tools/`, `window/`,
`asset/`, `time/`. Grep them for a type name to find every real usage:

```bash
grep -rn 'OrthographicProjection' ~/develop/bevy/bevy/examples/
```

The full source is next to it — `~/develop/bevy/bevy/crates/` — when an example is not
enough and the implementation has to be read.

### Sibling project — `~/develop/bevy/zxc`

A mature Bevy 0.19 project by the same author. Use it for project-level shape (how things
are organised here), and the upstream examples for engine API questions. Worth reading:

- `main.rs` — plugin registration order, `DefaultPlugins` / window / log setup
- `src/*/` — module layout (`mod.rs` plugin, `components.rs`, `systems.rs`), state-tag
  state machines, observer usage
- `Cargo.toml` — the working set of Bevy 0.19 feature flags and companion crate versions
- `CLAUDE.md` — the conventions this file is derived from

Two rules:

- **Read-only.** Do not edit anything under `zxc/` while working on this project. The one
  exception is `.claude/skills/` — those are symlinked here, so a skill edit is shared by
  design and lands in zxc's git history; commit it there separately.
- **Copy patterns, not scale.** ZXC carries a lot of machinery (RON config, async
  pathfinding, task queue, egui debug UI) this project does not have. Take the idiom, not
  the whole subsystem.

## Build & Development Commands

```bash
cargo build --verbose       # Build
cargo run                   # Run (prefer the live-app skill)
cargo test --verbose        # Run all tests
cargo clippy -- -D warnings # Lint (warnings are errors)
```

Run a single test:
```bash
cargo test test_name --verbose
```

## Verification After Each Task

After completing any task, run these in parallel:

```bash
cargo build --verbose
cargo test --verbose
cargo clippy -- -D warnings
# fmt only the files you changed
RUSTFMT=~/.rustup/toolchains/nightly-aarch64-apple-darwin/bin/rustfmt cargo fmt -- src/changed_file.rs
```

## Cargo Features

`bevy` is pulled in with `default-features = false` — only `2d`, `ui`, `dynamic_linking`,
`bevy_dev_tools`, `bevy_remote`. There is no 3d stack and no audio; adding a feature that
needs them means editing `Cargo.toml` deliberately, not by accident.

## Code Conventions

- Each feature is a Bevy plugin registered in `main.rs`; typical module layout is `mod.rs` (plugin), `components.rs`, `systems.rs`
- State tags use `*Tag` suffix; plugins use `*Plugin` suffix
- Event/observer handlers use `on_*` prefix
- Clippy `type_complexity` is allowed globally; `wildcard_imports` warns
- Formatting: block indent style, reorder imports (`.rustfmt.toml`)

## Bevy Time Types

`Res<Time>` is a context-sensitive alias — use it for **delta accumulation and timers** in any schedule:
- In `Update` → resolves to `Time<Virtual>` (virtual/scaled delta)
- In `FixedUpdate` → resolves to `Time<Fixed>` (per-step virtual delta)

**Never use `Res<Time<Virtual>>.delta_secs()` for timer accumulation inside `FixedUpdate`.** `Time<Virtual>` is updated once per frame; when FixedUpdate runs multiple times per frame (at high time_scale), each run gets the full frame delta, causing timers to advance `time_scale`× too fast.

Use `Res<Time<Virtual>>` when you need **`elapsed_secs()`** — the current total virtual time.

Use `Res<Time<Real>>` only for things that must ignore pause and time_scale.

## Code Intelligence

An MCP language server (rust-analyzer) is available. Prefer it over grepping or reading files for:

- **Type information** — use `hover` on any symbol before assuming its type
- **Finding definitions** — use `definition` instead of searching for declarations
- **Finding usages** — use `references` instead of grep
- **Diagnostics** — check `diagnostics` after making changes
- **Renames** — use the LSP `rename` tool rather than find-and-replace

Fall back to reading files only for context LSP doesn't provide (comments, logic flow).
