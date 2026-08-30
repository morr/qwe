---
name: world-lifecycle
description: Use when working on how a qwe world comes up, restarts or is replaced — AppState/PlayPhase and the warmup hold (loading.rs), SimBootPlugin, WorldInitSet, the MapLoadJob background thread and its loader screen, the WorldStarted seam and run-state resets, RestartEvent/RestartPending, the City resource and the city-switch reload, DespawnOnExit enforcement. Deep detail behind CONTEXT.md's App lifecycle summary.
---

# World lifecycle — deep detail

Detail behind `src/loading.rs`, `src/restart.rs`, `src/city.rs` and
`src/map/osm/download.rs`. `CONTEXT.md` names the states and the seams; this file is the
mechanism behind them, and why each piece sits in the schedule slot it does.

**Changing that mechanism is changing this file, in the same turn** — a new phase, a moved
reset, a new spawn site that has to be torn down, a changed loader state. This file is what
the next session reads instead of the code. The term itself still goes to `CONTEXT.md`.

## The two states

**AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
(progress text, red error + **Retry** button on failure). **All world spawning happens in
`OnEnter(Playing)`**, never in `Startup`.

**PlayPhase** (sub-state of `Playing`) — `Warmup → Live`.

**WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. The navmesh must be
filled before the population spawns, or humans land in the river.

## Warmup

During **Warmup** the world exists but `Time<Virtual>` is **paused** and the loader screen
stays up reading "Routing pawns... N left".

`poll_warmup` counts pawns *inside the camera view* that still hold a `PathfindingRequest` /
`PathfindingTask` and flips to `Live` when none are left, or after `WARMUP_TIMEOUT` (10 s,
logged as a warning). `WARMUP_GRACE` (0.5 s) keeps "no requests yet" from meaning "done".

**It counts only what the dispatcher will actually serve** — `wanderers_dispatched_at_zoom`
for the zoom half and **`UrgentPath` for the species half**, both of them the dispatcher's
own. Its margin (1.0 screens — what the player can actually see) stays its own on purpose.
Until `UrgentPath` existed it re-derived the species half by hand, and "the same cutoff" was
only half true.

Reason for the hold: all 20 000 humans queue a path in the same frame, and without it the
visible ones stood still for the first seconds. Typical warmup **~0.15 s** — see
`HumanFirstWanderTag` in the `species-behavior` skill.

`Live` is what despawns the loader and reveals the game UI (`GameUiRoot`).

Under **determinism** the warmup means something else entirely — it waits for the wanted
navigation backend and skips the pawn count altogether; see the `determinism` skill.

## SimBootPlugin

`loading.rs` — how a world is brought up, **one implementation for the game and for the
replay app**: the two states, the `WorldInitSet` chain, the warmup pause, and the
**WorldStarted** announcement on entering `Live` (chained *before* the unpause).

The pause lives here rather than in `SimTimePlugin` even though it drives `Time<Virtual>`:
it is a property of the phase, not of the speed regulator — and the replay app cannot take
the regulator (it measures wall clock), so before the move it ticked during warmup while the
game did not, and had to announce the world start by hand to stay ahead of that tick.

**Unpausing in `StateTransition` takes effect from the next frame**: this frame's
`Time<Virtual>` was already advanced in `First`. Pinned by `loading.rs`'s own tests.

## MapLoadJob / JobState

`map/osm/download.rs` — a background `std::thread` that prepares everything not needing ECS:

```
Connecting{attempt} → Downloading{bytes,total,bytes_per_sec} → Parsing
→ BuildingNavmesh → Pruning → Done(LoadedWorld{map, portal}) | Failed(msg)
```

Polled via `Arc<Mutex<_>>` by `poll_job`; every state is a line on the loader screen. The
thread fills the navmesh through the `ArcNavmesh` handle and returns the snapped portal
position.

- `total` is `None` in practice (chunked answers, gzip strips `content-length`), so the
  screen shows MB + rate.
- **`Connecting` is mostly Overpass computing the query server-side** — measured 62 s before
  the first byte on Paris. The screen says "Waiting for Overpass" and ticks seconds off
  `Time<Real>`; minutes here are normal, not a hang.

**Rule: heavy init belongs in this thread, not in `OnEnter(Playing)`** — no frame is drawn
inside a schedule, so work there freezes the loader on its last message.

