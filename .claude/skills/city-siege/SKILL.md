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

## Not yet in the code

The roadmap's next steps on this layer, in order: **corruption** (`SimSet::Territory`),
`Attack`/`strike`, demon kinds, souls, the outcome. Each lands here with its mechanism as
it is written; until then `ROADMAP.md` is the only description and it is a plan, not a
record.
