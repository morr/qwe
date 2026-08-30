# Polymesh — the polygonal navmesh and routing

Detail behind `navigation/polymesh/`. The summary lives in `CONTEXT.md` (Navigation).

## Poly navmesh

**Poly navmesh** (`navigation/polymesh/` — `build.rs` builds the mesh, `seams.rs`
computes the shared chunk borders, `stitch.rs` sews the layers together, `path.rs`
searches the result; Nav tab — `ui/navigation/`) — a *polygonal* polyanya mesh
triangulated from the same vector sources the grid fill rasterizes, recovering the
fidelity the 2 m grid loses (bridge curbs, narrow waterways). While the Polymesh backend
is on and the mesh is built, **it is the pathfinding backend** — see **Polygonal
routing** below. It is on **by default** (`PolymeshDebug::enabled`): the polygonal
search is the world's navigation, and the grid is the fallback (while the mesh builds,
and when the Nav tab is switched back to `Navmesh` by hand). `show` defaults
to *off* for the same reason — a default-on backend with a default-on overlay would
bury a fresh install's city under polygon edges.

The whole fill order collapses into one boolean (`i_overlay` difference):
union(water ∪ non-culvert waterways ∪ bridge curb bands ∪ buildings ∪ walls) −
union(bridge decks ∪ joining roads ∪ passages), where the areas' **ring holes enter the
union reversed** so NonZero subtracts them, clipped to the map rect **outset** by
`MAP_EDGE_MARGIN` (an inset clip would leave a walkable sliver along the map edge for
paths to sneak around a river; polyanya digests obstacles crossing its outer boundary —
triangle walkability is a point-in-polygon test of the triangle center), then CDT via
`polyanya::Triangulation` with agent-radius inflation.

**A ring hole is not an obstacle, but a hole of the *result* still is.** The input holes
are subtracted the way the grid's `row_spans` subtracts them (reversed contour + NonZero,
so a shed mapped inside a courtyard raises the winding back to +1 and stays solid); what
is dropped is a hole the *union* still has after the carves — `shape.first()` keeps only
outer contours, everywhere: the union output, the `obstacles` kept for the overlay, and
the per-chunk clip. That is exactly `prune_unreachable`'s verdict — a courtyard with no
arch is a pocket nothing reaches. The distinction is load-bearing for **islands**: Île de
la Cité and Île Saint-Louis are inner rings of the `natural=water` multipolygon "La
Seine", and Paris's portal hint (`MAP_CENTER_PORTAL_POS`) stands on the first of them.
Dropping the input hole buried the whole island — `contains(portal) == false`, every
street on it painted blocked by the overlay — while the grid walked it fine. Subtracting
it instead makes the island a hole of the water region, and the bridge decks then cut the
water ring open, so the island joins the outer contour. No bridge, no opening: an island
nothing reaches stays a dropped hole, as it should. Pinned by
`polymesh::tests::an_island_is_walkable_once_a_bridge_reaches_it`;
`examples/audit/polymesh_start_area` checks every city's portal hint against the mesh.

Deliberate deltas from the grid: the **diagonal seal pass has no analogue** (patches
raster corner-contact only), and deck/joining widths are **verbatim** — the grid's
`±tile·√2` corrections compensate wandering tile centers, which vectors don't have.
The deck carve is therefore `road.width`, the carriageway the renderer fills, *not*
the grid's `width + curb`: the curb bands live just outside the carriageway, and a
carve that wide eats half of the barrier it is supposed to leave standing.
The grid's **outward probe** — which curb tile is an interior seam of a composite
bridge (carriageway + its sidewalk way, mapped separately) and which is the outer
boundary — becomes the direct vector statement of the same intent: each way's curb
bands **minus the full drawn bands of every other bridge way**. Covered by a
neighbour ⇒ interior seam ⇒ open; uncovered ⇒ outer edge ⇒ blocks. N differences over
a few dozen bridges cost less than the single building union.

