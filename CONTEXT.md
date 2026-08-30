# CONTEXT

Domain glossary for QWE. Use these terms verbatim in commit messages, hypotheses, test
names, and code identifiers. If a concept you need isn't here, that's a signal — either
you're inventing language the project doesn't use (reconsider) or the file has a real gap
(update it in the same change that introduces the concept).

**What belongs here, and how much.** This file holds *terms and invariants*; the
*mechanisms and their justifications* live in seven detail skills —
`.claude/skills/{osm-map, navigation-deep, sim-speed, ui-panels, determinism,
species-behavior, world-lifecycle}` — loaded on demand (see CLAUDE.md, "Skills").
Budget per entry: **~5–10 lines** — what the term is, the invariant(s) that hold, where it
lives, at most a one-line why, then a pointer to the skill that owns the mechanism. A
measurement's *conclusion* stays here ("25 m is the measured median pitch"); the
methodology, the tables and the derivations go to the skill. A "this was tried and was
wrong because…" guardrail is one line, not a paragraph. **An entry that wants a table, a
derivation or a war story is telling you it belongs in a skill.** When a change touches a
concept, update the summary here and the detail in the skill **in the same change** — a
stale glossary is worse than none.

## Project shape

**QWE** is a 2D real-time simulation prototype: a **demon invasion of the Tula city
center**. The map is generated from real OpenStreetMap data at first launch. 20 000
humans wander the streets; demons pour out of a portal, chase and devour them; humans
panic and flee off-map. Built on **Bevy 0.19 ECS** — one plugin per feature, registered
in `main.rs`.

## Coordinates & units

- World units are **meters**. Origin — **south-west corner** of the map, y grows north.
  All world coordinates are positive. `MAP_SIZE = 5600 × 3700` m.
- **Navtile** — navigation grid cell, **2 m by default, runtime-switchable to 1 m** via the
  `navtile:` cycler in the Debug tab. Grid size is derived as `MAP_SIZE / navtile_size()`
  (2800 × 1850 tiles at 2 m); the live value is a process-global atomic
  (`settings::navtile_size()`), written only in `OnEnter(Loading)`, and **a filled `Navmesh`
  carries its own `grid_size`/`tile_size` snapshot** so stale snapshots never index against
  the switched atomic. Switching reloads the world like a city switch, except the camera
  stays put. `grid.rs`: `world_to_tile` / `tile_center`. Costs and the chunk scaling —
  **navigation-deep skill**.
- **Viewport** (`camera.rs`) — the piece of the world in frame, as a value: `centre`,
  `half_extent` (margin already applied), `zoom` (world m per logical pixel). `contains`
  — **the edge counts as inside**. Five visibility gates use it and **each keeps its own
  margin** (warmup 1.0, dispatcher/separation `VIEW_MARGIN` 1.2, movepath gizmos 3.0, door
  gizmos 1.5), because each asks a different question — the table is in the
  **navigation-deep skill**. Not Bevy's `Camera::viewport`, which is in pixels.
- **Geo anchor** — `GEO_CENTER_LAT/LON` (Tula, kremlin near frame center). Projection is
  local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW corner → (0,0),
  f64 math, `MAP_SIZE`-sized bbox derived from the center.
- **Z-layers** — constants in `settings.rs`, bottom to top: ground → parks → woods → grass
  → sand → water → waterways → alley casings → alleys → road casings → roads → bridge
  casings → bridges → rails → rail dashes → tram → corpses → portal → buildings (5) →
  units → tree shadows → trees (20). Three live in their own modules:
  `Z_BUILDING_SHADOW` 4.5, `Z_FACADE` 4.9 (`map/buildings/mod.rs`), `Z_WALL` 5.1
  (`map/roads.rs`). Units are y-sorted: `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR`
  (10 − y·0.002). **Invariant: the unit z range must stay above buildings (5) for any
  y ≤ MAP_SIZE.y** — a bigger map once sank northern units under roads.

## App lifecycle

Summary; the mechanism — **world-lifecycle skill** (states and the warmup hold,
`SimBootPlugin`, the load thread, the `WorldStarted` seam, restart slots, the city switch).

- **AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
  (progress, red error + **Retry**). **All world spawning happens in `OnEnter(Playing)`**,
  never in `Startup`.
- **PlayPhase** (sub-state of `Playing`) — `Warmup → Live`. During Warmup the world exists
  but `Time<Virtual>` is **paused** and the loader stays up counting pawns still routing
  *inside the camera view*; typical warmup **~0.15 s**, timeout 10 s. `Live` despawns the
  loader and reveals the game UI (`GameUiRoot`).
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. The navmesh must
  be filled before the population spawns, or humans land in the river.
