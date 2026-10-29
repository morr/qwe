---
name: species-behavior
description: Use when working on pawn behaviour in qwe — the human and demon decision ladders (human/decide.rs, demon/decide.rs), their behavior.rs state machines, wander/flee/chase/devour, the flee fan, PanicRecoil, Pace, chase claims and the lunge, the demon spawner, corpses, the spatial grids and the SimSet order — and on how a pawn is drawn: the silhouette atlas and its glyphs (silhouette/, silhouette/figure.rs), the screen-size floor and the two LOD systems, the portal vortex and its stain (portal.rs). Deep detail behind CONTEXT.md's Simulation and Look summaries.
---

# Species behaviour — deep detail

Detail behind `src/human/`, `src/demon/`, `src/movement/wander.rs` and `src/spatial.rs`;
the drawing half of a pawn — `src/silhouette/`, `src/portal.rs`, the looks — lives in
`references/look.md`.
`CONTEXT.md` names the states and the invariants; this file is the mechanism behind them and
the measured reason each rung sits where it does.

**Changing that mechanism is changing this file, in the same turn** — a new rung, a moved
threshold, a changed claim rule, a component that stops existing. This file is what the next
session reads instead of the code. The term itself still goes to `CONTEXT.md`.

Neighbouring skills: pathfinding, separation and destination slots are `navigation-deep`
(`references/crowd.md` for the crowd lab); the RNG stream every decision draws from is
`determinism`.

## References

- **`references/look.md`** — how a pawn is drawn: the silhouette atlas and its glyphs, the
  corpse pose and tint, the panic tint, the demon's halo, blood (pool, spatter, growth),
  the screen-size floor and the two LOD systems, and the portal vortex with its stain.

## The fixed step

**SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`): `SpatialRebuild →
DemonBehavior → HumanBehavior`; `SimPosition` / `PreviousSimPosition`, the pinned order
around them and the reason demons act first are `CONTEXT.md`'s. The one lesson that lives
here: **the demon spawner sits before `SimSet::SpatialRebuild` for the edge** — the edge is
what puts a sync point between the spawn commands and the grid rebuild, so a demon is in
the grid on the tick it is born. With no edge the flush landed on either side of the
rebuild depending on the executor.

**Movable** — the state machine and its transitions are `CONTEXT.md`'s. What lives here:
`to_idle` cancelling an undispatched `PathfindingRequest` (with its `RequestedAt`) is what
keeps a human calming down from leaving a flee search to be computed and thrown away.

## Spatial grids

**`SpatialGrid<T>`** — a uniform grid per marker type (`Demon`, `Human`), **60 m cells**;
cells hold **entities only**, positions read live from `SimPosition` through the `pos_of`
closure every query takes — storing `Vec2` in cells would go stale by up to a cell size.

**The human grid is incremental, the demon grid is rebuilt.**

- Humans (~20 000): `On<Add, Human>` / `On<Remove, Human>` observers cover spawn and
  death/despawn, and a step or a separation push reports itself through
  `SpatialGrid::moved`, which relocates only across a 60 m cell boundary — the cost scales
  with crossings, not population, **and the rule lives in the grid rather than in each
  mover**.
- Demons (~100): a full rebuild per tick (`rebuild_demon_grid`) is cheaper than bookkeeping,
  and the lunge moves demon `SimPosition` outside the mover system anyway.

**A tie in distance is broken by `PawnId`, not by traversal order** — `order_of`, a second
closure the search calls *only* on exact equality, so the key never costs anything on the
common path. Ties are rare (5 in 3.84 M searches over five simulated minutes on Tula), but
the alternative rule is "whoever the cell walk reached last", i.e. the history of spawns and
deaths. The ordering doctrine is `movement/order.rs`; why it matters is `determinism`.

## The decision ladder

`human/decide.rs`, `demon/decide.rs`. A species' rules live in a **pure** `decide(&…Sense,
…) -> …Action`, and `behavior.rs` only **applies** the answer — swap tags, queue a path
request, trigger the kill event — so the *order* of the rungs, which is half of what these
rules are, is readable in one place instead of spread over `continue`s. The shape is
`CONTEXT.md`'s. (`Entity` appears in exactly one place — `demon::decide::Victim`, so the
ladder can *name* the victim it picked; the rule's point, no world and no queries, is
intact.)

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

`movement/wander.rs` — the *order* of one target-picking step, shared by both species (the
chain is `CONTEXT.md`'s). Only **where** a pawn wants to go is per-species — for the human
that policy is `human/systems.rs::choose_target`, a pure function; the demon's is the two
lines around `away` in its own system. The skeleton deliberately does **not** open the
`SimRng` — the species does, in its own place, or the decision stream would advance on
pawns that took no decision this tick. The chase repath (`demon/behavior.rs`) goes through
`request_wander_path` too; the flee repath (`human/behavior.rs`) is the one hand-written
copy left. `WANDER_MAP_MARGIN` is the one map inset, in `settings.rs`.

## Human

States in `human/behavior.rs`, rules in `human/decide.rs`: **Wander** ⇄ **Flee**, plus
**Escape** and the terminal **Corpse**.

### Population size

**`PopulationSize`** (`human/components.rs`) — the count `spawn_population` settles; the
resource exists so a headless scene can run 64 pawns instead of 20 000 (the whole shape is
`CONTEXT.md`'s). The two readers stay in step by a type, not by discipline: both take the
**`PopulationSpawn`** `SystemParam` (`human/systems.rs` — navmesh, `HumanStyle`, `WorldSeed`,
`PopulationSize`, `Silhouettes`) and call its `spawn`, so neither can read a different set.

- **An app that wants another size inserts it after `add_plugins`** —
  `determinism::replay::replay_app` does exactly that, and swapping the two lines would
  silently put 20 000 humans back into the replay test.
- Demo stands do not use it: they call `spawn_population` directly (`crowd_demo`) because
  `HumanPlugin` would settle the whole city.

### Wander

**`WanderPause` is drawn per arrival, not stood at every one** — `roll_wander_pause`
(`human/systems.rs`), the share and the 2–10 s range are `CONTEXT.md`'s. A target is a
point of the passer-by's route, not the errand he went out for.

- **The roll goes on the target, not on the human.** A permanently hurrying fifth of the
  population would be two sorts of pedestrian instead of one who sometimes stops.
- **It is thrown at the choice of the next target** — the tail of `pick_wander_targets`,
  out of the same `WanderIndex::next` decision stream as the target itself, because that is
  where the pause is reloaded anyway; arrival knows nothing about the next target.
- **A zero pause costs no step**: a `Timer` in `Once` mode is finished on its first `tick`
  (`elapsed >= duration`), so "no pause" needs no separate state. Zero at spawn is the same
  mechanism — every human walks from the first frame.

Unconditional in exactly one place: **calm-down** (`FleeAction::CalmDown`,
`human/behavior.rs`) always stands the full 2–10 s, no share roll.

**WanderHeading** — the direction a human is walking, kept between walks (`CONTEXT.md`).
Every next target is picked inside a `WANDER_CONE` (±60°) around it; a building errand
samples `WANDER_BUILDING_TRIES` (8) random buildings and takes the first inside the cone.

**`HumanFirstWanderTag`** — the very first target after spawn is always the *near* stroll,
never a building errand; dropped when that target is picked. `PanicRecoil` overrides it.

**Pace** — a human's personal speed multiplier (the normalized storage and the 35 % ceiling
are `CONTEXT.md`'s); effective speed is `base × (1 + Pace × HumanStyle::spread)`, applied
to *both* bases (walk and flee). Every writer of a human's `Movable::speed` goes through
`Pace::speed`: the spawn (`spawn_population`), the **Speed spread** slider
(`sync_human_pace`), and both sides of the Wander ⇄ Flee transition — `panic` on the way in,
`FleeAction::CalmDown` on the way out. It is a component, not something derived from
`Movable::speed` — that field is overwritten on every Wander ⇄ Flee transition.

`sync_human_pace` applies slider moves, picking the base off `Has<HumanFleeTag>`. Its gate
is `systems::spread_changed` — a `Local`-backed compare of `HumanStyle::spread`, not
`resource_changed::<HumanStyle>`: the resource also carries `body_radius` (a slider on the
Nav tab), and gating on the whole resource re-tuned all 20 000 humans on every 0.01 m step
of a radius drag.

### Flee

The panic radius, the repath period, the hysteresis, the inverted `panic` check and the
flee fan are `CONTEXT.md`'s. What lives here:

- **The first repath happens on the panic tick itself** — `panic` inserts `FleeRepath`
  already due (`Timer::almost_finish`), so `flee`, chained right after it, takes the decision
  rung on that same tick; otherwise a walking human (`Moving`, so no `needs_path`) would
  keep its stroll path at flee speed for up to 1.2 s.
- **The exact nearest-demon search runs only on a fleeing human's decision ticks** — between
  them it only checks demon-grid *cell occupancy* (`any_in_cells_around`). The every-tick
  exact search used to cost 40 % of the sim tick.
- A fleeing human carries **`UrgentPath`** (it comes and goes with `HumanFleeTag`) — the
  dispatcher's "may not wait for the camera" marker; see `navigation-deep`.

**Escape** — a fleeing human within `ESCAPE_MARGIN` of the map border despawns,
`telemetry.escaped += 1`. It is a despawn inside `FixedUpdate`, where the chained `SimSet`s
give it a sync point (see CLAUDE.md, "Where a mass despawn may happen").

### PanicRecoil

The vector, the two filters (`RECOIL_CONE` ±45°, `RECOIL_MIN_ERRAND` 90 m), the
never-a-stroll rule and the no-buildings exception are `CONTEXT.md`'s. What lives here:

- **Why it is stored rather than synthesised.** It used to be built at the calm-down site as
  the negated `WanderHeading`, which dragged the **flee fan** in with it: ±0.6 rad (34°) of
  the ±45° cone spent before the ~13° of staleness — over budget on the comments' own
  numbers, and silently broken by any widening of the fan. Taking the ban **before** the fan
  is applied removes the coupling outright: the two angles no longer have to be compared at
  all.
- It is born in `human::decide`, where the demon's position is an input — the calm-down
  branch fires *because* the search found nobody, so it has nothing to build a ban from and
  leaves the stored one alone. Since the first repath lands on the panic tick, a fleeing
  human practically always carries a ban; the no-ban state survives only as the initial
  one, before a human has ever panicked.
- Rejected candidates are dropped **before** the "best-aligned of the 8" fallback, which
  used to hand back a building nearly 180° from the heading — straight at the demon;
  nothing acceptable → re-roll next frame, dropped at the first successful pick.
- On the no-buildings fallback stroll the cone is tested **after** `clamp_to_map`, which
  turns the direction around at the map edge.

### Corpse

The transition is **`human::to_corpse`**, one entry point, and it is where a corpse is
defined — the kill observer in `demon/` only reports that it happened. `to_corpse` calls
**`movement::strip_movement`**, which drops the movement footprint via
`remove_with_requires::<Movable>` (so `Movable`'s `#[require]` stays the single record of
what a movable entity drags along) plus the runtime pieces `#[require]` cannot reach —
request, tick stamp, retire deadline, destination claim, `UrgentPath`.

