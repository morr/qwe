# Grid navmesh fill — mechanics

Detail behind `navigation/navmesh/{fill,gates,raster,reach}.rs`. The one-line fill rules
and their order live in the navigation-deep skill; the terms in `CONTEXT.md` (Navigation).

## The three fill steps

In code the fill is three steps — `fill_base` (everything up to the walls), `fill_fences`,
`carve_passages` — because `open_sealed_fences` re-lays the last two on a clone of the
first instead of refilling the city.

## Ordinary roads and rails never touch the grid

**Ordinary roads do not touch the navmesh.** The grid starts all-passable and
`fill_from_mapdata` only ever *subtracts* (water, buildings, walls); roads enter it
solely through the `bridge` and `passage` carves. Pawns walk on grass and asphalt
alike. Consequently road **rendering** (`map/roads.rs`, `MeshBuilder::push_ribbon`) and
road **rasterization** (`Navmesh::set_polyline`, a capsule sweep by
`distance_to_segment` — round joints by construction) are two independent code paths
over the same `RoadLine`, and changing how a road is drawn cannot change where anything
walks. Changing `RoadLine::points` or `width` would change both at once.

**Rails do not touch the navmesh either, and deliberately so.** `MapData::rails` is
absent from `fill_from_mapdata` by design, not by omission (`tests/navigation.rs`
pins it): a rail line runs unbroken across the whole city, so blocking it would slice
the map in two and `prune_unreachable` would amputate whichever half does not hold the
portal. Pawns cross the tracks as if they were ground.

## Fences block, with gaps

**Fences block, with gaps** (`Navmesh::fill_fences`, `footprint::fence_gaps`). A fence
used to be pure decoration; now its band (`FENCE_BAND_WIDTH` 0.3 m — physical, not the
zoom-grown drawn ribbon; the grid barely cares, the 4-connected centerline chain is what
blocks) is impassable except at a **gap** `{ at, reach }`:

- **a non-bridge road whose centerline crosses the fence**, reach `width/2 / sin θ`
  (obliquity capped at 2×) — a footway through a gate, a service drive into a yard, a
  shared OSM node;
- **a road ending on the fence** (its end within half its width of the centerline) — a
  path drawn up to the gate and stopped there;
- **a road ending short of the fence but aimed at it** — reach `GAP_END_REACH`, which
  *is* `roads::network::STITCH_MAX_GAP` (6 m), because it is the same sloppy mapping the
  stitch answers: a drive is mapped «to the pavement» and dropped a few metres before the
  gate it goes through on the ground. Two conditions carry it, and dropping either is
  measurably worse: the end must be
  **loose in the stitch's sense** (`network::carries` — no other road at its node
  carrying its own surface; a street split into ways beside a fence would otherwise open
  a gate at every joint), and the fence must lie **ahead** of it (`(at − end)·heading > 0`
  — a road running *along* a fence ends beside it, not into it). Without this the drawn
  stitch ran the asphalt through an uncut fence;
- **a default gate** (the default gates below), reach `FENCE_GATE_WIDTH / 2` (a 3.5 m
  footway).

**Never band overlap**: the trap of this rule is a street running alongside a fence, whose
nominal 8–16 m ribbon covers the fence end to end — an overlap rule would have removed
every fence along every street and kept only those deep inside the blocks
(`a_street_along_the_fence_leaves_it_whole`). Bridges cut nothing: the span passes over.
**On the picture the same span does cut** — the fence layer lies above the bridges, so
`map::fences` drops what a deck covers (`footprint::BridgeDecks`, the osm-map skill).
That is the one deliberate difference between the drawn fence and the blocking one: the
fence under the bridge is still there for a pawn, it is simply hidden by the deck.
Roads are found through a 32 m cell index of their segments.

