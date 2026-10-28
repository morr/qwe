---
name: navigation-deep
description: Use when working on navigation or movement internals in qwe — the navmesh fill (bridge curbs, waterways, passages), pathfinding backends (A*/HPA*/northstar, polymesh), the request/dispatch/task pipeline, repath/coasting, rescue, pawn separation and destination slots. Deep detail behind CONTEXT.md's Navigation and Simulation sections.
---

# Navigation & movement — deep detail

This is the detail layer behind the **Navigation** and crowd parts of the
**Simulation** summaries in `CONTEXT.md`. The invariants (fill order, what blocks and
what doesn't, no corner cutting) live there; this file is the map — what exists, where
it lives, which reference carries the mechanism.

Four deep dives live next to this file and are read on demand:

- `references/grid-fill.md` — the grid navmesh fill mechanics: fences and their gaps,
  default gates, bridge curbs, waterways and culverts, passages, row-spans, the portal
  snap.
- `references/pipeline.md` — the dispatcher and backend internals: the northstar build
  and its mode branch, `Backend` collection, `NavMode`, the in-flight cap.
- `references/polymesh.md` — the polygonal navmesh: build, chunking/seams/stitching,
  corridor + string-pulling, routing, the divergence fixes and the vendored polyanya.
- `references/crowd.md` — pawn separation (all rules and their measured reasons, the
  separation lab) and destination slots.

When a change here introduces or retires a concept, update the matching summary bullet
in `CONTEXT.md` and the detail here in the same change.

## Navtile size

The navtile is **2 m by default and runtime-switchable to 1 m** via the `navtile:` cycler in
the Debug tab (`NavtileBase` in `src/grid.rs`, persisted in prefs). Switching it reloads the
world like a city switch, except the camera stays where it was — same city, same spot under
inspection.

**The live value is a process-global atomic**, read by `grid::navtile_size()`: the
background navmesh fill has no ECS access. The parse, where the same size is the door
generator's clearance, does **not** read it — it gets `ParseKnobs::navtile` as an
argument (osm-map `references/parse.md`). It is written only in
`OnEnter(Loading)`, before the load thread starts.

Grid size is derived as `MAP_SIZE / navtile_size()` (3800 × 2850 tiles at 2 m). **A filled
`Navmesh` carries its own `grid_size` / `tile_size` snapshot**, so a stale snapshot (a
cancelled northstar build) never indexes against the switched atomic. The snapshot owns the
conversions too: rasterisation (`set_area`/`row_spans`, `visit_polyline*`,
`visit_segment_tiles`), the navmesh-side queries (`line_of_sight`,
`snap_portal_position`), `prune_unreachable` (which takes a **world point**, not a tile,
for exactly this reason), everything `Walkable` asks (`allows`, `coast_allows`,
`nearest_free_point`), the tile→world step of `Backend::search`, and the grid overlay
(`ui/debug/overlays.rs::sync_navmesh_overlay`) all go through `Navmesh::to_tile` /
`Navmesh::tile_center`, never through `grid::world_to_tile`, which reads the atomic.

**The northstar chunk scales with the tile** to stay 50 world metres — 25 tiles at 2 m, 50 at
1 m. With the chunk pinned at 25 tiles a 1 m build explodes from ~14 s to ~140 s.

Cost of 1 m, measured: northstar build ~14 s vs ~11 s, HPA* ×1.7 CPU, +1.6 GB RSS. A change
to the navtile size is simulation input — it breaks a replay in flight (`determinism`).

**`src/grid.rs` owns the navtile**, and that is its whole reason for existing as a module of
its own: `DEFAULT_NAVTILE_SIZE`, the atomic and its `navtile_size()` / `set_navtile_size()`
(the setter is `pub(crate)` — the one safe writer is `loading::sync_navtile_size`),
`grid_size()`, the `NavtileBase` cycler, and the global conversions `world_to_tile` /
`tile_center` for callers with no `Navmesh` in hand (movement, wander, the door overlay);
those read the atomic. None of it is in `settings.rs`: the navtile is not a knob among the
world's knobs but the scale everything navigational is built in, and it has one owner.

## The grid navmesh

- **Navmesh** (`navigation/navmesh.rs`) — keeps only the type: the `Vec<bool>` passability
  grid, its `grid_size`/`tile_size` snapshot, `is_passable`/`set_passable` and the
  `to_tile`/`tile_center` conversions, plus `ArcNavmesh`. The storage contract (index,
  out-of-bounds reads impassable, 8-way `successors` with **no corner cutting**) is a
  `CONTEXT.md` term.
- **Where each rule lives.** Everything done *to* the grid is a submodule of
  `navigation/navmesh/`, one per role, each with its own `tests.rs` beside it:
  **`raster.rs`** — rasterisation (`set_area` and the row spans, `set_polyline*`,
  `visit_polyline*`, `visit_segment_tiles`); **`fill.rs`** — the fill from `MapData`
  (`fill_from_mapdata`, `fill_base`, `fill_fences`, `carve_passages`, `BridgeBands`);
  **`gates.rs`** — the default gates (`open_sealed_fences`, the door groups and the
  `SEALED_POCKET_MIN_AREA` / `GATE_ROUNDS` / `GATE_SPACING` knobs); **`reach.rs`** —
  reachability (`flood`, `neighbours`, `component`, `successors` with its step costs,
  `passable_from`, `prune_unreachable`, `open_gates_and_prune` + `GatesAndPrune`). They are
  all `impl Navmesh` blocks, so every method keeps its name and its visibility — what the
  split changes is the file you open, never the call.
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** (`bridge=yes` roads) → buildings block → walls block → **fences block,
  minus their gaps** → **building passages carve back through them**. One line per rule;
  the mechanics of every one — `references/grid-fill.md`:
  - **fences and their gaps** (`fill_fences`, `footprint::fence_gaps`) — the gap kinds,
    the mask-cut-not-carve idiom, the drawn-vs-blocking fence;
  - **default gates** (`open_sealed_fences`, stored in `MapData` as `FenceLine::gates`) —
    the pocket/door rule, the street-side pick, the gate rounds and spacing;
  - **bridge curbs and deck carves** (`fill_base`, `BridgeBands`) — the outward ownership
    probe, the carve short of the curb, the seal pass;
  - **linear waterways, culverts exempt** (`WaterLine::tunnel`) — the culvert portal cap;
  - **a rasterized polyline is a 4-connected chain** (`set_polyline`) — the centerline
    walk and why;
  - **building passages** (`PASSAGE_MAX_WIDTH`) — carved last, width-capped, end-extended
    half a width in both fills;
  - **row-span rasterization** (`row_spans`) — holes subtracted as intervals;
  - **ordinary roads and rails never touch the grid** — the fill only subtracts; rendering
    and rasterisation are independent paths over one `RoadLine`;
  - **prune_unreachable and the portal snap** (`reach.rs`, `snap_portal_position`) —
    4-connectivity, the ring search and its cap.
- **PortalPos** — a `CONTEXT.md` term (the city's `*_PORTAL_POS` is only a hint); the
  snap mechanics — `references/grid-fill.md`.

## Viewport — the value the gates ask

**`Viewport`** (`camera.rs`) — `Viewport::of(window, camera_transform, screens)` is the
single place that computes `window/2 · zoom · screens`; a gate asks it `contains`,
`min` / `max`, `distance_from_centre_squared`. What the value *is* — a `CONTEXT.md` term
(not Bevy's `Camera::viewport`, which is in pixels).

**And a resource.** `ViewportPlugin` runs `sync_viewport` once per frame in
`RunFixedMainLoopSystems::BeforeFixedMainLoop`, `.before(sim_time::begin_sim_load)` — so the
frame is computed before the sim-load bracket opens, and it is in place before both
`FixedUpdate` and `Update` run. The resource holds **exactly the frame** (`screens = 1.0`),
with nobody's margin in it; a gate adds its own with `Viewport::with_margin(screens)`.

- **It has no `Default`, on purpose** — the `Backend` precedent: a placeholder frame would
  quietly claim half the map is on screen and every gate would pass. A stand that raises
  these systems inserts the resource itself (`crowd_demo` adds `ViewportPlugin`,
  `tests/movement.rs::test_app` inserts a `viewport_at(..)` constant), and `sync_viewport`
  inserts it on the first frame in the game.
- **Known cost**: when the camera is gone (leaving the world despawns it) `sync_viewport`
  is skipped and the readers get **last frame's** frame with no signal. Before this each
  of them was skipped silently on its own for the same reason, which was not better.

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

**None of them asks for the camera any more** — they take `Res<Viewport>`. The one place
that still does is `sync_viewport`, and there the `PanCamera` half of `Single<&Transform,
(With<Camera2d>, With<PanCamera>)>` is load-bearing: a `dev::OffscreenShotEvent` raises a
second `Camera2d` for three frames, and a `Single` matching two entities makes the executor
skip the system **silently** — which used to mean three frames with no dispatch and, at 30×,
dozens of ticks with no separation, every time a screenshot was taken. Now it means three
frames on a slightly stale frame, which nothing can see. The filter is still what keeps the
offscreen camera out; a headless world that raises `ViewportPlugin` must put `PanCamera` on
its camera for the same reason it spawns a `PrimaryWindow`. A stand with no camera at all
inserts the resource instead.

## Backends & the pipeline

- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  in the Nav tab; **default is HPA\***, the rest stay switchable for comparison; IDA*/IDDFS
  are deliberately excluded (never finish on open grids). The numbers and the fallback —
  `references/pipeline.md`.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
  once from the final navmesh, wrapped in `Arc`, called directly from async tasks — the
  crate's plugin is not used. Until it lands the dispatcher **falls back to flat A\*** for
  HPA*/Theta* requests; **only the selected backend is built** (`northstar_wanted`). Where
  the build starts and why — `references/pipeline.md`.
- **Backend** (`navigation/backend.rs`) — the active backend as one cloneable `Send`
  snapshot: the grid navmesh Arc + the selected algorithm (+ the northstar hierarchy once
  built), or the polymesh overriding all of it. One snapshot, two writers (live re-takes
  per frame, deterministic frozen at `WorldStarted`), one collector — the write/collect
  mechanics, the answer diagnostics, the `SimPipeline` residency rule —
  `references/pipeline.md`.
- **NavMode** (`navigation/mode.rs`) — the backend question as a value, the only place
  the four resources are combined (`Pathfinder::mode`); a value, not a resource, taken
  fresh by each consumer. The truth table and the mode branch of the northstar build —
  `references/pipeline.md`.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
  `Movable::to_pathfinding` only queues a `PathfindingRequest`;
  `dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks.
  **Visibility gating**: peacefully wandering humans OUTSIDE the camera view (×1.2
  margin) are never dispatched — their requests wait until the camera arrives; at zoom ≥
  `WANDER_DISPATCH_MAX_ZOOM` (0.75 m/px) *no* wanderer counts as on screen. Demons and
  fleeing humans are always dispatched at any zoom. **The live dispatcher stands while
  the player pauses** (`live_dispatch_runs`); the warmup pause is the exception and must
  stay one. **Priority**: demons and fleeing humans (`URGENT`) before wandering humans in
  frame (`WANDER_ON_SCREEN`), within a priority nearest-to-camera-center first, capped at
  `MAX_PATHFINDING_IN_FLIGHT` (1024, `settings.rs`). **Who is urgent is not this module's
  business**: the species write `movement::UrgentPath`, the dispatcher asks
  `Has<UrgentPath>`. **A request is cancelled only by `to_idle` or consumed by dispatch;
  a re-target replaces the request, it does not overwrite it.** The internals
  (`queue_key`, the pause and zoom economics, the `Added`-filter re-target rule, the
  repath reply lag) — `references/pipeline.md`.
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
  northstar hierarchy. The ring search runs only for those actually walled in.
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
  could not find anyone. No mesh exists at that
  moment either — its build is async and starts in the same `OnEnter`.
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