**Conditional and async**: nothing builds while the Polymesh backend is off
(`PolymeshDebug`, persisted — on by default, so the usual path *is* a build on entering
the world); the build runs on `AsyncComputeTaskPool`
(`PolyNavmesh` resource: `PolymeshBuild` + generation counter + in-flight task,
cleared by `city.rs::reload_world` alongside `NorthstarGrid`). `PolymeshBuild`
carries the obstacle contours next to the mesh on purpose: polyanya stores only
**walkable** polygons, so without them the overlay could not paint what is blocked.
A radius-slider step supersedes the in-flight build, and superseding **cancels** it
through an `Arc<AtomicBool>` — the same machinery `NorthstarGrid` uses and for the same
reason: the task body is synchronous, so dropping the `Task` throws away the result but
not the work. Measured on Tula: **~5 s at radius 0, ~20 s at any non-zero radius** (the
obstacle inflation dominates), and one drag across the slider queues a build per step —
without the flag five superseded builds ran all cores to completion. Checks sit before
each long stage (boolean, clip, `as_navmesh`, `merge_polygons`); inside `i_overlay` and
`spade` there is nowhere to look.

**Chunked by default** (`CHUNK_TARGET_METERS` = 400 m, capped at `MAX_CHUNKS` = 240
layers): the map is cut into a grid of polyanya layers stitched along seams computed
once in world coordinates, and the search runs over the chunk graph. Now that the
divergence is fixed (see below) the hierarchy is the default, and it wins on both
numbers (Tula, 500 queries, radius 0.2, `examples/polymesh_bench`): **build 0.31 s vs
5.72 s** flat — each chunk triangulates from its own small edge set — and **5.66 ms
mean / 43 ms worst vs 6.18 / 104**, same misses. `QWE_POLYMESH_CHUNK_M` set larger than
the map returns the flat single layer, which is how "hierarchy's fault" is told apart
from "geometry's fault".

**The corridor is the route plus a corner fill** (`PolymeshBuild::corridor`): the
low-level polyanya query sees only the chunks the level-1 A* walked, every other layer
blocked — *and* the fourth chunk of each 2×2 block the route turns in. The graph is
four-connected (an edge is a shared seam **segment**, not a point), so a diagonal trip
is always a staircase A→B→C; with only those three open the free region has a reflex
corner, the shortest path must round its vertex, and that vertex is a chunk grid node.
polyanya's funnel lands on it *exactly* — with a non-empty `blocked_layers` any vertex
touching a blocked layer counts as a corner — and on screen dozens of paths converge on
one point and radiate out of it (`Show` + movepath). `examples/polymesh_corner_audit`
counts it: on Tula (400 queries, radius 0.4) a bend exactly on a grid node fell from
**40.9 % of paths to 16.2 %** with the corner fill, 7.2 → 3.15 m per bend, path length
against the straight line 1.090 → 1.061 (a flat mesh gives 1.037). It is paid for in
open area — the corridor grows from 9.6 to 12.9 chunks and `polymesh_bench` goes
5.61 → 6.23 ms mean, 45 → 75 ms worst, same misses — and only routes with a turn pay.
The added chunk is the expensive kind: it sits *on* the straight line to the goal, so
its polygons carry the smallest heuristic and get expanded first, all of them.
Opening the whole ring of neighbours instead, or filtering turns by "the straight
start→goal line touches that chunk", were both measured and rejected (the filter saves
a few percent of the time and gives back most of the bends).

**The polyline is then string-pulled** (`smoothed` / `segment_clear`): a waypoint is
dropped when the straight cut past it lies wholly on the mesh, tested by walking
polygons — not by sampling, which would step over a gap between buildings. The walk
crosses seams the way polyanya's own `successors` does (the polygon shared by both
endpoints of an edge), and it is deliberately conservative: an ambiguous crossing
(exactly through a vertex, a start that never sat on the mesh) counts as blocked.
Two subtleties cost a full debugging round each, both because **every waypoint is a
mesh vertex**: the walk must start from a point a centimetre *along* the segment (a
vertex belongs to several polygons, and the one localisation returns can be behind the
cut — the walk then finds no exit and reports "clear" without moving; that let 10 % of
smoothed segments run through blocks, one for 3.2 km), and a crossing at the very end
of the segment is an arrival, not an exit. Corridor and smoothing fix *different*
things and are both kept: the corridor shortens the route (1.090 → 1.061), smoothing
removes the bend that remains (16.2 % → 5.1 % of paths, 20 bends over 396 paths) and
costs 0.3 ms of the 6.5 ms mean.