- **SimBootPlugin** (`loading.rs`) — one world bring-up shared by the game and the replay
  app: the two states, the `WorldInitSet` chain, the warmup pause, the **WorldStarted**
  announcement on entering `Live` (chained before the unpause).
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` doing
  everything that needs no ECS: `Connecting → Downloading → Parsing → BuildingNavmesh →
  Pruning → Done | Failed`, every state a line on the loader screen. **Rule: heavy init
  belongs in this thread, not in `OnEnter(Playing)`** — no frame is drawn inside a schedule.
- **WorldStarted** (`loading.rs`, event) — "the world begins a new run", the single seam
  both lifecycle paths share, fired on entering `PlayPhase::Live` and on every restart. All
  run state (`SimClock` + `TickDebt`, `SimTick` + the frozen `Backend`, `Telemetry`,
  `DemonSpawner`) is reset by observers of it, each in its owning module (`grep
  "On<WorldStarted>"`). **Membership is held from the outside** by
  `a_restart_replays_the_run`, not by hand. Map-derived state (`NorthstarGrid`,
  `PolyNavmesh`) is **not** run state — a restart keeps the map.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns pawns and corpses, fires
  **WorldStarted**, respawns the population; the navmesh persists. Under **Deterministic**
  this replays the previous run tick for tick.
- **RestartPending** (`restart.rs`, resource) — "a restart was ordered", the only way to ask
  for one from anywhere but R (a changed **world seed**, a flipped **Deterministic**).
  Consumed in `PreUpdate` after `InputSystems` — the same slot R uses, because a mass
  despawn may not happen in `Update` (CLAUDE.md). Always `to_portal: true`.
- **City** (`city.rs`, resource, persisted) — `Tula | NewYork | Paris | Berlin | London |
  Tokyo | DevilsLake`, each with its geo center, portal hint and cache slug. `MAP_SIZE` and
  therefore the derived `grid_size()` are shared, so switching city never resizes the
  navmesh. UI — a select at
  bottom centre (`ui/city.rs`).
- **City switch = full world reload** — writing `City` sends the app back to
  `AppState::Loading`; the scene is torn down, the new extract downloaded and parsed, the
  same navmesh refilled, the camera reset. Gated on `in_state(Playing)`: restarting a load
  on top of a running one would put two threads into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city. Every
  world entity must carry it; the rule and the list of spawn sites live in **CLAUDE.md**
  ("World entities"), and `loading.rs::warn_leftover_world_entities` warns when something
  survived.

## OSM map pipeline

Summary; the mechanism — **osm-map skill** (entrance statistics in
`references/entrances.md`, planting and crowns in `references/trees.md`, the tag coverage
audit in `references/osm-coverage.md`, the crown algorithm in `references/tree-algo.md`).

- **Overpass** — the Overpass API, queried once per city with `[out:json]` + `out geom`;
  bbox is `MAP_SIZE` around the `City` geo center. Mirrors in `OVERPASS_URLS` are tried in
  order. **Bump `QUERY_VERSION` in `overpass.rs` whenever the query gains tags**
  (currently 7), or existing caches keep serving extracts that lack them.
- **Cache** — `assets/osm/{slug}_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored);
  the parameters live in the file name, so changing them invalidates it. Written only after
  a successful parse; the second launch never touches the network. `prune_stale_caches()`
  keeps exactly one current file per city.
- **Overpass fixture** (`map/osm/fixture.rs::Overpass`) — a scene given in **map metres**,
  turned into an Overpass response and fed through the real `parse`, so a test states its
  scene in the same numbers it later asserts on. **Add a tag case here, not another
  literal.**
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes, rings open. `AreaKind: Building | Kremlin | Water |
    Park | Wood | Grass | Sand`; **only Wood carries trees**. Buildings carry
    `height: Option<f32>` and `entrances: Vec<Vec2>`.
  - **RoadLine** — centerline + width by highway class (primary 16 → footway 3.5);
    `RoadClass: Street | Alley`; `bridge` / `passage` flags (the navmesh carves by them).
    Underground road is dropped (`is_road_underground`) — a **separate** predicate from
    `is_underground`, because the risk is asymmetric: an extra ribbon is cosmetic, an extra
    deletion is a hole in the navmesh.
  - **RailLine** — `railway=*` centerline; `RailKind: Active | Tram | Disused` *is* the
    drawing style; underground track is dropped. The rail branch of `parse_way` runs before
    `highway` and falls through — a way can be both street and track.
  - **WallLine** — `barrier=city_wall` (the kremlin), 3 m, impassable.
  - **WaterLine** — a *linear* watercourse (`river` 8 m → `ditch` 1.5 m), falling through
    `highway` like rails. `tunnel: bool` marks a **culvert**: not drawn, and the only
    watercourse kind that does **not** block the navmesh.
  - **TreeRow** / **TreeNode** — `natural=tree_row` avenues and single surveyed
    `natural=tree` trees, with optional `spacing`/`radius` from tags.
  - **trees / tree_appears_at** — what the renderer reads; `compose_trees` merges forest +
    avenues of the selected layout, `composed_for` caches which.
- **Building height** (`parse/tags.rs::building_height`) — metres from `height` or
  `building:levels` × 3 m; outside 2–600 m counts as no tag. `None` is normal — every
  consumer owns a default (`DEFAULT_BUILDING_HEIGHT` 15 m). Coverage varies wildly by city
  (NY 97 % … Tokyo 5 %) and is logged on load.
- **Entrances** — real `entrance=*` nodes are attached to building outlines by exact vertex
  lookup; coverage is thin everywhere, so `map/osm/entrances/` **generates** doors for the
  ~98 % of buildings without one. Doors face the street, the count follows building
  *length* at a measured pitch (`ENTRANCE_SPACING` 25 m, floor `ENTRANCE_MIN_SPACING` 12 m),
  walls a neighbour stands against get none, and the result is deterministic per building
  (LCG seeded by its first vertex). **Real doors always win.** The `doors` debug toggle
  draws them.
