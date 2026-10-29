# Roads — streets, sections, tapers, the drawn network

The street as a unit: how OSM ways glue back into streets (**RoadNetwork**), how the
lane count and width are settled (**Sections**), the width-change wedges (**Tapers**),
the ribbon primitive, and **The drawn network** — the `Drawn` preparation, its three
axes, the street axis smoothing, the stitches — plus the debugging overlays. The hub is
`roads.md`; the paired
alignment that moves the axes lives in `roads-pairs-medians.md`, the kerb returns the
axis prepares straight edges for in `roads-corners.md`.

- **Streets, sections, tapers** (`map/roads/network/streets.rs`, `sections.rs`,
  `map/roads/tapers.rs`) — the width of a street
  is not a property of its class but the consequence of its lanes.
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
    draw the join (`roads-merges.md`, **Merges**).
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
    as a drive beside a sidewalk three times wider; a big ring is
    driven in two rows, while a small park ring keeps its one lane, as on Yandex. The radius is read off
    the way alone — the perimeter of a closed one, the circle through the ends and the
    middle of an arc — since the rings are assembled only when drawn. Residential and unclassified
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
    reload** (**RoadStyle and RoadShape** in `roads.md`, the hub — **Lane width is a world
    reload** below). **It is the one roads stage that moves the model**: the
    width is read by the passes after it (houses off the sidewalks, blocks and lots pulled
    to the roads), and then by bridge curbs, the navmesh's bridge corridors and the cars.
    Paths take theirs from the parse (`parse-geometry.md`, **Path width**). The gallery parses each cut window on its own, so
    a street there is inferred from the window's ways only.
  - **Tapers** (`tapers::Tapers`, built **once** per `mesh_roads` by `roads::Drawn` over
    the roads as drawn — the ribbon, the paint wedges and the kerb pockets' row breaks
    (`car_clearings(roads, &Tapers)`) all read that one value, and the car row reads it
    off its own `Drawn::nodal`; only the car gallery, which has no map, builds
    `Tapers::of_map` over bare roads) — where a way ends and another goes on
    from the node collinearly — a joint of one street or a continuation across streets
    (above) — and their widths differ by 0.1 m or more, the wider way's drawn path is **cut** at that end
    by `RoadShape::taper` (5–20, default 10) × the difference (at most `TAPER_MAX_SHARE` 45 % of its drawn
    length, since both ends may taper; under 1 m no taper). The cut end gets a **butt** cap
    (`push_ribbon_trimmed`) — a round cap of the full width
    would bulge out of the taper — and the piece is laid by `MeshBuilder::push_taper`: a
    strip whose width runs linearly from the seam way's to its own (the seam way is the
    narrow neighbour for a narrowing taper, the wide one for a counter-taper), joined by bisector
    vertices, with ribbon coords scaled to the local half width, so the lane lines fan out
    with the edges (proper lane geometry through a change of count is the paint stage's,
    `roads-markings.md`, **Markings**).
    **The two butt ends at the cut must meet exactly**, and two things used to break it,
    both in the merge of close points (`merge_close_points`, a quarter of the width): it
    kept the *first* of two close points, so a taper cut just past a vertex ended short of
    the seam; and each piece merged its own short end link, so the taper
    ended square to one direction and the body started square to another — on a bend that
    left a wedge of pavement across the carriageway (gallery sample `06_taper_on_a_bend`).
    Now a ribbon and a
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
    covers nothing, its layer hides no step). The side with the
    arm keeps its full width up to the node, as before: the joining road's asphalt and
    the kerb returns cover the step there — **but only an arm whose kerb radius is at
    least the step** (`corners/arm.rs::road_radius` of the arm and the wide road, the smaller,
    against half the width difference). A covered step still reads as a **tooth** when it
    is higher than the corner beside it — a drive's radius does not swallow a lane's
    step; a street's 6 m or an avenue's 10 m corner does. Such a side is free,
    and it narrows. **A bent joint with an obtuse continuation grows the mouth instead
    of narrowing it**: a joint of the pair that leaves the node at more than
    `PASS_ALIGN` 15° is a fork, not a continuation, and when the wide way is carried on
    at an obtuse angle (`MAX_ANGLE` 155°–180°) by *another* road of the node, the
    narrowing taper ate a lane off the mouth and
    halved the axis into a single tail, and standing it down left the wide
    kerb a straight butt at full width. So the seam's narrow way **and** the
    obtuse continuer each take a **counter-taper** (`tapers::widen_mouth`): they start at
    the seam with the wide way's section and shed to their own over the taper length,
    only on the side facing the wide way (the other side belongs to a different corner —
    a nose, a joining arc). `Taper::seam` names the road whose section stands at the
    seam — the narrow neighbour for a narrowing taper, the wide one for a counter-taper;
    the ribbon (`push_wedges`), the lane paint (`WedgeEnd`), the kerb arms and the pairs
    (`SeamWedge`) all read it that one way. With the kerbs meeting at the node at the
    wide way's width, the corner rules close the mouth in arcs — and `fillet_arc` gates
    on the angle **between the arms**, not between the sloped kerb edges: under a taper
    the edge drifts off the axis by atan(slope), and the obtuse wedge's gate refuses a
    sloped kerb, leaving the corner without a piece. Without an obtuse continuation the
    bent joint tapers as before. A
    second test was tried and dropped — a slanting arm covers only the
    foot of the step's wall (`half / |cos|` of its angle to the axis): it freed the side
    where the short way's wedge came out steeper than the step it replaced. A crossing covers both sides — no taper, the
    step sinks in the node. `Taper::kept` names the untouched kerb (`+1` left) for the
    paint: on a one-sided wedge the seam section's lanes hug that kerb
    (`paint::kept_frame` — the body's grid with the far bound pulled in by the missing
    lanes; the symmetric wedge keeps `narrow_frame` and its drift), and the new lanes are
    born at the narrowing kerb with the kerb itself. The mesher lays it with
    `push_taper_sided` (half widths `[[left from, left to], [right …]]` in the wedge path's
    frame — `roads/ribbons.rs::wedge_halves` swaps the sides for a tail wedge, whose path runs against
    the way); `push_taper` is the symmetric case of it. **The kerb returns read the
    tapered end at the seam way's width**: an `Arm` carries `half: [left, right]`, and at
    an end vertex with a taper the tapered sides take the seam way's half width and
    sidewalk (`Drawn::taper_ends` hands `kerb_returns` the `Taper` itself) — the fillet
    then meets the wedge edge at the wedge's own 2–3° slant instead of floating a metre off
    it, which is what the pure-seam rule was protecting against.
    The same taper is laid in the **sidewalk** band, per side as well: the narrowing side
    from the narrow way's band (its bare half where it has no sidewalk) to this way's, the
    kept side at this way's band; a side without a sidewalk by tag is the bare half, so a
    one-sided street gets its wedge too. The half width per side is
    one answer, `Drawn::band_half(road, side)`, asked for the narrow way and for this one
    (**The drawn network** below). No taper
    on bridges or passages.
    **A half of a divided street keeps its partner-side kerb straight through a taper by
    its axis, not by extra asphalt**: the alignment (**Paired halves → Alignment** in
    `roads-pairs-medians.md`) sets
    the wedge's axis by its tapered half width, so the symmetric wedge narrows only on the
    outer side. (The old order — the median measured off the full width, the wedge
    narrowing toward it, the half's full sidewalk band showing in the gap — needed a
    patch ribbon under the wedge, which left a miss between the two axes of the half at
    the seam and stood on the wrong side of a tail wedge; the patch is gone with the
    cause.) The dark line beside the median may look like a seam: it can be a real metal
    fence down the median (`barrier=fence`, often with an admin boundary) with its
    shadow — `tools/osm_near` does not list it because it skips boundaries.
    Drawing only: navmesh, cars and parse see each way's width as is.
  - **The network overlay** — `map/roads/network/overlay.rs::mesh_network_overlay`
    (re-exported `qwe::map::mesh_network_overlay`, `settings::Z_NETWORK_OVERLAY` 29 — the
    three road overlays' rungs sit in `settings.rs` with their order asserted): every street in its own colour,
    the line thicker by the way's lanes, a white dot on every seam of a street. Shown by the
    game's Debug → Overlays **Road network** row (`DebugRoadNetwork`,
    `ui/debug/overlays.rs::sync_road_network_overlay`) and by the gallery's `Network` row
    (or `ROADS_NETWORK=1`; `examples/demos/roads/overlay.rs` keeps only the toggles).
  - **The OSM contours overlay** — `map/osm/contours.rs::mesh_osm_contours` (z 29.5): the
    geometry before the finishing passes over the drawn map (`references/parse-seam.md`,
    the parse seam). Gallery row `Contours` / `ROADS_CONTOURS=1`, game Debug → Overlays
    **OSM contours**. **Every visual road report starts here**: the magenta axis and its
    node squares lying on the defect say "data", lying off it say "our parse or drawing".
  - **Raw OSM, drawing level** (`RawOsm::Draw`, `references/parse-seam.md`; gallery
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
    car placement (`cars::park_on`) and the standing wagons (`wagons::mesh_wagons`,
    `WagonReport::hidden`), and drops the stall and pitch markings in
    `spawn::mesh_surfaces`; lots and pitches stay as their polygons. Buildings are drawn
    as always (their outline is already the data's — the parse level skipped squaring and
    pulling).
  - **A one-off window** — `ROADS_AT=x,y[,half]` (map metres, half 40 m by default)
    replaces the manifest column with a single window around that point
    (`samples.rs::window_from_env`), so a report's place can be shot before/after without
    adding a sample in breach of the manifest's criteria; `ROADS_SAMPLE=1` frames it.
    A tour of several such windows in one run is `ROADS_SHOTS` (see the gallery
    section, `roads-gallery.md`).
- **Ribbon** — a constant-width band along a polyline (`MeshBuilder::push_ribbon`), how
  every road, alley and kremlin wall is drawn. The `roads::push_ribbon` **wrapper** over
  it exists only to map a `RoadJoin` (for roads the constant `ROAD_JOIN` = `Round`; a user knob only on the tree-row band) onto
  the pair below, so the layers
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
  - **Why a ring must be drawn closed**: laid open, a ring lays **two round caps on its
    own asphalt** at the seam — and inside a cap the ribbon frame is
    **frozen** at the end's direction (`FanCoords::Cap`), while outside it curves with the
    ring, so the lane dashes step sideways and the wear ruts change direction along a
    crisp circle. The caps are right at a loose end and harmless at a junction (the break
    blanks the markings and the wear there anyway) — the ring is the one place a cap
    lands on live asphalt of the same road.
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
  rasteriser left a hairline across the whole ribbon. The same holds for the
  vertices `GapProfile::split_path` inserts, which are collinear by construction.
- **`Drawn`** (`roads/drawn.rs`) — the prepared roads as one value, built once at the
  top of `mesh_roads` (`Drawn::new(map, style, shape)`) instead of the locals that used
  to open it. **Its fields are closed**; what it holds is reached by queries: `road(i)` /
  `roads()` (the map's roads with the substitutions on their index — driveway crossings,
  then ring arcs, the later one winning; an unsubstituted road is a borrow of the
  map's), `nodes()`, `axis(i, Axis)` / `axes(Axis)`, `pairs()`, `rings()`,
  `stitches()`, `tapers()`, `merges()`, `lots()`, and the per-road answers
  `taper_ends(i)`, `is_merged(i, end)`, `stitched_end(i)`, `stitch_offset(i)`,
  `on_ring(i)`. The pair side is not among them: it is asked of `pairs()`
  (**Paired halves → Queries** in `roads-pairs-medians.md`),
  one owner rather than a delegate on each. Every vector
  is indexed by `map.roads` (an `assert` in the build, not an empty answer on a
  mismatch). The OSM points are not in it — whatever finds a node by `node_key`, and
  the kerb-pocket seed, reads `RoadLine::points` off the map. Not to be
  confused with `network::DrawnEdges`, the outer edges the stitches measure against.
  **Three sidewalk rules live on it, and they are three on purpose**:
  `sidewalk_drawn(i)` — the band that is *drawn*: `SidewalkProfile::any` (the map's
  sidewalk) under the `Sidewalks` knob, minus the crossing piece in a pair's opening
  (`across_median`, **Kerb return** in `roads-corners.md`) — the ribbon and the turning circles read
  it; its per-side form `sidewalk_on(i, side)` is the same answer where the tag puts a
  sidewalk on that side (`SidewalkProfile::sides`), one call where the kerb pockets,
  the kerb returns and the merge edges each used to AND the tag in by hand; the pair
  side is still the consumer's to drop (`Pairs::beside`, `band_pieces`) — pinned by
  `roads/pockets/tests.rs::a_pocket_on_the_side_without_a_sidewalk_pushes_no_sidewalk`
  and `a_one_sided_street_turns_its_sidewalk_only_on_its_side`; `sidewalk_mapped(i)` —
  the sidewalk the *map* has, knob or no knob, minus the same piece — the junction
  paint reads it, a rule zebra being a question of the model and not of a display
  toggle (`roads/tests.rs::rule_zebras_do_not_follow_the_sidewalk_knob`); `band_half(i,
  side)` — the half width of the band on one side, `width / 2 + drawn sidewalk ∧
  sidewalks[side]`, the bare kerb where the tag has no sidewalk — the per-side wedge
  (**Streets, sections, tapers** above) reads it for the road and for its narrow neighbour.
  The crossing piece is pinned by
  `roads/network/pairs/tests.rs::a_crossing_piece_between_two_halves_carries_no_sidewalk` (drawn exactly as
  the same piece tagged `sidewalk=no`), the per-side wedge by
  `a_one_sided_sidewalk_wedge_keeps_the_bare_kerb_on_the_untagged_side`.
- **`Axis`** (`drawn.rs`) — which axis a consumer takes, named in the call rather than
  implied by which local it read. A road has three: the **OSM points**
  (`RoadLine::points`) — everything that keys a node (`junctions::node_key`): the base
  marking breaks, `Tapers::new` and `free_sides`, `row_breaks` / `crossing_breaks` /
  `join_way_ends`, `Pairs::align::continued`, the turning circles, the kerb-pocket seed;
  **`Axis::Nodal`** (the street axis after smoothing, before the stitches — its ends are
  still OSM points): `street_stations`, `merges::merges` and `merge_bands`,
  `kerb_returns`, `all_kerbsides` and `pockets::outline`, `KerbLots::frontage`,
  `tram_bands`, `across_median`, `Pairs::runs`, the car row; **`Axis::Ribbon`** (with
  the stitches — a stitched end sits on another road's axis and keys no node):
  `NodePaint::new`, `Turns::new`, `GoreRoad::new`, `splitters`, `merge_ramps`,
  `nose_fill`, `merge_axis`, `RoadIslands::new`, `wedge_ends`, `Painter::paint`, every
  ribbon. `axes(which)` hands a submodule the whole slice as borrows (a stitched copy
  exists only on a road a stitch touched); `mesh_roads` binds them once as `nodal` and
  `ribbon`. The tram band on the nodal axis is a pin, not an accident
  (`roads/tests.rs::tram_band_follows_the_nodal_axis` — on the ribbon it would run on down
  every stitch); `tram_band.rs` decides for itself what a junction is (`near_asphalt`,
  40 m / 4 m), pinned by its own `a_band_bridges_a_junction_but_not_open_ground`.
  **The base marking breaks stay on the OSM roads**, not on `roads()`: a ring arc drawn
  at its ring's section would move the base break on an approach
  (`roads/drawn/tests.rs::a_ring_arc_base_break_reaches_by_the_osm_width`); a driveway crossing
  stays `Highway::Path` in both and moves nothing. The merge **edges** stay a
  `RoadReport` field of their own (`merge_edges`) — the ribbon lays them, not the
  preparation.
- **The street axis** (`roads/axis.rs::street_axes`). The
  ribbon of every way that lies in a street ([`RoadNetwork`]) is drawn along **one curve
  per street**, not per way — per-way Chaikin pinned both ends of each way, so every
  OSM seam was a corner, and a shared node was never cut, so a through street kinked at
  every junction on a bend. Per run of a street (bridges and arches split it — their
  points are the navmesh's, and they keep the old `centerline`):
  - all three numbers come from one knob, **curve tolerance** (`RoadShape::curve_tolerance`,
    0–5 m, default 3 — how far the axis may leave the OSM points): `Curve::of(t)` gives
    `simplify = t/3`, `deviation = 2t/3`, `radius = 10t` (1 m / 2 m / 30 m at the
    default; there is no `Strong` 60 m step). At 0 there is no curve at all — no
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
    ~1000 of them across the city in the first cut. **A full reversal (bend exactly π)
    is left a corner** (`Corner::bend` takes `MIN_BEND..PI`): OSM has spikes where a way
    steps a metre off a node and the next way of the street comes straight back, and
    `tan(π/2)` in `f32` is *negative* (−2.3·10⁷) — the arc's reach went to −4.6·10⁸ m and
    the axis end flew half a billion metres off (pinned by
    `roads/axis/tests.rs::a_spike_that_doubles_back_keeps_the_axis_finite`);
  - a **pinned node** — one a third **carriageway** touches (a street or a drive,
    `axis::pins`) — stays exactly in place: the kerb
    returns, the marking breaks, the stitches and the tapers all find each other by it.
    A node shared only with a footway pins nothing: the footway ends under the
    street's asphalt and none of those four reads it, while a pinned crosswalk 15 m from
    a junction bent the axis there and left the kerb return no straight edge.
    The street passes it along a **straight stretch on the bisector**
    (`ThroughBend::pad`, up to `THROUGH_RUN` 24 m each way, at most 2 m off the links, at
    most half of each), and the bend goes to two arcs at the stretch's ends. **Two such
    nodes in a zigzag whose stretches would meet in the middle of the link between them**
    (bends turning opposite ways, each at most `SHARED_MAX_BEND` 15°, `reach` of both,
    not yet cut to the link, summing past its length) share that link as
    the straight instead (`shared_pad`): each node's stretch runs on along the link past
    the node just far enough to leave the node `KERB_STRAIGHT` of straight edge together
    with half the shared link (never less than the end arc needs at half-width), and its
    bend goes to one arc on the far side. (The first cut ran the stretch on
    `deviation / sin(bend)` long — 16 m on a 7° bend — and an OSM-straight road read as
    a snake; and a real turn put on one short arc on the far side broke the kerb — only
    jitter gets the shared link; in a C-bend the two stretches meet without the step
    anyway.) A sharp T right next to a bridge end keeps its corner
    — the axis has no room for the arc there, and the paint breaks at such a node
    instead (**Junction paint** in `roads-junction-paint.md`, `bent`). A bend
    under `THROUGH_MIN_BEND` 4° stays a corner (the junction's asphalt covers it, and a
    stretch would only shorten the straight edge); one over `THROUGH_MAX_BEND` 50° is a
    turn, not a through street, and stays a corner as well. A Hermite curve through the
    node was tried first and rejected: it bends hardest at the node itself, exactly where
    the kerb return needs the edge straight;
  - the curve is **cut back into its ways at the seams**, at the arc point nearest the
    seam node, and each piece takes its own way's point order. A seam is therefore no
    longer an OSM node on the drawn path, and a seam bent over 25° no longer gets a
    "kerb return" between two pieces of one street.
  Paths are in no street and go through `centerline` — Chaikin with every shared node
  pinned, a closed way **round the cycle** (`smooth_pinned`'s `closed`), as before. The
  **parked cars stand on the same axes** (`cars::park_cars` takes them; the game passes
  `MapData::network`, the bench and the gallery glue the streets themselves). The axis
  moves no `RoadLine::points`.
- **Stitches** (`stitches`) — a loose end (not closed, not a bridge or a passage, no other
  road at its node that **carries** it: a street is carried only by a street, an alley by
  anything) looks ahead for the nearest centreline of a road it may join: the closest
  point on each segment within 60° of its heading, and the heading's ray hit, scored by the
  gap to that road's **drawn edge**, ≤ `STITCH_MAX_GAP` 6 m. The drawn edge is the outer
  edge of the **sidewalk** when the road carries one (the mapped edge of
  `SidewalkProfile` under `RoadStyle::sidewalks`, the closure `Drawn::new` hands
  `network::stitches`): OSM maps a drive «to the pavement footway», which sits metres
  off the axis, and measured to the asphalt the drive stayed short — it
  butted into the sand ribbon with the street showing again beyond the sidewalk. If the end already lies inside a
  carrying ribbon, nothing is done. The stitched point is pulled back by
  `own half − target half` when the own ribbon is wider, so its round cap does not poke
  past the far edge; the segment is probed every metre against buildings and water (a grid
  of their AABBs, 32 m), and a drive that ends at a garage wall stays ended. The point is
  appended to the drawn path (`Stitches::apply`), so the sidewalk band follows too; where
  it landed — the target road, its segment and the nearest point on it — is kept in
  `Stitches::targets`, and that point is a junction node for the breaks, the paint and
  the turn paths (**Junctions** in `roads-junctions.md`). The
  markings see nothing of it: the end's dead-end break stands and the extension is
  past it.
- **Lane width is a world reload** (**Sections** above — the parse reads it), not a rebuild:
  the parse gets it as an **argument** (`ParseKnobs::lane_width`, built by
  `loading.rs::start_job`, `parse-seam.md`, the parse seam). The paint and the ruts still
  read a process global — `shape::lane_width()` / `set_lane_width()`, an `AtomicU32`:
  threading it through five pure-function interfaces for one number would widen them
  all. The map keeps what it was parsed with, `MapData::knobs`, the one source both
  ends read: `shape::adopt_lane_width` writes the global from it on
  `OnEnter(Playing)`, chained before `surface::retune_surface_materials` and before
  `WorldInitSet::Navmesh`; `city.rs::reload_world` fires on `lane_width_moved` — same
  city, the camera stays. The one-lane Y legs of a roundabout (`leg_sections`) are
  sections of the parse too and take the snapshot, not the global. The surface shader
  gets it as `SurfaceParams::lane_width` — the retune on `OnEnter(Playing)` is what
  makes the ruts follow a new width.
- **Smoothing off the street axis** — a bridge or a path goes through `centerline`
  with Chaikin corner-cutting (`Smoothing::Light` when the curve tolerance is above 0,
  `Off` at 0): only bends over `MIN_SMOOTH_ANGLE` (10°) are cut and the cut length is
  clamped to the road width. `passage` roads are never smoothed — their endpoints are
  pinned to building outline vertices that `arch_openings` looks the arch up by — and a
  node shared with another road is never moved. Smoothing works on a **copy** —
  `RoadLine::points` and `width` are load-bearing for the navmesh (`bridge`/`passage`
  carves), arches, tree planting and the entrance generator. **The rule itself lives in
  `map/smooth.rs`** (`Smoothing`, `smooth_path`, `smooth_pinned`, the private
  `chaikin`); six modules read it. The **variant names** `Off`/`Light`/`Strong` and the
  field name `smoothing` are values in `settings.toml` and may not be renamed; the
  type and the module may. `centerline` lives in `roads/drawn.rs` — it is the road wrapper
  that adds the arch and shared-node pins.
