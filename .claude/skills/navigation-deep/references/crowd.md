# Crowd — separation and destination slots

Detail behind `movement/separation/` and `movement/destination.rs`. The summaries live
in `CONTEXT.md` (Simulation).

## Separation

**Separation** (`movement/separation/`, toggle in the Nav tab, persisted)
— soft pairwise anti-overlap: pawns on screen keep their body radii apart. The human
radius is the `HumanStyle::body_radius` knob (0.3…1.2 m, Nav tab and the crowd demo);
`HUMAN_BODY_RADIUS` 0.9 m is only its `Default`, and the demon's radius is never a
separate knob — always twice the human one (`separation::demon_radius`,
`DEMON_RADIUS_RATIO`, default `DEMON_BODY_RADIUS` 1.8 m). At the default that is
deliberately **larger** than half the sprite, so a resting pair leaves a visible gap
(1.8 m rest distance against a 1.0 m `HUMAN_SIZE`). At the earlier 0.45 m the rest
distance (0.9 m) was *narrower* than the sprite and a correctly
separated crowd still drew as a solid mosaic. Deliberately local and cosmetic, four
gates in order: **the mode** (`separation_runs`) — no separation under determinism, and
none on the grid backend either (`PolymeshDebug::enabled` off): grid waypoints sit in
navtile centers and the walk puts a pawn back on them every step, so a separated pair
is re-collapsed by the next tick and all the mechanism adds is jitter and holds.
Personal space presupposes metric waypoints, i.e. the polygonal mesh. It is the
*toggle*, not mesh readiness: while the mesh builds the grid serves the requests, but
blinking separation over that transition is worse than half a second of the old
behavior. The panel treats this exactly like determinism — the `Enabled` row
reads `Off`, dimmed and unclickable (`separation_allowed_by_mode`, the one rule shared
by the schedule and the panel), and `SeparationHolds` is cleared as soon as the mode
turns the run off, or pawns held by the last run would stay slowed forever.
Then: the toggle; **once per rendered frame** (it lives in `FixedUpdate`
right after `move_moving_entities` — the only point where the tick's positions are
final and the snapshot is already taken, so the push reaches the screen through
interpolation — but at 30x that schedule runs ~1920 ticks/s and even 0.03 ms per tick
would eat ~6% of a real second; ticks between runs only accumulate virtual dt, and the
run's outputs — holds, steer, block — serve every tick until the next run: freshness is
per-frame by charter, the one residual speed dependence, bounded at one frame of
staleness and released per tick against fresh positions); **zoom
below `SEPARATION_MAX_ZOOM`** (same 0.75 as the wander-dispatch cutoff — farther out a
pawn is 1–2 px and overlap does not read). Candidates come from both coarse grids via
`for_each_in_rect` over the viewport (`VIEW_MARGIN` slack), then a throwaway fine grid
(`separation_cell()` — twice the demon radius, floored at `SEPARATION_CELL` 2.4 m, so
3.6 m at the default radius: the cell must not be smaller than the largest sum
demon+demon or an overlapping pair falls outside the 3 × 3 scan, and with the radius on a
slider a plain constant would sooner or later be too small; head+next linked lists in
`Local` buffers — no steady-state
allocations) resolves pairs: each pair's overlap decays as `exp(-SEPARATION_RATE · t)`
(8/s) of virtual time, capped at `SEPARATION_MAX_SPEED · dt` per run, with
`SEPARATION_MAX_STEP` (0.3 m) as the teleport-guard floor of the final clamp
(`max_step.max(max_speed · dt)`) — the run is **dt-invariant**: one long dt equals N
short ones, so separation behaves the same at every sim speed. The correction is split
by mobility
(human 1.0, demon 0.25, devouring demon 0 — it pushes but never moves off its corpse).

Rules that keep the push from fighting the walk it is correcting, each fixing a
symptom that was visible on screen:

- **the longitudinal push is kept on purpose** (`damp_along_heading`, share
  `SeparationStyle::backstep`, default `SEPARATION_BACKSTEP` **1.0 — nothing is damped
  at all**) — the retired `across_heading` zeroed the component along a moving pawn's
  own heading, because it read as a follower reversing for a step. The sweep that
  replaced it with a knob came out monotone the other way on the demo's funnel (200
  pawns aimed at one point; overlapping pairs / worst overlap, table under
  `SEPARATION_BACKSTEP`): 0.0 → 630…2251 / 1.79 m, 1.0 → 100…204 / 0.92 m. Damping it
  takes away the only thing a converging crowd can widen with — outward *is* backward
  along the heading — so the jam rotates instead of spreading. The reversal it was
  aimed at is cured by the next rule (`shares`) instead, and the knob stays only so the
  sweep can be redone with a slider rather than a rebuild;
- **the pawn behind gives way** (`shares`) — for a pair on roughly the same course
  (courses less than 90° apart) where **exactly one** has the other in front, the
  follower takes the entire correction and the leader none, instead of the leader being
  shoved in the back by someone who caught up. When each has the other in front the pair
  is converging, not queueing — the solver's `opposed` — and the split stays by mobility:
  the sidestep is scaled by the *yielder's* share, and `yields` picks the yielder by
  `PawnId`, so a converging pair treated as a queue had its sidestep multiplied by zero
  half the time;
- **a walker squeezes past a stander** (`pass_squeeze`, `SEPARATION_PASS_SQUEEZE` 0.6) —
  in a pair where exactly one is walking the rest distance shrinks to 60 % for as long as
  the pass lasts; two standers and two walkers keep the full distance. This is what makes
  the destination-slot lattice passable at all: its step is 2.0 m while a walker needs
  `2 × rest` = 3.6 m of clearance between two settled pawns, so inner slots were
  unreachable except *through* bodies. Squeezing the **pass**, not the crowd, is the whole
  point — the earlier `compress` knob shrank by neighbour count, i.e. hardest exactly
  where the crowd stands still, and a settled crowd ended up permanently overlapped (93 %
  of a pawn's time inside another body on the lab's funnel);
- **a fifth of pawns dodge left** (`left_share`, `SEPARATION_LEFT_SHARE` 0.2) — the side
  is personal and stable (a `PawnId` hash, like the coincidence axis). One side for
  everyone produces two pictures that do not occur in life: a same-way flow congealing
  into a single column, and pawns that failed to reach a dense crowd orbiting it as one
  carousel. Measured best on the lab's street: the lowest time-in-separation of anything
  tried, and the only knob that actually widens the flow;
- **head-on pairs step right** (`sidestep`, strength `SeparationStyle::sidestep`) — two
  pawns walking straight at each other have their pair axis collinear with both
  velocities, so the plain correction has no lateral component at all and they lock
  together until an outside asymmetry frees them. Whatever `backstep` does with that
  correction it stays collinear, so in the push branch the sidestep is the *only*
  lateral component such a pair ever gets and it must stay above zero (the steering
  below is the other answer, and it bends the walk instead of the push);
- **a blocked pawn steers aside** (`SeparationSteer`, strength `SEPARATION_STEER` 1.0) —
  the push moves a **position** while the walk immediately carries the pawn back toward
  its goal, so the two forces cancel and the whole result goes into distance walked.
  Steering turns the **heading** of that same walk instead: full speed kept, the pawn
  arcs around and never "returns". A symmetric head-on flow does not disperse without it
  at all — the pair axis is collinear with both courses, so no pair has any lateral
  component (the lab measured a spread of exactly 0.00 m at every `rate`, `hold` and
  `sidestep`). Its side is the same personal one as `left_share`, and it is released
  within `steer_release` of a waypoint, or a pawn circles the point instead of passing it.
  It is also **dropped for the tick whose deflected step would land in an impassable
  tile** (`Walkable::coast_allows`, grid-only like coasting): the bend reaches ~45° of a
  full step and carries the pawn *off its path*, and a pawn steered into a building is
  neither pushed back out (a push into an impassable tile is dropped by the same rule)
  nor rescued (state `Moving`, path valid). The refusal walks the plain course, it does
  not halt;