- **Trees** (`map/osm/planting.rs`) — planted **only inside Wood polygons** plus standalone
  surveyed trees and `tree_row` avenues; deterministic LCGs seeded by geometry. **Planting
  runs once at the density ceiling**; the density slider shows a monotone *prefix*
  (`tree_appears_at`), never a replant. The ceiling (`TREE_DENSITY_MAX` 6.5×) is derived
  from `TREE_MIN_SPACING` (6 m) saturation, not chosen. Health check — the `osm parse: N
  trees planted of M asked …` log line. **Crown geometry** is all in `CrownParams`
  (`map/trees/crown.rs`), built by `crown_variant`; **the city is drawn with
  `CrownParams::default()`**, whose `seed` picks the **crown set** (the city: **set 5**) —
  a whole `TREE_VARIANTS` of silhouettes at once, since **a single variant cannot be
  re-rolled**. Every crown side by side, knobs live: `cargo run --example tree_gallery`.
- **Footprint bands** (`map/footprint.rs`) — the strips linear geometry occupies on the
  ground, as **(centerline, width, role)** values (`deck_band` / `curb_bands` /
  `passage_band` / `channel_band` / `wall.band()`) plus the width policy. One construction,
  three consumers: the grid fill rasterizes, the mesh build outlines, the renderer draws its
  own smoothed copy. **A drawn band and a blocking band match by construction, not by
  discipline.**
- **Merged meshes** (`map/meshing.rs`, `map/spawn.rs`, `map/roads.rs`, `map/buildings/`) —
  one merged `Mesh2d` per layer: earcut triangulation, per-vertex colors, one white
  `ColorMaterial`; ~7000 buildings cost a handful of entities. Trees stay individual
  entities; tree and building **shadows** are each one merged mesh. **Ribbon**
  (`push_ribbon`) — constant-width band along a polyline with join/cap knobs. **Junctions
  are not computed** — overlapping `Round` caps in one opaque flat-colored layer are what
  makes them look joined; **keep the road layer opaque**.
- **Style resources** — each is BRP-writable, persisted, and a change rebuilds only its own
  layers from the unchanged `MapData`: **RoadStyle** (join / smoothing / casing — smoothing
  works on a *copy*, since `RoadLine::points`/`width` are load-bearing for navmesh, arches,
  planting and entrances), **BuildingHeightMode**, **TreeStyle**, **TreeRowStyle**,
  **ConiferNoiseStyle**. **`CrownParams` is deliberately not one of them** — a plain
  struct, no BRP, no prefs; only the `tree_gallery` example varies it. **Bridge / rail /
  tram layers** have their own z-slots and primitives (`push_dashes`, `push_ticks`, tram
  zoom LOD).

## Navigation

Summary; the mechanism and the measurements — **navigation-deep skill** (polymesh in
`references/polymesh.md`, separation & slots in `references/crowd.md`).

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * grid_size.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** → buildings block → walls block → **building passages carve back through
  them**. Without bridges the Упа river bisects the map and no cross-river path exists.
- **Bridge curbs are impassable** — the same two bands the renderer draws; on dry spans they
  stop a pawn stepping off the deck sideways.
- **Linear waterways block, unlike rails** — water is crossed by bridge, not waded;
  **culverts do not block at all**. The health check after any change here is the
  **pruned-tile count** in the log: a jump of thousands means a watercourse severed a
  district.
- **A rasterized polyline is a 4-connected chain, by construction** — `set_polyline` walks
  the centerline tile by tile on top of the capsule test, because a thin slanted band
  otherwise degenerates into corner-touching tiles that northstar and `line_of_sight` slip
  through. Pinned by `tests/navigation.rs`.
- **Ordinary roads do not touch the navmesh** — the grid starts all-passable and the fill
  only subtracts; roads enter it solely through the `bridge`/`passage` carves. **Rails do
  not touch it either, deliberately** (pinned): blocking an unbroken cross-city line would
  let `prune_unreachable` amputate half the map.
- **Building passage** (арка) — `tunnel=building_passage` / `covered=…` sets
  `RoadLine::passage`; carved passable **last**, width capped by `PASSAGE_MAX_WIDTH`.
  Without it, arch-only courtyards get sealed by the prune.
- **prune_unreachable** — BFS flood from the portal; unreachable pockets become impassable,
  because an A* to an unreachable target floods the whole region (a 12 000 request backlog
  once "froze" the crowd).
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>`; async tasks read it off-thread. Filled and pruned
  by the map-load thread while the loader is up.
- **PortalPos** (resource) — the actual portal position; `PORTAL_POS` is only a hint,
  `snap_portal_position` spirals to the nearest tile with clearance, between fill and prune.
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable: A* / Dijkstra /
  Fringe / BFS / **HPA*** (28× cheaper than flat A* at ~10 % longer paths) / Theta*.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid`, built
  lazily (~12 s) **only when a northstar algorithm is selected**; until it lands the
  dispatcher falls back to flat A*.
- **Backend / Walkable** (`navigation/backend.rs`) — the active backend as one cheap-clone
  `Send` snapshot, and **the resource the whole simulation reads** (`Res<Backend>`). The two
  modes differ not in type but in **who writes it**: live re-takes it every frame, under
  determinism it is frozen for the run. **It has no `Default` on purpose**, so **every
  system taking `Res<Backend>` must sit in a `SimPipeline` set**. `walkable()` is the
  passability view: `allows`/`nearest_free_point` are backend-strict, `sift_target` /
  `line_of_sight` / `coast_allows` stay deliberately grid-only. **Invariant: outside
  `navigation/` and `ui/`, the names `PolymeshBuild` / `PolymeshDebug` /
  `PathfindingAlgorithm` do not appear** — the door for "run this world on the flat grid" is
  **`navigation::use_flat_grid(&mut World)`**.