**The gap is cut out of the fence mask, never carved into the grid** — the bridge-curb
idiom: `visit_polyline` collects the fence's tiles, the ones within `reach + tile·√2` of a
gap are dropped, the rest are blocked. Carving the road would reopen the house or water it
touches (`a_gate_does_not_open_the_house_behind_it`). The `tile·√2` margin is not taste:
a fence is a 4-connected staircase, and one tile taken out of a slanted one leaves its
neighbours touching through the hole — `a_footway_through_the_fence_opens_a_gate` runs
five angles for that reason. The polygonal mesh does the same in vectors: all fence
ribbons **minus** 16-gon gap discs (circumscribed, so no hair of barrier survives at the
edge), one `Difference` into the blockers — never into the carves, which are subtracted
from the union of everything. **The renderer draws the same gaps**
(`footprint::fence_pieces` over the same discs, osm-map skill) — the grid alone cuts
`tile·√2` wider, a rasterisation allowance like the bridge deck's.

## Default gates

**Default gates** (`Navmesh::open_sealed_fences`, stored in `FenceLine::gates`). The rule:

- runs in the load thread **between fill and prune**, from the snapped portal;
- a **pocket** is a component passable now, reachable without fences, unreachable with
  them; it gets a gate if it **holds a door** (a door tile or its 8 neighbours — the
  `find_passable_tile_near` circle) or is at least `SEALED_POCKET_MIN_AREA` **400 m²**
  (metres, so the navtile cycler does not change which yards open). A door-less sliver
  between a fence and a blank wall stays cut off;
- **a door the fence closed point-blank** (all nine tiles blocked, but reachable without
  fences) gets one too — no pocket exists there;
- the gate tile is a fence-only tile (blocked now, passable without fences) on the
  pocket's edge, **preferring one that already touches the reachable side** — a double
  fence therefore opens the inner one first and the next round the outer
  (`GATE_ROUNDS` 6) — and among those **the one nearest a carriageway's edge**
  (`footprint::StreetEdges`: `roads::is_carriageway` segments in 32 m cells, the
  centerline distance minus half the width, capped at `STREET_REACH` 96 m; the key is in
  decimetres so it stays an integer, ties by tile index). Without it the gate went to the
  lowest tile index — on Tula's school ground that was the back corner by a footpath, and
  a school or a works is entered from the street
  (`a_sealed_plot_gets_its_gate_on_the_street_side`);
- the gate point is the nearest point of the nearest fence, written into
  **`MapData`**, not the grid — `fence_gaps` reads it, so the polygonal mesh built later
  from the same map has the same gates (pinned by
  `parity_tests::fences_block_and_gates_open_in_both_fills`);
- the grid after a round is `base.clone()` + `fill_fences` + `carve_passages`, and
  reachability grows from the new gates (`extend_reachable`) rather than being
  re-flooded: a gate only ever opens tiles.
- **two gates of one round stand at least `GATE_SPACING` 10 m apart**: neighbouring
  pockets of a round are usually pieces of one yard cut by a door or a bend, and they
  pick neighbouring tiles — without the spacing Tula got five gates 2–3 m apart, one
  ten-metre breach. A pocket the neighbour's gate did not open is taken by the next round.

The cost is mostly the one full flood (a tight index-arithmetic
`flood`; the iterator version was three times slower) — the no-fence reachability is
grown from it through the fence tiles rather than flooded again. Offline:
`cargo run --example fence_prune_audit -- [city]` builds the map with and without
fences and prints the audit numbers.

## Bridge curbs are impassable

**Bridge curbs are impassable** — the same two bands the renderer draws
(`RoadLine::curb_bands` in `map/footprint.rs`: edge centerlines from `miter_offsets` +
curb width, one construction for the grid fill, the mesh build and the renderer, so
the blocked strip matches the drawn one by construction). Over water
this changes nothing (water already blocks); on dry spans — approaches, overpasses —
the curb is what stops a pawn from stepping off the deck sideways. Only the two
longitudinal edges block, the deck ends stay open. All curbs block *before* any deck
carves — the render layering (curbs under fills) repeated in the grid, so at a
junction of two bridge ways one way's deck re-carves the other's curb and the bridge
is never walled across by its own curb. The deck carve is `width + curb − tile·√2`
(`navmesh/fill.rs::fill_base`) — it stops **half a tile diagonal short of the curb
centerline**, which at the default 2 m navtile leaves it narrower than the deck itself
by `tile·√2 − curb`: a curb-chain tile's center wanders up to half a diagonal (√2 m)
off the curb centerline — i.e. *into* the deck on a slanted bridge — and carving out
to the centerline re-opened those tiles, turning the barrier into a dashed line. Deck
connectivity survives the narrowing because `set_polyline` always walks the
centerline chain, the same guarantee thin waterways rely on.

