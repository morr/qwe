# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required Reading

**`CONTEXT.md` — load it before touching any domain or simulation code.** It is the
domain glossary: what a navtile, flee fan, chase claim, bridge corridor, or prune pass
*is*, which invariants hold (fill order, z-ranges, SimSet ordering, telemetry math), and
where each concept lives. Use its terms verbatim in code, commits, and test names.
When a change introduces or retires a domain concept, update `CONTEXT.md` in the same
change — a stale glossary is worse than none.

**`OSM.md` — load it before touching the OSM pipeline** (the Overpass query, `map/osm/*`,
or anything that decides which map features get drawn). It is the coverage audit: which
tags reach the map, which ones are downloaded and thrown away, which ones are never asked
for at all, with per-city counts — so a feature that already exists does not get
"added" twice, and the cheap gaps are visible. It also carries the two scripts in
`tools/osm_audit/` that regenerate those numbers. Widening the query or adding a branch to
`parse_way` means updating `OSM.md` in the same change.

This file is the *how to work here*; `CONTEXT.md` is the *what this project is*.

## Skills

Skills hold the detail; this file holds the map. Load them — don't reconstruct their content from here.

- **`git` — MANDATORY before any git operation** (staging, committing, branching, rebasing, history inspection, conflict resolution). Blocking prerequisite: do NOT run git commands until it is loaded.
- **`live-app` — before running the app** (`cargo run`, smoke-testing in the real app, querying the live world over BRP on port 15702). The skill is engine-level and shared with zxc; **this project's inventory lives in `.claude/live-app-project.md`** (ready markers, `SimSpeed` vs `Time<Virtual>`, screenshots, camera, registered types, toggles) and `.claude/live-app.json` configures the `brp` CLI for it. Read the appendix together with the skill.
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

### Running cargo so progress stays visible

A cold `cargo build` here takes minutes. Two hard rules:

- **Always run `cargo build` / `run` / `test` / `clippy` with `run_in_background: true`.**
  Output streams to a background task the user can watch live; the model is notified on
  exit. A foreground cargo command blocks the session with nothing on screen.
- **Never pipe cargo through `tail` / `head` / `grep`.** The pipe buffers everything until
  the process ends, so no progress is visible, and cargo drops its progress bar when
  stderr is not a tty. Use `--message-format=short` when the output needs to be smaller;
  read the tail of the finished background output instead of pre-trimming it.

Bash timeouts are raised to 10 min in `.claude/settings.json` (`BASH_DEFAULT_TIMEOUT_MS`)
for the cases that do run in the foreground.

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

### Performance check

**Every task ends with a judgement call on performance, stated out loud.** First reason
about it empirically — could this change have made anything slower? Look for the usual
suspects:

- new work per frame or per fixed step, especially anything iterating entities, tiles or
  map features;
- new work per map element at load (parse, navmesh fill, meshing) — the map carries tens
  of thousands of buildings and roads, so an innocent nested loop is quadratic;
- more geometry in a merged mesh, more entities, more gizmos;
- a new allocation, lock, or index rebuilt on every call instead of once.

If the answer is a confident **no**, say so in one line and move on — do not measure for
the sake of measuring.

If the answer is **yes or maybe**, measure before and after and report the numbers.
What to measure, by kind of change:

- load-time work — the timings already logged (`navmesh filled in …`, `pruned … in …`),
  or wrap the new step in `Instant::now()` the same way;
- per-frame work — `fps` / frame time from the diagnostics overlay in the live app, on
  the same city and the same camera position, and the `sim/*_ms` diagnostics for
  simulation systems;
- memory or entity count — `brp count`, the `entities:` line in the overlay.

Then **give a verdict**: the cost is negligible and stays; the cost is real and here is
the optimisation; or the cost is real, here is what it buys, and it is the user's call.
Never report "probably fine" without either the reasoning or the numbers behind it.

## Cargo Features & Dependencies

`bevy` is pulled in with `default-features = false` — only `2d`, `ui`, `dynamic_linking`,
`bevy_dev_tools`, `bevy_remote`, `bevy_camera_controller`, `pan_camera`, `default_font`.
There is no 3d stack and no audio; adding a feature that needs them means editing
`Cargo.toml` deliberately, not by accident.

Current third-party crates and why they exist:

