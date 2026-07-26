# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

Skills hold the detail; this file holds the map. Load them — don't reconstruct their content from here.

- **`git` — MANDATORY before any git operation** (staging, committing, branching, rebasing, history inspection, conflict resolution). Blocking prerequisite: do NOT run git commands until it is loaded.
- **`live-app` — before running the app** (`cargo run`, smoke-testing in the real app, querying the live world over BRP on port 15702).
- **`bevy` — when writing or debugging Bevy code.** Bevy 0.19 API facts that pre-0.19 training data gets wrong.

All three are symlinks into `zxc/.claude/skills/` — editing one edits zxc's copy too (see Reference Points).

**Caveat on the `bevy` skill:** it was written for zxc. Its "0.19 facts that get written wrong" section and `references/api_0_19.md` are engine-level and apply here verbatim. Its "This project's conventions" section and the other references describe **zxc's** machinery — `crate::prelude::*`, `exclusive_state_tags!`, `config()`, `log_state_change!`/`log_event!`, z-index constants, the `debug_ui` feature. None of that exists in this project; don't introduce it just because the skill mentions it.

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

`dynamic_linking` is already enabled in `Cargo.toml` — never pass `--features bevy/dynamic_linking`.

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
- Keyboard/mouse gates belong in the schedule as run conditions (`run_if(input_just_pressed(..))`), not as an early `return` inside the system

## Third-party Crates

There are none yet, and 0.19 absorbed a lot of what used to need them. Check `~/develop/bevy/bevy/crates/` for a first-party option before adding a dependency — notably `bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin}` (pan/zoom, `min_zoom`/`max_zoom`) covers what `bevy_pancam` is used for in zxc. Don't copy zxc's dependency set wholesale; it predates those additions.

## Bevy Time Types

`Res<Time>` is a context-sensitive alias — use it for **delta accumulation and timers** in any schedule:
- In `Update` → resolves to `Time<Virtual>` (virtual/scaled delta)
- In `FixedUpdate` → resolves to `Time<Fixed>` (per-step virtual delta)

**Never use `Res<Time<Virtual>>.delta_secs()` for timer accumulation inside `FixedUpdate`.** `Time<Virtual>` is updated once per frame; when FixedUpdate runs multiple times per frame (at high time_scale), each run gets the full frame delta, causing timers to advance `time_scale`× too fast.

Use `Res<Time<Virtual>>` when you need **`elapsed_secs()`** — the current total virtual time.

Use `Res<Time<Real>>` only for things that must ignore pause and time_scale.
