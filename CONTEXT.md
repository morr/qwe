# CONTEXT

Domain glossary for QWE. Use these terms verbatim in commit messages, hypotheses, test
names, and code identifiers. If a concept you need isn't here, that's a signal — either
you're inventing language the project doesn't use (reconsider) or the file has a real gap
(update it in the same change that introduces the concept).

## Project shape

**QWE** is a 2D real-time simulation prototype: a **demon invasion of the Tula city
center**. The map is generated from real OpenStreetMap data at first launch. 20 000
humans wander the streets; demons pour out of a portal, chase and devour them; humans
panic and flee off-map. Built on **Bevy 0.19 ECS** — one plugin per feature, registered
in `main.rs`.

## Coordinates & units

- World units are **meters**. Origin — **south-west corner** of the map, y grows north.
  All world coordinates are positive. `MAP_SIZE = 3000 × 2250` m.
- **Navtile** — navigation grid cell, `NAVTILE_SIZE = 2` m. `GRID_SIZE = 1500 × 1125`
  tiles, hand-maintained as `MAP_SIZE / NAVTILE_SIZE` (they can silently desync — keep
  them in step). `grid.rs`: `world_to_tile` / `tile_center` / `tile_in_bounds`.
- **Geo anchor** — `GEO_CENTER_LAT/LON` (Tula, kremlin near frame center). Projection is
  local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW corner → (0,0),
  f64 math, `MAP_SIZE`-sized bbox derived from the center.
- **Z-layers** — constants in `settings.rs`: ground 0 → parks 0.5 → water 1 → alleys 1.5
  → roads 2 → corpses 3 → portal 4 → buildings 5 → units → trees 20. Units are y-sorted:
  `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR` (10 − y·0.002). **Invariant: the unit z
  range must stay above buildings (5) for any y ≤ MAP_SIZE.y** — a bigger map once sank
  northern units under roads.

## App lifecycle

- **AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
  (progress text, red error + **Retry** button on failure). All world spawning happens in
  `OnEnter(Playing)`.
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. Navmesh must
  be filled before population spawns, or humans land in the river.
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` download:
  `Connecting → Downloading{bytes,total} → Parsing → Done(MapData) | Failed(msg)`,
  polled via `Arc<Mutex<_>>` by `poll_job`.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns humans/corpses/demons/walkers,
  resets `DemonSpawner` + `Telemetry`, respawns population. The navmesh persists — it is
  filled once per app run.

## OSM map pipeline

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). Query covers: `building` (way+rel),
  `highway` (way), `natural=water` / `waterway=riverbank` (way+rel), `leisure=park|garden`,
  `landuse=grass|recreation_ground|forest`, `barrier=city_wall`.
- **Cache** — `assets/osm/tula_{lat}_{lon}_{w}x{h}.json` (gitignored). Parameters live in
  the file name, so changing settings invalidates it. Written **only after successful
  parse**; a broken cache self-heals (deleted, re-downloaded). Second launch never
  touches the network.
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes; rings are open (no repeated last point).
    `AreaKind: Building | Kremlin | Water | Park`.
  - **RoadLine** — centerline polyline + width by highway class (primary 16 → footway
    3.5). `RoadClass: Street | Alley` (alleys = footways, park paths; different color and
    z). `bridge` flag — see navmesh.
  - **WallLine** — `barrier=city_wall` (the Tula kremlin), 3 m wide, kremlin red,
    impassable.
  - **trees** — `(pos, radius)` pairs, precomputed at parse.
- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.
- **Trees** — deterministic LCG seeded per park polygon, density ~1 / 1600 m², rejection
  sampling inside the polygon, never on buildings or within `TREE_CLEARANCE` of a road
  edge (park alleys count as roads).
- **Rendering** (`map/meshing.rs` + `map/spawn.rs`) — **one merged `Mesh2d` per layer**
  (parks, water, alleys, roads, facades, roofs, walls): `MeshBuilder` triangulates
  polygons via `earcutr` (holes supported, degenerate contours skipped + counted) and
  emits per-vertex colors over a single white `ColorMaterial`. ~2800 buildings cost ~7
  entities. **Facade** — pseudo-3D: the footprint polygon shifted (0, −3) in a darker
  color at z just below the roof, visible only along south edges. Trees stay individual
  entities (shared circle mesh, 3 materials).

## Navigation

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * GRID_SIZE.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water blocks → **bridge corridors carve
  passable strips back** (`bridge=yes` roads) → buildings block → walls block. Without
  bridges the Упа river bisects the map and no cross-river path exists.
- **prune_unreachable** — BFS flood from the portal tile; passable-but-unreachable
  pockets (enclosed courtyards, islands) become impassable. Reason: an A* request to an
  unreachable target floods the whole reachable region (tens of ms each); before pruning
  this once piled up a 12 000-request backlog and humans "froze". 4-connectivity matches
  A* reachability because of the no-corner-cutting rule.
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>` resource; async A* tasks read it off-thread.
  Starts empty (all passable), filled by `fill_navmesh` in `WorldInitSet::Navmesh`.
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  by the bottom-left button: A* / Dijkstra / Fringe / BFS (all from the `pathfinding`
  crate over the navmesh) plus **HPA*** and **Theta*** (hierarchical, from
  `bevy_northstar`). IDA*/IDDFS are deliberately excluded (never finish on open grids).
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/pathfinding_bench.rs`
  (1.3 ms vs 36.4 ms mean, 15 ms vs 450 ms worst case) at ~10% longer paths. The other
  five stay switchable for comparison.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
  once from the final navmesh (after pruning; chunk 25, ~1.5 s build), wrapped in `Arc`,
  called directly from async tasks — the crate's plugin is not used. Long paths cost
  ~0.5 ms vs ~40 ms for flat A*.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
  `Movable::to_pathfinding` only queues a `PathfindingRequest`;
  `dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks
  (polled with `check_ready`). **Visibility gating**: peacefully wandering humans
  OUTSIDE the camera view (×1.2 margin) are never dispatched — their requests wait
  until the camera arrives; demons and fleeing humans are always dispatched. In-frame
  requests go nearest-to-camera-center first, capped at `MAX_PATHFINDING_IN_FLIGHT`
  (512). The speed panel shows in-flight / queued / avg ms.
