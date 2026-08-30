---
name: species-behavior
description: Use when working on pawn behaviour in qwe — the human and demon decision ladders (human/decide.rs, demon/decide.rs), their behavior.rs state machines, wander/flee/chase/devour, the flee fan, PanicRecoil, Pace, chase claims and the lunge, the demon spawner, corpses, the spatial grids and the SimSet order. Deep detail behind CONTEXT.md's Simulation summary.
---

# Species behaviour — deep detail

Detail behind `src/human/`, `src/demon/`, `src/movement/wander.rs` and `src/spatial.rs`.
`CONTEXT.md` names the states and the invariants; this file is the mechanism behind them and
the measured reason each rung sits where it does.

**Changing that mechanism is changing this file, in the same turn** — a new rung, a moved
threshold, a changed claim rule, a component that stops existing. This file is what the next
session reads instead of the code. The term itself still goes to `CONTEXT.md`.

Neighbouring skills: pathfinding, separation and destination slots are `navigation-deep`
(`references/crowd.md` for the crowd lab); the RNG stream every decision draws from is
`determinism`.

## The fixed step

**SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`): `SpatialRebuild →
DemonBehavior → HumanBehavior`. **Demons act before humans so a kill lands before `escape`**
— a human is never counted both killed and escaped in one tick.

**SimPosition / PreviousSimPosition** — simulation-space positions; `Transform` is
interpolated between them in `RunFixedMainLoop` (after the fixed loop). Systems mutate
`SimPosition`, **never `Transform.translation.xy`** directly.

Fixed-step order is explicit and load-bearing:

- `snapshot_previous_sim_positions` **before** `SimSet::SpatialRebuild`;
- the demon spawner (`spawn_initial_burst`, `tick_spawner`) **before**
  `SimSet::SpatialRebuild` — the edge is what puts a sync point between the spawn commands
  and the grid rebuild, so a demon is in the grid on the tick it is born. With no edge the
  flush landed on either side of the rebuild depending on the executor;
- `move_moving_entities` **after** `SimSet::HumanBehavior` — behavior may move `SimPosition`
  itself (the demon lunge), and a snapshot taken after that would flatten one tick of
  interpolation.

**Movable** — `{speed, path: VecDeque<IVec2>, state}` with `MovableState: Idle |
Pathfinding(goal) | Moving(goal) | PathfindingError`. `to_pathfinding` queues the search and
keeps the current path (repath on the move — `navigation-deep`); `to_idle` is the only
transition that stops movement.

## Spatial grids

**`SpatialGrid<T>`** — a uniform grid per marker type (`Demon`, `Human`), **60 m cells**
(≥ the largest search radius, so a radius query is a 3×3 cell walk).

Cells hold **entities only** — a candidate's position is read live from `SimPosition` through
the `pos_of` closure every query takes; storing `Vec2` in cells would go stale by up to a
cell size. API: `nearest_in_range_where` / `for_each_in_cells_around` / `for_each_in_rect` /
`any_in_cells_around`.

**A tie in distance is broken by `PawnId`, not by traversal order** — `order_of`, a second
closure the search calls *only* on exact equality, so the key never costs anything on the
common path. Ties are rare (5 in 3.84 M searches over five simulated minutes on Tula), but
the alternative rule is "whoever the cell walk reached last", i.e. the history of spawns and
deaths. The ordering doctrine is `movement/order.rs`; why it matters is `determinism`.

**The human grid is incremental, the demon grid is rebuilt.**

- Humans (~20 000): `On<Add, Human>` / `On<Remove, Human>` observers cover spawn and
  death/despawn, and a step or a separation push reports itself through
  `SpatialGrid::moved`, which relocates only across a 60 m cell boundary — the cost scales
  with crossings, not population, **and the rule lives in the grid rather than in each
  mover**.
- Demons (~100): a full rebuild per tick (`rebuild_demon_grid`) is cheaper than bookkeeping,
  and the lunge moves demon `SimPosition` outside the mover system anyway.

## The decision ladder

`human/decide.rs`, `demon/decide.rs`. A species' rules live in a **pure** `decide(&…Sense,
…) -> …Action`: plain values in, one enum variant out, no `Commands`, no queries, tested
without an `App`.

(`Entity` appears in exactly one place — `demon::decide::Victim`, so the ladder can *name*
the victim it picked; the rule's point, no world and no queries, is intact.)

`behavior.rs` only **applies** the answer — swap tags, queue a path request, trigger the kill
event — so the *order* of the rungs, which is half of what these rules are, is readable in
one place instead of spread over `continue`s.

Three things deliberately stay outside `decide`:

- **Expensive senses** are asked lazily through a closure, at most once: the demon's
  `line_of_sight` only once the distance already permits a lunge; the human's threat through
  the `ThreatProbe` the ladder itself picks (exact nearest-demon search on decision ticks,
  cell occupancy between them).
- **Anything that touches the world** stays in the system, but its *terms* come out of
  `decide`: the target-switch search is run by `chase`, while its radius and chaser limit are
  a `SwitchRule` the ladder returned.
- **RNG rolls** stay in `behavior.rs` — the decision stream must advance on exactly the ticks
  it advanced on before (`determinism`). Not every roll hangs off a rung: the flee repath
  period (`human::panic`) and the devour pause (`demon::on_demon_caught_human`, an
  **observer**) are rolled on a *transition*, and each still costs the pawn one decision
  number.

## The wander skeleton

`movement/wander.rs` — the *order* of one target-picking step, shared by both species:

`ready_to_pick` (the state guard) → the species' own policy → `point_in_cone` /
`clamp_to_map` → `request_wander_path` (sift + `to_pathfinding`) → `heading_towards`.

Only **where** a pawn wants to go is per-species; the rest was written twice, and the
sift-then-request tail four times. The chase repath (`demon/behavior.rs`) now goes through
`request_wander_path` too; the flee repath (`human/behavior.rs`) is the one hand-written copy
left. The skeleton deliberately does **not** open the `SimRng`
— the species does, in its own place, or the decision stream would advance on pawns that
took no decision this tick. `WANDER_MAP_MARGIN` is the one map inset, in `settings.rs`; it
used to be declared in both species files.

## Human

States in `human/behavior.rs`, rules in `human/decide.rs`: **Wander** ⇄ **Flee**, plus
**Escape** and the terminal **Corpse**.

### Wander

`WanderPause` 2–10 s *between* walks, zero at spawn. Then:

- **80 % — a building errand** anywhere in the city. These are the long routes and the real
  pathfinding load.
- **20 % — a stroll** 20–40 m nearby.

The one exception is the first target after calming down from panic (see **PanicRecoil**).

**WanderHeading** — the direction a human is walking, kept between walks. Every next target
is picked inside a `WANDER_CONE` (60°) around it; a building errand samples
`WANDER_BUILDING_TRIES` (8) random buildings and takes the first inside the cone. Without
the heading each pick was uniformly random and pawns wobbled in place. `flee` rewrites it to
the away-vector on every repath, so a calmed human resumes facing away from the demon rather
than on its stale pre-panic course.

**`HumanFirstWanderTag`** — the very first target after spawn is always the *near* stroll,
never a building errand; dropped when that target is picked. All 20 000 humans queue their
first path in the same frame: **with errands first the on-screen pawns took 3.9 s to route,
with strolls first — 0.15 s.** `PanicRecoil` overrides it.

**Pace** — a human's personal speed multiplier, rolled once at spawn and stored
**normalized** (−1…+1); effective speed is `base × (1 + Pace × HumanStyle::spread)`, applied
to *both* bases (walk and flee) — all three writers of a human's `Movable::speed` go through
`Pace::speed`.

- Normalized storage is what makes the **Speed spread** slider widen/narrow the ordering the
  crowd already rolled instead of re-dealing it.
- A component, not something derived from `Movable::speed` — that field is overwritten on
  every Wander ⇄ Flee transition.
- Ceiling 35 % is derived: above it the fastest humans outrun the slowest demon setting.
- `sync_human_pace` applies slider moves (`resource_changed`), picking the base off
  `Has<HumanFleeTag>`.

### Flee

Enter when a demon is within `HUMAN_PANIC_RADIUS` (60 m). Repath every 0.7–1.2 s, stepping
40–60 m away from the nearest demon. Calm-down at ×1.5 radius hysteresis
(`RADIUS_HYSTERESIS`).

**The Wander → Flee check (`panic`) is inverted**: each demon collects neighbours from the
human grid instead of every wanderer polling the demon grid, so its cost tracks the crowd
near demons, not the city population.

**The exact nearest-demon search runs only on a fleeing human's decision ticks** — between
them it only checks demon-grid *cell occupancy* (`any_in_cells_around`). The every-tick
exact search used to cost 40 % of the sim tick.

**Flee fan** — a non-chased fleeing human rotates its away-vector by a deterministic
per-entity angle (±0.6 rad) so crowds spread instead of forming a column. Actively chased
humans flee straight.

A fleeing human carries **`UrgentPath`** (it comes and goes with `HumanFleeTag`) — the
dispatcher's "may not wait for the camera" marker; see `navigation-deep`.

**Escape** — a fleeing human within `ESCAPE_MARGIN` of the map border despawns,
`telemetry.escaped += 1`. It is a despawn inside `FixedUpdate`, where the chained `SimSet`s
give it a sync point (see CLAUDE.md, "Where a mass despawn may happen").

### PanicRecoil

A unit vector *toward* the demon, written on **every flee repath** (`FleeAction::Flee { ban
}`) and **never queried live**: `pick_wander_targets` must stay off the demon grid, and by
calm-down the demon is already >90 m away.

It is born in `human::decide`, where the demon's position is an input — the calm-down branch
fires *because* the search found nobody, so it has nothing to build a ban from and leaves the
stored one alone. A human who panics and calms down before the first repath therefore carries
no ban (the old one pointed along a stale stroll course, at nothing).

While it is on, the next target must be an errand clearing two filters in
`pick_building_ahead`:

- not within `RECOIL_CONE` (±45°) of the recoil vector;
- not closer than `RECOIL_MIN_ERRAND` (90 m) — a nearby building just outside the cone
  reproduces the short walk being ruled out.

Rejected candidates are dropped **before** the "best-aligned of the 8" fallback, which used
to hand back a building nearly 180° from the heading — straight at the demon. Nothing
acceptable → re-roll next frame (**never a stroll**); dropped at the first successful pick.

**Why it is stored rather than synthesised.** It used to be built at the calm-down site as
the negated `WanderHeading`, which dragged the **flee fan** in with it: ±0.6 rad (34°) of the
±45° cone spent before the ~13° of staleness — over budget on the comments' own numbers, and
silently broken by any widening of the fan. Taking the ban **before** the fan is applied
removes the coupling outright: the two angles no longer have to be compared at all.

### Corpse

**`CorpseTag`** — a killed human: behavior/movement components removed, dark lying sprite at
`Z_CORPSE`. Not in the human spatial grid (the grid filters on `Human`).

The transition is **`human::to_corpse`**, one entry point, and it is where a corpse is
defined — the kill observer in `demon/` only reports that it happened. Each module takes back
its own components: `to_corpse` drops the human behaviour and calls
**`movement::strip_movement`**, which drops the movement footprint via
`remove_with_requires::<Movable>` (so `Movable`'s `#[require]` stays the single record of what
a movable entity drags along) plus the runtime pieces `#[require]` cannot reach — request,
tick stamp, retire deadline, destination claim, `UrgentPath`.

