---
name: city-siege
description: Use when working on the M1 siege layer of qwe — the districts (district.rs: components inside grid cells, the shard rule, the label raster, dist_to_heart), the district_city fixture, and, as the milestone lands, the corruption spread, the bastions and their quota, souls and the outcome. Deep detail behind CONTEXT.md's Districts entry and ROADMAP.md steps 3–10.
---

# City siege — deep detail

The detail layer behind the **Districts** entry of `CONTEXT.md` and the M1 steps of
`ROADMAP.md` («Срез и скверна»). What is here is what the code obeys; what the roadmap
plans and the code does not yet carry is marked as such. When a mechanism here changes,
this file changes in the same commit — the term itself goes to `CONTEXT.md`.

## Districts (`district.rs`)

**Why not cells.** The first plan was "a cell is a district, land = has a building or a
road". Checked against the map it fails M1's own promise: the Упа is 60–100 m wide, and on
long stretches it lies *inside* a row of 400 × 411 m cells; such a cell has houses on both
banks, is "land", its neighbours north and south are "land", and corruption would walk
across the river without a bridge. A finer grid does not help — the river always lies
inside some cell. Connectivity does (decision 9 of the roadmap).

**Model.** `DISTRICT_GRID` (14 × 9) sets only the *scale*. A **District** is a connected
component of passable navtiles inside one cell (4-adjacency, on the pruned navmesh —
`Navmesh::is_passable`). A cell cut by water yields one district per bank; a cell with a
bridge yields one district, because the deck is passable and joins the banks. Two
districts are **neighbours** when at least one pair of their tiles is adjacent across a
cell border. Water, walls and pruned pockets belong to no district.

**Build** (`Districts::build`, load thread, right after `prune_unreachable`, on the
snapped portal and heart — see the `world-lifecycle` skill for the thread):

1. Flood fill per component with an explicit stack, restricted to tiles of the same cell.
   The cell of a tile is `tile * DISTRICT_GRID / grid_size` (integer), so the cells share
   the map evenly instead of leaving a 100 m strip at the top.
2. Border lengths between components — one pass over the tiles looking right and up; a
   differing label across an edge is always a cell border, since inside a cell adjacent
   tiles were filled together.
3. **Shards.** A component under `DISTRICT_MIN_AREA` (1600 m², stated in **metres** so
   that the 1 m navtile does not turn a four-times-smaller yard into a shard) is merged
   into the neighbour with the longest shared border, smallest shard first, repeated to a
   fixed point; the merge folds the shard's border map into the absorber's. Isolated
   shards do not exist after prune — everything passable is reachable from the portal,
   hence borders something. Without this rule the heart would be surrounded by
   twenty-tile districts each with a bastion quota of its own (step 6).
4. Dense ids, neighbour lists (sorted, deduplicated), centroid = mean tile centre.
5. `dist_to_heart` — BFS over the neighbour graph from the heart's district, in hops;
   `None` where the heart is in no district (snapped into water) or unreachable.
6. **Label raster** — `DISTRICT_LABEL_METERS` (8 m) → 700 × 463 cells; a cell's label is
   the district of the navtile at its centre. `district_at(pos)` is an O(1) read of it.
   A label per navtile (5.2 M at 2 m, 21 M at 1 m) is never stored: components are
   computed on the full navmesh, but read by position.

Cost: one pass over all tiles plus the border pass — the same order as the prune BFS;
logged as `districts: N in …` on the load thread. Memory during the build: a `u16` per
navtile (10 MB at 2 m, 41 MB at 1 m), freed with the thread. Tula on the slice frame,
2 m navtile: **161 districts in 109 ms** (prune took 71 ms on the same load), the
portal's district 9 hops from the heart's; on the overlay the Упа reads as a colour break
everywhere but at the bridges.

**What it is not.** Not run state — a restart keeps it; a city switch or a navtile
change reloads the world, so it is rebuilt with the navmesh. Not a pathfinding
structure — nothing routes over it.

## The fixture: `district_city()` (`map/osm/fixture.rs`)