A curb tile does
**not** block unconditionally — OSM cuts one physical bridge into several ways
(carriageway and its sidewalk are parallel ribbons), so each curb tile records
**every** way that owns it (`CurbTile::owners`) and the decision is an
**outward probe**: step one tile away from the owning way's centerline — if
that point lands inside
another bridge way's ribbon, the tile is an interior seam and stays open; if
it lands on nothing, the tile is the outer boundary of the whole composite
and blocks. The decision is an `any` over the owners, so a dropped owner can
only *lose* barrier, never add it — and a pair is the typical case, not the
maximum: a junction of footbridge ways routinely puts three or more curb
chains on one tile. The probe asks a **spatial
hash of the bridge ribbons** (`BridgeBands`, 32 m cells — the
`planting/index.rs` idiom), never a linear scan of every bridge way: curb tiles
grow with the number of bridges, so the scan was the quadratic part of the fill.
The index is exact, not
approximate — a segment goes into every cell of its AABB grown by its own
`curb_reach`, so a single cell lookup can never miss a covering ribbon.
This survives nominal class widths swallowing a parallel sidewalk
whole (primary is 16 m by default — a "covered by a neighbour ribbon" rule
would open *both* of the pair's outer curbs there).

**The curb tiles live in `bevy::platform::collections::HashMap`, not `std`'s**, like
the map's indexes (the `bevy` skill says why in general, the osm-map skill what it
bought there). Here it bought **nothing measurable** and is kept only for one
vocabulary. **The swap cannot move the mesh**, which is what had
to be checked before making it in a replay-bearing fill: the blocking decision is taken
per tile from its own owners, and the seal pass collects into a `Vec` and applies
after the walk, so neither reads a neighbour the same round — the iteration order is
not an input to either.

A **joining** non-bridge
road opens the curb its panel covers — joining means sharing a node with a
bridge way (`JOIN_EPSILON`); a riverbank path passing a few metres *under*
the span shares no node and opens nothing. The rule only refrains from
blocking, it never re-opens — an open-by-rule tile over water stays water. After the
deck carves a **seal pass** restores the barrier where it degraded to corner contact:
on a narrow (alley-width) bridge the deck centerline chain claims the same tiles as
the way's own curb chain, the deck wins, and the curb continues one column over —
touching diagonally. Our A* cannot cut that corner but northstar's
`OrdinalGrid` (HPA*, Theta*) steps straight through it, the thin-waterway
hazard again. The seal blocks, of the two open orthogonal neighbours of such
a diagonal pair, the one **farther** from the owning bridge centerline — the
deck stays walkable, the gap closes on the outside. Known cost of a
single-level grid: a street passing *under* a dry overpass gets the curb
bands stamped across it — passable where the street runs, blocked either side
of it.

## Linear waterways block, unlike rails

**Linear waterways block, unlike rails** — a `WaterLine` is water, and water is crossed
by bridge, not waded. They carry the rail hazard (an unbroken thread across the
city that `prune_unreachable` would amputate a bank of), so two things keep the map
connected and both are load-bearing: the bridge carve runs *after* this fill, and
**culverts do not block at all** (`WaterLine::tunnel`) — where a stream crosses under a
street, OSM far more often pipes it (`tunnel=culvert`) than bridges the street over it.
The one place the fill stops short of the capsule is the
**culvert portal**: `set_polyline_capped` drops the tiles past the end plane there, so
the mouth of the pipe — the only dry crossing a channel has — is not plugged by the
half-disk. Same `water_line_caps` rule the ribbon is drawn with.

## A rasterized polyline is a 4-connected chain, by construction

