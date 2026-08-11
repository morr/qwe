# CONTEXT

Domain glossary for QWE. Use these terms verbatim in commit messages, hypotheses, test
names, and code identifiers. If a concept you need isn't here, that's a signal — either
you're inventing language the project doesn't use (reconsider) or the file has a real gap
(update it in the same change that introduces the concept).

**What belongs here, and how much.** This file holds *terms and invariants*; the
*mechanisms and their justifications* live in four detail skills —
`.claude/skills/{osm-map, navigation-deep, sim-speed, ui-panels}` — loaded on demand
(see CLAUDE.md, "Skills"). Budget per entry: ~10–15 lines — what the term is, the
invariant(s) that hold, where it lives, at most a one-line why. A measurement's
*conclusion* stays here ("25 m is the measured median pitch"); the methodology, the
tables and the derivations go to the skill the entry points at. A "this was tried and
was wrong because…" guardrail is one line, not a paragraph. An entry that wants a table
or a derivation is telling you it belongs in a skill. When a change touches a concept,
update the summary here and the detail in the skill **in the same change** — a stale
glossary is worse than none.

## Project shape

**QWE** is a 2D real-time simulation prototype: a **demon invasion of the Tula city
center**. The map is generated from real OpenStreetMap data at first launch. 20 000
humans wander the streets; demons pour out of a portal, chase and devour them; humans
panic and flee off-map. Built on **Bevy 0.19 ECS** — one plugin per feature, registered
in `main.rs`.

## Coordinates & units

- World units are **meters**. Origin — **south-west corner** of the map, y grows north.
  All world coordinates are positive. `MAP_SIZE = 5600 × 3700` m.
- **Navtile** — navigation grid cell, **2 m by default, runtime-switchable to 1 m** via
  the `navtile:` cycler in the debug panel (`NavtileBase` in `settings.rs`, persisted in
  prefs; changing it reloads the world like a city switch, except the camera stays where
  it was — same city, same spot under inspection). The live value is a
  process-global atomic read by `settings::navtile_size()` — background threads (navmesh
  fill, entrance generation) have no ECS access; it is written only in
  `OnEnter(Loading)` before the load thread starts. Grid size is derived as
  `MAP_SIZE / navtile_size()` (2800 × 1850 tiles at 2 m); a filled `Navmesh` carries its
  own `grid_size`/`tile_size` snapshot, so stale snapshots (a cancelled northstar build)
  never index against the switched atomic. The northstar chunk scales to stay 50 world
  meters (25 tiles at 2 m, 50 at 1 m) — with the tile-25 chunk a 1 m build explodes from
  ~14 s to ~140 s. Cost of 1 m: northstar build ~14 s vs ~11 s, HPA* ×1.7 CPU,
  +1.6 GB RSS. `grid.rs`: `world_to_tile` / `tile_center`.
- **Geo anchor** — `GEO_CENTER_LAT/LON` (Tula, kremlin near frame center). Projection is
  local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW corner → (0,0),
  f64 math, `MAP_SIZE`-sized bbox derived from the center.
- **Z-layers** — constants in `settings.rs`: ground 0 → parks 0.5 → woods 0.55 → grass
  0.6 → sand 0.7 → water 1 → waterways 1.05 → alley casings 1.4 → alleys 1.5
  → road casings 1.9 → roads 2
  → bridge casings 2.1 → bridges 2.2
  → rails 2.4 → rail dashes 2.5 → tram 2.6 → corpses 3 → portal 4 → buildings 5 → units → tree
  shadows 19 → trees 20. Three more
  live in their own modules: `Z_BUILDING_SHADOW` 4.5 and `Z_FACADE` 4.9
  (`map/buildings/mod.rs`), `Z_WALL` 5.1 (`map/roads.rs`). Units are y-sorted:
  `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR` (10 − y·0.002). **Invariant: the unit z
  range must stay above buildings (5) for any y ≤ MAP_SIZE.y** — a bigger map once sank
  northern units under roads.

## App lifecycle

- **AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
  (progress text, red error + **Retry** button on failure). All world spawning happens in
  `OnEnter(Playing)`.
- **PlayPhase** (sub-state of `Playing`) — `Warmup → Live`. During **Warmup** the world
  exists but `Time<Virtual>` is **paused** and the loader screen stays up reading
  "Routing pawns... N left"; `poll_warmup` counts pawns *inside the camera view* that
  still hold a `PathfindingRequest`/`PathfindingTask` and flips to `Live` when none are
  left (or after `WARMUP_TIMEOUT` = 10 s, logged as a warning). It counts only what the
  dispatcher will actually serve (`wanderers_dispatched_at_zoom`, the same cutoff
  `dispatch_pathfinding_requests` uses); `WARMUP_GRACE` = 0.5 s keeps "no requests yet"
  from meaning "done". Reason for the hold: all 20 000 humans queue a path in the same
  frame, and without it the visible ones stood still for the first seconds. Typical
  warmup **~0.15 s** — see `HumanFirstWanderTag`. `Live` is what despawns the loader and
  reveals the game UI (`GameUiRoot`).
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. Navmesh must
  be filled before population spawns, or humans land in the river.
