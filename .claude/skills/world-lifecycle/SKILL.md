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
`PathfindingTask` and flips to `Live` when none are left, or after `WARMUP_TIMEOUT` (logged
as a warning). `WARMUP_GRACE` keeps "no requests yet" from meaning "done".

**It counts only what the dispatcher will actually serve** — the zoom half goes through
`movement::waits_for_camera` (the dispatcher's own rule, warmup asking with margin 1.0 —
exactly the frame, what the player can actually see) and **`UrgentPath` for the species
half**. Until `UrgentPath` existed it re-derived the species half by hand, and "the same
cutoff" was only half true.

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
→ BuildingNavmesh → Pruning → BuildingMeshes
→ Done(LoadedWorld{map, portal, meshes}) | Failed(msg)
```

Polled via `Arc<Mutex<_>>` by `poll_job`; every state is a line on the loader screen. **The
navmesh work has two shapes, picked by the entry fence bucket** (`fences::entry_step_hidden`,
read from the `StyleSnapshot`'s entry zoom). A **visible** fence step — the fences lane is
expensive there (2.3–2.8 s on Paris), and the perf lesson of run №12 is that the lanes' race
for cores must not move its start — keeps the serial prefix: the thread fills the navmesh
through the `ArcNavmesh` handle and returns the snapped portal position
(`prepare::fill_navmesh_and_snap_portal`). After the portal snap it makes **one** call —
`Navmesh::open_gates_and_prune(map, portal, stage)` — which opens the **default fence gates**
and only then prunes, because the order is a rule: a fence that sealed a plot with doors has
to open before that plot is thrown away as unreachable. `Pruning` is not set by the thread
directly: the `stage` callback the call makes between the two steps is what logs the gate
line and flips `JobState::Pruning`. The gate step **writes into the `MapData`** the thread
hands back, so the polygonal mesh built later from the resource sees the same gates
(navigation-deep skill).

A **hidden** fence step (the entry zoom past the ladder's last bound) runs the **overlapped
load** instead (`prepare::build_overlapped_world`): a navmesh lane (fill → snap →
`open_sealed_fences` on a **clone** of the map) goes beside the meshing lanes; the prune
follows as a second wave once the gates lane joins (`prune_unreachable` + `drop_gate_scratch`
— the gates-then-prune order holds), and the polymesh lane — the only lane besides fences
that reads gates — builds from the clone, which has the gates already open. The clone's
gates merge into the world's `MapData` after all lanes join. The fences lane may start with
the first wave only because the gate guarantees the hidden step: `entry_layers` exits
before reading any gaps.

**Before everything, `run()` tries the binary snapshot** (`map/osm/snapshot.rs`, the
osm-map skill): a serialized `MapData` read instead of the parse, showing as `Parsing` on
the loader. A hit skips the JSON read and `parse` entirely — the gates and pruning above
run on the decoded map exactly as on a parsed one (the snapshot is written from the
parse output, before any of those steps touch it).

**The last stage meshes every map layer, in the same thread, in parallel groups**
(`map/prepare.rs::build_world_meshes`): buildings ∥ roads (both verge steps) ∥ polymesh ∥
fences (the entry bucket's step) ∥ the small lane (parking layout, surfaces, trees with the
tree-set compose and the conifer field, rails of every zoom step, the car placement) — one
thread per group, since the
`MapData` is final by then (compose runs before the lanes; it is the only writer) and each
group fills its own lists. Styles — height mode, road style/shape, tree styles, car
occupancy, the polymesh panel, the **entry zoom** (`camera::next_entry_zoom`, the same
decision `place_camera_on_world_ready` makes at entry: the fences lane must know the
bucket the camera will open on) — arrive as a `StyleSnapshot` taken at thread start
(`loading.rs::MeshInputs`); a knob changed mid-load is picked up by the regular `rebuilds_on`
rebuild on the first Playing frame, exactly like a change after entry. The thread writes the
lane-width global itself (from `MapData::knobs`, the same value `adopt_lane_width` writes at
entry) and chunks the opaque layers (`into_chunks`) — the old entry-frame cost, split 15.7 s,
lives here now. The polymesh lane follows the Navmesh panel's toggle and hands its build to
`PolyNavmesh::set_ready` in the `Done` branch, so the async panel build never starts. A panic
in any lane is `Failed`, not a hung loader — and so is a panic **anywhere on the load
thread**: `start_load_thread` wraps the whole body (`run` — download, snapshot, parse; the
serial `build_navmesh`; the `finish_world` heads/tails) in `catch_unwind`, and a caught
panic lands as `Failed("load thread panicked: <payload>")` with the Retry button showing
(no review №25, 2026-11-20: before the catch those panics stalled `JobState` mid-phase and
the loader screen hung silently forever). The navmesh `RwLock` and the job `Mutex` are
**poison-tolerant everywhere** (`unwrap_or_else(PoisonError::into_inner)` — `ArcNavmesh`
helpers, `Backend::grid`, the two overlapped-lane writes in `prepare.rs`,
`build_navmesh`'s write, `MapLoadJob::set`, and both `loading.rs` lock sites): a panic
holding the write guard poisons the lock for the rest of the process, and without
tolerance every Retry would panic on `PoisonError` forever — the recovery exists because
the next load overwrites the whole grid. Readers only run in `Playing`, which a failed
load never enters.

`Done` is the whole world: `poll_job` inserts `MapData`, `PortalPos`, the prebuilt
**`InspectIndex`** and a
**`PreparedMeshes`** resource; `OnEnter(Playing)` then only *spawns* — `spawn_map` (parking
layout resource, surfaces, the road split build — common layers once, both steps' own
layers, buildings), `spawn_rails` (all steps),
`cars::seed_parked_cars` (the placement cache pre-filled), `trees::spawn_prepared_trees`,
and `fences::rebuild_fences` takes the entry-bucket fence layer out of `PreparedMeshes`
(each system takes its lists out of `PreparedMeshes` with `std::mem::take`; the fence
layer was built for the predicted entry zoom — a mismatch with the camera's bucket falls
back to the in-frame build). The cheap,
camera-bucket-dependent meshes (cars' bodies, wagons, industry, tram, the tree-row
band) are still built at entry — they cost 1–15 ms each.

- `total` is `None` in practice (chunked answers, gzip strips `content-length`), so the
  screen shows MB + rate.
- **`Connecting` is mostly Overpass computing the query server-side** — measured 62 s before
  the first byte on Paris. The screen says "Waiting for Overpass" and ticks seconds off
  `Time<Real>`; minutes here are normal, not a hang.

**Rule: heavy init belongs in this thread, not in `OnEnter(Playing)`** — no frame is drawn
inside a schedule, so work there freezes the loader on its last message. The meshing is the
rule's second application, after the navmesh: entry-frame cost measured 15.7 s on Paris
before the move (journal entry 10), ~1.5 s after it.

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

**Process-level state is neither.** The silhouette atlas (`Silhouettes`, filled once in
`Startup` by `SilhouettePlugin` — an asset, not a world entity, which is why it belongs in
`Startup` next to the camera and the UI shell rather than in `OnEnter(Playing)`) depends
on neither the city nor the run and survives a restart and a city switch alike;
`spawn_population`, `spawn_demon`, `on_restart`, the kill observer and `spawn_portal`
read it as a plain resource. An app without `SilhouettePlugin`
(replay, tests) `init_resource`s it empty and draws squares — never a rebuild.

## Restart

**RestartEvent** (`restart.rs`, R key or BRP) — despawns humans / corpses / demons / walkers
(demon halos and blood pools are their children and go with them; souls are not on the list —
a `SoulMote` despawns itself, see `species-behavior`), fires
**WorldStarted**, respawns the population. The navmesh persists: it is filled once per city.
Under determinism this replays the previous run tick for tick.

**RestartPending** (`restart.rs`, resource) — "a restart was ordered". The only way to ask
for one from anywhere but the R key: changing the **world seed** or flipping
**Deterministic**, whether from the panel or over BRP.

`trigger_pending_restart` consumes it in **`PreUpdate` after `InputSystems`** — the same slot
the R key uses and for the same reason: `on_restart` tears the scene down inside an observer,
so triggering it from `Update` would kill entities that sibling systems have already queued
commands for (CLAUDE.md, "Where a mass despawn may happen").

It always fires `RestartEvent { to_portal: true }`: a changed world setting means a
*different* world, and leaving the camera where it was makes the restart invisible.

**RR** (double press of R) and any `to_portal: true` restart move the camera to the
portal at `START_ZOOM` regardless of `CameraPositionMode` — the camera side is `ui-panels`.

## City and the city switch

**City** (`city.rs`, resource, remembered by `prefs.rs`) — which city the map is built
from; the enum in `city.rs` is the list. Moscow is nine extracts, one per administrative
okrug, since one `MAP_SIZE` frame covers only a sliver of it. Each city carries its **geo
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
stays where it was (same city, same spot under inspection). A settled new **lane width**
reloads the same way, and so does **Raw OSM** (`RawOsm`, Debug → World build):
`parse_knobs_diverged` (city.rs) builds the knobs as they stand — the settled
`RoadShapeOnMap`, the navtile resource, `RawOsm` — and hands them to
`ParseKnobs::diverged`, the snapshot's own comparison against `MapData::knobs`: the field
list of the reload lives at the snapshot, so a new parse knob joins the comparison with
its field. It is a comparison, not a change window, so a flip made during `Loading` is
picked up on the next `Playing` frame; a knob resource may also be absent (a scene
without `MapPlugin`), which reads as "has not moved". The `retuned::<NavtileBase>` arm
stays in the condition — the change window is the navtile's own detector beside the
comparison.

**What the load thread parses with is an argument.** `start_job` (and the Retry button)
build `ParseKnobs { lane_width, navtile, raw }` from the knobs as they stand and hand them to
`start_load_thread`; the parse reads no global. `sync_navtile_size` still writes the navtile
atomic first, because the navmesh fill in the same thread reads it. The paint's lane-width
global is written on the other end, in `OnEnter(Playing)`, from `MapData::knobs`
(`roads::shape::adopt_lane_width`) — so the world is painted with the width it was parsed
with even if the knob moved while it loaded.

## DespawnOnExit — the only teardown

**`DespawnOnExit(AppState::Playing)` is the *only* thing that clears the old city.** Every
world entity must carry it. The rule and the list of spawn sites live in **CLAUDE.md**
("World entities") — keep that list in step when adding a spawn site.

`loading.rs::warn_leftover_world_entities` warns on every entry into `Loading` if something
with a `Transform` that is neither a camera nor a UI node survived. A `world reload: N scene
entities survived Playing` line in the log means a spawn site is missing the component — fix
the site, never silence the warning.