The query, the mirrors and the cache are the `osm-map` skill.

## WorldStarted — the run seam

`loading.rs`, event. "The world begins a new run" — the single seam both lifecycle paths
share. Fired from exactly **two** places:

- entering `PlayPhase::Live` (first launch, city switch);
- every restart (`on_restart` — it passes through no state, so `OnEnter` never refires for
  it).

**All run state is reset by observers of this event, each living in its owning module** —
`SimClock` + `TickDebt`, `SimTick` + the frozen `Backend`, `Telemetry`, `DemonSpawner`,
`SeparationStats` (the crowd-demo counters). `grep "On<WorldStarted>"` enumerates them.

**Membership is not kept by hand.** It is held from the outside by
`a_restart_replays_the_run`, which runs a second run in the *same* `App` and compares
fingerprints, so a forgotten reset diverges whether or not anyone wrote it down. What that
test cannot see is state invisible to both the simulation and the outcome counters; such a
reset needs its own pin next to its observer (the regulator has one in `sim_time`, the frozen
`Backend` one in `determinism`, the separation counters one in
`tests/movement.rs::a_new_run_inherits_no_separation_counters` — separation does not run
under determinism at all, so the fingerprint cannot see them). Details — the `determinism`
skill.

**Map-derived state is not run state.** `NorthstarGrid` and `PolyNavmesh` are cleared by the
city switch alone — a restart keeps the map, and with it the 12 s northstar hierarchy.

## Restart

**RestartEvent** (`restart.rs`, R key or BRP) — despawns humans / corpses / demons / walkers,
fires **WorldStarted**, respawns the population. The navmesh persists: it is filled once per
city. Under determinism this replays the previous run tick for tick.

**RestartPending** (`restart.rs`, resource) — "a restart was ordered". The only way to ask
for one from anywhere but the R key: changing the **world seed** or flipping
**Deterministic**, whether from the panel or over BRP.

`trigger_pending_restart` consumes it in **`PreUpdate` after `InputSystems`** — the same slot
the R key uses and for the same reason: `on_restart` tears the scene down inside an observer,
so triggering it from `Update` would kill entities that sibling systems have already queued
commands for (CLAUDE.md, "Where a mass despawn may happen").

It always fires `RestartEvent { to_portal: true }`: a changed world setting means a
*different* world, and leaving the camera where it was makes the restart invisible.

**RR** (double R within 0.5 s) and any `to_portal: true` restart move the camera to the
portal at `START_ZOOM` regardless of `CameraPositionMode` — the camera side is `ui-panels`.

## City and the city switch

**City** (`city.rs`, resource, remembered by `prefs.rs`) — which city the map is built from:
`Tula | NewYork | Paris | Berlin | London | Tokyo | DevilsLake`. Each carries its **geo
center** (bbox center of the Overpass extract), its **portal hint** and its **cache slug**.
`MAP_SIZE` and therefore the derived `grid_size()` are shared, so switching city never
resizes the navmesh.

**A city switch is a full world reload.** Writing `City` (the select or BRP) sends the app
back to `AppState::Loading`:

1. leaving `Playing` despawns the scene (`DespawnOnExit`, below);
2. the load thread downloads / re-parses the new extract;
3. it refills the same navmesh (`fill_from_mapdata` resets it first) and re-snaps the portal;
4. `OnEnter(Playing)` rebuilds map and population and resets the camera
   (`camera.rs::place_camera_on_world_ready`).

`NorthstarGrid`, `PolyNavmesh` and `WarmupProgress` are reset on the way; run state waits for
**WorldStarted** on the new world's `Live` entry.

**The switch is gated on `in_state(Playing)`** — restarting a load on top of a running one
would put two threads into one navmesh.

The same reload path is taken by the **navtile size** cycler, with one difference: the camera
stays where it was (same city, same spot under inspection).

## DespawnOnExit — the only teardown

**`DespawnOnExit(AppState::Playing)` is the *only* thing that clears the old city.** Every
world entity must carry it. The rule and the list of spawn sites live in **CLAUDE.md**
("World entities") — keep that list in step when adding a spawn site.

`loading.rs::warn_leftover_world_entities` warns on every entry into `Loading` if something
with a `Transform` that is neither a camera nor a UI node survived. A `world reload: N scene
entities survived Playing` line in the log means a spawn site is missing the component — fix
the site, never silence the warning.
