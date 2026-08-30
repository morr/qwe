---
name: navigation-deep
description: Use when working on navigation or movement internals in qwe — the navmesh fill (bridge curbs, waterways, passages), pathfinding backends (A*/HPA*/northstar, polymesh), the request/dispatch/task pipeline, repath/coasting, rescue, pawn separation and destination slots. Deep detail behind CONTEXT.md's Navigation and Simulation sections.
---

# Navigation & movement — deep detail

This is the detail layer behind the **Navigation** and crowd parts of the
**Simulation** summaries in `CONTEXT.md`. The invariants (fill order, what blocks and
what doesn't, no corner cutting) live there; this file holds the mechanisms and the
measurements behind them.

Two deep dives live next to this file and are read on demand:

- `references/polymesh.md` — the polygonal navmesh: build, chunking/seams/stitching,
  corridor + string-pulling, routing, the divergence fixes and the vendored polyanya.
- `references/crowd.md` — pawn separation (all rules and their measured reasons, the
  separation lab) and destination slots.

When a change here introduces or retires a concept, update the matching summary bullet
in `CONTEXT.md` and the detail here in the same change.

## Navtile size

The navtile is **2 m by default and runtime-switchable to 1 m** via the `navtile:` cycler in
the Debug tab (`NavtileBase` in `settings.rs`, persisted in prefs). Switching it reloads the
world like a city switch, except the camera stays where it was — same city, same spot under
inspection.

**The live value is a process-global atomic**, read by `settings::navtile_size()`: background
threads (navmesh fill, entrance generation) have no ECS access. It is written only in
`OnEnter(Loading)`, before the load thread starts.

Grid size is derived as `MAP_SIZE / navtile_size()` (2800 × 1850 tiles at 2 m). **A filled
`Navmesh` carries its own `grid_size` / `tile_size` snapshot**, so a stale snapshot (a
cancelled northstar build) never indexes against the switched atomic. The snapshot owns the
conversions too: rasterisation (`set_area`/`row_spans`, `visit_polyline*`,
`visit_segment_tiles`) and the navmesh-side queries (`line_of_sight`,
`snap_portal_position`) go through `Navmesh::to_tile` / `Navmesh::tile_center`, never
through `grid::world_to_tile`, which reads the atomic.

**The northstar chunk scales with the tile** to stay 50 world metres — 25 tiles at 2 m, 50 at
1 m. With the chunk pinned at 25 tiles a 1 m build explodes from ~14 s to ~140 s.

Cost of 1 m, measured: northstar build ~14 s vs ~11 s, HPA* ×1.7 CPU, +1.6 GB RSS. A change
to the navtile size is simulation input — it breaks a replay in flight (`determinism`).

`grid.rs` holds the global conversions — `world_to_tile` / `tile_center` — for callers with
no `Navmesh` in hand (`Walkable`, movement, wander, the overlays); they read the atomic.

## The grid navmesh

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * grid_size.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** (`bridge=yes` roads) → buildings block → walls block → **building
  passages carve back through them**. Without bridges the Упа river bisects the map and
  no cross-river path exists.
- **Bridge curbs are impassable** — the same two bands the renderer draws
  (`RoadLine::curb_bands` in `map/footprint.rs`: edge centerlines from `miter_offsets` +
  curb width, one construction for the grid fill, the mesh build and the renderer, so
  the blocked strip matches the drawn one by construction). Over water
  this changes nothing (water already blocks); on dry spans — approaches, overpasses —
  the curb is what stops a pawn from stepping off the deck sideways. Only the two
  longitudinal edges block, the deck ends stay open. All curbs block *before* any deck
  carves — the render layering (curbs under fills) repeated in the grid, so at a
  junction of two bridge ways one way's deck re-carves the other's curb and the bridge
  is never walled across by its own curb. The deck carve is `width + curb − tile·√2`
  (`navmesh.rs::fill_from_mapdata`) — it stops **half a tile diagonal short of the curb
  centerline**, which at the default 2 m navtile leaves it narrower than the deck itself
  by `tile·√2 − curb`: a curb-chain tile's center wanders up to half a diagonal (√2 m)
  off the curb centerline — i.e. *into* the deck on a slanted bridge — and carving out
  to the centerline re-opened those tiles, turning the barrier into a dashed line. Deck
  connectivity survives the narrowing because `set_polyline` always walks the
  centerline chain, the same guarantee thin waterways rely on. A curb tile does
  **not** block unconditionally — OSM cuts one physical bridge into several ways
  (carriageway and its sidewalk are parallel ribbons), so each curb tile records
  **every** way that owns it (`CurbTile::owners`) and the decision is an
  **outward probe**: step one tile away from the owning way's centerline — if
  that point lands inside
  another bridge way's ribbon, the tile is an interior seam and stays open; if
  it lands on nothing, the tile is the outer boundary of the whole composite
  and blocks. The decision is an `any` over the owners, so a dropped owner can
  only *lose* barrier, never add it — and a pair is the typical case, not the
  maximum: a junction of footbridge ways routinely puts three or more curb
  chains on one tile (London 528 such tiles out of 19 278, up to seven owners;
  New York 1 064; Tokyo 148; Tula 1). The two fixed slots the type used to keep
  cost London 3 curb tiles and Tokyo 2 — a hole in the parapet exactly where
  the branches meet. This survives nominal class widths swallowing a parallel sidewalk
  whole (primary is 16 m by default — a "covered by a neighbour ribbon" rule
  would open *both* of the pair's outer curbs there). The probe asks a **spatial
  hash of the bridge ribbons** (`BridgeBands`, 32 m cells — the
  `planting/index.rs` idiom), never a linear scan of every bridge way: curb tiles
  grow with the number of bridges, so the scan was the quadratic part of the fill
  (London carries 499 bridge ways against Tula's 61). The index is exact, not
  approximate — a segment goes into every cell of its AABB grown by its own
  `curb_reach`, so a single cell lookup can never miss a covering ribbon. A **joining** non-bridge
  road opens the curb its panel covers — joining means sharing a node with a
  bridge way (`JOIN_EPSILON`); a riverbank path passing a few metres *under*
  the span shares no node and opens nothing. The rule only refrains from
  blocking, it never re-opens — an open-by-rule tile over water stays water. After the
  deck carves a **seal pass** restores the barrier where it degraded to corner contact:
  on a narrow (alley-width) bridge the deck centerline chain claims the same tiles as
  the way's own curb chain, the deck wins, and the curb continues one column over —
  touching diagonally. Our A* cannot cut that corner but northstar's
  `OrdinalGrid` (HPA*, Theta*) steps straight through it, the thin-waterway
  hazard again. The seal blocks, of the two open orthogonal neighbours of such
  a diagonal pair, the one **farther** from the owning bridge centerline — the
  deck stays walkable, the gap closes on the outside. Known cost of a
  single-level grid: a street passing *under* a dry overpass gets the curb
  bands stamped across it — passable where the street runs, blocked either side
  of it.
- **Linear waterways block, unlike rails** — a `WaterLine` is water, and water is crossed
  by bridge, not waded. They carry the rail hazard (an unbroken thread across the
  city that `prune_unreachable` would amputate a bank of), so two things keep the map
  connected and both are load-bearing: the bridge carve runs *after* this fill, and
  **culverts do not block at all** (`WaterLine::tunnel`) — where a stream crosses under a
  street, OSM far more often pipes it (`tunnel=culvert`) than bridges the street over it.
  The number that proves it held is the **pruned-tile count** in the log — a jump of
  thousands means a watercourse cut a district off, and that is the thing to check after
  any change here or in `water_class`. On Tula (25 waterways, 7 of them culverts) the
  channels took 6 461 tiles out of the passable set and pruning did **not** move at all
  (9 781 before and after), i.e. no bank was severed. Much of that is free: the Упа's
  *centerline* is also tagged `waterway=river`, and it runs inside the Упа water polygon,
  which was already impassable. The one place the fill stops short of the capsule is the
  **culvert portal**: `set_polyline_capped` drops the tiles past the end plane there, so
  the mouth of the pipe — the only dry crossing a channel has — is not plugged by the
  half-disk. Same `water_line_caps` rule the ribbon is drawn with.
- **A rasterized polyline is a 4-connected chain, by construction.** `set_polyline` marks
  tiles whose *center* is within half the width — and that alone is not a barrier: below
  `tile_size · √2` (2.83 m at the default 2 m tile) a slanted band degenerates into tiles
  touching only at their **corners** (the navmesh overlay draws it as a chequerboard
  along the line). Our own A* cannot step through that — it does not cut corners — but
  every other consumer can: `bevy_northstar`'s `OrdinalGrid` (HPA*, Theta*) is built
  with no corner-cutting filter and steps diagonally between two blocked tiles, and
  `line_of_sight` samples points along a ray and slips through the contact point. A
  2.5 m stream was crossed by pawns on HPA* for exactly this reason. So `set_polyline`
  also walks the centerline tile by tile (Amanatides–Woo; each step crosses one grid
  line, so consecutive tiles share an *edge*). Raising narrow widths to a minimum
  instead was tried and rejected: the threshold depends on the line's angle and on its
  offset against the grid, and even 3 m still left a gap. Pinned by
  `tests/navigation.rs`.
- **Ordinary roads do not touch the navmesh.** The grid starts all-passable and
  `fill_from_mapdata` only ever *subtracts* (water, buildings, walls); roads enter it
  solely through the `bridge` and `passage` carves above. Pawns walk on grass and asphalt
  alike. Consequently road **rendering** (`map/roads.rs`, `MeshBuilder::push_ribbon`) and
  road **rasterization** (`Navmesh::set_polyline`, a capsule sweep by
  `distance_to_segment` — round joints by construction) are two independent code paths
  over the same `RoadLine`, and changing how a road is drawn cannot change where anything
  walks. Changing `RoadLine::points` or `width` would change both at once.
- **Rails do not touch the navmesh either, and deliberately so.** `MapData::rails` is
  absent from `fill_from_mapdata` by design, not by omission (`tests/navigation.rs`
  pins it): a rail line runs unbroken across the whole city, so blocking it would slice
  the map in two and `prune_unreachable` would amputate whichever half does not hold the
  portal. Pawns cross the tracks as if they were ground.
- **Building passage** (арка) — a road that runs *through* a building: OSM
  `tunnel=building_passage`, or `covered=building_passage|yes` (both tag styles occur;
  `tunnel=yes` is an underground tunnel and is **not** one). `parse/tags.rs::is_building_passage`
  sets `RoadLine::passage`; the navmesh carves those centerlines passable **last**, after
  buildings and walls, since the whole point is to punch through a block that was just
  filled. Carve width is `min(road width, PASSAGE_MAX_WIDTH)` — the way is usually tagged
  `service` (5 m) but the arch itself is narrower, and an uncapped corridor would eat a
  tile of facade on each side. Tula has ~70 of them, London ~1700; without the carve,
  courtyards reachable only through an arch get sealed off by `prune_unreachable`.
  How the arch is drawn (the wall opening) is in the osm-map skill.
- **Row-span rasterization** (`row_spans`): an area is filled row by row — one pass over
  the ring per tile row yields the x-crossings, and the tiles between crossing pairs are
  set. Holes are subtracted as intervals, *not* merged into one even-odd list, so a hole
  poking outside its outer ring still subtracts instead of filling. Replaced a
  point-in-polygon test per tile of the AABB, which on London's Thames (huge bbox × long
  ring) cost 6.3 s of a 6.5 s fill; now 30 ms.
- **prune_unreachable** — BFS flood from the portal tile; passable-but-unreachable
  pockets (enclosed courtyards, islands) become impassable. Reason: an A* request to an
  unreachable target floods the whole reachable region (tens of ms each); before pruning
  this once piled up a 12 000-request backlog and humans "froze". 4-connectivity matches
  A* reachability because of the no-corner-cutting rule.
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>` resource; async A* tasks read it off-thread.
  Starts empty (all passable), filled and pruned by the map-load thread while the loader
  screen is still up (`JobState::BuildingNavmesh` / `Pruning`).
- **PortalPos** (resource) — actual portal position. `PORTAL_POS` in settings is only a
  **hint**; `snap_portal_position` spirals out to the nearest tile with clearance derived
  from `PORTAL_DIAMETER`, through the shared `nearest_tile_where` ring search — nearest is
  Euclidean, so the first ring with a hit is not the answer (a corner at `r·√2` loses to a
  straight tile of a later ring), capped at `PORTAL_SEARCH_METERS` = 400 m (`settings.rs`,
  next to `PORTAL_DIAMETER`; metres, not tiles, so the navtile cycler does not move the
  portal). Nothing clear inside the cap ⇒ `None`, and `map/osm/download.rs` logs
  `no clear spot for portal` and keeps the raw hint — the prune flood then starts from the
  unsnapped point. The map-load thread snaps it between fill and prune (the flood
  starts from the snapped position) and hands it back in `LoadedWorld`; `poll_job` inserts
  the resource before switching to `Playing`.

## Viewport — the value the gates ask

**`Viewport`** (`camera.rs`) — the piece of the world in frame, as a value: `centre`,
`half_extent` (margin already applied) and `zoom` (world metres per logical pixel).
`Viewport::of(window, camera_transform, screens)` is the single place that computes
`window/2 · zoom · screens`. What a gate asks it: `contains` (**the edge counts as inside**),
`min` / `max`, `distance_from_centre_squared`. Not Bevy's `Camera::viewport` — that one is in
pixels.

**Five gates use it and each keeps its own margin on purpose**, because each asks a different
question:

| gate | margin | why |
|---|---|---|
| warmup (`loading.rs::poll_warmup`) | 1.0 screens | counts what the player can actually see |
| pathfinding dispatcher (`movement/`) | `VIEW_MARGIN` 1.2 | pawns at the edge must not stall on a small camera move |
| separation (`movement/separation/`) | `VIEW_MARGIN` 1.2 | same reason |
| movepath gizmos | 3.0 | debug drawing, wants context around the frame |
| door gizmos | 1.5 | ditto |

**Do not unify them.** The warmup margin in particular is deliberately the strictest: it
counts what the player can actually see, not what the dispatcher is willing to serve early.

## Backends & the pipeline

- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  in the Nav tab: A* / Dijkstra / Fringe / BFS (all from the `pathfinding`
  crate over the navmesh) plus **HPA*** and **Theta*** (hierarchical, from
  `bevy_northstar`). IDA*/IDDFS are deliberately excluded (never finish on open grids).
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/bench/pathfinding_bench.rs`
  (1.3 ms vs 36.4 ms mean, 15 ms vs 450 ms worst case) at ~10% longer paths. The other
  five stay switchable for comparison.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
  once from the final navmesh (after pruning; chunk sized to 50 world metres — see *Navtile
  size*), wrapped in `Arc`, called directly
  from async tasks — the crate's plugin is not used. Long paths cost ~0.5 ms vs ~40 ms for
  flat A*. The build takes **~12 s** on the 5600 × 3700 map, so it runs as an
  `AsyncComputeTaskPool` task started on `OnEnter(PlayPhase::Live)` **in the live branch
  only** (the deterministic branch starts it earlier — next bullet) and picked up by
  `poll_northstar_build`; until it lands, `NorthstarGrid::get()` is `None` and the
  dispatcher **falls back to flat A\*** for HPA*/Theta* requests. Doing it inline cost
  11 s of frozen loader screen; starting it before the warmup ends made it fight the
  warmup's A* for cores through rayon (85 ms per search instead of 36 ms).
  **Only the selected backend is built** (`northstar_wanted`): the hierarchy serves
  HPA*/Theta* on the grid, so with `Algo: Polymesh`, or with a flat grid algorithm
  picked, those 12 s of every core are skipped entirely (measured on Tula: no
  `northstar grid built` line at all). Switching `Algo` back to `Navmesh` or cycling
  `Pathfind` to HPA* starts the build right then — the same lazy shape the polygonal mesh
  has, run from `Update` on a resource change.
  **Both ends of a request are bounds-checked against the grid's own dimensions**
  (`grid_point` → `OrdinalGrid::in_bounds`) before `pathfind`, and out of bounds is a
  silent `None` — same verdict the grid gives ("out-of-bounds reads impassable"). Without
  it the crate logs a `log::error!` per call, at dispatcher rate, for a pawn past the map
  edge. The size is asked of the grid, not of `settings::grid_size()`: the grid is built
  from a navmesh snapshot and outlives a navtile-size change.
- **Where the build starts is a mode branch — three registrations of
  `start_northstar_build`** (`navigation/mod.rs::NavigationPlugin`, all
  `run_if(northstar_wanted)`): `OnEnter(PlayPhase::Live)` in `SimPipeline::Live` (after the
  warmup, for the core contention above); `OnEnter(PlayPhase::Warmup)` in
  `SimPipeline::Deterministic` (before it); and `Update` on
  `resource_changed::<PathfindingAlgorithm>`/`<PolymeshDebug>` under
  `in_state(PlayPhase::Live)`, in no set and therefore in both modes — the lazy re-start on
  a panel switch. **Putting the deterministic run on the live site would hang the loader
  forever**, not merely pick a worse backend: `poll_warmup` returns early while
  `Pathfinder::mode().is_building()` holds, and `is_building()` covers
  `HierarchyPending { .. }` — including `wanted: true`, "not started yet". That early return
  sits before the `elapsed` increment, so `WARMUP_TIMEOUT` can never fire, and the phase
  whose `OnEnter` would start the build is exactly the one the warmup is holding:
  "Building navigation..." with nothing building. The core-contention argument does not
  carry over either — in this mode the warmup computes nothing (`FixedUpdate` is paused,
  and there is no pawn warmup), so waiting the build out *is* its job. Without the wait the
  frozen `Backend` taken at `WorldStarted` would carry `hierarchy: None` and the whole run
  would go on flat A* while the panel reads HPA*. The polymesh needs no such branch:
  `sync_polymesh_build` already runs in `OnEnter(AppState::Playing)`
  (`WorldInitSet::Spawn`), before the warmup in both modes. A restart on R crosses no
  `OnEnter` (`restart.rs::on_restart`), so no site fires and the previous run's hierarchy is
  reused — that is the determinism skill's "restarts do not pay it".
- **Backend** (`navigation/backend.rs`) — the active backend as one cloneable `Send`
  snapshot: the grid navmesh Arc + the selected algorithm (+ the northstar hierarchy once
  built), or the polymesh overriding all of it. It is **the resource the whole simulation
  reads** (`Res<Backend>`), seeded by `insert_backend` on `OnEnter(Playing)` — the live
  dispatcher already runs during warmup, before any `WorldStarted` — and dropped on
  `OnExit(Playing)` so the old city's `Arc`s go with it. **It has no `Default`, on purpose**:
  the placeholder used to be an empty everywhere-passable grid, and a run once went through
  buildings on it (`determinism/replay.rs`). An absent resource is loud — param validation
  fails and the default Bevy error handler panics (tests and the replay scene set `warn`) —
  unlike a silently wrong geometry. The cost is that **every system taking `Res<Backend>`
  must sit in a `SimPipeline` set**, which carries the world gate (`in_world`) along with the
  mode branch: the live pathfinding chain running in `Loading` crashed the app once.
  `Pathfinder::backend()` takes the live
  snapshot, handed to the simulation as the `Res<Backend>` resource; the deterministic
  mode writes it once on `WorldStarted` and freezes it for the run, while the live mode
  re-takes it every frame in `PreUpdate` (`refresh_backend`); `spawn_path_task` moves it into the
  async task, so both dispatchers share `Backend::search` and the modes can only differ
  in WHEN an answer is collected, never in what is computed. The collecting side is
  shared the same way: both receivers call one `accept_answer` (tally + apply), so what
  is genuinely per-mode shrinks to *how the answer is obtained* — `check_ready` with the
  hang watchdog in the live path, `block_on` charged to `SimLoad` in the deterministic
  one — plus the traversal order. The `answered`/`failed` pair is written by
  `AnswerTally`'s **`Drop`**, not at the end of the system: "both counters every run,
  including zeros" is what makes the panel's ratio-of-means valid, and discipline lost it
  twice (measurements only on frames with answers, then an `if due.is_empty() { return; }`
  in the deterministic receiver — both left the panel frozen on its last value).
  Pinned by `movement/tests.rs`, one test per receiver. `Backend::walkable()`
  returns the passability view `Walkable` (read lock taken once per system run — lazily
  in the collectors, where it is only needed under rescue): `allows` /
  `nearest_free_point` are backend-strict (grid first as the cheap test, then the
  agent-radius-inflated mesh), while `sift_target`, `line_of_sight` and `coast_allows`
  are deliberately grid-only — hot-loop cost, documented on the methods; switching one to
  the mesh-strict test is a one-line edit there, not a sweep over four behavior systems.
  Boundary invariant: outside `navigation/` and `ui/`, the names `PolymeshBuild` /
  `PolymeshDebug` / `PathfindingAlgorithm` do not appear. It was breached for two years'
  worth of commits by `determinism/replay.rs`, which had to name both resources to say
  "run on the flat grid"; the door for that is now `use_flat_grid(&mut World)`, which also
  seeds the two resources `Pathfinder` merely needs to validate — the reason the
  `determinism` test benches used to list four navigation names apiece.
  `ContinuousSpace` is the
  separation gate's question ("are paths metric polylines?") answered by the polymesh
  *toggle*, not build readiness — blinking separation off during the 0.3–20 s build was
  judged worse than finishing the transition on the grid.
- **NavMode** (`navigation/mode.rs`) — the backend question as a value, and the only place
  the four resources are combined (`Pathfinder::mode`). `Grid(Flat | HierarchyPending {
  wanted } | Hierarchy(g))` × `Mesh(Pending | Ready(b))`; the methods are the questions
  that used to be asked separately — `is_building`, `northstar_wanted`,
  `mesh`/`hierarchy`. The "is the mesh toggled on" question is deliberately not a table
  method: its consumers read the toggle itself (`ContinuousSpace` above for the
  separation gate — `MovementPlugin` is raised in tests without navigation, so it cannot
  see a `NavMode` — and the panel sections directly, inside the `PolymeshDebug`
  visibility boundary). The truth table is pinned by two tests in the
  module, which is the point: before, each column was assembled in a different file from a
  different combination of resources, and there was nowhere to compare them.
  `HierarchyPending { wanted }` is what keeps `northstar_wanted` honest — `wanted` is
  "not started yet" (`NorthstarGrid::is_missing`), so the 12-second build is still ordered
  exactly once. **`Mesh(Pending)` is the state that did not exist before**, and its absence
  was expensive: `polymesh_build()` collapsed "off" and "building" into one `None`, so a
  consumer needing the difference — `crowd_demo`'s overlay label — rebuilt the question
  from raw resources, with a comment admitting it. A value rather than a resource on
  purpose: the loader gate must read the live situation even in a
  deterministic run, where the `Backend` snapshot is deliberately frozen.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
  `Movable::to_pathfinding` only queues a `PathfindingRequest`;
  `dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks
  (polled with `check_ready`). **Visibility gating**: peacefully wandering humans
  OUTSIDE the camera view (×1.2 margin) are never dispatched — their requests wait
  until the camera arrives; at zoom ≥ `WANDER_DISPATCH_MAX_ZOOM` (0.75 m/px) *no*
  wanderer counts as on screen — a pawn is a dot there, and "in view" would otherwise
  mean half the map, flooding the task pool and the per-frame sort with ~17k peaceful
  requests. Demons and fleeing humans are always dispatched at any zoom.
  **Priority** (`priority::` in `movement/pathfinding.rs`): demons and fleeing humans
  (`URGENT`) go before wandering humans in frame (`WANDER_ON_SCREEN`), within a
  priority nearest-to-camera-center first, capped at `MAX_PATHFINDING_IN_FLIGHT`
  (1024, `settings.rs` — sized for the 30× repath rate, see its doc). The order only bites
  when the cap binds — in normal play in-flight sits around 100. The speed panel shows
  in-flight / queued / avg ms. Gate and priority together are one pure function,
  `pathfinding.rs::queue_key(&Viewport, position, urgent) -> Option<(u8,
  f32)>` — the whole rule as a value, so the four branches (urgent off-screen,
  wanderer in frame, wanderer off-screen, wide zoom) are tested without a camera or a
  window; the system around it is only queue assembly and the budget truncation.
  **Who is urgent is not this module's business**: the species write
  `movement::UrgentPath` and the dispatcher asks `Has<UrgentPath>`. It used to derive
  urgency from `Has<Human>` + `Has<HumanFleeTag>`, which put one species' name in seven
  movement queries and spread the rule over three modules in three polarities — and the
  third copy, in `loading::poll_warmup`, had already drifted to its own view margin.
  **A request is cancelled only by `to_idle`** (`Movable::stop_moving` removes it together
  with `RequestedAt`) or consumed by dispatch — nothing else takes it off. Leaving one on a
  pawn that has gone `Idle` therefore buys a full A* whose answer `accept_answer` drops on
  its first check, and the calm-down branch of `human::behavior::flee` used to do exactly
  that for hundreds of pawns a tick.
  **A re-target replaces the request, it does not overwrite it**: `to_pathfinding` removes
  `PathfindingRequest` and `RequestedAt` and inserts a fresh request in the same command
  chain. Both consumers — `stamp_pathfinding_requests` and `assign_destination_slots` —
  filter on `Added<PathfindingRequest>`, and `insert` over a live component arms only
  `Changed`, so a pawn re-targeted while queued used to keep the FIFO key of its first
  goal and got no slot for the new one, still holding a `DestinationClaim` on the
  abandoned point. The window is not exotic: a wanderer's request waits for the camera in
  live mode and sits in a population-long queue in the deterministic one, and panic
  re-targets right over it.
- **Repath on the move** — `to_pathfinding` keeps the current path and the
  `MovableStateMovingTag`, so an entity walks its old path while the new one is
  computed; `MovableStateMovingTag` therefore means "has a path **or is coasting**",
  *not* "state is `Moving`". Dispatch and pickup both live in `Update`, so a reply
  costs 2–3 frames — and a frame carries `speed × 1/fps` virtual seconds, so at 30×
  the reply lags by 1–1.5 virtual seconds while a fleeing human repaths every ~1 s:
  the old path routinely runs out before the reply lands.
- **The step itself** (`movement/step.rs::step_along_path`) — all five rules of one
  pawn-step (arrive, coast, hold, steer, slide) as one function over plain values:
  `&mut Movable`, a position, `dt`, the pawn's `StepModifiers` (its share of the
  separation output), the pawn's `StepTuning` (rest distance from its `BodyScale` — the
  step names no species), and a `Walkable`. It returns a
  `StepOutcome` (`Moved` / `Arrived { destination_reached }` / `Halted`) and touches no
  `Entity`, `Commands` or query — the same shape as `human::decide` / `demon::decide`.
  `move_moving_entities` is the plumbing around it: query, commands, the human grid.
  Both course changes the step makes on its own are gated on passability, which is what
  the `Walkable` argument is for: coasting **halts** on an impassable tile, steering is
  **dropped for that tick** and the pawn walks its plain course. `slide` is not gated —
  it is a lab knob, off by default, and only ever shortens the step.
  Tested by value in `movement/step/tests.rs`, which is how `slide` and `steer_release`
  got covered at all — both are lab knobs, off by default, so a whole-`App` test could
  not reach them.
- **Coasting** (`step_along_path`) — an entity whose path is exhausted while the
  state is still `Pathfinding` keeps moving along `Movable::last_direction` (the
  direction of its last step) as long as the tile ahead is passable; a zero vector,
  a wall or the map edge ends the coast (tag removed, as before). Arrival is not
  coasting: state `Moving` + empty path still means `to_idle` — a wanderer must stop
  at its destination. Before coasting, 26–42% of fleeing humans stood at any instant
  at 30× (measured); the reply (`to_moving`) or `PathfindingError` ends the coast.
  When the reply lands, up to `REPATH_TRIM_LIMIT` (`settings.rs`, 4 — coasting drifts 4–6 tiles off
  the request's start tile at flee speed) leading waypoints are dropped while the
  next one is no further than the first — without the trim the first step would be
  backwards; each drop is geometry-gated, the limit only guards corner-straightening.
- **find_passable_tile_near** — the target tile or its 8 neighbors only; callers must
  tolerate `None`.
- **Rescue** (`movement::rescue_from_impassable`) — a pawn standing on an impassable tile
  moves to the nearest free point (`Walkable::nearest_free_point`), both ends of the
  interpolation are set to the new point and the stale path is dropped (`to_idle`).
  **Where it moves to is two steps, not one.** On the mesh backend the snap comes first
  (`PolymeshBuild::nearest_free_point` — polyanya's `get_closest_point` within the
  0.75 m localisation tolerance, the same snap that seats the ends of a request): a
  pawn caught by a rebuild with a larger agent radius stands centimetres from free space,
  and a ring of tiles with a BVH query per candidate would cost three orders more than one
  snap. Only when the snap misses — the pawn is not against a wall but deep inside a
  house — does the shared `nearest_tile_where` ring search run, capped at
  `RESCUE_SEARCH_TILES` = 16 tiles (32 m); on the bare grid it is the only step. So the
  landing point is a mesh point, not necessarily a tile centre.
  Ways in exist by construction: the
  spawn sifts tiles but stands the pawn on a tile centre whose own corner may already be
  inside a house (fill marks a tile by its centre), the polygonal mesh calls passable
  what the grid does not (contours inflated by the agent radius), coasting and the demon
  lunge move `SimPosition` past the path. Fixing each entrance separately is pointless —
  the end state is one and the same, and it is terminal: behaviour picks a target, the
  search finds nothing, behaviour picks another, forever.
  **The trigger is a failed search**, not a clock (`listen_for_pathfinding_tasks`, the
  `result.path == None` branch). That is the only signal a stuck pawn raises about
  itself, and it selects exactly the ones worth rescuing: flat A* does not test the start
  tile at all, so a pawn a tile or two inside a house walks out on its own successors;
  polyanya snaps the start onto the mesh; `None` comes back only when there is really no
  way out — all eight neighbours impassable, or a start belonging to no chunk of the
  northstar hierarchy. Cost is one index into the passability `Vec` per failed answer
  (~45 a frame on Tula at 31 % failures), and the ring search runs only for those
  actually walled in. A periodic scan over all 20 000 pawns would do the same work
  thousands of times over for nothing.
  **What counts as free is the active backend** (`Backend::walkable()`): the grid tile first (an
  index into a `Vec`), and — while the polygonal mesh is built and selected —
  `PolymeshBuild::contains`, a layer-hinted `point_in_mesh`. The mesh is the stricter of
  the two: its contours are inflated by the agent radius, so a tile that clears the grid
  can be inside an obstacle on the mesh. The same `allows` re-checks the mesh snap of the
  step above (`.filter(|&snapped| self.allows(snapped))`), so the mesh answer can never
  land a pawn on a blocked grid tile — a failed re-check falls through to the ring search.
  `rescue_trapped_entities` is the same check as a full pass, and it runs in exactly one
  place: **every completed mesh build** (`polymesh_rebuilt` watches
  `PolyNavmesh::generation` — `resource_changed` would also fire when a build merely
  starts). That is the only moment passability changes under pawns already standing. It
  logs `rescued N entities` with its own duration when it moves anyone.
  It is also the **head of the `Update` movement chain** — explicitly `.before`
  `human::pick_wander_targets` and, through the chain, before
  `dispatch_pathfinding_requests` (`movement/mod.rs`). The teleport rewrites
  `SimPosition` and drops the pawn to `Idle`, which is exactly what those two read in
  the same frame: reaching the dispatcher first buys a full A* from the pre-teleport
  `start_tile`, and `accept_answer` drops that answer on its very first check — one
  budget slot per rescued pawn, on the one frame per city where they appear en masse.
  Rescued in front of the picker, the pawn files its next request from the new
  position in the same frame instead of standing one frame out.
  There is deliberately **no scan on entering the world**, though it looks called for: by
  then the grid is final and `spawn_population` picked its tiles with the very same
  `is_passable`, so the pass would re-test the predicate the spawn had just applied and
  could not find anyone (measured live on Tula: zero rescued). No mesh exists at that
  moment either — its build is async and starts in the same `OnEnter`, and the previous
  city's mesh is dropped by `city::reload`.
- **tiny_city / parity tests** (`map/osm/fixture.rs`, `navigation/parity_tests.rs`) —
  one `MapData` fixture with a zone per fill-rule class (river + only bridge, culvert,
  full-height wall building with an arch wider than `PASSAGE_MAX_WIDTH`, dry-span
  bridge with curbs and a joining street sharing a node, water multipolygon with an
  island hole opened by its bridge — the Paris start-area incident class). The parity
  tests build BOTH fills from it and assert identical verdicts on reachability pairs
  and single-point probes; the grid side runs fill + `prune_unreachable` from the
  `portal` landmark because the mesh drops unreachable pockets at build time. Probes
  stand farther from edges than one navtile and the agent-radius inflation, so the
  tests compare fill rules, not discretisation. ~50 ms per test — they run in every
  `cargo test`, unlike the offline audits that need a downloaded city.
- **pathfinding_bench** (`examples/bench/pathfinding_bench.rs`) — offline comparison of all six
  algorithms without booting Bevy: reads the OSM cache, rebuilds the navmesh exactly as
  the map-load thread does (fill → `snap_portal_position` → prune), generates one
  seeded task list mirroring human wander (80% random building, 20% short stroll) and
  replays that *same* list per algorithm across a shared atomic work cursor. Reports
  wall / cpu / avg / p50 / p95 / max and mean path length. Run it after touching
  `successors`, costs, or the navmesh fill.