- **SimBootPlugin** (`loading.rs`) — how a world is brought up, one implementation for
  the game and for the replay app: the two states, the `WorldInitSet` chain, the warmup
  pause, and the **WorldStarted** announcement on entering `Live` (chained before the
  unpause). The pause lives here rather than in `SimTimePlugin` even though it drives
  `Time<Virtual>`: it is a property of the phase, not of the speed regulator — and the
  replay app cannot take the regulator (it measures wall clock), so before the move it
  ticked during warmup while the game did not, and had to announce the world start by
  hand to stay ahead of that tick. Unpausing in `StateTransition` takes effect from the
  **next** frame: this frame's `Time<Virtual>` was already advanced in `First`.
  Pinned by `loading.rs`'s own tests.
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` that
  prepares everything not needing ECS: `Connecting{attempt} →
  Downloading{bytes,total,bytes_per_sec} → Parsing → BuildingNavmesh → Pruning →
  Done(LoadedWorld{map, portal}) | Failed(msg)`, polled via `Arc<Mutex<_>>` by
  `poll_job`, every state a line on the loader screen. `total` is `None` in practice
  (chunked answers, gzip strips `content-length`), so the screen shows MB + rate.
  `Connecting` is mostly **Overpass computing the query** server-side (measured: 62 s
  before the first byte on Paris) — the screen says "Waiting for Overpass" and ticks
  seconds off `Time<Real>`; minutes here are normal, not a hang. The thread fills the
  navmesh through the `ArcNavmesh` handle and returns the snapped portal position.
  **Rule: heavy init belongs in this thread, not in `OnEnter(Playing)`** — no frame is
  drawn inside a schedule, so work there freezes the loader on its last message.
- **WorldStarted** (`loading.rs`, event) — "the world begins a new run", the single seam
  both lifecycle paths share. Fired from exactly two places: entering `PlayPhase::Live`
  (first launch, city switch) and every restart (`on_restart` — it passes through no
  state, so `OnEnter` never refires for it). All run state — `SimClock` + `TickDebt`,
  `SimTick` + the frozen `Backend`, `Telemetry`, `DemonSpawner` — is reset by observers of
  this event, each living in its owning module; the full list is
  `grep "On<WorldStarted>"`. Map-derived state (`NorthstarGrid`, `PolyNavmesh`) is *not*
  run state and is cleared by the city switch alone — a restart keeps the map, and with
  it the 12 s northstar hierarchy.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns humans/corpses/demons/walkers,
  fires **WorldStarted**, respawns population. The navmesh persists — it is filled once
  per city. Under **Deterministic** (see "Determinism") this replays the previous run
  tick for tick.
- **RestartPending** (`restart.rs`, resource) — "a restart was ordered". The only way to
  ask for one from anywhere but the R key: changing the **world seed** or flipping
  **Deterministic**, whether from the panel or over BRP. `trigger_pending_restart`
  consumes it in `PreUpdate` after `InputSystems`, the same slot the R key uses and for
  the same reason — `on_restart` tears the scene down inside an observer, so triggering it
  from `Update` would kill entities that sibling systems have already queued commands for
  (see CLAUDE.md, "Where a mass despawn may happen"). It always fires
  `RestartEvent { to_portal: true }`: a changed world setting means a *different* world,
  and leaving the camera where it was makes the restart invisible.
- **City** (`city.rs`, resource, remembered by `prefs.rs`) — which city the map is built
  from: `Tula | NewYork | Paris | Berlin | London | Tokyo`. Each carries its **geo center**
  (bbox center of the Overpass extract), its **portal hint** and its **cache slug**;
  `MAP_SIZE` and therefore `GRID_SIZE` are shared, so switching city never resizes the
  navmesh. Panel — bottom centre (`ui/city.rs`), the current city's button is highlighted.
- **City switch = full world reload.** Writing `City` (button or BRP) sends the app back
  to `AppState::Loading`: leaving `Playing` despawns the scene, the load thread downloads
  / re-parses the new extract, refills the same navmesh (`fill_from_mapdata` resets it
  first), re-snaps the portal, and `OnEnter(Playing)` rebuilds map and population and
  resets the camera (`camera.rs::place_camera_on_world_ready`). `NorthstarGrid`,
  `PolyNavmesh` and `WarmupProgress` are reset on the way; run state waits for
  **WorldStarted** on the new world's `Live` entry. The switch is gated on
  `in_state(Playing)` — restarting a load on top of a running one would put two threads
  into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city.
  Every world entity must carry it; the list of spawn sites and the rule live in
  `CLAUDE.md` ("World entities"), and `loading.rs::warn_leftover_world_entities` warns on
  every entry into `Loading` if something survived.

## OSM map pipeline

Summary; mechanics, measurements and rendering detail — **osm-map skill**
(entrance statistics in its `references/entrances.md`, planting and crowns in
`references/trees.md`, tag coverage audit in `references/osm-coverage.md`, crown
algorithm in `references/tree-algo.md`).

- **Overpass** — the Overpass API, queried once per city with `[out:json]` + `out geom`;
  bbox is `MAP_SIZE` around the `City` geo center. Mirrors in `OVERPASS_URLS`
  (`download.rs`) are tried in order; a mirror answer must start with `{`. What is
  queried and what reaches the map — the osm-map skill's `references/osm-coverage.md`.
  **Bump `QUERY_VERSION` in `overpass.rs`
  whenever the query gains tags** (currently 7), or existing caches keep serving
  extracts that lack them.
- **Cache** — `assets/osm/{slug}_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored);
  parameters live in the file name, so changing them invalidates it. Written only after
  successful parse; second launch never touches the network. `prune_stale_caches()`
  keeps exactly one current file per city.
- **Overpass fixture** (`map/osm/fixture.rs::Overpass`) — a scene given in **map metres**
  (`way` / `area` / `node` / `relation` + tags), turned into an Overpass response and fed
  through the real `parse`. Geometry is unprojected via `GeoBounds::unproject`, so a test
  states its scene in the same numbers it later asserts on, and the response format lives
  in one place instead of a hand-escaped `format!` per test. Parse tests go through the
  JSON text on purpose — deserialization is part of what they cover. Add a tag case here,
  not another literal.
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes, rings open (no repeated last point).
    `AreaKind: Building | Kremlin | Water | Park | Wood | Grass | Sand`; **only Wood
    carries trees**. Buildings carry `height: Option<f32>` and `entrances: Vec<Vec2>`.
  - **RoadLine** — centerline + width by highway class (primary 16 → footway 3.5);
    `RoadClass: Street | Alley`; `bridge` / `passage` flags (the navmesh carves by them).
  - **RailLine** — `railway=*` centerline; `RailKind: Active | Tram | Disused` *is* the
    drawing style. `rail_class` is a whitelist; underground track is dropped
    (`is_underground`). The rail branch of `parse_way` runs before `highway` and falls
    through — a way can be both street and track.
  - **WallLine** — `barrier=city_wall` (the kremlin), 3 m, impassable.
  - **WaterLine** — a *linear* watercourse (`river` 8 m → `ditch` 1.5 m; `water_class`
    is a whitelist; falls through `highway` like rails). `tunnel: bool` marks a culvert:
    not drawn, and the only watercourse kind that does **not** block the navmesh. A
    **culvert portal** (open way ending against a piped one, `water_line_caps`) is cut
    flat instead of round-capped — one rule for both the ribbon mesh and the grid fill.
  - **TreeRow** / **TreeNode** — `natural=tree_row` avenues and single surveyed
    `natural=tree` trees, with optional `spacing`/`radius` from tags.
  - **trees / tree_appears_at** — what the renderer reads; `compose_trees` merges
    forest + avenues of the selected layout, `composed_for` caches which.
- **Building height** (`parse.rs::building_height`) — metres from `height` or
  `building:levels` × 3 m; outside 2–600 m counts as no tag. `None` is normal — every
  consumer owns a default (`DEFAULT_BUILDING_HEIGHT` 15 m). Coverage varies wildly by
  city (NY 97% … Tokyo 5%) and is logged on load.
- **Entrances** — real `entrance=*` nodes are attached to building outlines by exact
  vertex lookup; coverage is thin everywhere, so `map/osm/entrances/` **generates**
  doors for the ~98% of buildings without one. Doors face the street; the count follows
  building *length* at a measured pitch (`ENTRANCE_SPACING` 25 m, floor
  `ENTRANCE_MIN_SPACING` 12 m), capped per cohort; walls a neighbour stands against get
  none. Deterministic per building (LCG seeded by its first vertex). Real doors always
  win — generation only runs on buildings that got none. The `doors` debug toggle draws
  them. Full statistics and the cohort table — osm-map skill,
  `references/entrances.md`.