- **a blocked pawn eases off** (`SeparationHolds`, share `SeparationStyle::hold`,
  default `SEPARATION_HOLD` **1.0 — i.e. no easing off at all**, since steering solves
  the same problem better; every fraction below 1 measurably ruins convergence) — a
  *human* whose heading points into an overlapped
  neighbour that is **standing or oncoming** (only there is pressing futile — holding on
  any touch made whole same-way flows crawl at the hold fraction, companies of
  co-travellers strangling themselves) walks at a fraction of its speed until the next
  separation run, which
  collapses the walk-vs-separation equilibrium overlap from `speed / SEPARATION_RATE`
  (~0.35 m, a frozen jittering clump) to `hold ×` that (~0.07 m, invisible); 0 would be
  a full stop and makes dense crossings move in stop-motion jerks. Demons are never
  held (a chase must close in; the crowd flowing around a demon is already expressed
  by mobility) — but *bracing* is counted for everyone: the run's stuck clock
  (`SeparationState::braced` → `stuck`, read by `stuck_compress` and `slide_release`,
  both lab-only and zero in the game) ticks for a demon too, or the sliding ban
  (`SeparationBlock`), which it does get on the same condition, would never be
  released. The hold set is the one deliberate breach of "cosmetic": it feeds back
  into `move_moving_entities`, which also grants a held pawn **arrival** when it is
  within the rest distance of its goal — the blocking body will not let it any closer,
  and without the grant it would shove at that body forever. (Arrival on an exhausted
  path is likewise forgiven within the rest distance — separation may push a pawn off
  its final tile at the last moment.) The rest distance is `2 × the pawn's own radius`,
  taken from its `movement::BodyScale` (a ratio over the `body_radius` slider, `DEMON` =
  2×): the walk asks the component, not `Has<Human>`, which used to hand a demon-sized
  arrival slack to anything that was not a human — the `dev.rs` test walker included.
  The set is rebuilt from scratch each run and
  cleared by the toggle, the zoom gate and world entry, so under determinism it is
  empty from tick 0 and movement never depends on it.

Coincident positions split along a deterministic per-entity hash axis (the
`personal_spread` trick). A push into an impassable tile is dropped (`rescue_*` only
catches failed path searches, it would never find a pawn squeezed into a wall); a push
that crosses a 60 m cell boundary re-inserts the human into its grid. **Lunging demons
(`DemonLungeTag`) are exempt entirely** — the lunge writes `SimPosition` itself and
must close to `KILL_DISTANCE`, which is *smaller* than the demon+human radius sum;
separation fighting it would starve kills. Corpses are outside by construction (no
`SimPosition`). The sim is knowingly camera-dependent — the user's viewport-only
optimization accepts that; off-screen crowds still stack and pay nothing.
`sim/separation_ms` measures **per run** (~60/s), not per tick like its `sim/*_ms`
neighbours.