`tiny_city` cannot host district tests — its banks are empty ground. `district_city` is
one building over the whole map with yard-holes: four yards in a chain along the south
(each in its own `DISTRICT_GRID` cell), from the fourth a 100 m strip north across a
water band crossed by **one** `bridge`, a fifth yard at its end, a sixth to the east —
the heart. Yards are joined by `passage` roads (arches through the building). A 20 × 20 m
yard just across the cell border at x = 400, stitched to the first yard by its own
passage, is the shard.

`district.rs` tests pin: **8 districts** (six yards, the strip splits into a south-bank and
a north-bank district because the water band straddles a cell row border at y ≈ 822 and
the deck itself spans both); the portal district is **7 hops** from the heart; the shard
resolves to the first yard's district; water resolves to `None`; the two bank districts
differ yet are neighbours (through the deck). On `tiny_city`: a cell without a bridge has
its banks in two districts that are **not** neighbours; the cell with the bridge has both
banks in one district.

## Census (`DistrictCensus`)

Living humans per district, `Vec<u32>` indexed by `DistrictId`. `census_districts` runs
in `FixedUpdate`, `SimSet::SpatialRebuild`, `SimPipeline::BothModes`, and does its pass
only when `SimTick % DISTRICT_CENSUS_TICKS == 0` (64 ticks, one simulated second): a full
`Query<&SimPosition, With<Human>>` walk with a `district_at` lookup each — 20 000 reads
and 20 000 raster reads per simulated second, measured as `sim/census_ms`. It keys on
`SimTick` rather than a `Local` counter on purpose: the tick resets on `WorldStarted`, so
a restart replays the census on the same ticks, and corruption (which will read it)
stays inside the run fingerprint. Not run state — it recounts itself within a second of
any restart, so it has no `WorldStarted` observer.

## What the player sees (`ui/siege.rs`)

The siege layer is drawn for the player by `SiegeView` (Sim tab → Siege; mechanism in the
`ui-panels` skill): territory by corruption progress, amber where a standing bastion holds
the front, the heart gold; bars over wounded bastions; rings around front bastions; arrows
from Brutes to their targets; `Corrupted` / `Bastions` rows in the HUD. **The front** is
one predicate, `Corruption::on_front(districts, district)` — uncorrupted with a corrupted
neighbour — shared by the ring, the amber territory and `demon::besiege`, so what is drawn
as the front is exactly what the Brutes go for.

## Debug overlay (`ui/debug/overlays.rs::sync_district_overlay`)

`DebugDistricts` (Debug tab, row `Districts`; hotkey `T`; persisted like the other
toggles). One sprite over `MAP_SIZE` with a 700 × 463 texture — one texel per label-raster
cell — sampled `nearest`, at `Z_DISTRICT_OVERLAY` (5.35: above the grid-navmesh fill at
5.2 and the polymesh overlay at 5.3, below every unit). Texel colour: hue from the
district id stepped by the golden angle (ids are handed out in tile-scan order, so map
neighbours are often id neighbours), the heart's district lighter and fully saturated,
a district with no path to the heart grey; no district — transparent. Rebuilt on
`resource_changed` of the toggle or of `Districts` — the resource arrives with the world,
so the first `Playing` frame rebuilds it without an `OnEnter` registration. Gizmos were
rejected: ~150 districts with tile-accurate borders are hundreds of thousands of segments
per frame, the texture is a millisecond once per world.

## Bastion sites (`bastion/mod.rs::plan_sites`, `bastion/fill.rs`)

Where the bastions stand is map-derived like the districts, planned on the load thread
right after them and carried in `LoadedWorld.bastions` → `BastionSites`.

**Placing a point.** OSM gives a bastion as a building polygon, and its centroid lies
inside the building — an impassable tile no pawn can reach and no district contains. So
every point is **snapped** first: `nearest_tile_where` from the centroid's tile, up to
`BASTION_SNAP_METERS` (150 m — wider than the widest factory hall), to the nearest
passable tile; that lands on the pavement at the wall. The district is then read with
`Districts::district_near` within `BASTION_DISTRICT_REACH` (24 m) rather than
`district_at`: the label raster is 8 m coarse and labels a cell by its *centre* tile, so a
2 m pavement tile can sit in a cell whose centre is inside the house. A tagged bastion
with no passable tile in reach is dropped and counted (`dropped` in the log line).