- **Trees** (`map/osm/planting.rs`) — planted **only inside Wood polygons** plus
  standalone surveyed trees and `tree_row` avenues; deterministic LCGs seeded by
  geometry, obstacle checks via uniform-grid indexes (`planting/index.rs`). Planting
  runs **once at the density ceiling**; the density slider shows a monotone *prefix*
  (`tree_appears_at` / `visible_count`), never a replant. The ceiling
  (`TREE_DENSITY_MAX` 6.5×) is derived from `TREE_MIN_SPACING` (6 m) saturation, not
  chosen. Health check — the `osm parse: N trees planted of M asked …` log line.
  Detail (row bands, placement policies, thresholds) — osm-map skill,
  `references/trees.md`.
- **Footprint bands** (`map/footprint.rs`) — the strips linear geometry occupies on the
  ground, as **(centerline, width, role)** values (`deck_band` / `curb_bands` /
  `passage_band` / `channel_band` / `wall.band()`) plus the width policy (`casing_width`,
  `bridge_curb_width` — moved out of the renderer). One construction, three consumers:
  the grid fill rasterizes the centerline (keeping the 4-connected chain guarantee), the
  mesh build outlines it, the renderer draws its own smoothed copy and takes only the
  widths. **A drawn band and a blocking band match by construction, not by discipline.**
  `CurbCoverage` shares the curb-decision *inputs* (bridges + joining roads, one
  `ways_joined`); the decision itself stays two deliberate strategies (grid probe vs
  mesh polygon difference) — a shared point test handles neither, see its doc.
  `map/roads.rs`, `map/buildings/`): earcut triangulation, per-vertex colors, one white
  `ColorMaterial`; ~7000 buildings cost a handful of entities. Trees stay individual
  entities; tree and building **shadows** are each one merged mesh. **Ribbon**
  (`push_ribbon`) — constant-width band along a polyline with join/cap knobs.
  **Junctions are not computed** — overlapping `Round` caps in one opaque flat-colored
  layer are what makes them look joined; keep the road layer opaque.
- **Style resources** — each is BRP-writable, persisted, and a change rebuilds only its
  own layers from the unchanged `MapData`: **RoadStyle** (join / smoothing / casing →
  `rebuild_roads`; smoothing works on a *copy* — `RoadLine::points`/`width` are
  load-bearing for navmesh, arches, planting, entrances), **BuildingHeightMode**
  (Facade / Shadows / Shadows+tint / 2.5D / 2.5D+shadows+tint → `rebuild_buildings`),
  **TreeStyle** (shape / foliage / ink / variance / density / conifer knobs →
  `rebuild_trees`), **TreeRowStyle** (avenue knobs → recompose), **ConiferNoiseStyle**
  (fbm field of conifer stands; empirical-quantile cut keeps `conifer_share` exact).
  **Bridge / rail / tram layers** — own z-slots and primitives (`push_dashes`,
  `push_ticks`, tram zoom LOD). All detail — osm-map skill.

## Navigation

Summary; mechanics and measurements — **navigation-deep skill** (polymesh in its
`references/polymesh.md`, separation & slots in `references/crowd.md`).

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * GRID_SIZE.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** (`bridge=yes` roads) → buildings block → walls block → **building
  passages carve back through them**. Without bridges the Упа river bisects the map and
  no cross-river path exists.
- **Bridge curbs are impassable** — the same two bands the renderer draws; on dry spans
  they stop a pawn stepping off the deck sideways. Curb tiles decide by an **outward
  probe** (interior seams of a composite bridge stay open), a joining non-bridge road
  opens the curb it covers, the deck carve is narrower than the deck by a tile diagonal,
  and a **seal pass** closes diagonal corner-contact after the carve. Full mechanism —
  navigation-deep skill.
- **Linear waterways block, unlike rails** — water is crossed by bridge, not waded;
  **culverts do not block at all**. The health check after any change here is the
  **pruned-tile count** in the log: a jump of thousands means a watercourse severed a
  district.
- **A rasterized polyline is a 4-connected chain, by construction** — `set_polyline`
  walks the centerline tile by tile (Amanatides–Woo) on top of the capsule test, because
  a thin slanted band otherwise degenerates into corner-touching tiles that northstar
  and `line_of_sight` slip through. Pinned by `tests/navigation.rs`.
- **Ordinary roads do not touch the navmesh** — the grid starts all-passable and the
  fill only subtracts; roads enter it solely through the `bridge`/`passage` carves.
  Rendering and rasterization are independent code paths over the same `RoadLine`.
- **Rails do not touch the navmesh either, deliberately** (`tests/navigation.rs` pins
  it): blocking an unbroken cross-city line would let `prune_unreachable` amputate half
  the map.
- **Building passage** (арка) — `tunnel=building_passage` / `covered=…` sets
  `RoadLine::passage`; carved passable **last**, width capped by `PASSAGE_MAX_WIDTH`.
  Without it, arch-only courtyards get sealed by the prune. The drawn wall opening —
  osm-map skill.
- **prune_unreachable** — BFS flood from the portal; unreachable pockets become
  impassable, because an A* to an unreachable target floods the whole region (a 12 000
  request backlog once "froze" the crowd).
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>`; async tasks read it off-thread. Filled and
  pruned by the map-load thread while the loader is up.
- **PortalPos** (resource) — actual portal position; `PORTAL_POS` is only a hint,
  `snap_portal_position` spirals to the nearest tile with clearance, between fill and
  prune.
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable: A* / Dijkstra /
  Fringe / BFS / **HPA*** (default — 28× cheaper than flat A* at ~10% longer paths) /
  Theta*.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid`, built
  lazily (~12 s) on `AsyncComputeTaskPool` **only when a northstar algorithm is
  selected**; until it lands the dispatcher falls back to flat A*.