**Seam vertex sets are allowed to differ** between two neighbours, and the stitch is
written for that. Only the chunk *outline* is global (`seam_points` — every crossing of
an obstacle contour with a grid line, computed once for the whole map); the obstacles
themselves are clipped, simplified and triangulated per chunk, so a half-metre slit
between inflated contours can stay open in one chunk and close in its neighbour, and
spade can split a boundary edge at an intersection only one side knows about. A vertex
without a partner is therefore normal (Tula: 0–9 per map depending on radius, logged as
`seam vertices face a wall on the other side`) — an earlier `debug_assert` demanding
zero was measuring an invariant that never held, and it killed the build task whenever
the radius slider landed on the wrong value.
What is *not* allowed is a **one-way seam** (`verify_seams`), and `unstitchable` keeps
it impossible by construction: an unpaired vertex is dangerous exactly when it sits
strictly inside a segment the blind side keeps as a whole edge **and** the rich side
holds both ends of that segment in one polygon — then stitching would hand the
neighbour's edge a crossing its own two halves cannot answer. Such a segment is left
unstitched (both ends dropped, since stitching addresses vertices, not edges), and in a
debug build that is a **panic**: the mesh saved itself, but the geometry is broken and
the dev build says so at once. The message is written to be read by an agent handed the
log — what broke, what it costs, which functions to fix, what *not* to do, and the
offline repro (`examples/polymesh_seam_audit -- <radius>`, which prints both counts per
radius with coordinates). The sample it names is collected under `debug_assertions`
only. On Tula the assert fires on none of the nine slider radii — usually the extra
vertex splits the polygon too, so no shared polygon survives and nothing one-way can
form.

The build ends with **`mesh.bake()`**, strictly after `merge_polygons` (which starts by
un-baking). Baking is what makes the mesh queryable at scale: without it point location
is a linear scan over every polygon, twice per query, and an unreachable goal burns the
full `polygons.len() * 10` budget instead of failing at once on the island check.

A chunk can come out **fully blocked** — a river, a solid block of buildings, and the
layer has zero polygons. That is legal, and `polymesh chunked … N layers fully blocked`
counts them: New York has 4 of 140, every other city of the panel has none. Such a layer
is deliberately left *un-baked*: `BVH2d::build` (bvh2d 0.7, under
`Layer::bake_polygon_finder`) has no recursion base for an empty shape list — it splits
zero shapes into two empty halves forever and kills the process with a stack overflow in
the `AsyncComputeTaskPool` worker, whatever the stack size. The guard lives in
`vendor/polyanya/src/layers.rs`; every reader of `baked_polygons` already branches on
`None` into the linear scan, which over zero polygons correctly answers "not on the
mesh". Repro over all six cities: `examples/audit/polymesh_empty_layer_repro.rs`.

## Polygonal routing

**Polygonal routing** (`polymesh::find_path_polymesh`, dispatched in
`movement/pathfinding.rs`) — with the Polymesh backend on, `dispatch_pathfinding_requests`
routes through the polygonal mesh and the `PathfindingAlgorithm` cycler is bypassed.
**While the mesh is still building** (5–20 s) `Pathfinder::polymesh_build()` is `None`
and the grid serves the request — the same fallback shape HPA* uses while
`NorthstarGrid` builds.

A path is a **world-space polyline** (`Movable::path: VecDeque<Vec2>`,
`PathfindingResult::path: Option<Vec<Vec2>>`), always including its start point: the
consumer drops the first waypoint and reads a single-element path as "already there".
polyanya's `Path::path` omits the start, so `find_path_polymesh` prepends it; the grid
backends still return tiles and the dispatcher maps them through `tile_center`, so both
look identical downstream.

The **goal stays a tile** (`MovableState`, `PathfindingRequest`,
`MovableReachedDestinationEvent`): it is the identity that discards a stale answer and
the arrival test. Only waypoints became metric. The polygonal query therefore starts at
the pawn's real `SimPosition` and ends at `tile_center(end_tile)`.