- **NavMode** (`navigation/mode.rs`) — which backend is active **right now**, as one value:
  `Grid(Flat | HierarchyPending{wanted} | Hierarchy(g))` and `Mesh(Pending | Ready(b))`,
  computed in exactly one place (`Pathfinder::mode`). A *value*, not a resource, and taken
  fresh by each consumer — the loader gate must see the live situation even in a
  deterministic run, where `Backend` is frozen.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) — requests become
  async tasks with **visibility gating** (peaceful wanderers off-screen or at zoom ≥
  `WANDER_DISPATCH_MAX_ZOOM` wait; **`UrgentPath` always dispatches**) and **priority**
  (urgent first, nearest-to-camera, cap `MAX_PATHFINDING_IN_FLIGHT` 1024).
- **UrgentPath** (`movement/components.rs`) — "this pawn may not wait for the camera". The
  species own it: a demon and the test walker carry it always, a human only while panicking;
  `strip_movement` takes it off a corpse. **Movement asks `Has<UrgentPath>` and names no
  species at all.**
- **Repath on the move** — `to_pathfinding` keeps the current path; a pawn walks the old one
  while the new is computed. `MovableStateMovingTag` means "has a path **or is coasting**".
  **Coasting** — a pawn whose path ran out mid-repath keeps walking `last_direction` over
  passable tiles; on reply, up to `REPATH_TRIM_LIMIT` leading waypoints are trimmed.
- **Rescue** (`movement::rescue_from_impassable`) — a pawn on an impassable tile is moved to
  the nearest passable one. **Trigger is a failed search**, not a clock; what counts as free
  is the *active backend*. A full pass runs only after every completed polymesh build.
- **find_passable_tile_near** — target tile or its 8 neighbors only; callers tolerate `None`.
- **Poly navmesh** (`navigation/polymesh/`) — a polygonal polyanya mesh from the same vector
  sources the grid rasterizes, ring **holes subtracted** as the grid subtracts them; **the
  default pathfinding backend**, the grid serving as fallback while it builds (~5–20 s,
  async, cancellable). polyanya is **vendored** (`vendor/polyanya`, edits marked `QWE:`);
  `bounded_path` is the only door to it, an exhausted budget **panics** by design, and so
  does a task older than `PATHFINDING_TASK_HANG_SECS`.
- **Polygonal routing** (`find_path_polymesh`) — paths are **world-space polylines**
  (`VecDeque<Vec2>`, start point included); **the goal stays a tile** (identity for
  stale-answer filtering and arrival). A missed goal is `PathfindingError`, not a fallback —
  watch `answers: N/frame, X % failed` on the speed panel. Endpoint tolerance is 1 m, which
  also sets the agent-radius slider ceiling (0.6 m). Coasting and lunge `line_of_sight` stay
  grid tests.
- **tiny_city / parity tests** (`map/osm/fixture.rs`, `navigation/parity_tests.rs`) — the
  shared `MapData` fixture from which **both fills** are built and must agree probe by probe:
  the executable form of "one rule for both fills". Run after touching either fill; **new
  fill rules add a zone here, not a hand-built `MapData`.**
- **pathfinding_bench** (`examples/bench/pathfinding_bench.rs`) — offline comparison of all
  six algorithms over a seeded wander-shaped task list. Run it after touching `successors`,
  costs, or the navmesh fill.

## Determinism

Summary; the mechanism — **determinism skill** (seed derivation, the decision stream, the
`SimPipeline` sets, the deterministic dispatcher, the replay yards and what they pin).

- **World seed** (`rng.rs::WorldSeed`, persisted, panel row *Seed*) — the one number every
  simulation draw descends from. It governs the **simulation**, not the map: trees and
  entrances are seeded by their own coordinates and are reproducible without it. Capped at
  `i64::MAX` (`MAX_SEED`).
- **Seed derivation** — `seed_for(world_seed, domain, key)`, two rounds of splitmix64.
  `RngDomain: Population | Human | Demon`. **Nothing stores live RNG state**, so a restart
  has no RNG to reset — every stream is re-derived.
- **Placement stream** (`rng.rs::stream`, `RngDomain::Population`) — the one shared
  generator, held by `spawn_population` across every spawn. Legal precisely because its
  consumer is a fixed `0..count` loop rather than a query traversal, and everything
  personal (colour, `Pace`, heading) still comes from the pawn's own decision stream.
- **Decision stream** (`rng.rs::WanderIndex::next`, humans *and* demons) — a `SimRng` is
  built **per decision** and dies with it, seeded from `(PawnId, decision number)`: the
  pawn's observable identity plus which choice this is, never the history of a stream. So
  draws do not depend on query iteration order, on neighbours, or on how many draws the
  previous decision consumed. **Position is deliberately not an input** — it would close
  every pawn's trajectory into a cycle. It also advances on **transitions**, not only on
  ladder rungs — a kill costs the demon one decision number, rolled for the devour pause in
  the kill observer (the sites are listed in the determinism skill).
- **Species** (`rng.rs`, component) — the other half of a pawn's personal number. `PawnId` is
  unique only *within* a species, so every mixed ordering (`movement::order::pawn_key`) puts
  species first. **Variant order is part of the replay contract** — `Demon` is declared
  first.