- **Backend / Walkable** (`navigation/backend.rs`) — the active backend as one
  cheap-clone `Send` snapshot, and **the resource the whole simulation reads**
  (`Res<Backend>`); async tasks carry it whole, both dispatchers share `Backend::search`.
  The two modes differ not in type but in **who writes it**: `refresh_backend` re-takes it
  every frame in `PreUpdate` (`SimPipeline::Live`), while under determinism it is written
  once on **WorldStarted** and frozen for the run. Seeded by `insert_backend` on
  `OnEnter(Playing)` — the live dispatcher already runs during warmup, before any
  `WorldStarted` — and dropped on `OnExit(Playing)` so the old city's `Arc`s go with it.
  **It has no `Default` on purpose**: the placeholder used to be an empty
  everywhere-passable grid, and a run once went through buildings on it
  (`determinism/replay.rs`). An absent resource is loud — param validation fails and the
  default Bevy error handler panics (tests and the replay scene set `warn`) — unlike a
  silently wrong geometry. **The cost: every system taking `Res<Backend>` must be gated on
  `in_state(Playing)`** — `SimPipeline` picks the mode branch only and does not gate on
  state, and the live pathfinding chain running in `Loading` crashed the app once.
  `walkable()` is the passability view (one read lock per system run):
  `allows`/`nearest_free_point` are backend-strict (grid AND mesh), while `sift_target` /
  `line_of_sight` / `coast_allows` stay deliberately grid-only — the policy lives on the
  methods. **Invariant: outside `navigation/` and `ui/`, the names `PolymeshBuild` /
  `PolymeshDebug` / `PathfindingAlgorithm` do not appear.** `ContinuousSpace` answers the
  separation gate — the polymesh *toggle*, not build readiness.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) — requests become
  async tasks with **visibility gating** (peaceful wanderers off-screen or at zoom ≥
  `WANDER_DISPATCH_MAX_ZOOM` wait; demons and fleeing humans always dispatch) and
  **priority** (urgent first, nearest-to-camera, cap `MAX_PATHFINDING_IN_FLIGHT` 512).
- **Repath on the move** — `to_pathfinding` keeps the current path; a pawn walks the old
  path while the new one is computed. `MovableStateMovingTag` means "has a path **or is
  coasting**". **Coasting** — a pawn whose path ran out mid-repath keeps walking
  `last_direction` over passable tiles; on reply, up to `REPATH_TRIM_LIMIT` leading
  waypoints are trimmed so the first step is not backwards.
- **Rescue** (`movement::rescue_from_impassable`) — a pawn on an impassable tile is
  moved to the nearest passable one. **Trigger is a failed search**, not a clock; what
  counts as free is the *active backend* (`Walkable`). A full pass runs only after every
  completed polymesh build (`rescue_trapped_entities`) — the one moment passability
  changes under standing pawns.
- **find_passable_tile_near** — target tile or its 8 neighbors only; callers tolerate
  `None`.
- **Poly navmesh** (`navigation/polymesh/`) — a polygonal polyanya mesh from the same
  vector sources the grid rasterizes, ring **holes subtracted** as the grid subtracts
  them and only a hole of the *resulting* union dropped (≙ `prune_unreachable`'s
  unreachable pocket — an island opens when a bridge deck cuts its water ring);
  **the default pathfinding backend**
  (`PolymeshDebug::enabled`), the grid serving as fallback while it builds (~5–20 s,
  async, cancellable). Chunked into stitched layers (400 m target) — builds 18× faster
  than flat and searches slightly faster; the level-1 route plus a **corner fill**
  forms the corridor, and paths are **string-pulled** afterwards. polyanya is
  **vendored** (`vendor/polyanya`, edits marked `QWE:`) with divergence fixed at the
  root; `bounded_path` is the only door to it, an exhausted budget **panics** by design,
  and so does a task older than `PATHFINDING_TASK_HANG_SECS`. Everything measured —
  navigation-deep skill, `references/polymesh.md`.
- **Polygonal routing** (`find_path_polymesh`) — paths are **world-space polylines**
  (`VecDeque<Vec2>`, start point included); **the goal stays a tile** (identity for
  stale-answer filtering and arrival). A missed goal is `PathfindingError`, not a
  fallback — watch `answers: N/frame, X % failed` on the speed panel. Endpoint
  tolerance is 1 m (half a navtile), which also sets the agent-radius slider ceiling
  (0.6 m). Coasting and lunge `line_of_sight` stay grid tests.
- **tiny_city / parity tests** (`map/osm/fixture.rs`, `navigation/parity_tests.rs`) —
  the shared `MapData` fixture (river bridge, culvert, arch over the cap, dry-span
  curbs with a joining road, island hole) from which **both fills** are built and must
  agree probe by probe: the executable form of "one rule for both fills". Grid side
  mirrors the game pipeline (fill + prune from the portal landmark — the mesh drops
  unreachable pockets itself). Run after touching either fill; new fill rules add a
  zone here, not a hand-built `MapData`.
- **pathfinding_bench** (`examples/bench/pathfinding_bench.rs`) — offline comparison of
  all six algorithms over a seeded wander-shaped task list. Run it after touching
  `successors`, costs, or the navmesh fill.

## Determinism

- **World seed** (`rng.rs::WorldSeed`, remembered by `prefs.rs`, panel row *Seed* in
  World) — the one number every simulation draw descends from. It governs the
  **simulation**, not the map: OSM is parsed from a cache file, and trees and entrances
  are seeded by their own coordinates (`map/trees/crown.rs::Lcg`,
  `entrances::lcg_seeded_by`), so those are already reproducible without it. Capped at
  `i64::MAX` (`MAX_SEED`) — `toml` cannot store more, and the seed has to survive a
  restart of the app. The *new* button rolls a 9-digit one so it can be read off the
  screen and typed back in.
- **Seed derivation** — `seed_for(world_seed, domain, key)`, two rounds of splitmix64.
  Nothing stores live RNG state, so **a restart has no RNG to reset**: every stream is
  re-derived. `RngDomain: Population | Human | Demon`.
- **Decision stream** (`rng.rs::WanderIndex::next`, on humans *and* demons) — a `SimRng`
  is built per *decision* and dies with it, seeded from `(PawnId, decision number)`. The
  seed is therefore the pawn's **observable identity plus which choice this is**, never
  the history of a stream. Draws do not depend on query iteration order, on how many
  neighbours drew before it this tick, or on how many draws the pawn's previous decision
  happened to consume. Each of those has bitten: a single shared generator collapses under
  any reordering (`panic` draws its repath period while walking a `HashSet<Entity>`, whose
  order differs between runs), and a live per-pawn stream shifts under one added
  `rng.random()` inside a decision.
  Consequence worth having: pawn K's k-th decision draws the same numbers **whenever it
  happens**, so it is the same with the toggle on and off. In normal mode that makes the
  *opening* reproducible (measured: 99.8 % identical first targets across launches) but
  not the run — positions diverge with frame timing. Full replay is what the toggle is
  for. Position is deliberately *not* an input, tempting as it is: pawns stand on tile
  centres bit-for-bit, so `(pawn_id, tile) → target` would be a deterministic function,
  and every trajectory of one on a finite set eventually closes into a cycle — within
  minutes each human would pace a fixed loop forever. The decision number only ever
  grows.