- **find_passable_tile_near** — the target tile or its 8 neighbors only; callers must
  tolerate `None`.
- **pathfinding_bench** (`examples/pathfinding_bench.rs`) — offline comparison of all six
  algorithms without booting Bevy: reads the OSM cache, rebuilds the navmesh exactly as
  `WorldInitSet::Navmesh` does (fill → `snap_portal_position` → prune), generates one
  seeded task list mirroring human wander (80% random building, 20% short stroll) and
  replays that *same* list per algorithm across a shared atomic work cursor. Reports
  wall / cpu / avg / p50 / p95 / max and mean path length. Run it after touching
  `successors`, costs, or the navmesh fill.
- **PortalPos** (resource) — actual portal position. `PORTAL_POS` in settings is only a
  **hint**; `snap_portal` spirals out to the nearest tile with clearance derived from
  `PORTAL_DIAMETER`. Runs before `prune_unreachable` (the flood starts from the snapped
  position).

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
  `to_pathfinding` spawns the async A* task.
- **SpatialGrid<T>** — uniform grid of `(Entity, Vec2)` per marker type (`Demon`,
  `Human`), 60 m cells (≥ the largest search radius), fully rebuilt every tick.
  `nearest_in_range_where` — nearest entity passing a filter.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s *between*
  walks, zero at spawn so nobody stands around after launch; then 80%
  head to a random building anywhere in the city — long routes, the real pathfinding
  load — and 20% stroll 20–40 m nearby) ⇄ **Flee** (demon within `HUMAN_PANIC_RADIUS`
  60 m; repath every 0.7–1.2 s, step 40–60 m away from the nearest demon). **Flee fan** — a
  non-chased fleeing human rotates its away-vector by a deterministic per-entity angle
  (±0.6 rad) so crowds spread instead of forming a column; actively chased humans flee
  straight. Calm-down at ×1.5 radius hysteresis. **Escape** — a fleeing human within
  `ESCAPE_MARGIN` of the map border despawns, `telemetry.escaped += 1`.
- **CorpseTag** — a killed human: behavior/movement components removed, dark lying
  sprite at `Z_CORPSE`. Not in the human spatial grid (grid filters on `Human`).
- **Demon** states (`demon/behavior.rs`): **Wander** (target biased away from portal) →
  **Chase** → **Devour** → Wander. Chase claims: **max 2 chasers per target**
  (`ChaserCounts`); a demon sharing a target opportunistically **switches** to an
  unclaimed human no farther than ×1.5 its current distance. Repath throttle 0.4 s.
  **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight` to the victim,
  the demon drops its path and steps `SimPosition` straight at the target. Without it a
  chase never converts: a tile path aims at the *center* of the victim's tile while the
  victim keeps moving inside it, so the last ~1.4 m — more than `KILL_DISTANCE` — is
  never closed and the demon "almost catches" forever. The line-of-sight check is what
  keeps the lunge from cutting through a building when the victim rounds a corner.
  Kill at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); `killed_this_tick`
  HashSet dedupes double kills within one command flush. **Devour** — pause 1.5–2 s with
  a sine **pulse** ×1 → ×1.5 (0.5 s period), scale reset on exit.
- **DEMON_SPEED** — single constant, always `HUMAN_FLEE_SPEED × 1.35`, both wandering
  and chasing. Do not reintroduce per-state demon speeds.
- **DemonSpawner** — initial burst of 8 at the portal rim, then one per 5 s up to
  `DEMON_CAP = 100`. Runs in `FixedUpdate` so restart re-fires the burst for free.
- **Telemetry** — `{killed, escaped}`, BRP-readable. Invariant (check paused):
  `killed + escaped + alive == HUMAN_COUNT`. At high sim speed BRP reads are skewed —
  pause before asserting.

## UI & debug

- **Speed panel** (`ui/speed.rs`) — top-right: speed multiplier, pathfinding in-flight /
  avg ms, entity count. Fixed width + right-padded digits (no jitter).
- **Debug toggles** (`ui/debug.rs`) — grid / navmesh / movepath buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay is **one merged mesh** — per-tile entities once cost 330 k entities.
- **sim_time.rs** — Space pauses, `=`/`-` walk the speed ladder. Speeds above ~10×
  saturate the CPU with FixedUpdate ticks; fast-forward at 5–10×.
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
  `src/map/{meshing,spawn}.rs`.
- Navigation: `src/navigation/{navmesh,astar,mod}.rs`; movement/interpolation:
  `src/movement/`.
- State machines: `src/demon/behavior.rs`, `src/human/behavior.rs`.
- Tests: `tests/navigation.rs` (synthetic navmesh + hand-built `MapData`),
  `tests/spatial.rs`, unit tests inside `map/osm/*` and `map/meshing.rs` (projection,
  ring assembly, tree determinism, earcut).
