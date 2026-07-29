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
- **PlayPhase** (sub-state of `Playing`) — `Warmup → Live`. During **Warmup** the world
  exists but `Time<Virtual>` is **paused** and the loader screen stays up reading
  "Routing pawns... N left"; `poll_warmup` counts pawns *inside the camera view* that
  still hold a `PathfindingRequest`/`PathfindingTask` and flips to `Live` when none are
  left (or after `WARMUP_TIMEOUT` = 10 s, logged as a warning). Reason: all 20 000 humans
  queue a path in the same frame, and without the hold the visible ones stood still for
  the first seconds. Typical warmup **~0.15 s** — see `HumanFirstWanderTag`.
  `Live` is what despawns the loader and reveals the game UI (`GameUiRoot`).
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. Navmesh must
  be filled before population spawns, or humans land in the river.
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` that
  prepares everything not needing ECS: `Connecting → Downloading{bytes,total} → Parsing
  → BuildingNavmesh → Pruning → Done(LoadedWorld{map, portal}) | Failed(msg)`, polled via
  `Arc<Mutex<_>>` by `poll_job`, every state a line on the loader screen. It writes the
  navmesh through the `ArcNavmesh` handle it is given and returns the snapped portal
  position. **Rule: heavy init belongs in this thread, not in `OnEnter(Playing)`** — no
  frame is drawn inside a schedule, so work there freezes the loader on its last message.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns humans/corpses/demons/walkers,
  resets `DemonSpawner` + `Telemetry`, respawns population. The navmesh persists — it is
  filled once per city.
- **City** (`city.rs`, resource, remembered by `prefs.rs`) — which city the map is built
  from: `Tula | NewYork | Paris | Berlin | London`. Each carries its **geo center** (bbox
  center of the Overpass extract), its **portal hint** and its **cache slug**; `MAP_SIZE`
  and therefore `GRID_SIZE` are shared, so switching city never resizes the navmesh.
  Panel — bottom centre (`ui/city.rs`), the current city's button is highlighted.
- **City switch = full world reload.** Writing `City` (button or BRP) sends the app back
  to `AppState::Loading`: leaving `Playing` despawns the scene, the load thread downloads
  / re-parses the new extract, refills the same navmesh (`fill_from_mapdata` resets it
  first), re-snaps the portal, and `OnEnter(Playing)` rebuilds map, population and
  camera position. `DemonSpawner`, `Telemetry`, `NorthstarGrid` and `WarmupProgress` are
  reset on the way. The switch is gated on `in_state(Playing)` — restarting a load on top
  of a running one would put two threads into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city.
  Every world entity must carry it; the list of spawn sites and the rule live in
  `CLAUDE.md` ("World entities"), and `loading.rs::warn_leftover_world_entities` warns on
  every entry into `Loading` if something survived.

## OSM map pipeline

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). Query covers: `building` (way+rel),
  `highway` (way), `natural=water` / `waterway=riverbank` (way+rel), `leisure=park|garden`,
  `landuse=recreation_ground|forest` + `natural=wood`, `landuse=grass|meadow` /
  `natural=grassland|meadow`, `natural=sand|beach`, `barrier=city_wall`. The bbox is
  `MAP_SIZE` around the selected `City`'s geo center.
- **Mirrors** — `OVERPASS_URLS` in `download.rs` is tried in order (`overpass-api.de` →
  `kumi.systems` → `private.coffee`). On dense cities the main instance answers 504
  "server too busy", or worse, a **200 with an HTML error page** — hence the
  "response must start with `{`" check before a mirror is considered successful.
- **Cache** — `assets/osm/tula_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored).
  Parameters live in the file name, so changing settings invalidates it; **bump
  `QUERY_VERSION` in `overpass.rs` whenever the query gains tags**, or every existing
  cache keeps serving an extract that lacks them. Written **only after successful
  parse**; a broken cache self-heals (deleted, re-downloaded). Second launch never
  touches the network.
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes; rings are open (no repeated last point).
    `AreaKind: Building | Kremlin | Water | Park | Wood | Grass | Sand`. **Park** is the
    light base fill; **Wood** (`natural=wood` / `landuse=forest`) are the darker stands
    *inside* it and the **only** areas that carry trees; **Grass** (lawns, meadows) and
    **Sand** (beaches) also sit above the park fill, lighter green / sandy. Everything
    but Wood stays open ground — that is what makes the open half of a park read as a
    field, the way it does on OSM.
  - **RoadLine** — centerline polyline + width by highway class (primary 16 → footway
    3.5). `RoadClass: Street | Alley` (alleys = footways, park paths; different color and
    z). `bridge` flag — see navmesh.
  - **WallLine** — `barrier=city_wall` (the Tula kremlin), 3 m wide, kremlin red,
    impassable.
  - **trees** — `(pos, radius)` pairs, precomputed at parse.
- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.
- **Trees** — planted **only inside Wood polygons**, never across a whole park:
  deterministic LCG seeded per wood polygon, density ~1 / 1230 m², rejection sampling
  inside the polygon, never on buildings or within `TREE_CLEARANCE` (1.5 m) of a road
  edge (park alleys count as roads). Also rejected inside water or within
  `TREE_SHORE_CLEARANCE` (3 m) of a shoreline — a pond is drawn *over* the park fill, so
  an unfiltered tree grew out of the water — and anywhere inside a Grass or Sand polygon
  (a lawn is a lawn; overhang from a neighbouring tree is fine).
- **Rendering** (`map/meshing.rs` + `map/spawn.rs`) — **one merged `Mesh2d` per layer**
  (parks, water, alleys, roads, facades, roofs, walls): `MeshBuilder` triangulates
  polygons via `earcutr` (holes supported, degenerate contours skipped + counted) and
  emits per-vertex colors over a single white `ColorMaterial`. ~2800 buildings cost ~7
  entities. **Facade** — pseudo-3D: the footprint polygon shifted (0, −3) in a darker
  color at z just below the roof, visible only along south edges. Trees stay individual
  entities (see tree crowns below).
- **Tree crowns** (`map/trees.rs`, algorithm write-up — `TREE_ALGO.md`) — Watabou-style
  procedural trees: a jittered 12-gon **bloated** into a cloud outline (recursive
  outward midpoint extrusion), ink outline, dashed inner **bands** shaded away from the
  light, and a **long shadow** — the crown silhouette stretched ×1.4 along the 30°
  shadow axis on `Z_TREE_SHADOW`. `TREE_VARIANTS` unit-radius mesh pairs (crown+shadow)
  are reused across all trees; per tree — variant, quantized brightness tint (material
  multiplies vertex colors, so ink stays ink) and radius as `Transform::scale`.
  Geometry RNG is a deterministic Lehmer LCG (same family as tree planting).