Deliberately kept on the body: `PawnId` / `WanderIndex` (a pawn's identity, useful in
debug) and `Pace` / `WanderHeading` (the spawn roll — unreadable without `Movable`).

The drawn half — pose, tint, the blood children — is `references/look.md`; the summary is
`CONTEXT.md`'s **Corpse look** entry.

## Demon

States in `demon/behavior.rs`, rules in `demon/decide.rs`: **Wander** (target biased away
from the portal) → **Chase** → **Devour** → Wander. A demon carries `UrgentPath` always,
and `movement::BodyScale::DEMON` — its body is the one thing movement would otherwise have
to infer from the species. The look — the `Ember` glyph and the halo — is
`references/look.md`.

### Wander

**A point in the `DEMON_WANDER_CONE` (1.3 rad half-angle, ≈75°) around the away-from-portal
vector, `DEMON_WANDER_RANGE` (40–120 m) out** — the policy is `CONTEXT.md`'s. At the map
edge `clamp_to_map` turns the direction inward — that is all there is to the border, there
is no separate border rule. Against the human's stroll (60° half-angle, 20–40 m): a wider
cone and a walk two to three times longer, and **no errand branch at all** — a demon never
picks a building, so the 80/20 roll and `pick_building_ahead` are human-only. No
`WanderPause` analogue and no `WanderHeading` — the next target is taken the very tick
`Movable::to_idle` hands `NeedsWanderTarget` back. `WanderIndex::ready()` at spawn puts the
first target on the demon's first tick.

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

