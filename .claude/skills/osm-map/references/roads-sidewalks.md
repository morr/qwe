# Roads — sidewalks, verges, kerb pockets

The street's kerb side: the sidewalk band and its sides, paved paths, the unpaved
streets, the verges and their lawns, the kerb pockets. The hub — the phases of
`build_roads` and the subject index — is `roads.md`. The paired halves cut their band
on the paired side themselves (`roads-pairs-medians.md`, **Paired halves**).

- **Sidewalks** (laid by `roads/ribbons.rs::push_sidewalk`; `sidewalks` layer at `Z_SIDEWALK` 1.6, `SurfaceKind::
  Sidewalk`, light concrete `SIDEWALK_COLOR` over the asphalt-grey `ROAD_COLOR` — the
  brightness step between them is what reads as the kerb) — a **carriageway**
  (`is_carriageway`: `RoadClass::Street` whose `Highway::is_street` — so `service`
  drives get none however wide, and never a `passage`) gets a band `width + 2 ·
  band` (`osm::model::sidewalk_band`: 22 % of the width, 1.2–3 m per side, through
  `RoadLine::sidewalk()`). **The class decides, not the width**: an 8 m width test was
  the class in other words while the width came from the class; with the width derived
  from the lanes (**Sections** in `roads-streets-network.md`) a two-lane street is 7.6 m and a one-lane one-way
  4.3 m, and the old threshold would have taken their pavements away. It sits under the
  **street** ribbon (2.0): a crossing street's fill covers it and the sidewalk ends at
  the junction the way a real one does. It sits **over the alley** one (1.5) — the
  author's call: a yard footway running out onto a street stops at the pavement, not at
  the kerb. The price is the other reading of the same rule — a `footway` mapped
  *alongside* a street (OSM's own way of mapping a pavement) now sinks into the band
  instead of lying on it, which is what the band already draws anyway; and a path
  crossing a street still reaches the asphalt, because a **driveway crossing**
  (`network::driveway_crossings`) is redrawn as a `Street` and a footway that is not one
  is simply covered by the carriageway at 2.0 as before.
  **Paved paths** (`RoadLine::is_paved_path`, `pavement` from `parse-geometry.md`, **Pavement of
  untagged footways**) are not alleys in the drawing: `mesh_roads` lays them **in the
  sidewalk layer** (1.6) in `SIDEWALK_COLOR`; the alley layer keeps only the unpaved sand
  trails and paths nobody decided (a test's hand-built `None`) — a mapped pavement
  merges with the band instead of sitting beside it in another colour. The fill slot is
  picked **after** the street's own band in the loop — a borrow, not a rule. A junction's
  kerb return between two arms of which **both** are paved goes with it
  (`corners/arm.rs::Arm::paved` → `corners/gather.rs::KerbReturns::sidewalks`, counted in `outer[1]`); a pair
  with one paved and one dirt arm fills sand, under the paved ribbon, the way an
  unpaved street fills under asphalt. **Alley corner pieces exist only between arms
  that end at the node** — an L of two ways, a fork: there the butt ends leave a real
  notch. Where an arm *continues* through the node (`corners/gather.rs::continues` — the group
  holds an opposite arm within `MAX_ANGLE` of straight; OSM splits a path into two ways
  right at a crossing, so `Arm::end` proves nothing) no piece is drawn for its pairs:
  alley ribbons are not cut at junction nodes (the asphalt breaks are carriageways
  only), so at a crossing or a join mouth the ribbons cover the corner themselves and
  the piece was a patch on top of the seam. Pinned by
  `roads/tests.rs::a_paved_path_is_drawn_in_the_sidewalk_layer` and the three
  crossing/join/L tests in `corners/tests.rs`.
  **Tile overlaps** (`corners/islands.rs::tile_overlaps` → `corners/gather.rs::KerbReturns::overlaps`, counted in
  `RoadReport::tile_overlaps`): at a merge of two or more paved paths, a yard wedge can
  enter point-first into the seam between the arm edges — the sectors between the arms
  are closed by the kerb rules, but the strip of yard (or bare earth before it) along an
  edge outside them is closed by nothing. The overlap is a tile strip along each free
  edge of the node — `TILE_OVERLAP_WIDTH` 3 m beyond the edge, `TILE_OVERLAP_REACH` 5 m
  along it from the node, tucked under the ribbon by `OUTER_OVERLAP` — laid into the
  sidewalk layer, in `mesh_roads`, but **not fed to the lawn scraps**: the overlap is
  itself the fix, not paving that encloses a scrap. An edge is free when no other arm
  leaves into it within `TILE_OVERLAP_OCCUPIED` 80° (such a corner is already closed by
  a fillet, nose or fan), and the strip is laid only when the yard behind the edge runs
  deep enough for it — two probes at the strip's far end (0.4 · reach, half and full
  width) must both find yard (`YardBounds`, a `Grid` over the bounding boxes, cell
  `YARD_CELL` 32 m; the yards are `MapData::landuse` **and** `MapData::pockets`, both
  drawn as yard by `spawn::mesh_surfaces`): a wedge entering point-first shallower than
  the strip widens further out, and the near half of the strip lay on the green corner
  of the merge as a slab. The gap must also **open from the node**: right past the edge
  at the strip's mouth (`TILE_OVERLAP_MOUTH` 25 cm) there is no yard — bare earth or the
  oncoming ribbon — the yard starts further out. A yard pressed against the edge right
  at the node is no gap, and the strip lay on the lawn as a rectangular slab. Without a
  yard in reach there is no overlap — the ribbons do not swell — and a **through node**
  (every alley arm has its opposite, `continues`) is skipped whole: a crossing has no
  seam for the strip to close, and the strips landed on the crossing as bare
  rectangles. Pinned by
  `corners/tests.rs::a_tile_overlap_covers_the_yard_wedge_at_a_paved_merge`.
  **Unpaved streets** (`RoadLine::is_unpaved_street` — a `Street` whose `surface` is
  `unpaved|gravel|fine_gravel|pebblestone|ground|dirt|earth|mud|sand|grass|compacted|…`,
  the same vocabulary as the paths, `parse/tags.rs::surface_pavement`) are the private sector's
  lanes, and they used to be drawn as grey asphalt with a dashed axis. Now the fill goes
  to its own layer `unpaved_roads` at `Z_UNPAVED_ROAD` 1.8 — above the sidewalks and the
  alleys like any carriageway, **below the asphalt at 2.0**, so where a dirt lane runs
  out onto a paved street the junction is asphalt without any draw-order bargaining — in
  `UNPAVED_ROAD_COLOR` (a warm grey-brown, a step lighter than asphalt and darker than a
  sand trail) over `SurfaceKind::Unpaved`: mottle three times the asphalt's at 14 m
  (puddles, patches of fill), coarser grain, dark gravel speckle, and the same ruts as
  the asphalt (the lane frame stays — a dirt road is driven by the same wheels). No
  paint on dirt: no lane lines (`Painter::paint` returns), no zebra — neither by the
  rule nor off an OSM crossing node, which `node_paint` does not even collect on such a
  road — and no stop line or give-way line, whatever calls it; the sign stays a sign.
  A kerb return between two unpaved arms goes to their layer (`corners/arm.rs::Arm::unpaved` →
  `corners/gather.rs::KerbReturns::unpaved`), and with a radius of at most `DIRT_RADIUS` 3 m whatever
  their class (`kerb_radius`): the street's 6 m arc on a one-lane private-sector lane is
  wider than the lane itself. Pinned by
  `corners/tests.rs::two_dirt_roads_meet_with_a_small_corner`.
  **Between an unpaved and a paved arm there is no kerb return at all** — the way a
  gravel lane really meets a street: the asphalt runs straight past the mouth, and the
  dirt enters its edge as it is, without asphalt flares or bevels (the fillets read as a
  paved apron leaving the street for the dirt road). What such a pair still closes — a
  sharp fork's nose, an outer corner — goes to the unpaved layer, under the asphalt.
  **Asphalt split into two ways where the dirt crosses it** (the only two paved arms of
  the node, nearly collinear, both ends) butts on both sides like any junction arm, and
  with no fillet to the dirt the gap on the outside of its kink stayed open — the two
  asphalt arms get the asphalt outer corner between them (`outer_corner`, counted in
  `outer[0]`); pinned by
  `corners/tests.rs::asphalt_kinked_across_a_dirt_road_leaves_no_gap_at_the_seam`.
  **Asphalt continued by dirt** (two nearly collinear ends of one surface each,
  nothing else of their class at the node) ends square on both sides: the round cap of
  the asphalt lay on the gravel as a half-disc; the gap on the outside of a slight kink
  is an outer corner in dirt. For that the seam must stay on the node: the street axis
  passes a free seam as one arc and cuts the ways a few millimetres off it, where
  `kerb_returns` sees no node and the caps stayed round — so a seam where the surface
  changes is **pinned** (`roads/axis.rs::Run::stitch`, `resurfaced`), the same as a seam
  another street joins. Pinned by `a_dirt_road_enters_the_asphalt_without_kerb_returns`
  and `asphalt_turning_into_dirt_ends_square`; gallery `kaluga/07_gravel_tee`.
  **An asphalt street ending at a
  dirt road** — the only paved arm of its node, the node's other two or more arms dirt —
  stops at the dirt road's kerb: `corners/gather.rs::KerbReturns::setback` gives that end the widest dirt
  arm's half width, and `mesh_roads` cuts the fill back by it (`tapers::cut`; not on an
  end under a taper) — otherwise the butt lay over the gravel as a square tongue up to
  the dirt road's axis, with a step on each side. An asphalt road crossing the
  dirt one (two paved arms) runs through as before. The sidewalk is already gone by
  the parse (`untagged_sidewalks`). An untagged street stays asphalt — the tag is rare,
  and a guessed dirt road in a city block would be a worse lie than a missed one. Pinned
  by `roads/tests.rs::an_unpaved_street_draws_in_its_own_layer_without_lines`.
  A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `mesh_roads` `continue`s into `Bridges::push_deck`
  + `Bridges::fills` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`: it would hang past the deck edge over the water, and the deck
  already has its own kerb: `push_bridge_curb`, drawn unconditionally
  (except on the side of a paired deck, **Paired halves** in `roads-pairs-medians.md`). The approach's band ends
  **square** at the bridge head (`bridges::seam_ends`), with its verge: its half-disc
  showed past the deck curb on both sides.
  The road fill went from osm-carto white to asphalt grey together with the
  markings: a white line on white is invisible, and on grey the street grid also stops
  merging with the courtyards. At a junction the band turns the corner on the kerb's own
  arc — **Kerb returns → The sidewalk turns with the kerb** in `roads-corners.md`.
  **A half of a divided street has no band on its paired side** (`Pairs::band_pieces`
  cuts it, `roads/ribbons.rs::push_sidewalk` lays the pieces; **Paired halves** in `roads-pairs-medians.md`): along a pair
  run the band is the width plus one sidewalk, shifted half a sidewalk away from the
  partner, and the full band resumes past the run with a butt joint; a piece under
  `pairs::SIDEWALK_PIECE_MIN` 0.5 m between two runs is skipped (centimetre offcuts), and
  so is a piece left with no side at all. `None` from `band_pieces` is the whole band on
  both sides, laid uncut. On a half with a taper the runs are shifted by the head
  wedge's length, since the band is laid along the body past it.
  **`sidewalk=*` picks the sides**: `RoadLine::sidewalks` `[left, right]` along
  the points (`parse/tags.rs::tagged_sidewalks` — `both|left|right|no|none|separate`,
  refined by `sidewalk:both|left|right`; `no` and `separate` mean no band, a separate
  footway draws itself; `oneway=-1` swaps them with the points). Each side is a
  **`SidewalkSide`** (`osm/model.rs`): `Tagged` from the tag, `Inferred` without one,
  `None` where there is no band — whatever took it. **Untagged** (no
  `sidewalk*` key at all — `tagged_sidewalks` returns `None`) is decided by the street and
  what stands around it, not taken as "both": an unpaved `surface`
  (`gravel|unpaved|ground|dirt|compacted|…`, `parse/tags.rs::untagged_sidewalks`) never has
  one; a residential / unclassified / living street keeps both only where the mean
  **storeys of the blocks** along it (`cars::district::Districts::storeys_at`, the very
  measure that thins the parked row, probed every 40 m) are at least
  `SIDEWALK_STOREYS_MIN` 3 — the private sector and an empty field get a kerb with no
  band (`parse/sidewalks.rs::infer_sidewalks`, right after the drowned buildings, so the house
  pull and the block pull already read the decision); trunk…tertiary and the links keep
  both (silence means "yes" in a city centre and "no" among private houses). And any
  carriageway — arterials included — gives an untagged side up to a **separately mapped
  paved footway** running alongside it (`parse/verges.rs::measure_footways_beside_streets`,
  `parse-geometry.md`, **Sidewalks left to a separate footway**): the band beside it was a second
  sidewalk — but only behind a real lawn (`SEPARATE_LAWN` 1.5 m to the footway's near
  edge); a footway at the kerb keeps the band under it. **Any side** of a paved street
  with such a footway alongside — up to `VERGE_REACH` 10 m past the kerb (16 m on a
  two-way street), band or no band, dropped or `sidewalk=separate` — gets a **verge**
  (`RoadLine::verges`, metres from the kerb to the footway's axis, the median over the
  probes, and its profile by place, `RoadLine::verge_at`), drawn by
  `roads/verges.rs::push_verges` as a one-sided strip in sidewalk tile, from the axis past the
  kerb — with a profile a polygon of varying width on the path densified every
  `VERGE_STEP` 2.5 m, carried `VERGE_END_OVERLAP` 0.3 m past each end along the tangent
  so the verges of two ways of one street overlap at their seam (a disc of the end's
  width was meant to do that and never drew: it was a ribbon over a 1 cm stub, and
  `merge_ribbon_points` merged the stub into a point — a hairline across the verge at
  every such seam); without one the old ribbon — round-capped, in its own
  layer **`road_verges` at `Z_ROAD_VERGE` 0.1 — under the landuse blocks and every
  green**: a lawn mapped between the kerb and the footway stays a lawn, and bare ground
  there is paved (framed by the kerb returns at every corner of the centre). **The
  corner** comes from the kerb returns (`corners/gather.rs`, `KerbReturns::verges`): between
  two sidewalk-carrying arms where at least one side has a verge, a fillet by the verge
  at the node (`verge_at` there; the band where there is none) with the arc at the kerb
  radius less the **narrower** of the two — by the wider, as the band corners do, it
  left a wedge of ground along the narrower. **The corner arcs facing the sidewalk and
  the lawn are cut finer than the ribbon arcs** (`KERB_ARC_SUBDIVISION` 3 — the
  `fillet` of the verge, sidewalk **and road** corners, the kerb pad's own arc, and the
  `bend` of a corner a bent edge refuses the straight fillet at): at the ribbon's own
  `ARC_TOLERANCE` step a corner arc ran to half a dozen chords turning 12–17° on each
  other, and those joints are what the eye follows — the sidewalk band's edge against
  the lawn read as a broken line of 4–5 segments, and the road corner needs it just the
  same: at an ordinary crossing the corner piece's arc **is** the kerb line between the
  two streets' straight edges — the ribbons end at the node on straight butts and
  nothing else covers the corner. A third of the step turns each joint by 4–6°
  (`kerb_return_arcs_turn_smoothly_at_the_lawn`,
  `a_road_corner_arc_is_cut_at_the_kerb_step`). **A wide verge is a lawn** (`paved_verge`): tile covers a verge up to
  `VERGE_PAVED_MAX` 4 m whole; wider, only a `VERGE_KERB` 0.5 m strip at the kerb —
  **a step, not a ramp** (narrowing the tile over the extra width instead laid a thin
  slanted wedge of lawn between the tile and the footway, and teeth where the profile
  wavered around 4 m). Along a verge by place `roads/verges.rs::verge_runs` decides lawn or
  tile per vertex, folds a run shorter than `VERGE_RUN_MIN` 10 m between runs of the
  other kind into its neighbours (the end runs stay — the node's corner is there), and
  at each change inserts a vertex pair `VERGE_SEAM` 0.05 m either side of where the
  verge passes 4 m, so the tile ends in a seam across the street. Pinned by
  `a_verge_turns_from_tiles_to_lawn_across_the_street`.
  **A verge wider than `VERGE_MID_MAX` 5 m is not drawn at all from
  `VERGE_ZOOM_MAX` 2.0 m/px** (`VergeLods`, `VergeZoomBucket`): tile,
  kerb strip, lawn and yard — the whole side. A 5–11 m band of the yard's muted
  grass beside the carriageway read at that zoom as a second carriageway,
  and what remains in its place is the lawn or earth underneath: the
  park fill, the block's yard, ground. The gate is one place,
  `Drawn::verges_drawn` — the single answer the ribbon (`push_verges`), the
  kerb returns and the pads all read — so a dropped side's corner fillets and
  pads vanish with it. The 5 m line is its own, not `VERGE_PAVED_MAX`: up to
  4 m the tile still runs whole at mid zoom (two tile strips at the kerb read
  as a sidewalk, not a carriageway), and 4–5 m keeps the near treatment; the
  width is the side's widest probe (`RoadLine::verge_widest`), so a profile
  spilling above 5 m drops the side whole. The zoom bound is the crowns' near
  edge (`trees::TREE_LODS`) — a 2–6 m crown is 1–3 px there, and a verge band
  of the same width stops reading as a street detail. The crossing **rebuilds
  the road layers** (`rebuilds_on` gained `retuned::<VergeZoomBucket>` — the
  verge ribbons are baked into the shared layer meshes, there is nothing to
  flip), and the bucket is seeded at world entry before `spawn_map` builds
  them. Pinned by `a_wide_verge_is_not_drawn_past_the_near_zoom_step`.
  **A probe inside a crossing measures nothing**: the verge profile is probed every 5 m,
  and a probe standing in another carriageway's footprint (its asphalt + sidewalk band)
  is a junction mouth — a parallel footway found from there lies *across* the mouth, and
  the profile it fed ballooned over it, throwing the verge lawn past the crossing in a
  capped tongue into the neighbour block. Parallel hits at such probes are dropped;
  slanted ones stay — they are the footway wrapping the corner, the verge's own edge
  (`a_probe_inside_a_crossing_footprint_does_not_measure_across_it`).
  **Foreign here means a street of the road network, not a way** (`RoadNetwork::street_of`,
  built before the pass): a street glued from several ways stands, at the seam of its own
  links, in the footprint of its own continuation — the same as standing in its own
  asphalt — and suppressing by way there took the probes away from the seam links until
  they fell under the `SEPARATE_SHARE` 60 % bar and lost the verge whole
  (`a_probe_on_a_seam_of_its_own_street_measures_the_footway`).
  **The verge reaches under its footway** by `VERGE_UNDER_PATH` 0.5 m past the axis — the
  tile of a narrow verge and the lawn of a wide one (the kerb strip keeps its width): the
  profile is measured on the **raw** axes of the street and the footway, the ribbons lie
  on the smoothed ones, and a smoothed axis that runs metres off its raw one left the
  tile edge past the footway's half-width with a thread of ground a decimetre wide along
  the footway (read as a stroke of paint on the tiles). Under the footway the edge is
  not seen, and past a block the tile lies under it anyway (`Z_ROAD_VERGE` <
  `Z_LANDUSE`). Pinned by `a_verge_reaches_under_its_path`.
  The whole verge — body
  and end discs where the width is past 4 m — goes as grass under the tile, and **which
  grass is the neighbour's**: by default the yard's muted grass (`RESIDENTIAL_COLOR`,
  `SurfaceKind::Yard`, **`road_verge_yards` at `Z_ROAD_VERGE_YARD` 0.09**) — beside a
  residential block the noise is by world position, so there is no seam at the block's
  edge; a meadow (`GRASS_COLOR`, `SurfaceKind::Grass`, **`road_verge_lawns` at
  `Z_ROAD_VERGE_LAWN` 0.08**) only beside a **mapped lawn or park** (`Meadows::beside` —
  `MapData::parks` or `MapData::grass` under the verge's middle or 3 m past it, probed
  at a quarter, half and three quarters of the way), like the lawns it continues. One
  colour for both read wrong both ways — the meadow lay bright along every street of a
  district, the yard green lay heavy beside real meadows, and asking only for the yard
  left a light-green ribbon in front of a square, a lot or bare ground — hence the
  default is the yard grass and the meadow is the exception. A corner between two verges
  both wider than 4 m goes there too (`KerbReturns::verge_lawns`, `Meadows::under` — any
  vertex or the centre on a mapped lawn). **A corner with a lawn behind it
  gets a kerb pad** (`corners/shapes.rs::kerb_pad`): tile `KERB_PAD_WIDTH` 3 m deep along the
  road fillet's own arc (same centre, `FilletArc`), running on `KERB_PAD_RUN` 4 m along
  each straight kerb, into `KerbReturns::verges` — the zebras land at the corner, and
  without it they ended on grass, a lawn sickle between two lawns. The run
  stops at the first shared node on the arm's straight run (`Arm::crossed`): a footway
  crossing the street right past the corner lays its own ribbon over the verge, and the
  pad's tail beyond it stood in the lawn as a square of tile a metre across.
  **An arc tighter than the pad** (a drive's 2.5 m, or a fillet clamped by a short
  straight run) is not an annulus — its inner arc would collapse into the centre, and
  where an arm has no tail (its straight edge ends at the tangent) a tooth of lawn
  showed behind the centre; there the pad is the whole corner `KERB_PAD_WIDTH` deep from
  both kerbs, one convex piece from the arc out to where the two back edges meet.
  **The pad is no wider than the sidewalk band** on an arm that carries one: the lawn
  then starts behind the band, and a 3 m pad behind a 2.5 m band stepped the lawn's
  edge out by half a metre where the tail ended and where the arc began. Each arm has
  its own depth (its band, else the full 3 m), the tail keeps its arm's, the annulus's
  inner arc slides from one to the other (`FilletArc::points_between`) and the
  tight-arc corner meets both back edges at their own depths.
  **A lawn scrap is tiled** (`roads/scraps.rs`, `Scraps`): the pad, a crossing path
  and a footway turning toward it cut the lawn at a corner, and a footway pressed
  against the kerb leaves a thin run of it, so what is left visible is a green pocket in
  the middle of the tile. The question is asked of the drawn picture, not the tags: the
  verge lawn (`push_verges`
  hands it over as the band from the axis to the verge edge on its 2.5 m points, and the
  corner lawns `KerbReturns::verge_lawns` whole) **minus the paving around it** — every
  paved carriageway and paved path as a band of `Drawn::band_half` per side on its ribbon
  axis, the verge tile bands, the street/sidewalk kerb returns, noses and bends, the
  verge corners and pads — in one `i_overlay` difference per lawn piece, pieces across
  threads (`in_parallel`), the paving cut into `CHUNK` 16-link pieces and found through a
  `Grid` of 32 m cells. **Only the ends of a verge lawn that end in a node are asked**
  (`push_verges` gets `KerbReturns::butt` as `ends`; a seam of two ways of one street
  runs on into the next way's lawn) — `END_REACH` 10 points, 25 m from the end, the
  whole band when it is shorter than two of those, with the transverse edge where the
  piece is cut off the band kept as a **cut**: a piece reaching a cut runs on into the
  lawn and is never a scrap. **A bite is asked the same way**: an interior
  vertex of the street where another road *ends* on a shared node (`Bites` keeps the
  shared end nodes with their continuations; a continuation of the same street is a
  seam, not a bite) — the window opens `WEDGE_REACH` 6 points (15 m) to both sides,
  overlapping bites merge into one, both edges cuts. That is where a driveway mouth or
  a slip cut pinches the strip mid-block; asking every 40 m of the strip cost ~80 ms and
  a scrap mid-block without a bite (a footway kinking against the kerb between two
  drives) is therefore left as
  it was. The paving is gathered **after** the ribbons and only near a lawn piece
  (`Scraps::near_lawn`, a grid of the pieces' boxes): pushing every street band and
  every corner of the city cost ~10 ms more than the difference itself — the gore
  outlines (`Gores::outlines`, asphalt and hatched mounts alike) feed it too: a mount is
  not lawn. At a fork the mount **adjoins** the island tip without crossing it, so the
  subtraction severs nothing there — the tip is taken by the tail threshold below, not
  by the mount's outline. A piece of the
  remainder is a **scrap** when it is small — `SCRAP_AREA_MAX` 10 m², or thinner than
  `SCRAP_WIDTH_MAX` 1.5 m on average (twice the area over the perimeter), or a **wedge**:
  `SCRAP_WEDGE_AREA_MAX` 25 m² tapering to a tip, two edges of `SCRAP_TIP_EDGE` 1 m
  meeting under `SCRAP_TIP_ANGLE` 20° — and
  **enclosed**: at most `SCRAP_OPENING` 1 m of its boundary lies on the lawn polygon's
  own edge (an edge counts when its middle and both ends are within 2 cm of it). Past
  that edge the lawn goes on — the next street's verge, the corner lawn, a block over
  it — and a piece open to it is a strip of the same lawn, left alone (a mouth fang is
  open by metres — and the thresholds that would take it would take the
  intended corner lawns). A scrap
  grows `SCRAP_OVERLAP` 0.2 m (bevel) under the paving around it — the paving was taken
  on the unsmoothed band offsets — and goes into `road_verges` in tile colour; that
  layer lies above both lawn layers, so nothing else changes order. **Ground green is
  asked too**: a parse pocket or a mapped grass polygon is itself a candidate
  when it is a narrow strip — `GREEN_AREA_MAX` 500 m², `GREEN_WIDTH_MAX` 10 m on
  average — with a vertex within `GREEN_BITE` 2 m of the drawn paving
  (`Scraps::push_green`, fed after the paving so the bite is seen). The wedge of a
  courtyard pocket pinched between tiles and the fork island's tip
  live there, not in a verge strip; their scraps go into `sidewalks` in tile colour —
  the one tile layer above the grass (`Z_GRASS`), where an overlap with the
  neighbouring tile of the same material renders identically. A wide yard or a strip
  with no paving at its vertices is not a candidate; the enclosure metre keeps its
  meaning for a verge lawn — past its edge the lawn really runs on — but a pocket's own
  edge is not the lawn running on, it is the pocket's loose outline sticking out past
  the drawn tiles, so a wedge-sized green piece (up to `SCRAP_WEDGE_AREA_MAX`) may stay
  open to it over `GREEN_OPENING_MAX` 8 m and still tile. **A green wedge-tail is tiled
  whole**: the fork island's tip — a mapped grass wedge tapering to a 13° point — is one
  remainder that no threshold took; the gore and the pavements *adjoin* it without
  crossing, so nothing
  was severed off it. A tail is recognized by shape, not area: up to
  `GREEN_TAIL_AREA_MAX` 160 m², tapering to a tip, at most `GREEN_WIDTH_MAX` on average,
  and **severed** — at least `TAIL_BASE_MIN` 2 m of its boundary is drawn paving, so a
  lawn that paving merely grazes at a corner stays grass. For a tail the openness metre
  is not asked at all: its open sides are its own converging flanks, and a mouth fang
  is a blunt strip without a tip — the taper is what keeps it grass. The
  errors are safe
  one way: paving not counted only leaves a piece open to the lawn's edge, and it stays
  lawn; a band is taken **per side** (`band_half`), not by the wider one, because
  over-counted paving is what could fake a small enclosed piece. Parks and woods are
  never asked — a real lawn in a square is a polygon of its own, not a verge. (The
  ends-and-bites windows together cost about half of what the continuous survey did.)
  Pinned by `a_lawn_scrap_enclosed_by_paving_is_tiled`,
  `a_remainder_tapering_to_a_tip_is_a_scrap_beyond_the_average`,
  `a_ground_green_wedge_bitten_by_paving_is_a_scrap`,
  `a_green_piece_open_to_the_lawn_stays_grass`,
  `a_green_island_tail_tapering_along_is_paved_whole`,
  `a_fat_green_remainder_is_not_asked_into_a_scrap`,
  `a_wedge_bitten_mid_strip_is_asked_at_the_bite` and `scraps::tests`.
  In a microdistrict
  the tile laid from the kerb to a footway fifteen metres off made the street read as
  poured concrete; there it is a lawn with the footway on it, as in any Soviet yard,
  while the narrow paved verges of the centre's corners stay tile, and nothing between
  the kerb and the footway is bare ground either way. **A ring arc takes
  its verge on the outer side only** (`Ring::ccw` says which: the island is on the left of
  a counter-clockwise ring) — inside is the island and its lawn, and without it a
  parallelogram of bare ground lay between the ring's sidewalk and a footway along it;
  the arc's end discs lie under its own asphalt.
  Render-only: the navmesh and the house pull never read it.
  **One profile for the parse and the renderer**: `RoadLine::sidewalk()` →
  `SidewalkProfile` (`osm/model.rs`) — the sides as bools plus the **band by class**
  (`sidewalk_band`, 22 % of the width, 1.2–3 m; `Some` on a carriageway, a bridge
  included, `None` on a drive or a path). It is **derived, never stored**: the driveway
  crossings and the ring arcs are clones of a way with another `width`, and a stored band
  would go stale on them. It answers `band()` (by class, tag ignored), `on(side)`,
  `any()` (at least one side — `sidewalk=no|separate` on both gives `None`), `both()`,
  `sides()`, and the three edges the parse reads — **mapped edge**, **verge edge**,
  **kerb edge** (`references/parse-buildings.md`, **Houses pulled off the sidewalks**;
  `references/parse-geometry.md`, **Blocks
  pulled to the roads**). Every edge is one reach per road on both sides, even for a
  one-sided `sidewalk=right`; a per-side edge would change the map, so it is not here.
  The renderer's sidewalk is that profile under the Sidewalks toggle (`Drawn::sidewalk_drawn`,
  and per side `sidewalk_on` / `band_half` through `sides()` / `on(side)`).
  The drawn sidewalk is `None` when neither side has one; `push_sidewalk` lays a one-sided
  band the paired-half way (width plus one sidewalk, shifted half a sidewalk to its side)
  and ANDs the tag with the pair runs; the kerb returns drop the arc on a missing side;
  a taper's sidewalk wedge is laid per side, and a side without a sidewalk is the bare
  half (**Streets, sections, tapers** in `roads-streets-network.md` — a one-sided street gets it too). The parse's
  house pull still keeps its clearance
  on a side the tag took the sidewalk from — a verge instead of a sidewalk there.
- **Kerb pockets** (`map/roads/pockets.rs`) — where parked cars stand along a
  street, one answer for the ribbon and for `map::cars` (a second answer would put a car
  beside its pocket, on the sidewalk). `RoadLine::parking` `[left, right]` is
  `parse/tags.rs::tagged_parking` over `parking:<side>|both` (2022 scheme):
  `street_side` → `KerbParking::Pocket`, `lane|on_kerb|half_on_kerb|shoulder|yes` →
  `Lane`, `no|separate` → `No`, and `parking:<side>:restriction=no_stopping|no_parking|
  no_standing` → `No` over anything. `Untagged` is resolved by `pockets::kerb_parking`:
  trunk/primary/secondary → a pocket where that side has a sidewalk to cut it into, else
  none; motorway and links → none; the rest → the lane (the old behaviour). `kerbsides`
  lists the sides in the cars' order (one-way: the kerb of its traffic; two-way: right,
  then left — the RNG stream depends on it); a pocket side's pockets are the axis minus
  `reach + POCKET_CLEARANCE` 6 m around every **row break** (`pockets::row_breaks`: the
  junction breaks without stitches — with **service drives** among the participants, so a
  driveway into a yard breaks the row, `is_row_participant` — plus the taper clearings and
  the **marked OSM crossings**, half a zebra around the node, spilled onto the continuing
  way (`RoadNodes::next_way`, see below) when the node is under `CROSSING_SPILL` 10 m
  from its way's end — the same list the
  cars use; 6 m rather than 4 so the slant starts past a rule zebra, which stands 1–5 m
  past the junction edge), `POCKET_TAPER` 6 m slanted ends where a pocket stops inside the way, at least
  `POCKET_MIN` 10 m at full width. **A pocket across a way end is one pocket**:
  `pockets::all_kerbsides` (the one door for the ribbon and the cars) runs
  `join_way_ends` over the per-way answers — an end open into the way's end stays open
  only if the next way has an open piece on the same kerb there (the side sign flips on
  a way drawn the other way round), otherwise it gets its taper. **The next way is
  `RoadNodes::next_way`** (`roads/network/mod.rs`), the one answer to "which way goes on
  past this end": the most collinear pair in the node within `MAX_BEND` 50°, same class
  and flow as `RoadNetwork` glues streets, else a continuation across streets
  (`RoadNetwork::continuations`) — computed from the roads alone, so the car gallery,
  which has no network, gets it too. It must not be *any* way ending in the node: on a
  tee that picked the side way when it came first by index, and a pocket ran round a
  right-angle seam (pinned `pockets/tests.rs::a_kerb_pocket_stops_at_a_t_junction_side_way`,
  `a_zebra_at_a_way_end_spills_only_onto_the_continuation`,
  `a_kerb_pocket_does_not_turn_a_right_angle_way_end`).
  The pieces linked by open
  ends form a chain and `POCKET_MIN` is measured on the chain, not per way; a piece whose
  tapers outgrow it goes (a taper is not carried across a way end), and the pass repeats
  until nothing changes — per way, a short open piece was dropped while its neighbour
  stayed open into nothing, a square step. The rule bays still see only pieces of
  `POCKET_MIN` and more, so their RNG stream did not move. That full run is what a **tagged** side (`street_side`) gets. A
  side that is a pocket **by the rule** gets rare short bays out of it
  (`pockets::sparse_pockets`): a block run carries bays at all with
  `RULE_BLOCK_SHARE` 0.4, each bay `RULE_POCKET_LENGTH` 24–42 m with both tapers, bays
  `RULE_POCKET_GAP` 30–90 m apart, the first up to 30 m in. The RNG is `seed::Lcg`
  seeded by `seed_from_point` of the street's first point, salted by the side, so the
  ribbon and the cars get the same bays and a rebuild moves nothing. The full-block rule
  pocket read as an extra lane wherever no cars stood in it, and real bays are a few
  cars long; OSM's own bays arrive as tags or as separate `amenity=parking` +
  `parking=street_side` outlines, which reach the parking layer
  (`references/parking-layout.md`) and are paved up to the kerb, cut
  into the sidewalk (`parse-geometry.md`, **kerbside lot** in `pave_lots`).
  **No pocket beside a parking lot** (`pockets::KerbLots`, over `MapData::parking`): the
  kerb is probed every `LOT_STEP` 2 m at the pocket's outer edge, and a probe inside a
  lot or within `LOT_REACH` 6 m of its outline (a sidewalk and a verge) marks that
  stretch as the lot's frontage — the cars go to the lot, and a bay in front of it read
  as a spare lane. A rule bay touching a frontage is **dropped after** the bays
  are laid, so the RNG stream and every other bay of the block stay put; a tagged run
  is **cut** around the frontage like around a row break, its tapers outside it.
  **Any lot kind makes a frontage, not only the kerbside lot of that side.** Resolving
  the two OSM records of one lay-by at parse and letting only a `LotKind::Kerbside`
  outline silence the pocket was considered and rejected: a lay-by's lot is
  `parking=surface`, a `Yard`; narrowing the rule to kerbside lots would put the bay
  back in front of it. The two records of a lay-by are already reconciled by this same
  frontage — the kerbside lot is paved to the kerb and silences the pocket along itself
  — so a parse-side link would have added a second answer to a question this one already
  settles (`a_rule_pocket_is_not_laid_beside_a_parking_lot` pins it on a yard).
  Drawn by `mesh_roads` as three polygons per pocket from
  `pockets::outline` (inner edge 5 cm under the carriageway edge, outer edge between the
  tapers): asphalt `POCKET_WIDTH` 2.5 m wide in `roads` and the **sidewalk pushed out
  behind it** in `sidewalks` (the band is at most 3 m, a
  2.5 m pocket would eat it). A car on a pocket side stands at `half + POCKET_WIDTH` from
  the axis where its arclength falls in a pocket's full part, nowhere else unless the
  side also parks on the lane; `kerb pockets N` in the report.
