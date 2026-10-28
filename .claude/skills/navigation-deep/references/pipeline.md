# Pathfinding pipeline — backends, NavMode, dispatcher

Detail behind `navigation/{astar,backend,mode,northstar}.rs` and
`movement/pathfinding.rs`. The rules summary lives in the navigation-deep skill; the
terms in `CONTEXT.md` (Navigation, Determinism).

## PathfindingAlgorithm

**PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
in the Nav tab (the six algorithms are a `CONTEXT.md` term). IDA*/IDDFS are deliberately
excluded (never finish on open grids).
**Default is HPA\*** — 28× cheaper than flat A* per `examples/bench/pathfinding_bench.rs`
(1.3 ms vs 36.4 ms mean, 15 ms vs 450 ms worst case) at ~10% longer paths. The other
five stay switchable for comparison.

## NorthstarGrid

**NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
once from the final navmesh (after pruning; chunk sized to 50 world metres — see the
navtile-size section of the skill), wrapped in `Arc`, called directly
from async tasks — the crate's plugin is not used. Long paths cost ~0.5 ms vs ~40 ms for
flat A*. The build is seconds and grows with the map, so it runs as an
`AsyncComputeTaskPool` task started on `OnEnter(PlayPhase::Live)` **in the live branch
only** (the deterministic branch starts it earlier — next section) and picked up by
`poll_northstar_build`; until it lands, `NorthstarGrid::get()` is `None` and the
dispatcher **falls back to flat A\*** for HPA*/Theta* requests. Doing it inline cost
11 s of frozen loader screen; starting it before the warmup ends made it fight the
warmup's A* for cores through rayon (85 ms per search instead of 36 ms).
**Only the selected backend is built** (`northstar_wanted`): the hierarchy serves
HPA*/Theta* on the grid, so with `Algo: Polymesh`, or with a flat grid algorithm
picked, those seconds of every core are skipped entirely. Switching `Algo` back to
`Navmesh` or cycling `Pathfind` to HPA* starts the build right then — the same lazy
shape the polygonal mesh has, run from `Update` on a resource change.
**Both ends of a request are bounds-checked against the grid's own dimensions**
(`grid_point` → `OrdinalGrid::in_bounds`) before `pathfind`, and out of bounds is a
silent `None` — same verdict the grid gives ("out-of-bounds reads impassable"). Without
it the crate logs a `log::error!` per call, at dispatcher rate, for a pawn past the map
edge. The size is asked of the grid, not of `grid::grid_size()`: the grid is built
from a navmesh snapshot and outlives a navtile-size change.

## Where the build starts is a mode branch — three registrations of `start_northstar_build`

**Where the build starts is a mode branch — three registrations of
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

## Backend — one snapshot, two writers, one collector

**Backend** (`navigation/backend.rs`) — the active backend as one cloneable `Send`
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
including zeros" is what makes the panel's ratio-of-means valid — a percentage computed
per frame and then averaged would count frames instead of answers (a lone late failure
makes its frame read 100 %) and would freeze on its last value the moment answers stop;
the denominator is on screen for the same reason: 100 % of 0.7 answers a frame is a
trickle of hopeless repaths, not a dead navmesh. Discipline lost it twice (measurements
only on frames with answers, then an `if due.is_empty() { return; }` in the
deterministic receiver — both left the panel frozen on its last value).
Pinned by `movement/tests.rs`, one test per receiver.

`Backend::walkable()`
returns the passability view `Walkable` (read lock taken once per system run — lazily
in the collectors, where it is only needed under rescue): `allows` /
`nearest_free_point` are backend-strict (grid first as the cheap test, then the
agent-radius-inflated mesh), while `sift_target`, `line_of_sight` and `coast_allows`
are deliberately grid-only — hot-loop cost, documented on the methods; switching one to
the mesh-strict test is a one-line edit there, not a sweep over four behavior systems.
Boundary invariant: outside `navigation/` and `ui/`, the names `PolymeshBuild` /
`PolymeshDebug` / `PathfindingAlgorithm` do not appear (the full statement is a
`CONTEXT.md` term). The replay door `use_flat_grid(&mut World)` also
seeds the two resources `Pathfinder` merely needs to validate.