**Give up past ×1.5 that radius** — `RADIUS_HYSTERESIS`, the same enter/exit pattern as the
human's calm-down. It is the **first rung of `decide`** after the dead-target check: checked
before the kill, the lunge and the repath gate, so a demon dragged out of the ring leaves on
that same tick, and `GaveUp` is the one exit that frees its claim slot.

The two species' radii are not symmetric, and the ordering is worth holding: a human panics
at `HUMAN_PANIC_RADIUS` (60 m), a demon acquires at 45 m, so the demon almost always starts
its chase on an already-fleeing target.

### Chase claims

**Max 2 chasers per target**, counted by `ChaseClaims` (`demon/claims.rs`) — a value rebuilt
each tick from the `ChaseTarget`s of the demons in the query; **there is no standing claim
between ticks**. Only **GaveUp** releases a slot, because after `Kill` and `LostTarget` the
victim is gone and nobody will claim it anyway; a **switch** moves one slot instead, with
`ChaseClaims::transfer`.

### Repath and the switch rung

Repath throttle 0.4 s (`DEMON_CHASE_REPATH`), and on that same tick the demon may **switch**
target — **a rung of the ladder, not a tail after it**: the search is a lazy sense
(`better_victim`) that `decide` asks only on the repath rung, and the answer is
`ChaseAction::Switch { to: Victim }`.

- Sharing its target: it takes any *unclaimed* human no farther than **×1.5** its current
  distance — this is what breaks up a pincer.
- Otherwise: whoever is nearer than **×0.7** of the current target — the anti-flip-flop
  margin, since near-equidistant victims would trade the demon back and forth every repath
  tick.
- Both cases require **`line_of_sight`** to the candidate, checked on the search winner only.

**These three live in `demon/decide.rs`, not `settings.rs`, on purpose.**
`MAX_CHASERS_PER_TARGET`, `SWITCH_DISTANCE_FACTOR`, `CLOSER_SWITCH_FACTOR` are rules of the
ladder — same placement as `FLEE_STEP`/`FLEE_SPREAD` in `human/decide.rs`, while the metres
the ladder consumes (`DEMON_AGGRO_RADIUS`, `DEMON_LUNGE_RANGE`, `KILL_DISTANCE`,
`RADIUS_HYSTERESIS`) do come from `settings.rs`. What moves to `settings.rs` is a constant
more than one owner reads — both species declaring it (`WANDER_MAP_MARGIN`), or a knob the
panel and the model share. A knob's own `_MIN`/`_MAX`/`_STEP` range is *not* such a
constant: it bounds its resource's field — the panel only renders it — so it belongs beside
that resource. `HumanStyle` and `DemonStyle` do not clamp on read; their ranges are the
declaration of what the field may hold, not a guard on it. Don't relocate anything here to
"comply" with the tuning-constants rule.

**A chaser with no first path yet skips the repath tick and waits.** Repathing cancels the
in-flight search, and whenever the pipeline answers slower than the victim changes tiles the
demon cancelled every answer and stood frozen at the portal. Once the first path lands,
coasting covers the repath gaps and cancelling becomes safe.

**The throttle clock runs only on the rungs the ladder reaches.** `chase` *asks*
`ChaseRepath` (`remaining() <= time.delta()`) and ticks it on `Hold`, `Repath` and `Switch`
only — `Lunge` and the wait freeze it, and nothing else in the code ticks it. The timer is
inserted fresh at Wander → Chase, so the first path gets a whole throttle period of walking
instead of being repathed away the tick after it lands. After a **lunge** the freeze delays
nothing — the lunge's `to_idle` empties the path and an answer arriving meanwhile is dropped
(`accept_answer` accepts only in `Pathfinding`), so on the tick the lunge is cancelled the
demon is `Idle`, `needs_first_path` holds, and the repath/switch rung fires whatever the
timer says. What survives is the throttle's *phase*: that repath advances the frozen
remainder rather than a fresh period, so the **next** repath — and with it the next chance
to switch victim — may come one tick later instead of `DEMON_CHASE_REPATH` later. The
throttle is a floor between *timer-driven* repaths, not between repaths.

