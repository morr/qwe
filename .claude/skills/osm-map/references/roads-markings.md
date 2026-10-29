# Roads — markings, the paint layer

The lane lines, their breaks, the transverse paint. The hub is `roads.md`; the junction
side of the paint (zebras, stop lines, who breaks) is `roads-junction-paint.md`
(**Junction paint**), the turn wear and arrows `roads-bridges-wear.md` (**Turn paths**),
the asphalt the lines lie on `roads-bridges-wear.md` (**Asphalt wear**).

- **Markings — the paint layer** (`map/roads/paint.rs`, shader `assets/shaders/paint.wesl`).
  The lane lines are **geometry off the street axis**, not a pattern of the asphalt
  shader any more. The asphalt shader used to draw them from the
  ribbon's own width — `round((across + half width) / lane width)`, lanes split evenly
  from the ribbon's centre — so on a taper every line drifted with the width, and the dash
  phase restarted at every seam of two ways. Now:
  - **One lane frame** (`meshing::LaneFrame`, built by `paint::lane_frame(lanes)`): the
    grid node `origin` (on the axis for an even lane count, half a lane width off it
    for an odd one) and the carriageway bounds `±lanes · lane / 2`, `lane` =
    `shape::lane_width()`. Lane boundaries are
    `origin + k · lane` strictly inside the bounds. The **asphalt fill gets the same frame**
    (`MeshBuilder::set_lanes`, `roads::road_lanes` — every carriageway, one lane included)
    and lays its ruts on it (**Asphalt wear** in `roads-bridges-wear.md`), so the ruts sit exactly between the lines.
  - **Taper**: the frame drifts from the narrow section's to the wide one's over the
    wedge (`MeshBuilder::set_lane_taper` for the asphalt, `paint::narrow_frame(...).lerp`
    for the lines). The grid node is picked so that on the path **from the seam to the
    body** it moves by `[0, lane)` to the left — so lines both sections share stay put
    (2 → 4), and a parity change (2 → 3) slides the grid by half a lane over the taper, the
    new lane born on the right of the taper's run. **A one-way wedge adds its lanes at
    one kerb** (`paint::wedge_drift`): a two-way 2 → 4 gets a lane each
    side, which the symmetric grid is right for, but a one-way street gains or loses
    lanes at the kerb — an exit, a right-turn pocket. There the grid node is shifted by the whole difference of half
    widths, so on the body the narrow section's lanes stand against the other edge and
    the new ones grow in at the kerb (`MapData::traffic_side`); a `turn:lanes` on the wide
    way that starts with a left-only lane and does not end with a right-only one is a
    left-turn pocket, and the new lanes go to the far side instead. The axes in OSM run
    straight through such a seam, so the shared lines slide over the wedge by that shift
    rather than stay put — the only way to keep the asphalt continuous. **A line that the
    narrow section lacks is not painted on the wedge until its lane has room**: it starts
    **at full strength** where the distance to the nearer bound
    reaches `BIRTH_WIDTH` 0.8 of a lane (`paint::births` finds the station by bisection on
    the lerped frame, and the path gets a vertex there and another `BIRTH_EDGE` 2 cm
    before it on the seam side, so the GPU's alpha ramp is two centimetres long). Before
    it the line grew in from the kerb, its alpha the distance over `BIRTH_FADE` half a
    lane — a fading dash that real markings never have; `BIRTH_FADE` stays only on a
    merge ramp's frame (`roads/merges.rs`). **The axis on a widening wedge is the narrow
    section's**: a 4-lane body's double solid (`DOUBLE_AXIS_LANES`) starts at the end of
    a wedge from under four lanes, and over the wedge the axis is the single one, solid
    or dashed by the approach rule (`a_widening_paints_no_fading_lines_and_a_single_axis`).
    The markings were
    not checked against Yandex/2GIS/ГОСТ Р 52289 — the default decision stands. The
    asphalt wedge runs seam → body, so for the tail wedge
    its frame is the mirror (`paint::wedge_frames`); `the_wedge_asphalt_and_the_wedge_paint_share_one_grid`
    pins that both land on one grid.
    **Between two two-way sections the axis leads** (`paint::seam_origin`, carried as
    `WedgeEnd::origin`): the seam frame is the seam way's own frame
    (`lane_frame(seam)`, whatever the wedge's shape — the narrow neighbour on a
    narrowing wedge, the wide one on a counter-taper, `Taper::seam` — **Tapers** in
    `roads-streets-network.md`) with its node
    placed so that the wedge way's axis line lands on the seam way's axis — in the
    wedge way's frame, mirrored when the seam way is drawn against it — and the lerp
    then carries the axis to its own place over the wedge. Without it a seam of
    different splits jumped: a way with `lanes=5, lanes:forward=3` into six lanes has
    the five-lane axis on the flow border half a lane off the middle and the six-lane
    one in the middle, and the kept frame of that T-junction wedge put the whole grid
    half a lane off as well. The shift is snapped to whole half lanes (a 1e-7 tail made
    `Painter::paint`'s `ceil` add a line beyond the kerb) and taken only up to one lane;
    a one-way side, or a shift over a lane, leaves the frame to the wedge's shape rule.
    Pinned by `roads/paint/tests.rs::the_axis_runs_through_a_seam_of_different_splits`.
  - **Dashes by the street's arclength** (`paint::street_stations` over the network's
    ordered ways and the axis paths): 2 m dash, 6 m gap (ГОСТ 1.5 in town — the gap three
    times the dash; the old 3 / 3 read as a picket fence on a multi-lane street at the
    gallery zoom, Yandex draws about 1 : 2.5), and the phase runs through a seam.
  - **Solid near a junction**: the last `APPROACH` 25 m before a junction break (not a
    **quiet** one — a corner node's, `LineBreaks::quiet`, **Junction paint** in
    `roads-junction-paint.md`) — for a
    **lane line only on the approach**, in the direction its lanes flow
    (`paint::flows_forward` by the line's side of the axis and `MapData::traffic_side`, a
    one-way road forward): leaving a junction a lane line is dashed at once.
    `paint::approach_spans` finds the gap edges on the to-break profile (it is linear
    between vertices, so an edge is a zero on a link), `split_at_spans` puts a vertex at
    each span end, and the line goes out in pieces of `LineKind::Dashed` (10) and
    `LineKind::Solid` (11); the shader only draws what the kind says. The **axis** of an
    open street is split the same way into `AxisDashed` (12) / `AxisSolid` (13), but
    symmetrically — it separates two flows, so it is solid `APPROACH` on **both** sides of
    a break (`paint::near_spans`) — **and at a node the street passes through**: a
    leading road that does not yield (**Junction paint** in `roads-junction-paint.md`) gets no break there, only
    a `solid` zone in its `LineBreaks` (the reach its break would have had), and the axis is solid
    `APPROACH` either side of it (ГОСТ 1.1 at a side street).
    `Painter::paint` takes both lists as one `paint::LineBreaks { cut, solid }` — handed out
    per road by `PaintBreaks::of` (**Junctions** in `roads-junctions.md`), never assembled by hand. A **ring's**
    lines and its closed axis keep the old rule (kind 0/1 — solid by to-break alone): a
    ring's entries are not worth splitting a closed strip for. The axis of a two-way street with 4+ lanes is a **double solid** (0.15 m gap
    — ГОСТ 1.3's 10–15 cm; half a metre read as two separate lines, merging into one
    once the gap is under ~2 px); a two-lane two-way street has a
    dashed axis; a one-way street and a one-lane street have none. **The axis is the
    border between the flows** (`paint::axis_offset`), not the middle of the
    carriageway: `RoadLine::lanes_backward` lanes run against the points, the rest along
    them on the traffic side's half, so the axis lies on a lane border at
    `forward · lane − half` (right-hand traffic; mirrored for left). Without the split
    the lanes divide in half and an odd count gives its extra lane to the flow along the
    points — a three-lane two-way street gets a dashed axis, a five-lane one a double
    solid. `lane_count` and the lane frame do not move: the carriageway stays centred on the
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
  - **Transverse paint** (**Junction paint** in `roads-junction-paint.md`): a stop line is a strip across the
    lanes in the lane-line mesh (kind 3, dashed 0.6 / 0.6 m for give-way — kind 4); a
    zebra is **one quad** in its own mesh (kind 5) whose bars (1 m period, half filled)
    the shader draws by the coordinate across the road and fades by `visible()` into a
    plain light plank — so a far zebra is a mean tone, not a flicker. **A zebra is whole
    bars only** (`paint::whole_bars`): its ends step inward by
    `FIT_STEP` 5 cm until the cross-section there — both long edges of the 4 m plank
    and its middle — lies on street asphalt (`paint::PaintGround`: the triangles the
    `roads` layer has already laid, `MeshBuilder::triangles`, indexed only near the
    planks and the stop lines;
    a ribbon by its axis and half width was tried first and overstated the asphalt
    wherever the ribbon is narrower than the road — a taper head, a merge ramp — and an
    end bar lay on the sidewalk. `Carriageway` road areas are
    another mesh and are not probed), the span left takes as many whole bars as fit, and the
    plank shrinks to exactly them, centred in the span; the shader's first bar starts at the
    plank's end (`ZEBRA_FIRST_BAR`). **The ends grow outward first**: at a mouth the
    kerb — the corner fan — and on a width wedge the asphalt run wider than the road's nominal
    half width, and the plank, cut to that half width, stopped a bar or more short of the
    real kerb; the ends
    now step outward while the whole end section stays on asphalt, up to `ZEBRA_REACH` 3 m —
    past that it is the junction's field, not the crossing's carriageway — and then retreat
    inward as before, so a plank either reaches the kerb or misses it by less than one bar.
    The middle of a plank is not probed: a joined plank
    crosses a paved median or an island as before. No bar fits — no zebra
    (`roads/paint/tests.rs::a_zebra_is_whole_bars_centred_between_the_kerbs`). **A stop line is trimmed to the
    same asphalt** (`paint::trim_stop_line`): its ends step inward by the
    same `FIT_STEP` until the cross-section — the line's edges and its middle — lies on
    the street asphalt, because at the mouth of an exit the kerb of the main street
    stands slantwise in a section taper, and the line to the nominal half width lay past
    it on the verge tile.
    A line whose middle is off the street asphalt is not touched — a bridge deck's
    asphalt is another layer, and the line's own is with it; a `Carriageway` area *is*
    street asphalt and keeps the line whole. Nothing left — no line. For all three the
    ribbon's arclength runs across the road and «across» runs along it; to-break is a
    constant `NO_BREAK`.
  - **Layers**: `road_paint_wear_mask` + `road_paint_wear` at `Z_ROAD_WEAR_MASK` /
    `Z_ROAD_WEAR` (the turn paths' wear in two passes, under every line — **Turn paths**
    in `roads-bridges-wear.md`), then `road_paint_zebras` + `road_paint_lanes` +
    `road_paint_axes` at `Z_ROAD_PAINT` (above every
    street fill, **under** a parking lot — a lot laid over the carriageway hides its lines
    as it did when the asphalt shader drew them), `road_paint_islands` at
    `Z_ROAD_ISLANDS` 2.0035 (the hatched gores and splitter islands — above a lot's
    asphalt and its double line; hidden with the zebras, `PaintTag::Zebras`; **Parking**
    in `parking-big-lot.md`), and `bridge_paint_lanes` +
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
  - The report counts `paint N lines / M verts`.

  **Lane count** (`roads::lane_count`): `RoadLine::lanes`, which after the parse every street
  and drive carries (**Sections** in `roads-streets-network.md`); only a road built by hand in a test falls back
  to the width default (two-way: a lane pair per 7 m; one-way: a lane per 4.5 m), never
  more than the width allows at `MIN_LANE_WIDTH` 2.5 m. **A roundabout is no exception
  any more**: the "always one lane on a ring" rule went once the
  junction paint could break a line at each entry and the closed ribbon stopped showing its
  seam (**Ribbon** in `roads-streets-network.md`). A drawn ring takes one section for all its arcs — the widest
  arc's width and lane count (`roads/rings.rs::ring_arcs`) — see **Roundabouts** in `roads-rings.md`.
  **`lane_markings=no`** (`RoadLine::lane_markings`, `parse/tags.rs::has_lane_markings`)
  overrides the count: `Painter::paint` draws neither the axis nor the lane lines of such
  a street, however many lanes it has. The junction paint — zebras, stop lines,
  arrows — does not read it; the ruts do not either (the lane frame stays).
  **Breaks** — «to-break» is the signed distance to the nearest **marking break**
  (`meshing::Break { at, reach }`, passed as `RibbonBreaks::At`): negative inside a gap,
  so a paint line is **cut sharp** at the gap edge (`smoothstep(±0.7 px)`, the same
  antialiasing as its sides — a one-metre fade read as a blurred end)
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
  **A line does not end in a hook** (`paint::hook_breaks`): where the
  path's link at a gap edge is turned over `HOOK_BEND` 10° from its link `HOOK_REACH`
  3 m back on the visible side, the end of every line bends sideways (a smoothed axis
  turning 17° toward the continuation over the last two metres before the node leaves the
  double solid and the lane line in a crook). The line then ends at the farthest vertex of those metres where the path left
  the straight by over `HOOK_START` 2°; a street's arc turns that much over 3 m only
  below a 17 m radius. `Painter::paint` finds such edges on the profile of the cut
  breaks alone (a pocket or a throat gap is some lines' only), adds a break from that
  vertex to the edge (`HOOK_OVERLAP` 5 cm into the found gap, which `break_profile`
  merges) and re-cuts the path. A hook of a **quiet** break is quiet too — found on the
  profile without the quiet breaks it is not, and it gets no solid approach. A ring's
  closed lines are left alone. Pinned by `roads/paint/tests.rs::a_line_cut_at_a_break_ends_without_a_hook`.
