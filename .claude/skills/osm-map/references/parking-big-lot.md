# The big lot: kerb and gores

Detail behind `map/roads/lots.rs` and `map/roads/gores.rs`. **This file owns the gore
law** — the road subjects point here. The lot layout itself is `parking-layout.md`;
the outline the lot reads is a parse pass (**Blocks pulled to the roads → `pave_lots`**
in `parse.md`).

## Big and through

- **A big lot shows the road through it**: hiding the roads is right for a yard and
  wrong for eight hectares, which came out as a field of hatching with no landmark in
  it, while 2GIS and Yandex draw the boulevard. A small lot hides its roads as before
  (`parking-layout.md`).
- **Big** is `LotKind::Ground` (`parking::is_ground` reads it), the **paved** outline
  at `GROUND_MIN_AREA` 8000 m² or more — decided by the parse
  (`parse/lots.rs::settled_kind`, `references/parse.md`). **Through** is
  `parking::is_through`: a `Street` that is not a `parking_aisle`, a bridge or a
  passage, and is **one-way, a roundabout or a carriageway**. That is the one thing
  tags say about it: the mall's boulevard is `highway=service` + `oneway=yes`, while
  the plain `service` ways inside the other big lots are their aisles, merely
  untagged — showing every `service` way would have cut those lots' rows along every
  aisle.
- **Drawn by `roads/lots.rs`, inside `mesh_roads`** (so it follows `RoadStyle` and
  the very centreline the ribbon underneath is drawn from): `Grounds::push` hands
  every street's drawn path to the big lots it reaches, `Grounds::layers` builds two
  layers from them. **The road's asphalt is not laid a second time** — it is the
  lot's own tone; what shows the road is its **kerb**, `lot_sidewalks`
  (`Z_LOT_SIDEWALK` 2.002, `SurfaceKind::Sidewalk`, `SIDEWALK_COLOR` — measured
  pixel for pixel the same as a street's sidewalk; a thin light strip on dark asphalt
  only *looks* whiter). `RoadStyle::sidewalks` off takes it off.

## The kerb