**The crowd demo** — reproduced and measured on demand by
`examples/demos/crowd_demo/` — a windowed scene running the real `separate_pawns` on
an empty navmesh, with the crowd arranged into the cases that are otherwise waited for
(a pile, a funnel, counter-flowing columns, a walled corridor, real wander AI), a
body-radius gizmo per pawn and a live count of overlapping pairs. It navigates the way
the game does — `find_path_polymesh` over an empty `MapData`, on the default-on
polymesh backend, because separation only exists there; the scenario's corridor walls
are therefore written into `MapData::walls` as well as into the grid, and switching
scenarios rebuilds the mesh. A query that finds no path leaves the pawn standing and
ticks a `path misses` counter — falling back to a straight line would walk it through
the wall and read as a working scenario. There is deliberately no key to switch the
scene to the grid backend: separation does not run there, and that is what the scene is
for. Traps that scene made visible and any measurement of separation has to respect:
**count only pawns inside the camera rect** (off-screen ones are never separated by
design, and including them makes on/off indistinguishable), and **allow a millimetre
tail** — the solver is soft, so a converged crowd still reports pairs a few mm inside
the radius sum. Two more, both of which had silently voided every earlier comparison:
**a scenario's spawn spacing has to be re-checked against `HUMAN_BODY_RADIUS`** (the
columns kept a 1.2 m step through the 0.45 → 0.9 m radius change and spawned already
overlapping, so the run measured recovery from the spawn, not flow), and **lateral
spread must be measured against the crowd's own mean, not the map centre** (goals sit
in navtile centres, so a centre-relative figure is a constant and reads the same with
separation off).

**The separation lab** (`SeparationLab`, `SeparationStats`, `SeparationSteer`;
`tools/separation_lab/`, findings in its `REPORT.md`) — the numeric knobs of separation,
split in two by type. `SeparationLab`'s own seven fields **are live in the game**:
`rate`, `max_step`, `max_speed`, `steer`, `steer_release` come from `settings.rs`
constants, and `pass_squeeze` / `left_share` are on top of that the Nav tab's two
sliders. Everything the game does *not* run sits in `SeparationLab::experiments`
(`SeparationExperiments`) — `horizon`, `anticipation`, `anticipate_margin`, `lane_bias`,
`compress`, `compress_at`, `crowd_sidestep`, `idle_mobility`, `arrive_slack`, `slide`,
`stuck_compress`, `stuck_after`, `stuck_ramp`, `hard_core`, `slide_release` — every one
off by default, so those branches of the solver never execute in a game build, and a
`lab.experiments.…` read is the marker that says so. Promoting a knob to the game means
**moving the field**, which `game_defaults_keep_every_experiment_off` forces you to
notice. Deliberately **not** a `SettingsGroup` either way: values are measured on the
crowd demo, not chosen by the user. What `SeparationStats` counts is **one run of the
world**, not the session: its five counters (`runs`, `push_metres`, `worst_push` — the
teleport detector — and the two pair sums) are zeroed by a `WorldStarted` observer in
`movement/separation/`, the same seam `Telemetry` uses, because an `OnEnter(Playing)`
reset would miss the R restart entirely. What
it made visible: in a symmetric head-on flow the pair correction is collinear with
both headings, `sidestep` is gated off by `alone`, and so **no lateral force exists at
all** — the crowd stays a strictly one-dimensional chain and no value of `rate`,
`hold` or `sidestep` changes anything. `SeparationSteer` is the answer that measured
best: instead of displacing a blocked pawn, the run hands it a lateral direction and
`move_moving_entities` bends the *walk* by it, so the pawn keeps full speed and rounds
the obstruction instead of fighting its own path. Its one trap is worth remembering —
`Movable::last_direction` must stay the **desired** heading, because the steer side is
the right normal *of that heading*, and writing the bent course back turns the pawn
further right every frame until it circles in place.

## Destination slots

**Destination slot** (`movement/destination.rs`) — the reservation that stops two pawns
from being aimed at the same point. A **slot** is a `k × k` block of navtiles,
`k = max(1, ceil(rest distance / navtile_size()) + SlotLab::slack)`, claimed by one pawn
(`DestinationClaim`, reverse-indexed by the `DestinationClaims` resource); its goal is
strictly the block's **centre** tile, so the goals of neighbouring slots sit exactly
`k · navtile` apart — never less than the rest distance, for any combination of
`NavtileBase` and the `HumanStyle::body_radius` slider (the Nav tab's `Slots`
group, and the crowd demo). The floor of one tile belongs to the **sum** and lives in
`slot_side_with_slack` — the only place the demo's `slack` may be added to
`slot_side`, because a zero side makes `slot_of` divide by zero and a negative one
turns the lattice inside out.