**A rasterized polyline is a 4-connected chain, by construction.** `set_polyline` marks
tiles whose *center* is within half the width — and that alone is not a barrier: below
`tile_size · √2` (2.83 m at the default 2 m tile) a slanted band degenerates into tiles
touching only at their **corners** (the navmesh overlay draws it as a chequerboard
along the line). Our own A* cannot step through that — it does not cut corners — but
every other consumer can: `bevy_northstar`'s `OrdinalGrid` (HPA*, Theta*) is built
with no corner-cutting filter and steps diagonally between two blocked tiles, and
`line_of_sight` samples points along a ray and slips through the contact point. A
2.5 m stream was crossed by pawns on HPA* for exactly this reason. So `set_polyline`
also walks the centerline tile by tile (Amanatides–Woo; each step crosses one grid
line, so consecutive tiles share an *edge*). Raising narrow widths to a minimum
instead was tried and rejected: the threshold depends on the line's angle and on its
offset against the grid, and even 3 m still left a gap. Pinned by
`tests/navigation.rs`.

## Building passage (арка)

**Building passage** (арка) — a road that runs *through* a building: OSM
`tunnel=building_passage`, or `covered=building_passage|yes` (both tag styles occur;
`tunnel=yes` is an underground tunnel and is **not** one). `parse/tags.rs::is_building_passage`
sets `RoadLine::passage`; the navmesh carves those centerlines passable **last**, after
buildings and walls, since the whole point is to punch through a block that was just
filled. Carve width is `min(road width, PASSAGE_MAX_WIDTH)` — the way is usually tagged
`service` (5 m) but the arch itself is narrower, and an uncapped corridor would eat a
tile of facade on each side.
**The carve reaches half its width past both ends in both fills.** The grid gets it for
free — `set_polyline` is a capsule; the polygonal mesh extends the ribbon's end links
(`polymesh/build.rs::extended_ends`). It used to be butt-cut there, and at Tula's kremlin
gates — the passage way ends on the tower outline, the `city_wall` band runs right past
it — a hair of band closed every mouth: the kremlin became a hole of the obstacle union,
`shape.first()` dropped it, and the whole inside was an obstacle on the mesh while the grid
let pawns in. Pinned by `polymesh/tests.rs::an_arch_opens_a_barrier_lying_right_at_its_mouth`;
`cargo run --example navmesh_probe -- tula <x,y> …` prints, for points and for every
passage near the first one, the grid and mesh verdicts side by side.
How the arch is drawn (the wall opening) is in the osm-map skill.

## Row-span rasterization

**Row-span rasterization** (`row_spans`): an area is filled row by row — one pass over
the ring per tile row yields the x-crossings, and the tiles between crossing pairs are
set. Holes are subtracted as intervals, *not* merged into one even-odd list, so a hole
poking outside its outer ring still subtracts instead of filling. Replaced a
point-in-polygon test per tile of the AABB, which on London's Thames (huge bbox × long
ring) cost 6.3 s of a 6.5 s fill; now 30 ms.

## prune_unreachable and the portal snap

**prune_unreachable** — the shared `reach::flood` from the portal, taken as a **world
point** and converted through the navmesh's own snapshot; passable-but-unreachable
pockets (enclosed courtyards, islands) become impassable. 4-connectivity matches
A* reachability because of the no-corner-cutting rule.

**PortalPos** (resource) — actual portal position. `snap_portal_position` spirals out to
the nearest tile with clearance derived from `PORTAL_DIAMETER`, through the shared
`nearest_tile_where` ring search — nearest is Euclidean, so the first ring with a hit is
not the answer (a corner at `r·√2` loses to a straight tile of a later ring), capped at
`PORTAL_SEARCH_METERS` = 400 m (`settings.rs`, next to `PORTAL_DIAMETER`; metres, not
tiles, so the navtile cycler does not move the portal). Nothing clear inside the cap ⇒
`None`, and `map/osm/download.rs` logs `no clear spot for portal` and keeps the raw
hint — the prune flood then starts from the unsnapped point.