- **PawnId** (`rng.rs`) — a pawn's spawn ordinal within its species and run. Used wherever a
  stable "personal number" is needed: the RNG seed key, the flee-fan angle, the separation
  axis, the dispatcher tiebreak, the spatial-grid tiebreak. **Never `Entity`** — entity
  indices are recycled in a different order after a restart.
- **SimTick** (`determinism.rs`) — the step counter, incremented at the head of the
  `FixedUpdate` chain. **The unit of replay**: world state is a function of `(seed,
  settings, SimTick)`. Not `SimClock`, which counts virtual seconds and loses whatever
  `max_delta` discarded. **Compare states by tick, never by wall clock.**
- **Deterministic** (`determinism.rs::Determinism`, panel toggle) — gates *scheduling*, not
  the dice. On: *human* target picking moves to `FixedUpdate` (the demons' already runs there
  in both modes), answers land on a fixed tick, the dispatcher stops looking at the camera,
  the backend is frozen, separation is off. A run is
  deterministic or not from tick 0, so flipping it (like changing the seed) orders a restart
  via `RestartPending`, `to_portal: true`.
- **SimPipeline** (`determinism/mod.rs`) — the toggle in the schedule: **three** system sets
  — `Live`, `Deterministic`, `BothModes` — gated once for every schedule they appear in. A
  system declares its branch with `.in_set(..)` and **never reads the mode**. **The sets
  also carry the world gate** (`in_world`), so "no set" reads as "lives outside the world",
  not "both modes".
- **Retire tick** (`RetireAt`, `PATHFINDING_RETIRE_TICKS = 8`) — a request issued on tick `T`
  is applied on exactly `T + 8`, waiting on the search if it has not finished. That wait
  removes "when did the OS get around to it" from the simulation. **The constant must not
  scale with `SimSpeed`.**
- **Dispatch rate** (`PATHFINDING_WANDER_UNITS_PER_TICK` 128 / `_URGENT_` 64) — how much
  leaves the queue each tick, measured in *predicted search cost* (an integer), not in
  requests. **Never reuse `MAX_*_IN_FLIGHT` here** — those cap concurrent searches behind
  the visibility gate. **A long queue is the normal state of this mode**; at 30× it settles
  around 2–5×.
- **RequestedAt** — the tick a request was filed; the deterministic dispatcher's FIFO key is
  `(requested_at, species, pawn_id)`, all integers. **The camera does not appear in it at
  all.**
- **Frozen backend** — in this mode `Backend` is written once, on **WorldStarted**, and
  never refreshed; warmup waits for the wanted backend instead (~11–14 s on first entry into
  a city on HPA, deliberately; restarts do not pay it). **No pawn warmup in this mode.**
- **NeedsWanderTarget** (`movement/components.rs`) — marker held exactly on `Idle` and
  `PathfindingError`; without it each `FixedUpdate` run would scan all 17 000 wanderers.
- **Replay check** (`determinism/replay.rs`, `tests/determinism.rs`,
  `examples/acceptance/determinism_replay.rs`) — three claims: the same seed replays tick for
  tick, a ragged frame rate does not change the run, a different seed does. Two conditions
  make it bite, both learned the hard way: **the world must actually move** and **the scene
  must be crowded**. `a_restart_replays_the_run` runs its second half in the *same* `App` —
  that is what catches state outliving the reset.
- **Frame rate does not matter.** `Time<Fixed>`'s step is constant regardless of fps and of
  `SimSpeed`; a slow machine replays the same run more slowly.
- **The replay contract** — 1:1 holds only while `DemonStyle` / `HumanStyle` /
  `SeparationStyle` / the algorithm / the navtile size are left alone mid-run. Sliders are
  simulation input. Not enforced by code. **Not claimed**: float reproducibility across
  machines, or replaying a run made with the toggle *off*.

## Simulation

Summary; species behaviour — **species-behavior skill**; the crowd (separation, slots) —
**navigation-deep skill**, `references/crowd.md`.

