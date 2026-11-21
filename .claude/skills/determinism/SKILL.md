---
name: determinism
description: Use when working on qwe's determinism — the world seed and RNG derivation (rng.rs), the per-decision stream, PawnId/Species identity, SimTick, the SimPipeline system sets, the deterministic pathfinding dispatcher (retire tick, dispatch rate, FIFO key), the frozen backend, and the replay checks (determinism/replay.rs, tests/determinism.rs, examples/acceptance/). Deep detail behind CONTEXT.md's Determinism summary.
---

# Determinism & replay — deep detail

Detail behind `src/rng.rs`, `src/determinism/` and the replay tests. `CONTEXT.md` names
the terms (world seed, decision stream, `SimTick`, `Deterministic`, `SimPipeline`); this
file is the mechanism behind them, and the reasons each rule is shaped the way it is.

**Changing that mechanism is changing this file, in the same turn** — a new RNG domain, a
moved dispatcher key, a changed retire deadline, a system that switches `SimPipeline` set.
This file is what the next session reads instead of the code; a rule left standing here
after the code stopped obeying it is worse than no rule at all. The term itself still goes
to `CONTEXT.md`.

## The seed and its derivation

**World seed** (`rng.rs::WorldSeed`, remembered by `prefs.rs`, panel row *Seed* in the Sim
tab) is the one number every simulation draw descends from. It governs the **simulation**,
not the map: OSM is parsed from a cache file, and trees and entrances are seeded by their
own coordinates (`map/seed.rs::Lcg`, `entrances::lcg_seeded_by`), so those are
already reproducible without it.

Capped at `i64::MAX` (`MAX_SEED`) — `toml` cannot store more, and the seed has to survive a
restart of the app. The *new* button rolls a 9-digit one so it can be read off the screen
and typed back in.

**`seed_for(world_seed, domain, key)`** — two rounds of splitmix64. `RngDomain: Population
| Human | Demon`. **Nothing stores live RNG state**, so a restart has no RNG to reset:
every stream is re-derived from the seed.

### The placement stream — the one shared generator, and why it is allowed

`rng.rs::stream(world_seed, domain, key)` builds a stream that belongs to no pawn. The
only live one is `stream(world_seed, RngDomain::Population, 0)`, held across the **whole**
`human/systems.rs::place_population` pass: the rejection loop that samples tiles until one
is passable draws a variable number of samples per human, and that is fine inside one
sequential pass. The batched spawn keeps it that way deliberately: the pump computes
**all** placements on its first call (`PendingPopulation::placements`) and every later
batch only spawns pawns from the finished list — had the stream been drained batch by
batch, its generator would live in a resource between calls, and that is exactly the
shape the rule below forbids.

This is the shape the next section calls rejected, so the exception is spelled out:

- **The consumption order is a fixed counter, not a traversal.** `for index in 0..count`
  over a plain `usize` — no query, no `HashSet<Entity>`, no camera, nothing whose order
  can differ between a run and its replay.
