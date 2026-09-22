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
  4.3 m, and the old threshold would have taken their pavements away. It sits under the **street** ribbons (1.9 / 2.0) for the
  casing reason: a crossing street's fill covers it and the sidewalk ends at the junction
  the way a real one does. It sits **over the alley** ones (1.4 / 1.5), and that is the
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
  with a butt joint. On a half with a taper the runs are not re-cut and the band stays
  full.
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
    node — and to a node shared with **another** street, which stays exactly in place (kerb
    returns, breaks and stitches find each other by it). Then the path is thinned back by
    Douglas–Peucker at `SIMPLIFY_TOLERANCE` 3 cm keeping every shared node, and the
    median's midline and the two inner kerbs are sampled off the aligned axes and thinned
    the same way; the thinning is what took the stage from +130 k vertices and +50 ms
    down to +24 k and +18 ms. Ends of two medians closer than `JOIN_GAP` 5 m are drawn
    together (`join_ends`): a half of two ways is two runs, and the gap at the seam was a
    hole in the double line and a kerb island on the «Макси» boulevard.
  - **Paved median** (gap ≤ `MEDIAN_GAP` 3 m) — `push_paved` lays a ribbon down the
    midline as wide as the axes are apart into the `roads` layer **before** the halves
    (no lane frame, so no ruts; the halves lay theirs over it), and the paint layer draws
    a **double solid** down the midline (`Painter::paint_median`, the axes mesh).
  - **Lawn** (wider) — the contour between the inner kerbs, opened by `NOSE_SHARE` 0.45 of
    the gap for a **rounded nose**, goes into the `sidewalks` layer (it shows as a
    `MEDIAN_KERB` 0.5 m kerb along each half), and shrunk by the kerb it is grass in
    `road_medians` (`Z_ROAD_MEDIAN` 1.7, `SurfaceKind::Grass`, the meadow colour). Drawn
    whatever `RoadStyle::sidewalks` says: a lawn is still a lawn.
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
    (`lanes`, else `lanes:forward` + `lanes:backward` — one direction alone is not a sum),
    else the **nearest tagged way of its street** by the distance between their middles
    along it, else `default_lanes` by class (four on primary/secondary two-way, two on the
    rest, half of that one-way, one on a service drive). Then a **lone jump** — a run shorter
    than `SPIKE_MAX_LENGTH` 60 m with the same count on both sides and another of its own —
    is cut to its neighbours. `RoadLine::lanes` is **overwritten** with the result on every
    street and drive, and `width = lanes × lane width + 2 × EDGE_WIDTH` — 3.3 m a lane on a
    street (`STREET_LANE_WIDTH`), 3.0 on a service drive, 0.5 m of edge each side: a
    two-lane street is 7.6 m, a six-lane avenue 20.8, a one-lane one-way half 4.3 — where
    the class gave 8, 16 and 16. **It is the one roads stage that moves the model**: the
    width is read by the passes after it (houses off the sidewalks, blocks and lots pulled
    to the roads), and then by bridge curbs, the navmesh's bridge corridors and the cars.
    Paths keep their class width (3.5). The gallery parses each cut window on its own, so
    a street there is inferred from the window's ways only.
  - **Tapers** (`tapers::Tapers`, in `mesh_roads`) — where two ways of one street meet at a
    **pure seam** (`RoadNodes::roads_at` = exactly those two; at a junction the step sinks
    into the junction's asphalt, and the kerb returns are built on the full width) and
    their widths differ by 0.1 m or more, the wider way's drawn path is **cut** at that end
    by `TAPER_PER_METER` 10 × the difference (at most `TAPER_MAX_SHARE` 45 % of its drawn
    length, since both ends may taper; under 1 m no taper). The cut end gets a **butt** cap
    (`push_ribbon_trimmed`, `push_street_fill`'s `trimmed`) — a round cap of the full width
    would bulge out of the taper — and the piece is laid by `MeshBuilder::push_taper`: a
    strip whose width runs linearly from the narrow way's to its own, joined by bisector
    vertices, with ribbon coords scaled to the local half width, so the lane lines fan out
    with the edges (proper lane geometry through a change of count is the paint stage's).
    The same taper is laid in the **sidewalk** band (from the narrow way's band) and, with
    casing on, in the casing. No taper on bridges, passages or under `RoadJoin::Square`.
    Drawing only: navmesh, cars and parse see each way's width as is.
  - **The network overlay** — the gallery's `Network` row (or `ROADS_NETWORK=1`,
    `examples/demos/roads/overlay.rs`): every street in its own colour, the line thicker by
    the way's lanes, a white dot on every seam of a street.
- **Markings — the paint layer** (`map/roads/paint.rs`, shader `assets/shaders/paint.wgsl`).
  The lane lines are **geometry off the street axis**, not a pattern of the asphalt
  shader any more. Until stage 3 of the roads plan the asphalt shader drew them from the
  ribbon's own width — `round((across + half width) / lane width)`, lanes split evenly
  from the ribbon's centre — so on a taper every line drifted with the width, and the dash
  phase restarted at every seam of two ways. Now:
  - **One lane frame** (`meshing::LaneFrame`, built by `paint::lane_frame(lanes)`): the
    grid node `origin` (on the axis for an even lane count, `STREET_LANE_WIDTH / 2` off it
    for an odd one) and the carriageway bounds `±lanes · 3.3 / 2`. Lane boundaries are
    `origin + k · 3.3` strictly inside the bounds. The **asphalt fill gets the same frame**
    (`MeshBuilder::set_lanes`, `roads::road_lanes` — every carriageway, one lane included)
    and lays its ruts on it (**Asphalt wear**), so the ruts sit exactly between the lines.
  - **Taper**: the frame drifts from the narrow section's to the wide one's over the
    wedge (`MeshBuilder::set_lane_taper` for the asphalt, `paint::narrow_frame(...).lerp`
    for the lines). The grid node is picked so that on the path **from the seam to the
    body** it moves by `[0, lane)` to the left — so lines both sections share stay put
    (2 → 4), and a parity change (2 → 3) slides the grid by half a lane over the taper, the
    new lane born on the right of the taper's run. A line that the narrow section lacks
    grows in from the kerb: its alpha is the distance to the nearer bound over
    `BIRTH_FADE` (half a lane). The asphalt wedge runs seam → body, so for the tail wedge
    its frame is the mirror (`paint::wedge_frames`); `the_wedge_asphalt_and_the_wedge_paint_share_one_grid`
    pins that both land on one grid.
  - **Dashes by the street's arclength** (`paint::street_stations` over the network's
    ordered ways and the axis paths): 3 m / 3 m, and the phase runs through a seam.
  - **Solid near a junction**: the last `APPROACH` 25 m before a junction break. The axis
    of a two-way street with 4+ lanes is a **double solid** (0.5 m gap, merging into one
    line once the gap is under ~2 px); a two-lane two-way street has a dashed axis; an odd
    two-way street and a one-way street have none.
  - **Geometry**: one strip per line (`MeshBuilder::push_paint_strip`, miter joins),
    `LANE_STRIP` 0.6 m / `AXIS_STRIP` 1.4 m half-width — wider than the 0.15 m line so the
    1.3 px floor and the ±0.7 px antialiasing still fit at the farthest zoom where the line
    is drawn. `ATTRIBUTE_RIBBON` here is `[across from the line, street arclength,
    to-break, kind]`, the birth alpha rides the vertex colour. `to-break` comes from
    `meshing::break_profile` — the same `GapProfile` the asphalt ribbon uses, so the
    lines stop at the same junction gaps.
  - **Layers**: `road_paint_lanes` + `road_paint_axes` at `Z_ROAD_PAINT` (above every
    street fill, **under** a parking lot — a lot laid over the carriageway hides its lines
    as it did when the asphalt shader drew them), `bridge_paint_lanes` +
    `bridge_paint_axes` at `Z_BRIDGE_PAINT` (a street's paint under an overpass must not
    lie over the deck). Material `PaintMaterial` (blend), its handle in
    `SurfaceMaterials` next to the surface ones, `MaterialSpec::Paint`.
  - **LOD**: the shader fades lane lines from 0.32 to `LANE_ZOOM_MAX` 0.4 m/px and axes
    to `AXIS_ZOOM_MAX` 0.9; `PaintLods` (the same thresholds) hides the two meshes by
    `Visibility` (`paint::show_paint`, `PaintTag` on the entity, set by
    `spawn_road_meshes` by the layer's name) — **no rebuild** at a threshold. The gallery
    does not run the ladder and relies on the shader fade.
  - **`RoadPaintStyle`** (group `road_paint`): `paint` 0–1 (0.85) — the line opacity,
    `wear` 0–0.15 (0.075) — the rut amplitude. Both uniforms
    (`surface::retune_surface_materials`), a knob drag rebuilds nothing; the Markings
    toggle of `RoadStyle` still decides whether the paint layer is built at all.
  - The report counts `paint N lines / M verts`. Tula at stage 3: see the roads plan's
    stage log.

  **Lane count** (`roads::lane_count`): `RoadLine::lanes`, which after the parse every street
  and drive carries (**Sections** below); only a road built by hand in a test falls back
  to the width default (two-way: a lane pair per 7 m; one-way: a lane per 4.5 m), never
  more than the width allows at `MIN_LANE_WIDTH` 2.5 m, and **always one on
  a roundabout** (a one-lane ring has no lines, and cutting a two-lane ring's line at every
  entry looks worse than none). **A roundabout here is `RoadLine::is_roundabout` — tag or
  shape**, and the difference is the whole rule: the tagged rings of Tula are cut into
  open arcs and got their single lane all along, while the one ring that is a *closed*
  way — ТРЦ «Макси», way 397005605, `oneway=yes` with no `junction` tag — carried
  `lanes=2`, so it was drawn with dashed lane lines and asphalt wear all the way round,
  which is also what made its seam visible (**Ribbon** below).
  **Breaks** — «to-break» is the signed distance to the nearest **marking break**
  (`meshing::Break { at, reach }`, passed as `RibbonBreaks::At`): negative inside a gap,
  so a paint line fades at the gap edge (`smoothstep(0, 1)`) and the ruts fade over 5 m.
  Junctions (`junctions::marking_breaks`) are computed **always** now, markings on or
  off — the ruts need them too. The mesher projects each break's world point onto its own
  (smoothed, merged) path — that is why a break is a point, not an arclength: the smoothed
  centreline and the OSM node may disagree — merges overlapping gaps, and inserts a vertex
  at every kink of the distance function (each gap centre and the crossover between
  neighbours) so the GPU's linear interpolation is exact. The round cap extrapolates the
  last quad's slope: a listed end goes negative, an unlisted one keeps growing — that is
  what carries the line through a seam between two ways of one road. A ribbon with no
  breaks at all gets `along + FAR_FROM_BREAKS` (dashes need a growing coordinate). The
  `Square` join falls back to `push_polyline`, which knows only the ends.
- **Ribbon** — a constant-width band along a polyline (`MeshBuilder::push_ribbon`), how
  every road, alley and kremlin wall is drawn. The `roads::push_ribbon` **wrapper** over
  it exists only to map a `RoadJoin` (the user's knob) onto the pair below, so the layers
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
  **Junction geometry is still not computed as a union**: roads are independent polylines
  drawn overlapping in one opaque layer, and `Round` caps are what makes a junction *look*
  joined — the caps of the ways meeting at a node overlap into a rounded blob, exactly
  how osm-carto gets its smooth junctions (`stroke-linejoin: round` + `stroke-linecap:
  round`). The fill order is **narrow first, wide last** (`mesh_roads` sorts by width), so
  the main road's fill and its gapped line lie over the side street's cap. This is why the
  road layer must stay opaque with a world-position colour: transparency or a per-way tint
  would expose every crossing.
- **The drawn network** (`map/roads/network.rs`, `map/roads/corners.rs`) — what the ribbons
  are laid *from* is not quite `MapData::roads`, and the difference is four render-only
  corrections, all built on **`RoadNodes`** (every node two roads of any class share, same
  5 cm key as the junctions, `junctions::node_key`). None of them moves `RoadLine::points`:
  the navmesh, doors, trees, arches and the parked cars still read OSM as it is. All four
  are counted in the `road meshing:` line, with the time spent before the first ribbon.
  - **The street axis** (`roads/axis.rs::street_axes`, stage 2 of the roads rework). The
    ribbon of every way that lies in a street ([`RoadNetwork`]) is drawn along **one curve
    per street**, not per way — per-way Chaikin pinned both ends of each way, so every
    OSM seam was a corner, and a shared node was never cut, so a through street kinked at
    every junction on a bend. Per run of a street (bridges and arches split it — their
    points are the navmesh's, and they keep the old `centerline`):
    - the ways are stitched into one polyline and **simplified** (Douglas–Peucker,
      `SIMPLIFY_TOLERANCE` 1 m), keeping the run ends, the seams and the pinned nodes;
    - every free vertex becomes an **arc tangent to both links**: the step's radius
      (`Light` 30 m, `Strong` 60 m), capped by `MAX_DEVIATION` 2 m from the vertex,
      floored by half the width (a smaller radius folds the inner edge — where the links
      are too short for it the corner counts as `tight corners` in the log line). An arc
      takes at most half of each link — and next to a pinned node at most what leaves the
      node `KERB_STRAIGHT` 12 m of straight edge (never less than ¾ of a short link):
      **a kerb return is laid only on a straight edge**, and an arc eating into it cost
      ~1000 kerb returns across Tula in the first cut;
    - a **pinned node** — one a third road touches — stays exactly in place: the kerb
      returns, the marking breaks, the stitches and the tapers all find each other by it.
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
    appended to the drawn path (`Stitches::apply`), so the sidewalk band follows too. The
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
    corner of the two facing edges is found, a circle is fitted tangent to both — of
    radius `0.6 × (half + half)` clamped 1.5–9 m between roads of one width, but only
    `MINOR_RADIUS_SHARE` 0.4 × the narrower half width when the half widths differ by
    more than `MINOR_WIDTH_STEP` 0.5 m (1 m for a 5 m drive into a street, 1.6 m for a
    residential street into an avenue). The author's call from a screenshot: at the
    shared-sum radius every drive entered its street as a wide funnel, which is not how
    a minor road meets a main one — and the wedge `[corner, tangent, arc…,
    tangent]` goes into that class's fill builder **before any ribbon** — ribbons and their
    markings then lie over it, and since it is pushed with no ribbon coords it carries no
    wear or markings of its own. Two clamps: the tangent never runs past an arm's straight
    run (past the next vertex the edge has turned), and when **both** roads carry a
    sidewalk, `r ≤ 3.4 × the narrower sidewalk` — the arc's nearest point to the corner is
    `r·(1 − 1/√2)` inside it, and beyond `s·√2/(√2 − 1)` the wedge would show past both
    sidewalks on the ground. With one sidewalk or none, a flare over the ground is exactly
    what a drive's kerb return looks like and is left alone. It is pushed as a **fan from
    the corner** (`push_convex`), which is correct although the wedge is concave: the arc
    between the tangent points is precisely the part of the circle visible from the corner.
    Its straight sides reach `OVERLAP` 5 cm under both ribbons: a side lying exactly on a
    ribbon edge without sharing its vertices rasterizes with dropouts, a dotted light
    crack along the drive edge.
    Mixed-class arms get nothing: a grey wedge over a sand
    footway would read as asphalt spilled onto the path. Bridges and passages give no arms
    (their paths go in as `None`), and under `RoadJoin::Square` no returns are built at
    all — that join is kept for comparison with the old picture. Tula: 8710.
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
        geometry, not a fallback: a minor entry (a residential street into an avenue,
        radius 1.6 m against a 3 m sidewalk) has no arc for the outer edge to follow on the
        ground either. Same for a run too short for the tangent.
      - **The asphalt wedge still lands on pavement.** `SIDEWALK_COVER` keeps the kerb
        radius under 3.4 sidewalk widths, and under that bound the sidewalk fillet's disk
        is nested in the kerb's, so the asphalt wedge lies inside the sidewalk bands and
        their fillet whatever the angle. On Tula's streets the bound never binds (1.76 m
        sidewalk → 5.98 m ceiling against a 4.8 m radius); it is load-bearing for the
        nesting, not for the radius.
      - Arms with **different** sidewalk widths cannot share one concentric arc; the
        radius then takes the wider of the two (the conservative side — a smaller radius
        keeps the asphalt wedge inside), and the two differ by at most ~0.2 m, since a
        wider gap between the half widths sends the pair to the minor branch anyway.
      - Tula: **903** of them against the asphalt's 8710 (the pairs need two sidewalks and
        a radius over the sidewalk width), ~8 k of the road layers' 387 k vertices, in the
        `road meshing:` line. Load-time only, like the rest of this module.
- **RoadStyle** (resource, BRP-writable, persisted; section `ui/roads.rs` below Buildings)
  — how road ribbons are drawn; any change reruns `rebuild_roads` (despawn
  `RoadLayerTag` layers, respawn from the unchanged `MapData`). Five independent knobs —
  **sidewalks** and **markings** (both on by default) are described above, the three
  older ones:
  - **join** — `Square` (the historical `push_polyline`: an independent quad per segment
    with *both ends* extended by half a width; no joins at all, which is what produced
    the notches on bends and the wedges at junctions), `Miter`, `Round` (default).
  - **smoothing** — `Off` / `Light` (default) / `Strong` (`map/smooth.rs::Smoothing`).
    On a street it picks the arc radius of **the street axis** (30 / 60 m, see **The drawn
    network** above); on a bridge and a path it is Chaikin corner-cutting, 1 or 2
    iterations: only bends over `MIN_SMOOTH_ANGLE` (10°) are cut and the cut length is
    clamped to the road width. `passage` roads are never smoothed — their endpoints are
    pinned to building outline vertices that `arch_openings` looks the arch up by — and a
    node shared with another road is never moved.
  - **casing** — a darker outline, its own merged layer at `Z_ALLEY_CASING` (1.4) /
    `Z_ROAD_CASING` (1.9), width `+2·casing_width` (8% of the road, 0.3–1 m). Both fills
    (1.5 / 2.0) sit above both casings on purpose: otherwise a casing would cut every
    crossing in half. Off by default.

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
  as a bridge, so the curb draws **always**, independent of `RoadStyle::casing`, and is
  thicker than a casing and lighter where the casing is darker, so the two never blend.
  Curb caps are always `Butt` (`push_bridge_curb`) — the deck ends
  in a square cut; a `Round` half-disc or the `Square` end-extension would poke a curb
  tongue past the bridge end. The deck sits above `Z_ROAD` so an overpass covers the
  street it crosses, and below `Z_RAIL` so a track on the bridge stays visible; curbs
  below fills for the casing reason (a junction of two bridge ways is never cut by a
  curb band). Street and footbridge fills share one mesh — bridge-over-bridge overlap
  is push order, rare enough not to warrant four layers. Rails carry no bridge flag —
  rail bridges are out of scope. The curb is not just paint: the navmesh blocks the
  same bands (see **Bridge curbs are impassable** in the navigation-deep skill).
- **Asphalt wear** (`surface.wgsl`, `SurfaceParams::wear`, on `SurfaceKind::Street` only)
  — what keeps a road from being one flat tone, in the **lane frame** so it follows the
  lane rather than the compass: **wheel ruts** — a polished band `RUT_OFFSET` 0.85 m
  either side of each lane's middle (a car's track is 1.5 m), `RUT_SIGMA` 0.32 m wide,
  amplitude `SurfaceParams::wear` — the **Wear** knob (`RoadPaintStyle::wear`, 7.5 % by
  default, was the shader constant `RUT_AMP`). The lane is `fract` of
  `across_from_grid_node / LANE_WIDTH` (3.3, the city's one lane width), inside the
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
  is a thing that happens: traffic fans out over a crossing and polishes nothing. The gate
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

`cargo run --example roads` shows a city's typical road junctions in a column — eighteen
for Tula (the sixteenth, `16_lanes_taper`, is a one-way primary going from four lanes to
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
  Each sample goes through the game's `parse_response` with all eight finishing passes, then
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
  gallery — it logs the `geo` to write in its place. Only Tula has a manifest; another
  city shows "no samples yet".
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
  of the map beside its side panel, not on the viewport. The rest of the recipe (capture
  twice, the map paints its tiles lazily; a live browser through Claude in Chrome, headless
  Chrome gets a `limited` stub) is in the docs of `examples/demos/roads/samples.rs`.
- The panel carries the city switch and the five `RoadStyle` rows; a change rebuilds every
  sample. `ROADS_SHOT=path.png` takes a frame and exits, `ROADS_SAMPLE=N` frames sample N.