- **SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`): `SpatialRebuild →
  DemonBehavior → HumanBehavior`. **Demons act before humans so a kill lands before
  `escape`** — a human is never counted both killed and escaped in one tick.
- **SimPosition / PreviousSimPosition** — simulation-space positions; `Transform` is
  interpolated between them in `RunFixedMainLoop`. Systems mutate `SimPosition`, **never
  `Transform.translation.xy`**. `snapshot_previous_sim_positions` runs **before**
  `SimSet::SpatialRebuild` (as does the demon spawner) and `move_moving_entities` **after**
  `SimSet::HumanBehavior`, because behavior may move `SimPosition` itself (the demon lunge).
- **Movable** — `{speed, path: VecDeque<IVec2>, state}` with `MovableState: Idle |
  Pathfinding(goal) | Moving(goal) | PathfindingError`. `to_pathfinding` queues the search
  and keeps the current path; **`to_idle` is the only transition that stops movement** —
  and it stops it whole: path, `MovableStateMovingTag`, and a `PathfindingRequest` that has
  not been dispatched yet, with its `RequestedAt`.
- **SpatialGrid<T>** — uniform grid per marker type (`Demon`, `Human`), 60 m cells (≥ the
  largest search radius, so a radius query is a 3×3 cell walk). Cells hold **entities
  only** — positions are read live through the `pos_of` closure. **A tie in distance is
  broken by `PawnId`, not by traversal order** (`order_of`, called only on exact equality).
  **The human grid is incremental** (observers + `SpatialGrid::moved` across cell
  boundaries), **the demon grid is rebuilt** each tick.
- **Decision ladder** (`human/decide.rs`, `demon/decide.rs`) — a species' rules live in a
  pure `decide(&…Sense, …) -> …Action`: plain values in, one enum variant out, no
  `Commands`, no queries, tested without an `App`. `behavior.rs` only applies the answer, so
  the *order of the rungs* is readable in one place. Deliberately outside: **expensive
  senses** (asked lazily, at most once), **anything touching the world** (its *terms* still
  come out of `decide`), and **the RNG rolls** (the decision stream must advance on exactly
  the ticks it did before).
- **Wander skeleton** (`movement/wander.rs`) — the *order* of one target-picking step,
  shared by both species: `ready_to_pick` → the species' policy → `point_in_cone` /
  `clamp_to_map` → `request_wander_path` → `heading_towards`. Only **where** a pawn wants to
  go is per-species. It deliberately does **not** open the `SimRng`.
- **PopulationSize** (`human/components.rs`, resource) — how many humans
  `spawn_population` settles. **No knob, and not a `settings.rs` value**: its `Default`
  *is* `HUMAN_COUNT` (20 000) and the game never changes it; the resource exists so a
  headless scene can run a small crowd. Read at two sites that must stay in step —
  `human::spawn_humans` under `WorldInitSet::Spawn` and `restart::on_restart`, so a
  restart respawns the same number. Today the only non-default user is the replay yard
  (`determinism::replay::replay_app`'s `population` argument; `tests/determinism.rs`
  runs 64). **It is the right-hand side of the telemetry invariant** — see Telemetry.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s, rolled per
  arrival and drawn by only `HUMAN_WANDER_PAUSE_SHARE` 20 % of them — the rest pick the
  next target the same frame; 80 % a building errand anywhere in the city — the real
  pathfinding load — and 20 % a 20–40 m stroll) ⇄ **Flee** (a demon within
  `HUMAN_PANIC_RADIUS` 60 m; the first repath on the panic tick itself, then every
  0.7–1.2 s, stepping 40–60 m away), calm-down at ×1.5
  radius hysteresis. The Wander → Flee
  check is **inverted** — demons collect neighbours from the human grid, so its cost tracks
  the crowd near demons, not the city population. **Flee fan** — a non-chased fleeing human
  rotates its away-vector by a deterministic per-entity angle (±0.6 rad) so crowds spread;
  chased humans flee straight. **Escape** — a fleeing human within `ESCAPE_MARGIN` of the
  border despawns, `telemetry.escaped += 1`.
- **WanderHeading** — the direction a human is walking, kept between walks; every next
  target is picked inside a `WANDER_CONE` (±60°) around it. Without it pawns wobbled in
  place. `flee` rewrites it to the away-vector on every repath.
- **PanicRecoil** — a unit vector *toward* the demon, written on **every flee repath** and
  **never queried live** (`pick_wander_targets` must stay off the demon grid). While it is
  on, the next target must be an errand outside `RECOIL_CONE` (±45°) and farther than
  `RECOIL_MIN_ERRAND` (90 m); nothing acceptable → re-roll next frame, **never a stroll** —
  except on a map with no buildings at all, where the fallback stroll is filtered by the
  cone alone, with no distance floor.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near* stroll,
  never an errand. Measured: errands first routed the on-screen pawns in 3.9 s, strolls
  first in 0.15 s. `PanicRecoil` overrides it.
- **Pace** — a human's personal speed multiplier, rolled once at spawn and stored
  **normalized** (−1…+1), applied to *both* bases through `Pace::speed`. Normalized storage
  is what lets the **Speed spread** slider widen the ordering the crowd already rolled
  instead of re-dealing it. Ceiling 35 % is derived: above it the fastest humans outrun the
  slowest demon setting.
- **CorpseTag** — a killed human: behavior/movement components removed, dark lying sprite at
  `Z_CORPSE`, not in the human spatial grid. The transition is **`human::to_corpse`**, one
  entry point; the kill observer in `demon/` only reports that it happened. It calls
  **`movement::strip_movement`**, so `Movable`'s `#[require]` stays the single record of
  what a movable entity drags along.
- **Demon** states (`demon/behavior.rs`): **Wander** (a point in the `DEMON_WANDER_CONE`
  (1.3 rad half-angle) around the away-from-portal vector, `DEMON_WANDER_RANGE` 40–120 m;
  no `WanderPause` analogue and no stored `WanderHeading` — the next target is picked the
  same frame the demon goes idle) → **Chase** (nearest human within `DEMON_AGGRO_RADIUS`
  45 m with a free claim slot; give-up at ×1.5 radius hysteresis, 67.5 m) → **Devour** →
  Wander. **Chase claims** —
  **max 2 chasers per target**
  (`ChaseClaims`, `demon/claims.rs`), a value rebuilt each tick; there is no standing claim
  between ticks, only **GaveUp** releases a slot, and a switch *transfers* one. Repath
  throttle 0.4 s (`DEMON_CHASE_REPATH`), and on that tick the demon may **switch** target — **a rung of the ladder,
  not a tail after it**. **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with
  `line_of_sight`, the demon drops its path and steps `SimPosition` straight at the target;
  without it a chase never converts. Kill at `KILL_DISTANCE` triggers
  `DemonCaughtHumanEvent`. **Devour** — pause 1.5–2 s with a sine pulse ×1 → ×1.5; the pause
  is rolled in the kill observer from the demon's **decision stream**, so a kill advances its
  `WanderIndex`.