## NavMode

**NavMode** (`navigation/mode.rs`) — the backend question as a value, and the only place
the four resources are combined (`Pathfinder::mode`). `Grid(Flat | HierarchyPending {
wanted } | Hierarchy(g))` × `Mesh(Pending | Ready(b))`; the methods are the questions
that used to be asked separately — `is_building`, `northstar_wanted`,
`mesh`/`hierarchy`. The "is the mesh toggled on" question is deliberately not a table
method: its consumers read the toggle itself (`ContinuousSpace` for the
separation gate — its rule lives in `references/crowd.md` — and the panel sections
directly, inside the `PolymeshDebug`
visibility boundary; `MovementPlugin` is raised in tests without navigation, so it cannot
see a `NavMode`). The truth table is pinned by two tests in the
module, which is the point: before, each column was assembled in a different file from a
different combination of resources, and there was nowhere to compare them.
`HierarchyPending { wanted }` is what keeps `northstar_wanted` honest — `wanted` is
"not started yet" (`NorthstarGrid::is_missing`), so the build is still ordered
exactly once. **`Mesh(Pending)` is the state that did not exist before**, and its absence
was expensive: `polymesh_build()` collapsed "off" and "building" into one `None`, so a
consumer needing the difference — `crowd_demo`'s overlay label — rebuilt the question
from raw resources, with a comment admitting it.

## The dispatcher

**PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
`Movable::to_pathfinding` only queues a `PathfindingRequest`;
`dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks
(polled with `check_ready`).

**Visibility gating**: at zoom ≥ `WANDER_DISPATCH_MAX_ZOOM` (0.75 m/px) *no*
wanderer counts as on screen — a pawn is a dot there, and "in view" would otherwise
mean half the map, flooding the task pool and the per-frame sort with ~17k peaceful
requests. Demons and fleeing humans are always dispatched at any zoom.
**The live dispatcher stands while the player pauses** (`live_dispatch_runs`): nothing
walks, the queue does not change, and the pass over ~17k waiting requests was ~0.3 ms a
frame for nothing; requests pile up and leave on the first unpaused frame. The warmup
pause is the exception and must stay one — `loading::pause_world` holds
`Time<Virtual>` paused through `PlayPhase::Warmup`, and the warmup waits for exactly
these answers. The deterministic dispatcher is untouched (it is `FixedUpdate`, which a
pause stops anyway), so replay is not affected.

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

**Repath on the move** — `to_pathfinding` keeps the current path and the
`MovableStateMovingTag`, so an entity walks its old path while the new one is
computed; `MovableStateMovingTag` therefore means "has a path **or is coasting**",
*not* "state is `Moving`". Dispatch and pickup both live in `Update`, so a reply
costs 2–3 frames — and a frame carries `speed × 1/fps` virtual seconds, so at 30×
the reply lags by 1–1.5 virtual seconds while a fleeing human repaths every ~1 s:
the old path routinely runs out before the reply lands.

## The in-flight cap is not a polymesh knob

**The in-flight cap is not a polymesh knob.** There is one limit for both backends,
`MAX_PATHFINDING_IN_FLIGHT` = 1024 in `settings.rs`, sized for the dispatcher's
demand at 30×; no `MAX_POLYMESH_PATHFINDING_IN_FLIGHT` exists. A separate, lower polymesh
cap did exist and was deleted once it had been raised to the grid's value: a cap does not
bound the memory of a diverging search — one query explodes, not their number — while a low
one stalled the world, a couple of heavy searches holding almost every slot and the
dispatcher handing out nothing to anyone. The mesh search's memory is bounded by its step
budget (`polymesh/path.rs::SEARCH_POPS_PER_POLYGON`), the slots by that one cap; the full
rationale lives in the constant's own doc comment.