- **PawnId** (`rng.rs`) — a pawn's spawn ordinal within its species and run (humans
  `0..HUMAN_COUNT`, demons `DemonSpawner::spawned`). Used wherever a stable "personal
  number" is needed — the RNG seed key, the flee-fan angle (`personal_spread`), the
  separation axis (`coincident_direction`), the dispatcher tiebreak. **Never `Entity`**:
  entity indices are recycled in a different order after a restart (the free list depends
  on who was eaten in the previous run), so an `entity.index()` hash would drift between a
  run and its replay under an identical seed.
- **SimTick** (`determinism.rs`) — the step counter, incremented at the head of the
  `FixedUpdate` chain. **The unit of replay**: world state is a function of
  `(seed, settings, SimTick)`. Not the same as `SimClock`, which counts virtual seconds
  and loses whatever `max_delta` discarded on a long frame. Compare states by tick, never
  by wall clock.
- **Deterministic** (`determinism.rs::Determinism`, panel toggle) — gates *scheduling*,
  not the dice; the RNG work above is unconditional. Off: today's behavior. On: wander
  target picking runs in `FixedUpdate`, pathfinding answers land on a fixed tick, the
  dispatcher stops looking at the camera, the navigation backend is frozen, and pawn
  separation is off — the Navigation panel's `Separation` row reads `off`, dimmed and
  unclickable, rather than a toggle that flips a resource nothing reads. A run is
  deterministic or not from tick 0, so flipping the toggle (like changing the seed) orders
  a restart via `RestartPending` — and that restart carries `RestartEvent { to_portal:
  true }`: a changed seed or a flipped toggle is a *different world*, and without the
  camera move the setting reads as having done nothing.
- **SimPipeline** (`determinism/mod.rs`) — the toggle in the schedule: two system sets,
  `Live` and `Deterministic`, gated once in `DeterminismPlugin` for every schedule they
  appear in (`Update`, `FixedUpdate`, both `OnEnter` phases). A system declares its branch
  with `.in_set(..)` and never reads the mode; a forgotten `run_if` is no longer a way to
  break replay, and a system with no set is visible by running in both modes. The one
  place the mode is still read directly is `separation_runs`, which needs it **negated**
  ("separation is off — clear its leftovers"), and a set cannot be negated.
  `MovementPlugin` / `HumanPlugin` / `NavigationPlugin` add `DeterminismPlugin` if absent:
  an unconfigured set gates nothing, so both branches would run at once.
- **Frame rate does not matter.** `Time<Fixed>`'s step is constant regardless of fps and
  of `SimSpeed`; the answer to a path query waits for its tick; and everything left in
  `Update` only draws. A slow machine therefore replays the same run more slowly — it does
  not replay a different one.
- **Replay check** (`determinism/replay.rs`, `tests/determinism.rs`,
  `examples/acceptance/determinism_replay.rs`) — same machinery, two scales: the test runs
  a synthetic yard (`fixture::crowded_yard`, dozens of pawns, 96 ticks, ~1 s in
  `cargo test`), the example runs Tula with 20 000 pawns for minutes. Three claims: the
  same seed replays tick for tick, a ragged frame rate does not change the run, a
  different seed does. Two things make it bite, both learned the hard way:
  - **the world must actually move.** `apply_pathfinding_results` needs `SimLoad`, which
    lives in `SimTimePlugin` — absent from the replay app, the system failed parameter
    validation and was skipped in silence, so no path was ever applied and the check
    compared two equally frozen worlds. `replay_app` inserts the resource;
    `Fingerprint::moving` fails the test if a run has nobody moving, so the same class of
    vacuous pass cannot come back.
  - **the scene must be crowded.** Spread over the map, pawns only walk their paths, and
    walking is linear in time — nothing diverges. Divergence is born at thresholds: panic
    radius, demon lunge. Hence the yard, where the whole population and the portal share
    120 m. Verified by mutation: moving `move_moving_entities` into `Update` fails the
    ragged-frame test on the yard and passes on a scattered map.
- **Retire tick** (`RetireAt`, `PATHFINDING_RETIRE_TICKS = 8`) — a request issued on tick
  `T` is applied on exactly `T + 8`, whether or not the search finished; if it did not,
  `apply_pathfinding_results` waits on it (`block_on`). That wait *is* the mechanism: it
  removes "when did the OS get around to it" from the simulation. Eight ticks ≈ 125 ms at
  1×, which is what today's `request → dispatch → task → collect` pipeline already costs,
  so pawn behavior is unchanged. It does **not** set throughput — the dispatch rate below
  does; K only buys a batch wall time before its join, and pays in path staleness. The
  constant must not scale with `SimSpeed` — that is user input and may not influence
  replayed content.
- **Dispatch rate** (`PATHFINDING_WANDER_UNITS_PER_TICK = 128`,
  `PATHFINDING_URGENT_UNITS_PER_TICK = 64`) — how much leaves the queue each tick,
  measured in *predicted search cost*, not in requests. A request costs
  `1 + chebyshev_tiles / PATHFINDING_UNIT_TILES` (integer: a float sum would depend on
  iteration order) — measured, a stroll and a cross-city errand differ 20×, so a budget
  in requests cannot fit both. The rate is derived from what the pool chews per tick
  (~35 ms of CPU). **Never reuse `MAX_*_IN_FLIGHT` here** — those cap *concurrent*
  searches behind the visibility gate; with no gate the same number once meant 65 000
  searches per real second and 2.6 fps. With the rate, the errand wave drains over ~30
  virtual seconds at 60 fps; a long queue is the *normal* state of this mode. At 30× the
  mode settles around 2–5× — by design, and the failure rate is visible for the same
  reason (the sample is the whole map, not the easy on-screen subset).
- **RequestedAt** — the tick a request was filed; the FIFO key of the deterministic
  dispatcher, whose key is `(requested_at, species, pawn_id)` — all integers, since ties
  between floats have no defined order. **Species precedes the number** because `PawnId` is
  only unique *within* a species, and the urgent queue mixes demons with fleeing humans.
  A small rate plus this FIFO *is* the deterministic replacement for the camera gate:
  distant pawns still wait longer, but reproducibly rather than because the player looked
  away. The camera does not appear in it at all.
- **Frozen backend** (`determinism/mod.rs::on_world_started`) — in this mode the
  `Backend` resource is written once, on every **WorldStarted**, and never refreshed:
  `refresh_backend` is `SimPipeline::Live` only. northstar and polymesh finish building at
  some moment of *real* time; a re-taken snapshot would switch backends mid-run, and a
  replay would switch on a different tick. (This is the whole of what the retired
  `DeterministicRun` resource used to be — one resource with two update policies replaced
  two types for one concept.) In this mode warmup waits for the wanted backend instead
  (`NavigationBuildPending`, `loading.rs::poll_warmup`), which costs ~11–14 s on first
  entry into a city on HPA — deliberately. Restarts do not pay it.