- **DEMON_SPEED** — one base for every state, `HUMAN_FLEE_SPEED × 1.35`. **Do not
  reintroduce per-state demon speeds**: the only multipliers are the two user ones,
  `DemonStyle::speed` and `DemonStyle::lunge`.
- **DemonSpawner** — initial burst at the portal rim, then one demon per interval up to the
  cap; cap and interval live in **`DemonStyle`**, `DEMON_CAP` / `DEMON_SPAWN_INTERVAL` are
  only its `Default`. Lowering the cap never despawns demons already out. **The spawner runs
  only in `PlayPhase::Live`, and that is an invariant**: it hands out `PawnId`s from a
  counter `WorldStarted` resets, so a burst fired before the announcement deals the same
  numbers twice. Matching precondition: **no demon may be alive when a run starts**. It runs
  **before `SimSet::SpatialRebuild`**, so a demon enters the demon grid and acts on the tick
  it is born on.
- **Separation** (`movement/separation/`, Nav tab, persisted) — soft pairwise
  anti-overlap, **on-screen only, cosmetic by charter**: pawns keep their body radii
  apart (a resting human pair at 1.8 m, against a 1.0 m `HUMAN_SIZE`). The radius is a
  knob — **`HumanStyle::body_radius`**, and `HUMAN_BODY_RADIUS` 0.9 m is only its
  `Default`; the demon's is never a separate knob, always `2 ×` it
  (`separation::demon_radius`, `DEMON_RADIUS_RATIO`, default `DEMON_BODY_RADIUS` 1.8 m).
  Runs **only on the
  polymesh backend and never under determinism** (grid waypoints re-collapse any push), once
  per rendered frame, below `SEPARATION_MAX_ZOOM`. Lunging demons are exempt; the one
  deliberate breach of "cosmetic" is `SeparationHolds` + rest-distance arrival forgiveness.
  Measured on demand by `examples/demos/crowd_demo/`.
- **Destination slot** (`movement/destination.rs`) — the reservation that stops two pawns
  from being aimed at the same point: a `k × k` navtile block per claimed goal, goal
  strictly the block's **centre** tile; a taken slot ring-searches outward. Claims are
  released on next target selection, despawn, or corpse strip — **not on arrival** (a
  standing pawn *is* the occupancy). **Chase and flee are excluded** by design. Runs in
  **both** modes — it is simulation, not cosmetics.
- **Telemetry** — `{killed, escaped}`, BRP-readable; `killed` is the **Souls reaped** HUD
  counter (`ui/stats.rs`), *not* a row of the Sim tab's **World** section — that section
  holds the seed and the determinism row. Invariant (check paused):
  `killed + escaped + alive == PopulationSize` — the number the spawn actually read, not
  the constant: in the game that is the default `HUMAN_COUNT`, in a replay run whatever
  `replay_app` was given. At high sim speed BRP reads are skewed — pause before asserting.

## UI & debug

Summary; panel internals — **ui-panels skill**; the speed regulator — **sim-speed skill**.

- **UI input never reaches the world** — the panel sits over the map, so a click, drag or
  scroll that lands on it must not also drive the camera or anything in the world.
  `camera.rs::drag_pan` decides *in the press frame* whether the gesture belongs to the UI
  (`pointer_over_ui` over `HoverMap`) and holds that verdict until release; `zoom_to_cursor`
  runs under `not(hovering_ui)`. The rule and the idiom — **CLAUDE.md**.
- **Two layers.** **HUD**, always on screen: run counters (`ui/stats.rs`), telemetry +
  **Speed button** (`ui/speed.rs`), the **City** select bottom centre (`ui/city.rs`), hotkey
  help (`ui/hotkeys.rs`), the agent **BRP** badge (`ui/brp.rs`). And **one settings panel**
  with four tabs (`ui/shell.rs`): **Map**, **Nav**, **Sim**, **Debug**. `Tab` (or the `-`/`+`
  button, or a click on the open tab) collapses it to the tab strip; the open tab and the
  collapsed flag are a persisted settings group (`UiShellState`). **Section order inside a
  tab is `SectionSlot`'s declaration order** (`sort_sections`), because the sections are
  spawned by eight systems in eight plugins.
- **Knob** (`ui/knob.rs`) — a panel row **bound to one field of one resource**, in two
  shapes: `spawn_knob` (slider) and `spawn_cycle_row` (button that cycles a value).
  `app.add_knobs::<R>()` registers the drag observer and the label/thumb sync **once per
  resource**, however many knobs it has. **Use it for any panel row driven by a resource**;
  the Nav tab's rows are the deliberate exception, since their text is computed
  from several resources at once.
- **Widgets & theme** (`ui/theme.rs`) — the controls are first-party **`bevy_feathers`**,
  themed by `create_dark_theme()` with colour overrides only. The plaques are **translucent**
  over the map and the text is **brighter** than feathers': the panel keeps its legibility
  with type, not with an opaque fill. `PanelWidgetsPlugin` installs `FeathersCorePlugin` —
  **not** the `FeathersPlugins` group, since `TabNavigationPlugin` would let Tab+Space both
  press a button and pause the sim. Everything that is not a widget is coloured by **design
  tokens**; **no hand-written UI colours are left**. A **value row rests transparent**;
  **"active" is `ButtonVariant::Primary`**. Every container node comes from `ui_node` /
  `ui_row` / `ui_column`, which carry the `ThemedText` marker — every container *between a
  font source and a label*, that is; a root is the source itself (`InheritableFont` requires
  the marker) and stays a bare `Node`.