Deliberately kept on the body: `PawnId` / `WanderIndex` (a pawn's identity, useful in debug)
and `Pace` / `WanderHeading` (the spawn roll — unreadable without `Movable`).

## Demon

States in `demon/behavior.rs`, rules in `demon/decide.rs`: **Wander** (target biased away
from the portal) → **Chase** → **Devour** → Wander. A demon carries `UrgentPath` always.

### Wander

**A point in the `DEMON_WANDER_CONE` (1.3 rad half-angle, ≈75°) around the away-from-portal
vector, `DEMON_WANDER_RANGE` (40–120 m) out** — `pick_wander_targets` (`demon/systems.rs`)
builds the course as `(sim_position − portal_pos).normalize_or(<random angle>)` and hands it
to `point_in_cone`. "Away from the portal" is the whole of the species' policy; everything
around it is the shared skeleton. At the map edge `clamp_to_map` turns the direction inward —
that is all the system's comment about the border means; there is no separate border rule.

Against the human's stroll (60° half-angle, 20–40 m): a wider cone and a walk two to three
times longer, and **no errand branch at all** — a demon never picks a building, so the 80/20
roll and `pick_building_ahead` are human-only.

**No `WanderPause` analogue and no `WanderHeading`.** The demon takes its next target on the
very tick `Movable::to_idle` hands `NeedsWanderTarget` back, so it never stands between
walks; and its course is recomputed from the portal on every pick instead of being kept, so
`heading_towards` — the skeleton's last step — is never called for a demon.
`WanderIndex::ready()` at spawn puts the first target on the demon's first tick.

**Three draws from the decision stream on every pick, in this order: fallback angle → cone
rotation → distance.** The first is the argument of `normalize_or`, which takes a *value*,
not a closure — it is rolled even when the demon is far from the portal and the fallback is
thrown away. Making it lazy would drop a draw on nearly every pick and desynchronise every
recorded replay (`determinism`): if it ever changes, it changes as a replay-breaking change,
not as a tidy-up.

### Acquisition and give-up

**Enter when the nearest human within `DEMON_AGGRO_RADIUS` (45 m) still has a free claim
slot** — `acquire_targets` walks the *human* grid (`nearest_in_range_where` with the
`ChaseClaims::is_full` filter), so a victim that already has its two chasers is skipped, not
queued behind them. A demon takes aggro on the first tick it exists, the initial burst
included.

**Give up past ×1.5 that radius (67.5 m)** — `RADIUS_HYSTERESIS`, the same enter/exit pattern
as the human's calm-down. It is the **first rung of `decide`** after the dead-target check:
checked before the kill, the lunge and the repath gate, so a demon dragged out of the ring
leaves on that same tick, and `GaveUp` is the one exit that frees its claim slot.

The two species' radii are not symmetric, and the ordering is worth holding: a human panics
at `HUMAN_PANIC_RADIUS` (60 m), a demon acquires at 45 m, so the demon almost always starts
its chase on an already-fleeing target.

### Chase claims

**Max 2 chasers per target**, counted by `ChaseClaims` (`demon/claims.rs`) — a value rebuilt
each tick from the `ChaseTarget`s of the demons in the query; **there is no standing claim
between ticks**.

Which exits from a chase free a slot is `ChaseAction::releases_claim`, and the rule is
asymmetric: only **GaveUp** releases, because after `Kill` and `LostTarget` the victim is gone
and nobody will claim it anyway. A **switch** moves one slot instead, with
`ChaseClaims::transfer` — the operation that replaced the `release` + `claim` pair the
exhaustive match could not see.

### Repath and the switch rung

Repath throttle 0.4 s (`DEMON_CHASE_REPATH`), and on that same tick the demon may **switch** target — **a rung of
the ladder, not a tail after it**: the search is a lazy sense (`better_victim`) that `decide`
asks only on the repath rung, and the answer is `ChaseAction::Switch { to: Victim }`.

- Sharing its target: it takes any *unclaimed* human no farther than **×1.5** its current
  distance — this is what breaks up a pincer.
- Otherwise: whoever is nearer than **×0.7** of the current target — the anti-flip-flop
  margin, since near-equidistant victims would trade the demon back and forth every repath
  tick.
- Both cases require **`line_of_sight`** to the candidate, checked on the search winner only.

**These three live in `demon/decide.rs`, not `settings.rs`, on purpose.**
`MAX_CHASERS_PER_TARGET`, `SWITCH_DISTANCE_FACTOR`, `CLOSER_SWITCH_FACTOR` are rules of the
ladder — same placement as `FLEE_STEP`/`FLEE_SPREAD` in `human/decide.rs`, while the metres the
ladder consumes (`DEMON_AGGRO_RADIUS`, `DEMON_LUNGE_RANGE`, `KILL_DISTANCE`,
`RADIUS_HYSTERESIS`) do come from `settings.rs`. What moves to `settings.rs` is a knob or a
constant both species declare (`WANDER_MAP_MARGIN`, commit `432eabf`). Don't relocate them to
"comply" with the tuning-constants rule.

**A chaser with no first path yet skips the repath tick and waits.** Repathing cancels the
in-flight search, and whenever the pipeline answers slower than the victim changes tiles the
demon cancelled every answer and stood frozen at the portal. Once the first path lands,
coasting covers the repath gaps and cancelling becomes safe.

**The throttle clock runs only on the rungs the ladder reaches.** `chase` *asks*
`ChaseRepath` (`remaining() <= time.delta()`) and ticks it on `Hold`, `Repath` and `Switch`
only — `Lunge` and the wait freeze it, and nothing else in the code ticks it. For the wait
that is the point: the timer is inserted fresh at Wander → Chase, so the first path gets a
whole throttle period of walking instead of being repathed away the tick after it lands.
After a **lunge** the freeze delays nothing — the lunge's `to_idle` empties the path and an
answer arriving meanwhile is dropped (`accept_answer` accepts only in `Pathfinding`), so on
the tick the lunge is cancelled the demon is `Idle`, `needs_first_path` holds, and the
repath/switch rung fires whatever the timer says. What survives is the throttle's *phase*:
that repath advances the frozen remainder rather than a fresh period, so the **next** repath —
and with it the next chance to switch victim — may come one tick later instead of
`DEMON_CHASE_REPATH` later. The throttle is a floor between *timer-driven* repaths, not
between repaths.

### Lunge, kill, devour

**Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight`, the demon drops its
path and steps `SimPosition` straight at the target at its speed plus `DemonStyle::lunge`.
Without it a chase never converts: a tile path aims at the tile *center* while the victim
keeps moving inside it, and the last ~1.4 m is never closed. A lunging demon carries
**`DemonLungeTag`** (set/cleared in `chase`); `draw_lunge_paths` draws its arrow at the
victim's live position. Lunging demons are exempt from separation.

**Kill** at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); a `killed_this_tick`
HashSet dedupes double kills within one command flush.

**What each exit from a chase strips is one list plus one exception.** The list is
`ChaseComponents` (`demon/components.rs`) — the four chase components, removed whole by
both exits. The kill observer removes `PathfindingTask` / `PathfindingRequest` on top,
because an answer landing in Devour would `to_moving` the demon off the corpse; it may,
because it calls `Movable::to_idle` first. `back_to_wander` touches no `Movable`, so it
must **not** cancel the search: a demon left in `MovableState::Pathfinding` with no
request and no `NeedsWanderTarget` never wanders again.

**Devour** — pause `DEMON_DEVOUR_PAUSE` (1.5–2 s) with a sine **pulse** ×1 →
`DEVOUR_PULSE_MAX_SCALE` (1.5) over `DEVOUR_PULSE_PERIOD` (0.5 s), scale reset on exit.
All four numbers live in `settings.rs`, none inline in `demon/behavior.rs`.

The pause is rolled **in the kill observer, from the demon's own decision stream** — so a
kill advances the killer's `WanderIndex` by one, the only such advance outside a
decide-driven system. It stays deterministic because the observer is reached from `chase`'s
command flush inside the same `FixedUpdate` tick; moving the trigger to `Update`, or rolling
the pause from a shared generator, breaks replay (`determinism`).

### Speed

**`DEMON_SPEED`** — one base for every state, `HUMAN_FLEE_SPEED × 1.35` (true of the *average*
human since `Pace`).

**Do not reintroduce per-state demon speeds.** The only multipliers are the two user ones:
`DemonStyle::speed` (whole demon) and `DemonStyle::lunge` (lunge phase only — applied at the
one line in `chase` that steps `SimPosition`, never written into `Movable::speed`).
`Movable::speed` is written once, at spawn; `sync_demon_speed` applies slider moves
(`resource_changed`).

### The spawner

**`DemonSpawner`** — initial burst at the portal rim, then one demon per interval up to the
cap. Runs in `FixedUpdate` so a restart re-fires the burst for free. Cap and interval live in
**`DemonStyle { cap, interval, speed, lunge }`** (sliders of the Sim tab's Demon section,
persisted); `DEMON_CAP` / `DEMON_SPAWN_INTERVAL` are only its `Default`.

- The burst is capped too (`DEMON_INITIAL_BURST.min(cap)`, fanned over the reduced count).
- Lowering the cap never despawns demons already out.
- The timer's period is re-synced inside `tick_spawner`, because restart and city switch
  rebuild `DemonSpawner` whole.

**A demon acts from the first tick it exists**, the initial burst included — held by the
schedule, not by luck: the spawner sits `.before(SimSet::SpatialRebuild)` and in
`SimPipeline::BothModes` (`demon/mod.rs`). A
`DemonSpawnPause` (0.5–3 s staging) existed and was removed on request; staging an entrance
again means a new component, not reviving that one.

**The spawner runs only in `PlayPhase::Live`, and that is an invariant, not a detail.** It
hands out `PawnId`s from a counter that `WorldStarted` — announced on entering `Live` —
resets to zero, so a burst fired before the announcement gets its numbers dealt a second
time. Two demons then share a `PawnId`, which breaks both the pawn's RNG stream and the
deterministic dispatcher's queue key (it died on a duplicate key ~30 ticks in). Relying on
"warmup keeps the world paused" was not enough — that pause belongs to `sim_time` and space
unpauses it. Matching precondition on the reset: **no demon may be alive when a run starts**
(`demon::on_world_started` says so).

## Telemetry

`{killed, escaped}`, BRP-readable; `killed` is what the **Souls reaped** HUD counter shows
(`ui/stats.rs`, first in the left column, outside the settings tabs — panel internals live
in the **ui-panels skill**). The Sim tab's **World** section is a different thing: seed and
determinism row, no telemetry.

**Invariant (check paused): `killed + escaped + alive == HUMAN_COUNT`.** At high sim speed
BRP reads are skewed — pause before asserting.