A **missed goal is `PathfindingError`, not a fallback**: with a non-zero agent radius a
target picked by tile passability can land inside an inflated obstacle, and polyanya
only snaps endpoints within `search_delta * search_steps` (0.2 m). The cost of that
choice is visible in the speed panel as `answers: N/frame, X % failed` — a pawn whose
own position is off-mesh fails *every* repath and stands still, so the number is worth
watching. It is computed from **two** diagnostics, `pathfinding/answered` and
`pathfinding/failed`, both written every frame including zeros, and shown as the ratio
of their averages: a percentage computed per frame and then averaged would count
frames instead of answers (a lone late failure makes its frame read 100 %) and would
freeze on its last value the moment answers stop. The denominator is on screen for the
same reason: 100 % of 0.7 answers a frame is a trickle of hopeless repaths, not a dead
navmesh.

Coasting and the demon lunge's `line_of_sight` stay **grid** tests: they are cheap
guards against walking into a wall, not path searches.

Two knobs exist only because the default fails at city scale, both measured on Tula
(40 199 polygons after merging, 20 000 pawns, 30×):

- **Endpoint tolerance** (`SEARCH_DELTA * SEARCH_STEPS` = 1 m, half a navtile). polyanya
  defaults to 0.2 m, exactly the agent radius, and that is not enough: 80 % of wander
  targets are building outline vertices, and the grid calls a tile passable when its
  centre clears the polygon by a centimetre. **96 % of requests failed** with the
  default against 3.5 % on the grid; at 1 m it is **0.6 %**.
- **`MAX_POLYMESH_PATHFINDING_IN_FLIGHT`** equals the grid's 1024 — an earlier low cap
  tried to contain runaway memory and instead stalled the whole dispatcher.

The same arithmetic sets the **ceiling of the agent radius slider**
(`POLYMESH_AGENT_RADIUS_MAX` = 0.6, range 0.2–0.6): the tolerance rescues a goal that
sits inside the inflation by less than a metre, and the inflation grows with the radius,
so the two meet. Misses over the ladder (`examples/polymesh_miss_audit`, 600 queries,
the seeded set of `polymesh_bench`): 0.5 % at 0.2–0.3, 1.0 % at 0.4, 1.7 % at 0.6,
2.7 % at 0.7, then the cliff — 6.7 %, 12.3 %, **20.7 % at 1.0**, where 47 % of goals no
longer sit on the mesh at all and 88 % of the misses are the goal walled in, not a
search that ran out. Physics agrees with the number: a human is 0.5 m across
(`HUMAN_SIZE` is the doubled, readable size), so 0.6 is already twice the real body.
The audit is the tool for that question — it splits each miss into an endpoint that
never sat on the mesh and one that sat in another connected component.

**Divergence, resolved twice over.** A single search used to allocate unbounded memory
(flat ~3 GB, then past 17 GB in seconds, OS kill): polyanya's iteration budget caps
only queue *pops* while `successors` pushes fans of nodes unchecked. Fixed at the root
in the **vendored** `vendor/polyanya` (a `[patch.crates-io]` path dep, edits marked
`QWE:`): exact node repeats — same polygon, root and interval — are deduplicated,
killing the cycle where a corner vertex on a seam's collinear edge chain spins
equal-cost nodes around its polygon ring forever (root_history only drops strictly
worse nodes). Belt and braces on top: `bounded_path` is the **only door to polyanya**
— the corridor branch included, via the vendored `Mesh::get_path_on_layers` (the polled
search honoring blocked layers; the blocking `path_on_layers` is not used, its internal
limit counts the whole mesh and cannot be interrupted). The external work budget scales
to the open polygon count (40 pops each, min 4096 polls — 10 was measured on the flat
mesh and starved healthy long corridor routes, which converge at ×2; see
`examples/audit/polymesh_budget_repro`), a `NotFound` returns
immediately instead of idling out the limit, and an exhausted budget is a **panic in
every build**, with both endpoints in the message: a diverging search must kill the
game so the geometry (or the degenerate start/goal that caused it) gets fixed, not
silently eat the async pool — live symptom of the silent version was demons frozen at
the portal, an idle-looking pipeline and 400 %+ CPU. A one-way seam (`verify_seams`)
panics in debug for the same reason. Third layer: `PathfindingTask` carries its spawn
time, and the receiver panics on a task older than `PATHFINDING_TASK_HANG_SECS` — a
search hung in any *new* way (a lock, a loop in another backend) surfaces as a crash,
never as pawns quietly standing still. Measured after the fix: 2000 chunked queries,
0.7 % missed, 5.3 ms mean, 42 ms worst, flat memory.