The radius lives with the **human**, not with separation, precisely because slots read it
too and they run even when separation is toggled off — while it sat in `SeparationStyle`
the panel printed `off` under determinism and the knob went on reshaping the slot
lattice. Without slots, separation has no
way out at all: `move_moving_entities` only pops a waypoint when the tick's travel
budget covers the remaining distance, and a pawn pressing into a taken point is pushed
back exactly as far as it steps, so overlap parks on the equilibrium
`HUMAN_WALK_SPEED / (SEPARATION_RATE × share)` = 0.70 m and stays there — the pair
either orbits (with the sidestep) or stands and jitters (without it), and a crowd that
reaches a shared point never settles. Why a block and not a tile: one-per-tile only
guarantees a non-overlapping resting crowd while `2 × radius ≤ navtile_size()`, which
`NavtileBase::M1` (1 m tiles against a 1.8 m rest distance) and any radius above 1.0 m
break. Why not the user's fractional lattice: every goal here is navtile-keyed
(`MovableState`, `PathfindingRequest::end_tile`, the stale-answer filter, the arrival
test, `tile_center(end_tile)` in the polymesh), so a point that is not a tile centre is
a point no pawn can be said to have reached. The centre tile is fixed on purpose — let a
block pick any passable tile in itself and two neighbouring blocks pick adjacent
corners, which is the very thing the block exists to prevent; the price is that a block
with an impassable centre goes unused and the pitch rounds up to a whole tile (up to
~2× the rest distance), so a crowd parks a little sparser than strictly needed. A taken
slot moves the goal outward by ring search (`nearest_tile_where`, bounded by the
`SlotSearch` resource, `CLAIM_SEARCH_METERS` 16 m by default and a slider in the crowd
demo — deliberately not a `SettingsGroup`, it is a tuning bound and the demo must not
write the game's config). Nothing free inside the bound is the one branch where the old
pathology returns in full: the goal stays the shared unclaimed tile and the pawn presses
into it forever, exactly as before slots. That is still better than refusing a target,
which would park the pawn for good — and with the bound on a slider the branch is
visible on demand (drop `Slot search` to 2 m on the funnel and the tail of the crowd
collapses onto one point). A pawn that finds nothing also loses its previous claim, so a
saturated crowd churns reservations.

### Who gets which slot — `SlotMatching`

Ring search per pawn is only *one* of the two ways the goals of a batch are handed out,
and not the default one. The single entry point for both is the free function
`claim_batch` (`movement/destination.rs`), used by `assign_destination_slots` in the game
and by `drive_routes` in the crowd demo; the mode is `SlotLab::matching`.

`claim_batch` first splits the batch into **groups with the identical desired
`end_tile`**, keeping the order of each group's first member, and a **group of one goes
to `claim_slot` in either mode** — the batch answer degenerates to the greedy one there
and costs measurably more. That is the ordinary case in the game, where every wanderer
picks its own tile: the batch is almost all singletons, so the mode's price stays where
it was introduced for — a crowd walking into one point (the demo's funnel, a pile at the
portal).

- **`SlotMatching::Greedy`** — pawn after pawn, in the order the batch arrived: each one
  ring-searches for the nearest free slot to the goal (`claim_slot` above). The
  centre goes to whoever is processed first and the outermost slots go to whoever is
  left, i.e. exactly where a mismatch costs most there is no choice left to make.
- **`SlotMatching::Batch`** (`#[default]`, `DestinationClaims::claim_group`) — slot after
  slot, **from the rim of the bunch inward**. It collects the `n` free slots nearest the
  goal (the same bunch greedy would have handed out; a square scan of the same
  `SlotSearch` radius, sorted by `(distance², slot.x, slot.y)` — the coordinate tail
  because up to eight slots tie at one distance and the pick between them has to be
  stable — then truncated to the group size), and gives each slot, starting with the
  farthest, to the nearest pawn that has none yet. The rim first because the cost of a
  mismatch scales with how far the slot sits from the goal: a pawn on a rim of radius `R`
  walks `R − ρ·cos(angle difference)` to a slot at `ρ`, so the angle decides everything at
  the outermost slot and nothing at all at the central one (`ρ = 0`).

Both modes end with the same shape of answer — everyone on their own slot, the crowd
packing from the goal outward — and differ only in distance walked, which is why this is
a lab knob and not a user setting. Batch became the default because the measurement paid
for it on every criterion (`tools/separation_slots_lab/REPORT.md`, 3.1: `net` −23 %,
`travel` −36 %, `through` −70 %, idle drift −49 %, and `sep_ms` *falls* because the crowd
spreads out). Its price is `O(radius² + n·m)` per group against `n` ring searches. The
failure mode is unchanged: a group short of free slots leaves its tail with `None`, i.e.
the shared unclaimed goal, exactly as `claim_slot` does. Previous claims of the whole
group are released **before** the free slots are collected, or members would route around
their own last-tick slots and the bunch would drift off the goal for nothing.

`SlotLab` (`Reflect` resource, registered in `MovementPlugin`) is to slots what
`SeparationLab` is to separation: a measuring rig whose default reproduces the shipped
behaviour. Of its four knobs the game reads **only** `matching` and `slack` (extra
navtiles added to the slot side, 0 in the game); `claim_at` (defer the claim until the
pawn is within N m of its goal) and `regroup` (walk a settled pawn back onto its slot
after it is shoved off) are consumed by the demo alone — the game's `regroup_onto_slots`
was dropped, because a settled pawn there always carries `NeedsWanderTarget` and picks a
new goal anyway.

The claim is **not** released on arrival: a pawn standing on its slot is exactly the
occupancy being modelled. It moves on the next target selection, and is released by an
`On<Remove, DestinationClaim>` observer on despawn (escape, restart, city switch) and by
the corpse strip in `demon/behavior.rs`. Hook point is a single system,
`assign_destination_slots` over `Added<PathfindingRequest>`, registered in both
dispatcher chains — human wander, demon wander and the test walker are all covered
without touching their behaviour systems; the `Update` registration needs an explicit
`.after(human::pick_wander_targets)`, or the request reaches the dispatcher unslotted
every so often. The `Added` filter is why `Movable::to_pathfinding` removes the old request before
inserting a new one — an in-place overwrite would arm only `Changed` and the re-targeted
pawn would silently keep its previous slot. **Chase is excluded** (a shared goal is the pincer, by design) and so is
**flee** (targets churn every 0.7–1.2 s and point off-map — so a panicking crowd is not
covered by slots). Unlike separation this runs in **both** modes: it is simulation, not
cosmetics, which is also why there is no camera gate — unslotted clumps would pile up
and freeze off screen, and the camera would arrive into exactly the pathology. No
`HashMap` iteration reaches the output (keyed lookups only, and `claim_batch`'s grouping
keeps insertion order) and the assignment batch is
sorted **nearest-pawn-first** — by the squared distance from the pawn to the target it
asked for, with `(species, PawnId)` (`order::pawn_key`) as the tail that breaks the tie.
Nearest-first because the place at the goal must go to the pawn already standing next to
it: sorted by id alone, a pawn from the far rim takes the middle while the one right
there is sent outward, which is a detour for both and a counter-flow between them.
It used to be `(species, PawnId)` alone; `2a1035e` swapped the leading key so the pawn
already closest takes the nearest slot instead of the one that happens to hash first.
The tail is what keeps the order total, so `sort_unstable` cannot depend on the query's
traversal order — the same key discipline as `apply_pathfinding_results`, just with a
float in front of it that is a pure function of two positions. The order matters past
the sort too: under `SlotMatching::Batch` the groups are handed out in the order of
their first member.
Changing the radius slider or the navtile size re-keys the lattice, so the index is
dropped and rebuilt from the next selections.
