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
own coordinates (`map/trees/crown.rs::Lcg`, `entrances::lcg_seeded_by`), so those are
already reproducible without it.

Capped at `i64::MAX` (`MAX_SEED`) — `toml` cannot store more, and the seed has to survive a
restart of the app. The *new* button rolls a 9-digit one so it can be read off the screen
and typed back in.

**`seed_for(world_seed, domain, key)`** — two rounds of splitmix64. `RngDomain: Population
| Human | Demon`. **Nothing stores live RNG state**, so a restart has no RNG to reset:
every stream is re-derived from the seed.

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
  while walking a `HashSet<Entity>`, whose order differs between runs.
- **A live per-pawn stream** shifts under one added `rng.random()` inside a decision.
- **Position as an input** is tempting and wrong: pawns stand on tile centres bit-for-bit,
  so `(pawn_id, tile) → target` would be a deterministic function, and every trajectory of
  one on a finite set eventually closes into a cycle — within minutes each human would pace
  a fixed loop forever. The decision number only ever grows.

**The RNG rolls therefore stay in `behavior.rs`, not in the shared wander skeleton** — the
stream must advance on exactly the ticks it advanced on before, i.e. only on the pawns that
actually took a decision this tick (see the `species-behavior` skill).

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

`determinism.rs` — the step counter, incremented at the head of the `FixedUpdate` chain.
**The unit of replay**: world state is a function of `(seed, settings, SimTick)`.

Not the same as `SimClock`, which counts virtual seconds and loses whatever `max_delta`
discarded on a long frame. **Compare states by tick, never by wall clock.**

## The toggle

**`Determinism`** (`determinism.rs`, panel toggle) gates *scheduling*, not the dice — the
RNG work above is unconditional.

| | off (default) | on |
|---|---|---|
| wander target picking | `Update` | `FixedUpdate` |
| pathfinding answers | when the task lands | on a fixed tick (`RetireAt`) |
| dispatcher | camera-gated, priority by distance | FIFO by `(requested_at, species, pawn_id)` |
| navigation backend | re-taken every frame | frozen for the run |
| pawn separation | on (polymesh, on-screen) | off — the Navigation panel's `Separation` row reads `off`, dimmed and unclickable, rather than a toggle that flips a resource nothing reads |

A run is deterministic or not from tick 0, so flipping the toggle (like changing the seed)
orders a restart via `RestartPending` — and that restart carries `RestartEvent { to_portal:
true }`: a changed seed or a flipped toggle is a *different world*, and without the camera
move the setting reads as having done nothing.

## SimPipeline — the toggle in the schedule

`determinism/mod.rs`. **Three** system sets — `Live`, `Deterministic` and `BothModes` —
gated once in `DeterminismPlugin` for every schedule they appear in (`Update`,
`FixedUpdate`, both `OnEnter` phases). A system declares its branch with `.in_set(..)` and
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
- The one place the mode is still read directly is `separation_runs`, which needs it
  **negated** ("separation is off — clear its leftovers"), and a set cannot be negated.
- `MovementPlugin` / `HumanPlugin` / `NavigationPlugin` add `DeterminismPlugin` if absent:
  an unconfigured set gates nothing, so both branches would run at once.

**Every system taking `Res<Backend>` must sit in a `SimPipeline` set** — that is what
carries the world gate along with the mode branch.

## The deterministic dispatcher

### Retire tick

`RetireAt`, `PATHFINDING_RETIRE_TICKS = 8`. A request issued on tick `T` is applied on
exactly `T + 8`, whether or not the search finished; if it did not,
`apply_pathfinding_results` waits on it (`block_on`).

**That wait *is* the mechanism** — it removes "when did the OS get around to it" from the
simulation. Eight ticks ≈ 125 ms at 1×, which is what today's `request → dispatch → task →
collect` pipeline already costs, so pawn behavior is unchanged.

It does **not** set throughput — the dispatch rate below does; K only buys a batch wall time
before its join, and pays in path staleness. **The constant must not scale with `SimSpeed`**
— that is user input and may not influence replayed content.

### Dispatch rate

`PATHFINDING_WANDER_UNITS_PER_TICK = 128`, `PATHFINDING_URGENT_UNITS_PER_TICK = 64` — how
much leaves the queue each tick, measured in *predicted search cost*, not in requests.

A request costs `1 + chebyshev_tiles / PATHFINDING_UNIT_TILES`, an **integer**: a float sum
would depend on iteration order. Measured, a stroll and a cross-city errand differ 20×, so a
budget in requests cannot fit both. The rate is derived from what the pool chews per tick
(~35 ms of CPU).

**Never reuse `MAX_*_IN_FLIGHT` here** — those cap *concurrent* searches behind the
visibility gate; with no gate the same number once meant 65 000 searches per real second and
2.6 fps.

With the rate, the errand wave drains over ~30 virtual seconds at 60 fps; **a long queue is
the normal state of this mode**. At 30× the mode settles around 2–5× — by design, and the
failure rate is visible for the same reason (the sample is the whole map, not the easy
on-screen subset). Watch `answers: N/frame, X % failed` on the speed panel.

### The FIFO key

**`RequestedAt`** — the tick a request was filed. The dispatcher's key is `(requested_at,
species, pawn_id)`, all integers, since ties between floats have no defined order. **Species
precedes the number** because `PawnId` is only unique *within* a species, and the urgent
queue mixes demons with fleeing humans.

A small rate plus this FIFO *is* the deterministic replacement for the camera gate: distant
pawns still wait longer, but reproducibly rather than because the player looked away. **The
camera does not appear in it at all.**

### Frozen backend

`determinism/mod.rs::on_world_started`. In this mode the `Backend` resource is written once,
on every **WorldStarted**, and never refreshed: `refresh_backend` is `SimPipeline::Live`
only.

northstar and polymesh finish building at some moment of *real* time; a re-taken snapshot
would switch backends mid-run, and a replay would switch on a different tick. (This is the
whole of what the retired `DeterministicRun` resource used to be — one resource with two
update policies replaced two types for one concept.)

Warmup therefore waits for the wanted backend instead (`NavigationBuildPending`,
`loading.rs::poll_warmup`), which costs ~11–14 s on first entry into a city on HPA —
deliberately. Restarts do not pay it.

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
includes**, with a reason per line: that boundary is exactly what `a_restart_replays_the_run`
can and cannot see (10 of the game's 19 plugins), and it was previously invisible from
`loading.rs`, where the claim about it is written.

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

**Not claimed**: float reproducibility across machines or compilers; replaying a run made
with the toggle *off*. `bevy_northstar` builds its grid with rayon, so cross-process HPA
replay is unaudited — within one session the grid outlives R, so restarts are safe.
