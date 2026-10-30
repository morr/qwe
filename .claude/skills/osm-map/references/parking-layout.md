# Parking lot layout

Detail behind `map/parking.rs` — how stalls, aisles and lot shapes are laid out. The
module is a parent plus subjects: the parent holds the constants, the stall model,
`Outline`/`Surroundings`, the placement grid and the paint; the aisle-based layout
lives in `map/parking/aisles.rs`, the invented layout in `map/parking/generated.rs`,
the tests in `map/parking/tests.rs`. The big lot's kerb and the gores live in
`parking-big-lot.md`; the cars that fill the stalls in `parking-parked-cars.md`. The
lot outline reaching its road is a parse pass
(**Blocks pulled to the roads → `pave_lots`** in `parse-geometry.md`); everything here reads
the outline it produced.

## Parking

- **Parking** (`map/parking.rs`) — an `amenity=parking` area is drawn as asphalt
  (`Z_PARKING` 2.001) with the **stalls painted on it** (`Z_PARKING_LINES` 2.004 —
  above the two big-lot road layers, see `parking-big-lot.md`). The markings go in a
  **flat-material** layer of their own, not through `SurfaceMaterial`: the procedural
  asphalt grain belongs under the paint, not on it, and a 12 cm line is the one thing
  on this map that must stay pure white.
  - **Road asphalt, no rim.** No rim on the lot's outline — the lot lies over the
    roads, so its outline crosses every drive's asphalt, and anything laid along it
    is a seam. `PARKING_COLOR` is `roads::ROAD_COLOR` and the fill is a bare
    `push_polygon`.
  - **The layout is computed once per world load** into `ParkingLayout` (a resource,
    filled by `spawn_map` from `ParkingLayout::new(&map.parking, &map.roads)`, which
    lays the lots out largest first through `stalls_beside(lot, aisles, through,
    drives)` — the production door; the arity-two `stalls` beside it is
    `#[cfg(test)]`), and the paint and the cars both read it.
    **It has a timer of its own**: `ParkingLayout::new` runs *before* `mesh_surfaces`
    at both call sites, so the `SurfaceReport` timer never sees it — the app prints a
    `parking layout: N lots, M stalls in …` line beside `surface meshing:`, and
    `measure_surfaces` carries a `parking layout` row inside the bench's `surfaces`
    line. Re-read that row before adding the next probe to the layout.
  - **Which way the rows run is read out of OSM, not guessed** — `service=parking_aisle`
    (`RoadLine::parking_aisle`, `tags::is_parking_aisle`), the only thing the layout
    takes from the road network.
    - **The main direction is read in two passes** (`main_of`): a rough
      length-weighted direction over every aisle link inside the lot, then the same
      answer again over the links within `AISLE_SPREAD` 30° of it, so the cross aisles
      stop voting. Thirty degrees is more than the aisles' own spread and less than
      the turn to a cross aisle. `rows_of` then keeps only the straight links, joining
      consecutive ones of a single OSM way into one run while the next begins where
      the last ended (`JOIN_SLACK` 1 cm — the links come from one polyline and meet
      exactly; the tolerance is for `f32` at map scale).
    - **Two lanes closer than `LANE_MERGE` (`STALL_DEPTH` 5.2 m) are one lane.** A long
      aisle in OSM is routinely cut into two ways with a slight kink, and counted as
      two lanes they would open a half-metre pocket between them.
    - **A pocket too narrow for a pair gets a single row down its middle**, provided
      `MIN_AISLE` 3 m is left on **both** sides of it (`gap ≥ STALL_DEPTH + MIN_AISLE`
      after the pair's `gap ≥ 2 × depth + PAIR_AISLE` has refused). Three metres is
      the narrowest strip a car still squeezes through to the stall; below it the
      pocket is left as bare asphalt.
    - **`Placed` stays** — a grid at `OVERLAP_CELL` and a separating-axis test with
      `OVERLAP_SLACK` 5 cm of give (without the give the fp error between two
      neighbours touching exactly would drop every second stall). Pockets do not
      overlap by construction, so it only guards odd data.
    - The row otherwise ends where the outline ends it. **Spurs and perimeter loops
      that `rows_of` throws away never become corridors** — taking them was measured
      and rejected; `the_elbow_that_leads_to_an_aisle_gets_no_rows` pins it.
    - **The strictest tag, not any `service`.** `driveway`, `alley` and `drive-through`
      lead *to* a lot, not along its rows; taking them would turn the rows 90°.
  - **A lot with no aisle in it gets an invented layout** (`generated_rows`) — rows
    along the **longest side of the outline** (`PolyArea::longest_side`), the same one
    the parse's kerbside lot reads in `runs_along`.
  - The invented layout's own law: **the field starts with a single row at the edge,
    not with a pair** — a pair-first start put the very first row's back in the second
    row and its nose in the lot's own edge, where nothing could reach it.
  - **A stall survives three tests**: all four corners inside the outline (`fits`,
    the test the roof clutter uses for its boxes), a run of at least `MIN_ROW_RUN`
    2 stalls, and asphalt in front of the nose (`reachable`, `PAIR_AISLE/2` ahead).
    On the invented layout `reachable` is meaningful only together with the
    per-row nose rule — with one nose per lot it would delete the wall-side row
    rather than turn it. **A lot striped one row wide is not asked**: it is a strip
    along a street, entered from the street, and the outline knows nothing of the
    street — the exception `every_row_has_an_aisle_to_drive_in_from` skips. On the
    invented layout a break by `row_places` (a cross drive) deliberately does not end
    a run. Measured cost of the two invented-layout rules: −2.0 % of stalls, emptying
    only the lots that held exactly one stall — the lone bar on empty asphalt
    `MIN_ROW_RUN` exists against. `push_markings` tests the **bar** with
    `fits_with(EDGE_MARGIN)` on both paths anyway, so the stroke into nothing is
    already suppressed.
  - **`MIN_AREA` 120 m² gates `push_markings` only** — `fill_lots` reads the
    `ParkingLayout` unfiltered, so a small yard keeps its stalls and its cars, on
    unmarked asphalt.
  - The paint is drawn as the **border between stalls** (one bar to the left of each
    stall, neighbours coinciding), not as a rectangle per stall: that is what a lot
    looks like, and it is cheaper than finding each stall's neighbour. Plus a
    **closing** bar where a run of stalls begins — found by asking whether the
    *previous* stall in the list stands one stall width away, since the stalls of a
    row are generated in order. A bar also stops `LINE_GAP` 0.5 m short of a stall's
    back, or the two rows of a pair read as one line straight through both of them.
    Emitting the far side of an aisle forwards instead of **back to front** is not a
    crash — it silently doubles every bar, two coincident quads per stall.
  - **The lot lies over every road ribbon and sidewalk** (`Z_PARKING` above `Z_ROAD`,
    under the pitch and water): its own outline clips every ribbon exactly, and the
    lot's asphalt *is* the aisle. **Cutting the ribbon's axis at the outline was tried
    first and removed** — a ribbon has width, so the cut left a round cap poking into
    the lot where an aisle ends on the outline, a square end beside a pointed lot
    corner, a step where an aisle running along the edge crosses it, and the light
    ribbon of a road that crosses the lot. The price of the lot on top is stated: an
    outline mapped over a real carriageway hides that stretch of asphalt, markings and
    sidewalk — the kerb cars lie above (`Z_CAR`) and stay. Render-only: the navmesh,
    doors and tree planting still see every road. Parking touches neither the navmesh
    nor tree planting, like the landuse blocks.
  - **On a small lot the stall layout leaves no gap for a road, and must not.** Stalls
    dropped under a crossing road read as an unexplained empty band across the rows
    once the lot hides the road. The rule that a gap and a visible road go together is
    the same one the big lot obeys from the other side (`parking-big-lot.md`).
  - **`Z_PARKING_LINES` lies above both big-lot layers** (`Z_LOT_SIDEWALK`,
    `Z_ROAD_ISLANDS`), so a stall bar is never covered.

## Reading the roads around a lot

- **What the layout does read of the roads around a lot** (`Surroundings`, built per
  lot in `ParkingLayout::new` from the streets whose bounds come within `AISLE` of it):
  the through roads (see `parking-big-lot.md`), and the **drives** — every street band
  beside the lot is asphalt a stall can be entered from (`reachable` asks the lot
  **or** `Surroundings::paved`). Without it a strip along a drive lost every stall:
  its nose is in the drive, and the drive is not in the outline. On a big lot a
  through road is left out of the drives — there is a kerb in the way.

## Fields, blocks, pairs, pockets

- **Aisle fields** (`fields_of`). A big lot is not always one grid. Fields are peeled
  off in turn by the same `main_of` over the links not yet taken; a second field needs
  `FIELD_MIN_LANES` 3 distinct lanes, which is what keeps a chain of spurs (one line,
  one lane) from becoming one. The lane grid of each field still runs out to the
  outline, so **whose ground it is** is decided per stall: the nearest aisle run must
  be of the stall's own field (`Frame::territory`). A lot with one field is never
  asked.
- **Aisle blocks** (`blocks_of`) — a field is split once more, and each block is a
  `Field` with its **own lane grid**. A lane is one number, an offset across the lot,
  and the grid used to be one per direction. Runs join a block by two rules:
  - **neighbours** — within `BLOCK_LANE_REACH` 40 m across (two steps of the grid: a
    pocket may carry a footway instead of an aisle) and overlapping along by
    `BLOCK_OVERLAP_SHARE` 0.5 of the shorter. The share matters: aisles crossed
    ~26° off square overlap by a couple of metres without being neighbours;
  - **pieces of one lane** — collinear within `BLOCK_COLLINEAR` 1 m, up to two
    `AISLE` apart, and **no through road between them** (`Surroundings::crossed`):
    the gap on a boulevard is as wide as a cross drive's, and only the road tells
    them apart. Without this rule the cross drive would split its block and
    `cross_drives_of` would find nothing.

    A block stripes `BLOCK_REACH` 30 m past the ends of its own runs, not the whole
    lot — the rest is another block's by `Frame::territory` anyway — and `push_row`
    asks the outline before `owns`, which walks every run of the lot. Pinned by
    `aisles_across_a_through_road_keep_their_own_lane_grids`.
- **A pair of rows is laid as a rectangle** (`Frame::push_pair`). Two rules:
  - **whose ground it is is asked once per pair**, at the pair's centre (`cells`'s
    `anchor`), not per stall: on the seam of two blocks the left row went to one and
    the right to the other, each from its own grid;
  - **a short overhang is trimmed**: stalls of one row with no partner behind them,
    up to `PAIR_OVERHANG` 2 in a run, go. A longer run stays — that is a row along a
    skewed edge where the second one has nowhere to stand, and a single row at the
    edge is legitimate.

    `push_row` is now `lay(cells(..))`: `cells` answers per slot of the lane's grid,
    `lay` applies `MIN_ROW_RUN` and the list order `push_markings` expects. Pinned by
    `a_pair_of_rows_cut_askew_is_trimmed_to_a_rectangle`.