### Lunge, kill, devour

**Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight`, the demon drops its
path and steps `SimPosition` straight at the target at its speed plus `DemonStyle::lunge`.
Without it a chase never converts: a tile path aims at the tile *center* while the victim
keeps moving inside it, and the last ~1.4 m is never closed. A lunging demon carries
**`DemonLungeTag`** (set/cleared in `chase`); `draw_lunge_paths` draws its arrow at the
victim's live position.

**Kill** at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); a
`killed_this_tick` HashSet dedupes double kills within one command flush. The observer also
**releases a soul** at the victim's `SimPosition` — the visible side of `Telemetry::killed`;
the spark is `CONTEXT.md`'s **Soul** entry.

**What each exit from a chase strips is one list plus one exception.** The list is
`ChaseComponents` (`demon/components.rs`) — the four chase components, removed whole by
both exits. The kill observer removes `PathfindingTask` / `PathfindingRequest` on top,
because an answer landing in Devour would `to_moving` the demon off the corpse; it may,
because it calls `Movable::to_idle` first. `back_to_wander` touches no `Movable`, so it
must **not** cancel the search: a demon left in `MovableState::Pathfinding` with no request
and no `NeedsWanderTarget` never wanders again.

**Devour** — pause `DEMON_DEVOUR_PAUSE` (1.5–2 s) with a sine **pulse** ×1 →
`DEVOUR_PULSE_MAX_SCALE` (1.5) over `DEVOUR_PULSE_PERIOD` (0.5 s), scale reset on exit.
All four numbers live in `settings.rs`, none inline in `demon/behavior.rs`. The pause is
rolled **in the kill observer, from the demon's own decision stream** — so a kill advances
the killer's `WanderIndex` by one, the only such advance outside a decide-driven system. It
stays deterministic because the observer is reached from `chase`'s command flush inside the
same `FixedUpdate` tick; moving the trigger to `Update`, or rolling the pause from a shared
generator, breaks replay (`determinism`).

### Speed

**`DEMON_SPEED`** — one base for every state, `HUMAN_FLEE_SPEED × 1.35` (true of the
*average* human since `Pace`); **no per-state demon speeds** — the rule is `CONTEXT.md`'s.
The `DemonStyle::lunge` multiplier is applied at the one line in `chase` that steps
`SimPosition`, never written into `Movable::speed`. `Movable::speed` is written once, at
spawn; `sync_demon_speed` applies slider moves (`resource_changed`).

### The spawner

**`DemonSpawner`** — the burst, the interval, the cap in **`DemonStyle { cap, interval,
speed, lunge }`** (sliders of the Sim tab's Demon section, persisted; `DEMON_CAP` /
`DEMON_SPAWN_INTERVAL` are only its `Default`) and the `PlayPhase::Live` invariant are
`CONTEXT.md`'s. The burst is capped too (`DEMON_INITIAL_BURST.min(cap)`, fanned over the
reduced count), and the timer's period is re-synced inside `tick_spawner`, because restart
and city switch rebuild `DemonSpawner` whole.

**A demon acts from the first tick it exists**, the initial burst included — held by the
schedule, not by luck: the spawner sits `.before(SimSet::SpatialRebuild)` and in
`SimPipeline::BothModes` (`demon/mod.rs`). The `Live`-only rule is not covered by the
warmup hold — that pause belongs to `sim_time` and space unpauses it — which is why the
`WorldStarted` reset is matched on the precondition **no demon may be alive when a run
starts** (`demon::on_world_started` says so). A `DemonSpawnPause` (0.5–3 s staging)
existed and was removed on request; staging an entrance again means a new component,
not reviving that one.

## Telemetry

`{killed, escaped}`, BRP-readable; `killed` is what the **Souls reaped** HUD counter shows
(`ui/stats.rs`, first in the left column, outside the settings tabs — panel internals live
in the **ui-panels skill**). The Sim tab's **World** section is a different thing: seed and
determinism row, no telemetry. The invariant and the check-paused rule are `CONTEXT.md`'s.