- **The kerb is a polygon** (`kerbs`, `i_overlay`): the bands of the through roads
  whose axis enters the lot (`width + 2 · SidewalkProfile::kerb` — its sidewalk band
  by class, tag not read, or `LOT_KERB` 1.2 m on a drive; the stalls stand clear of
  the matching **kerb edge** plus `THROUGH_CLEARANCE`, see `Surroundings::cover`
  below) plus the **island of every roundabout** (the ring's own polygon), **minus
  the asphalt of every street on the lot** — an aisle cuts its **mouth**, so along
  the boulevard the kerb comes out as the islands at the row ends — **and minus the
  gores** at the roundabouts, where there is no kerb between the carriageways at
  all. The result is cut to the lot's outline **grown by `KERB_OVERHANG` 2 m**,
  **opened** by `KERB_OPENING` 0.3 m (slivers under 0.6 m go, corners round) and
  scraps under `MIN_KERB_AREA` 6 m² are dropped. The overhang is what joins the kerb
  to the sidewalk of the street the road enters the lot from: cut exactly on the
  outline it met that sidewalk with a jog — the outline is rounded there by the
  paving's closing and the sidewalk is not.
- **Closedness comes from the raw OSM points, never from the drawn path**
  (`Grounds::push`, `GoreRoad::new`): whether a way is a ring is a property of the
  way, not of a style knob. A closed way used to be smoothed as an open one and cut
  the corner at its **seam** — the two ends of a drawn ring stood metres apart and
  `is_ring` on it was false unless the seam node happened to be shared and therefore
  pinned. That is fixed at the source (a closed way is smoothed round the cycle —
  **The street axis** in `roads.md`), so the re-closing both modules do is now a
  belt: a drawn ring already comes back closed.
- **Ribbons were here first and were removed** — do not bring them back: each road's
  asphalt lay over every *other* road's kerb, so where slip lanes fan out at a
  roundabout the kerbs were chopped into ragged scraps, the island stayed a ring, and
  the two kerbs of a boulevard's halves fused into a light thread between them.
- **The median is a double solid line, not a kerb** (layer `lot_lines`,
  `Z_LOT_LINES` 2.003, flat paint, gated on `RoadStyle::markings`). A boulevard in
  OSM is two one-way ways side by side with half a metre of asphalt between them —
  **paired halves**, found and aligned for the whole city by the network
  (`roads/network/pairs.rs`, `roads.md` → **Paired halves**; the lot looked for them
  itself until then, on its own streets only). The lot takes each **paved** median's
  midline, cuts out the stretch over the lot by probes every `PROBE_STEP` 2 m
  (`on_lot` — a straight midline is two points, both often outside the lot) no
  shorter than `MEDIAN_MIN` (= `PAIR_MIN` 8 m), lays it with `push_rails`
  (`DOUBLE_LINE_WIDTH` 0.2 m at `DOUBLE_LINE_GAUGE` 0.5 m — wider than the real
  0.15 / 0.3, which fuse into a hair at lot zoom) and subtracts a strip as wide as
  the axes are apart from the kerb. Its own line is needed because the street paint
  layer (which draws the same double solid everywhere else) lies under the lot's
  asphalt. The median is computed **once per pair**: from both sides at once (the
  lot's first try) two double lines lay on each other a few centimetres off. The
  «Макси» boulevard is two `service` drives, which is why a `service` one-way is
  `pairable`.

## Gores — the splitter islands at a roundabout

**Gore** (`roads/gores.rs`) — the splitter island at a roundabout: the wedge between
the entry arm, the exit arm and the ring is **asphalt with diagonal hatching**, not a
triangle of sidewalk or kerb. An approach in OSM is two one-way ways, entry and exit,
fanning out to two nodes of the ring; nobody drives or walks on the wedge. It is a
property of the **network at a ring, not of a lot**, so it is computed for every
roundabout in the city and the lot only subtracts it from its kerb.

- **A roundabout is the tag or the shape** — `RoadLine::is_roundabout`, the one
  notion, on the model: `junction=roundabout|circular`, or a closed one-way way. The
  big ring at ТРЦ «Макси» is plain `oneway=yes` closed on itself; by the tag alone
  none of its three islands was found. The shape test started here and was lifted to
  the model when `lane_count` and the parked cars turned out to need the same answer
  — see **Markings → lane_count** in `roads.md`.
- **An arm** is a one-way, non-ring street with an end within `ARM_SNAP` 1 m of a
  ring vertex, taken for `ARM_REACH` 40 m from the ring — an avenue's carriageway
  runs for hundreds of metres and would hatch the whole median with its opposite.
  **Only its fan goes into the closing** (`gores::fan_head`): the gap from the arm to
  the nearest other arm's road is probed every `FAN_PROBE` 1 m from the ring, and the
  arm is cut at the end of the last `FAN_WINDOW` 2 m over which the gap narrows
  faster than `FAN_SLOPE` 0.05 (0.5 m per 10 m) — where the halves stop parting and
  run side by side. Before, the look hung on the two thresholds and not on the road:
  where two halves run 5 m apart between mini-rings 88 m apart, the 40 m from each
  ring almost met and a long narrow hatch ran ring to ring, while where the gap was
  under `GORE_MIN_WIDTH` the double solid ran from the ring — now it is the double
  solid all along the parallel stretch. The arm is still whole for the "touches two
  arms" tests; only the closing network and its reach are cut. A real fan narrows the
  whole way and keeps its length
  (`only_the_fan_of_an_approach_goes_into_the_closing`).
- **The wedge** is what a closing by `GORE_CLOSING` 6 m of the rings' and arms'
  asphalt pulls shut, minus the asphalt of every street nearby and the **island of
  the ring** — the axis polygon of a ring mapped as one closed way, and of every ring
  chained from arcs (`rings::Ring::path`, handed in as `islands`). The second used to
  be missing: no arc of a chained ring is a closed way, and where the arc between an
  entry's and an exit's ring nodes is steep, the fan's chord runs past the ring's
  inner edge — a hatched lens lay on the lane by the island. The same closing fills
  the fillet on the **outer** side of an arm, and arms meet a ring at a shallow
  angle, so that fillet is not small: what tells them apart is the neighbours, not
  the area — a gore **touches two arms** (`ARM_TOUCH`), a fillet one.
  `GORE_MIN_AREA` 12 m² on top.
- **The fan is taken whole** (`gores::fans`). The closing only pulls shut what is
  under two radii wide, and arms diverge faster than that: on a long one-lane
  approach it left two scraps at the tip where Yandex draws one triangle over the
  whole fan. So every **entry and exit** (one flowing into the ring, one out of it)
  that start at **different** ring nodes and end in one node — or within
  `FAN_MOUTH` 12 m of each other (an entry ending ten metres short of the exit's
  node, behind a short link) — gives the outline "out along one, back along the
  other"; the chord between the ring nodes runs over the ring's asphalt and island,
  which the clip takes out with the arms. An arm longer than `FAN_REACH` 55 m is no
  fan: past that the two are streets around a block. The fan is clipped like a closed
  shape and joined with them (`simplify_shape`, NonZero), so a wedge found both ways
  is one gore. **The legs of a Y-approach count as an entry and an exit**
  (`Rings::leg_flow`, **Roundabouts** in `roads.md`): `GoreRoad::new` makes such a
  two-way way one-way and turns its path along its flow, so the fan between the two
  legs is found the same way
  (`roads/tests.rs::a_y_approach_gets_one_island_between_its_legs`).
- **The subtraction is one wedge's business.** The closing has already broken the
  city into separate shapes, and asphalt from the other end of town touches none of
  them, so the difference runs per closed shape against the clip contours whose
  bounds overlap it, and is skipped outright where the bounds say fewer than two arms
  can reach (the same question the exact test asks below, asked of a rectangle
  first). One boolean for all of them cost two to three times as much — half the work
  went on the road bands intersecting each other where there is no wedge at all.
  **A street reaches a wedge by its kerb, not by its axis** — eight metres on an
  avenue — so the bounds it is tested against are grown by its half width; by the
  axis alone one bit of asphalt was missed, and a gore kept a 3-vertex sliver of road
  under it.
- **Two shapes, not one.** The whole wedge, grown `ASPHALT_PAD` 0.5 m under the road
  edges, is **asphalt** (pushed into the `roads` layer — above the sidewalks, so it
  covers their triangle); the wedge **opened** by `GORE_OPENING` 0.3 m — without tips
  and necks too thin for a stripe — is what gets the outline (0.3 m) and the hatching
  (0.25 m every 1.6 m, at 45° to the wedge's long axis). The outline is the heavier
  of the two on purpose: at 0.2 against 0.35 it sank into a hatching of the same
  weight and the island read as a smudge; Yandex reads an island by its outline
  first. One shape for both (tried first) showed ground wherever the opening had cut.
  **A sliver is not hatched**: an opened shape that nowhere reaches `GORE_MIN_WIDTH`
  1.5 m (`is_wide` — shrunk by half of it, nothing is left) keeps its asphalt but
  gets no outline and no stripes, and the double solid runs through the gap instead.
  In a metre-wide sliver the outline and the stripes merged into a ladder with the
  double line inside it.
- **A mouth is hatched too** (`gores::mouths` → `Gores::add_mouths`): a void **hemmed
  in on every side by carriageways** — a hole of the union of the one-way
  carriageways and the carriageways meeting them at a node (all of them costs three
  times as much, and a hole between two-way streets is a block or a node triangle,
  `corners::small_islands`), their drawn axes simplified by `MOUTH_SIMPLIFY` 0.5 m
  and stroked with bevel joins and round caps (a square cap at a way seam stood into
  the mouth as a step), opened by `MOUTH_OPENING` 0.75 m (the hairline between halves
  running side by side is the same hole as the mouth they part from) and kept from
  `GORE_MIN_AREA` to `MOUTH_MAX_AREA` 250 m² — unless something is **mapped** there:
  a building, water, green, a lot, a pitch, a road area, a ring island, a
  node-triangle island, a gore already found, a fork gore's hatching included
  (`Gores::outlines`; a vertex of its outline inside, or one of the hole's inside
  it), or a footway vertex or a pair's midline probe (`MOUTH_MEDIAN_PROBE` 2 m)
  deeper than `MOUTH_POINT_DEPTH` 1 m — a crossing clipping the tip stays out of it —
  and unless the node's own asphalt (the street kerb returns, noses and bends, laid
  before the ribbons) already paves `MOUTH_PAVED_SHARE` half of it, probed every
  metre: that is plain junction asphalt, and a hatched islet in the middle of it is
  not what was missing. A zebra across the void's tip cuts its **hatching** back
  (`clear_crossings`, the zebra's axis grown by half `ZEBRA_LENGTH` plus
  `MOUTH_CROSSING_GAP` 0.5 m): the hatching lay over the crossing's bars. The asphalt
  is not cut — `Gores::add_mouths` paves the whole void (`settle_asphalt`) and
  hatches only the cleared shape (`settle_hatched`), or the strip under the zebra was
  bare ground again. Both halves are `settle`'s, shared with `Gores::of`.
  `a_void_hemmed_in_by_carriageways_is_a_mouth`.
- **The paint is the road paint layer's**: `Gores::islands` hands each hatched shape
  and the direction across its stripes to `Painter::paint_island`, which lays the
  outline as a **band area** — the shape minus the shape shrunk by `EDGE_WIDTH`
  0.3 m (`paint::island_band`, miter joins), drawn with `LineKind::Edge` at a zero
  coordinate across, i.e. full cover — and the shrunk shape as the hatching. Before
  the band area the outline was a closed paint strip along the opened contour: the
  strip overlapped itself at every vertex and at a 10° tip did not meet itself but
  turned back in a whisker, while the hatching lay under the outline and doubled the
  paint there. An area band cannot overlap itself, and at a sharp tip its two sides
  merge into a solid point
  (`an_island_outline_stays_inside_and_does_not_overlap_the_hatching`). The hatching
  is a paint area too (`LineKind::Hatch`, `MeshBuilder::push_paint_area` — the ribbon
  attribute carries the world coordinate across the stripes), and `paint.wgsl` draws
  the stripes and fades them into their mean share when they get finer than a few
  pixels, with the zebras' zoom (`PaintTag::Zebras`). Its own mesh,
  `road_paint_islands` at `Z_ROAD_ISLANDS` 2.0035: above a lot's asphalt and its
  double line, where the rest of the road paint would be covered. Before, the
  stripes were real quads clipped by one boolean for the city, in `lot_lines`, with
  no LOD at all. Gated on `RoadStyle::markings`; the count is `gores N` in the
  `road meshing:` line.
- **Splitters by rule** (`gores::splitters`): an approach mapped as **one two-way
  way** has no fan, so the closing finds no wedge. A carriageway of two lanes or
  more ending at a node of a drawn ring (`roads/rings.rs`) of radius ≥ 10 m gets a
  teardrop on its axis — from 1 m past the ring's kerb, `0.6 × radius` long (6–20 m),
  `0.06 × radius` half wide at the base (0.6–1.5 m) — hatched like a gore; the
  approach is widened around it by an asphalt flare so each lane keeps its width,
  and its paint and ruts break over the island's length. Island and flare are one
  construction, `gores::sweep`: a band of variable half-width along the approach's
  drawn path — the union (`NonZero`) of a trapezoid per piece between stations
  (evenly spaced **plus every path vertex inside the span**, so a piece never
  straddles a bend) and a 16-sided disc at each vertex. Two sides offset along each
  link's normal, which it replaced, jumped `reach · sin(bend)` at a bend and crossed
  themselves on its inner side — a metre-deep saw on the kerb where approaches run
  along the ring before they join it. The flare also **grows** from the carriageway's
  half-width at the ring's kerb to the full `half` at the island's base (a
  smoothstep), instead of starting full-width with a square end whose corners stuck
  out of the ring kerb as steps.
  `a_sweep_round_a_sharp_bend_covers_the_band_without_teeth` pins the sweep.
  **An island stands only on an approach that leaves the ring**: the drawn axis at
  the island's base and the chord from its base to its tip must both lie within 35°
  of the ray from the ring's centre (`SPLITTER_MIN_OUTWARD`), or no island and no
  flare. An approach mapped into its node *along* the ring put the teardrop on the
  ring's carriageway and bent it into a hook; most such approaches are straightened
  first by `rings::reshape` (**Roundabouts** in `roads.md`), and what still runs
  along the ring gets none —
  `roads/tests.rs::a_splitter_island_stays_off_the_ring_when_the_approach_comes_in_along_it`.
  **A leg of a Y-approach gets none** — its island is the fan between it and the
  other leg (above); a splitter on each leg bent two hooks into every approach.
  `roads/tests.rs::a_two_way_approach_gets_a_splitter_island` holds it.
- A **fork gore** (`corners::fork_gore`) is the same hatching ahead of the nose of a
  sharp fork of two streets away from a ring: it starts where the narrower street's
  axis has left the wider one's carriageway and widens to the nose, so the two
  ribbons' overlap is no longer one long tongue of plain asphalt.

## Rules on the lot's asphalt

- **No stall under a through road or its kerb** (`Surroundings::cover`, the band
  `width / 2 + kerb + THROUGH_CLEARANCE` 0.5 m, four corners and the centre; a cheap
  centre-distance reject first).
- **The perimeter drive gets no kerb**, by construction rather than by a rule: a road
  band is not part of the paved outline (see **Blocks pulled to the roads**), so its
  axis is outside the lot (`enters` is false) and it only cuts, never adds. The lot's
  asphalt simply runs into the drive's.
- A carriageway crossing a big lot loses its lane markings on it (the lot's asphalt
  covers its ribbon) — the stated price.
- **Cost — real, and per road rebuild, not per frame.** Not optimised: the kerb and
  the gores depend on the map, the settled `RoadShape` (curve tolerance) and two
  `RoadStyle` toggles only (`sidewalks`, `markings`) and on the sun not at all, so
  caching them per world load is the obvious cut — but the hitch on a sun change is
  **not** theirs to fix: the building layer rebuilds on the same `SunOnMap` and costs
  more than they do. **A `dev` bench number here is a real number, not one to be
  discounted by an imagined release factor**: what these steps spend is `i_overlay`,
  a dependency, and a dependency is built optimised in `dev` too.
