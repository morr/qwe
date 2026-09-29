# Roads — sidewalks, markings, junctions, bridges

Detail behind `map/roads.rs`, `map/roads/*` and the street half of `surface.wgsl`.
`SKILL.md` keeps what every layer shares — the layer seam, the surface material and its
`Ribbon` attribute, the shadow rules, zoom buckets — and the `RoadLine` model; this file
carries how a street is drawn: the sidewalk band, lane markings and their breaks, the
ribbon primitive, junctions and the drawn network (pinned nodes, driveway crossings,
stitches, kerb returns), `RoadStyle`, the bridge layers, asphalt wear, and the junction
gallery `examples/demos/roads`, and **Paired halves** — a divided street and its median.
The big lot's kerb, the stretch of the boulevard's double line over the lot and the gores
at a roundabout are written up under **Parking → A big lot shows the road through it** in
`parking.md`; the parse passes that read a road's width
(houses pulled off the sidewalks, blocks pulled to the roads) are in `parse.md`.

## Rendering

- **Sidewalks** (`map/roads.rs`, `sidewalks` layer at `Z_SIDEWALK` 1.6, `SurfaceKind::
  Sidewalk`, light concrete `SIDEWALK_COLOR` over the asphalt-grey `ROAD_COLOR` — the
  brightness step between them is what reads as the kerb) — a **carriageway**
  (`is_carriageway`: `RoadClass::Street` whose `Highway::is_street` — so `service`
  drives get none however wide, and never a `passage`) gets a band `width + 2 ·
  band` (`osm::model::sidewalk_band`: 22 % of the width, 1.2–3 m per side, through
  `RoadLine::sidewalk()`). **The class
  decides, not the width**: the test used to be `width ≥ STREET_MIN_WIDTH` 8 m, which was
  the class in other words while the width came from the class; with the width derived
  from the lanes (**Sections** below) a two-lane street is 7.6 m and a one-lane one-way
  4.3 m, and the old threshold would have taken their pavements away. It sits under the **street** ribbon (2.0): a crossing
  street's fill covers it and the sidewalk ends at the junction
  the way a real one does. It sits **over the alley** one (1.5), and that is the
  author's call from a screenshot of a yard footway running out onto улица: the path used
  to draw a sand ribbon straight across the light band and on to the kerb, while on a
  photo it stops at the pavement. The price is the other reading of the same rule — a
  `footway` mapped *alongside* a street (OSM's own way of mapping a pavement) now sinks
  into the band instead of lying on it, which is what the band already draws anyway; and a
  path crossing a street still reaches the asphalt, because a **driveway crossing**
  (`network::driveway_crossings`) is redrawn as a `Street` and a footway that is not one
  is simply covered by the carriageway at 2.0 as before.
  **Paved paths** (`RoadLine::is_paved_path`, `pavement` from `parse.md`, **Pavement of
  untagged footways**) are not alleys in the drawing: `mesh_roads` lays them **in the
  sidewalk layer** (1.6) in `SIDEWALK_COLOR`, the alley layer keeps only the unpaved sand
  trails and paths nobody decided (a test's hand-built `None`). Before this every path was
  sand, and in the centre that meant a `footway=sidewalk, surface=paving_stones` lying as
  a sand strip beside the concrete band of its own street and a paved square's grid of
  alleys reading as a beach (scout C1, Frunze street and the Kremlin garden). In the
  sidewalk layer a mapped pavement merges with the band instead of sitting beside it in
  another colour. The fill slot is picked **after** the street's own band in the loop — a
  borrow, not a rule. A junction's kerb return between two arms of which **either** is
  paved goes with it (`corners::Arm::paved` → `KerbReturns::sidewalks`, counted in
  `outer[1]`): a sand fillet in the corner of two tiled alleys read as a stain. Pinned by
  `roads/tests.rs::a_paved_path_is_drawn_in_the_sidewalk_layer`.
  **Unpaved streets** (`RoadLine::is_unpaved_street` — a `Street` whose `surface` is
  `unpaved|gravel|fine_gravel|pebblestone|ground|dirt|earth|mud|sand|grass|compacted|…`,
  the same vocabulary as the paths, `tags.rs::surface_pavement`) are the private sector's
  lanes, and they used to be drawn as grey asphalt with a dashed axis (scout D4: Rostov
  04, Kaluga 06, Oryol 06; Kaluga carries 126 such streets and drives, Rostov 37, Oryol
  39). Now the fill goes to its own layer `unpaved_roads` at `Z_UNPAVED_ROAD` 1.8 —
  above the sidewalks and the alleys like any carriageway, **below the asphalt at 2.0**,
  so where a dirt lane runs out onto a paved street the junction is asphalt without any
  draw-order bargaining — in `UNPAVED_ROAD_COLOR` (a warm grey-brown, a step lighter
  than asphalt and darker than a sand trail) over `SurfaceKind::Unpaved`: mottle three
  times the asphalt's at 14 m (puddles, patches of fill), coarser grain, dark gravel
  speckle, and the same ruts as the asphalt (the lane frame stays — a dirt road is
  driven by the same wheels). No paint on dirt: no lane lines (`Painter::paint`
  returns), no zebra — neither by the rule nor off an OSM crossing node, which
  `node_paint` does not even collect on such a road — and no stop line or give-way line,
  whatever calls it (Kaluga 06: the `give_way` node on Новаторский drew two white stubs
  of the dashed line across the gravel; the sign stays a sign). A kerb return between two
  unpaved arms goes to their layer (`corners::Arm::unpaved` → `KerbReturns::unpaved`),
  and with a radius of at most `DIRT_RADIUS` 3 m whatever their class (`kerb_radius`):
  the street's 6 m on a one-lane private-sector lane (Tula, 18-й × 8-й проезд Мясново)
  was a crisp kerb arc wider than the lane itself — a paved crossing cast in gravel.
  Pinned by `corners.rs::two_dirt_roads_meet_with_a_small_corner`.
  **Between an unpaved and a paved arm there is no kerb return at all** — the way a
  gravel lane really meets a street: the asphalt runs straight past the mouth, not
  branching off, and the dirt enters its edge as it is, without asphalt flares or
  bevels. The asphalt fillets there read as a paved apron leaving the street for the
  dirt road (Kaluga 07, Новаторский переулок into Новаторская). What such a pair still
  closes — a sharp fork's nose, an outer corner — goes to the unpaved layer, under the
  asphalt. **Asphalt split into two ways where the dirt crosses it** (the only two paved
  arms of the node, nearly collinear, both ends) butts on both sides like any junction
  arm, and with no fillet to the dirt the gap on the outside of its kink stayed open: a
  thin line of gravel across the asphalt (Kaluga 06, Молодёжная, where the way with
  `lane_markings=no` begins on the gravel crossing). The two asphalt arms get the
  asphalt outer corner between them (`outer_corner`, counted in `outer[0]`); pinned by
  `corners.rs::asphalt_kinked_across_a_dirt_road_leaves_no_gap_at_the_seam`.
  **Asphalt continued by dirt** (two nearly collinear ends of one surface each,
  nothing else of their class at the node) ends square on both sides: the round cap of
  the asphalt lay on the gravel as a half-disc; the gap on the outside of a slight kink
  is an outer corner in dirt. For that the seam must stay on the node: the street axis
  passes a free seam as one arc and cuts the ways a few millimetres off it, where
  `kerb_returns` sees no node and the caps stayed round — so a seam where the surface
  changes is **pinned** (`axis.rs::Run::stitch`, `resurfaced`), the same as a seam
  another street joins. Pinned by `a_dirt_road_enters_the_asphalt_without_kerb_returns`
  and `asphalt_turning_into_dirt_ends_square`; gallery `kaluga/07_gravel_tee`.
  **An asphalt street ending at a
  dirt road** — the only paved arm of its node, the node's other two or more arms dirt —
  stops at the dirt road's kerb: `KerbReturns::setback` gives that end the widest dirt
  arm's half width, and `mesh_roads` cuts the fill back by it (`tapers::cut`; not on an
  end under a taper). Carried to the node like every ribbon, its butt lay over the gravel
  as a square tongue up to the dirt road's axis, with a step on each side where it was
  wider than the kerb returns (scout R2, Tula 13); now the asphalt ribbon ends flush with
  the dirt edge, square, with no kerb returns (the rule above). An asphalt road crossing the
  dirt one (two paved arms) runs through as before
  (`an_asphalt_street_stops_at_the_edge_of_the_dirt_road_it_meets`). The sidewalk is already gone by
  the parse (`untagged_sidewalks`). An untagged street stays asphalt — the tag is rare
  (Tula has a handful), and a guessed dirt road in a city block would be a worse lie
  than a missed one. Pinned by `roads/tests.rs::an_unpaved_street_draws_in_its_own_layer_without_lines`.
  A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `mesh_roads` `continue`s into `Bridges::push_deck`
  + `Bridges::fills` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`. It would hang a metre or three past the deck edge over the
  water, and the deck already has its own kerb: `push_bridge_curb`, drawn unconditionally.
  The road fill went from osm-carto white to asphalt grey together with the
  markings: a white line on white is invisible, and on grey the street grid also stops
  merging with the courtyards. At a junction the band turns the corner on the kerb's own
  arc — **The drawn network → Kerb returns → The sidewalk turns with the kerb** below.
  **A half of a divided street has no band on its paired side** (`Pairs::band_pieces`
  cuts it, `roads::push_sidewalk` lays the pieces; **Paired halves** below): along a pair
  run the band is the width plus one sidewalk, shifted half a sidewalk away from the
  partner, and the full band resumes past the run with a butt joint; a piece under
  `pairs::SIDEWALK_PIECE_MIN` 0.5 m between two runs is skipped (centimetre offcuts), and
  so is a piece left with no side at all. `None` from `band_pieces` is the whole band on
  both sides, laid uncut. On a half with a taper the runs are shifted by the head
  wedge's length, since the band is laid along the body past it (`Pairs::unpaired_pieces`,
  which left the paired side on, is gone).
  **`sidewalk=*` picks the sides** (stage 7): `RoadLine::sidewalks` `[left, right]` along
  the points (`parse/tags.rs::tagged_sidewalks` — `both|left|right|no|none|separate`,
  refined by `sidewalk:both|left|right`; `no` and `separate` mean no band, a separate
  footway draws itself; `oneway=-1` swaps them with the points). Each side is a
  **`SidewalkSide`** (`osm/model.rs`): `Tagged` from the tag, `Inferred` without one,
  `None` where there is no band — whatever took it. **Untagged** (no
  `sidewalk*` key at all — `tagged_sidewalks` returns `None`) is decided by the street and
  what stands around it, not taken as "both": an unpaved `surface`
  (`gravel|unpaved|ground|dirt|compacted|…`, `tags.rs::untagged_sidewalks`) never has
  one; a residential / unclassified / living street keeps both only where the mean
  **storeys of the blocks** along it (`cars::district::Districts::storeys_at`, the very
  measure that thins the parked row, probed every 40 m) are at least
  `SIDEWALK_STOREYS_MIN` 3 — the private sector and an empty field get a kerb with no
  band (`parse.rs::infer_sidewalks`, right after the drowned buildings, so the house
  pull and the block pull already read the decision); trunk…tertiary and the links keep
  both. The trigger was the Yandex comparison: in Tula silence means "yes" in the centre
  and "no" among private houses (galleries 09, 13, the side streets of 03 and 19), and
  the band there drew the rule zebras after it. Tula: see the `osm parse: N of M
  untagged residential streets left without sidewalks` line. And any carriageway —
  arterials included — gives an untagged side up to a **separately mapped paved footway**
  running alongside it (`parse/verges.rs::measure_footways_beside_streets`, `parse.md`,
  **Sidewalks left to a separate footway**): the band beside it was a second sidewalk —
  but only behind a real lawn (`SEPARATE_LAWN` 1.5 m to the footway's near edge); a
  footway at the kerb keeps the band under it. **Any side** of a paved street with such a
  footway alongside — up to `VERGE_REACH` 10 m past the kerb (16 m on a two-way street),
  band or no band, dropped or `sidewalk=separate` — gets a **verge** (`RoadLine::verges`,
  metres from the kerb to the footway's axis, the median over the probes, and its
  profile by place, `RoadLine::verge_at`), drawn by `roads.rs::push_verges` as a
  one-sided strip in sidewalk tile, from the axis past the kerb — with a profile a
  polygon of varying width on the path densified every `VERGE_STEP` 2.5 m, carried
  `VERGE_END_OVERLAP` 0.3 m past each end along the tangent so the verges of two ways of
  one street overlap at their seam (a disc of the end's width was meant to do that and
  never drew: it was a ribbon over a 1 cm stub, and `merge_ribbon_points` merged the stub
  into a point — a hairline across the verge at every such seam, Oryol 04 north); without
  one the old ribbon — round-capped, in its own
  layer **`road_verges` at `Z_ROAD_VERGE` 0.1 — under the landuse blocks and every
  green**: a lawn mapped between the kerb and the footway stays a lawn, and bare ground
  there — a hole down to the earth framed by the kerb returns at every corner of Tula's
  centre (scout R3, galleries 15, 21; Kaluga 03 and Oryol 03 too) — is paved. **The
  corner** comes from the kerb returns (`corners.rs`, `KerbReturns::verges`): between two
  sidewalk-carrying arms where at least one side has a verge, a fillet by the verge at
  the node (`verge_at` there; the band where there is none) with the arc at the kerb radius less the **narrower** of the
  two — by the wider, as the band corners do, it left a wedge of ground along the
  narrower. **A wide verge is a lawn** (`paved_verge`): tile covers a verge up to
  `VERGE_PAVED_MAX` 4 m whole; wider, only a `VERGE_KERB` 0.5 m strip at the kerb —
  **a step, not a ramp**. The tile used to narrow over 2 m of extra width, and where a
  footway leaves the street slowly its edge ran obliquely across the whole verge: a
  thin slanted wedge of lawn between the tile and the footway, and teeth where the
  profile wavered around 4 m (Oryol 03, roads tails L7). Along a verge by place
  `roads.rs::verge_runs` decides lawn or tile per vertex, folds a run shorter than
  `VERGE_RUN_MIN` 10 m between runs of the other kind into its neighbours (the end runs
  stay — the node's corner is there), and at each change inserts a vertex pair
  `VERGE_SEAM` 0.05 m either side of where the verge passes 4 m, so the tile ends in a
  seam across the street. Pinned by `a_verge_turns_from_tiles_to_lawn_across_the_street`.
  The whole verge — body
  and end discs where the width is past 4 m — goes as grass under the tile, and **which
  grass is the neighbour's**: by default the yard's muted grass (`RESIDENTIAL_COLOR`,
  `SurfaceKind::Yard`, **`road_verge_yards` at `Z_ROAD_VERGE_YARD` 0.09**) — beside a
  residential block the noise is by world position, so there is no seam at the block's
  edge; a meadow (`GRASS_COLOR`, `SurfaceKind::Grass`, **`road_verge_lawns` at
  `Z_ROAD_VERGE_LAWN` 0.08**) only beside a **mapped lawn or park** (`Meadows::beside` —
  `MapData::parks` or `MapData::grass` under the verge's middle or 3 m past it, probed
  at a quarter, half and three quarters of the way), like the lawns it continues. One
  colour for both read wrong both ways: the meadow lay as a bright ribbon along every
  street of the district frame d2, the yard green lay heavy beside the meadows of Tula
  15's square (roads plan №42). Asking for the yard was not enough either — the verge
  in front of a square, a lot or bare ground stayed a light-green ribbon on the district
  (the east side of Фрунзе, d2; roads tails L1), hence the default is the yard grass
  and the meadow is the exception. A corner between two verges both wider than 4 m
  goes there too (`KerbReturns::verge_lawns`, `Meadows::under` — any vertex or the
  centre on a mapped lawn). **A corner with a lawn behind it
  gets a kerb pad** (`corners.rs::kerb_pad`): tile `KERB_PAD_WIDTH` 3 m deep along the
  road fillet's own arc (same centre, `FilletArc`), running on `KERB_PAD_RUN` 4 m along
  each straight kerb, into `KerbReturns::verges` — the zebras land at the corner, and
  without it they ended on grass, a lawn sickle between two lawns (Tula 01).
  In a microdistrict (Фрунзе in Tula, the district frame d2)
  the tile laid from the kerb to a footway fifteen metres off made the street read as
  poured concrete; there it is a lawn with the footway on it, as in any Soviet yard,
  while the narrow paved verges of the centre's corners stay tile, and nothing between
  the kerb and the footway is bare ground either way (roads plan S4). **A ring arc takes
  its verge on the outer side only** (`Ring::ccw` says which: the island is on the left of
  a counter-clockwise ring) — inside is the island and its lawn, and without it a
  parallelogram of bare ground lay between the ring's sidewalk and a footway along it
  (Kaluga 01, roads plan S3); the arc's end discs lie under its own asphalt.
  Render-only: the navmesh and the house pull never read it.
  **One profile for the parse and the renderer**: `RoadLine::sidewalk()` →
  `SidewalkProfile` (`osm/model.rs`) — the sides as bools plus the **band by class**
  (`sidewalk_band`, 22 % of the width, 1.2–3 m; `Some` on a carriageway, a bridge
  included, `None` on a drive or a path). It is **derived, never stored**: the driveway
  crossings and the ring arcs are clones of a way with another `width`, and a stored band
  would go stale on them. It answers `band()` (by class, tag ignored), `on(side)`,
  `any()` (at least one side — `sidewalk=no|separate` on both gives `None`), `both()`,
  `sides()`, and the three edges the parse reads — **mapped edge**, **verge edge**,
  **kerb edge** (`references/parse.md`, **Houses pulled off the sidewalks** and **Blocks
  pulled to the roads**). Every edge is one reach per road on both sides, even for a
  one-sided `sidewalk=right`; a per-side edge would change the map, so it is not here.
  The renderer's sidewalk is that profile under the Sidewalks toggle (`Drawn::sidewalk_drawn`,
  and per side `sidewalk_on` / `band_half` through `sides()` / `on(side)`).
  The drawn sidewalk is `None` when neither side has one; `push_sidewalk` lays a one-sided
  band the paired-half way (width plus one sidewalk, shifted half a sidewalk to its side)
  and ANDs the tag with the pair runs; the kerb returns drop the arc on a missing side;
  a taper's sidewalk wedge is laid per side, and a side without a sidewalk is the bare
  half (**Streets, sections, tapers** — a one-sided street gets it too). Tula: 44 `no`,
  42 `separate`, 30 `right`, 10 `left`. The parse's house pull still keeps its clearance
  on a side the tag took the sidewalk from — a verge instead of a sidewalk there.
- **Kerb pockets** (`map/roads/pockets.rs`, stage 7) — where parked cars stand along a
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
  past the junction edge, the author's report of a bay cut off flat at a zebra, 3284
  2806), `POCKET_TAPER` 6 m slanted ends where a pocket stops inside the way, at least
  `POCKET_MIN` 10 m at full width. **A pocket across a way end is one pocket**:
  `pockets::all_kerbsides` (the one door for the ribbon and the cars) runs
  `join_way_ends` over the per-way answers — an end open into the way's end stays open
  only if the next way has an open piece on the same kerb there (the side sign flips on
  a way drawn the other way round), otherwise it gets its taper. **The next way is
  `RoadNodes::next_way`** (`roads/network/mod.rs`), the one answer to "which way goes on
  past this end": the most collinear pair in the node within `MAX_BEND` 50°, same class
  and flow as `RoadNetwork` glues streets, else a continuation across streets
  (`RoadNetwork::continuations`) — computed from the roads alone, so the car gallery,
  which has no network, gets it too. Both the pocket join and the zebra spill used to
  take *any* way ending in the node: on a tee that was the side way when it came first by
  index (pinned `a_kerb_pocket_stops_at_a_t_junction_side_way`,
  `a_zebra_at_a_way_end_spills_only_onto_the_continuation`), and a pocket ran round a
  right-angle seam (`a_kerb_pocket_does_not_turn_a_right_angle_way_end`). No city moved:
  a tee of three row participants is a junction break that closes the pockets anyway, and
  on Tula, Berlin, Kaluga and Ryazan no vertex of the road or car layers changed. The
  pieces linked by open
  ends form a chain and `POCKET_MIN` is measured on the chain, not per way; a piece whose
  tapers outgrow it goes (a taper is not carried across a way end), and the pass repeats
  until nothing changes. Per way, a short open piece was dropped while its neighbour
  stayed open into nothing — a square step (ул. Дзержинского, 5994 3185: a 28 m way from
  a crossing and a 23 m one to a driveway). The rule bays still see only pieces of
  `POCKET_MIN` and more, so their RNG stream did not move. That full run is what a **tagged** side (`street_side`) gets. A
  side that is a pocket **by the rule** gets rare short bays out of it
  (`pockets::sparse_pockets`): a block run carries bays at all with
  `RULE_BLOCK_SHARE` 0.4, each bay `RULE_POCKET_LENGTH` 24–42 m with both tapers, bays
  `RULE_POCKET_GAP` 30–90 m apart, the first up to 30 m in. The RNG is `seed::Lcg`
  seeded by `seed_from_point` of the street's first point, salted by the side, so the
  ribbon and the cars get the same bays and a rebuild moves nothing. The full-block rule
  pocket read as an extra lane wherever no cars stood in it (the gallery has none),
  and real bays are a few cars long; OSM's own bays arrive as tags or as separate
  `amenity=parking` + `parking=street_side` outlines (Berlin 3275, Tula 56), which
  reach the parking layer (`references/parking.md`) and are paved up to the kerb, cut
  into the sidewalk (`parse.md`, **kerbside lot** in `pave_lots`).
  **No pocket beside a parking lot** (`pockets::KerbLots`, over `MapData::parking`): the
  kerb is probed every `LOT_STEP` 2 m at the pocket's outer edge, and a probe inside a
  lot or within `LOT_REACH` 6 m of its outline (a sidewalk and a verge) marks that
  stretch as the lot's frontage — the cars go to the lot, and a bay in front of it read
  as a spare lane (the author's report, Советская by the theatre, 6040 2900: the lot 4 m
  past the bay's edge). A rule bay touching a frontage is **dropped after** the bays
  are laid, so the RNG stream and every other bay of the block stay put; a tagged run
  is **cut** around the frontage like around a row break, its tapers outside it.
  **Any lot kind makes a frontage, not only the kerbside lot of that side.** Resolving
  the two OSM records of one lay-by — the road's `parking:<side>=street_side` and a
  `LotKind::Kerbside` outline — onto the road side at parse, and letting only that lot
  silence the pocket, was considered and rejected: the rule exists
  for the author's report above, and that lot is `parking=surface` (way 479605523), a
  `Yard`; narrowing it to kerbside lots would put the bay back in front of the theatre.
  The two records of a lay-by are already reconciled by this same frontage — the kerbside
  lot is paved to the kerb and silences the pocket along itself — so a parse-side link
  would have added a second answer to a question this one already settles
  (`a_rule_pocket_is_not_laid_beside_a_parking_lot` pins it on a yard).
  Drawn by `mesh_roads` as three polygons per pocket from
  `pockets::outline` (inner edge 5 cm under the carriageway edge, outer edge between the
  tapers): asphalt `POCKET_WIDTH` 2.5 m wide in `roads` (no casing — the road casing
  layers are gone, stage 8) and the **sidewalk pushed out behind it** in `sidewalks` (the band is at most 3 m, a
  2.5 m pocket would eat it). A car on a pocket side stands at `half + POCKET_WIDTH` from
  the axis where its arclength falls in a pocket's full part, nowhere else unless the
  side also parks on the lane. Tula: 610 pockets; `kerb pockets N` in the report.
- **Turning circles** — a `RoadNodeKind::TurningCircle` on the free end of a street
  (not shared, not a bridge or an arch) gets a disc of `turning_radius` — half the width
  × 2.2, 6–10 m — in `roads` and a sidewalk ring of the road's own
  sidewalk. An untagged dead end ends in the round cap of the ribbon anyway
  (`ROAD_JOIN` is `RoadJoin::Round`). Tula's cache has none; `turning circles N` in the report, pinned
  by `a_turning_circle_widens_the_dead_end`.
- **Paired halves** (`map/roads/network/pairs.rs`, drawing in `map/roads/medians.rs`) — a
  divided street is two opposite one-way ways side by side, and each used to be drawn as
  a street of its own: its own sidewalk on both sides, a hairline of sidewalk under a
  half-metre gap, two pavements down the middle of a lawn, and where the mapper drew the
  halves closer than their width, one ribbon over the other with its lane lines through
  the neighbour's (Красноармейский проспект: 3 + 3 lanes with the axes 9.5 m apart, not
  11). Measured on Tula before this: 101 medians, 45 of them up to 3 m (22 overlapping),
  and the gap wandering within a pair by 1–2 m (up to 8 on a few).
  - **Finding** (`Pairs::new`, on the drawn axes of `axis::street_axes`, so once for the
    roads and once for the cars) — from every `PROBE_STEP` 2 m of a `pairable` road (a
    one-way `Street`-class way: a street **or a `service` drive** — the «Макси» boulevard
    is two service drives — never a parking aisle, a ring, a bridge or an arch) the
    nearest other one of the **same `Highway`** running **against** it (`PAIR_PARALLEL`
    0.9) and beside it (`PAIR_SKEW` 0.35) with `-PAIR_OVERLAP` 3.3 … `PAIR_MAX_GAP` 15 m
    between the kerbs. The overlap is a lane, not the lot's old 1 m: opposite halves never
    merge like lanes do, so an overlap is sloppy mapping and the alignment fixes it. The
    distance is to the **foot on the neighbour segment's line**, allowed `END_OVERHANG`
    3 m past its end: to the clamped closest point, a probe near the neighbour's end saw
    that end shifted along the axis, lost the pair metres before every node and seam, and
    the double solid stopped short of the junction (the author's report, sample 2).
    Consecutive probes with one partner are a **run** (`PairRun`, `PAIR_MIN` 8 m), and a
    run's gap is its median. **The partner holds from probe to probe** while it is within
    `PARTNER_SLACK` 0.5 m of the nearest: at a seam of the opposite half its two ways
    stand end to end, and inside `END_OVERHANG` the nearest flipped between them every
    other probe, cutting the pair into pieces under `PAIR_MIN` (roads plan №32). **A short
    way can be a half too** (`RunKind::Short`): a run under `PAIR_MIN` but at least half
    of it and `PAIR_COVER` 0.75 of the way's own length is taken in a second pass if the
    way shares an end with a half that found a pair — a street cut into ten-metre ways at
    a bridge; two slips meeting are no continuation of a pair and stay unpaired. Before,
    Первомайский in Ryazan (gallery 03) had a 9 m and a 13 m way between the lawn and the
    bridge left unpaired, and their inner sidewalks tiled the whole median with a square
    of bare ground in its corner. One `Median` per pair of runs, from the half with the lower
    index — the old lot code computed it from both sides first and got two double lines
    a few centimetres apart.
  - **Queries** — what a half differs by is asked of `Pairs`, not read off its runs in
    each consumer: `beside(road, at, slack)` — is a run there and is the partner on the
    left (the kerb returns pass two probes of slack, a run ending where the probes stopped
    finding the partner; the sidewalk wedge of a half, bare on the partner's side, takes
    none, the middle of a wedge lying inside the run); `partners(road)` as `Partner { road, paved }` (the
    junction paint's zebra plank); `is_paired(half, other)` (the merges, which widen it to
    the streets); `across_median(road, path, nodes)` — the cross-street piece in the
    median's opening (**Kerb return** below); `band_pieces(road, sides, stitch, total)` —
    the sidewalk band cut into pieces without the pair side (**Sidewalks** above);
    `has_runs(road)` — is the road a half anywhere (the bridge decks the alignment
    carries along, `roads/axis.rs`); `medians()`. `Drawn` answers none of them
    itself: it hands out `pairs()`, one owner. **The fields are closed** — `PairRun`,
    `Median` and `Pairs` keep them `pub(super)`, so only `network/*` (`align`, the pair
    tests) reads them. A median is read through `roads()`, `gap()`, `width()`,
    `midline()`, `inner()`, `is_paved()`, `carries_tram()`, and the one change it takes
    from outside is `Median::extend(end, along)` — the midline and both inner edges
    lengthened at one end, each along its own last link, which `medians::reach_breaks`
    uses to carry a median to the junction (the tip and its heading are
    `along::tip_of`, shared with the gores). Tests build runs with
    `PairRun::for_test`, `Pairs::of_runs` and `Drawn::with_pairs` (`Pairs::set_runs`).
  - **One door** (`medians::draw`) — every median of every pair goes through one call
    from `mesh_roads`, and every other function of `medians.rs` is private to it: the
    base breaks it opens on, the reach to the junction, the paved strip or the tram bed
    in the streets layer, the lawn (kerb in the sidewalks, grass in `road_medians`, the
    rest between the kerbs as asphalt), the zebras cut through as passages, the second
    pass that carries each double solid up to the nose of its pair's lawn and bridges its
    stubs. What it needs from neighbours comes in `MedianInputs` — the median base, the
    paint breaks and the zebras (`Junctions`), the half widths, the markings knob, and
    three closures (`Merges::is_pure_node`, `Gores::reach`, `RoadNetwork::street_of`) —
    so `medians` pulls in neither `gores` nor `merges`' logic. It **paints nothing**: the
    double solids come back in `MedianDrawing::painted`, ready (midline + breaks), and
    `mesh_roads` hands them to `Painter::paint_median` — otherwise `medians` would drag
    in `paint.rs`. The rest of `MedianDrawing` is what the neighbours read further down
    `mesh_roads`: `paved` (the tram band, the big lot's edge), `lawn_kerbs` (the merge
    nose, `merges::nose_fill`), `ends` (the merge axis meeting a median,
    `merges::merge_axis`) and `bed_caps()` (the asphalt past a tram bed's end). The push
    order into each builder is the old loop's, pinned by
    `tests.rs::the_median_loop_lays_the_same_vertices`; `draw` itself is tested on a bare
    avenue in `medians.rs` (`a_paved_median_hands_back_its_double_solid_instead_of_painting_it`,
    `a_lawn_median_hands_back_its_kerbs_and_nose_ends`).
  - **Alignment** (`Pairs::align`) — each half is densified to `ALIGN_STEP` 4 m and moved
    so that it stands at half the target distance from the midpoint between it and the
    partner's original axis: target gap = the run's median, paved ones no narrower than
    `PAVED_MIN_GAP` 0.5. The weight fades (smoothstep) over `ALIGN_TRANSITION` 20 m to a
    run's end — unless the end is a seam whose continuation carries a run at the same
    node. **A node shared with another road moves with the half**: `align` returns each
    such node as `(OSM point, drawn point)`, `Pairs::follow_moved_nodes` moves that
    vertex of every other road there to the same place (its neighbours follow, fading
    over `FOLLOW_FADE` 12 m, up to the next node), and `RoadNodes::alias` makes the new
    place find the same roads — kerb returns, breaks, stitches and rings look nodes up by
    a vertex of the drawn axis. Before, every node with a carriageway was pinned, and at
    each yard exit the axis fell back to OSM: on Красноармейский the distance between
    the axes dipped from 11.4 to 9.9–10.7 m every 50–100 m and the kerb waved by a metre
    and a half (the author's report after the seam fix). A node moved by two halves —
    a half's end and its continuation's start — is put at the **mean** of the two moves,
    so both drawn ends stay one vertex (apart, they missed each other by 5–8 cm and the
    node was lost). A **continuation** is a way of the same street *or* the coaxial way
    past the end (`RoadNodes::next_way`): at a crossing the network's street ends
    (Красноармейский is tertiary up to the node and primary past it) while the half goes
    on, and a fade on both sides narrowed the avenue at every crossing. **A bridge is a
    half like any** (`pairable` excludes only arches): unpaired, its axes stood where
    the mapper laid them, 9.5 m apart on Красноармейский over the canal, and the
    approaches converged on them. The navmesh carves the bridge by the OSM points, and
    the alignment moves the halves *outward*, so the walkable part stays inside the
    drawn deck. Such a bridge is also **smoothed with its street** (`axis.rs`, the
    `excluded` rule keeps out only unpaired bridges and arches), and a footway bridge
    mapped beside it follows it (`follow_bridge_sidewalks`: every vertex takes the shift
    of the nearest deck point): bridge decks lie over the streets, and the footway deck
    left at its OSM place put its tail over the moved approach as a pale tooth. At a node
    where two halves meet (the mean above), both ends are put on **one tangent**
    (`align_seam_ends`, a vertex `SEAM_TAIL` 0.5 m in along the bisector): the ribbons
    end square to their own axis, and a 2° kink between a bridge and its approach showed
    as a notch in the kerb. **A run is judged by its chain** in `Pairs::new`: pieces of
    probes that change partner with no gap count their length together (a piece under
    `PAIR_MIN / 2` inside a chain is a sliver at a seam and is dropped) — a 16 m way at a
    seam of the opposite half saw 7.4 m of each of its two ways, neither piece reached
    `PAIR_MIN`, and the half went unpaired into the bridge. **Pinned** is now only a node
    the other road cannot follow to: one with another aligned half, a ring or a bridge
    (a street or a drive — a footway pins nothing, as on **The street axis**). Next to
    such a node the axis is not moved at all for `PIN_STRAIGHT` 16 m and the fade begins beyond it: a
    kerb return is laid only on a straight edge, and a 10 m return to a crossing avenue
    needs its half width plus the tangent (stage 5 — before it the fade bent the edge from
    the node on, and the corners of sample 2 came out a metre or two). **Runs of one half
    closer than `RUN_BRIDGE` 12 m are one span** (`spans`): the partner changes at every
    seam of the *opposite* half, and a few probes at that seam find nothing (8 m holes on
    Красноармейский), so a fade at each run end let the axis fall back to OSM for ~40 m at
    every seam — on halves mapped 3.2 m over each other the carriageway "breathed" by
    2–3 m every 50–100 m (the author's report, Красноармейский and Советская). Inside a
    span the target gap passes from run to run over `ALIGN_TRANSITION` (`span_gap`), and a
    point near a partner seam measures against whichever of the partner ways is nearer.
    A continuation "carries a run at the same node" when that run reaches within
    `RUN_BRIDGE` of it (it was one probe step, and a seam of the own half is often a seam
    of the partner too, where the probes miss), and a span whose end is continued runs
    to the very end of the drawn axis without fading. The seam is looked up by the
    way's **OSM end point**, never by the end of the drawn axis: the smoothed axis is
    cut into ways at the point of the arc nearest the seam (**The street axis**), a few
    millimetres off the node, and `RoadNodes` found nothing there — every seam of the
    own half counted as a run end, and the axis fell back to OSM at each one
    (Красноармейский: 1.3–1.9 m dips every 35–80 m, the author's report after the
    `RUN_BRIDGE` fix). The pair tests that align raw points could not see it;
    `a_seam_of_the_own_half_on_a_smoothed_axis_does_not_let_the_axes_go` runs
    `street_axes` with the default curve.
    **At a seam with a taper the gap changes along the wedge, not across the node**
    (`seam_blend`, `SeamWedge`): the narrow way keeps its own gap up to the node, the
    wide one goes from the narrow one's gap at the node to its own at the end of the
    fitted wedge, linearly, like the wedge itself — so both kerbs of the wedge are
    straight lines. The `ALIGN_TRANSITION` blend centred on the node used to fall on the
    same seam as the wedge, and where a lawn median ends at a change of section the
    outer kerb first went in with the closing gap and then out with the wedge — a
    0.2 m dogleg (Tula, gallery 16: Советская 2 → 4 lanes, the lawn of 4.8 m becoming a
    0.8 m paved median; `a_gap_changing_at_a_taper_seam_changes_along_the_wedge_and_keeps_the_kerbs_straight`).
    A seam without a taper keeps the centred blend.
    **The distance is measured with the half widths that face each other, and on a
    taper that is the tapered one** (`facing_half`, fed by `street_axes` with a
    `Tapers::new` over the roads as parsed — the same joints `Drawn` finds later): a
    wedge that narrows the partner's side gives the half, at `d` metres from the seam,
    `narrow + (wide − narrow)·d/L` over the wedge length `L` (`tapers::fit`), both for
    the half being moved and for the partner at its nearest point, and the median's
    inner edges use the same halves. With the full width the wide way's axis stood
    half the width difference further from the middle than the narrow one's, so at a
    2 → 4 seam the two axes of one half missed each other by 1.4 m: a step on the outer
    kerb and a 1 m hole to the ground at the inner corner, between the two medians and
    the wedge (Tula 16, Советская, the author's report after stage №14). Now the axes
    meet (within ~0.3 m where the two ends measure against different partner ways) and
    the wedge's inner edge runs on straight: only the outer kerb narrows
    (`a_widening_seam_of_the_own_half_meets_and_keeps_the_inner_kerb_straight`).
    Then the path is thinned back by
    Douglas–Peucker at `SIMPLIFY_TOLERANCE` 3 cm keeping every shared node, and the
    median's midline and the two inner kerbs are sampled off the aligned axes and thinned
    the same way; the thinning is what took the stage from +130 k vertices and +50 ms
    down to +24 k and +18 ms. Ends of two medians closer than `pairs::JOIN_GAP` 5 m are
    drawn together (`join_ends`): a half of two ways is two runs, and the gap at the seam
    was a hole in the double line and a kerb island on the «Макси» boulevard. **At a pure
    seam of a half** (only its two ways among the streets at the node, drawn where the
    alignment put it) the ends join up to `SEAM_JOIN_GAP` 8 m apart, with the seam within
    8 m of their midpoint: a lawn measured along the partner half ends a few metres short
    of the own half's seam, and once the gap stopped closing across a taper seam (above)
    the lawn's tip and the paved median past the seam stood 5.07 m apart in sample 16 — a
    hole to the ground between them. `JOIN_GAP` itself stays: raised to 6 m, it joined two
    medians across the six-arm node of Oryol 04 and grew its asphalt
    (`median_ends_meet_further_apart_at_a_seam_of_a_half_than_elsewhere`). For the same
    reason a half's sidewalk is not drawn in a gap shorter than `pairs::PAIR_SIDE_REACH`
    12 m between two runs on the same side (`Pairs::band_pieces`) — whatever the runs are,
    paved, lawn or tram bed: their medians are drawn tip to tip, and the sidewalk lay
    between them as a pale patch (the seam of Советская's paved and lawn medians in
    sample 16 is 8 m, past the old `JOIN_GAP` 5 m) — **nor between the end run and the
    road's end** when that stretch is under the same 12 m: the probes lose the partner a
    few metres before the node where the halves converge, and both halves' paired-side
    sidewalks lay there as a pale wedge poking into the junction (Kaluga, Кирова ×
    Плеханова). A half with a taper takes the same pieces, shifted by the head wedge's
    length (the body starts past it), and its wedge lays no sidewalk on the paired side
    either (`Pairs::beside` at the wedge's middle).
  - **Paved median** (gap ≤ `RoadShape::median_gap`, 1–6 m, default 3; the flag is
    stored on `Median` at construction — `Pairs::new(roads, paths, median_gap, rails)` — and
    `Median::is_paved` reads it; the pair tests take the knob's default) — `push_paved` lays a ribbon down the
    midline as wide as the axes are apart into the `roads` layer **before** the halves
    (no lane frame, so no ruts; the halves lay theirs over it) **plus the contour between
    the inner kerbs** widened `FILL_OVERLAP` 2.5 m under each half (`between_edges`; it
    was 1 m, and where a half bends hard at a node the drawn ribbon rounds the bend while
    the median's kerb runs a chord, and a half-metre sidewalk spike showed between them —
    Вокзальная at Первомайский, Ryazan 03, roads plan №32). Each kerb is widened **away
    from the kerb across**, not away from the midline: the wider half's kerb crosses
    the midline, and pushed away from it that kerb went under the *narrower* half, the
    whole contour lay under the narrow ribbon and the gap between the kerbs showed the
    ground — a needle at 1 m of overlap, a 12 × 2 m plank at 2.5 (Oryol 05, Московская
    at 10.9 and 7.6 m; Ryazan 03 and Rostov 02 had thinner ones). The same contour is
    the lawn median's asphalt and the tram bed. The
    midline is measured between the *axes*, so between halves of different widths it
    lies near the narrower one's kerb and the ribbon fell short of the wider one's where
    the gap widens toward a lawn — a pale tongue along the double solid (sample 16). The
    paint layer draws a **double solid** down the midline (`Painter::paint_median`, the
    axes mesh). It is painted **after** every lawn is known: at a seam with a lawn of
    the same pair (`medians::reach_nose`) a terminal link under `SEAM_STUB` 1 m — the
    stub the two medians' shared seam point leaves, turned toward it, which curled the
    line into a hook — is dropped, and the line runs straight on up to `NOSE_REACH` 12 m
    until it meets that lawn's kerb, stopping `PAINT_NOSE_CLEARANCE` 0.5 m short. An end
    with no lawn ahead (a junction) is left as it was.
  - **Lawn** (wider) — the contour between the inner kerbs, opened by `NOSE_SHARE` 0.45 of
    the gap for a **rounded nose**, goes into the `sidewalks` layer (it shows as a
    `MEDIAN_KERB` 0.5 m kerb along each half), and shrunk by the kerb it is grass in
    `road_medians` (`Z_ROAD_MEDIAN` 1.7, `SurfaceKind::Grass`, the meadow colour); a lawn
    or kerb piece under `MIN_LAWN_AREA` 4 m² is not drawn, nor is a kerb piece with no
    drawn grass inside it (a pale stub on the junction field; asphalt lies there
    instead). Drawn
    whatever `RoadStyle::sidewalks` says: a lawn is still a lawn. **The pieces are cut
    on a densified midline** (`lawn_outlines` over `densified`: every link split into
    parts of at most `LAWN_STEP` 1 m, alike on the midline and both edges), a station
    counting while it is more than `NOSE_CLEARANCE` past every break **measured along
    the midline** (a break lies on a half's axis, off to the side, and a circle round
    it covered the midline a couple of metres short — the nose poked in between the
    two zebras; a break more than `BREAK_ASIDE` 12 m past the kerb is no break of this
    pair); the contour keeps only the piece's two ends and the original vertices. The
    breaks are the base ones **and the paint's** facing across both halves
    (`crossing_breaks` over `paint().of(half).cut`), **but a zebra does not end a lawn**:
    `split_zebras` takes every paint break a zebra passes through out of that list and
    turns it into a **crossing** — a break `ZEBRA_LENGTH` wide where the zebra meets the
    half's axis — and `push_lawn` cuts a **passage** (`passages`: a quad across the whole
    median, the zebra's length along the midline) out of the grass only. The kerb runs
    through it, so the pedestrian crosses the median on the island's paving between two
    pieces of lawn; a paint break with no zebra in it (a stop line at a node) still ends
    the lawn with a nose. A paint break round a zebra is half the zebra, the signal's stop
    line and the paint clearance — nine to eleven metres — and two `crossing:island=yes`
    zebras set six metres apart along the axis, plus the node's break, left no piece at
    all: Рязань 03's Вокзальная lost its whole lawn (roads plan S1, a regression of the
    densified cut). A piece shorter than `MIN_LAWN_RUN` 3 m is dropped (not `PAIR_MIN`:
    a five-metre island between a zebra and a node is a normal lawn), and the nose of a
    piece shorter than the gap is rounded by `NOSE_SHARE` of its **length**, not of the
    gap — the opening erases everything under two radii, and a 6 m piece of an 8.8 m
    median vanished whole. On the OSM vertices alone a
    straight avenue lost its lawn span by span: the first vertex past the junction lay
    inside the break and the next one sixty metres on, so the whole span was bare asphalt
    with no line between the halves (Kaluga 02, the east arm of Кирова, roads plan E2);
    and with both vertices outside a break in the middle of a link, the lawn ran right
    across the junction. **Everything between
    the inner kerbs that is not lawn is asphalt** (`push_lawn` → `uncovered`): the
    contour between the kerbs (`FILL_OVERLAP` under the halves) minus the drawn kerbs
    inflated by `CUT_MARGIN` 5 cm, into the `roads` layer before the halves. The
    opening erases whatever is narrower than two nose radii — at a run's end, where the
    alignment only spreads the gap to the lawn's width, or where the halves converge to
    a node, that is metres of wedge — and under it lay nothing: the half's sidewalk or
    the ground showed as a pale tongue behind the nose (Tula 16/24) and a pale wedge on
    the junction field (Kaluga 02). The margin matters: flush with the kerb the
    difference left hairlines of asphalt, which the roads layer (above the grass) drew
    as dashes along the kerb.
  - **Pocket at a median end** (`medians::end_caps`, any median but a tram bed, which has
    `bed_caps`) — a median ends where the pair's probes ran out, and when the gap ahead
    is closed by another carriageway — a U-turn or a link between the halves — rather than
    opened by a junction, the few metres between the median's end and that road were
    nobody's: the sidewalks of three roads, a scrap of verge lawn and bare ground, a pale
    four-sided stub of an island with a hairline crack of a way seam over it (Ryazan 03,
    Вокзальная under the link above Первомайский, roads tails L2). Now an end with **no
    break on its line** (the `reach_breaks` test, breaks behind the tip included — those
    ends are opened by a junction) whose ray meets, within `MEDIAN_EXTEND` 12 m, the axis
    of a carriageway from a node of either half near the tip (`MedianInputs::closer`,
    `closing_reach` — the first axis crossed, so it never paves past a cross street into
    the next median) gets a rectangle of asphalt from the inner kerbs, `CAP_WIDER` wider
    on each side — `FILL_OVERLAP` 2.5 m under the halves, like the fill between the
    kerbs; at 1 m the half's sidewalk band still lay under its ribbon there, and the
    hairline of its way seam showed over it — up to that axis, minus the median's own
    lawn kerb, into the `roads` layer before the ribbons. Rounding the stub into an island was
    the alternative and was not taken: the island is a few metres of kerb between three
    ribbons and reads as a crumb either way. Pinned by
    `a_pocket_between_the_median_end_and_a_closing_link_is_asphalt` and
    `the_closing_reach_is_the_first_axis_across_the_ray`.
  - **Tram bed** (`Median::carries_tram`, found in `Pairs::new(roads, paths,
    median_gap, rails)`) — a run whose gap is at most `TRAM_BED_MAX_GAP` 8 m and at
    least `TRAM_SHARE_MIN` half of whose probes have a `RailKind::Tram` link within
    half the gap (never less than `TRAM_REACH_MIN` 2 m) of the midpoint between the
    axes. The rails come through a `Grid` of tram links (`Tracks`), so the probes do not
    walk every track of the city. Wider than 8 m it is a reserved track on grass
    (Воздухофлотская, 3.2 km in Tula) and stays a lawn. A bed is **always paved**, the
    knob notwithstanding, and `PairRun::tram` / `paved` carry it to the zebras (one plank
    across both halves, like any paved pair). **Each bed run keeps its own gap** (its
    median, like any pair). A shared gap per chain of streets (`share_tram_gaps`, stage E)
    was tried and removed: Советская's chain median is 4.0 m while the halves on the
    stretch south of Коминтерна are mapped 6.6 m apart, so between junctions the halves
    were pulled 1.3 m each toward the middle and sprang back at every pinned node — the
    avenue narrowed and widened by 2.6 m. The step at a seam it was written against is
    now `span_gap`'s job (**Alignment** above).
    **Drawing** — each half is widened to the middle by its own inner lane, without
    marking: `push_bed` lays the asphalt from inner kerb to inner kerb (`FILL_OVERLAP`
    1 m under each ribbon, like the paved and lawn fills — 5 cm left the ground showing
    where a half's ribbon wobbles at a seam) **as a contour, not a ribbon**, so it follows the kerbs
    where the gap wanders and ends **square** — the round cap of the old ribbon lay over
    the nose of the lawn next to it (Коминтерна, where the tram turns off Советская and
    the median north of the node is grass again); the lawn beside a bed takes the bed's
    ends as breaks (`bed_ends`), so its nose stands `NOSE_CLEARANCE` short of them.
    Between the square end and the rounded nose nothing lay — the half's sidewalk showed
    through as a pale square (a half with a wedge keeps its paired-side sidewalk), so
    `bed_caps` carries the bed `BED_CAP` further on **minus the lawn's kerb contour**
    (`push_lawn` returns it): the grass (`Z_ROAD_MEDIAN` 1.7) lies *under* the streets
    (2.0), and a plain extension would have eaten the nose. An end with **no lawn
    kerb near** is carried on too, whole, `BED_CAP` plus the bed's width and `BED_WIDER`
    1 m wider than the kerbs on each side: the node's paint breaks had cut away the lawn
    that used to lie there, and a 2×2 m square of bare ground was left in front of the bed
    end in the middle of Орёл 04's six-arm tram node (roads plan S2). (The sidewalk in a short gap
    between two runs is the general rule of **Paired halves**, **Alignment** above.) No
    lane frame, so no ruts over the tram lane. The double solid runs **down the middle**,
    between the tracks (as 2GIS draws it). What makes the tram lane read is the
    **tram band** below, not paint. Stage-B history: the first version (`c53c6524`)
    drew the bed as dark asphalt with a double solid along **each edge** — the GOST
    reserved track — and read as «a dark corridor with no rails»; the author chose the
    Yandex picture instead (plan «Трамвайное полотно на Советской»).
    **Why the axes are not moved.** The plan asked for each half's axis to shift Δ/2
    towards the middle with the width grown by Δ, so the outer kerb stays put. That was
    prototyped on paper and dropped: everything that finds a node by a vertex of the
    drawn path — `corners::kerb_returns` (`is_shared` at 5 cm), the zebras' crossing
    marks, the ends of `node_paint` and the stitches — would have lost the nodes on
    every half of Советская, and pinning the axis at each cross street would have made
    the tram lane die and be born every 150–300 m. The contour between the inner kerbs
    is the same picture — outer kerb, sidewalk, cars and lane frame all untouched by
    construction — with the model and the drawn axis left alone. `RoadReport::drawn.medians`
    is `[paved, lawn, tram beds]` (`Pairs::count`).
  - **Tram band** (`roads/tram_band.rs`) — a lighter strip of asphalt (`TRAM_BAND_COLOR`,
    `ROAD_COLOR` lighter by about 8 %) `TRAM_BAND_WIDTH` 3.3 m wide along every tram
    track that lies **under a street's asphalt**: on a bed and on a single street with
    the tram down its axis alike (21.5 km of Tula), the way Yandex fills it. A track is
    probed every 2 m against a `Grid` of the drawn street links (no bridges, no arches)
    and of the paved medians; a probe is covered when a link runs **along** it
    (`ALONG_MIN` cos 0.8 — a tram crossing a street is a level crossing, not a lane),
    the foot falls on the link (±`LINK_SLACK` 1 m) and the strip fits inside that
    asphalt (`EDGE_SLACK` 0.6 m). An uncovered stretch between two covered probes of one
    track is **bridged** when it is under `BRIDGE_MAX` 40 m and every probe of it lies within
    `BRIDGE_SLACK` 4 m of some link's asphalt in any direction: tracks between the halves
    of a divided street cross a junction where the median has ended and each half's
    ribbon is 3 m short of them, and the band broke in mid-junction (gallery
    `27_tram_through_junction`, Советская × Красноармейский, the author's report). Open
    ground between two pieces of street is farther than that from any link and still cuts
    the band. Covered runs shorter than `MIN_RUN` 12 m are dropped.
    Laid into `roads` **after every ribbon and carriageway area**, butt-ended, without a
    lane frame; the paint is its own layer above, so the lane lines and the double solid
    lie on top of it. The pieces are laid as **one cover** (`band_cover`): each stroked
    `CLOSE_GAP` 3 m wider and half of it longer at each end, unioned (`i_overlay`,
    NonZero) and inset back by half the gap — a closing, so edges and ends stay where they
    were while any gap under 3 m between neighbouring bands fills. Laid piece by piece,
    the plain asphalt left between a turning track and the straight one, between two
    tracks' pieces cut at different probes, read as dark holes in the light band (sample
    5, the author's report); the closing cost nothing measurable (roads 172.1 → 172.7 ms,
    1.7 k fewer vertices). `RoadReport::tram_bands` counts the pieces. Only the
    streets whose box touches a cell of a tram link are indexed, and a covered run is
    thinned back by Douglas–Peucker at 5 cm — one vertex per 2 m probe cost 65 k
    vertices on Tula. **Cost** (`map_meshing`, Tula, dev): road meshing 171 → ~178 ms,
    881 → 887 k vertices — bands ~4 ms, bed caps ~2 ms, the rest is the twelve medians
    wider than 8 m that are lawns again (a lawn with a nose costs more than a paved
    ribbon). Tula: 107 paved + 32 lawn, 37 of them tram beds, 100 band pieces.
  - **Where it opens** — `crossing_breaks`: only a junction break of one half **facing** a
    break of the other (within the axes' distance plus both reaches; the axes stand
    `Median::width` — the gap between the **inner edges**, not between the axes — plus
    both halves' half-widths apart; the method was called `apart()` and the test read
    it as the axes' distance, so a narrow cross street between two three-lane halves
    did not open the median, and Московская across Пушкина's halves at a skew
    left the double solid running into the crossing, Oryol 05) — a crossing
    street, a U-turn link, a zebra's footway. A street into one half does not open the
    median: the far half runs past, and the double solid runs past with it (sample 12's
    note). At such a break the lawn stops `NOSE_CLEARANCE` 1 m short of it, and
    `reach_breaks` carries the midline and the kerbs on to the break centre (at most
    `MEDIAN_EXTEND` 12 m) so that the double solid dies at the break edge like the lane
    lines do, whatever the probes did. Toward a gore at a ring the line is trimmed and
    reached by `Gores::reach` (`MEDIAN_GORE_GAP` 0.6 m short of the hatching).
  - **Cars** need nothing: a one-way half parks one row on its driving-side kerb, i.e.
    away from the partner, and the row stands on the aligned axis. **Navmesh** does not
    see any of it — `RoadLine::points` never move.
  - Tula after this: 79 paved + 61 lawn medians; road meshing 90 → ~110 ms (pairs 5.5 ms,
    align 2.3 ms — both paid by the car layer's axes too — the medians 10 ms, mostly the
    lawn outlines); 753 k → 788 k vertices, paint 27 k → 34 k.
- **Streets, sections, tapers** (`map/roads/network/streets.rs`, `sections.rs`,
  `map/roads/tapers.rs`) — the first stage of the roads rework: the width of a street
  stops being a property of its class and becomes the consequence of its lanes.
  - **Streets** (`RoadNetwork`, kept in `MapData::network`). OSM cuts one street at every
    tag change, so the ways are glued back through their seams: at a node the ends of
    different ways pair up by **the most collinear pair of one `Highway` class** — pairs
    sorted by the bend, greedy, nothing sharper than `MAX_BEND` 50° — and a one-way pair
    only if the flow runs through the node (one way in, one out) and both are one-way. Only
    **ends** pair: a way passing a node is continuous there already. Rings (tag or shape)
    and closed ways are streets of one way — glued to an approach they would lead the
    street round the circle; paths (`Highway::Path`) are in no street. The direction of an
    end is a 10 m chord (`ARM_REACH`), since OSM's first link can be half a metre long.
    Nodes are walked in key order, so the gluing does not depend on the map's order.
    **Continuations** (`RoadNetwork::continuations`): the ends left unpaired are paired
    once more by the same rule without the class and one-way tests — a secondary going on
    as a residential, a two-way street becoming one-way. They make no street (a section is
    never inferred across a class), but the taper below is laid on them too. An end with a
    **twin** — another free one-way end in the node with the opposite flow, running within
    `MAX_BEND` of it — is skipped: those are the halves of a **merge**, whose own wedges
    draw the join.
  - **Sections** (`sections::apply`, **step 0 of `finish_parse`**). The **split** of a
    two-way way's lanes between the flows (`RoadLine::lanes_backward`: `lanes:backward`,
    else `lanes − lanes:forward`; none with a `lanes:both_ways` centre lane) is settled
    last (`settle_splits`): a split that does not fit the final count (a cut spike, more
    backward lanes than lanes) is dropped, and a way without one takes it from the
    nearest way of its street with the same count, mirrored for a way drawn against the
    street, so an odd street's axis does not jump half a lane at a seam. A way's lanes:
    the tag (`lanes`, else `lanes:forward` + `lanes:backward` + `lanes:both_ways`, the
    last one optional — one direction alone is not a sum),
    else the **nearest tagged way of its street** by the distance between their middles
    along it, else `default_lanes` by class (four on motorway/trunk/primary/secondary
    two-way, two on the rest — every `*_link` included, half of that one-way, one on a
    service drive; **a one-way `tertiary` keeps the two-way count**, and a ring — tag or
    shape — whose own way gives a radius of `WIDE_RING_RADIUS` 30 m or more gets at least
    `WIDE_RING_LANES` 2, `inferred_lanes`). The two exceptions are generated data, not
    read: a one-way tertiary is the carriageway of a two-way one driven in one direction
    (the grids of Ростов, Рязань, Калуга — two or three lanes), and one lane of 4.3 m read
    as a drive beside a sidewalk three times wider (Ростов 01, scout D1); a big ring is
    driven in two rows (Рязань, площадь Мичурина, r 52 m, tertiary, scout E17), while
    Рязань 05's 20 m park ring keeps its one lane, as on Yandex. The radius is read off
    the way alone — the perimeter of a closed one, the circle through the ends and the
    middle of an arc — since the rings are assembled only when drawn. Untagged one-way
    tertiaries per v15 cache: Rostov 127, Ryazan 65, Berlin 44, Kaluga 27, Oryol 16,
    Moscow 14, Tula 7 (some take a neighbour's tag first). Residential and unclassified
    one-ways stay at one: their houses stand closer, and a wider default would push them
    off the sidewalks for nothing. Then a **lone jump** — a run shorter
    than `SPIKE_MAX_LENGTH` 60 m with the same count on both sides and another of its own —
    is cut to its neighbours. `RoadLine::lanes` is **overwritten** with the result on every
    street and drive, and `width = lanes × lane width + 2 × EDGE_WIDTH` — a lane on a
    street is the **lane width** knob (`RoadShape::lane_width` 2.75–3.75 m, default 3.3,
    handed to `sections::apply(map, street_lane)` as an **argument** — the pass reads no
    global, and neither do `sections::lane_width` / `section_width`, which take it the same
    way), on a service drive that minus `SERVICE_LANE_NARROWING`
    0.3, 0.5 m of edge each side: at the default a
    two-lane street is 7.6 m, a six-lane avenue 20.8, a one-lane one-way half 4.3 — where
    the class gave 8, 16 and 16. Because the parse reads it, a new lane width is a **world
    reload** (**RoadStyle and RoadShape** below). **It is the one roads stage that moves the model**: the
    width is read by the passes after it (houses off the sidewalks, blocks and lots pulled
    to the roads), and then by bridge curbs, the navmesh's bridge corridors and the cars.
    Paths take theirs from the parse (`parse.md`, **Path width**). The gallery parses each cut window on its own, so
    a street there is inferred from the window's ways only.
  - **Tapers** (`tapers::Tapers`, built **once** per `mesh_roads` by `roads::Drawn` over
    the roads as drawn — the ribbon, the paint wedges and the kerb pockets' row breaks
    (`car_clearings(roads, &Tapers)`) all read that one value, and the car row reads it
    off its own `Drawn::nodal`; only the car gallery, which has no map, builds
    `Tapers::of_map` over bare roads) — where a way ends and another goes on
    from the node collinearly — a joint of one street or a continuation across streets
    (above) — and their widths differ by 0.1 m or more, the wider way's drawn path is **cut** at that end
    by `RoadShape::taper` (5–20, default 10; `Tapers::new(drawn, network, nodes, per_meter)`
    — the old `TAPER_PER_METER` is test-only) × the difference (at most `TAPER_MAX_SHARE` 45 % of its drawn
    length, since both ends may taper; under 1 m no taper). The cut end gets a **butt** cap
    (`push_ribbon_trimmed`, `push_street_fill`'s `trimmed`) — a round cap of the full width
    would bulge out of the taper — and the piece is laid by `MeshBuilder::push_taper`: a
    strip whose width runs linearly from the narrow way's to its own, joined by bisector
    vertices, with ribbon coords scaled to the local half width, so the lane lines fan out
    with the edges (proper lane geometry through a change of count is the paint stage's).
    **The two butt ends at the cut must meet exactly**, and two things used to break it,
    both in the merge of close points (`merge_close_points`, a quarter of the width): it
    kept the *first* of two close points, so a taper cut 0.34 m past a vertex ended on the
    vertex, short of the seam; and each piece merged its own short end link, so the taper
    ended square to one direction and the body started square to another. On a bend that
    left a wedge of pavement across the carriageway — Leipziger Straße in Berlin, 0 at one
    kerb to 0.6 m at the other (gallery sample `06_taper_on_a_bend`). Now a ribbon and a
    taper merge through `merge_ribbon_points` (an open path keeps its end point; the one
    before it goes instead), and a butt end is square to the **original** end link
    (`meshing::butt_normal`), which the cut puts on the same segment for both pieces;
    `break_profile` and `to_break_beyond` merge the same way, so the paint follows the
    ribbon's path. The polymesh still calls `merge_close_points` — its footprint must not
    move (`meshing/tests.rs::a_taper_meets_the_body_cut_just_past_a_vertex`).
    **The taper is per side** (`Taper::sides`, `[left, right]` along the way's points):
    at a pure seam both kerbs narrow; at a junction only the kerb with **no other
    carriageway arm on it** (`tapers::free_sides` — every `RoadClass::Street` road in the
    node, one arm per vertex it has there, so a crossing road covers both sides; a footway
    covers nothing, its layer hides no step). It used to be pure seams only, and a
    two-lane street ending at a T where a one-lane one went on showed a kerb tooth on the
    far side (Крестовоздвиженская площадь into Союзная, gallery 27). The side with the
    arm keeps its full width up to the node, as before: the joining road's asphalt and
    the kerb returns cover the step there — **but only an arm whose kerb radius is at
    least the step** (`corners::road_radius` of the arm and the wide road, the smaller,
    against half the width difference). A covered step still reads as a **tooth** when it
    is higher than the corner beside it: on either side of a 5 m drive (radius 2.5) the
    kerb of Болдина stood 3.3 m apart where 2 lanes became 4 (Tula, gallery 22); a
    street's 6 m or an avenue's 10 m corner swallows a lane's step. Such a side is free,
    and it narrows. A second test was tried and dropped — a slanting arm covers only the
    foot of the step's wall (`half / |cos|` of its angle to the axis): it freed the side at
    Orel's wide T (gallery 03), where the short way's wedge came out steeper than the step
    it replaced. A crossing covers both sides — no taper, the
    step sinks in the node. `Taper::kept` names the untouched kerb (`+1` left) for the
    paint: on a one-sided wedge the narrow section's lanes hug that kerb
    (`paint::kept_frame` — the body's grid with the far bound pulled in by the missing
    lanes; the symmetric wedge keeps `narrow_frame` and its drift), and the new lanes are
    born at the narrowing kerb with the kerb itself. The mesher lays it with
    `push_taper_sided` (half widths `[[left from, left to], [right …]]` in the wedge path's
    frame — `roads::wedge_halves` swaps the sides for a tail wedge, whose path runs against
    the way); `push_taper` is the symmetric case of it. **The kerb returns read the
    tapered end at the narrow width**: an `Arm` carries `half: [left, right]`, and at an
    end vertex with a taper the tapered sides take the narrow road's half width and
    sidewalk (`Drawn::taper_ends` hands `kerb_returns` the `Taper` itself) — the fillet
    then meets the wedge edge at the wedge's own 2–3° slant instead of floating a metre off
    it, which is what the pure-seam rule was protecting against.
    The same taper is laid in the **sidewalk** band, per side as well: the narrowing side
    from the narrow way's band (its bare half where it has no sidewalk) to this way's, the
    kept side at this way's band; a side without a sidewalk by tag is the bare half, so a
    one-sided street gets its wedge too (it used to get none). The half width per side is
    one answer, `Drawn::band_half(road, side)`, asked for the narrow way and for this one
    (**The drawn network** below). No taper
    on bridges or passages.
    **A half of a divided street keeps its partner-side kerb straight through a taper by
    its axis, not by extra asphalt**: the alignment (**Paired halves → Alignment**) sets
    the wedge's axis by its tapered half width, so the symmetric wedge narrows only on the
    outer side. It used to be the other way round — the median measured off the full
    width, the wedge narrowing toward it, the half's full sidewalk band showing in the gap
    (roads plan D3, gallery 16) — and a ribbon of half the wide width offset toward the
    partner was pushed under the wedge to cover it. That patch is gone with the cause: it
    left the 1.4 m miss between the two axes of the half at the seam (a hole at the
    median nose, a step on the outer kerb), and on a tail wedge, whose path runs against
    the way, it stood on the wrong side. The dark line beside the median on
    16 is **not** a seam: it is a real metal fence down the median (way 357798630,
    `barrier=fence` + an admin boundary, `height=1`) with its shadow — `tools/osm_near`
    does not list it because it skips boundaries.
    Drawing only: navmesh, cars and parse see each way's width as is.
  - **The network overlay** — `map/roads/network/overlay.rs::mesh_network_overlay`
    (re-exported `qwe::map::mesh_network_overlay`, z 29): every street in its own colour,
    the line thicker by the way's lanes, a white dot on every seam of a street. Shown by the
    game's Debug → Overlays **Road network** row (`DebugRoadNetwork`,
    `ui/debug/overlays.rs::sync_road_network_overlay`) and by the gallery's `Network` row
    (or `ROADS_NETWORK=1`; `examples/demos/roads/overlay.rs` keeps only the toggles).
  - **The OSM contours overlay** — `map/osm/contours.rs::mesh_osm_contours` (z 29.5): the
    geometry before the finishing passes over the drawn map (`references/parse.md`, the
    parse seam). Gallery row `Contours` / `ROADS_CONTOURS=1`, game Debug → Overlays
    **OSM contours**. **Every visual road report starts here**: the magenta axis and its
    node squares lying on the defect say "data", lying off it say "our parse or drawing".
  - **Raw OSM, drawing level** (`RawOsm::Draw`, `references/parse.md`; gallery
    `ROADS_RAW=draw`) — `mesh_roads` hands over to **`mesh_raw_roads`**, a separate
    build rather than switches inside the full one (its passes are interwoven, and half
    of them off would draw a picture neither the data nor the game has): every way one
    bare ribbon along its OSM axis at the parse's width (section width by a `lanes` tag,
    else the class width), miter joins and butt caps — no node gluing, kerb returns,
    mouths, pairs, medians, gores, tapers, sidewalk bands, verge lawns, turning circles,
    kerb pockets, big-lot kerb, tram band, paint or ruts (no lane frame). Kept: the
    layers by surface (asphalt, unpaved, sand paths, paved paths as sidewalk tiles), the
    `RoadArea` outlines as their own polygons, the fortress wall ribbon. A bridge is
    drawn in the street layer, without deck, curb or shadow. The same level empties the
    car placement (`cars::park_on`) and drops the stall and pitch markings in
    `spawn::mesh_surfaces`; lots and pitches stay as their polygons. Buildings are drawn
    as always (their outline is already the data's — the parse level skipped squaring and
    pulling). Tula, one debug run's `road meshing:` lines: 161 k vertices in 6 ms, against
    1.09 M in 315 ms for the full build (790 k in 264 ms on the parse level).
  - **A one-off window** — `ROADS_AT=x,y[,half]` (map metres, half 40 m by default)
    replaces the manifest column with a single window around that point
    (`samples.rs::window_from_env`), so a report's place can be shot before/after without
    adding a sample in breach of the manifest's criteria; `ROADS_SAMPLE=1` frames it.
- **Markings — the paint layer** (`map/roads/paint.rs`, shader `assets/shaders/paint.wgsl`).
  The lane lines are **geometry off the street axis**, not a pattern of the asphalt
  shader any more. Until stage 3 of the roads plan the asphalt shader drew them from the
  ribbon's own width — `round((across + half width) / lane width)`, lanes split evenly
  from the ribbon's centre — so on a taper every line drifted with the width, and the dash
  phase restarted at every seam of two ways. Now:
  - **One lane frame** (`meshing::LaneFrame`, built by `paint::lane_frame(lanes)`): the
    grid node `origin` (on the axis for an even lane count, half a lane width off it
    for an odd one) and the carriageway bounds `±lanes · lane / 2`, `lane` =
    `shape::lane_width()`. Lane boundaries are
    `origin + k · lane` strictly inside the bounds. The **asphalt fill gets the same frame**
    (`MeshBuilder::set_lanes`, `roads::road_lanes` — every carriageway, one lane included)
    and lays its ruts on it (**Asphalt wear**), so the ruts sit exactly between the lines.
  - **Taper**: the frame drifts from the narrow section's to the wide one's over the
    wedge (`MeshBuilder::set_lane_taper` for the asphalt, `paint::narrow_frame(...).lerp`
    for the lines). The grid node is picked so that on the path **from the seam to the
    body** it moves by `[0, lane)` to the left — so lines both sections share stay put
    (2 → 4), and a parity change (2 → 3) slides the grid by half a lane over the taper, the
    new lane born on the right of the taper's run. **A one-way wedge adds its lanes at
    one kerb** (`paint::wedge_drift`, roads plan D4): a two-way 2 → 4 gets a lane each
    side, which the symmetric grid is right for, but a one-way street gains or loses
    lanes at the kerb — an exit, a right-turn pocket (gallery 16, Советская 2 → 4 to the
    right-hand slip). There the grid node is shifted by the whole difference of half
    widths, so on the body the narrow section's lanes stand against the other edge and
    the new ones grow in at the kerb (`MapData::traffic_side`); a `turn:lanes` on the wide
    way that starts with a left-only lane and does not end with a right-only one is a
    left-turn pocket, and the new lanes go to the far side instead. The axes in OSM run
    straight through such a seam, so the shared lines slide over the wedge by that shift
    rather than stay put — the only way to keep the asphalt continuous. A line that the narrow section lacks
    grows in from the kerb: its alpha is the distance to the nearer bound over
    `BIRTH_FADE` (half a lane). The asphalt wedge runs seam → body, so for the tail wedge
    its frame is the mirror (`paint::wedge_frames`); `the_wedge_asphalt_and_the_wedge_paint_share_one_grid`
    pins that both land on one grid.
    **Between two two-way sections the axis leads** (`paint::seam_origin`, carried as
    `WedgeEnd::origin`, roads plan №30): the seam frame is the narrow neighbour's own
    frame (`lane_frame(narrow)`, whatever the wedge's shape) with its node placed so that
    the wide way's axis line lands on the neighbour's axis — in the wide way's frame,
    mirrored when the neighbour is drawn against it — and the lerp then carries the axis
    to its own place over the wedge. Without it a seam of different splits jumped:
    Текучёва in Rostov (gallery 03), `lanes=5, lanes:forward=3` into six lanes, has the
    five-lane axis on the flow border half a lane off the middle and the six-lane one in
    the middle, and the kept frame of that T-junction wedge put the whole grid half a
    lane off as well. The shift is snapped to whole half lanes (a 1e-7 tail made
    `Painter::paint`'s `ceil` add a line beyond the kerb) and taken only up to one lane;
    a one-way side, or a shift over a lane, leaves the frame to the wedge's shape rule.
    Pinned by `the_axis_runs_through_a_seam_of_different_splits`.
  - **Dashes by the street's arclength** (`paint::street_stations` over the network's
    ordered ways and the axis paths): 2 m dash, 6 m gap (ГОСТ 1.5 in town — the gap three
    times the dash; the old 3 / 3 read as a picket fence on a multi-lane street at the
    gallery zoom, Yandex draws about 1 : 2.5), and the phase runs through a seam.
  - **Solid near a junction**: the last `APPROACH` 25 m before a junction break — for a
    **lane line only on the approach**, in the direction its lanes flow
    (`paint::flows_forward` by the line's side of the axis and `MapData::traffic_side`, a
    one-way road forward): leaving a junction a lane line is dashed at once (the author's
    report — Первомайская, 3265 2802, carried a solid line on the exit side).
    `paint::approach_spans` finds the gap edges on the to-break profile (it is linear
    between vertices, so an edge is a zero on a link), `split_at_spans` puts a vertex at
    each span end, and the line goes out in pieces of `LineKind::Dashed` (10) and
    `LineKind::Solid` (11); the shader only draws what the kind says. The **axis** of an
    open street is split the same way into `AxisDashed` (12) / `AxisSolid` (13), but
    symmetrically — it separates two flows, so it is solid `APPROACH` on **both** sides of
    a break (`paint::near_spans`) — **and at a node the street passes through**: a
    leading road that does not yield (**Junction paint** below) gets no break there, only
    a `solid` zone in its `LineBreaks` (the reach its break would have had), and the axis is solid
    `APPROACH` either side of it (ГОСТ 1.1 at a side street; Yandex draws the
    Циолковского on gallery 19 so, while ours ran dashed straight past both side streets).
    `Painter::paint` takes both lists as one `paint::LineBreaks { cut, solid }` — handed out
    per road by `PaintBreaks::of` (**Junctions** below), never assembled by hand. A **ring's**
    lines and its closed axis keep the old rule (kind 0/1 — solid by to-break alone): a
    ring's entries are not worth splitting a closed strip for. The axis of a two-way street with 4+ lanes is a **double solid** (0.15 m gap
    — ГОСТ 1.3's 10–15 cm; half a metre read as two separate lines, the author's report —
    merging into one line once the gap is under ~2 px); a two-lane two-way street has a
    dashed axis; a one-way street and a one-lane street have none. **The axis is the
    border between the flows** (`paint::axis_offset`), not the middle of the
    carriageway: `RoadLine::lanes_backward` lanes run against the points, the rest along
    them on the traffic side's half, so the axis lies on a lane border at
    `forward · lane − half` (right-hand traffic; mirrored for left). Without the split
    the lanes divide in half and an odd count gives its extra lane to the flow along the
    points — a three-lane two-way street gets a dashed axis, a five-lane one a double
    solid. Before, the parity decided whether there was an axis at all, and Текучёва in
    Rostov (gallery 03, `lanes=5, lanes:forward=3`) was dashes only (roads plan D3).
    `lane_count` and the lane frame do not move: the carriageway stays centred on the
    way, only which of its borders is the axis changes. The stop line of a two-way arm
    runs from its kerb to that axis, and the lanes of an arm (`turns::arm_lanes`) split
    at it.
  - **Geometry**: one strip per line (`MeshBuilder::push_paint_strip`, miter joins),
    `LANE_STRIP` 0.6 m / `AXIS_STRIP` 1.4 m half-width — wider than the 0.15 m line so the
    1.3 px floor and the ±0.7 px antialiasing still fit at the farthest zoom where the line
    is drawn. `ATTRIBUTE_RIBBON` here is `[across from the line, street arclength,
    to-break, kind]`, the birth alpha rides the vertex colour. `to-break` comes from
    `meshing::break_profile` — the same `GapProfile` the asphalt ribbon uses, so the
    lines stop at the same junction gaps.
  - **Transverse paint** (**Junction paint** below): a stop line is a strip across the
    lanes in the lane-line mesh (kind 3, dashed 0.6 / 0.6 m for give-way — kind 4); a
    zebra is **one quad** in its own mesh (kind 5) whose bars (1 m period, half filled)
    the shader draws by the coordinate across the road and fades by `visible()` into a
    plain light plank — so a far zebra is a mean tone, not a flicker. For all three the
    ribbon's arclength runs across the road and «across» runs along it; to-break is a
    constant `NO_BREAK`.
  - **Layers**: `road_paint_wear_mask` + `road_paint_wear` at `Z_ROAD_WEAR_MASK` /
    `Z_ROAD_WEAR` (the turn paths' wear in two passes, under every line — **Turn paths**
    below), then `road_paint_zebras` + `road_paint_lanes` +
    `road_paint_axes` at `Z_ROAD_PAINT` (above every
    street fill, **under** a parking lot — a lot laid over the carriageway hides its lines
    as it did when the asphalt shader drew them), `road_paint_islands` at
    `Z_ROAD_ISLANDS` 2.0035 (the hatched gores and splitter islands — above a lot's
    asphalt and its double line; hidden with the zebras, `PaintTag::Zebras`; **Parking**
    in `parking.md`), and `bridge_paint_lanes` +
    `bridge_paint_axes` at `Z_BRIDGE_PAINT` (a street's paint under an overpass must not
    lie over the deck). Material `PaintMaterial` (blend), its handle in
    `SurfaceMaterials` next to the surface ones, `MaterialSpec::Paint`.
  - **LOD**: the shader fades lane lines and stop lines from 0.32 to `LANE_ZOOM_MAX`
    0.4 m/px, zebras to `ZEBRA_ZOOM_MAX` 0.6 (their bars are gone into the plank by
    ~0.25) and axes and the turn wear to `AXIS_ZOOM_MAX` 0.9; `PaintLods` (the same thresholds, four steps) hides the meshes by
    `Visibility` (`paint::show_paint`, `PaintTag` on the entity, set by
    `spawn_road_meshes` by the layer's name) — **no rebuild** at a threshold. The gallery
    does not run the ladder and relies on the shader fade.
  - **`RoadPaintStyle`** (group `road_paint`): `paint` 0–1 (0.7 — Yandex's lines are
    about 60 % white; 0.85 read heavier than the reference on 05 and 18) — the line opacity,
    `wear` 0–0.15 (0.075) — the rut amplitude, `turn_wear` 0–0.08 (0.035) — the turn
    paths' rut amplitude. All three uniforms
    (`surface::retune_surface_materials`), a knob drag rebuilds nothing; the Markings
    toggle of `RoadStyle` still decides whether the paint layer is built at all.
  - The report counts `paint N lines / M verts`. Tula at stage 3: see the roads plan's
    stage log.

  **Lane count** (`roads::lane_count`): `RoadLine::lanes`, which after the parse every street
  and drive carries (**Sections** below); only a road built by hand in a test falls back
  to the width default (two-way: a lane pair per 7 m; one-way: a lane per 4.5 m), never
  more than the width allows at `MIN_LANE_WIDTH` 2.5 m. **A roundabout is no exception
  any more** (roads plan, stage 6): the "always one lane on a ring" rule went once the
  junction paint could break a line at each entry and the closed ribbon stopped showing its
  seam (**Ribbon** below). A drawn ring takes one section for all its arcs — the widest
  arc's width and lane count (`roads::ring_arcs`; Tula's primary ring has arcs of 3 and
  2 lanes, and the ribbon would step) — see **Roundabouts** below.
  **`lane_markings=no`** (`RoadLine::lane_markings`, `parse/tags.rs::has_lane_markings`)
  overrides the count: `Painter::paint` draws neither the axis nor the lane lines of such
  a street, however many lanes it has (Tula, 11 ways; gallery 09, Бухоновский переулок,
  drew a dashed axis Yandex does not have). The junction paint — zebras, stop lines,
  arrows — does not read it; the ruts do not either (the lane frame stays).
  **Breaks** — «to-break» is the signed distance to the nearest **marking break**
  (`meshing::Break { at, reach }`, passed as `RibbonBreaks::At`): negative inside a gap,
  so a paint line is **cut sharp** at the gap edge (`smoothstep(±0.7 px)`, the same
  antialiasing as its sides — a one-metre fade read as a blurred end, the author's report)
  and the ruts fade over 5 m.
  Junctions (`junctions::marking_breaks`) are computed **always** now, markings on or
  off — the ruts need them too. The mesher projects each break's world point onto its own
  (smoothed, merged) path — that is why a break is a point, not an arclength: the smoothed
  centreline and the OSM node may disagree — merges overlapping gaps, and inserts a vertex
  at every kink of the distance function (each gap centre and the crossover between
  neighbours) so the GPU's linear interpolation is exact. The round cap extrapolates the
  last quad's slope: a listed end goes negative, an unlisted one keeps growing — that is
  what carries the line through a seam between two ways of one road. A ribbon with no
  breaks at all gets `along + FAR_FROM_BREAKS` (dashes need a growing coordinate). The
  `Square` join falls back to `push_polyline`, which knows only the ends — roads never take
  it any more (`ROAD_JOIN` is `Round`); only the tree-row band's own Joins row can.
- **Ribbon** — a constant-width band along a polyline (`MeshBuilder::push_ribbon`), how
  every road, alley and kremlin wall is drawn. The `roads::push_ribbon` **wrapper** over
  it exists only to map a `RoadJoin` (for roads the constant `ROAD_JOIN` = `Round` since
  stage 8; a user knob only on the tree-row band) onto the pair below, so the layers
  whose join is a constant — fences, rails, the tram — call `MeshBuilder::push_ribbon`
  themselves with `RibbonJoin::Round` / `RibbonCap::Round` and `closed: false`; going
  through the wrapper made the road's style read as theirs.
  - **A closed way is drawn as a closed ribbon**, and both road doors — the wrapper and
    `push_street_fill` — take the flag from the path itself (`is_ring`), not as an
    argument: a ring has no ends, and there is nothing but its own shape to decide that
    by. A closed ribbon gets **no caps at all** (the cap style then changes nothing, which
    is what `a_closed_ribbon_has_no_caps_and_joins_its_seam` pins) and its seam gets an
    ordinary join fan; `merge_close_points(closed)` drops the repeated last point, and
    `arclengths` closes the loop. The street fill goes through
    `MeshBuilder::push_ribbon_shaped` with a `RibbonShape` for this — `push_ribbon_broken`
    is gone, an eighth argument would have tripped clippy's `too_many_arguments`, and the
    shape was already the type the builder used inside.
  - **Why it matters, from the author's screenshot of the mall's ring**: drawn open, a
    ring laid **two round caps on its own asphalt** at the seam — an 8 m disc for an 8 m
    road, centred exactly on the way's first vertex — and inside a cap the ribbon frame is
    **frozen** at the end's direction (`FanCoords::Cap`), while outside it curves with the
    ring. So the lane dashes stepped sideways and the wear ruts changed direction along a
    crisp circle. Nothing was wrong with the caps: they are right at a loose end and
    harmless at a junction (the break blanks the markings and the wear there anyway) —
    the ring is the one place a cap lands on live asphalt of the same road.
  - **«До разрыва» on a closed ribbon is measured around the circle** (`GapProfile`'s
    `period`): the short way of the two, so the seam is no different from any other point
    at that distance. Three consequences worth knowing before touching it — a break that
    projects onto the **closing link** is found there too (`project_onto_path` takes the
    wrap segment), the kink where the nearest break changes wraps across the seam and, on
    a ring with a single break, sits at the **antipode**, and the vertex for a kink on the
    closing link is *appended* rather than inserted (`append_vertex_at`) because that link
    has no pair of neighbours to insert between. `RibbonBreaks::Ends` on a closed ribbon
    means **no breaks**, not two at the seam: a ring has no ends to fade at.
  Two knobs, both named after their SVG /
  Mapnik counterparts: **join** (`Miter` — bisector offsets capped by `MITER_LIMIT`;
  `Round` — an arc of radius half-width on the **outer** side of the bend, the side where
  butt-ended segment quads leave a gap) and **cap** (`Butt` — cut at the last point;
  `Round` — a half-disc half-a-width past it). Arc tessellation is driven by
  `ARC_TOLERANCE` (5 cm of chord sagitta), so a 16 m primary gets more chords than a
  3.5 m footway; the **same tolerance decides whether a join fan is emitted at all** —
  a bend is skipped only when `half_width · turn` is under it. An angle threshold was
  tried first and was wrong: 5° on an alley still leaves a 15 cm slit, plainly visible
  as a pale cut across the road when zoomed in. **A bend that gets no fan is joined by
  shared vertices** — both quads end on the bisector (`miter_offsets`) there. With each
  quad on its own normal the two ends meet only on the axis, and even at 0.4° the
  rasteriser left a hairline across the whole ribbon: a drive in Tula (way 2065, three
  almost collinear points by a parking lot, `cam 5300 3282`) showed two pale lines of the
  sidewalk under it, and the darker asphalt made them loud. The same holds for the
  vertices `GapProfile::split_path` inserts, which are collinear by construction.
- **Roundabouts** (`map/roads/rings.rs`, roads plan stage 6) — a ring is drawn as one
  smooth figure instead of its faceted OSM polyline, inside `axis::street_axes` (after the
  pairs are aligned), so the cars, the gallery and every layer see the same axis. Only with
  a curve tolerance above 0 (`RoadShape::curve_tolerance`); at 0 the ring keeps the OSM
  points.
  - **What a ring is.** Candidates are `RoadLine::is_roundabout` streets (tag or a closed
    one-way), not bridges, not arches. A closed way is a ring by itself; open arcs are
    chained end to start by node until they come back to the first (`chains`, at most
    `MAX_ARCS` 32 of them) — a chain that breaks off is left alone; a ring whose fitted
    semi-axis is under `MIN_RADIUS` 4 m keeps its OSM points. Tula: 12 tagged arcs make its two big rings (primary,
    six arcs, r ≈ 40 m; secondary, six arcs, a 150 × 115 m "egg"), plus seven closed
    one-ways (the «Макси» ring and the service rings).
  - **The figure** (`fit`): the loop sampled every metre, centre and axes by its moments,
    semi-axes by least squares in those axes; a circle when they differ by under 8 %. A
    loop is rejected (drawn as before) when an OSM vertex is further from the figure than
    `max(4 m, 35 % of the minor semi-axis)` — a long loop round a square is not a ring. The
    secondary egg is 14 m off its ellipse and still passes: the nodes hold its shape.
  - **Nodes stay put.** Every shared vertex of the ring (and every arc end) is a *pin*:
    the ellipse is scaled along the ray from the centre by a factor that is exactly one
    at each pin and a periodic Hermite spline between them (`Ring::scale`), so everything
    that finds a road by its node — kerb returns, breaks, junction paint, stitches — still
    finds it. The seam of a closed way nobody joins is not a pin: the axis runs on the
    figure there (pinning it swelled the whole circle to that one vertex). Each arc is
    sampled at 5 cm chord sagitta with its interior pins as exact vertices.
  - **Approaches enter by a tangent arc** (`bend_approach`): a one-way arm ending (or
    starting) at a ring node gets its last `0.5 × radius` metres (6–20 m, at most 60 % of
    the way, never past another shared node) replaced by the turn paths' Bézier
    (`turns::curve`), tangent to the arm and arriving at `ENTRY_ANGLE` 25° to the ring's
    travel — inward for an entry, outward for an exit. An arm already within 5° is left
    (its heading is taken over its last `HEADING_BASE` 3 m); the arc stops `PIN_MARGIN`
    1 m short of a node the arm shares with someone else, and an arc left shorter than
    `BEND_MIN_LENGTH` 4 m is not built. **A two-way arm** is bent the same way, into the
    node **along the ray** from the centre, but only when its axis arrives more than
    `TWO_WAY_BEND_MIN_ANGLE` 30° off that ray — i.e. when the mapper led it into the node
    along the ring. Its butt end is square to the last link, and square to an axis
    running along the ring that end lay across the kerb and stuck out of it as a step on
    both sides (Рязань 05, the author's report); the island rule reads the bent axis
    (**Splitters by rule** in `parking.md`). A radial arm is left as it is
    (`a_two_way_approach_along_the_ring_is_bent_into_it_across`).
    **A Y-approach is not two two-way arms** (`rings::y_legs`, `Rings::leg_flow`): two
    two-way ways, each at most `LEG_MAX` 55 m, that leave one node and end in two
    **different** nodes of one ring are its **legs** — an entry and an exit mapped without
    `oneway` (Рязань 05: every approach is such a «Y», legs of 15–25 m into nodes 20 m
    apart). The entry is the leg whose ring node lies **downstream** along the ring's
    travel (one who enters turns with the travel, so the legs never cross), and each leg
    is bent as a one-way arm of its flow, by the tangent arc. Bent along the ray as
    two-way arms, both swept the whole triangle between them with asphalt and a splitter
    stood on each and hooked onto the ring. A leg is also **drawn one lane wide**
    (`roads.rs::leg_sections`, a `Drawn` substitute like `ring_arcs`: `section_width` of
    one lane, `lanes` 1) — two 7.6 m two-way ribbons covered the whole wedge and left the
    island no room — and its wedge is the fan's (**The fan is taken whole** in
    `parking.md`). **The other leg may be a tail of other streets** (`rings::tail_walk`):
    Рязань 05, north — the «Старая дорога» runs on past the fork, and the end of the
    street coming from the north takes it into the second ring node, so only one short
    way leaves the fork. A lone such way (group of one in `y_legs`) is still a leg when
    a walk from its far node along approach streets (not itself, not ring arcs; at most
    one change of street; within `LEG_MAX`) reaches **another** node of the same ring
    within `TAIL_ARC` (a quarter of the ring) — the shortest walk decides entry or exit
    by the same downstream rule. The tail is **two streets**: the first runs **through**
    the fork (an inner vertex of it) and the second, started on it, ends in the ring. A
    street that goes into the ring by itself is a neighbouring approach, not a tail — on
    Рязань 04's big ring (60 m) two wide two-way approaches found such one-street
    "tails" 23 and 38 m along the ring, narrowed to a lane and opened a wedge of bare
    ground. The leg then goes one lane by the tangent arc; the tail
    is drawn as it is, being part of other streets
    (`the_legs_of_a_y_approach_enter_and_leave_along_the_ring`,
    `a_y_approach_whose_other_leg_is_a_tail_of_two_streets`).
  - **A tail Y is straightened in the data, not in the drawing**
    (`rings/straighten.rs::straighten_tails`, the parse's first finishing pass, before
    the sections — it moves nodes and cuts a way, and the street network is assembled
    from the ways). Drawn as found, it had no island: on Рязань 05 north the fork lies
    10 m off the ring's axis, the joint where the second street takes the tail over
    3.6 m, and from there the full-width two-way tail runs along the ring — one smooth
    asphalt flow from the fork to the ring. No path substitute can fix that, since the
    nodes stay put in `Drawn`. So the pass runs the same `fit_rings` + `y_legs` over the
    raw points (`y_legs` hands the tail Ys back as a second list, `Tail` / `TailWalk`)
    and, per tail: the **fork and the joint merge into one node** (`fork_point`) on the
    ray through the middle of the ring arc between the two leg nodes, `FORK_SHARE` 0.7
    of the chord between them off the axis (at least `FORK_MIN` 10 m; the two «Y» of two
    ways on the same ring stand 11 m off at a 20 m chord and their island barely
    shows); the first street loses the piece between fork and joint and ends in the new
    fork; the second street's run from the joint to the ring becomes **a way of its own**
    (cut off when the street goes on past the joint), straight; the own leg is
    straightened too; every road through the fork or the joint is moved with them.
    What reaches the drawing is an ordinary Y of two ways — one-lane entry and exit by
    the tangent arc and the fan's hatched island between them. Left as it is: a vertex
    that would go (between fork and joint, between joint and ring, inside the leg)
    shared with any road — it would be left hanging; a second street not ending in the
    ring node; a fork moving more than `SHIFT_MAX` 20 m, or a leg longer than
    `LEG_MAX`. Only the tail Ys are touched — a Y of two ways, a lone two-way approach,
    a short way with no tail keep their points
    (`a_y_tail_along_the_ring_becomes_two_legs_from_a_fork_off_it`,
    `other_approaches_are_not_straightened`, `a_tail_crossed_by_another_road_is_left_as_it_is`).
    Plausibility over faithfulness: the fork is no longer where the mapper put it
    (about 10 m further out), but the approach reads as an approach. Roads carve the
    navmesh only through bridges, passages and fence gaps, so the pass does not reach
    it in a park; the doors, blocks and lots see the same fork as the ribbon.
  - **Webs** (`Rings::webs`, `webs_along`): wherever a street — an arm, its continuation,
    or a slip road that bypasses the ring without entering it (Tula, gallery 04,
    south-east) — runs **along** the ring outside it (within `WEB_ALONG` cos 0.7 of the
    ring's tangent) with a gap under `WEB_GAP` 2.5 m between the kerbs, the strip between
    the two axes is asphalt, pushed under the ribbons — the sidewalk used to show through
    as a crescent. Every street way near a ring is walked at 1 m steps and each such
    stretch becomes one web, so a gap spanning two ways is two webs meeting at the shared
    node. A street meeting the ring across (a two-way arm into a node) is not along it and
    gets none: that corner is a kerb return's.
  - **One section, one kerb.** All arcs are drawn at the widest arc's width and lanes
    (`roads::ring_arcs`); the sidewalk is drawn once per ring as a closed ribbon, **outside
    only**, and the central island gets a `MEDIAN_KERB` 0.5 m kerb along the inner edge
    instead of a sidewalk ring (`push_ring_edges`). **The island is a lawn**
    (`roads::ring_island_lawns`, layer `ring_islands` at `Z_RING_ISLAND` 0.05, roads plan
    №31): the ring's closed drawn axis filled with grass — the ring's asphalt covers the
    outer half, so the lawn shows up to the inner kerb. The mapped grass on an island is
    usually smaller than the drawn island and left a ring of pale ground round it (Orel
    01 and 02). The layer sits right over the bare ground, **under** everything mapped
    on the island — grass, a park, a block, a square, water, a house stay themselves,
    and only the ground turns to lawn. Over the grass it would also hide the faint rim
    of the grass polygon (visible in Ryazan 04 as a circle in the lawn), but it repainted
    an island park as lawn too (Kaluga 05) — tried and rejected. **The rim goes by a
    second layer instead** (`roads::ring_island_grass`, layer `ring_island_grass` at
    `Z_RING_GRASS` 0.65 — over the grass, under the sand, roads plan №37): each island
    intersected (`i_overlay`, NonZero) with every `MapData::grass` polygon whose box
    touches it, laid again in the grass colour and material, so the lawn and the mapped
    grass meet with no rim between them (Ryazan 04: an 81 m `landuse=grass` circle on a
    60 m ring; Orel 02). Only grass is laid — a park, a wood, a block on the island are
    not touched (Kaluga 05 identical). A mapped grass patch inside a park island
    (Ryazan 05's octagon) keeps its fill and loses only its inner rim.
  - The report counts `rings N (M webs)`. Tula (release): 8 rings, 19 webs (11 while
    only the arms were walked); the road build did not move (118.5 ms against 120.7 when
    the rings came, 125.4 → 125.6 when the webs spread to every street along a ring).
- **Junctions** (`map/roads/junctions.rs`) — computed for the markings only, and from
  **shared nodes**, not segment intersections: Overpass `out geom` gives no node ids, but
  a node shared by two ways projects to the same `Vec2` on both (quantised to 5 cm to be
  safe). Participants are carriageways (`is_carriageway`). A node with two or more
  distinct participants is a junction and hands every road there a `Break` of reach
  `half the widest other road + JUNCTION_MARGIN` (1 m); a node where exactly two ways
  *end* is a continuation (one road split by a tag change) and no break; a lone way end
  is a dead end (reach 0); a closed way's seam is not an end. Segment intersection was
  rejected on purpose: a bridge over a street shares no node with it and must not break
  either line, which the node rule cannot do wrong. A service drive or a footway joining a
  street is not a participant and leaves the street's line whole. Count and time are in
  the `road meshing:` log line (`junctions N`).
  **A stitch is a node too** (`junctions::with_stitches`, over `Stitches::targets`): the
  loose end the network pulled onto another road's axis (**Stitches** below) meets it at
  the target point, the target passes that node *inside a segment* (`Visit::inner`), and
  the end's own OSM node stops being a dead end. Without it the 89 stitched ends of Tula
  joined with no break, no zebra and ruts fading at their OSM end, short of the street.
  These are the **asphalt breaks** — the ruts fade and the medians open on them. The paint
  layer breaks on its own set (**Junction paint** below), where a main road keeps its lines,
  and the street fill takes `NodePaint::asphalt` — these minus the ones on a junction's
  **leading** road, whose ruts run through (**Turn paths** below). The medians keep the
  unrewritten set: a median opens at a crossing whoever leads it
  (`tests.rs::the_median_base_keeps_the_break_a_leading_road_lost`).
  **`Junctions`** (`junctions.rs`) is the one value `mesh_roads` asks all of this of —
  `Junctions::new(&Drawn, map, paved, style)` owns the three computations over the
  shared nodes: the base breaks (`marking_breaks`), the **Junction paint** below
  (`NodePaint`) and the **row breaks** (`pockets::row_breaks`, **Kerb pockets**). Out of
  them come **five sets of breaks per road, and merging any two is a regression, not a
  simplification** — so each leaves through a door of its own and **as a type of its
  own**, and a consumer handed the wrong set does not compile: the **base**
  (`median_base()`, a bare `&[Vec<Break>]` — the medians; nobody rewrites it), the
  **asphalt** (`asphalt()` → `node_paint::AsphaltBreaks`, `of(road)` — the base minus
  the leading road's, the fill and its ruts), the paint **cut** and **solid** (`paint()`
  → `node_paint::PaintBreaks`, whose `of(road)` is the `paint::LineBreaks { cut, solid }`
  `Painter::paint` takes — where lines stop, and where an axis stays solid through a
  node its road passes), and the **row** (`row()` → `pockets::RowBreaks`, `of(road)` —
  no stitches, but service drives, taper clearings and OSM zebras: the kerb pockets and
  the cars). The two views are built by `NodePaint` (`asphalt()`, `lines()`), which owns
  the vectors; `RowBreaks` is an owned value that **only `pockets::row_breaks` /
  `row_breaks_over` produce** — a `MarkingBreaks` cannot be passed off as one, which is
  what keeps `all_kerbsides` and `park_cars` off the base. What the paint *draws* — the
  pockets at way ends, the zebras, the stop lines, and the `Junction` clusters the turn
  paths and the fill order read — is `node_paint()`. Pinned by
  `tests.rs::row_breaks_ignore_stitches_but_paint_breaks_do_not`: a stitched side street
  is a junction for the base and the paint and a dead end for the row. Its counters are
  one value as well, `counts() -> JunctionCounts` (junctions, clusters, main through,
  leading roads, zebras, stop lines, paint pockets), nested in the report as
  `RoadReport::junctions` the way `DrawnStats` is in `drawn`; the `road meshing:` line
  prints what it printed before, and `leading()` — one flag per road — is what the fill
  order sorts by (`tests.rs::the_report_counts_the_junction_paint`). **The nodes are
  walked once**: `shared_nodes` over the row's
  participants (`pockets::is_row_participant` — everything driven on, the carriageways
  among it) gives the row, `restrict` keeps the carriageways' visits of the same nodes
  (bit for bit what a second walk would find — the node's point is its first remaining
  visit's) and `stitch` adds the stitches; the base and `NodePaint::new` take that one
  list. The paint used to walk the roads *as drawn*, the base the map's: the nodes are
  the same, because `Drawn` never moves a point and `is_carriageway` reads neither the
  width nor the class a driveway crossing is given
  (`tests.rs::one_shared_node_pass_feeds_both_base_and_paint`) — only the reach of a
  break differs (`a_ring_arc_base_break_reaches_by_the_osm_width`), and that is read off
  the map's widths. Three walks of the points per `mesh_roads` (and two stitch passes)
  became one of each; the car layer still walks its own (`mesh_cars` builds no
  `Junctions`). The one change from outside is `add_splitters` — a splitter
  island (**Roundabouts**) is found on the ribbon axes and the rings, which the nodes do
  not know, and cuts both the paint and the asphalt of its approach
  (`a_splitter_gap_reaches_both_asphalt_and_paint`); the base and the row never see it.
  Likewise three notions of «node» live here and stay three: `SharedNode::is_junction`
  (the breaks), the corner node of `corners` (a kerb return between a drive and a street,
  with no paint break) and the cluster of `node_paint::Junction` — one module, not one
  concept. The car layer is not a `Junctions` client: its `Drawn::nodal` has no stitches
  and no paint, so `mesh_cars` calls `pockets::row_breaks` itself.
  **Junction geometry is not computed as a union**: roads are independent polylines
  drawn overlapping in one opaque layer. Until stage 5 the `Round` caps were what made a
  junction *look* joined — the caps of the ways meeting at a node overlapped into a
  rounded blob, the osm-carto way (`stroke-linejoin: round` + `stroke-linecap: round`).
  Now an arm ending in a junction ends **square** on the node, and the junction's own
  pieces — the kerb returns and the outer corners — fill the rest (**Kerb returns**
  below). The fill order is **narrow first, wide last** (`mesh_roads` sorts by width), so
  the main road's fill and its gapped line lie over the side street's end — and a road that
  **leads** some junction goes after all the others whatever its width: its ruts run
  through the node, and a wider side street laid over them would cut them. «After all the
  others» is **per junction** (`roads::fill_order`, a topological sort over «arm before
  its junction's leader», the width key as the priority, the first by key on a cycle): a
  side street that itself leads a junction further on used to land in the leaders' tail
  and, being wider, lay its square end over the main road — a rut-less square in the
  middle of the crossing (Tula, 5968 1582, the author's report). This is why the
  road layer must stay opaque with a world-position colour: transparency or a per-way tint
  would expose every crossing.
- **Junction paint** (`map/roads/node_paint.rs`, `NodePaint::new(&Drawn, base, map, paved,
  style)` — the roads, the ribbon axes, the stitches, the merges, the mapped sidewalk, the
  pair partners and the ring arcs all come off `Drawn`; built by `Junctions::new`
  on the ribbon axes — **always**, markings on or off: with markings off it paints
  nothing, but the leading roads and the junction arms it finds are the asphalt's, not the
  paint's) — what the paint layer does at a junction. It starts from the asphalt breaks
  and rewrites them per road:
  - **Clusters**: junction nodes (`junctions::with_stitches`, `SharedNode::is_junction` —
    the same rule as the asphalt breaks, stitches included) whose **zones** overlap are one
    junction. A zone
    is the half width of the node's widest road plus `CLUSTER_ZONE` 6 m (the street kerb
    radius); union-find over `Grid::pairs`. One set of arms, one break per road: two nodes
    of a cluster on one road get a bridging break between them, so no orphan dash is left
    between. The case it was written for is Tsiolkovsky street (Tula, 4703, 332), gallery
    sample 19: Shchorsa street and Tsiolkovsky lane join from opposite sides 17 m apart.
  - **Who breaks**: an **arm** is where a road leaves the cluster (a piece between two
    nodes of one cluster is inside it); a street **passes** a cluster when it has two arms
    there (a ring always passes). A road breaks its paint when the cluster is signalized
    (`TrafficSignals` inside a node's zone or a signalized crossing within 30 m), when its
    street does not pass, or when another road outranks it — a higher rank, or an equal
    rank that also passes (a crossing). Rank is the `highway` class (trunk 5, primary 4,
    secondary 3, tertiary 2, residential / unclassified / living street / links 1),
    doubled, and a `stop` / `give_way` node on the road within 30 m of the cluster takes
    half a step off. The other half of a divided street (`Pairs::runs` partner) is no
    rival. So a side street **joining** a through street — even of the same class — does
    not break its lines: the dashes run through the junction on the same axis, and the
    report counts it as `main through`. **A crossroads is not a joining**: when the other
    streets have two arms or more there (`crossed`), an equal-rank road breaks even if its
    rival does not "pass" by street identity — OSM splits a cross street into two streets
    at a oneway change, and Петра Алексеева (Tula, 5968 1582) ran its dashes straight
    through a four-way crossing of equals because Макса Смирнова is one-way south of it
    and two-way north. **A crossroads of tertiary rank or higher breaks the higher road
    too** (`CROSSING_CUTS_RANK` 2 — the crossroads' rank is the lower of its two
    streets): no road paints its lanes through the field of a real crossroads, and
    before this a primary kept its solid line diagonally across a secondary pair (Орёл,
    gallery sample 05 — Московская over Пушкина; the signals there stand 27–29 m out,
    beyond the cluster zone). A residential crossroads does not cut a main road, and a
    joining street of any rank never does (Ростов 03 stays through) — pinned by
    `a_tertiary_crossing_breaks_the_primary_too_but_a_residential_one_does_not`. Signals
    aside, such a road **leads** the junction
    (`Junction::leading`), and so does a ring road (an arc of `on_ring` or a closed way —
    whether it passes or not, since OSM ends an arc at every entry) whatever the
    approaches' class — a ring has priority; an approach never leads a ring node. The leading road loses its asphalt breaks there
    (`NodePaint::asphalt`): the ruts run through, signals or not.
  - **Zebras and stop lines on the arms that break**: an OSM crossing on the arm (a
    `Crossing { marked: true }` node on the road, between the node and
    `ARM_CROSSING_REACH` 35 m past the junction edge — measured from the edge, since a
    wide junction's edge is itself tens of metres from the node) becomes the zebra; without
    one, `CrossingMode::Generated` puts a zebra `ZEBRA_SETBACK` 1 m past the edge — if the cluster joins two streets
    with sidewalks (by the `sidewalk=*` tag — `RoadLine::sidewalks`, like `kerb_parking`;
    the Sidewalks toggle only hides the band, `crossings` is the zebra's own knob) — or
    the cluster is **signalized** and the arm itself has a sidewalk on both sides by its
    tags (Tula, gallery 08: Ложевая is `sidewalk=no`, so the T with signals had one
    walked street and not a single zebra) — one of
    the cluster's streets is at least `tertiary` (`RULE_ZEBRA_RANK`) or the cluster is
    signalized, no road of the cluster is a ring arc (`on_ring`, the check the lane
    arrows use — and no closed ring passes it), the
    arm is not a `*_link` and the next junction node on the same road
    lies at least `RULE_ZEBRA_ROOM` 30 m past the edge (`nodes_along`). A shorter arm is
    a link between two nodes — the branches of a fork's triangle (Tula, gallery 06, 22 and
    31 m) had a rule zebra at both ends and a stop line between them within fifteen metres;
    the crossing is left to the outer arms. An OSM crossing ignores the room. **A
    street already crossed by the data gets no rule zebra**: a marked OSM crossing on any
    road of the same street (`street_of`) visiting the cluster, within
    `RULE_ZEBRA_DATA_REACH` (= `ARM_CROSSING_REACH`, 35 m) of the arm's node point, drops
    the rule zebra from every arm of that street — the mapper said where this junction is
    crossed, and a second zebra twenty metres from the first read as a mistake (Tula,
    gallery 12: the T of Халтурина into Красноармейский had the rule zebra west of the T
    and the signalled OSM crossing east of it; Yandex draws one). The stop line stays;
    the other street's arms are untouched. A zebra is `ZEBRA_LENGTH` 4 m along the
    road, across the carriageway less 0.3 m at each kerb. The stop line is called by the
    same things as the zebra — a zebra on the arm, signals, a stop / give-way sign, or a
    street of at least `tertiary` in the cluster; two residential streets with no sign get
    neither (gallery 13: Yandex draws the cross bare). The stop line (0.4 m) stands
    `STOP_GAP` 1 m behind the zebra (or 1 m past the edge without one), across the lanes
    **coming to the node** — axis to kerb on the traffic side (`MapData::traffic_side`)
    for a two-way road, the full width for a one-way one that flows toward the node, none
    on a one-way arm leaving it. A `give_way` sign without signals makes it dashed. **A
    stop line needs room for a queue behind it**: at least `STOP_QUEUE_ROOM` 10 m (two
    cars) of free road from its back to whichever comes first upstream — the asphalt of
    the next junction node on the road (its node point less the half width of its widest
    other road) or an OSM crossing's zebra. Shorter, and the line is dropped (the zebra
    stays): it lay on the throat between the halves of a divided street, where a waiting
    car would stand on the neighbouring junction or on the zebra across the median
    (Ryazan, gallery 07: Горького through Есенина's lawn, 28 m node to node — one line
    right behind the median zebra, the other just out of the far half; Rostov,
    Театральный × Красноармейская, the same). 1–3 % of the lines per city (Tula 759 →
    751, Ryazan 705 → 688, Kaluga 819 → 799, Oryol 742 → 729, Rostov 1680 → 1664).
    Ring entry lines are exempt. The
    two halves of a divided street cross on **one line**: `align_pair` moves the second
    zebra onto the first's line across the street (to the OSM one if there is one, else to
    the farther one); when **both** are OSM crossings — a `highway=crossing` node on each
    half, which mappers place a metre apart (gallery 02: 0.9 and 1.2 m) — both move to the
    line halfway between them. Over a **paved** median (`PairRun::paved`, read through
    `Pairs::partners` as `pairs::Partner { road, paved }`) the two aligned zebras then become **one
    plank** kerb to kerb (`join_zebras`: parallel within `JOIN_PARALLEL`, on one line
    within `JOIN_OFFSET` 1 m, the gap between them at most `node_paint::JOIN_GAP` 8.8 m —
    `TRAM_BED_MAX_GAP` plus `EDGE_INSET` 0.3 m off both kerbs plus `OVERLAP_SLACK` 0.2 m,
    since the widest paved median is a tram bed and at a flat 8 m a bed wider than 7.4 m
    kept two planks and a seam): the shader
    counts the bars from the plank's end, so two planks put the bars out of step at the
    seam — Yandex draws 02 and 12 as one plank. The median's double solid is broken there
    anyway (its breaks are the halves'). A lawn median keeps two zebras, each to its
    kerb. **Paint inside another road's asphalt is dropped**: a stop
    line, or a rule zebra, whose point on the arm lies within another cluster road's half
    width (less `EDGE_INSET`) of its axis. The edge is measured from the node point, and
    an arm merging at a shallow angle (a link into Пролетарская, gallery 08; the fork's
    throat, 06) is still under its neighbour's ribbon there — the paint lay as a stub in
    the middle of the junction (roads plan D2). **The edge of an arm is where its
    cross-section leaves the other roads' asphalt** (`node_paint::clear_reach`, roads plan
    D11): from the break reach (half the widest other road + 1 m) outward in
    `EDGE_STEP` 0.5 m steps until both points `half width − EDGE_INSET` off the arm's axis
    are outside every other road of the cluster (its half width off its axis; the
    arm's own street and its paired half are no rivals) **and** outside every paved
    island of a node triangle (`corners::small_islands`, handed to `NodePaint::new` as
    `paved`), up to `EDGE_SEARCH` 25 m. The rule zebra, the stop line and the turn paths
    and arrows (`JunctionArm::edge`) measure from it, and the lane lines break from the
    node to the outermost paint — and on an arm with **no paint at all** (a one-way
    leaving the node, an arm no zebra or stop line is called for) to the edge itself:
    at a shallow crossing the break reach is the neighbour's half width, 3 m for a
    one-lane street, while its asphalt runs a dozen metres along the arm, and the lines
    crossed the junction field (Орёл 05, Московская at 23° to the Пушкина pair; roads
    plan G3). A **link** with no paint — an arm whose section never left within
    `EDGE_SEARCH` — breaks up to where its **lines** leave the other roads' asphalt
    (`lines_edge`: the same walk with the half width of `line_half`, the outermost lane
    line's offset — the lane frame less one lane, plus a two-way axis shift; none on a
    one-lane road): a branch peeling off a four-lane primary at 21° (Вокзальная out of
    Первомайский, Ryazan 03, roads tails L2) never clears its section within 25 m, so its
    edge stayed at the neighbour's half width and its lane line started in the middle of
    the primary's lanes, crossing their line. The primary itself is not touched — its
    section's two side points clear the narrow branch at once, and the main road keeps
    its lines past the fork; the branch's line now starts past the primary's kerb
    (`a_fork_branch_keeps_its_line_off_the_lanes_of_the_main_road`). The turn paths keep
    the arm's edge as it was. An **OSM crossing keeps its place** — it is measured
    from the reach as before: pushed past the new edge, it no longer fitted a short arm
    with its `ARM_TAIL` and was lost (gallery 04, south). **Not at a ring**: an approach
    is fitted into the ring tangentially and runs over its asphalt for tens of metres, so
    the other roads' asphalt would push the edge past the crossing; the turn paths keep
    the break reach there. **The paint of a ring entry is set by the ring alone**
    (`RingEntry`, `ring_entry`): the ring roads of the cluster (`on_ring` or a closed way)
    are the only rivals, and each of three points across the arm — the far end of the stop
    line (the left kerb of a one-way entry, the axis of a two-way one), its kerb end, the
    other kerb — is walked out in `EDGE_STEP` from the node until it leaves the ring's
    ribbon. The line of an entry without a zebra then runs **from where one end left the
    ring to where the other did** — along the ring's edge, as it stands on the ground,
    not across the approach — and is always dashed (**give way** to the ring; solid only
    under signals); the arm's edge, from which the lane lines break, is where the whole
    section has left. The old line stood across the approach at the break reach (half the
    ring + 1 m from the node), which on a tangential entry is still the middle of the
    ring: it ran over the ring's lanes up to the island's kerb, with the approach's
    solid lines after it (gallery 04, south and north-west). The walk goes out to
    `RING_EDGE_SEARCH` 60 m (`leaves_ring`), not the arm's `EDGE_SEARCH` 25 m: the east
    entry of gallery 04 (way 131741966) runs along the ring's asphalt for about thirty
    metres, the entry was not found at all, and its lane line ran over the ring toward
    its axis as a merge would, solid for the last 25 m (roads tails L5,
    `a_long_tangential_ring_entry_yields_and_keeps_its_lines_off_the_ring`).
    **A ring exit has a throat** (`node_paint::Throat`, `throat_on`): for a one-way arm
    leaving a ring node, both kerbs of its section are walked out of the ring's asphalt
    the same way, and the ring road's stretch from half the exit's width before the
    node's projection (the exit's ribbon end already lies on the outer lane there, and a
    dash cut by the node was left as a stub) to the projection of the point that left
    last (the shorter way round a closed ring) breaks
    the ring's lane lines **on the exit's side of the ring's axis only** (`side`, the sign
    of the path's left normal; `Painter::paint` keys the profile by it, bits 2 and 3 of
    the pocket mask). The outer lane is the exit there, and its dashes ran straight
    across the mouth out of step with the exit's solid line (gallery 04, south, roads
    tails L5); the inner lines run through. The throat is no approach: the ring's lines
    stay dashed before it (the solid approach spans are taken off the profile without
    the throat). An arc that ends at the node gets only that stretch before it — both
    projections fall on its end (`a_ring_exit_breaks_the_outer_ring_line_across_its_throat`). **A ring road never gets a
    stop line** and **always leads** its node — OSM cuts a ring into arcs at every entry,
    so an arc "passes" by street identity nowhere: every arc broke at every entry with a
    stop line across all its lanes and 25 m of solid approach lines (gallery 04 south,
    Ryazan 01, Kaluga 01). And an approach never leads a ring node, even where the network
    carries its street on into an arc.
    **An arm that never leaves the junction's asphalt is a link** (`JunctionArm::link`,
    `ArmPlan::link`) — a throat of a complex junction, not an approach to it: no rule
    zebra, no stop line, no arrows. At the fork of gallery 06 the triangle's 22 and 31 m
    branches run from node to node across the paved island, and their stop line and
    arrows lay on it (Yandex has only the centre lines there). A length rule (under
    30 m to the next node) was tried first and took the stop lines off every short
    approach — the fan at 04 south among them. Zebras that land on one another — the two branches of a fork at
    one node, an OSM crossing beside a rule one — are reduced to one (`without_overlaps`,
    the last step of `NodePaint::new`, after the mid-block crossings): the OSM zebra
    stays, of two generated the first; a plank counts as inside another when a 9-point
    probe of its line falls within the other's box less `OVERLAP_SLACK` 0.2 m, so the
    halves of a divided street standing side by side keep both. The report's `zebras N`
    is the count after it. An arm whose way ends less than `ARM_TAIL` 8 m past the paint is a
    link inside a complex junction and gets nothing. The paint break covers the edge to
    the outermost stroke plus `PAINT_CLEAR` 1 m (it was 0.5 with the metre-long fade, which
    ended the visible line about a metre out anyway; the cut is sharp now).
  - **A paint gap spills over a way end** (`node_paint::spill_over_ends`): OSM splits a
    street at a signal or a crossing, and a zebra at the very end of a short way cut the
    lines of that way only — the next way's double solid started flush with the zebra
    (Первомайская, 3279 2799: way 396629201 ends 1.7 m past its crossing). Whatever
    `Walk::gap` clamps at a path end is kept as a spill (`Walk::spills`) and handed, as a
    break from that end, to the one other carriageway ending at the same point — only at a
    plain continuation, never at a junction node, where every road has its own break and a
    leader keeps its lines.
  - **Mid-block crossings**: every marked OSM crossing on a carriageway not taken by an
    arm is a zebra with a gap in the lines around it, and stop lines on both approaches
    if it is signalized. A crossing inside a break already there is skipped.
  - **Pocket**: when a street passes without breaking and its arm on the far side has
    fewer lanes, the lines of the wide arm that lie outside the narrow one's lane frame
    end at the junction edge (`Pocket`, one extra break for those lines only —
    `meshing::break_distances` re-measures to-break on the already cut path) — solid for
    the approach, as a turn pocket reads — instead of running into the junction.
  - **Short runs**: a run of lines under `MIN_RUN` 6 m between two breaks is closed. A
    way end that is a pure seam into a way with **fewer lanes** (`narrowing_ends`) counts
    as a break here: the lines the neighbour lacks die in the taper anyway, and a 9 m
    street between a junction and its taper left a two-metre dash at the kerb (gallery 08).
  - **The median's double solid** breaks on the paint breaks as well (both halves'
    zebras and stop lines, `medians::crossing_breaks` over them), not only on the asphalt
    ones. Those breaks lie on the halves' axes, off to the side of the midline, so a long
    one covers the midline short of its end; a piece of the double solid under
    `MEDIAN_MIN_RUN` 6 m between two breaks or between a break and the run's end is
    closed as well (`medians::bridge_short_pieces`) — a metre-to-five stub was left in the
    middle of Орёл 05's junction. A midline no break touches stays as it is. **The paved
    median keeps the full paint breaks, zebra and stop lines together** — unlike the lawn,
    which a zebra only cuts a passage through. Narrowing them to the zebra was tried for
    Moskovskaya at Pushkina (Орёл 05, roads plan №41), where four crossings within thirty
    metres leave no piece of double solid: the line came back there, but the neighbouring
    pair's double solid ran on past a stop line into the junction, so it stays by rule.
  - Not drawn from data: `footway=crossing` ways are not parsed (the crossing node is
    what Tula maps). Islands and `RoadArea` outlines are drawn by **Safety islands** below.
  The report counts `junctions N (C clusters, main through T), zebras Z (O from OSM),
  stop lines S, pockets P`.
- **Safety islands and carriageway areas** (`map/roads/islands.rs`, `RoadIslands`, roads
  plan I6) — the first reader of the v15 islands and `RoadArea` outlines:
  - **An island node** — `RoadNodeKind::Island` or a crossing with `island` — on a
    **two-way** carriageway of two lanes or more (found by `node_key` of its vertices;
    the node is a vertex of the way) becomes a kerbed lens on the drawn axis:
    `REFUGE_LENGTH` 8 m along it (the zebra plus a metre of kerb each side),
    `REFUGE_HALF_WIDTH` 0.9 m, an ellipse profile to a point at both ends. A one-way
    street has no room between opposing lanes for it and gets none.
  - **An `Island` outline** is a kerbed island by its outline.
  - Both are sidewalk-coloured and go into the **`lot_sidewalks`** layer
    (`Z_LOT_SIDEWALK` 2.002) — above the road asphalt **and** its paint: the way's
    ribbon runs straight through the island (OSM does not split the axis around a
    refuge), and on the ground the lane lines and the zebra stop at its kerb, which is
    exactly what covering them does. The stroke of a flare around the island is not
    drawn.
  - **A `Carriageway` outline** is asphalt in the `roads` layer, under the ribbons: a
    square, a lay-by, a widening the axis does not describe.
  - **A `Walkway` outline** (`area:highway=footway|pedestrian|…`, `highway=pedestrian`
    + `area=yes`) is **paving in the `sidewalks` layer**, the colour of a paved path.
    It used to be left "to the sidewalks and alleys that already cover them", and they
    did not: Tula's 62×75 m `area:highway=footway` plaza on the Lenina park paths
    (way 27582887) was bare ground with a service drive ending in a round cap in the
    middle of nothing, and the 18×18 m `pedestrian` square on the ring island at
    2539 2393 (way 234168508) was a ring of path ribbon round a patch of ground.
  - **The closed line of an area is not drawn as a ribbon** (`RoadIslands::outlines`,
    one flag per drawn road, skipped at the top of the ribbon loop). The parse still
    hands `highway=*` + `area=yes` over as both an outline and a `RoadLine` (every other
    reader of `MapData::roads` keeps seeing it as before), and that line is recognised here as a
    closed ring whose points are exactly an outline's (indexed by `node_key` of the
    first vertex). Laid as a ribbon it was the ring above; a carriageway area's line
    is skipped likewise, its outline being asphalt already.
  - Tula v15: 19 walkway areas (the report's `walkway areas W`), 2 of them
    `pedestrian` + `area=yes`.
  - **Tula has almost none of it** (no island at all, one `crossing:island`, a dozen
    service-yard outlines — `references/osm-coverage.md`, «v15»), so the gallery check
    is Berlin (`ROADS_CITY=berlin`, samples 4–6: `area:highway=traffic_island` at
    Rosenthaler Platz and the boulevards, `area:highway=primary|tertiary` outlines,
    signalized crossings with islands). The report counts `safety islands N + A areas,
    carriageway areas C, walkway areas W`.
- **Turn paths** (`map/roads/turns.rs`, `Turns::new(&Drawn, junctions, side)` over
  `NodePaint::junctions`, on the ribbon axes) — the
  wear a junction gets from traffic crossing it. The lane ruts fade in a junction gap (a
  car crossing a junction is not in a lane), so without these the middle of every node was
  bare asphalt, and a real one is polished lighter than its approaches.
  - **Arms** (`JunctionArm`): every arm of the cluster, rings included (a closed ring gets
    two, one each way from the node, which the zebras never see), with its **edge** — the
    arclength on its drawn axis where the junction gap ends (half the widest other road
    plus 1 m, the asphalt break's reach), on a ring taken around the seam.
  - **Lanes on an arm**: the body lane frame (`paint::lane_frame`), lane centres between
    the lines; a one-way road carries traffic along its points only, a two-way road along
    them on the traffic side's side of the axis (`MapData::traffic_side`,
    `paint::axis_offset` — an odd road's extra lane goes to the flow along the points or
    as `lanes_backward` says) — except a one-lane road, driven both ways. Lanes are
    counted **from the kerb**.
  - **Maneuvers** by the turn angle between the in-lane's travel and the out-lane's:
    under 35° straight, over 150° a U-turn (not drawn), else a **near** turn (toward the
    kerb — right under right-hand traffic) or a **far** one. Which lanes into which:
    `RoadLine::turns` (`turn:lanes`, parsed per direction of flow, left to right) when the
    tag's lane count matches the arm's; otherwise the rule — straight from each lane into
    its own (kerb-first, as many as both sides have), near only from the kerb lane into the
    kerb lane, far only from the inner lane into the inner lane — the lane nearest a turn
    turns and goes straight, the rest go straight (the author's rule). **At the stem of a
    T** (`dead_end`: the approach has no straight exit) every lane turns both ways except
    the two outer ones, each of which turns only its own way. Tagged far turns pair from
    the axis side, the rest from the kerb.
  - **Straight along a leading road is skipped**: its ruts are the asphalt's. On a
    junction with no leader (a crossing of equals) both straights are curves, and their
    weaker wear laid crosswise is the «both go through at half strength» the plan asked
    for — a light cross, not a light square.
  - **Curve**: a cubic Bézier from lane centre at one edge to lane centre at the other,
    tangent to both (control arm a third of the chord, 0.39 of it at a quarter turn), cut
    into links by **sagitta** — a link's chord at most `SAGITTA` 3 cm off the arc, no finer
    than 3° (one link for a plain straight, four for a lane shift; 15° per link read as
    facets on the turn — the author's report). Plus a **tail** of `TURN_TAIL` 5 m straight
    into the lane, **one per lane end** (`JunctionWear::tails`), however many maneuvers
    start or end there: over the tail the rut fades to nothing while the lane rut, faded
    to nothing at the gap edge over the same 5 m (`WEAR_FADE`), fades in.
  - **Drawn like a shadow**: a rut over a rut is no lighter, as a shadow over a shadow is
    no darker — the author's rule, after the first version (a strip per curve,
    alpha-blended) lit every crossing of two ruts and the whole middle of a crossing of
    equals. A strip per curve and per tail (`Painter::paint_turn_wear`, kind 6), the two
    gaussian ruts ±0.85 m (σ 0.32, the lane ruts' profile) drawn by the shader, the
    strength in the vertex alpha (1 on a curve, 1 → 0 along a tail) — laid **twice**, in
    two meshes and two passes of the paint material (`PaintPass`, a `bind_group_data` key
    that picks a shader def and the blend state):
    - `road_paint_wear_mask` at `Z_ROAD_WEAR_MASK` writes **only the frame's alpha**, with
      the `Min` blend op, the value `1 − rut`: the asphalt under it is opaque (alpha 1), so
      what is left in a pixel is `1 −` the **largest** rut of all the strips there;
    - `road_paint_wear` at `Z_ROAD_WEAR` returns 1 and blends `colour × Dst +
      dst × (1 − Dst alpha)` — the pixel times `1 + rut`, the multiplicative rut of
      `surface.wgsl`, so **Turn wear** (`RoadPaintStyle::turn_wear`, 0–8 %, 3.5) reads in
      the same percent as Wear — and writes alpha 1 back. A second strip over the same
      pixel then reads alpha 1 and changes nothing.
    The mask mesh sits below the apply mesh, and the transparent phase draws them whole in
    z order, so every mask strip lands before any apply. Both fade by `visible(lane)` and
    the axis zoom like the lines (`PaintTag::Wear`), and both are built with markings off
    too, like the ruts. **The union was tried and dropped**: `i_overlay` over the ruts of
    Tula — the `shadow::push_union` construction — took the road build from 112 to 841 ms
    (428 per junction with shared tails) and 1.1 M vertices; the two passes cost what the
    strips cost — 121 ms against 112 before stage 5в, 108 k vertices per mesh. What the mask cannot see is the asphalt's own ruts: a turn rut crossing
    the leading road's lane rut still adds to it.
  - **No guide dashes**: the 1.7 marking of a far turn was built from the same curves and
    dropped at the author's call — dashed arcs across the junction read as clutter.
  - **Lane arrows** (stage 7, `Turns::arrows`, `LaneArrow`) — per incoming lane its
    maneuvers: `turn:lanes` when the tag matched the arm, otherwise — only on an approach
    with `ARROW_MIN_LANES` 2+ lanes of its direction — the maneuvers the rule grants it
    above (collected before the leading-road skip, so a straight along the leader still
    counts); none on a bridge, none for a lane with no maneuver, and **no rule arrows at a
    node with a ring arm** (`on_ring`, from `Axes::rings` — «straight» there means
    «into the ring», gallery 17 showed arrows on the ring itself), and **none where the
    approach has no choice** (`has_choice`: the lanes together grant fewer than two of
    left / through / right — at the split of a divided road the other half leaves as a
    U-turn, and gallery 17 carried two «through» arrows on its west exit;
    `no_rule_arrows_where_the_approach_has_no_choice`); tagged ones stay. `Painter::paint_arrow`
    lays them as filled polygons in the **lanes** mesh, `LineKind::Arrow` (kind 9, cover
    1, fades with the lane lines at `LANE_ZOOM_MAX` — the near zoom only): a stem along
    the travel, a head if straight is allowed, a 45° branch with its own head per allowed
    turn; 5 m long, tip `ARROW_SETBACK` 4 m behind the farthest zebra or stop line crossing
    that lane's axis within `ARROW_MARK_REACH` 30 m of the edge (`Painter::
    arrow_setback`), or 4 m from the edge with none — a zebra stands off the edge by its
    crossing's position, and a fixed setback from the edge put arrows on it (gallery 1,
    21). The setback is measured **along the lane**: each arrow carries `LaneArrow::back`,
    its lane's centreline from the edge back against the travel for `ARROW_BACK` 60 m (the
    drawn axis offset by the lane, `turns::lane_back`), and `paint_arrow` puts tip and tail
    on it by arc length (`map::along::place_on_path`); a straight line back from the edge left the
    lane on a curved approach — 20 m out it sat on the lawn (Leipziger Straße, Berlin).
    A lane shorter than that falls back to the straight line. **A second row** stands
    `ARROW_REPEAT` 20 m behind the first (`Painter::repeat_setback`; Yandex puts them at
    5 and 20–25 m from the crossing on 01, 02, 15, and ГОСТ 1.18 repeats them): only where
    the lane's centreline runs on `ARROW_REPEAT_CLEAR` 5 m past its tail, no zebra or
    stop line crosses the lane between the rows, and the row is clear of the approach
    road's own breaks (`LaneArrow::road` → `PaintBreaks::of(road).cut`) — the centreline is the
    whole way's, and without that the row lay in the previous junction. A short block
    therefore keeps one row. The marks sit in a `Grid`
    (`paint::ArrowMarks`): a scan over all 3600 per arrow
    cost Tula 4 ms of the road build. Stage 7 on Tula: 120.9 ms (118.5 before), 920 k
    vertices (900 k), 1089 arrows, 610 kerb pockets. Drawn under their own
    `RoadStyle::arrows` toggle (stage 8), independent of `markings`.
  The report counts `turn paths W, arrows A, leading roads L`.
- **The drawn network** (`map/roads/network/mod.rs`, `map/roads/corners.rs`) — what the ribbons
  are laid *from* is not quite `MapData::roads`, and the difference is four render-only
  corrections, all built on **`RoadNodes`** (every node two roads of any class share, same
  5 cm key as the junctions, `junctions::node_key`). None of them moves `RoadLine::points`:
  the navmesh, doors, trees and arches still read OSM as it is (the parked cars stand on
  the drawn street axis — **The street axis** below). All four
  are counted in the `road meshing:` line, with the time spent before the first ribbon.
  - **`Drawn`** (`roads/drawn.rs`) — the prepared roads as one value, built once at the
    top of `mesh_roads` (`Drawn::new(map, style, shape)`) instead of the locals that used
    to open it. **Its fields are closed**; what it holds is reached by queries: `road(i)` /
    `roads()` (the map's roads with the substitutions on their index — driveway crossings,
    then ring arcs, the later one winning; an unsubstituted road is a borrow of the map's),
    `nodes()` (`RoadNodes`), `axis(i, Axis)` / `axes(Axis)` (below), `pairs()`, `rings()`,
    `stitches()`, `tapers()` (the one taper pass of the layer, **Tapers** above),
    `merges()`, `lots()` (the `KerbLots` the pockets and the car row share), and the
    per-road answers that used to be closures in `mesh_roads`: `taper_ends(i)`,
    `is_merged(i, end)`, `stitched_end(i)`, `stitch_offset(i)`, `on_ring(i)`. The pair
    side is not among them: it is asked of `pairs()` (**Paired halves → Queries**), one
    owner rather than a delegate on each. Every vector is
    indexed by `map.roads` (an `assert` in the build, not an empty answer on a mismatch).
    The OSM points are not in it — whatever finds a node by `node_key`, and the kerb-pocket
    seed, reads `RoadLine::points` off the map. Not to be
    confused with `network::DrawnEdges`, the outer edges the stitches measure against.
    **Three sidewalk rules live on it, and they are three on purpose**:
    `sidewalk_drawn(i)` — the band that is *drawn*: `SidewalkProfile::any` (the map's
    sidewalk) under the `Sidewalks` knob, minus the crossing piece in a pair's opening
    (`across_median`, **Kerb return** below) — the ribbon and the turning circles read it;
    its per-side form `sidewalk_on(i, side)` is the same answer where the tag puts a
    sidewalk on that side (`SidewalkProfile::sides`), one call where the kerb pockets, the
    kerb returns (the road's own sides and the narrow neighbour's under a wedge) and the
    merge edges (`merge_bands` takes it as `sidewalk(half, side)`) each used to AND the
    tag in by hand; the pair side is still the consumer's to drop (`Pairs::beside`,
    `band_pieces`) — pinned by `tests.rs::a_pocket_on_the_side_without_a_sidewalk_pushes_no_sidewalk`
    and `a_one_sided_street_turns_its_sidewalk_only_on_its_side`; `sidewalk_mapped(i)` — the sidewalk
    the *map* has, knob or no knob, minus the same piece — the junction paint reads it, a
    rule zebra being a question of the model and not of a display toggle
    (`tests.rs::rule_zebras_do_not_follow_the_sidewalk_knob`); `band_half(i, side)` — the
    half width of the band on one side, `width / 2 + drawn sidewalk ∧ sidewalks[side]`, the
    bare kerb where the tag has no sidewalk — the per-side wedge (**Streets, sections,
    tapers**) reads it for the road and for its narrow neighbour. The crossing piece is
    pinned by `tests.rs::a_crossing_piece_between_two_halves_carries_no_sidewalk` (drawn
    exactly as the same piece tagged `sidewalk=no`), the per-side wedge by
    `a_one_sided_sidewalk_wedge_keeps_the_bare_kerb_on_the_untagged_side`.
  - **`Axis`** (`drawn.rs`) — which axis a consumer takes, named in the call rather than
    implied by which local it read. A road has three: the **OSM points**
    (`RoadLine::points`) — everything that keys a node (`junctions::node_key`): the base
    marking breaks, `Tapers::new` and `free_sides`, `row_breaks` / `crossing_breaks` /
    `join_way_ends`, `Pairs::align::continued`, the turning circles, the kerb-pocket seed;
    **`Axis::Nodal`** (the street axis after smoothing, before the stitches — its ends are
    still OSM points): `street_stations`, `merges::merges` and `merge_bands`,
    `kerb_returns`, `all_kerbsides` and `pockets::outline`, `KerbLots::frontage`,
    `tram_bands`, `across_median`, `Pairs::runs`, the car row; **`Axis::Ribbon`** (with the
    stitches — a stitched end sits on another road's axis and keys no node):
    `NodePaint::new`, `Turns::new`, `GoreRoad::new`, `splitters`, `merge_ramps`,
    `nose_fill`, `merge_axis`, `RoadIslands::new`, `wedge_ends`, `Painter::paint`, every
    ribbon. `axes(which)` hands a submodule the whole slice as borrows (a stitched copy
    exists only on a road a stitch touched); `mesh_roads` binds them once as `nodal` and
    `ribbon`. The tram band on the nodal axis is a pin, not an accident
    (`tests.rs::tram_band_follows_the_nodal_axis` — on the ribbon it would run on down every
    stitch); `tram_band.rs` decides for itself what a junction is (`near_asphalt`, 40 m /
    4 m), pinned by its own `a_band_bridges_a_junction_but_not_open_ground`.
    **The base marking breaks stay on the OSM roads**, not on `roads()`: a ring arc drawn
    at its ring's section would move the base break on an approach
    (`tests.rs::a_ring_arc_base_break_reaches_by_the_osm_width`); a driveway crossing
    stays `Highway::Path` in both and moves nothing.
    Its counters are one value too, `Drawn::stats() -> DrawnStats` (crossings, stitches,
    seams, tight corners, tapers, merges, medians, rings), nested in the report as
    `RoadReport::drawn`; the `road meshing:` line prints exactly what it printed before.
    The merge **edges** stay a `RoadReport` field of their own (`merge_edges`) — the
    ribbon lays them, not the preparation.
  - **The street axis** (`roads/axis.rs::street_axes`, stage 2 of the roads rework). The
    ribbon of every way that lies in a street ([`RoadNetwork`]) is drawn along **one curve
    per street**, not per way — per-way Chaikin pinned both ends of each way, so every
    OSM seam was a corner, and a shared node was never cut, so a through street kinked at
    every junction on a bend. Per run of a street (bridges and arches split it — their
    points are the navmesh's, and they keep the old `centerline`):
    - all three numbers come from one knob, **curve tolerance** (`RoadShape::curve_tolerance`,
      0–5 m, default 3 — how far the axis may leave the OSM points): `Curve::of(t)` gives
      `simplify = t/3`, `deviation = 2t/3`, `radius = 10t` (1 m / 2 m / 30 m at the
      default, the old `Light` step; the axis no longer has a `Strong` 60 m step, and the old
      `SIMPLIFY_TOLERANCE` 1 m / `MAX_DEVIATION` 2 m constants are gone). At 0 there is no curve at all — no
      arcs, no ring reshape — and roads off a street (paths, bridges) go through
      `centerline` with `Smoothing::Light` when t > 0, `Off` at 0 (`Curve::smoothing`);
    - the ways are stitched into one polyline and **simplified** (Douglas–Peucker,
      `simplify`), keeping the run ends, the seams and the pinned nodes;
    - every free vertex with a bend of at least `MIN_BEND` 0.5° (below it the arc would be
      a centimetre) becomes an **arc tangent to both links**: `radius`, capped by
      `deviation` from the vertex,
      floored by half the width (a smaller radius folds the inner edge — where the links
      are too short for it the corner counts as `tight corners` in the log line). An arc
      takes at most half of each link — and next to a pinned node at most what leaves the
      node `KERB_STRAIGHT` 12 m of straight edge (never less than ¾ of a short link):
      **a kerb return is laid only on a straight edge**, and an arc eating into it cost
      ~1000 kerb returns across Tula in the first cut. **A full reversal (bend exactly π)
      is left a corner** (`Corner::bend` takes `MIN_BEND..PI`): OSM has spikes where a way
      steps a metre off a node and the next way of the street comes straight back, and
      `tan(π/2)` in `f32` is *negative* (−2.3·10⁷) — the arc's reach went to −4.6·10⁸ m,
      the axis end flew half a billion metres off and the Moscow NE load hung in
      `Pairs::new` (pinned by `axis/tests.rs::a_spike_that_doubles_back_keeps_the_axis_finite`);
    - a **pinned node** — one a third **carriageway** touches (a street or a drive,
      `axis::pins`) — stays exactly in place: the kerb
      returns, the marking breaks, the stitches and the tapers all find each other by it.
      A node shared only with a footway pins nothing (stage 5): the footway ends under the
      street's asphalt and none of those four reads it, while a pinned crosswalk 15 m from
      a junction bent the axis there and left the kerb return no straight edge (sample 2,
      the 20.8 m street across the divided avenue: 7 m of straight edge, a 2 m corner). Tula:
      smooth seams 285 → 367, tight corners 274 → 145.
      The street passes it along a **straight stretch on the bisector**
      (`through_pad`, up to `THROUGH_RUN` 24 m each way, at most 2 m off the links, at
      most half of each), and the bend goes to two arcs at the stretch's ends. A bend
      under `THROUGH_MIN_BEND` 4° stays a corner (the junction's asphalt covers it, and a
      stretch would only shorten the straight edge); one over `THROUGH_MAX_BEND` 50° is a
      turn, not a through street, and stays a corner as well. A Hermite curve through the
      node was tried first and rejected: it bends hardest at the node itself, exactly where
      the kerb return needs the edge straight;
    - the curve is **cut back into its ways at the seams**, at the arc point nearest the
      seam node, and each piece takes its own way's point order. A seam is therefore no
      longer an OSM node on the drawn path, and a seam bent over 25° no longer gets a
      "kerb return" between two pieces of one street (~230 of them in Tula).
    Paths are in no street and go through `centerline` — Chaikin with every shared node
    pinned, a closed way **round the cycle** (`smooth_pinned`'s `closed`), as before. The
    **parked cars stand on the same axes** (`cars::park_cars` takes them; the game passes
    `MapData::network`, the bench and the gallery glue the streets themselves), which
    closed the old "the row walks a chord the ribbon no longer draws". Tula, release:
    285 seams passed as one curve, road meshing 78 → 87 ms (the part before the ribbons,
    axes included, 22 → 26 ms; the rest is the ~11 % more vertices of the arcs); kerb
    returns 15269 → 14982 (the seam returns above plus ~50 at junctions), sidewalk
    returns 1550 → 1423, gores 12 → 10 (two thin slivers where an approach merges almost
    parallel into the ring; the mall ring keeps all three), `navmesh: pruned` untouched —
    the axis moves no `RoadLine::points`.
  - **Driveway crossings** (`driveway_crossings`) — an `Alley` way under
    `CROSSING_MAX_LENGTH` 20 m whose **both** ends are ends of (non-bridge) streets is drawn
    as a `Street` at the narrower street's width. Found from a screenshot on проспект Ленина
    (Tula ways 4175 → 4176 → 80, `cam 4265 1357`): a service drive, ten metres of `footway`
    across the pavement, the drive again — the sand ribbon lay under the asphalt and cut a
    strip of ground across the entry. A crosswalk is safe from the rule by construction: its
    ends are on pavement footways and it crosses the carriageway with an interior node.
    The substitution is `drawn: Vec<&RoadLine>` in `mesh_roads`, and everything below reads
    `drawn`, not `map.roads`. Tula: 8.
  - **Stitches** (`stitches`) — a loose end (not closed, not a bridge or a passage, no other
    road at its node that **carries** it: a street is carried only by a street, an alley by
    anything) looks ahead for the nearest centreline of a road it may join: the closest
    point on each segment within 60° of its heading, and the heading's ray hit, scored by the
    gap to that road's **drawn edge**, ≤ `STITCH_MAX_GAP` 6 m. The drawn edge is the outer
    edge of the **sidewalk** when the road carries one (the mapped edge of
    `SidewalkProfile` under `RoadStyle::sidewalks`, the closure `Drawn::new` hands
    `network::stitches`): OSM maps a drive «to the pavement footway», which sits ~9 m off
    a 12 m street's axis, and measured to the asphalt the drive stayed 7.4 m short — it
    butted into the sand ribbon with the street showing again beyond the sidewalk (Tula way
    1309163271 at Первомайская, `cam 3420 2726`). If the end already lies inside a
    carrying ribbon, nothing is done. The stitched point is pulled back by
    `own half − target half` when the own ribbon is wider, so its round cap does not poke
    past the far edge; the segment is probed every metre against buildings and water (a grid
    of their AABBs, 32 m), and a drive that ends at a garage wall stays ended. The point is
    appended to the drawn path (`Stitches::apply`), so the sidewalk band follows too; where
    it landed — the target road, its segment and the nearest point on it — is kept in
    `Stitches::targets`, and that point is a junction node for the breaks, the paint and
    the turn paths (**Junctions** above). The
    markings see nothing of it: the end's dead-end break stands and the extension is
    past it. Tula: 39.
  - **Kerb returns** (`kerb_returns(&Drawn, scale)`, with `small_islands(&Drawn)` beside
    it — both read everything they need off `Drawn`: the nodal axes, the drawn sidewalk,
    the pair runs, the tapers, the merges) — the rounded corner of a junction. At every shared
    node, arms are collected from the **nodal** paths (pinned, so the node is a vertex of
    each): a direction to the first vertex at least 0.5 m away and the straight **run** —
    to that vertex and on through every next one within `STRAIGHT_TOLERANCE` 0.15 m of
    the arm's line. OSM puts vertices on a straight drive wherever it likes (the node
    where the pavement footway crosses it, 2 m off the street), and a run cut at the
    first of them clipped the tangent to nothing: the drive met the street with square
    corners (reported from a screenshot; 8220 → 8710 returns on Tula). **Both are measured
    on the vertices the ribbon keeps** (`meshing::ribbon_vertices`, the mask behind
    `merge_ribbon_points`, at the ribbon's own `ribbon_merge_distance` — a quarter of the
    road's width): the ribbon merges axis points closer than that, and its edge runs
    straight to the next vertex it kept, not to the merged one. On a 20 m avenue that is
    five metres, and a direction aimed at a merged vertex parted from the drawn edge by
    degrees — the fillet's side stood off the kerb as a light sliver of sidewalk (Tula,
    Сойфера × Лейтейзена, R4); a run carried through a merged vertex on a smoothed bend
    ended the arc beyond the kerb as a tongue of asphalt in the sidewalk (Халтурина ×
    Гоголевская, R8). A node the ribbon merged away itself falls back to the axis
    vertices. Arms of one class are sorted by angle, and between neighbours 25°–155° apart the
    corner of the two facing edges is found, a circle is fitted tangent to both — its
    radius **by the minor class of the pair** (`kerb_radius`, stage 5 of the roads
    rework): `MAJOR_RADIUS` 10 m between avenues (`trunk`…`secondary` and their links),
    `STREET_RADIUS` 6 m with a street (`tertiary`, residential, `unclassified`),
    `DRIVE_RADIUS` 2.5 m with a drive, a living street or a driveway crossing,
    `PATH_RADIUS` 2 m between footways, and never more than `DIRT_RADIUS` 3 m between
    two dirt roads (**Unpaved streets** above). It used to follow the widths — `0.6 × (half +
    half)`, and only 0.4 × the narrower half width for a minor entry — and on a divided
    avenue, where halves of different lane counts meet in one node, every corner came out
    a metre or two (sample 2). The wedge `[corner, tangent, arc…,
    tangent]` goes into that class's fill builder **before any ribbon** — ribbons and their
    markings then lie over it, and since it is pushed with no ribbon coords it carries no
    wear or markings of its own. One clamp: the tangent never runs past an arm's straight
    run (past the next vertex the edge has turned) **nor into a taper** (`Drawn::taper_ends`
    — the run ends where the wedge begins, since the edge there is already
    closer to the axis; gallery 08's 9 m street tapering to one lane put two spikes of
    asphalt and sidewalk out of its corners) — which is why the street axis keeps
    `KERB_STRAIGHT` next to a pinned node and pins only on carriageway nodes, and the
    paired halves keep `PIN_STRAIGHT` 16 m unshifted there (**The street axis** above,
    **Paired halves**). The old `r ≤ 3.4 × the narrower sidewalk` cap is gone: the sidewalk
    fillet below is concentric and shares the kerb's tangent lines, so the asphalt wedge
    lies inside it at any radius over the sidewalk width. With one sidewalk or none, a
    flare over the ground is exactly what a drive's kerb return looks like. It is pushed as a **fan from
    the corner** (`push_convex`), which is correct although the wedge is concave: the arc
    between the tangent points is precisely the part of the circle visible from the corner.
    Its straight sides reach `OVERLAP` 5 cm under both ribbons: a side lying exactly on a
    ribbon edge without sharing its vertices rasterizes with dropouts, a dotted light
    crack along the drive edge.
    Mixed-class arms get nothing: a grey wedge over a sand
    footway would read as asphalt spilled onto the path — and so do a dirt street and an
    asphalt one (**Unpaved streets** above). Bridges and passages give no arms
    (their paths go in as `None`). The radii are scaled by `RoadShape::corner_radius`
    (0.5–2, `kerb_returns(..., scale)`).
    - **An arm that ends in a junction ends square** (`KerbReturns::butt`, stage 5). A
      junction here is a class group of three arms or more, or of two meeting at an angle
      (25°–155° either side); two nearly collinear ends are one road continued and keep
      their round caps. Every arm of a junction that is an **end** of its drawn path gets a
      `Butt` cap in the fill and the sidewalk band (`trimmed` in `mesh_roads`,
      the same flag the taper ends use; a stitched end is not in a node and stays round).
      A round cap of a wide road ending on a narrow one stuck out past the narrow one's far
      edge as a half-disc — the blob at the bottom of sample 6 is what that looked like.
      The butt end lies on the node, inside the crossing road.
    - **The outer corner of a junction is a fan** (`outer_corner`): between two neighbour
      arms more than 180° apart — two streets meeting at a corner with nothing running
      through, a fork seen from outside — a fan from the node with the radius sliding from
      one half width to the other, what the round caps used to give for free. Pushed like a
      return, before the ribbons; its centre sits `OVERLAP` behind the node and its sides
      reach `OVERLAP` into the butt ends. The same fan in the sidewalk layer on `half +
      sidewalk`. Counted apart in the log line (`outer corners`). "More than 180°" means
      past `MIN_OUTER` 0.05° — rounding noise, no more: a street split into two ways at a
      junction with a slight kink (Ложевая at Пролетарская, Tula, gallery 08, 0.7°) leaves
      a wedge between the two butt ends on the side away from the crossing road, and at the
      old 1° threshold it showed as a light hairline. Tula: outer corners 960 → 1632,
      117 → 202 on sidewalks; the road build did not move (125.6 ms). The fan's straight
      sides reach `OUTER_OVERLAP` 0.3 m (not `OVERLAP`) into the butt ends: they follow the
      nodal axis's arms while a butt end follows the ribbon's own last link, and a degree
      between the two is fifteen centimetres at the far edge of a sidewalk band. **In the
      sidewalk layer the fan is also laid between exactly two banded arms** when both are
      way ends and the streets at the node make a junction — a street split into two ways
      where a drive without a sidewalk joins it: its band ends are butt (the street node is
      a junction), yet two banded arms alone read as a continuation and got no corner, and a
      hairline crossed the sidewalk (Oryol 04 north).
    - **A sharp fork gets a nose** (`corners::nose`, `KerbReturns::noses`): between two
      neighbour arms under 25° (`MIN_ANGLE`, down to `NOSE_MIN_ANGLE` 2°), and between
      arms up to `NOSE_MAX_ANGLE` 60° whose fillet did not fit (the straight run shorter
      than the tangent — a drive leaving a secondary at 34° with a kink 10 m out, gallery
      06), the two facing edges used to meet in a mathematical point: a spike of sidewalk
      and ground between the carriageways (galleries 04 ×3, 06, 14). A fillet cannot help
      — its tangent `r / tan(θ/2)` runs tens of metres, far past the straight run — so the
      nose is a different figure: an arc of a **small** radius (`NOSE_SHARE` 0.4 of the
      pair's kerb radius, capped at `NOSE_RADIUS` 1.5 m × the corner knob — 1.5 on
      streets, 1 with a drive, 0.8 on footways) where the edges have parted by two radii.
      It is found on the **axes** (`Arm::trail`, the nodal path from the node out to
      `NOSE_REACH` 100 m, built only for a nose), not on the arms' first directions: the
      tip lies 30–60 m from the node, where a ring exit or a curving branch has long left
      the straight line. `NOSE_STEP` 0.5 m stations along the first arm: its edge (with
      the taper — `Arm::half_at`, linear over the fitted wedge the ribbon lays), the nearest
      point of the second axis (searched in a `NOSE_WINDOW` of links around the previous
      hit) and the gap between the edges; the tip is the last station where they still
      crossed, the centre starts in the middle of the first gap of two radii and is then
      **settled** (`NOSE_SETTLE` 8 passes) until it stands one radius off both edges — a
      ring's edge bulges towards the island, and the midpoint's arc stopped short of the
      straight edge with a notch. The outline — tip, the first edge, the arc between the
      feet, the second edge back — follows bent edges, so it is **not** a fan: it goes into
      `noses` with its `Fill` (street/alley layer, unpaved, sidewalk) and is triangulated
      whole (`push_polygon`); the narrow part of the wedge keeps every `NOSE_THIN` 4th
      station only. The sidewalk gets its own nose on the band edges (half + sidewalk),
      with the same small radius — the concentric `r − sidewalk` would go negative, and the
      tip of an island is all paving anyway. Tula: 385 noses, +7 k of the road
      layers' 849 k vertices, the build unmoved (215–221 ms before, 210–213 after, the
      `map_meshing` bench). The first cut built the trail for every arm and searched the
      whole second axis at every station: +90 ms.
    - **A sharp fork of two streets gets a hatched gore ahead of its nose**
      (`corners::fork_gore`, `KerbReturns::fork_gores` → `Gores::add_forks`, painted like
      every gore by `road_paint_islands`). The nose stands where the ribbons' edges have
      parted, and before it the two ribbons overlap for tens of metres: at Tula's 8–15°
      fork of Курковая and a residential street (gallery 14) one plain tongue of asphalt
      ran 34 m from the node to the nose. On the ground the lanes part earlier and the
      space between them is hatched. The gore is built off the nose's own stations
      (`Station`: both facing edges, the gap, the half widths, the inward normals): it
      **starts** at the first station where the overlap of the edges is down to the
      narrower half width — the narrower street's axis has left the wider one's
      carriageway — widens linearly to the gap at the station where the nose was found
      (two radii), and **ends square** on the line through the nose arc's apex (running it
      along the arc laid the outline as a white bracket over the kerb, with hooks at its
      feet). Where the gore is wider than the gap, its sides go into both ribbons, each
      by its share of the half widths, so both carriageways narrow together; past the
      edge crossing it is exactly the nose's asphalt. Only at a **fork** — a node of
      exactly three street arms, the two under `MIN_ANGLE` 25° (not the nose of a fillet
      that did not fit, up to 60°): on a six-arm node the gores lay as hatched islets in
      the middle of its asphalt (Oryol, gallery 04), at Tula 06's 34° drive as a scrap in
      the throat. Only between two `Highway::is_street` arms, neither of them a ring arc (a
      ring hatches its own wedges — `Gores::add_forks` also skips a gore that touches
      one), not on footways, drives or dirt, and not shorter than `FORK_GORE_MIN` 8 m —
      a fork under 25° gives at least 12 m (a lane and two nose radii over the sine), and
      the 3–5 m ones came from bent arms inside a complex node (Oryol 04, two hatched
      triangles afloat in its asphalt).
      The asphalt under it is the ribbons and the nose: only the hatching is added
      (`a_sharp_street_fork_hatches_a_gore_ahead_of_its_nose`).
    - **A small island of three nodes is paved** (`small_islands`): three shared nodes
      pairwise joined by pieces of streets (a fork's triangle, Tula, gallery 06: sides
      17–31 m) whose inradius, less the widest half width of the three, is under
      `ISLAND_FILL` 2 m, with a perimeter under `ISLAND_PERIMETER_MAX` 120 m. Of such an
      island only a lens between the ribbons is left, and the ground showed in it as a
      light crescent; the whole triangle is pushed as asphalt under the ribbons, like a
      ring's web. A larger triangle is a real island and keeps its fill. Counted in the log
      line (`small islands`); Tula 23 — in gallery 04, 05 and 17 they lay wholly under the
      ribbons already, in 15 the same lens closed at a median nose.
    - **The junction is not unioned into one polygon.** The plan's stage 5 asked for the
      asphalt of a node as an `i_overlay` union of its ribbons; the pieces above give the
      same picture lying under the ribbons of their layer (a ribbon covers every seam
      between them), and a union per node — about nine thousand of them on Tula — would
      have cost hundreds of milliseconds of loading. The junction's paint (stop lines,
      zebras) is placed along its arms and needs no outline.
    Tula, release (stage 5): 15101 returns + 2268 on the sidewalks, 964 + 120 outer
    corners; the road layers went from 788 k to 665 k vertices (the round caps at junction
    ends were the costly part) and `road meshing` 112 → 107 ms.
    - **The sidewalk turns with the kerb** (`KerbReturns::sidewalks`), and it is an
      *addition*, not the subtraction this doc used to call impossible: the corner between
      two sidewalk bands is a **concave** notch exactly like the asphalt one, so the same
      `fillet` fills it — laid on the band edges (`half + sidewalk` instead of `half`) with
      a radius smaller by the sidewalk width. That single subtraction is what makes the two
      arcs **concentric**: a fillet's centre sits at `corner + bisector · r/sin(α/2)`, and
      pushing the corner out by `s/sin(α/2)` while taking `s` off the radius leaves it
      where it was. So the band keeps a constant width all the way round the corner, which
      is what a photo shows. Reported from a screenshot of улица Кооперативная × 2-й проезд
      Мясново (`cam 1931 4189`): the asphalt rolled out into the corner on its arc, shaving
      the light band to a sliver, and past it the band's square step stuck out onto the
      grass.
      - **The pairing is its own**, over the arms that carry a sidewalk rather than over
        the class group: a drive without one must not break the band of the street it comes
        out of (the street runs straight past it), while two streets with a drive between
        them still get their corner — the drive's asphalt is drawn over it.
      - **A radius under the sidewalk width leaves the corner square**, and that is the
        geometry, not a fallback: a drive with a sidewalk into an avenue (2.5 m against a
        3 m sidewalk) has no arc for the outer edge to follow on the ground either. Same
        for a run too short for the tangent, and for a fillet the run clamp brings under
        `MIN_RADIUS` 0.5 m — it would not show.
      - **The asphalt wedge lands on pavement at any radius**: the two arcs are concentric
        and share their tangent lines (the foot of the perpendicular from the centre to an
        edge is the same point for the road edge and the band edge), so the wedge between
        the road edges and the kerb arc lies inside the one between the band edges and the
        sidewalk arc. The cap `r ≤ 3.4 × sidewalk` that used to guard this is gone.
      - Arms with **different** sidewalk widths cannot share one concentric arc; the
        radius then takes the wider of the two (the conservative side — a smaller radius
        keeps the asphalt wedge inside).
      - **The side of a paired half has no sidewalk corner**: an arm carries a sidewalk
        per side (`Arm::sidewalk`, left and right of its heading), and where the arm lies in
        a pair run (`Pairs::beside`, the runs of **Paired halves** with two probes of slack) the
        partner's side is `None`. The corner is taken from the first arm's left to the
        second's right, so a corner facing the median gets none — it put light arcs into
        the median opening of a divided avenue.
      - **A crossing street's piece between two halves carries no sidewalk at all**
        (`Pairs::across_median`, asked once per road by `Drawn::new`, under
        `MEDIAN_CROSSING_MAX` 40 m, one end in a
        node with a half and the other in a node with its partner): it lies in the median
        opening, and its band showed as a light disc in the middle of the junction
        (sample 15).
      - Load-time only, like the rest of this module.
  - **Merges** (`roads/merges.rs`, roads plan D6 / tram-bed plan stage 2) — a node
    where a divided street becomes an ordinary one: a one-way half ends there flowing
    **in**, its pair partner starts there flowing **out**, both leave the node within 40°
    of each other (`MERGE_ALIGN`, a 20 m chord — OSM's first link may be half a metre),
    and a **two-way** way of the same `Highway` ends there leaving it the other way.
    Bridges and arches take no part. **The pair is checked by streets** (`paired`, over
    `Pairs::is_paired` and `Pairs::partners`): a
    run of **Paired halves** between the two ways themselves, or between any way of the
    one's street and the other's street — OSM cuts a half into 16–22 m ways at the node,
    and on such a piece no run forms (the first version asked the ways and found 4).
    `merges(drawn, paths, nodes, runs, network)` finds them on the drawn axes, once per
    load; Tula has **5** of the 18 nodes a tag-and-angle scan of the cache offers — the
    other 13 are forks and one-way couplets that `Pairs` does not pair, so no median is
    drawn there either. One is on a tram bed (Демидовская Плотина × Карла Маркса, 3 + 3
    lanes into 4, gallery sample 25), sample 26 is a bare one (Рязанская).
    - **Not a junction.** `kerb_returns` asks `Drawn::is_merged(road, end)`: the merge's three arms
      give each other no square ends and no fillet or outer corner — a node whose class
      group is only merge arms is skipped altogether, so all three ribbons end round, as a
      continuation does. With a fourth road at the node the junction stands, only the
      pairs of merge arms are left out. As a junction it laid square ends and an outer
      corner fan from the node, and the half's outer kerb — OSM brings both axes into the
      node, so the kerb stood at the half's own half width from it — stepped out to the
      continuation's in a spike.
    - **The merge wedge** (`merge_bands`) — per half, the continuation's half width minus
      its own (`MERGE_MIN_STEP` 0.1 m, less is nothing; a continuation no wider than a
      half gets no band), over `RoadShape::taper` × twice that difference — a taper of
      the same width step — at most `MERGE_MAX_SHARE` 0.6 of the path it runs on. That
      path is the half's drawn axis from the node **carried on through the ways of its
      street** (`from_node`, seams matched within `SEAM_SLACK` 2 m — the pair alignment
      moves a piece's end by a metre), so a 16 m last way does not cut the wedge to ten
      metres. The side is geometric — outward is away from the partner's axis 20 m out —
      not a run's `left`, which a short last way does not have. The path is resampled
      every `MERGE_STEP` 2 m from the node and the band runs from the half's kerb
      (`MERGE_OVERLAP` 5 cm under its ribbon) outward to a reach that falls by smoothstep from the continuation's half width at the node
      to the half's own at the wedge's end, into `streets` **before** the ribbons; the
      same band plus the half's sidewalk width goes into `sidewalks` where the half has a
      tagged sidewalk on its outer side. Each side has its own step, so a kerb already in
      line with the continuation gets nothing — the wedge is **by kerbs**, not about the
      axis. The merges are `RoadReport::drawn.merges`, the bands `RoadReport::merge_edges`.
    - **The paint runs through** — three pieces, each measured against sample 26, where
      the halves' lines ran into the node as two solid lines closing in a V, the
      continuation's started past a 13 m gap, and the double solid ended at the lawn:
      - **No breaks at a pure merge** (`Merge::pure` — no other carriageway at the node).
        `node_paint` saw three streets at a node (each half is a street of its own, the
        continuation a third), so all three yielded and broke by the neighbour's half width
        plus a metre (8.1 m on the halves). `NodePaint::new` reads `Drawn::merges`: a pure merge
        node is dropped from the junction list, its base breaks leave all three roads and a
        `solid` break goes on each — the continuation's axis is solid there. No zebra, stop
        line or turn wear. A merge on a junction (sample 25, five roads) breaks as any
        junction does.
      - **The ramp** (`merge_ramps` → `paint::MergeRamp`, passed to `Painter::paint`): per
        way of each half, from the node on through its street (`walk`, the ways and how far
        each starts from the node). At the node the half's frame is **its side of the
        continuation's** — from the continuation's outer lane edge to its axis, on its grid
        (a node on the axis for even lanes, half a lane off for odd); the grid is placed so
        the half's **outer** line stays outer, which leaves the extra lanes at the pair's
        side. The frame goes from there to the body by smoothstep, like the kerb band, over
        `RoadShape::taper` × the largest shift (twice the kerb step, twice the grid shift,
        at least a lane) and at most 0.6 of the path. The partner-side edge shrinks on the
        **far half** of the ramp only: the extra lane ends there and its line fades away
        from the axis — shrinking with the rest put two faint dashes right against the
        double solid. The whole grid moves as one, so lines never cross. The painter inserts
        a vertex every `RAMP_STEP` 2 m on the ramp. Рязанская (3 + 3 into 4): each half's
        median-side line fades, the outer one runs into the continuation's lane line.
      - **The axis** (`merge_axis` → `Painter::paint_merge_axis`, double from four lanes,
        single solid below): from the node up the middle between the halves, to the nearest
        `MedianEnd` of the pair's median within `AXIS_REACH` 60 m — a paved median's
        midline tip (the median's own double solid no longer breaks at a pure merge node, so
        the two meet), or a lawn kerb point less `NOSE_CLEARANCE` 1 m. With no median it
        stops where the halves' kerbs part (axes further apart than the two half widths).
      - **Asphalt up to the lawn's nose** (`nose_fill`): where the halves' kerbs have parted
        and the lawn has not begun, the ground between them was bare — a wedge before the
        nose at Рязанская (sample 26). The outline between the halves' axes, from the node
        to `NOSE_FILL_BEYOND` 4 m past the nose's tip, less the lawn kerbs within
        `NOSE_FILL_NEAR` 10 m (the grass lies under the streets, and a cut square across
        the tip left specks of ground at the kerb's corners), goes into `roads` before the
        ribbons. Only before a lawn: a paved median is asphalt to the node anyway.
      - **The ruts follow the ramp.** A pure merge node takes the halves' and the
        continuation's `asphalt` breaks off too, and the half's fill gets the same ramp as a
        lane profile: `MergeRamp::lane_profile` samples `frame_at` every `RAMP_STEP` along
        the way (pairs at the way's ends when the ramp crosses them), and
        `MeshBuilder::set_lane_profile` puts a ribbon vertex on each pair
        (`PROFILE_MERGE` 0.1 m) and interpolates the frame per vertex, fans included —
        `push_ribbon_shaped` only; `set_lanes` / `set_lane_taper` clear the profile. So
        the ruts run between the paint lines through the node instead of fading 8 m short.
        The asphalt's profile is the paint's with one difference: on the pair side it is
        never narrower than the body. The half's ribbon runs on to the node over its
        partner, and past the paint frame squeezed toward the axis it was asphalt with no
        ruts — a smooth patch by the continuation's axis (sample 26); near the node the
        grid carried on is the continuation's own, so the ruts fall where the partner's do.
    - **Советская × Коминтерна is not a merge**, whatever the tram-bed plan assumed: all
      four ways there are one-way — the pair turns east and goes on as 3 + 4 lanes with a
      lawn, while the tram leaves for Коминтерна. The bed ends there square at the lawn's
      nose (`bed_caps`), and that is the whole of it (sample 24).
- **RoadStyle and RoadShape** — three resources behind the roads, split by **how a change
  reaches the map**: `RoadPaintStyle` (**Markings — the paint layer** above) is uniforms
  only, a drag rebuilds nothing; `RoadStyle` is toggles, each click one rebuild;
  `RoadShape` is sliders that move geometry, so the map follows a **settled** copy.
  - **RoadStyle** (resource, BRP-writable, persisted; toggles in the Roads and Road paint
    sections, `ui/roads.rs` / `ui/road_paint.rs`) — what gets drawn; any change reruns
    `rebuild_roads` (despawn `RoadLayerTag` layers, respawn from the unchanged `MapData`).
    Five toggles: **sidewalks** and **markings** (both on) are described above,
    **crossings** (`CrossingMode`: `Off` / `Osm` / `Generated`, the default) and
    **stop_lines** (on) in **Junction paint**, **arrows** (**off**, at the user's request) —
    the lane arrows, their own toggle since stage 8, no longer under `markings`. Stage 8 took out `join`, `smoothing`
    and `casing`; old keys in `settings.toml` are ignored silently (bevy_settings applies
    only the fields the type has). The join is the constant `ROAD_JOIN` = `RoadJoin::Round`
    (the `Square`-only branches — no tapers, no kerb returns — went with it); `RoadJoin`
    and `Smoothing` stay for the tree-row band's own Joins / Smoothing rows and for rails,
    the tram and water (`Smoothing::Light`). The dark road/alley **casing layers are gone**
    (`alley_casings`, `road_casings`, `Z_ALLEY_CASING`, `Z_ROAD_CASING` and their colours),
    so `mesh_roads` yields **24 layers**: twelve ribbons (with `unpaved_roads`, **Unpaved
    streets** above, and `road_verges`, **Sidewalks**), the wide verges' lawn as meadow
    and as yard grass (`road_verge_lawns`, `road_verge_yards`, **Sidewalks**), the ring islands' lawn and their
    grass without the rim (`ring_islands`, `ring_island_grass`, **Roundabouts**) + eight
    paint layers. `bridge_casings`
    stays — it is the bridge curb (**Bridge layers** below); `footprint::casing_width`
    stays for the tree-row band and the planting index.
  - **RoadShape** (`map/roads/shape.rs`, group `road_shape`; five sliders in the Roads
    section, table `ui::shape_knobs()` shared with the gallery) — the ranges sit beside it
    and the accessors clamp on read: **lane_width** 2.75–3.75 m (3.3), **taper** 5–20 m per
    metre of width difference (10), **curve_tolerance** 0–5 m (3; 0 = the axis on the OSM
    points), **median_gap** 1–6 m (3; paved median with a double solid up to it, lawn
    wider), **corner_radius** 0.5–2 (1.0, a multiplier on the per-class kerb radius table,
    `kerb_returns(..., scale)`). Signatures carry it: `mesh_roads(map, style, shape)`,
    `mesh_cars(bucket, style, shape, map, layout)`, `axis::street_axes(.., &RoadShape)`.
  - **RoadShapeOnMap(RoadShape)** is what the map follows: `settle_road_shape` copies the
    slider value after 0.35 s of quiet (the `SunStyle` → `SunOnMap` trick),
    `seed_road_shape` at Startup, `track_pref::<RoadShapeOnMap>`. `roads::rebuilds_on` =
    `RoadStyle` | `RoadShapeOnMap` | `SunOnMap`; `cars::rebuilds_on` reads
    `RoadShapeOnMap` instead of `RoadStyle`.
  - **Lane width is a world reload**, not a rebuild: the parse reads it (**Sections**). The
    parse gets it as an **argument** — `ParseKnobs::lane_width`, built by
    `loading.rs::start_job` from the `RoadShape` knob and handed to the load thread
    (`parse.md`, the parse seam). The paint and the ruts still read a process global —
    `shape::lane_width()` / `set_lane_width()`, an `AtomicU32`, the same way as the sun:
    threading it through `Painter::paint`, `lane_frame`, `wedge_frames`, `merge_ramps` and
    `arm_lanes` would widen five interfaces inside pure functions for one number.
    The map keeps what it was parsed with, `MapData::knobs`, and that snapshot is the one
    source both ends read: `shape::adopt_lane_width` writes the global from it on
    `OnEnter(Playing)`, chained before `surface::retune_surface_materials` and before
    `WorldInitSet::Navmesh` (so before any road layer is meshed); `city.rs::reload_world`
    fires on `lane_width_moved` (the settled `RoadShapeOnMap` width differs from
    `MapData::knobs.lane_width`) — same city, the camera stays. The one-lane Y legs of a
    roundabout (`leg_sections`) are sections of the parse too and take the snapshot, not
    the global. Paint and turns read the global (`BIRTH_FADE` is half a lane), and the
    surface shader gets it as `SurfaceParams::lane_width` (`surface.wgsl` no longer
    hardcodes 3.3) — the retune on `OnEnter(Playing)` is what makes the ruts follow a new
    width.
  - **Smoothing off the street axis** — a bridge or a path goes through `centerline`
    with Chaikin corner-cutting (`Smoothing::Light` when the curve tolerance is above 0,
    `Off` at 0): only bends over `MIN_SMOOTH_ANGLE` (10°) are cut and the cut length is
    clamped to the road width. `passage` roads are never smoothed — their endpoints are
    pinned to building outline vertices that `arch_openings` looks the arch up by — and a
    node shared with another road is never moved.

  Smoothing works on a **copy** — `RoadLine::points` and `width` are load-bearing for the
  navmesh (`bridge`/`passage` carves), arches, tree planting and the entrance generator,
  and none of them may shift because the drawing changed. **The rule itself lives in
  `map/smooth.rs`**, not in `roads.rs`: `Smoothing`, `smooth_path`, the `pinned` variant
  `smooth_pinned` and the private `chaikin` with its two constants. Six modules read it —
  roads, rails, the tram, the waterways, the parked cars and the tree-row band — and it
  used to sit in the middle of `roads.rs`, between the road palette and the bridge
  shadows, which is why the enum was called `RoadSmoothing`. The **variant names**
  `Off`/`Light`/`Strong` and the field name `smoothing` are values in `settings.toml` and
  may not be renamed; the type and the module may. `centerline` stays in `roads.rs` — it
  is the road wrapper that adds the arch and shared-node pins.
- **Bridge layers** (`map/roads/bridges.rs`, same `RoadLayerTag`) — a road with `bridge` leaves
  its class layers for the **three** `bridge_shadows` (`Z_BRIDGE_SHADOW` 2.05) +
  `bridge_casings` (`Z_BRIDGE_CASING` 2.1) + `bridges`
  (`Z_BRIDGE` 2.2). The **shadow** is the deck's own band, offset along `shadow_dir()` by
  the deck height through the usual `shadow_length_scale()`, on a blended material of its
  own (the flat white one would eat the vertex alpha). Nothing else produced it, because
  the ground shadow layer only knows buildings, and a bridge over the river is the most
  visible thing on the water. It sits **under** the deck and **over** what the bridge
  crosses — except what the z ladder draws above bridges: the rails (a tram on a bridge
  must stay visible), the tram line, wagons, parked cars, fences.
  Eight decisions in `map/roads/bridges.rs` (`Bridges`, `probe_underneath`, `bridge_height`,
  `bridge_shadow_path`, `bridge_penumbra`, `push_bridge_shadows`) make it read instead of
  lie, and every one of them was a bug report first:

  - **A bridge is a connected chain of ways, not one way** (`Bridges`, `BridgeSpan`).
    OSM cuts a bridge into pieces at every tag change: Tula's **61 bridge ways are 56
    bridges**, and 8 of those ways are glued into 3 — the Оружейный мост interchange
    424 + 95 + 299 m (818 m, a fork: three ways meeting at one node), a footway
    34 + 129 + 22 m (185 m), and 39 + 4 m. Per-way the shadow lied twice over: the ramp
    below fired at every *internal* node, where the deck is at full height, so the shadow
    dipped under the deck at the interchange's fork and at both joints of the footway;
    and `SHORT_SPAN` was applied to the piece, so a 22 m middle section of a 185 m bridge
    was interrogated as a footbridge and could lose its shadow outright.
    - **Glued end to end, not by `ways_joined`.** That predicate answers «do these two
      polylines touch anywhere» — it is what decides whether a road joins a bridge for
      the curb — and here it is wrong in both directions: on Tula it would fuse **two
      pairs of footbridges that merely cross**, and a way-end landing in the *middle* of
      another bridge (Tula has none, but nothing in the data forbids it) would turn a
      branch into a continuation. What is matched is endpoint against endpoint, at the
      same `JOIN_EPSILON` (0.5 m) — the tolerance is the price of the projection, and
      that part is shared.
    - **The geometry is not glued — the arithmetic is.** Two reasons, and either alone
      is enough: pieces of one bridge may differ in width (one polyline cannot carry
      two decks), and **three way-ends meet at one node** on Tula's own fork, the
      Оружейный мост interchange above, where the three pieces are not a chain at all. So a
      way keeps its own points and its own width, and takes from its bridge two numbers:
      the **span** (the sum over the whole connected component) and, per end, the
      distance to the nearest **free end** through the neighbouring ways. A fork costs
      that model nothing: the distance runs along the shortest of the three branches,
      the node is not a free end, and the deck there stays up.
    - Distances come from relaxing over the way-graph until it converges (tens of edges
      — a priority queue would be ceremony). A component with **no** free end at all — a
      ring flyover — comes out at infinity, i.e. fully raised everywhere, which is right:
      it never sits down on the ground.
  - **A span under `SHORT_SPAN` (35 m) has to prove there is a gap under it**
    (`probe_underneath` over `Underneath` every 2 m — water outlines, watercourse
    channels, rails; **never roads**, because a road is exactly what an approach
    embankment runs along). The question is put to the **whole chain** and answered once
    for it, so a piece over dry land next to a piece over the river keeps the river's
    answer. Proportional height was not enough on its own: the western
    approach to the Упа bridge on Советская улица is four ways of 23–30 m carrying `bridge=yes` and
    `layer=1`, and on the ground it is solid fill, which no tag distinguishes from a
    span. A long way is never asked — there is no 100 m embankment — which also keeps the
    probe off the bridges that would cost the most to test. `Underneath` precomputes an
    AABB per outline for that: the probe is per 2 m of deck and a city carries up to a
    thousand water outlines.
  - **The height follows the span** (`bridge_height` = `SPAN_TO_HEIGHT` 1/8 of the length,
    capped at `BRIDGE_HEIGHT` 6 m, so a span over 48 m is at the ceiling). It was a flat
    6 m, and OSM hands `bridge=yes` to far more than spans: embankment steps, the pavement
    beside a flyover, a 4 m plank over a storm drain. A 6 m shadow under a 20 m path that
    stands on level ground is the loudest lie the map can tell, because **a shadow reads
    as height** and nothing else on the map states it.
  - **The centerline is densified to `SHADOW_STEP` (2 m) first** (`densify`). The ramp
    below lives in the vertices, and **42 of Tula's 61 bridge ways carry exactly two
    points** — the 137 m Упа bridge on Советская among them — so every vertex was an end, the rise was zero
    everywhere, and the shadow landed exactly under the deck, i.e. nowhere. The ways that
    did have vertices (a 571 m flyover with 42 of them, one per ~14 m) got a shadow that
    stepped from vertex to vertex in visible teeth.
  - **The offset ramps from zero at each free end of the chain** (smoothstep over
    `RAMP_SHARE` 0.25 of the **span** capped at `RAMP_MAX` 25 m) — at the abutment the
    deck is on the ground and casts nothing, and the constant-offset version poked a dark
    band past the deck onto the street that joins it, which is exactly where a bridge
    meets a road and where the eye is. The distance a point measures is its own arclength
    **plus what lies beyond its way's joint** (`BridgeSpan::from_start` / `from_end`), so
    an internal joint never ramps and the rise runs continuously across it — both ways
    read the same number at the node they share.
  - **The rise is clamped by the span left ahead of the shadow** (`room / |offset·t|`,
    `room` measured through the chain to the end the offset points at). The ramp is not
    enough on its own: a
    smoothstep climbs faster than the arc advances, so on a short bridge the shadow
    overtook its own end anyway and lay past the abutment as a wedge — the artifact that
    survived two rounds of fixing the ramp.
  - **The band is `SHADOW_SPREAD` (1 m) wider than the deck on each side**, tapering with
    the same rise. A plate's shadow is its silhouette *translated*, so the offset splits
    into a crosswise part (the visible strip beside the deck) and a lengthwise one (the
    band slides along itself and stays under the deck) — a bridge running along the sun
    azimuth has no crosswise part at all and showed nothing, which is what the footbridges
    over the ponds did. The fringe is the water the deck cuts off from the sky plus the
    railing shadow and the slab's thickness, and unlike the offset it barely depends on
    the height.

  - **The edge is soft, and the width comes from the span** (`bridge_penumbra`). Every
    other shadow on the map fades at its edge — buildings `PENUMBRA_WIDTH` 1 m, cars and
    fences `SHADOW_BLUR` 0.35 — and the deck's was a hard cut. The law is the one the car
    already states: «a house's is a metre, three times ours, **because its shadow is three
    to ten times longer**» — so the width is a share (`PENUMBRA_SHARE` 0.3) of the
    shadow's *own* length, i.e. of `bridge_height(span) × shadow_length_scale()`, not a
    constant. A bridge is the tallest thing casting a shadow here and the most varied (a
    2 m plank over a pond against a 6 m flyover), which is exactly why a constant is
    wrong. The ends of the clamp are the two numbers already on the map:
    `PENUMBRA_MIN` 0.35 (nothing here has a softer edge than a parked car) and
    `PENUMBRA_MAX` 1.0 (nothing has a softer one than a house). So a 16 m footbridge gets
    0.36 m, a 40 m bridge 0.9, anything over 44 m the ceiling. The ends do **not** ride
    the sun — they are constants of neighbouring layers — while the share does, through
    `shadow_length_scale()`: a low sun lengthens the shadow and softens its edge.
    - **It tapers by `rise`, not by direction.** Buildings, cars and fences taper theirs
      by `direction · shadow_dir()` because those objects stand *on the ground*: their
      shadow is hard where it meets the object and soft at the far end. A deck is a plate
      *in the air*, so every point of its outline casts from the same height and the
      penumbra is uniform all the way round — except at the abutments, where the deck
      sits down and `rise` is zero. A directional taper would put a metre of soft shadow
      past one deck end onto the street, which is the contact skirt the buildings removed.

  Because the width varies along the band it is **not** a `push_ribbon`: the rails come
  from `miter_offsets` scaled per point (`shadow_edges`), and the core is the closed
  contour of those rails — the left one forward, the right one back — handed to the union
  below and pushed as `push_polygon` per union shape. The penumbra is two quad strips along
  those same rails
  (`push_quad_gradient`, opaque on the rail, alpha 0 at the outer lip), and a segment
  whose rise is zero at both ends emits none.

  - **The miter is taken on the deck's centerline, before the offset** — it is computed
    in `bridge_shadow_path` and travels on `ShadowPoint::normal`, and `shadow_edges` only
    stretches it by the local half width. A plate's shadow is its silhouette *translated*,
    so the band's cross-section stands across the **deck**; mitering the already-offset
    path instead makes it stand across the curve the band bends into, and the two differ
    exactly at the abutment, where the offset starts from zero: the displaced centerline
    leaves the deck end sideways, its joint normal is tilted, the butt edge comes out
    skewed, and one of its corners runs past the abutment by `half width × sin` of that
    tilt. On the map that is a dark tongue poking out from under the end of the curb, on
    the side the sun throws to — 0.75 m on the service bridge over the Упа
    (way 160142247, 22.1 m, `cam 6164 3393` at zoom 0.05), with the other corner cut the
    same amount *into* the deck, where the curb hides it. Reported from a screenshot;
    pinned by `the_shadow_never_runs_past_the_abutment`, which asserts that no vertex of
    the layer — core, penumbra or all — lies past either end of the deck.

  **The cores of all bridges are unioned** (`i_overlay`, NonZero — the buildings' and the
  fences' construction), and that is measured, not precautionary: OSM maps a road bridge's
  pavement as a **parallel way of its own** carrying the same `bridge=yes`, and on Tula
  **28 pairs of different bridges** have shadow cores that overlap. In a translucent layer
  that reads as a strip of double darkness running the whole length of the bridge.
  **The penumbra is laid per bridge, before the union**, and cannot be moved after it: its
  width is `rise`, the local height of the deck over the ground, and the union output is
  contours with no `rise` on them. Dropping the taper instead is the option that costs
  more (see the previous bullet). Two neighbours' bands may therefore overlap each other —
  the price the building shadows already state and accept, since both fade to zero. A penumbra
  buried in a neighbour's core is not laid — **and that probe is filtered by the cores'
  bounding boxes**, which is not a micro-optimisation but the difference between 2.5 ms
  and 26 ms on Tula, i.e. between a fifth of the road layer and nothing. The probe runs
  per quad of the band (the path is densified to `SHADOW_STEP` 2 m, so the Упа bridge
  alone is ~70 of them, twice over for the two sides), and unfiltered it walked the ring
  of **every** bridge in the city, each ring twice the band's own length. Only neighbours
  on one deck ever overlap — 28 pairs of 56 bridges — so all but two of those rings are
  answered by their box. Same reject as `probe_underneath`'s `Underneath` a few bullets
  up.

  About the deck itself: a light concrete **curb** (`BRIDGE_CURB_COLOR` 0.80, 12% of the width
  clamped 0.8–2 m) under the fill in the class color — a parapet over the asphalt-grey
  deck. The 2GIS look — the curb bands along both deck edges are what makes a bridge read
  as a bridge, so the curb draws **always** — it is the one casing-like layer left
  (`bridge_casings`); the dark road/alley casings were removed in stage 8.
  Curb caps are always `Butt` (`push_bridge_curb`) — the deck ends
  in a square cut; a `Round` half-disc would poke a curb
  tongue past the bridge end. The deck sits above `Z_ROAD` so an overpass covers the
  street it crosses and above `Z_RAIL` so a road bridge over the railway covers the
  track (**Rail layers** in `layers.md`), and below `Z_TRAM` so a tram on the bridge
  stays visible; curbs
  sit below the fills so a junction of two bridge ways is never cut by a
  curb band. Street and footbridge fills share one mesh — bridge-over-bridge overlap
  is push order, rare enough not to warrant four layers. Rails carry no bridge flag —
  rail bridges are out of scope. The curb is not just paint: the navmesh blocks the
  same bands (see **Bridge curbs are impassable** in the navigation-deep skill).

  **The owner is `Bridges`** (`roads/bridges.rs`) — a value `mesh_roads` makes once and
  calls from inside its fill-order loop, not a layer module of its own. `push_deck(road,
  points, line)` lays the curb and queues the shadow band; the **fill** is still pushed
  by the loop, into `Bridges::fills()`, because a deck has to stay in `fill_order` and
  take its street's lane frame and asphalt breaks (the ribbon attribute carries both —
  pinned by `deck_fill_carries_its_streets_lane_frame`), and bridge-over-bridge is push
  order. A layer module with a `deck` closure was the rejected shape: the closure would
  have repeated half the loop body. `layers()` hands back the three `LayerMesh`es with
  their z and materials (the shadow `Blend`, the curb `Flat`, the deck the streets'
  `Surface`), the shadow cores unioned there; `count()` is the
  `BridgeReport` on `RoadReport::bridges` — ways, bridges (chains) and the ones casting
  a shadow; v15 caches: Tula 91 / 86 / 73, Berlin 429 / 325 / 278, Kaluga 55 / 52 / 48,
  Ryazan 84 / 80 / 70 (the «61 ways, 56 bridges» above was counted on an older Tula
  extract). The width of the curb comes from `footprint::bridge_curb_width` through
  `RoadLine::curb_reach` and stays in `footprint` — it is the seam with the navmesh.
- **Asphalt wear** (`surface.wgsl`, `SurfaceParams::wear`, on `SurfaceKind::Street` only)
  — what keeps a road from being one flat tone, in the **lane frame** so it follows the
  lane rather than the compass: **wheel ruts** — a polished band `RUT_OFFSET` 0.85 m
  either side of each lane's middle (a car's track is 1.5 m), `RUT_SIGMA` 0.32 m wide
  (both in `roads/paint.rs`, the one owner: the shader reads them as
  `SurfaceParams::rut_offset` / `rut_sigma`, the turn paths as the same fields of
  `PaintParams`, so the two rut profiles cannot drift apart),
  amplitude `SurfaceParams::wear` — the **Wear** knob (`RoadPaintStyle::wear`, 7.5 % by
  default, was the shader constant `RUT_AMP`). The lane is `fract` of
  `across_from_grid_node / SurfaceParams::lane_width` (the **lane width** knob, 3.3 by
  default — the city's one lane width, set by `retune_surface_materials`), inside the
  carriageway bounds `low..high` with a 0.3 m fade at each — the attribute layout of
  **Markings — the paint layer** above. So the ruts stand on the very grid the paint
  lines do, a taper included.
  - **The ruts are zero-mean**: the band's share of the lane (`2·σ·√(2π) / lane width`)
    is subtracted from it, so between the ruts the asphalt is a touch darker and the lane
    on average is exactly `ROAD_COLOR`. Added as a plain brightening, a marked street was
    ~3 % lighter than an unmarked drive of the same colour, and the two read as different
    asphalt wherever one ran into the other — reported from a screenshot as a colour
    step at the junction.
  - **Kerb dirt was the second effect and is gone** (7 % darker over the outer 0.7 m).
    A junction gap comes only from two *carriageways* meeting; a service drive or a
    residential lane under 8 m joins a street with no gap, and the wider street, drawn
    on top, laid its dark kerb band straight across every drive mouth — the same report.
    Bringing it back needs a gap the drive can make without cutting the lane dashes.

  **Repair patches were the third and are gone.** A 6 m cell of the *world* grid was
  hashed and, above a threshold, darkened whole; the `smoothstep` softened the hash, not
  the shape, so the patch's edge was exactly the cell boundary. What that draws is a
  chequerboard oriented to the compass: right angles, two chosen neighbours fused into one
  block, and a staircase of squares across any street that is not axis-aligned. It is
  literally the construction **A cell grid places a feature, it never *is* the feature**
  was written against after the same mistake on the roofs, and the claim standing here
  that wear "already follows" that rule was false. Taken out by the author's call on the
  picture. The way back in, if it is wanted: the cell must be the *lane's* cell (the
  ribbon frame, so the grid turns with the road), and inside it the patch must be a
  rectangle smaller than the cell with a jittered centre and size, a crisp saw-cut edge
  and a seam — not a fill of the cell.

  The ruts fade by `visible(...)` like the rest of the surface texture, by their lane
  pitch.

  **The rut lines overlay** (`roads/ruts.rs`, Debug → Overlays → **Rut lines**,
  `DebugRutLines`; gallery row `Ruts` / `ROADS_RUTS=1`) draws where both sources of wear
  lie: every lane's axis (orange) with its two wheel lines at `RUT_OFFSET` (pale orange) —
  the shader's ruts — and the junction turn curves and tails of `JunctionWear` (green) —
  the paint layer's. The lines come out of the same build: `mesh_roads_with_ruts` records
  each ribbon's axis and body `LaneFrame` where it calls `set_lanes`, and moves
  `turns.wear` in after `paint_turn_wear`; the game keeps them as the `RutLines`
  resource (`rebuild_roads`, `spawn_map`) and `ui/debug/overlays.rs::sync_rut_overlay`
  follows that resource, so a shape knob moves the overlay with the ruts. `mesh_roads`
  is the same build without the third return, for the tests and the bench. **Known
  simplification**: on a taper wedge and a merge ramp the shader's frame drifts
  (`set_lane_taper`, `set_lane_profile`), while the overlay draws the body frame over the
  whole axis, so there its wheel lines sit up to part of a lane off the painted ruts. Raw
  OSM's drawing level lays no ruts and records none.

  **And they fade out in a junction gap**, by `smoothstep(0, WEAR_FADE, to_break)` over
  the same `to_break` the lane dashes use — the second component of `ATTRIBUTE_RIBBON`,
  negative inside a gap. `WEAR_FADE` is 5 m, not the dashes' 1 m: over a metre the ruts
  stopped across the lane at the junction edge like a seam.
  Reported from a screenshot of a four-way crossing: the roads are independent overlapping
  ribbons, so each was drawing its own wear across the other. The kerb dirt (since removed,
  above) was then the louder half — a dark band along a street's edge carried straight
  over the crossing street's asphalt, where there is no kerb — and the ruts the subtler,
  two lanes' polished bands meeting at right angles in the middle of the junction. Neither
  is a thing that happens: traffic fans out over a crossing and polishes nothing. Since
  stage 5в that is only half the story: a junction's **leading** road keeps its ruts through
  the node (its asphalt breaks are dropped, `NodePaint::asphalt`), and what the rest of the
  traffic polishes — each maneuver's own pair of ruts — is laid by the **Turn paths** above
  in a layer of its own, not by this shader. The gate
  costs one `smoothstep` on the wear amplitude `w`, so anything added to the block later
  fades with the ruts. The block is
  gated on `high > low`: only a ribbon with a lane frame has that (`roads::road_lanes` —
  every carriageway, a one-lane street included, whatever the Markings toggle says; the
  wear has its own knob now). Water's ribbon carries `half width, 0` there and polygons
  zeros, so neither passes; an areal fill of the same `Street` material carries no
  ribbon and stays flat — the **parking lot** among them, which shares the material and
  would otherwise have grown ruts across its stalls. Before the paint layer the gate was
  `lanes >= 2` off the markings code, so wear reached exactly the roads the lane lines
  reached and switched off with Markings.

## The junction gallery — `examples/demos/roads`

`cargo run --example roads` shows a city's typical road junctions in a column —
twenty-eight for Tula (the per-side taper added `28_narrowing_t`, Крестовоздвиженская
площадь ending in two lanes at a T where Союзная goes on in one and a lane joins from the
side — the taper on the free kerb of **Streets, sections, tapers**;
`27_tram_through_junction`, Советская × Красноармейский, the tram between the halves
crossing a junction — the **Tram band** bridge; the tram-bed and merge plans added three:
`24_tram_bed_end`,
Советская at Коминтерна, where the tram turns off and the bed ends square at the nose of
the lawn — all four ways are one-way and the pair goes on, so **not** a merge (**Tram
bed**); `25_divided_merge`, Демидовская Плотина's halves ending on a two-way street at a
node with a crossing street, and `26_pure_merge`, Рязанская's halves with no other arm —
**Merges**; stage 7 added three: `21_turn_pocket`, Пушкинская at проспект Ленина, a
two-way tertiary going from 2 + 1 lanes to 3 + 1 before the signals,
`turn:lanes:forward` `left|through|right` — the new lane born in a wedge and the lines
ending at the stop line (it first stood on проспект Ленина's pocket, the same window as
`15_skew_avenues_link` five metres away, and was moved; it has no Yandex reference yet,
nor have 25–28); `22_lane_change`, two-way
secondary улица Болдина going from two lanes to four at a seam — the two-way twin of
`16_lanes_taper`; `23_s_curve`, Путейская улица's 200 m right-then-left bend in one
way — the smoothed axis carrying the ribbon, sidewalk and dashes; the twentieth, `20_roundabout_arcs`, is the secondary ring of six arcs — the
"egg" of **Roundabouts**; `04_roundabout_large` is the primary one; the nineteenth,
`19_offset_joins`, is Tsiolkovsky street with two side streets
joining from opposite sides 17 m apart — one cluster of **Junction paint**; the sixteenth, `16_lanes_taper`, is a one-way primary going from four lanes to
two at a pure seam — the taper of **Streets, sections, tapers**; the seventeenth,
`17_ring_gores`, is the mall ring the plan's acceptance names — three hatched gores and
the boulevard's double solid line must survive every stage; the eighteenth,
`18_lawn_median`, is Советская улица with two tram tracks in the 5 m between the halves
and a lane into one of them — each half widened to the middle, the tram band, the double
solid between the tracks, the sidewalk only outside (**Tram bed**); the stem is kept from
before the tracks were drawn, since the Yandex shot is keyed by it): crossings of avenues (square and skew), of an avenue and a street, of a divided
avenue and a street, of private-sector streets and of yard drives, T's into an avenue and
into one half of a divided one, a fork round a triangular island, a roundabout, five
arms, a drive into a street, a street that narrows, a sharp bend, a dead end — each with
its full address and **game coordinates** in a caption to the left of its window.
Ryazan, Kaluga, Oryol and Rostov have columns of their own (`ROADS_CITY=<slug>`, six
samples each with a Yandex reference, plus Ryazan's `07_signals_across_pair` without
one — the stop lines' queue room): the types Tula lacks or draws differently —
Rostov's one-way grid, crossings of two divided avenues, a T into a six-lane two-way
street, unpaved and gravel private-sector crossings, rings of every size (a three-lane
primary, two rings side by side, a narrow ring round a square, a park ring, an oval one,
a five-arm mini ring, a closed one-way loop without `junction`), six arms in one node.
**Belgorod** has two, both from the author's reports and both drawn wrong when added:
`01_divided_merge_junction` (улица Попова × Павлова — the halves of a divided secondary
meet in the signalled junction node itself, the east half kinked in OSM right at the
node; the game bulges that half and zebras only the west one, Yandex keeps both halves
straight with a paved strip between them up to one zebra across the arm) and
`02_links_into_avenue` (улица Победы — two one-way `primary_link`s leave one node of a
four-lane primary: not a merge, the class differs; the entry's lane runs into the solid
line by the hatched gore).
**The column is a list of junction *types*, one sample per type, with no target count**;
the rules for adding one — one type once, readable in the window at a glance, flat road
junctions only (no level crossings, no multi-level interchanges, no arch through a house:
from above the roof covers the road and it reads as a house drawn on it), checked by eye
and against the game — are written out in `examples/demos/roads/samples.rs`. They were
learnt the hard way: thirty nodes picked from the cache by their arm count came down to
fifteen once looked at, because a "T" or a "five-arm" node is routinely a plain crossing
in the frame.
It is the one gallery whose input is **OSM data, not hand-written geometry**, so it is the
place to look at a road-network defect end to end:

- **A sample is a window of the game's own Overpass cache, cut at start-up** — the
  gallery reads `assets/osm/<city>_…_vN.json` once (`download::city_extract`: the cache,
  or the game's loader when there is none — same path, same file), deserializes it and
  cuts every window out of the elements with **`map::osm::crop`** (`Cropper`, indexes
  built once; `GeoRect::around`), with no JSON in between. So **a sample always equals the
  game**: raise `QUERY_VERSION` and the new tags and nodes are in the samples from the next
  start. It used to be fifteen frozen files cut by a Python `tools/osm_crop` from the v14
  cache; every query bump would have left them silently behind the game — exactly what the
  gallery exists to catch. The price is that a sample is no longer immutable: a
  re-downloaded cache may bring a mapper's edit into the junction.
  Each sample goes through the game's `parse_response` with all nine finishing passes, then
  the game's `mesh_*` and `spawn_*` doors (surfaces with the parking layout, roads,
  buildings, fences, rails, tree rows, trees; near zoom buckets). **No parked cars, by the
  author's call**: the gallery is about the carriageway and the junction, and a kerb row
  covers exactly those — the edge, the kerb return, the markings by the crossing. The
  gallery owns no geometry at all. `F5` re-reads the cache and cuts again.
- **A frozen sample is the exception, not the rule** — `data/<city>/<name>.json` beside
  the manifest wins over the cache. That is how a bug repro is pinned and how "edit a tag
  in the file, press `F5`" still works. The file is made by `ROADS_DUMP=<dir>`, which
  writes every sample as the gallery cut it (one element per line, tags sorted —
  `crop::to_json`). The cache is not even read when every sample is frozen.
- **The manifest** `data/<city>.json` lists the samples: `name` (the stem of the Yandex
  shot and of a frozen file), title, what to look at, window centre as **lat/lon** (map
  metres move with `MAP_SIZE`) and the visible `half`. Adding one: find the node
  (`tools/osm_near`), put `"at": [x, y]` in the manifest instead of `geo`, start the
  gallery — it logs the `geo` to write in its place. Tula and **Berlin** have manifests;
  another city shows "no samples yet". Berlin is there for the sections: `lanes` is on
  about half of its streets (Tula: 97 %), so its five samples — Moritzplatz ring, an
  untagged residential cross, an untagged tertiary × tagged street, Unter den Linden ×
  Friedrichstraße, Rosenthaler Platz — show the lane count inferred by class next to the
  tagged one. Its extract is 109 MB; the first run downloads it through the game loader.
- **Cost**: reading and deserializing Tula's 18 MB cache and cutting fifteen windows —
  see the `roads: extract … read in …` log line; the cut itself is milliseconds behind a
  per-element bounding-box prefilter and a node → roads index for `joining_roads`.
- **What the cut does — and a way is never clipped.** Every way touching the window is
  kept **whole**, because the pipeline keys on a way's own points: a street's first point
  seeds its row of cars, a lot's outline decides its stall layout, a shared vertex decides
  whether a house is squared. The first version clipped ways to the window and the sample
  visibly stopped matching the game (other stalls in the lot, other cars in it). Roads
  that share a node with a kept road are added too (`joining_roads`, one level) so the
  kept streets have their junctions, and so are the **footways along a kept street**
  anywhere on its length (`footways_along`: a path way — the `road_class` Alley values —
  with a vertex within `FOOTWAY_REACH` 32 m of a kept carriageway's link). The parse
  decides a street's sidewalk band and its verge by probes along the **whole** street
  (60 % of them must find a footway), and a 600 m avenue kept whole with only its
  window's footways lost its verge altogether: a pocket of bare ground framed by tile
  at Tula 02's south-east corner (roads plan S3) that the game, parsing the whole city,
  never had. Only non-building multipolygons (a river runs for
  kilometres) are polygon-clipped; the tag-only `is_in` boundary (the `driving_side`
  carrier) is kept with its `name:*` tags dropped; nodes inside the window are kept (doors,
  trees today; crossings and signals once the query asks for them). `CROP_MARGIN` is 120 m
  beyond the visible half — the reach the parked cars read their quarter by
  (`cars::district`).
- **The layers are clipped to the window as meshes** — `MeshBuilder::clip_to_rect`, per
  triangle, colour and the `Ribbon`/`Roof` attributes interpolated linearly, i.e. exactly as
  the GPU would, so nothing inside the window moves. A ground-coloured mask over the rest
  was the first attempt and **cannot work**: the windows stand 30 m apart, a sample's rim
  lands *inside the neighbour's window*, and a mask only covers what is outside all of
  them — reported by the author as houses standing across the avenue. Crowns are entities,
  not a layer: they are kept by their centre.
- **Checked against the game** (offscreen shots of the same spots, samples 2, 7, 9): roads,
  markings, kerb returns, sidewalks, buildings, shadows and lots with their stalls come out
  identical; a difference means the gallery has drifted from the game's pipeline. Worth
  knowing if the cars ever come back (they were drawn while this was checked): a lot's cars
  matched the game to the car, **the kerb rows did not and cannot** — an accepted car spends
  several RNG rolls, and how many are accepted upstream depends on the buildings along the
  *whole* street, which a window does not hold. Same rule, another draw.
- **Game coordinates are real**: a sample is projected with its city's `GeoBounds`, and is
  moved into the column by shifting the spawned entities (`place_new`, by `Added<Mesh2d>`),
  which is why samples are built one per frame and why the game's spawn doors are called
  as they are. The address is read from the sample itself (`name` of the highways through
  the centre, the country off the boundary relation).
- **The reference beside each window is a Yandex Maps screenshot of the same extent** —
  `data/<city>/<name>.yandex.png`, drawn to the right of the render at the same size
  (a missing file just leaves the place empty). It is cut by arithmetic, not by eye: `z=19`
  is 0.1747 m per CSS pixel at Tula's latitude, so the window is a square of
  `2·half / 0.1747` px round the map centre — and where that centre sits in the frame is
  **measured with a `&pt=lon,lat` marker**, because Yandex centres `ll` on the visible part
  of the map beside its side panel, not on the viewport — read off the DOM
  (`.map-placemark`'s bounding rect), once per browser window size, then shot again
  without `pt`. The rest of the recipe (capture
  twice, the map paints its tiles lazily; a live browser through Claude in Chrome, headless
  Chrome gets a `limited` stub) is in the docs of `examples/demos/roads/samples.rs`.
- The panel mirrors the game's (`examples/demos/roads/panel.rs`): the city select (the
  game's own `qwe::ui::spawn_city_select`, full panel width), a
  **Roads** header (the five `RoadShape` sliders from `qwe::ui::shape_knobs` + Sidewalks),
  a **Road paint** header (Markings, Paint, Crossings, Stop lines, Arrows, Wear, Turn wear)
  and the Network row; a change rebuilds every sample. The gallery settles `RoadShape`
  like the game, hands the settled lane width to `parse_response` in `ParseKnobs` and sets
  the paint's lane-width global (`apply_lane_width`) before re-parsing its samples. `ROADS_SHOT=path.png` takes a frame and exits — counting the shared
  `gallery_shot.rs` frames only from the frame every sample is built and placed
  (`gallery_ready`), since the samples build one per frame and a fixed frame number left
  the late ones (Tula's 28th) empty; `ROADS_SHOT_SCALE=2` takes it at twice the window's
  logical size — the same frame, sharper; `ROADS_SAMPLE=N` frames sample N,
  `ROADS_CITY=<slug>` opens the gallery on that city (the automatic shot of a city other
  than Tula).
- **Captions are always drawn**, even where they run under the panel or off the screen.
  A `hide_captions_under_panel` system once hid a caption whose left edge reached the
  panel, and it was removed at the user's request: on zooming in the whole caption
  vanished while most of it still lay on empty ground.