- **TreeStyle** (resource, BRP-writable) — the watabou «Style settings → Trees» tab:
  `foliage`, `details` (ink), `variance` (brightness spread), `shape`. **TreeShape** is
  `Cotton | Conifer | Palm` — cloud outline (`bloat`), spiky cone (`Spiker::simple`),
  bent fronds (`Spiker::bent`). Any change reruns `rebuild_trees` (despawn `TreeTag`,
  respawn from the unchanged `MapData::trees` positions); the panel lives in
  `ui/trees.rs`, bottom-right, one cycling button per field.

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
  Starts empty (all passable), filled and pruned by the map-load thread while the loader
  screen is still up (`JobState::BuildingNavmesh` / `Pruning`).
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  by the bottom-left button: A* / Dijkstra / Fringe / BFS (all from the `pathfinding`
  crate over the navmesh) plus **HPA*** and **Theta*** (hierarchical, from
  `bevy_northstar`). IDA*/IDDFS are deliberately excluded (never finish on open grids).
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/pathfinding_bench.rs`
  (1.3 ms vs 36.4 ms mean, 15 ms vs 450 ms worst case) at ~10% longer paths. The other
  five stay switchable for comparison.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
  once from the final navmesh (after pruning; chunk 25), wrapped in `Arc`, called directly
  from async tasks — the crate's plugin is not used. Long paths cost ~0.5 ms vs ~40 ms for
  flat A*. The build takes **~12 s** on the 5600 × 3700 map, so it runs as an
  `AsyncComputeTaskPool` task started on `OnEnter(PlayPhase::Live)` and picked up by
  `poll_northstar_build`; until it lands, `NorthstarGrid::get()` is `None` and the
  dispatcher **falls back to flat A\*** for HPA*/Theta* requests. Doing it inline cost
  11 s of frozen loader screen; starting it before the warmup ends made it fight the
  warmup's A* for cores through rayon (85 ms per search instead of 36 ms).
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
  the map-load thread does (fill → `snap_portal_position` → prune), generates one
  seeded task list mirroring human wander (80% random building, 20% short stroll) and
  replays that *same* list per algorithm across a shared atomic work cursor. Reports
  wall / cpu / avg / p50 / p95 / max and mean path length. Run it after touching
  `successors`, costs, or the navmesh fill.
- **PortalPos** (resource) — actual portal position. `PORTAL_POS` in settings is only a
  **hint**; `snap_portal_position` spirals out to the nearest tile with clearance derived
  from `PORTAL_DIAMETER`. The map-load thread snaps it between fill and prune (the flood
  starts from the snapped position) and hands it back in `LoadedWorld`; `poll_job` inserts
  the resource before switching to `Playing`.

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
- **WanderHeading** — the direction a human is walking, kept between walks. Every next
  target, near stroll or cross-city errand, is picked inside a `WANDER_CONE` (60°)
  cone around it — a building errand samples `WANDER_BUILDING_TRIES` (8) random
  buildings and takes the first one inside the cone. Without the heading each pick was
  uniformly random and pawns wobbled in place instead of walking somewhere.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near*
  stroll, never a building errand; the tag is dropped when that target is picked. All
  20 000 humans queue their first path in the same frame, and cross-city A* costs
  hundreds of ms per request: with errands first the on-screen pawns took 3.9 s to route
  (the whole `PlayPhase::Warmup`), with strolls first — 0.15 s.
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
- **Tree style panel** (`ui/trees.rs`) — bottom-right: shape / foliage / crown details /
  color variance, one button per row cycling through a fixed palette (`bevy_ui` has no
  text input, so hex fields became cycles). Writes `TreeStyle`; `map::trees::rebuild_trees`
  picks the change up. Also settable over BRP: `res set TreeStyle .shape '"Conifer"'`.
- **Debug toggles** (`ui/debug.rs`) — grid / navmesh / movepath buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay is **one merged mesh** — per-tile entities once cost 330 k entities.
- **sim_time.rs** — Space pauses, `=`/`-` walk the speed ladder.
  - **SimSpeed** — `{requested, effective, actual}`. `requested` is what the ladder says;
    `effective` is the regulator's command, what reaches `Time<Virtual>` after **fps
    throttling**; `actual` is measured — virtual seconds per real second, averaged over
    `ACTUAL_SPEED_WINDOW` (0.5 s of *real* time, so long frames weigh what they cost).
    `actual` is the only honest one: Bevy clips a frame's virtual delta at `max_delta`, so
    a stall eats simulated time behind the regulator's back. The panel and `is_throttled`
    read `actual`.
  - **Speed ceiling** — Bevy hands `FixedUpdate` at most `Time<Virtual>::max_delta`
    (`MAX_FRAME_DELTA` = 0.25 s, pinned explicitly at startup) of virtual time per frame,
    so a speed of S is only real if `S ≤ fps × MAX_FRAME_DELTA` — 15 at 60 fps, 10 at
    40 fps. Above the ceiling the ticks pile into frames, `Update` (path dispatcher,
    input, UI) starves, and humans that finish a route just stand there.
    `throttle_speed_to_fps` closes the loop on measured fps and eases `effective` toward
    the ceiling (`SPEED_SETTLE_RATE` up, the faster `SPEED_DROP_RATE` down). It throttles
    **below 1× too** — under 4 fps even real time is unaffordable — down to
    `MIN_SIM_SPEED` (0.1). The panel shows `Speed: 15x → 8.6x` when limited, and
    `Speed: 1x → 0.42x` while something (the async northstar build, say) is starving the
    frame.
  - Set the requested speed over BRP with `res set SimSpeed .requested N` — `brp speed`
    writes `Time<Virtual>` directly and the throttle overwrites it on the next frame.
  - **Per-tick cost** (`sim/*_ms` diagnostics, 20 000 humans / 100 demons): `panic`
    ~1.8 ms ≫ `spatial` ~0.7 ms > `move` ~0.16 ms ≈ `flee` ~0.14 ms ≫ `chase` ~0.01 ms.
    `panic` scans every wandering human against the demon grid every tick — that single
    system is what sets the speed ceiling.
- **Remembered UI options** (`prefs.rs`) — every UI-settable resource (`DebugGrid`,
  `DebugNavmesh`, `DrawMovePaths`, `PathfindingAlgorithm`, `TreeStyle`) is a
  `bevy::settings::SettingsGroup`, so a click survives a restart. `SettingsPlugin` reads
  `settings.toml` from the OS settings dir (macOS:
  `~/Library/Preferences/com.github.morr.qwe/`) while the `App` is still being
  built, before any schedule; `PrefsPlugin` is registered **last** because that scan needs
  the other plugins' `register_type` calls to have run. Any change to those resources —
  click, key P, BRP — triggers `SaveSettingsSync::IfChanged`. Delete the file to reset.
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
  `tests/spatial.rs`, `tests/movement.rs` (fixed-step walk over several waypoints,
  step length independent of `time_scale`, `Transform` interpolation), unit tests inside
  `map/osm/*` and `map/meshing.rs` (projection, ring assembly, tree determinism, earcut).