- **Shared kits** — `ui/slider.rs` (the layer under the knob kit) and `ui/rows.rs`
  (`spawn_value_row`; a row whose click does nothing gets `bevy::ui::InteractionDisabled`).
  **Use these, don't hand-roll a panel row.**
- **Debug tab** (`ui/debug/`) — the grid / doors / movepath / noise overlay rows, the
  `Camera start` and `Navtile` cyclers (global settings, deliberately not under a backend
  section) and **`reset`** (`prefs::ResetSettings`). The navmesh overlay is **one merged
  mesh** (per-tile entities once cost 330 k); the noise overlay is one CPU-built texture
  sprite.
- **Camera start view** (`camera.rs`) — `CameraPositionMode` (`reset | save`, persisted):
  where the camera stands when the world comes up. **RR** (double R within 0.5 s) and
  `RestartEvent { to_portal: true }` go to the portal at `START_ZOOM` regardless of mode. A
  city switch always resets to the new portal.
- **Sim speed** (`sim_time.rs`) — Space pauses; `=`/`-` walk `SPEED_LADDER` (1-2-5-10-20-30);
  **`MAX_SIM_SPEED` 30× is a deliberate product cap**. **SimSpeed** `{requested, pipeline,
  affordable, effective, actual}` — a regulator throttles `effective` to what the machine
  carries, and `guard_frame_budget` hard-stops a frame's fixed loop at `SIM_FRAME_BUDGET_MS`,
  booking stripped ticks into **TickDebt**. **`actual` is the only honest reading.** Set speed
  over BRP with `res set SimSpeed .requested N`, never `brp speed`. The regulator's memory is
  reset by **WorldStarted**. **SimClock** — virtual seconds of the current world, zeroed by
  the same observer; compare states by `SimTick`, not by it.
- **Tunable resource** (`prefs.rs`) — a resource the user retunes from a panel, a hotkey or
  a BRP write. Two things are asked of one: **`app.track_pref::<T>()`** persists it (a
  `bevy::settings::SettingsGroup` in `settings.toml`), and the run condition
  **`retuned::<T>`** (`changed && !added`) gates whatever rebuilds on it. **Registration
  sits with the resource's own plugin**, next to its `init_resource`; `PrefsPlugin` is
  registered **last**. **`ResetSettings`** puts every group back to its `Default`, found
  through the type registry, **never a list**, so a new tunable is covered the day it is
  declared.
- **dev.rs** — `TakeScreenshotEvent` (BRP-triggerable) → `screenshot.png` (gitignored);
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
- **BRP** — `RemoteHttpPlugin` on port 15702; drive it via the `live-app` skill's `brp`
  script only.

## Naming conventions worth preserving

- **`*Tag`** — marker/state components (`HumanFleeTag`, `DemonDevourTag`, `CorpseTag`).
  **`*Plugin`** — one per feature module. **`on_*`** — event/observer handlers.
- **Пешки** — the user's word for humans/units in feedback; maps to `Human`.
- **Behavior module** — per-species state machine lives in `behavior.rs`
  (`demon/behavior.rs`, `human/behavior.rs`), separate from `systems.rs` (spawning,
  wander targets).
- **Hysteresis** — every enter/exit radius pair uses `RADIUS_HYSTERESIS = 1.5` on exit;
  keep new radii consistent with this pattern.
- **macOS occlusion throttling** — a fully covered window parks the main thread; fps and
  BRP timings are only meaningful with the window visible. Not a perf bug.

## Cross-references

- All tuning constants: `src/settings.rs` (sizes, speeds, radii, spawn rates, z-layers,
  geo anchor). Not there: a number that *is* a rule of a decision ladder rather than a knob
  over it stays beside its `decide.rs` — `MAX_CHASERS_PER_TARGET` and the ×1.5/×0.7 switch
  factors in `demon/decide.rs`, `FLEE_STEP`/`FLEE_SPREAD`/`ESCAPE_MARGIN` in
  `human/decide.rs`. A constant both species declare moves to `settings.rs`
  (`WANDER_MAP_MARGIN`). Detail — **species-behavior skill**.
- OSM pipeline: `src/map/osm/{overpass,download,parse,model}.rs`; rendering:
  `src/map/{meshing,spawn}.rs`. Detail — **osm-map skill** (its `references/` also carry
  the tag coverage audit and the crown-algorithm write-up).
- Navigation: `src/navigation/{navmesh,astar,northstar,polymesh}`; movement/interpolation:
  `src/movement/`. Detail — **navigation-deep skill** (crowd and slots in
  `references/crowd.md`).
- World bring-up, restart, city switch: `src/{loading,restart,city}.rs`,
  `src/map/osm/download.rs`. Detail — **world-lifecycle skill**.
- Determinism and replay: `src/rng.rs`, `src/determinism/`. Detail — **determinism skill**.
- Species behaviour: `src/human/`, `src/demon/`, `src/movement/wander.rs`, `src/spatial.rs`.
  Detail — **species-behavior skill**.
- Speed & regulator: `src/sim_time.rs`. Detail — **sim-speed skill**.
- UI: `src/ui/`, camera: `src/camera.rs`. Detail — **ui-panels skill**.
- Tests: `tests/navigation.rs` (synthetic navmesh + hand-built `MapData`),
  `tests/spatial.rs`, `tests/movement.rs`, `tests/determinism.rs`, unit tests inside
  `map/osm/*` and `map/meshing.rs`.