- **Nothing personal comes out of it.** Colour, `Pace` and `WanderHeading` are drawn from
  the human's own `decision_stream(world_seed, RngDomain::Human, pawn_id,
  WanderIndex::SPAWN)`, so human N looks and walks the same however many times the tile
  rejection missed for his neighbours. The shared stream decides *placement* only.
- **It never crosses a system boundary.** Created and dropped inside the call, so there is
  no generator resource for a restart to reset — a restart re-inserts the
  `PendingPopulation` queue and the pump's first call runs `place_population` again,
  re-deriving the stream from the seed.

The rule to carry forward: **a shared stream is legal only while the loop draining it is a
`for i in 0..n` inside one function.** Move those draws into a system that iterates a
query, or keep the generator in a resource between calls, and the exception is gone.

Pinned by `human/systems.rs::population_is_a_function_of_the_seed` /
`different_seeds_give_different_populations` (bit-for-bit fingerprint of the whole spawn)
and by `rng/tests.rs::seed_for_is_stable`, which holds `seed_for(1, Population, 0)` as a
golden value. `examples/demos/crowd_demo/scenario.rs` drains the same `(Population, 0)`
stream under the same fixed-count loops — the same idiom, not a second exception.

**The horde's wave stream is the same shape** (`demon/horde.rs::plan_wave`, phase 4 of
the ×10 plan): `stream(seed, Demon, HORDE_WAVE_KEY + wave_index)` draws the hearth tile,
the release scatter and the lull of one wave inside one `plan_wave` call — a fixed
`0..n` counter over a plain loop, no world state in the inputs (the cap truncation happens
in the system, after the pure plan). The wave keys sit in the **upper half of the key
space** (from 2⁶³): decision-stream keys are `(pawn_id << 32) | decision`, and real
pawn ordinals stay orders of magnitude below 2³¹, so the two families cannot collide.
Pinned by `horde::tests::the_same_seed_plays_the_same_wave` (pure schedule, no app) and
by the replay pins — the replay yard runs with waves on (yard-scale `HordeStyle`
inserted in `replay_app`, like `PopulationSize`), so `a_restart_replays_the_run` covers
schedule and release.

## The decision stream

`rng.rs::WanderIndex::next`, on humans *and* demons. A `SimRng` is built **per decision**
and dies with it, seeded from `(PawnId, decision number)`. The seed is therefore the pawn's
**observable identity plus which choice this is** — never the history of a stream.

Consequences, all of them deliberate:

- Draws do not depend on query iteration order, on how many neighbours drew before it this
  tick, or on how many draws the pawn's previous decision happened to consume.
- Pawn K's k-th decision draws the same numbers **whenever it happens**, so it is the same
  with the toggle on and off. In normal mode that makes the *opening* reproducible
  (measured: 99.8 % identical first targets across launches) but not the run — positions
  diverge with frame timing. Full replay is what the toggle is for.

Each rejected alternative has bitten:

- **One shared generator** collapses under any reordering — `panic` draws its repath period
  while walking a `HashSet<Entity>`, whose order differs between runs. The single exception
  is the placement stream above, whose consumer is a fixed `0..count` loop.
- **A live per-pawn stream** shifts under one added `rng.random()` inside a decision.
- **Position as an input** is tempting and wrong: pawns stand on tile centres bit-for-bit,
  so `(pawn_id, tile) → target` would be a deterministic function, and every trajectory of
  one on a finite set eventually closes into a cycle — within minutes each human would pace
  a fixed loop forever. The decision number only ever grows.

**The RNG rolls therefore stay in `behavior.rs`, not in the shared wander skeleton** — the
stream must advance on exactly the ticks it advanced on before, i.e. only on the pawns that
actually took a decision this tick (see the `species-behavior` skill).

**Where the stream actually advances** — seven `WanderIndex::next` sites, and not all of them
are a wander decision: `human/systems.rs::pick_wander_targets` (target),
`human/behavior.rs::panic` (flee repath period), `human/behavior.rs::flee` (flee step, and
the wander pause on the calm-down rung), `demon/systems.rs::pick_wander_targets` (target),
**`human/lod.rs::far_slice` — the far tiers' slice system** (stroll target + wander pause,
flee-repath period at the panic onset, flee step and calm-down pause — Live-only, so it
does not run in a replay, but its rolls mirror the near sites so a pawn's personality does
not depend on its tier), and
**`demon/behavior.rs::on_demon_caught_human` — an observer**: a kill costs the killer
one decision number, drawn for the devour pause.

That observer is still "on a tick": it is triggered only from `chase`
(`TickStage::DemonBehavior`, `FixedUpdate`) and runs on that system's command flush, inside
the same tick — the flush is a schedule position, not wall clock. Its *order* among several
kills in one tick does not reach the dice either, because the stream is seeded from
`(PawnId, decision number)`; what the replay contract needs is only that a given demon
advances on the same ticks, and a demon can kill at most once per tick
(`killed_this_tick` in `chase` dedupes the rest). Pinned by
`demon::behavior::tests::a_kill_spends_the_demons_next_decision_number`.

**A new `next` outside a `decide`-driven system belongs in this list.** The site that is
easy to get wrong is not the roll but the schedule it hangs off: an observer reached from
`Update`, or a roll behind an `if let Ok(..)` whose condition is not itself deterministic.

## Pawn identity

**`PawnId`** (`rng.rs`) — a pawn's spawn ordinal within its species and run (humans
`0..HUMAN_COUNT`, demons `DemonSpawner::spawned`). Used wherever a stable "personal number"
is needed: the RNG seed key, the flee-fan angle (`personal_spread`), the separation axis
(`coincident_direction`), the dispatcher tiebreak, the spatial-grid distance tiebreak.

**Never `Entity`.** Entity indices are recycled in a different order after a restart (the
free list depends on who was eaten in the previous run), so an `entity.index()` hash would
drift between a run and its replay under an identical seed.

**`Species`** (`rng.rs`, component) — the other half of a pawn's personal number, riding
next to `PawnId` in the same spawn bundle. `PawnId` is unique only *within* a species, so
every mixed ordering (`movement::order::pawn_key`) puts species first.

- **Variant order is part of the replay contract**: `Demon` is declared first because the
  key used to be `u8::from(is_human)`, and a pawn with no `Species` (the test walker) still
  sorts as a demon.
- A component rather than a field of `PawnId`: that type crosses BRP and the replay
  fingerprint, and the RNG seed key must stay `(domain, ordinal)` bit-for-bit.

The ordering doctrine itself — which comparisons are allowed to decide anything, and why a
tie is broken by `PawnId` rather than by traversal order — lives in `movement/order.rs`.

## SimTick

`determinism/mod.rs` — the step counter, incremented at the head of the `FixedUpdate` chain.
**The unit of replay**: world state is a function of `(seed, settings, SimTick)`.

Not the same as `SimClock`, which counts virtual seconds and loses whatever `max_delta`
discarded on a long frame. **Compare states by tick, never by wall clock.**

## The toggle

**`Determinism`** (`determinism/mod.rs`, panel toggle) gates *scheduling*, not the dice — the
RNG work above is unconditional.

| | off (default) | on |
|---|---|---|
| human wander target picking | `Update` | `FixedUpdate` |
| pathfinding answers | when the task lands | on a fixed tick (`RetireAt`) |
| dispatcher | camera-gated, priority by distance | FIFO by `(requested_at, species, pawn_id)` |
| navigation backend | re-taken every frame | frozen for the run |
| northstar hierarchy build | started after the warmup (`OnEnter(PlayPhase::Live)`) | started before it (`OnEnter(PlayPhase::Warmup)`) |
| pawn separation | on (polymesh, on-screen) | off — the Nav tab's `Separation` row reads `off`, dimmed and unclickable, rather than a toggle that flips a resource nothing reads |
| camera LOD (`human/lod.rs`) | far pawns live on slices (`FarTag`) | **does not exist** — `FarTag` is never inserted, the run keeps full AI without a camera (pinned by `lod::tests::no_far_tag_appears_in_a_deterministic_run`) |

**Only the humans' picking moves.** `demon::systems::pick_wander_targets` is registered once,
in `FixedUpdate` inside `TickStage::DemonBehavior`, and runs there in **both** modes
(`src/demon/mod.rs`): in `Update` the choice ran once per frame, i.e. depended on fps, and
demons are a few hundred, so the extra tick runs cost nothing. The humans' copy is the one
registered twice (`src/human/mod.rs`), and it stays in `Update` in the live branch because the
warmup hangs on it — `loading.rs::poll_warmup` waits for wander requests, and `FixedUpdate`
does not run while the warmup holds the sim paused.

A run is deterministic or not from tick 0, so flipping the toggle (like changing the seed)
orders a restart via `RestartPending` — and that restart carries `RestartEvent { to_portal:
true }`: a changed seed or a flipped toggle is a *different world*, and without the camera
move the setting reads as having done nothing.

## SimPipeline — the toggle in the schedule

`determinism/mod.rs`. **Three** system sets — `Live`, `Deterministic` and `BothModes` —
gated once in `DeterminismPlugin` for every schedule they appear in (`PreUpdate`,
`Update`, `FixedUpdate`, both `OnEnter` phases). A system declares its branch with `.in_set(..)` and
**never reads the mode**; a forgotten `run_if` is no longer a way to break replay.

**The sets also carry the world gate** (`in_world`): world resources exist only between
`OnEnter(Playing)` and `OnExit`, and a simulation system running in `Loading` dies on
parameter validation — which crashed a launch once. That gate used to be twenty-five
hand-written `in_state(Playing)`, nine of them on one `FixedUpdate` chain.

- `BothModes` exists so a system running in both modes has somewhere to say so and still
  get the gate. The cost is that "no set" no longer reads as "both modes" but as "lives
  outside the world" — worth it, but it is a live trap when adding a system.
- `in_world` takes an `Option<Res<State<_>>>` because `MovementPlugin` and its neighbours
  are raised in tests with no states at all.
- Two places still read the mode directly. `separation_runs` needs it **negated**
  ("separation is off — clear its leftovers"), and a set cannot be negated.
  `human::pick_wander_targets` reads it to skip the wide first-wander sift ring
  in a replay (`species-behavior`: a live-only entry-storm optimisation; the
  replay files its wander requests by the tick) — the same function is
  registered in both branches, so the branch cannot come from the set.
- `MovementPlugin` / `HumanPlugin` / `NavigationPlugin` / `DemonPlugin` add
  `DeterminismPlugin` if absent: an unconfigured set gates nothing, so both branches would
  run at once — and, since the sets carry `in_world`, the world gate would go with them.
- The tick order is **`TickStage`** (`tick_stage.rs`): one ordered list — `Guard → Tick →
  Answers → Spawner → DemonGrid → DemonBehavior → HumanBehavior → Move → Separation →
  Dispatch` (`TickStage::ORDER`). `TickStagePlugin` — raised by `SpatialPlugin`, the plugin
  every order-needing assembly raises — chains the stages pairwise in `FixedUpdate`;
  plugins place systems in stage sets, and the cross-plugin `.before`/`.after` edges are
  gone (Bevy 0.20 set configs take tuples, not `Vec`, so the pairwise edges are what keeps
  the list the single source). Stages are an **ordering** spine, **not a second world gate**
  — they carry no `run_if` at all, and a behaviour system inside them declares its
  `SimPipeline` branch anyway. Both chains do (`BothModes`): the demons'
  `pick_wander_targets/acquire_targets/chase/devour` and the humans' `panic/flee/escape`.
  Each is pinned by a test that raises its plugin **without** `SpatialPlugin` and runs
  `FixedUpdate` in `Loading`, where the world resources do not exist
  (`demon::tests::demon_behavior_does_not_run_outside_the_world`,
  `human::tests::human_behavior_does_not_run_outside_the_world`); the listed order itself
  is pinned by `tick_stage::tests::the_schedule_chains_the_stages_in_the_listed_order`,
  which inspects the built schedule's dependency graph. The one gate the old `SimSet` chain
  carried (`in_state(Playing)` on `rebuild_demon_grid`) lives explicitly beside that
  system's registration now.

**Every system taking `Res<Backend>` must sit in a `SimPipeline` set** — that is what
carries the world gate along with the mode branch.

## The deterministic dispatcher

### Retire tick

`RetireAt` — the answer deadline of this mode: stamped at dispatch (the tick the request
leaves the queue, not the tick it was filed) and honoured by `apply_pathfinding_results`
`PATHFINDING_RETIRE_TICKS` ticks later, waiting on the search (`block_on`) if it has
not finished. Why the deadline counts from dispatch and not from `RequestedAt`, the
125 ms ≈ live-pipeline parity at 1×, throughput belonging to the dispatch rate rather than
this constant, and why it must not scale with `SimSpeed` — that is the rustdoc of
`PATHFINDING_RETIRE_TICKS` in `src/settings.rs`.

### Dispatch rate

`PATHFINDING_WANDER_UNITS_PER_TICK` / `PATHFINDING_URGENT_UNITS_PER_TICK` —
how much leaves the queue each tick, in units of *predicted search cost*
(`PATHFINDING_UNIT_TILES`). Why the unit is cost rather than requests, the integer-metric
rule, and the ~35 ms-per-tick pool budget the rates are derived from live in the rustdoc
of those constants in `src/settings.rs`. The queue arithmetic itself is its own
unit-tested module — `movement/pathfinding/budget.rs` (`QueuedRequest`,
`request_units`, `push_within_budget`, `take_within_budget`); the dispatcher around
it only assembles the queue from the request query and hands out tasks.

**Never reuse `MAX_*_IN_FLIGHT` here** — those are the live mode's concurrent-search cap
behind the visibility gate; this dispatcher has no gate and ignores them entirely (the
rustdoc of `MAX_PATHFINDING_IN_FLIGHT`, and the doc of
`movement::pathfinding::dispatch_pathfinding_requests_deterministic`).

**A long queue is the normal state of this mode.** At 30× it settles around 2–5× — by
design; and the failure rate then reads worse than live because the sample is the whole
map, not the easy on-screen subset. Watch `answers: N/frame, X % failed` on the speed
panel.

### The FIFO key

**`RequestedAt`** — the tick a request was filed. The dispatcher's key is `(requested_at,
species, pawn_id)`, all integers, since ties between floats have no defined order
(`QueuedRequest::key`, in the same `budget.rs`). **Species
precedes the number** because `PawnId` is only unique *within* a species, and the urgent
queue mixes demons with fleeing humans.

A small rate plus this FIFO *is* the deterministic replacement for the camera gate: distant
pawns still wait longer, but reproducibly rather than because the player looked away. **The
camera does not appear in it at all.**

A re-target re-files the request, so its `RequestedAt` is the tick of the LATEST goal, not
of the first: `to_pathfinding` removes request and mark together and
`stamp_pathfinding_requests` stamps the fresh pair on the same tick. A new destination is a
new request, and it queues behind everything filed before it.

### Frozen backend

`determinism/mod.rs::on_world_started`. In this mode the `Backend` resource is written once,
on every **WorldStarted**, and never refreshed: `refresh_backend` is `SimPipeline::Live`
only.

northstar and polymesh finish building at some moment of *real* time; a re-taken snapshot
would switch backends mid-run, and a replay would switch on a different tick. (This is the
whole of what the retired `DeterministicRun` resource used to be — one resource with two
update policies replaced two types for one concept.)

Warmup therefore waits for the wanted backend instead — `loading.rs::poll_warmup` returns
early while `Pathfinder::mode().is_building()` holds (`navigation/mode.rs`), and its
`elapsed` counter stands meanwhile, so `WARMUP_TIMEOUT` cannot cut a build short.
That costs the northstar build (~22–28 s on the current map, the measurement and its
history in `navigation/northstar.rs`'s module doc) on first entry into a city on HPA —
deliberately. Restarts do not pay
it.

And it is not only the *wait* that is deterministic-only — the build is **started** earlier
too: `start_northstar_build` is registered a second time, on `OnEnter(PlayPhase::Warmup)` in
`SimPipeline::Deterministic` (`navigation/mod.rs`; that `OnEnter` is one of the schedules
`gate_pipelines` configures). Without that site the wait would not fall back, it would
deadlock: `is_building()` is true for `HierarchyPending { wanted: true }` ("not started
yet") as well, the early return freezes `elapsed` so the 10 s timeout never fires, and the
phase that would start the build is `Live` — the one the warmup is holding. The live
branch's reason for starting late (rayon fighting the warmup's A* for cores) does not apply
here: this mode's warmup computes nothing. Detail — the `navigation-deep` skill,
**NorthstarGrid**.

**No pawn warmup in this mode**: once the backend is built it enters `Live` immediately. The
pipeline lives in `FixedUpdate`, which is paused during warmup, so the pawn counter could
not move (it burned the full timeout); and "pawns on screen" is a camera notion that may not
influence tick count. The dispatch rate starts the population in a wave over a couple of
seconds.

### NeedsWanderTarget

`movement/components.rs` — marker held exactly on `Idle` and `PathfindingError`, maintained
only by the `Movable` transitions. Target picking moves to `FixedUpdate` in this mode, i.e.
~30 runs per frame at 30×; without the marker each run would scan all 17 000 wanderers to
find the few thousand standing ones.

## What replay is checked with

### Two yards, and they are not duplicates

- **`sim_yard::behavior_yard`** (test-only) — the four resources a species' behaviour stand
  needs (backend, grid, diagnostics store, clock). Answers "did this rung fire".
- **`replay_app`** — the run yard. Answers "does the whole run replay".

**The list of plugins `replay_app` deliberately leaves out lives next to the list it
includes**, with a reason per line: that boundary — the ten included plugins, everything
else named out below them — is exactly what `a_restart_replays_the_run` can and cannot see,
and it was previously invisible from `loading.rs`, where the claim about it is written.

### The replay check

`determinism/replay.rs`, `tests/determinism.rs`, `examples/acceptance/determinism_replay.rs`
— same machinery, two scales: the test runs a synthetic yard (`fixture::crowded_yard`,
dozens of pawns, 96 ticks, ~1 s in `cargo test`); the example runs Tula with 20 000 pawns
for minutes.

Three claims: the same seed replays tick for tick, a ragged frame rate does not change the
run, a different seed does.

Two things make it bite, both learned the hard way:

- **The world must actually move.** `apply_pathfinding_results` needs `SimLoad`, which lives
  in `SimTimePlugin` — absent from the replay app, the system failed parameter validation
  and was skipped in silence, so no path was ever applied and the check compared two equally
  frozen worlds. `replay_app` inserts the resource; `Fingerprint::moving` fails the test if
  a run has nobody moving, so the same class of vacuous pass cannot come back.
- **The scene must be crowded.** Spread over the map, pawns only walk their paths, and
  walking is linear in time — nothing diverges. Divergence is born at thresholds: panic
  radius, demon lunge. Hence the yard, where the whole population and the portal share
  120 m. Verified by mutation: moving `move_moving_entities` into `Update` fails the
  ragged-frame test on the yard and passes on a scattered map.

### Restart replays the run

`tests/determinism.rs::a_restart_replays_the_run` — a run to tick N, `RestartEvent`, a second
run to N, identical fingerprints. It runs the second half in the **same `App`**, which is
what makes it catch state outliving the reset; the other checks build a fresh app each time
and cannot.

This test is also what holds the **WorldStarted** reset membership from the outside — a
forgotten reset diverges whether or not anyone wrote it down (see the `world-lifecycle`
skill). What it cannot see is state invisible to both the simulation and the outcome
counters; such a reset needs its own pin next to its observer (the regulator has one in
`sim_time`, the frozen `Backend` one in `determinism` — the replay yard pins flat A*, so the
seeded and the announced snapshots coincide there by construction).

Both defects it first caught were in how the replay app was assembled, not in the reset:

- It never announced `WorldStarted`, so the run kept the placeholder `Backend` — an empty,
  everywhere-passable grid — and the whole run pathed through buildings. That placeholder is
  now gone: **`Backend` has no `Default`** (see the `navigation-deep` skill).
- It left the algorithm at the default HPA* while relying on the hierarchy "not being ready
  in time" — a restart re-freezes the backend, and by then it was.

So a replay app must **announce the world start before its first tick** (the demon burst
goes out on tick 1, and a spawner reset after it hands the second burst the same `PawnId`s)
and **pin the algorithm** to one that needs no build.

## The contract

**1:1 replay holds only while `DemonStyle` / `HumanStyle` / `SeparationStyle` / the
algorithm / the navtile size are left alone mid-run.** Sliders are simulation input. Not
enforced by code.

**Frame rate does not matter.** `Time<Fixed>`'s step is constant regardless of fps and of
`SimSpeed`; the answer to a path query waits for its tick; everything left in `Update` only
draws. A slow machine replays the same run more slowly — it does not replay a different one.

**Boot timing must not matter either — and one way it did (разрез №44, fixed).** The
virtual delta of the world-entry frame is counted in `First`, *before* the `StateTransition`
that pauses the world, so the entry frame used to run 1–2 unannounced ticks — the
deterministic run then started on the leftovers of that invisible pre-announce run (a demon
burst from a not-yet-reset spawner, requests stamped with `RequestedAt` ≥ 1), and the run's
trajectory was a function of the entry frame's timing. Two live boots of one seed diverged
bit-for-bit; №43's BRP-poll correlation was coincidence (polls slowed the entry
frame and moved the tick count). The fix: `pause_world` zeroes the frame's delta
(`advance_by(ZERO)`), so the world stands until `Live` at any delta — pinned by
`loading.rs`'s warmup test with a real delta. What is *still* open: the live app's
**acquisition order varies between boots** (the burst demons' pick requests apply 6+2 with
a varying pair; chase claims flip; tick 1 diverges) — the replay yard is unaffected, but a
**cross-process replay of a live boot is not claimed** until that is cut (deepening plan,
«Разрез №44б», the perf43 worktree keeps the full instrument).

**Not claimed**: float reproducibility across machines or compilers; replaying a run made
with the toggle *off*. `bevy_northstar` builds its grid with rayon, so cross-process HPA
replay is unaudited — within one session the grid outlives R, so restarts are safe.