- **No pawn warmup** — once the backend is built, this mode enters `Live` immediately:
  the pipeline lives in `FixedUpdate`, which is paused during warmup, so the pawn counter
  could not move (it burned the full timeout); and "pawns on screen" is a camera notion
  that may not influence tick count. The dispatch rate starts the population in a wave
  over a couple of seconds.
- **NeedsWanderTarget** (`movement/components.rs`) — marker held exactly on `Idle` and
  `PathfindingError`, maintained only by the `Movable` transitions. Target picking moves to
  `FixedUpdate` in this mode, i.e. ~30 runs per frame at 30×; without the marker each run
  would scan all 17 000 wanderers to find the few thousand standing ones.
- **The replay contract** — 1:1 holds only while `DemonStyle` / `HumanStyle` /
  `SeparationStyle` / the algorithm / the navtile size are left alone mid-run. Sliders are
  simulation input. Not enforced by code.
- **Restart replays the run** (`tests/determinism.rs::a_restart_replays_the_run`) — a run
  to tick N, `RestartEvent`, a second run to N, identical fingerprints. It runs the second
  half in the *same* `App`, which is what makes it catch state outliving the reset; the
  other checks build a fresh app each time and cannot. Both defects it first caught were
  in how the replay app was assembled, not in the reset: it never announced
  `WorldStarted` (so the run kept the placeholder `Backend` — an empty,
  everywhere-passable grid, and the whole run pathed through buildings; that placeholder
  is now gone, `Backend` has no `Default`), and it left the
  algorithm at the default HPA* while relying on the hierarchy "not being ready in time"
  (a restart re-freezes the backend, and by then it was). A replay app must therefore
  **announce the world start before its first tick** — the demon burst goes out on tick 1,
  and a spawner reset after it hands the second burst the same `PawnId`s — and **pin the
  algorithm** to one that needs no build.
- **Not claimed**: float reproducibility across machines or compilers; replaying a run
  made with the toggle *off*. `bevy_northstar` builds its grid with rayon, so cross-process
  HPA replay is unaudited — within one session the grid outlives R, so restarts are safe.

## Simulation