- `pathfinding` — A*/Dijkstra/Fringe/BFS over the navmesh grid
- `bevy_northstar` — hierarchical HPA*/Theta* (its `Grid` is used directly, without the
  crate's plugin)
- `polyanya` — polygonal navmesh (`navigation/polymesh/`): CDT + agent-radius
  inflation of vector obstacles; used directly, no bevy integration. `polyanya_glam` is a
  renamed `glam` 0.30 dep only to construct its `Vec2` (bevy 0.19 ships glam 0.32)
- `rand` 0.9 — gameplay randomness (`rand::rng()`, `random_range`)
- `serde` / `serde_json` — Overpass JSON parsing
- `ureq` 3 — blocking HTTPS download of the OSM extract (rustls + gzip by default)
- `earcutr` — polygon triangulation (with holes) for merged map meshes
- `i_overlay` — boolean union of building shadow sweeps, so the translucent shadow
  layer never overlaps itself

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
- **Every entity of the game world carries `DespawnOnExit(AppState::Playing)`** — see
  "World entities" below. Adding a spawn site without it leaks the entity into the next
  city.
- Tuning constants (sizes, speeds, radii, z-layers) live in `src/settings.rs`, not
  inline in systems
- Clippy `type_complexity` is allowed globally; `wildcard_imports` warns
- Formatting: block indent style, reorder imports (`.rustfmt.toml`); fmt needs the
  nightly rustfmt (see Verification)
- Keyboard/mouse gates belong in the schedule as run conditions
  (`run_if(input_just_pressed(..))`), not as an early `return` inside the system

## UI input must not reach the game world

**A mouse event that lands on a UI panel belongs to the UI and to nothing else.** A click
on a button, a drag of a slider, a scroll over a panel must never also pan or zoom the
camera, select or command anything in the world, or trigger a world-space gizmo — the
panels sit *over* the map, so every one of those reads as the map reacting to a click the
user aimed at a widget.

Anything reading raw mouse state (`ButtonInput<MouseButton>`, `CursorMoved`,
`AccumulatedMouseScroll`) to act on the world therefore has to check first. The idiom is
`bevy::picking::hover::HoverMap` — if any hovered entity has a `Node`, the pointer is over
the UI (`camera.rs::pointer_over_ui`, ported from `zxc/src/input.rs`). Systems driven by
`Pointer<…>` observers on world entities need no gate: UI picking already consumes those.

For a **drag**, decide once, in the frame of the press, and hold that decision until the
button is released (`camera.rs::DragPan`). A per-frame "is the cursor over UI" test breaks
the moment a drag leaves the panel — pulling a slider to its end hands the rest of the
gesture to the camera.

## World entities — the `DespawnOnExit` rule

Switching the city (`City` resource, panel at the bottom centre) reloads the world by
sending the app back to `AppState::Loading`. Nothing despawns the previous city by hand:
the scene is cleared **only** by `DespawnOnExit(AppState::Playing)` on each entity.

So: **anything spawned while `Playing` that belongs to the world — a unit, a corpse, a
map mesh, a tree, an overlay, a marker, a projectile — must be spawned with
`DespawnOnExit(AppState::Playing)`.** No exceptions, including entities spawned from
observers, `FixedUpdate` systems, or dev tools. Current spawn sites (keep this list in
step when adding one):

| entity | file |
|---|---|
| ground sprite, merged area layer meshes (parks/woods/grass/sand/water), waterway ribbons | `map/spawn.rs` |
| road layers (alleys/roads/rails/walls + casings) | `map/roads.rs::spawn_roads` |
| tram mesh | `map/tram.rs::spawn_tram` |
| building layers (facades/roofs/shadows/extrusion) | `map/buildings/mod.rs::spawn_buildings` |
| tree crowns + shadows | `map/trees.rs::spawn_trees` (geometry — `trees/crown.rs`) |
| portal | `portal.rs` |
| humans (and corpses — same entity, retagged) | `human/systems.rs::spawn_population` |
| demons | `demon/systems.rs::spawn_demon` |
| navmesh overlay | `ui/debug.rs::sync_navmesh_overlay` |
| conifer noise overlay | `ui/debug.rs::sync_conifer_noise_overlay` |
| polymesh overlay | `ui/navigation/overlay.rs::sync_polymesh_overlay` |
| test walker | `dev.rs::on_spawn_test_walker` |

Not world entities, and deliberately without the component: the camera (`camera.rs`), the
UI panels (`ui/*`, hidden/shown via `GameUiRoot`), the loader screen (`loading.rs`, has
its own despawn on `PlayPhase::Live`).

The rule is backed at runtime by `loading.rs::warn_leftover_world_entities`: on every
entry into `Loading` it warns about anything that still has a `Transform` and is neither a
camera nor a UI node. A `world reload: N scene entities survived Playing` line in the log
means a spawn site is missing the component — fix the site, don't silence the warning.

### Where a mass despawn may happen

**Never despawn world entities from the middle of `Update`.** Every `Update` system that
touches pawns — `human::pick_wander_targets`, `dispatch_pathfinding_requests`,
`listen_for_pathfinding_tasks` — hands the schedule a command buffer that is applied at
the end of the run. A despawn from a system or observer in the same schedule kills the
entities those buffers name, and the buffer then panics with `Entity despawned`.

The two existing mass despawns are both outside `Update` by design: the city switch runs
in `StateTransition` (`DespawnOnExit`), and restart on R triggers from `PreUpdate` after
`InputSystems` (`restart.rs`). `escape` despawns a single human inside `FixedUpdate`,
where the chained `SimSet`s give it a sync point. A new despawn site belongs in one of
those slots — silencing the command errors instead only hides the ordering mistake.

## Bevy Time Types

`Res<Time>` is a context-sensitive alias — use it for **delta accumulation and timers** in any schedule:
- In `Update` → resolves to `Time<Virtual>` (virtual/scaled delta)
- In `FixedUpdate` → resolves to `Time<Fixed>` (per-step virtual delta)

**Never use `Res<Time<Virtual>>.delta_secs()` for timer accumulation inside `FixedUpdate`.** `Time<Virtual>` is updated once per frame; when FixedUpdate runs multiple times per frame (at high time_scale), each run gets the full frame delta, causing timers to advance `time_scale`× too fast.

Use `Res<Time<Virtual>>` when you need **`elapsed_secs()`** — the current total virtual time.

Use `Res<Time<Real>>` only for things that must ignore pause and time_scale.