**Quota.** `closeness = 1 − dist_to_heart / max_dist` (0 for a district with no path);
`quota(closeness)` is the first step of `BASTION_QUOTA_STEPS` whose bound is not below
it — 0 up to 0.3, 1 up to 0.7, 2 up to 0.9, 3 at the heart. These are the roadmap's
starting numbers, to be tuned from the `bastions: N tagged, M strongholds` log line, not
in advance.

**Top-up** (`fill_quota`, pure: pre-snapped inputs, no navmesh, no ECS). Candidates are
buildings of at least `STRONGHOLD_MIN_AREA` (300 m²) whose snapped point has a district;
a building already hosting a tagged bastion (a site within `BASTION_DEDUP_METERS` of its
point in the same district) is not one. Each candidate draws one lot from
`lcg_seeded_by(first outline vertex)` — the map's own lot, no `WorldSeed`, the same
family doors and tree planting use — and a district short of its quota takes the
smallest lots first. A district without eligible buildings keeps what it has. Tests in
`fill.rs` pin the steps, the "two tags + quota 3 → one Stronghold, always the same
building" case, quota 0, the tagged-building exclusion and scarcity.

## Bastion entities (`bastion/mod.rs`)

`spawn_bastions` (`OnEnter(Playing)`, `WorldInitSet::Spawn`) spawns one entity per
site: `Bastion { kind, district }`, `Health::full(bastion_hp(closeness))`, a
`BASTION_MARKER_SIZE` (12 m) square sprite coloured by kind at `Z_BASTION` (5.4 — above
the roofs and every debug layer, below the units), `DespawnOnExit(Playing)`. Tula: 157
sites → 157 entities (`brp count Bastion`).

**HP gradient** — `bastion_hp(closeness) = BASTION_HP × (1 + BASTION_HEART_GAIN ×
closeness)`: 100 at the edge, 400 at the heart. Per-kind multipliers are M2.

**Ruin.** `combat::Destroyed { entity }` is the generic "health hit zero" event; the
bastion's `on_destroyed` observer answers it: `BastionsStanding[district] −= 1`, the
sprite goes `RUIN_COLOR`, `RuinTag` is inserted, `BastionDestroyed { entity, district }`
fires. `Bastion` stays on the entity — the corpse idiom of `human::to_corpse` — so a
second `Destroyed` on a ruin is a no-op (the query is `Without<RuinTag>`).

**Restart heals in place** (roadmap decision 11). `on_world_started`: `BastionsStanding`
is rebuilt from the sites (all standing), every `Health` refilled, every `RuinTag`
removed and its sprite relit. The restart despawn list in `restart.rs` does not grow,
and a bastion never passes through the spawner twice. `BastionsStanding` is the only
run state here and it is in the run fingerprint, so `a_restart_replays_the_run` would
catch a forgotten reset. The unit test pins ruin → standing −1, no double count, heal
in place without a respawn.

## Corruption (`corruption.rs`)

`Corruption { progress: Vec<f32>, to_heart: Option<u16> }`, run state. `on_world_started`
zeroes it and sets the portal's district to 1 — the invasion starts corrupted at the
portal, on the first tick of every run.

**The step** (`step`, pure — `progress`, the districts, the census, the standing
bastions, `dt`): snapshot which districts are corrupted at the start of the step; then
for every uncorrupted district with a corrupted neighbour in that snapshot and
`standing[d] == 0`, `progress += dt × CORRUPTION_RATE / (1 + humans /
CORRUPTION_CROWD_HALF)`, clamped to 1. The snapshot is what makes a district corrupted
*this* step start infecting the *next* one, whatever the index order; the index walk, no
RNG and district-independent accumulation make it deterministic for free. A short
`humans` (the census has not run yet) or `standing` reads as zero. Returns the districts
that crossed 1 this step; `spread_corruption` fires `DistrictCorrupted` for each and
recomputes `to_heart` — `hops_to_heart`, a BFS from the whole corrupted set, bastions
passable — only then, ~160 districts per recompute. `Res<Time>` inside `FixedUpdate` is
`Time<Fixed>`, so `dt` is the tick.

