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
  sidewalk_width` (`sidewalk_band`: 22 % of the width, 1.2–3 m per side). **The class
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
  A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `mesh_roads` `continue`s into `bridge_casings`
  + `bridges` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`. It would hang a metre or three past the deck edge over the
  water, and the deck already has its own kerb: `push_bridge_curb`, drawn unconditionally.
  The road fill went from osm-carto white to asphalt grey together with the
  markings: a white line on white is invisible, and on grey the street grid also stops
  merging with the courtyards. At a junction the band turns the corner on the kerb's own
  arc — **The drawn network → Kerb returns → The sidewalk turns with the kerb** below.
  **A half of a divided street has no band on its paired side** (`roads::push_sidewalk`,
  **Paired halves** below): along a pair run the band is the width plus one sidewalk,
  shifted half a sidewalk away from the partner, and the full band resumes past the run
  with a butt joint; a piece under `SIDEWALK_PIECE_MIN` 0.5 m between two runs is
  skipped (centimetre offcuts). On a half with a taper the runs are not re-cut and the
  band stays full.
  **`sidewalk=*` picks the sides** (stage 7): `RoadLine::sidewalks` `[left, right]` along
  the points (`parse/tags.rs::tagged_sidewalks` — `both|left|right|no|none|separate`,
  refined by `sidewalk:both|left|right`; `no` and `separate` mean no band, a separate
  footway draws itself; `oneway=-1` swaps them with the points). **Untagged** (no
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
  untagged residential streets left without sidewalks` line.
  `drawn_sidewalk` is `None` when neither side has one; `push_sidewalk` lays a one-sided
  band the paired-half way (width plus one sidewalk, shifted half a sidewalk to its side)
  and ANDs the tag with the pair runs; the kerb returns drop the arc on a missing side;
  a taper's sidewalk wedge is symmetric and skipped on a one-sided street. Tula: 44 `no`,
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
  way when the node is under `CROSSING_SPILL` 10 m from its way's end — the same list the
  cars use; 6 m rather than 4 so the slant starts past a rule zebra, which stands 1–5 m
  past the junction edge, the author's report of a bay cut off flat at a zebra, 3284
  2806), `POCKET_TAPER` 6 m slanted ends where a pocket stops inside the way (none where
  it runs into the way's end, so it continues on the next way), at least `POCKET_MIN`
  10 m at full width. That full run is what a **tagged** side (`street_side`) gets. A
  side that is a pocket **by the rule** gets rare short bays out of it
  (`pockets::sparse_pockets`): a block run carries bays at all with
  `RULE_BLOCK_SHARE` 0.4, each bay `RULE_POCKET_LENGTH` 24–42 m with both tapers, bays
  `RULE_POCKET_GAP` 30–90 m apart, the first up to 30 m in. The RNG is `seed::Lcg`
  seeded by `seed_from_point` of the street's first point, salted by the side, so the
  ribbon and the cars get the same bays and a rebuild moves nothing. The full-block rule
  pocket read as an extra lane wherever no cars stood in it (the gallery has none),
  and real bays are a few cars long; OSM's own bays arrive as tags or as separate
  `amenity=parking` + `parking=street_side` outlines (Berlin 3275, Tula 56), which
  reach the parking layer (`references/parking.md`).
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
    run's gap is its median. One `Median` per pair of runs, from the half with the lower
    index — the old lot code computed it from both sides first and got two double lines
    a few centimetres apart.
  - **Alignment** (`Pairs::align`) — each half is densified to `ALIGN_STEP` 4 m and moved
    so that it stands at half the target distance from the midpoint between it and the
    partner's original axis: target gap = the run's median, paved ones no narrower than
    `PAVED_MIN_GAP` 0.5. The weight fades (smoothstep) over `ALIGN_TRANSITION` 20 m to a
    run's end — unless the end is a seam whose continuation carries a run at the same
    node — and to a node shared with **another** carriageway (a street or a drive — a
    footway crossing pins nothing, as on **The street axis**), which stays exactly in
    place (kerb returns, breaks and stitches find each other by it). Next to such a node
    the axis is not moved at all for `PIN_STRAIGHT` 16 m and the fade begins beyond it: a
    kerb return is laid only on a straight edge, and a 10 m return to a crossing avenue
    needs its half width plus the tangent (stage 5 — before it the fade bent the edge from
    the node on, and the corners of sample 2 came out a metre or two). Then the path is thinned back by
    Douglas–Peucker at `SIMPLIFY_TOLERANCE` 3 cm keeping every shared node, and the
    median's midline and the two inner kerbs are sampled off the aligned axes and thinned
    the same way; the thinning is what took the stage from +130 k vertices and +50 ms
    down to +24 k and +18 ms. Ends of two medians closer than `JOIN_GAP` 5 m are drawn
    together (`join_ends`): a half of two ways is two runs, and the gap at the seam was a
    hole in the double line and a kerb island on the «Макси» boulevard.
  - **Paved median** (gap ≤ `RoadShape::median_gap`, 1–6 m, default 3; the flag is
    stored on `Median` at construction — `Pairs::new(roads, paths, median_gap)` — and
    `Median::is_paved` reads it; the pair tests take the knob's default) — `push_paved` lays a ribbon down the
    midline as wide as the axes are apart into the `roads` layer **before** the halves
    (no lane frame, so no ruts; the halves lay theirs over it), and the paint layer draws
    a **double solid** down the midline (`Painter::paint_median`, the axes mesh).
  - **Lawn** (wider) — the contour between the inner kerbs, opened by `NOSE_SHARE` 0.45 of
    the gap for a **rounded nose**, goes into the `sidewalks` layer (it shows as a
    `MEDIAN_KERB` 0.5 m kerb along each half), and shrunk by the kerb it is grass in
    `road_medians` (`Z_ROAD_MEDIAN` 1.7, `SurfaceKind::Grass`, the meadow colour); a lawn
    or kerb piece under `MIN_LAWN_AREA` 4 m² is not drawn. Drawn
    whatever `RoadStyle::sidewalks` says: a lawn is still a lawn.
  - **Tram track bed** (`medians::carries_tram`) — a median of any width with a
    `RailKind::Tram` axis within half its width on at least `TRAM_SHARE_MIN` half of the
    midline (probed every 5 m) is paved like a narrow one, and its double solids run
    along **both edges**, `TRAM_EDGE_INSET` 0.3 m inside the halves' inner kerbs, not
    down the middle: the rails lie between them (the tram layer is off by default). In
    OSM Советская is two halves with the tram ways in a 5 m gap, and the width alone
    read it as a lawn down the avenue (gallery 18). `MeshReport::medians` counts what is
    drawn, so a tram bed counts as paved.
  - **Where it opens** — `crossing_breaks`: only a junction break of one half **facing** a
    break of the other (within the axes' distance plus both reaches) — a crossing
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
  - **Sections** (`sections::apply`, **step 0 of `finish_parse`**). A way's lanes: the tag
    (`lanes`, else `lanes:forward` + `lanes:backward` + `lanes:both_ways`, the last one
    optional — one direction alone is not a sum),
    else the **nearest tagged way of its street** by the distance between their middles
    along it, else `default_lanes` by class (four on motorway/trunk/primary/secondary
    two-way, two on the rest — every `*_link` included, half of that one-way, one on a
    service drive). Then a **lone jump** — a run shorter
    than `SPIKE_MAX_LENGTH` 60 m with the same count on both sides and another of its own —
    is cut to its neighbours. `RoadLine::lanes` is **overwritten** with the result on every
    street and drive, and `width = lanes × lane width + 2 × EDGE_WIDTH` — a lane on a
    street is the **lane width** knob (`shape::lane_width()`, `RoadShape::lane_width`
    2.75–3.75 m, default 3.3), on a service drive that minus `SERVICE_LANE_NARROWING`
    0.3, 0.5 m of edge each side: at the default a
    two-lane street is 7.6 m, a six-lane avenue 20.8, a one-lane one-way half 4.3 — where
    the class gave 8, 16 and 16. Because the parse reads it, a new lane width is a **world
    reload** (**RoadStyle and RoadShape** below). **It is the one roads stage that moves the model**: the
    width is read by the passes after it (houses off the sidewalks, blocks and lots pulled
    to the roads), and then by bridge curbs, the navmesh's bridge corridors and the cars.
    Paths keep their class width (3.5). The gallery parses each cut window on its own, so
    a street there is inferred from the window's ways only.
  - **Tapers** (`tapers::Tapers`, in `mesh_roads`) — where two ways of one street meet at a
    **pure seam** (`RoadNodes::roads_at` = exactly those two; at a junction the step sinks
    into the junction's asphalt, and the kerb returns are built on the full width) and
    their widths differ by 0.1 m or more, the wider way's drawn path is **cut** at that end
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
    The same taper is laid in the **sidewalk** band (from the narrow way's band). No taper
    on bridges or passages.
    **A half of a divided street gets asphalt under its taper on the partner's side**
    (`mesh_roads`, a wedge whose middle lies in a pair run): a ribbon of half the wide
    way's width along the wedge, offset a quarter width toward the partner, butt ends, no
    lane frame, pushed before the wedge. The symmetric wedge narrows toward the median as
    well, while the median (**Paired halves**) is measured off the full width — and in the
    gap between them lay the half's full sidewalk band (a tapered half keeps it), a light
    strip the length of the wedge (roads plan D3, gallery 16). The dark line beside it on
    16 is **not** a seam: it is a real metal fence down the median (way 357798630,
    `barrier=fence` + an admin boundary, `height=1`) with its shadow — `tools/osm_near`
    does not list it because it skips boundaries.
    Drawing only: navmesh, cars and parse see each way's width as is.
  - **The network overlay** — `map/roads/network/overlay.rs::mesh_network_overlay`
    (re-exported `qwe::map::mesh_network_overlay`, z 29): every street in its own colour,
    the line thicker by the way's lanes, a white dot on every seam of a street. Shown by the
    game's Debug → Overlays **Road network** row (`DebugRoadNetwork`,
    `ui/debug/overlays.rs::sync_road_network_overlay`) and by the gallery's `Network` row
    (or `ROADS_NETWORK=1`; `examples/demos/roads/overlay.rs` keeps only the toggle).
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
  - **Dashes by the street's arclength** (`paint::street_stations` over the network's
    ordered ways and the axis paths): 3 m / 3 m, and the phase runs through a seam.
  - **Solid near a junction**: the last `APPROACH` 25 m before a junction break — for a
    **lane line only on the approach**, in the direction its lanes flow
    (`paint::flows_forward` by the line's side of the axis and `MapData::traffic_side`, a
    one-way road forward): leaving a junction a lane line is dashed at once (the author's
    report — Первомайская, 3265 2802, carried a solid line on the exit side).
    `paint::approach_spans` finds the gap edges on the to-break profile (it is linear
    between vertices, so an edge is a zero on a link), `split_at_spans` puts a vertex at
    each span end, and the line goes out in pieces of `LineKind::Dashed` (10) and
    `LineKind::Solid` (11); the shader only draws what the kind says. An **axis** and a
    **ring's** lane lines keep the old symmetric rule (kind 0/1 — solid by to-break alone):
    the axis separates two flows, and a ring's entries are not worth splitting a closed
    strip for. The axis of a two-way street with 4+ lanes is a **double solid** (0.15 m gap
    — ГОСТ 1.3's 10–15 cm; half a metre read as two separate lines, the author's report —
    merging into one line once the gap is under ~2 px); a two-lane two-way street has a
    dashed axis; an odd two-way street and a one-way street have none.
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
  - **`RoadPaintStyle`** (group `road_paint`): `paint` 0–1 (0.85) — the line opacity,
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
    `BEND_MIN_LENGTH` 4 m is not built.
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
    instead of a sidewalk ring (`push_ring_edges`). The island's fill is whatever the map
    has there (a park, a lawn, the ground).
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
  unrewritten set: a median opens at a crossing whoever leads it.
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
- **Junction paint** (`map/roads/node_paint.rs`, `NodePaint::new`, called by `mesh_roads`
  on the stitched axes — **always**, markings on or off: with markings off it paints
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
    and two-way north. Signals aside, such a road **leads** the junction
    (`Junction::leading`), and so does a roundabout that passes it whatever the approaches'
    class — a ring has priority. The leading road loses its asphalt breaks there
    (`NodePaint::asphalt`): the ruts run through, signals or not.
  - **Zebras and stop lines on the arms that break**: an OSM crossing on the arm (a
    `Crossing { marked: true }` node on the road, between the node and
    `ARM_CROSSING_REACH` 35 m past the junction edge — measured from the edge, since a
    wide junction's edge is itself tens of metres from the node) becomes the zebra; without
    one, `CrossingMode::Generated` puts a zebra `ZEBRA_SETBACK` 1 m past the edge — if the cluster joins two streets
    with sidewalks (by the `sidewalk=*` tag — `RoadLine::sidewalks`, like `kerb_parking`;
    the Sidewalks toggle only hides the band, `crossings` is the zebra's own knob), one of
    the cluster's streets is at least `tertiary` (`RULE_ZEBRA_RANK`) or the cluster is
    signalized, no road of the cluster is a ring arc (`on_ring`, the check the lane
    arrows use — and no closed ring passes it), the
    arm is not a `*_link` and the next junction node on the same road
    lies at least `RULE_ZEBRA_ROOM` 30 m past the edge (`nodes_along`). A shorter arm is
    a link between two nodes — the branches of a fork's triangle (Tula, gallery 06, 22 and
    31 m) had a rule zebra at both ends and a stop line between them within fifteen metres;
    the crossing is left to the outer arms. An OSM crossing ignores the room. A zebra is `ZEBRA_LENGTH` 4 m along the
    road, across the carriageway less 0.3 m at each kerb. The stop line is called by the
    same things as the zebra — a zebra on the arm, signals, a stop / give-way sign, or a
    street of at least `tertiary` in the cluster; two residential streets with no sign get
    neither (gallery 13: Yandex draws the cross bare). The stop line (0.4 m) stands
    `STOP_GAP` 1 m behind the zebra (or 1 m past the edge without one), across the lanes
    **coming to the node** — axis to kerb on the traffic side (`MapData::traffic_side`)
    for a two-way road, the full width for a one-way one that flows toward the node, none
    on a one-way arm leaving it. A `give_way` sign without signals makes it dashed. The
    two halves of a divided street cross on **one line**: `align_pair` moves the second
    zebra onto the first's line across the street (to the OSM one if there is one, else to
    the farther one); when **both** are OSM crossings — a `highway=crossing` node on each
    half, which mappers place a metre apart (gallery 02: 0.9 and 1.2 m) — both move to the
    line halfway between them. Over a **paved** median (`PairRun::paved`, handed to
    `NodePaint::new` as `Partner { road, paved }`) the two aligned zebras then become **one
    plank** kerb to kerb (`join_zebras`: parallel within `JOIN_PARALLEL`, on one line
    within `JOIN_OFFSET` 1 m, the gap between them at most `JOIN_GAP` 8 m): the shader
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
    node to the outermost paint. An **OSM crossing keeps its place** — it is measured
    from the reach as before: pushed past the new edge, it no longer fitted a short arm
    with its `ARM_TAIL` and was lost (gallery 04, south). **Not at a ring**: an approach
    is fitted into the ring tangentially and runs over its asphalt for tens of metres.
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
    ones.
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
    square, a lay-by, a widening the axis does not describe. `Walkway` outlines are left
    to the sidewalks and alleys that already cover them.
  - **Tula has almost none of it** (no island at all, one `crossing:island`, a dozen
    service-yard outlines — `references/osm-coverage.md`, «v15»), so the gallery check
    is Berlin (`ROADS_CITY=berlin`, samples 4–6: `area:highway=traffic_island` at
    Rosenthaler Platz and the boulevards, `area:highway=primary|tertiary` outlines,
    signalized crossings with islands). The report counts `safety islands N + A areas,
    carriageway areas C`.
- **Turn paths** (`map/roads/turns.rs`, `Turns::new` over `NodePaint::junctions`) — the
  wear a junction gets from traffic crossing it. The lane ruts fade in a junction gap (a
  car crossing a junction is not in a lane), so without these the middle of every node was
  bare asphalt, and a real one is polished lighter than its approaches.
  - **Arms** (`JunctionArm`): every arm of the cluster, rings included (a closed ring gets
    two, one each way from the node, which the zebras never see), with its **edge** — the
    arclength on its drawn axis where the junction gap ends (half the widest other road
    plus 1 m, the asphalt break's reach), on a ring taken around the seam.
  - **Lanes on an arm**: the body lane frame (`paint::lane_frame`), lane centres between
    the lines; a one-way road carries traffic along its points only, a two-way road along
    them on the traffic side's half (`MapData::traffic_side`), the middle lane of an odd
    two-way road belonging to neither — except a one-lane road, driven both ways. Lanes
    are counted **from the kerb**.
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
    on it by arc length (`paint::along_back`); a straight line back from the edge left the
    lane on a curved approach — 20 m out it sat on the lawn (Leipziger Straße, Berlin).
    A lane shorter than that falls back to the straight line. **A second row** stands
    `ARROW_REPEAT` 20 m behind the first (`Painter::repeat_setback`; Yandex puts them at
    5 and 20–25 m from the crossing on 01, 02, 15, and ГОСТ 1.18 repeats them): only where
    the lane's centreline runs on `ARROW_REPEAT_CLEAR` 5 m past its tail, no zebra or
    stop line crosses the lane between the rows, and the row is clear of the approach
    road's own breaks (`LaneArrow::road` → `NodePaint::breaks`) — the centreline is the
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
      ~1000 kerb returns across Tula in the first cut;
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
    edge of the **sidewalk** when the road carries one (`drawn_sidewalk`, so it follows
    `RoadStyle::sidewalks`): OSM maps a drive «to the pavement footway», which sits ~9 m off
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
  - **Kerb returns** (`kerb_returns`) — the rounded corner of a junction. At every shared
    node, arms are collected from the **drawn** paths (pinned, so the node is a vertex of
    each): a direction to the first vertex at least 0.5 m away and the straight **run** —
    to that vertex and on through every next one within `STRAIGHT_TOLERANCE` 0.15 m of
    the arm's line. OSM puts vertices on a straight drive wherever it likes (the node
    where the pavement footway crosses it, 2 m off the street), and a run cut at the
    first of them clipped the tangent to nothing: the drive met the street with square
    corners (reported from a screenshot; 8220 → 8710 returns on Tula). Arms of one class are sorted by angle, and between neighbours 25°–155° apart the
    corner of the two facing edges is found, a circle is fitted tangent to both — its
    radius **by the minor class of the pair** (`kerb_radius`, stage 5 of the roads
    rework): `MAJOR_RADIUS` 10 m between avenues (`trunk`…`secondary` and their links),
    `STREET_RADIUS` 6 m with a street (`tertiary`, residential, `unclassified`),
    `DRIVE_RADIUS` 2.5 m with a drive, a living street or a driveway crossing,
    `PATH_RADIUS` 2 m between footways. It used to follow the widths — `0.6 × (half +
    half)`, and only 0.4 × the narrower half width for a minor entry — and on a divided
    avenue, where halves of different lane counts meet in one node, every corner came out
    a metre or two (sample 2). The wedge `[corner, tangent, arc…,
    tangent]` goes into that class's fill builder **before any ribbon** — ribbons and their
    markings then lie over it, and since it is pushed with no ribbon coords it carries no
    wear or markings of its own. One clamp: the tangent never runs past an arm's straight
    run (past the next vertex the edge has turned) **nor into a taper** (`kerb_returns`'s
    `tapers` — the run ends where the wedge begins, since the edge there is already
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
    footway would read as asphalt spilled onto the path. Bridges and passages give no arms
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
      117 → 202 on sidewalks; the road build did not move (125.6 ms).
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
        a pair run (`paired`, the runs of **Paired halves** with two probes of slack) the
        partner's side is `None`. The corner is taken from the first arm's left to the
        second's right, so a corner facing the median gets none — it put light arcs into
        the median opening of a divided avenue.
      - **A crossing street's piece between two halves carries no sidewalk at all**
        (`across_median` in `mesh_roads`, under `MEDIAN_CROSSING_MAX` 40 m, one end in a
        node with a half and the other in a node with its partner): it lies in the median
        opening, and its band showed as a light disc in the middle of the junction
        (sample 15).
      - Load-time only, like the rest of this module.
- **RoadStyle and RoadShape** — three resources behind the roads, split by **how a change
  reaches the map**: `RoadPaintStyle` (**Markings — the paint layer** above) is uniforms
  only, a drag rebuilds nothing; `RoadStyle` is toggles, each click one rebuild;
  `RoadShape` is sliders that move geometry, so the map follows a **settled** copy.
  - **RoadStyle** (resource, BRP-writable, persisted; toggles in the Roads and Road paint
    sections, `ui/roads.rs` / `ui/road_paint.rs`) — what gets drawn; any change reruns
    `rebuild_roads` (despawn `RoadLayerTag` layers, respawn from the unchanged `MapData`).
    Five toggles: **sidewalks** and **markings** (both on) are described above,
    **crossings** (`CrossingMode`: `Off` / `Osm` / `Generated`, the default) and
    **stop_lines** (on) in **Junction paint**, **arrows** (on) — the lane arrows, their own
    toggle since stage 8, no longer under `markings`. Stage 8 took out `join`, `smoothing`
    and `casing`; old keys in `settings.toml` are ignored silently (bevy_settings applies
    only the fields the type has). The join is the constant `ROAD_JOIN` = `RoadJoin::Round`
    (the `Square`-only branches — no tapers, no kerb returns — went with it); `RoadJoin`
    and `Smoothing` stay for the tree-row band's own Joins / Smoothing rows and for rails,
    the tram and water (`Smoothing::Light`). The dark road/alley **casing layers are gone**
    (`alley_casings`, `road_casings`, `Z_ALLEY_CASING`, `Z_ROAD_CASING` and their colours),
    so `mesh_roads` yields **18 layers**: ten ribbons + eight paint layers. `bridge_casings`
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
  - **Lane width is a world reload**, not a rebuild: the parse reads it (**Sections**), and
    the parse runs on the load thread with no ECS, so it travels as a process global —
    `shape::lane_width()` / `set_lane_width()`, an `AtomicU32`, the same way as the sun
    and the navtile size. `loading.rs::sync_lane_width` writes it on `OnEnter(Loading)`
    right before `start_job` (next to `sync_navtile_size`); `city.rs::reload_world` fires
    on `lane_width_moved` (the settled width differs from the global) — same city, the
    camera stays. Paint and turns read the same global (`BIRTH_FADE` is half a lane), and
    the surface shader gets it as `SurfaceParams::lane_width` (`surface.wgsl` no longer
    hardcodes 3.3); `surface::retune_surface_materials` also runs on `OnEnter(Playing)`
    so the ruts follow a new width.
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
- **Bridge layers** (`map/roads.rs`, same `RoadLayerTag`) — a road with `bridge` leaves
  its class layers for the **three** `bridge_shadows` (`Z_BRIDGE_SHADOW` 2.05) +
  `bridge_casings` (`Z_BRIDGE_CASING` 2.1) + `bridges`
  (`Z_BRIDGE` 2.2). The **shadow** is the deck's own band, offset along `shadow_dir()` by
  the deck height through the usual `shadow_length_scale()`, on a blended material of its
  own (the flat white one would eat the vertex alpha). Nothing else produced it, because
  the ground shadow layer only knows buildings, and a bridge over the river is the most
  visible thing on the water. It sits **under** the deck and **over** what the bridge
  crosses — except what the z ladder draws above bridges: the rails (a tram on a bridge
  must stay visible), the tram line, wagons, parked cars, fences.
  Eight decisions in `map/roads.rs` (`Bridges`, `probe_underneath`, `bridge_height`,
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
twenty-three for Tula (stage 7 added three: `21_turn_pocket`, проспект Ленина's one-way
half widening from two lanes to three before a node, `turn:lanes` `left|left|right` —
the **lane arrows** and a pocket's line ending in the gap; `22_lane_change`, two-way
secondary улица Болдина going from two lanes to four at a seam — the two-way twin of
`16_lanes_taper`; `23_s_curve`, Путейская улица's 200 m right-then-left bend in one
way — the smoothed axis carrying the ribbon, sidewalk and dashes; the twentieth, `20_roundabout_arcs`, is the secondary ring of six arcs — the
"egg" of **Roundabouts**; `04_roundabout_large` is the primary one; the nineteenth,
`19_offset_joins`, is Tsiolkovsky street with two side streets
joining from opposite sides 17 m apart — one cluster of **Junction paint**; the sixteenth, `16_lanes_taper`, is a one-way primary going from four lanes to
two at a pure seam — the taper of **Streets, sections, tapers**; the seventeenth,
`17_ring_gores`, is the mall ring the plan's acceptance names — three hatched gores and
the boulevard's double solid line must survive every stage; the eighteenth,
`18_lawn_median`, is Советская улица with a 5 m lawn between the halves and a lane into
one of them — **Paired halves**): crossings of avenues (square and skew), of an avenue and a street, of a divided
avenue and a street, of private-sector streets and of yard drives, T's into an avenue and
into one half of a divided one, a fork round a triangular island, a roundabout, five
arms, a drive into a street, a street that narrows, a sharp bend, a dead end — each with
its full address and **game coordinates** in a caption to the left of its window.
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
  kept streets have their junctions. Only non-building multipolygons (a river runs for
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
- The panel mirrors the game's (`examples/demos/roads/panel.rs`): the city switch, a
  **Roads** header (the five `RoadShape` sliders from `qwe::ui::shape_knobs` + Sidewalks),
  a **Road paint** header (Markings, Paint, Crossings, Stop lines, Arrows, Wear, Turn wear)
  and the Network row; a change rebuilds every sample. The gallery settles `RoadShape`
  like the game and sets the lane-width global (`apply_lane_width`) before re-parsing its
  samples. `ROADS_SHOT=path.png` takes a frame and exits, `ROADS_SAMPLE=N` frames sample N,
  `ROADS_CITY=<slug>` opens the gallery on that city (the automatic shot of a city other
  than Tula).
- **A caption under the panel is hidden** (`hide_captions_under_panel`, every frame): the
  panel is translucent as in the game, and in a close-up (`ROADS_SAMPLE`) the caption
  thirty metres left of the window showed through it in broken lines. A caption line whose
  screen span reaches into `panel_span()` gets `Visibility::Hidden`; in the overview the
  captions stand right of the panel and stay.
