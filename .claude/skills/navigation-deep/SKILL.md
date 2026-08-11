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

## The grid navmesh

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * GRID_SIZE.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
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
  is never walled across by its own curb. The deck carve is **narrower than the deck by
  a tile diagonal**: a curb-chain tile's center wanders up to half a diagonal (√2 m)
  off the curb centerline — i.e. *into* the deck on a slanted bridge — and a full-width
  carve re-opened those tiles, turning the barrier into a dashed line. Deck
  connectivity survives the narrowing because `set_polyline` always walks the
  centerline chain, the same guarantee thin waterways rely on. A curb tile does
  **not** block unconditionally — OSM cuts one physical bridge into several ways
  (carriageway and its sidewalk are parallel ribbons), so each curb tile records
  its owners (`CurbTile`) and the decision is an **outward probe**: step one
  tile away from the owning way's centerline — if that point lands inside
  another bridge way's ribbon, the tile is an interior seam and stays open; if
  it lands on nothing, the tile is the outer boundary of the whole composite
  and blocks. This survives nominal class widths swallowing a parallel sidewalk
  whole (primary is 16 m by default — a "covered by a neighbour ribbon" rule
  would open *both* of the pair's outer curbs there). A **joining** non-bridge
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
  `tunnel=yes` is an underground tunnel and is **not** one). `parse::is_building_passage`
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
  from `PORTAL_DIAMETER`. The map-load thread snaps it between fill and prune (the flood
  starts from the snapped position) and hands it back in `LoadedWorld`; `poll_job` inserts
  the resource before switching to `Playing`.

## Backends & the pipeline

- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  in the Navigation panel: A* / Dijkstra / Fringe / BFS (all from the `pathfinding`
  crate over the navmesh) plus **HPA*** and **Theta*** (hierarchical, from
  `bevy_northstar`). IDA*/IDDFS are deliberately excluded (never finish on open grids).
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/bench/pathfinding_bench.rs`
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
  **Only the selected backend is built** (`northstar_wanted`): the hierarchy serves
  HPA*/Theta* on the grid, so with `Algo: Polymesh`, or with a flat grid algorithm
  picked, those 12 s of every core are skipped entirely (measured on Tula: no
  `northstar grid built` line at all). Switching `Algo` back to `Navmesh` or cycling
  `Pathfind` to HPA* starts the build right then — the same lazy shape the polygonal mesh
  has, run from `Update` on a resource change.
- **Backend** (`navigation/backend.rs`) — the active backend as one cloneable `Send`
  snapshot: the grid navmesh Arc + the selected algorithm (+ the northstar hierarchy once
  built), or the polymesh overriding all of it. `Pathfinder::backend()` takes the live
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
  `PolymeshDebug` / `PathfindingAlgorithm` do not appear. `ContinuousSpace` is the
  separation gate's question ("are paths metric polylines?") answered by the polymesh
  *toggle*, not build readiness — blinking separation off during the 0.3–20 s build was
  judged worse than finishing the transition on the grid.
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
  (512). The order only bites when the cap binds — in normal play in-flight sits
  around 100 of 512. The speed panel shows in-flight / queued / avg ms.
- **Repath on the move** — `to_pathfinding` keeps the current path and the
  `MovableStateMovingTag`, so an entity walks its old path while the new one is
  computed; `MovableStateMovingTag` therefore means "has a path **or is coasting**",
  *not* "state is `Moving`". Dispatch and pickup both live in `Update`, so a reply
  costs 2–3 frames — and a frame carries `speed × 1/fps` virtual seconds, so at 30×
  the reply lags by 1–1.5 virtual seconds while a fleeing human repaths every ~1 s:
  the old path routinely runs out before the reply lands.
- **Coasting** (`move_moving_entities`) — an entity whose path is exhausted while the
  state is still `Pathfinding` keeps moving along `Movable::last_direction` (the
  direction of its last step) as long as the tile ahead is passable; a zero vector,
  a wall or the map edge ends the coast (tag removed, as before). Arrival is not
  coasting: state `Moving` + empty path still means `to_idle` — a wanderer must stop
  at its destination. Before coasting, 26–42% of fleeing humans stood at any instant
  at 30× (measured); the reply (`to_moving`) or `PathfindingError` ends the coast.
  When the reply lands, up to `REPATH_TRIM_LIMIT` (4 — coasting drifts 4–6 tiles off
  the request's start tile at flee speed) leading waypoints are dropped while the
  next one is no further than the first — without the trim the first step would be
  backwards; each drop is geometry-gated, the limit only guards corner-straightening.
- **find_passable_tile_near** — the target tile or its 8 neighbors only; callers must
  tolerate `None`.
- **Rescue** (`movement::rescue_from_impassable`) — a pawn standing on an impassable tile
  moves to the nearest passable one (`nearest_passable_tile`, ring search capped at
  `RESCUE_SEARCH_TILES` = 16 tiles), both ends of the interpolation are set to the new
  point and the stale path is dropped (`to_idle`). Ways in exist by construction: the
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
  can be inside an obstacle on the mesh.
  `rescue_trapped_entities` is the same check as a full pass, and it runs in exactly one
  place: **every completed mesh build** (`polymesh_rebuilt` watches
  `PolyNavmesh::generation` — `resource_changed` would also fire when a build merely
  starts). That is the only moment passability changes under pawns already standing. It
  logs `rescued N entities` with its own duration when it moves anyone.
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