Schedule: `SimSet::Territory` (fourth in the spine, after `HumanBehavior`),
`SimPipeline::BothModes`, `run_if(in_state(PlayPhase::Live))`. Reads `Districts`,
`DistrictCensus`, `BastionsStanding`; writes `Corruption`; touches no pawn.

Numbers (all `settings.rs`, all roadmap starting values to be tuned in step 11):
`CORRUPTION_RATE = 1/30` per second — an empty district falls in 30 s; `CROWD_HALF = 50`
— 50 humans make it a minute, 200 two and a half. Corruption never recedes (holy ground
is M7). Tests: the four-district chain (grows only next to the corrupted, held by a
bastion, halved by 50 humans, the corrupted untouched, crosses 1 exactly once and infects
onward from the next step) and `district_city` on an empty map: the heart falls after
`dist_to_heart × 30 s` to within a tick per hop.

**Overlay.** `sync_district_overlay` mixes each district's colour toward
`CORRUPTION_COLOR` by its progress. It runs every `Playing` frame but rebuilds the
texture only when a district crosses one of `CORRUPTION_SHADES` (16) steps — the marker
carries an FNV key of the quantised progress vector — or when `Districts` changes.

## Strike (`combat.rs`)

`Attack { damage, period }`, `AttackCooldown(Timer)` (`ready(period)` — pre-ticked, so the
first blow lands at once), `AttackTarget(Entity)`. `strike_verdict(distance,
cooldown_ready, target_alive)` is the pure rule: `Finished` (target at zero — the ladder
sees it next tick), `OutOfReach` (over `ATTACK_REACH`, 3 m: the bastion's point is a
pavement tile, the attacker stands on a neighbour, and 3 m clears the 2.83 m navtile
diagonal the way `DEMON_LUNGE_RANGE` does), `Cooling`, `Hit`. The `strike` system ticks
every attacker's cooldown with `Res<Time>` (the fixed step), reads the target's
`Transform` (bastions do not move; a mobile target with `SimPosition` is M2), and on `Hit`
resets the cooldown, applies `damage`, and fires `Destroyed` on the blow that reaches
zero — `Health::damage` returns `true` once. A target that no longer exists is skipped;
removing the `AttackTarget` is the ladder's job. Registered by `DemonPlugin` as the
**tail of the demon chain** (`pick_wander_targets → acquire_targets → chase → devour →
strike`, `SimSet::DemonBehavior`, `BothModes`): the Brute's ladder sets the target on a
tick and the blow lands on the same tick. Tests: the verdict table, and a two-blow run
in a bare `World` (wound, finish with one `Destroyed`, nothing on the third).

## Who breaks a bastion — the front