- **SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`):
  `SpatialRebuild → DemonBehavior → HumanBehavior`. Demons act before humans so a kill
  lands before `escape` — a human is never counted both killed and escaped in one tick.
- **SimPosition / PreviousSimPosition** — simulation-space positions; `Transform` is
  interpolated between them in `RunFixedMainLoop` (after the fixed loop). Systems mutate
  `SimPosition`, never `Transform.translation.xy` directly. Fixed-step order is explicit:
  `snapshot_previous_sim_positions` **before** `SimSet::SpatialRebuild`,
  `move_moving_entities` **after** `SimSet::HumanBehavior` — behavior may move
  `SimPosition` itself (demon lunge), and a snapshot taken after that would flatten one
  tick of interpolation.
- **Movable** — `{speed, path: VecDeque<IVec2>, state}`;
  `MovableState: Idle | Pathfinding(goal) | Moving(goal) | PathfindingError`.
  `to_pathfinding` queues the search and keeps the current path (see *Repath on the
  move*); `to_idle` is the only transition that stops movement.
- **SpatialGrid<T>** — uniform grid per marker type (`Demon`, `Human`), 60 m cells
  (≥ the largest search radius, so a radius query is a 3×3 cell walk). Cells hold
  **entities only** — a candidate's position is read live from `SimPosition` through the
  `pos_of` closure every query takes; storing `Vec2` in cells would go stale by up to a
  cell size. `nearest_in_range_where` / `for_each_in_cells_around` / `for_each_in_rect`.
- **The human grid is incremental, the demon grid is rebuilt.** Humans (~20 000):
  `On<Add, Human>` / `On<Remove, Human>` observers cover spawn and death/despawn, and
  `move_moving_entities` moves an entity between cells on a 60 m boundary crossing — the
  cost scales with crossings, not population. Demons (~100): full rebuild per tick
  (`rebuild_demon_grid`) is cheaper than bookkeeping, and the lunge moves demon
  `SimPosition` outside the mover system anyway.
- **Separation** (`movement/separation/`, Navigation panel, persisted) — soft pairwise
  anti-overlap, **on-screen only, cosmetic by charter**: pawns keep their body radii
  (`HUMAN_BODY_RADIUS` 0.585 m / `DEMON_BODY_RADIUS` 1.17 m — deliberately larger than
  half the sprite) apart. Runs **only on the polymesh backend and never under
  determinism** (`separation_allowed_by_mode` — grid waypoints re-collapse any push), once
  per rendered frame, below `SEPARATION_MAX_ZOOM`. The push is only ever *across* the
  heading; the follower gives way; a walker squeezes past a stander; a fifth of pawns
  dodge left; head-on pairs sidestep; a blocked pawn **steers** (bends its walk heading)
  rather than stopping. Lunging demons are exempt; the one deliberate breach of
  "cosmetic" is `SeparationHolds` + rest-distance arrival forgiveness in
  `move_moving_entities`. Measured on demand by `examples/demos/crowd_demo.rs`; every
  rule's measured reason and the lab — navigation-deep skill, `references/crowd.md`.
- **Destination slot** (`movement/destination.rs`) — the reservation that stops two pawns
  from being aimed at the same point: a `k × k` navtile block per claimed goal
  (`k = ceil(rest distance / navtile_size())`), goal strictly the block's **centre**
  tile; a taken slot ring-searches outward (`SlotSearch`, 16 m default). Claims are
  released on next target selection, despawn, or corpse strip — **not** on arrival (a
  standing pawn *is* the occupancy). Covers wander (human, demon, test walker) via one
  system over `Added<PathfindingRequest>`; **chase and flee are excluded** by design.
  Runs in **both** modes — it is simulation, not cosmetics, hence no camera gate. Detail
  and the failure modes it fixes — navigation-deep skill, `references/crowd.md`.
- **Decision ladder** (`human/decide.rs`, `demon/decide.rs`) — a species' rules live in a
  pure `decide(&…Sense, …) -> …Action`: plain values in, one enum variant out, no
  `Entity`, no `Commands`, no queries, tested without an `App`. `behavior.rs` only
  applies the answer — swap tags, queue a path request, trigger the kill event — so the
  *order* of the rungs, which is half of what these rules are, is readable in one place
  instead of spread over `continue`s. Two things deliberately stay outside. **Expensive
  senses** are asked lazily through a closure, at most once: the demon's `line_of_sight`
  only once the distance already permits a lunge, the human's threat through the
  `ThreatProbe` the ladder itself picks (exact nearest-demon search on decision ticks,
  cell occupancy between them). **Anything that touches the world** stays in the system,
  but its *terms* come out of `decide` — the target-switch search is run by `chase`, its
  radius and chaser limit are a `SwitchRule` the ladder returned. RNG rolls stay in
  `behavior.rs` for a third reason: the decision stream must advance on exactly the ticks
  it advanced on before (see **Decision stream**).
- **Human** states (`human/behavior.rs`, rules in `human/decide.rs`): **Wander**
  (`WanderPause` 2–10 s *between*
  walks, zero at spawn; then 80% head to a random building anywhere in the city — long
  routes, the real pathfinding load — and 20% stroll 20–40 m nearby; the one exception
  is the first target after calming down from panic — see **PanicRecoil**) ⇄
  **Flee** (demon within `HUMAN_PANIC_RADIUS` 60 m; repath every 0.7–1.2 s, step
  40–60 m away from the nearest demon). The Wander → Flee check (`panic`) is
  **inverted**: each demon collects neighbors from the human grid instead of every
  wanderer polling the demon grid, so its cost tracks the crowd near demons, not the
  city population. **Flee fan** — a non-chased fleeing human rotates its away-vector by
  a deterministic per-entity angle (±0.6 rad) so crowds spread instead of forming a
  column; actively chased humans flee straight. Calm-down at ×1.5 radius hysteresis; the
  exact nearest-demon search runs only on a fleeing human's decision ticks — between
  them it only checks demon-grid *cell occupancy* (`any_in_cells_around`); the
  every-tick exact search used to cost 40% of the sim tick. **Escape** — a fleeing
  human within `ESCAPE_MARGIN` of the map border despawns, `telemetry.escaped += 1`.
- **WanderHeading** — the direction a human is walking, kept between walks. Every next
  target is picked inside a `WANDER_CONE` (60°) cone around it — a building errand
  samples `WANDER_BUILDING_TRIES` (8) random buildings and takes the first inside the
  cone. Without the heading each pick was uniformly random and pawns wobbled in place.
  `flee` rewrites it to the away-vector on every repath, so a calmed human resumes
  facing away from the demon rather than on its stale pre-panic course.
- **PanicRecoil** — inserted on the Flee → Wander calm-down, a unit vector *toward* the
  demon (the negated `WanderHeading`; remembered during flee, never queried live —
  `pick_wander_targets` must stay off the demon grid, and at calm-down the demon is
  already >90 m away). While it is on, the next target must be an errand clearing two
  filters in `pick_building_ahead`: not within `RECOIL_CONE` (±45°) of the recoil
  vector, and not closer than `RECOIL_MIN_ERRAND` (90 m — a nearby building just outside
  the cone reproduces the short walk being ruled out). Rejected candidates are dropped
  *before* the "best-aligned of the 8" fallback, which used to hand back a building
  nearly 180° from the heading — straight at the demon. Nothing acceptable → re-roll
  next frame (never a stroll); dropped at the first successful pick.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near*
  stroll, never a building errand; dropped when that target is picked. All 20 000 humans
  queue their first path in the same frame: with errands first the on-screen pawns took
  3.9 s to route, with strolls first — 0.15 s. `PanicRecoil` overrides it.
- **Pace** — a human's personal speed multiplier, rolled once at spawn and stored
  **normalized** (−1…+1); effective speed is `base × (1 + Pace × HumanStyle::spread)`,
  applied to *both* bases (walk and flee) — all three writers of a human's
  `Movable::speed` go through `Pace::speed`. Normalized storage is what makes the
  **Speed spread** slider widen/narrow the ordering the crowd already rolled instead of
  re-dealing it. A component, not derived from `Movable::speed` — that field is
  overwritten on every Wander ⇄ Flee transition. Ceiling 35% is derived: above it the
  fastest humans outrun the slowest demon setting. `sync_human_pace` applies slider
  moves (`resource_changed`), picking the base off `Has<HumanFleeTag>`.
- **CorpseTag** — a killed human: behavior/movement components removed, dark lying
  sprite at `Z_CORPSE`. Not in the human spatial grid (grid filters on `Human`).
- **Demon spawn** — a demon acts from the first tick it exists, the initial burst
  included. A `DemonSpawnPause` (0.5–3 s staging) existed and was removed on request;
  staging an entrance again means a new component, not reviving that one.
- **Demon** states (`demon/behavior.rs`, rules in `demon/decide.rs`): **Wander** (target
  biased away from portal) →
  **Chase** → **Devour** → Wander. Chase claims: **max 2 chasers per target**
  (`ChaserCounts`). Repath throttle 0.4 s, and on that same tick the demon may
  **switch** target: sharing its target, it takes any *unclaimed* human no farther than
  ×1.5 its current distance (the pincer breaks up); otherwise whoever is nearer than
  **×0.7** of the current target — the anti-flip-flop margin (near-equidistant victims
  would trade the demon back and forth every repath tick). Both cases require
  **`line_of_sight`** to the candidate, checked on the search winner only. A chaser with
  **no first path yet** skips the repath tick and waits — repathing cancels the
  in-flight search, and whenever the pipeline answers slower than the victim changes
  tiles the demon cancelled every answer and stood frozen at the portal. Once the first
  path lands, coasting covers the repath gaps and cancelling becomes safe.
  **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight`, the demon
  drops its path and steps `SimPosition` straight at the target at its speed plus
  `DemonStyle::lunge`. Without it a chase never converts: a tile path aims at the tile
  *center* while the victim keeps moving inside it, and the last ~1.4 m is never
  closed. A lunging demon carries **`DemonLungeTag`** (set/cleared in `chase`);
  `draw_lunge_paths` draws its arrow at the victim's live position.
  Kill at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); `killed_this_tick`
  HashSet dedupes double kills within one command flush. **Devour** — pause 1.5–2 s with
  a sine **pulse** ×1 → ×1.5 (0.5 s period), scale reset on exit.
- **DEMON_SPEED** — one base for every state, `HUMAN_FLEE_SPEED × 1.35` (true of the
  *average* human since `Pace`). Do not reintroduce per-state demon speeds: the only
  multipliers are the two user ones, `DemonStyle::speed` (whole demon) and
  `DemonStyle::lunge` (lunge phase only — applied at the one line in `chase` that steps
  `SimPosition`, never written into `Movable::speed`). `Movable::speed` is written
  once, at spawn; `sync_demon_speed` applies slider moves (`resource_changed`).
- **DemonSpawner** — initial burst at the portal rim, then one demon per interval up to
  the cap. Runs in `FixedUpdate` so restart re-fires the burst for free. Cap and
  interval live in **`DemonStyle { cap, interval, speed, lunge }`** (Demon panel
  sliders, persisted); `DEMON_CAP` / `DEMON_SPAWN_INTERVAL` are only its `Default`. The
  burst is capped too (`DEMON_INITIAL_BURST.min(cap)`, fanned over the reduced count);
  lowering the cap never despawns demons already out; the timer's period is re-synced
  inside `tick_spawner` because restart and city switch rebuild `DemonSpawner` whole.