- **Pockets** (`pocket_depth`, `pocket_rows`) — a one-row strip along a street.
  Recognised by thickness, `2 · area / perimeter`, from `STALL_DEPTH_MIN` up to a
  stall with both margins; the stall takes the strip's own depth with `STRIP_MARGIN`
  0.2 m (clearance for the corner test, not a design gap). The row is laid **along the
  sides of the outline**, longest first, not on the bounding-box grid — a strip a
  third of a kilometre long bends with its street, and one degree of bend walks a
  straight grid out of it; the row along the opposite side lands on the first and is
  dropped by `Placed`. One nose per side, toward the street if `paved` finds it in
  front. The bar test follows (`push_markings`): it asks for `EDGE_MARGIN` beyond the
  bar **along the row** and for as much margin in depth as the stall itself has, or a
  pocket would get no paint.
- **Parallel rows** (`parallel_rows`) — a **kerbside lot** (`LotKind::Kerbside`)
  whose thickness leaves less than `STALL_DEPTH_MIN` for a stall across it. That is
  most OSM lay-bys. The cars stand **along the kerb**: a stall is `PARALLEL_LENGTH`
  6 m down the sides of the outline (the car row's own pitch — `cars::CAR_PITCH` is
  defined as this constant, so the two cannot drift apart — room for a Gazelle),
  longest side first, `PARALLEL_WIDTH` 2.2 m across in the corner test, centred on
  the strip — across the strip by `strip_width`, the short side of the rectangle with
  the outline's area and perimeter, since `2 · area / perimeter` reads a 4 × 60 m
  strip as 3.75 m and would push the row a quarter metre off centre. The opposite
  side's row lands on the first and `Placed` drops it. `Stall::along` is the car's
  heading, along the kerb, and it follows the **traffic of its side** (`TrafficSide`,
  an argument of `ParkingLayout::new`): the street is behind the strip edge where
  `paved` finds its carriageway, and with right-hand traffic it is on the car's left.
  `Stall::parallel` marks such a stall and `push_markings` skips it — the bars along
  its sides would lie along the kerb and read as a lane edge. A **yard** strip that
  thin stays empty (`a_strip_thinner_than_a_stall_gets_no_stalls`); only the lot OSM
  calls a lay-by is read as one.
- **Overlapping lots do not share stalls.** Two lots paved to one drive both hold the
  gap between them, and both layouts striped it — cars of two lots parked across each
  other. Lots are laid out from the largest down against one shared `Placed`, and a
  stall landing on one already standing is dropped; results are stored in map order,
  so `ParkingLayout.0[i]` is still `MapData::parking[i]`.

## Outline

- **`Outline`** — every point test of the layout and the markings goes through an
  index of the ring's edges by horizontal bands of `OUTLINE_BAND` 4 m (same even-odd
  answer as `point_in_area`, pinned by `the_outline_index_agrees_with_the_full_ring`,
  which checks it against the full walk on the three shapes the paving makes: a
  concave outline with a hole, a lot with a hole out to the edge of its bounding box,
  and a thin part cut off by the apron). A paved outline carries its fillets — the
  mall lot is 1220 vertices, and walking the ring for each probe cost 87 ms on that
  lot alone. **Every** point test means it: `aisle_rows` once asked `point_in_area`
  for its probes long after the index existed — there is one way to ask the outline a
  question, not two.
