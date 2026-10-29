# Roads — roundabouts (rings)

A ring drawn as one smooth figure: the fit, the pinned nodes, the bent approaches, the
Y-legs and their tails, the webs, the one section and the closed fill, the island lawn
and its grass. Code: `map/roads/rings.rs`, `rings/straighten.rs`. The hub is
`roads.md`; the hatched wedges a ring hatches itself are the gores
(`roads-corners.md`, **Kerb returns**, and `parking-big-lot.md`), the medians end at
them per `roads-pairs-medians.md`.

- **Roundabouts** (`map/roads/rings.rs`) — a ring is drawn as one
  smooth figure instead of its faceted OSM polyline, inside `roads/axis.rs::street_axes` (after the
  pairs are aligned), so the cars, the gallery and every layer see the same axis. Only with
  a curve tolerance above 0 (`RoadShape::curve_tolerance`); at 0 the ring keeps the OSM
  points.
  - **What a ring is.** Candidates are `RoadLine::is_roundabout` streets (tag or a closed
    one-way), not bridges, not arches. A closed way is a ring by itself; open arcs are
    chained end to start by node until they come back to the first (`chains`, at most
    `MAX_ARCS` 32 of them) — a chain that breaks off is left alone; a ring whose fitted
    semi-axis is under `MIN_RADIUS` 4 m keeps its OSM points.
  - **The figure** (`fit`): the loop sampled every metre, centre and axes by its moments,
    semi-axes by least squares in those axes; a circle when they differ by under 8 %. A
    loop is rejected (drawn as before) when an OSM vertex is further from the figure than
    `max(4 m, 35 % of the minor semi-axis)` — a long loop round a square is not a ring
    (a real mapped "egg" 14 m off its ellipse still passes: the nodes hold its shape).
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
    (`roads/turns.rs::curve`), tangent to the arm and arriving at `ENTRY_ANGLE` 25° to the ring's
    travel — inward for an entry, outward for an exit. An arm already within 5° is left
    (its heading is taken over its last `HEADING_BASE` 3 m); the arc stops `PIN_MARGIN`
    1 m short of a node the arm shares with someone else, and an arc left shorter than
    `BEND_MIN_LENGTH` 4 m is not built. **A two-way arm** is bent the same way, into the
    node **along the ray** from the centre, but only when its axis arrives more than
    `TWO_WAY_BEND_MIN_ANGLE` 30° off that ray — i.e. when the mapper led it into the node
    along the ring. Its butt end is square to the last link, and square to an axis
    running along the ring that end lay across the kerb and stuck out of it as a step on
    both sides; the island rule reads the bent axis
    (**Splitters by rule** in `parking-big-lot.md`). A radial arm is left as it is
    (`a_two_way_approach_along_the_ring_is_bent_into_it_across`).
    **A Y-approach is not two two-way arms** (`rings::y_legs`, `Rings::leg_flow`): two
    two-way ways, each at most `LEG_MAX` 55 m, that leave one node and end in two
    **different** nodes of one ring are its **legs** — an entry and an exit mapped without
    `oneway`. The entry is the leg whose ring node lies **downstream** along the ring's
    travel (one who enters turns with the travel, so the legs never cross), and each leg
    is bent as a one-way arm of its flow, by the tangent arc. Bent along the ray as
    two-way arms, both swept the whole triangle between them with asphalt and a splitter
    stood on each and hooked onto the ring. A leg is also **drawn one lane wide**
    (`roads/rings.rs::leg_sections`, a `Drawn` substitute like `ring_arcs`: `section_width` of
    one lane, `lanes` 1) — two two-way ribbons covered the whole wedge and left the
    island no room — and its wedge is the fan's (**The fan is taken whole** in
    `parking-big-lot.md`). **The other leg may be a tail of other streets** (`rings::tail_walk`):
    a lone way (group of one in `y_legs`) is still a leg when
    a walk from its far node along approach streets (not itself, not ring arcs; at most
    one change of street; within `LEG_MAX`) reaches **another** node of the same ring
    within `TAIL_ARC` (a quarter of the ring) — the shortest walk decides entry or exit
    by the same downstream rule. The tail is **two streets**: the first runs **through**
    the fork (an inner vertex of it) and the second, started on it, ends in the ring. A
    street that goes into the ring by itself is a neighbouring approach, not a tail —
    two wide two-way approaches on one big ring found such one-street "tails" along the
    ring, narrowed to a lane and opened a wedge of bare ground. The leg then goes one lane by the tangent arc; the tail
    is drawn as it is, being part of other streets
    (`the_legs_of_a_y_approach_enter_and_leave_along_the_ring`,
    `a_y_approach_whose_other_leg_is_a_tail_of_two_streets`).
  - **A tail Y is straightened in the data, not in the drawing**
    (`rings/straighten.rs::straighten_tails`, the parse's first finishing pass, before
    the sections — it moves nodes and cuts a way, and the street network is assembled
    from the ways). Drawn as found, it had no island: the fork lies off the ring's axis,
    the joint where the second street takes the tail over is a few metres in, and from
    there the full-width two-way tail runs along the ring — one smooth
    asphalt flow from the fork to the ring. No path substitute can fix that, since the
    nodes stay put in `Drawn`. So the pass runs the same `fit_rings` + `y_legs` over the
    raw points (`y_legs` hands the tail Ys back as a second list, `Tail` / `TailWalk`)
    and, per tail: the **fork and the joint merge into one node** (`fork_point`) on the
    ray through the middle of the ring arc between the two leg nodes, `FORK_SHARE` 0.7
    of the chord between them off the axis (at least `FORK_MIN` 10 m — the two «Y» of
    two ways on the same ring stand ~11 m off at a 20 m chord and their island barely
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
    or a slip road that bypasses the ring without entering it — runs **along** the ring
    outside it (within `WEB_ALONG` cos 0.7 of the
    ring's tangent) with a gap under `WEB_GAP` 2.5 m between the kerbs, the strip between
    the two axes is asphalt, pushed under the ribbons — the sidewalk used to show through
    as a crescent. Every street way near a ring is walked at 1 m steps and each such
    stretch becomes one web, so a gap spanning two ways is two webs meeting at the shared
    node. A street meeting the ring across (a two-way arm into a node) is not along it and
    gets none: that corner is a kerb return's.
  - **One section, one kerb.** All arcs are drawn at the widest arc's width and lanes
    (`roads/rings.rs::ring_arcs`), and the **asphalt is one closed fill along the ring's drawn
    axis** (`Ring::path`). The width is **one field, `Ring::width`** (the widest arc,
    set where the ring is fitted), and every consumer reads it — the arcs' section, the
    closed fill, the kerb and sidewalk (`push_ring_edges`), the approach splitters
    (`gores::splitters`) and the webs; before, each took its own source and only agreed because `ring_arcs`
    had already widened every arc. There is deliberately **no width transition** between
    a 3-lane and a 2-lane arc: the ring is one ribbon at its widest arc's width. The fill is laid in the fill order at the place of the ring's first
    arc, with the asphalt breaks of all its arcs and one rut record on the closed axis;
    the arcs lay no fill of their own (their paint, verges and ground stay per arc). An
    arc ending in a node with an approach is a junction arm, and as its own ribbon it
    ended square across its last chord: on the curve the square ends of neighbouring arcs
    fanned apart and left wedge slits along the outer kerb. A closed
    fill has no ends. The sidewalk is drawn once per ring as a closed ribbon, **outside
    only**, and the central island gets a kerb along the inner edge instead of a sidewalk
    ring (`push_ring_edges`): a `MEDIAN_KERB` 0.5 m band when a path **ends on the
    island** (`ring_island_reached` — an end of any `RoadClass::Alley` inside the ring's
    closed axis, once per ring), else a thin `RING_KERB_LINE` 0.15 m line — nobody can
    walk onto an island no path leads to, and its 0.5 m band read as a sidewalk ring.
    **The approaches' sidewalk bands
    stop at the kerb**: a band reaches its node on the ring's axis, the approach comes in
    along a tangent arc, and the band's end lay across the island as a crescent over the
    lawn. Trimming the bands opened gaps between them and the
    ring's outer sidewalk, so instead the part of each approach band beyond the island's
    kerb (`ring_island_cover`: the closed axis shrunk by the ring's half width plus the
    kerb, intersected with the bands exactly as `push_sidewalk` laid them — the same
    body, width and round or butt ends, collected in the road loop — grown by
    `ISLAND_COVER_PAD` 0.1 m) is laid again as grass in `road_medians`, over the
    sidewalks (`an_island_no_path_reaches_gets_a_kerb_line_only`). Stroking the drawn
    axis with round caps instead put bright lawn discs over the islands' own fills where
    no band reached.
    Render only — the navmesh never read the kerb or the bands. **The island is a lawn**
    (`roads/rings.rs::ring_island_lawns`, layer `ring_islands` at `Z_RING_ISLAND` 0.05): the
    ring's closed drawn axis filled with grass — the ring's asphalt covers the
    outer half, so the lawn shows up to the inner kerb. The mapped grass on an island is
    usually smaller than the drawn island and left a ring of pale ground round it. The layer sits right over the bare ground, **under** everything mapped
    on the island — grass, a park, a block, a square, water, a house stay themselves,
    and only the ground turns to lawn — which is why the parse's ground pockets take
    the island as a cover (`rings::islands`, `references/parse-geometry.md`): a pocket sown there
    is yard grass over this lawn. Over the grass it would also hide the faint rim
    of the grass polygon, but it repainted an island park as lawn too — tried and
    rejected. **The rim goes by a second layer instead** (`roads/rings.rs::ring_island_grass`,
    layer `ring_island_grass` at `Z_RING_GRASS` 0.65 — over the grass, under the sand):
    each island
    intersected (`i_overlay`, NonZero) with every `MapData::grass` polygon whose box
    touches it, laid again in the grass colour and material, so the lawn and the mapped
    grass meet with no rim between them. Only grass is laid — a park, a wood, a block on
    the island are not touched. A mapped grass patch inside a park island keeps its fill
    and loses only its inner rim.
  - The report counts `rings N (M webs)`.