- **Telemetry** — `{killed, escaped}`, BRP-readable, and `killed` is what the World panel
  shows as **Souls**. Invariant (check paused):
  `killed + escaped + alive == HUMAN_COUNT`. At high sim speed BRP reads are skewed —
  pause before asserting.

## UI & debug

Summary; panel internals — **ui-panels skill**; the speed regulator — **sim-speed
skill**.

- **UI input never reaches the world** — the panels sit over the map, so a click, drag or
  scroll that lands on one must not also drive the camera or anything in the world.
  `camera.rs::drag_pan` decides *in the press frame* whether the gesture belongs to the UI
  (`pointer_over_ui` over `HoverMap`) and holds that verdict until release. The rule and
  the idiom — CLAUDE.md.
- **Panel map** — top-right: telemetry + **Speed button** (`ui/speed.rs`); top-left:
  **World / Demon / Human** (`ui/stats.rs` — run counters, `DemonStyle` and `HumanStyle`
  sliders); bottom centre: **City** (`ui/city.rs`); right column bottom-up: Tree rows →
  Trees → Buildings → Roads → hotkey help; left column bottom-up: debug toggles →
  Noise → **Navigation** (`ui/navigation/` — backend cycler `Algo: Navmesh ⇄ Polymesh`,
  the selected backend's settings only, and the `Separation` / `Slots` crowd-knob
  groups). Columns are stacked by **measured** heights (`stack_bottom_columns`;
  `ComputedNode::size` is *physical* px — multiply by `inverse_scale_factor`).
- **Knob** (`ui/knob.rs`) — a panel row **bound to one field of one resource**, in two
  shapes: `spawn_knob` (slider, `SliderBinding<R> { get, set, range, text }`) and
  `spawn_cycle_row` (button that cycles a value, `CycleBinding<R> { cycle, text }`) — all
  of them function pointers. `app.add_knobs::<R>()` registers the drag observer and the
  label/thumb sync **once per resource**, however many knobs it has and across however
  many panels. Panels used to write both by hand — thirteen observers and eight sync
  systems differing only in which field they touched. Use it for any panel row driven by
  a resource; the Navigation panel's rows are the deliberate exception, since their text
  is computed from several resources at once.
- **Shared kits** — `ui/slider.rs` (`spawn_slider_row`, `quantize`, `apply_step`,
  `retarget`, one `sync_slider_thumbs` for all panels — the layer under the knob kit,
  called directly only by the crowd demo, whose sliders drive demo-local state rather
  than a resource) and `ui/rows.rs` (`spawn_value_row` button rows, `RowInert` for rows
  whose click currently does nothing). Use these, don't hand-roll a panel row.
- **Debug toggles** (`ui/debug.rs`) — grid / doors / movepath / noise buttons, plus the
  `camera:` and `navtile:` cyclers (global settings, deliberately not under a backend
  section). The navmesh overlay is **one merged mesh** (per-tile entities once cost
  330 k); the noise overlay is one CPU-built texture sprite. A cycler goes green while
  its resource equals `Default::default()`.
- **Camera start view** (`camera.rs`) — `CameraPositionMode` (`reset | save`, default
  `save`, persisted): where the camera stands when the world comes up. `save` writes
  `SavedCameraView` on exit (a `Last` system after `bevy::window::ExitSystems`) and
  debounced during play (`Time<Real>` — virtual time would never fire while paused).
  **RR** (double R within 0.5 s) and `RestartEvent { to_portal: true }` go to the portal
  at `START_ZOOM` regardless of mode. A city switch always resets to the new portal.
- **Sim speed** (`sim_time.rs`) — Space pauses; `=`/`-` walk `SPEED_LADDER`
  (1-2-5-10-20-30); `MAX_SIM_SPEED` 30× is a **deliberate product cap**. **SimSpeed**
  `{requested, pipeline, affordable, effective, actual}` — a regulator throttles
  `effective` to what the machine carries (CPU solve + pathfinding-pipeline ceiling),
  and `guard_frame_budget` (first system of `FixedUpdate`) hard-stops a frame's fixed
  loop at `SIM_FRAME_BUDGET_MS`, booking stripped ticks into **TickDebt**. `actual` is
  the only honest reading — the panel and `is_throttled` use it. Set speed over BRP with
  `res set SimSpeed .requested N`, never `brp speed`. The regulator's memory (`pipeline`
  backoff, `SimLoad`'s smoothed tick cost) is reset by **WorldStarted** — a new run
  never starts throttled by the previous world's measurements. **SimClock** — virtual
  seconds of the current world, zeroed by the same observer; compare states by
  `SimTick`, not by it.
  Per-tick cost is ~0.1 ms (`sim/*_ms` diagnostics). The whole control loop — sim-speed
  skill.
- **Tunable resource** (`prefs.rs`) — a resource the user retunes: a panel, a hotkey or a
  BRP write. Two things are asked of one, and both live in `prefs.rs`:
  **`app.track_pref::<T>()`** persists it (a `bevy::settings::SettingsGroup` in
  `settings.toml`, macOS `~/Library/Preferences/com.github.morr.qwe/`, saved via
  `SaveSettingsSync::IfChanged`), and the run condition **`retuned::<T>`** —
  `changed && !added` — gates whatever rebuilds on it. Registration sits with the
  resource's own plugin, next to its `init_resource`; the hand-kept mirror it replaced
  had lost `RoadStyle`. `SavedCameraView` is deliberately *not* tracked — it changes on
  every camera-drag frame and has its own debounce (`camera::track_camera_view`).
  `PrefsPlugin` is registered **last** (the settings scan needs every `register_type`).
  Delete the file to reset.
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
  geo anchor).
- OSM pipeline: `src/map/osm/{overpass,download,parse,model}.rs`; rendering:
  `src/map/{meshing,spawn}.rs`. Detail — **osm-map skill** (its `references/` also
  carry the tag coverage audit and the crown-algorithm write-up).
- Navigation: `src/navigation/{navmesh,astar,northstar,polymesh}`;
  movement/interpolation: `src/movement/`. Detail — **navigation-deep skill**.
- Speed & regulator: `src/sim_time.rs`. Detail — **sim-speed skill**.
- UI: `src/ui/`, camera: `src/camera.rs`. Detail — **ui-panels skill**.
- State machines: `src/demon/behavior.rs`, `src/human/behavior.rs`.
- Tests: `tests/navigation.rs` (synthetic navmesh + hand-built `MapData`),
  `tests/spatial.rs`, `tests/movement.rs` (fixed-step walk over several waypoints,
  step length independent of `time_scale`, `Transform` interpolation), unit tests inside
  `map/osm/*` and `map/meshing.rs` (projection, ring assembly, tree determinism, earcut).