The consumer of `AttackTarget` is the Brute (`species-behavior` skill, "The Brute's
ladder"). What this layer defines for it is the **front**: a standing bastion in an
uncorrupted district with at least one corrupted neighbour. That is exactly the set of
bastions currently holding corruption back (`corruption::step` skips a district while
`BastionsStanding[d] > 0`), so a Brute always works where the field is stuck. `besiege`
rebuilds the list every tick from `Districts::neighbours`, `Corruption::is_corrupted` and
the `Bastion` query — dozens of entries; a Brute with a target never reads it. A
bastion behind a river is frontline by the graph even when the walk goes over a distant
bridge; in M1 that is accepted (`ROADMAP.md`, step 8's risk).

## Souls and summoning (`souls.rs`, `demon/systems.rs::summon`)

`Souls { earned, spent }`, run state (reset on `WorldStarted`, in the fingerprint);
`available()` saturates at zero. `earned` is incremented in the **same observer** as
`Telemetry::killed` (`demon::behavior::on_demon_caught_human`), the only kill site, so
`earned == killed` holds by construction.

`SummonRequested { kind }` is a `Message` (not an event): it is written from `Update` —
the `1` / `2` hotkeys (`SoulsPlugin`, gated on `typing_in_text_input` because the seed
field takes digits), the HUD buttons, `brp msg SummonRequested '{"kind":"Brute"}'` — and
read on the fixed step by `summon`, chained right after `spawn_initial_burst` in the
spawner slot (`before(SimSet::SpatialRebuild)`, `BothModes`, `run_if(PlayPhase::Live)`):
`PawnId`s are dealt only after `WorldStarted`, and a request written during warmup burns
in the message buffer instead of waiting for the first live tick. `summon` refuses when
the living demons already reach `DemonStyle::cap` or when `available()` is under the
price; otherwise it charges `spent` and calls `spawn_demon(kind)` at the portal rim,
logging `summoned …`.

**Price** — `summon_cost(kind, alive_of_kind) = ceil(base × (1 + SUMMON_COST_GROWTH ×
alive_of_kind))`: `SUMMON_COST_IMP` 3, `SUMMON_COST_BRUTE` 25, growth 5 % per living demon
of that kind. Roadmap starting values, tuned in step 11; the roadmap's risk stands — the
burst of eight Imps must eat 25 humans before the first Brute, so the first minutes are
watching.

## Outcome (`outcome.rs`)

`Outcome { Running | Won { tick } | Lost { tick, reason: Stalemate } }`, run state in the
fingerprint (variant + tick), reset on `WorldStarted`. `judge` is the pure rule: the
heart's district corrupted → `Won`; else no living demon **and** `available <
summon_cost(Imp, 0)` → `Lost(Stalemate)` — with demons alive the run goes on however
empty the purse, and without demons enough souls for one Imp is still a game. The
judge runs in `SimSet::Territory` **after** `spread_corruption` (the win is declared on the
tick the heart falls, not the next), `BothModes`, `Live` only; once the outcome leaves
`Running` it stops looking. On the transition it pauses `Time<Virtual>` (roadmap decision
6: no menu, R does everything) and logs `outcome: …`. `on_world_started` unpauses only
if the outcome it resets was not `Running` — a Space pause is the player's (pinned by
`a_new_run_lifts_only_the_outcomes_pause`). The plaque is `ui/outcome.rs` (the
`ui-panels` skill); the HUD's `To heart` row is `Corruption::to_heart`.

In M1 a stalemate is hard to reach honestly (nothing kills demons; "no demon alive" means
a burst that caught nobody), so it is checked on a stand: `brp despawn` every demon with
zero souls available → `Lost` on the next tick.

## The acceptance stand (`examples/acceptance/m1_win.rs`)

`cargo run --example m1_win` — the whole siege loop headless, on `district_city`, on
`replay_app_with` (the `determinism` skill): the districts and one hand-made
`BastionSites` (a Stronghold at `city.south_bank`, the district that holds the bridge,
HP by `closeness` like a real site) go in through the configure hook; 200 humans; at
`SUMMON_TICK` (10) the purse gets `SOULS_GRANT` (100) and a `SummonRequested { Brute }` is
written straight into the world (`World::write_message` — the message survives the next
`First` swap and `summon` reads it on that update's fixed step). The run goes in
`CHUNK_TICKS` (1 280) slices until `Outcome` leaves `Running` or `MAX_TICKS` (60 000, a
quarter hour); a `SiegeLog` resource filled by two observers (`DistrictCorrupted`,
`BastionDestroyed`) records the ticks. Two runs on seed 1, then three checks: `Won` before
the cap; the same outcome (variant + tick) in both runs; every north-bank district
(centroid past `NORTH_BANK_Y` 900) corrupted **after** the bastion's district — the bridge
was the road. The numbers it prints are the baseline table in `ROADMAP.md`, step 11.

The stand is why `run_to_tick` now returns on a standing world: the judge's pause used to
leave it spinning on the winning tick.
