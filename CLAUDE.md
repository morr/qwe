# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required Reading

**`CONTEXT.md` — load it before touching any domain or simulation code.** It is the
domain glossary: what a navtile, flee fan, chase claim, bridge corridor, or prune pass
*is*, which invariants hold (fill order, z-ranges, SimSet ordering, telemetry math), and
where each concept lives. Use its terms verbatim in code, commits, and test names.
When a change introduces or retires a domain concept, update `CONTEXT.md` in the same
change — a stale glossary is worse than none.

This file is the *how to work here*; `CONTEXT.md` is the *what this project is*.

## Skills

Skills hold the detail; this file holds the map. Load them — don't reconstruct their content from here.

- **`git` — MANDATORY before any git operation** (staging, committing, branching, rebasing, history inspection, conflict resolution). Blocking prerequisite: do NOT run git commands until it is loaded.
- **`live-app` — before running the app** (`cargo run`, smoke-testing in the real app, querying the live world over BRP on port 15702).
- **`bevy` — when writing or debugging Bevy code.** Bevy 0.19 API facts that pre-0.19 training data gets wrong.

All three are symlinks into `zxc/.claude/skills/` — editing one edits zxc's copy too (see Reference Points).

**Caveat on the `bevy` skill:** it was written for zxc. Its "0.19 facts that get written wrong" section and `references/api_0_19.md` are engine-level and apply here verbatim. Its "This project's conventions" section and the other references describe **zxc's** machinery — `crate::prelude::*`, `exclusive_state_tags!`, `config()`, `log_state_change!`/`log_event!`, z-index constants, the `debug_ui` feature. None of that exists in this project; don't introduce it just because the skill mentions it.

## Project Overview

QWE is a 2D demon-invasion simulation prototype built with **Bevy 0.19** (Rust, edition
2024): the Tula city center generated from OpenStreetMap data, 20 000 wandering humans,
demons spawning from a portal. ECS with a plugin-based modular design; the simulation
runs in `FixedUpdate`, the world spawns in `OnEnter(AppState::Playing)` after the OSM
map loads. Full domain picture — `CONTEXT.md`.

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

First `cargo run` downloads the OSM extract from Overpass into `assets/osm/` (gitignored
cache); subsequent runs are offline. Deleting the cache file forces a re-download.

## Verification After Each Task

After completing any task, run these in parallel:

```bash
cargo build --verbose
cargo test --verbose
cargo clippy -- -D warnings
# fmt only the files you changed
RUSTFMT=~/.rustup/toolchains/nightly-aarch64-apple-darwin/bin/rustfmt cargo fmt -- src/changed_file.rs
```

For changes that only manifest at runtime (behavior, rendering, UI), verify in the live
app too — the `live-app` skill (BRP counts, telemetry, `TakeScreenshotEvent`) is the
tool for that, not guessing from code.

## Cargo Features & Dependencies

`bevy` is pulled in with `default-features = false` — only `2d`, `ui`, `dynamic_linking`,
`bevy_dev_tools`, `bevy_remote`, `bevy_camera_controller`, `pan_camera`, `default_font`.
There is no 3d stack and no audio; adding a feature that needs them means editing
`Cargo.toml` deliberately, not by accident.

Current third-party crates and why they exist:

- `pathfinding` — A* over the navmesh grid
- `rand` 0.9 — gameplay randomness (`rand::rng()`, `random_range`)
- `serde` / `serde_json` — Overpass JSON parsing
- `ureq` 3 — blocking HTTPS download of the OSM extract (rustls + gzip by default)
- `earcutr` — polygon triangulation (with holes) for merged map meshes

Before adding anything else, check `~/develop/bevy/bevy/crates/` for a first-party
option — 0.19 absorbed a lot of what used to need crates (e.g. first-party `PanCamera`
covers what `bevy_pancam` did in zxc). Don't copy zxc's dependency set; it predates
those additions.

## Code Conventions

- Each feature is a Bevy plugin registered in `main.rs`; typical module layout is
  `mod.rs` (plugin), `components.rs`, `systems.rs`, plus `behavior.rs` for a species'
  state machine (demon, human)
- State tags use `*Tag` suffix; plugins use `*Plugin` suffix; event/observer handlers
  use `on_*` prefix
- World spawning goes in `OnEnter(AppState::Playing)` under `WorldInitSet`
  (`Navmesh → Spawn`), never in `Startup`; per-tick simulation goes in `FixedUpdate`
  inside a `SimSet` and is gated on `Playing`
- Tuning constants (sizes, speeds, radii, z-layers) live in `src/settings.rs`, not
  inline in systems
- Clippy `type_complexity` is allowed globally; `wildcard_imports` warns
- Formatting: block indent style, reorder imports (`.rustfmt.toml`); fmt needs the
  nightly rustfmt (see Verification)
- Keyboard/mouse gates belong in the schedule as run conditions
  (`run_if(input_just_pressed(..))`), not as an early `return` inside the system

## Bevy Time Types

`Res<Time>` is a context-sensitive alias — use it for **delta accumulation and timers** in any schedule:
- In `Update` → resolves to `Time<Virtual>` (virtual/scaled delta)
- In `FixedUpdate` → resolves to `Time<Fixed>` (per-step virtual delta)

**Never use `Res<Time<Virtual>>.delta_secs()` for timer accumulation inside `FixedUpdate`.** `Time<Virtual>` is updated once per frame; when FixedUpdate runs multiple times per frame (at high time_scale), each run gets the full frame delta, causing timers to advance `time_scale`× too fast.

Use `Res<Time<Virtual>>` when you need **`elapsed_secs()`** — the current total virtual time.

Use `Res<Time<Real>>` only for things that must ignore pause and time_scale.
