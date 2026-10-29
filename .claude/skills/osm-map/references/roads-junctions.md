# Roads — junctions, driveway crossings, turning circles, safety islands

The junction as the build sees it: the shared nodes and the five sets of breaks, the
fill order, the crossings a driveway chain becomes, the disc at a dead end, and the
island and area outlines. The hub is `roads.md`; the paint side of a junction is
`roads-junction-paint.md` (**Junction paint**), the corner pieces `roads-corners.md`
(**Kerb returns**), the row breaks of the parked cars `roads-sidewalks.md`
(**Kerb pockets**).

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
  **A stitch is a node too** (`junctions::with_stitches`, over `Stitches::targets` — **Stitches**
  in `roads-streets-network.md`): the
  loose end the network pulled onto another road's axis meets it at
  the target point, the target passes that node *inside a segment* (`Visit::inner`), and
  the end's own OSM node stops being a dead end — otherwise the stitched end joins with
  no break, no zebra and ruts fading at its OSM end, short of the street.
  These are the **asphalt breaks** — the ruts fade and the medians open on them. The paint
  layer breaks on its own set (**Junction paint** in `roads-junction-paint.md`), where a main road keeps its lines,
  and the street fill takes `NodePaint::asphalt` — these minus the ones on a junction's
  **leading** road, whose ruts run through (**Turn paths** in `roads-bridges-wear.md`). The medians keep the
  unrewritten set: a median opens at a crossing whoever leads it
  (`roads/medians/tests.rs::the_median_base_keeps_the_break_a_leading_road_lost`).
  **`Junctions`** (`junctions.rs`) is the one value `mesh_roads` asks all of this of:
  `Junctions::new(&Drawn, map, paved, style)` owns the three computations over the
  shared nodes (base breaks, `NodePaint`, row breaks). Out of them come **five sets of
  breaks per road, and merging any two is a regression, not a simplification** — each
  leaves through a door of its own and **as a type of its own**, so a consumer handed the
  wrong set does not compile: the **base** (`median_base()`, the medians', nobody rewrites
  it), the **asphalt** (`asphalt()` → `AsphaltBreaks` — the base minus the leading road's,
  the fill and its ruts), the paint **cut** and **solid** (`paint()` → `PaintBreaks`,
  `of(road)` = `LineBreaks { cut, solid }` — where lines stop, and where an axis stays
  solid through a node its road passes), and the **row** (`row()` → `pockets::RowBreaks` —
  no stitches, but service drives, taper clearings and OSM zebras: the kerb pockets and
  the cars). `RowBreaks` is an owned value that **only `pockets::row_breaks` /
  `row_breaks_over` produce** — what keeps `all_kerbsides` and `park_cars` off the base.
  What the paint *draws* leaves through `node_paint()` as views of its own, exactly like
  the breaks — `pockets()`, `marks()`, `junctions()` — so `NodePaint` owns all of its
  vectors and no field leaves the module. Pinned by
  `roads/drawn/tests.rs::row_breaks_ignore_stitches_but_paint_breaks_do_not`: a stitched side street
  is a junction for the base and the paint and a dead end for the row. `leading()` — one
  flag per road — is what the fill order sorts by. **The nodes are
  walked once**: `shared_nodes` over the row's
  participants (`pockets::is_row_participant` — everything driven on, the carriageways
  among it) gives the row, `restrict` keeps the carriageways' visits of the same nodes
  (bit for bit what a second walk would find — the node's point is its first remaining
  visit's) and `stitch` adds the stitches; the base and `NodePaint::new` take that one
  list — the nodes are the same for paint and base, because `Drawn` never moves a point
  and `is_carriageway` reads neither the width nor the class a driveway crossing is given
  (`roads/drawn/tests.rs::one_shared_node_pass_feeds_both_base_and_paint`) — only the reach of a
  break differs (`a_ring_arc_base_break_reaches_by_the_osm_width`), read off the map's
  widths. The car layer still walks its own (`mesh_cars` builds no `Junctions`): its
  `Drawn::nodal` has no stitches and no paint, so `mesh_cars` calls
  `pockets::row_breaks` itself. The one change from outside is `add_splitters` — a
  splitter island (**Roundabouts** in `roads-rings.md`) is
  found on the ribbon axes and the rings, which the
  nodes do not know, and cuts both the paint and the asphalt of its approach
  (`roads/rings/tests.rs::a_splitter_gap_reaches_both_asphalt_and_paint`); the base and the row never see it.
  Likewise three notions of «node» live here and stay three: `SharedNode::is_junction`
  (the breaks), the corner node of `corners` (a kerb return between a drive and a street,
  with no paint break) and the cluster of `node_paint::Junction` — one module, not one
  concept.
  **Junction geometry is not computed as a union**: roads are independent polylines
  drawn overlapping in one opaque layer. Originally the `Round` caps were what made a
  junction *look* joined — the caps of the ways meeting at a node overlapped into a
  rounded blob, the osm-carto way. Now an arm ending in a junction ends **square** on the
  node, and the junction's own pieces — the kerb returns and the outer corners — fill the
  rest (**Kerb returns** in `roads-corners.md`). The fill order is **narrow first, wide last** (`mesh_roads` sorts by width), so
  the main road's fill and its gapped line lie over the side street's end — and a road that
  **leads** some junction goes after all the others whatever its width: its ruts run
  through the node, and a wider side street laid over them would cut them. «After all the
  others» is **per junction** (`roads::fill_order`, a topological sort over «arm before
  its junction's leader», the width key as the priority, the first by key on a cycle): a
  side street that itself leads a junction further on used to land in the leaders' tail
  and, being wider, lay its square end over the main road — a rut-less square in the
  middle of the crossing. This is why the
  road layer must stay opaque with a world-position colour: transparency or a per-way tint
  would expose every crossing.
- **Driveway crossings** (`driveway_crossings`) — an `Alley` way under
  `CROSSING_MAX_LENGTH` 20 m whose **both** ends are ends of (non-bridge) streets is drawn
  as a `Street` at the narrower street's width: a drive–footway–drive chain drawn as an
  alley cut a strip of ground across a street entry, and the footway strip crossed the
  carriageway. A crosswalk is safe from the rule by construction: its
  ends are on pavement footways and it crosses the carriageway with an interior node.
  The substitution is `drawn: Vec<&RoadLine>` in `mesh_roads`, and everything below reads
  `drawn`, not `map.roads`.
- **Turning circles** — a `RoadNodeKind::TurningCircle` on the free end of a street
  (not shared, not a bridge or an arch) gets a disc of `turning_radius` — half the width
  × 2.2, 6–10 m — in `roads` and a sidewalk ring of the road's own
  sidewalk. An untagged dead end ends in the round cap of the ribbon anyway
  (`ROAD_JOIN` is `RoadJoin::Round`); `turning circles N` in the report, pinned
  by `a_turning_circle_widens_the_dead_end`.
- **Safety islands and carriageway areas** (`map/roads/islands.rs`, `RoadIslands`) — the
  first reader of the islands and `RoadArea` outlines:
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
    Left "to the sidewalks and alleys that already cover them", they did not: a large
    `area:highway=footway` plaza was bare ground with a service drive ending in a round
    cap in the middle of nothing.
  - **The closed line of an area is not drawn as a ribbon** (`RoadIslands::outlines`,
    one flag per drawn road, skipped at the top of the ribbon loop). The parse still
    hands `highway=*` + `area=yes` over as both an outline and a `RoadLine` (every other
    reader of `MapData::roads` keeps seeing it as before), and that line is recognised here as a
    closed ring whose points are exactly an outline's (indexed by `node_key` of the
    first vertex). Laid as a ribbon it was the ring above; a carriageway area's line
    is skipped likewise, its outline being asphalt already.
  - **Tula has almost none of it** (no island at all, one `crossing:island`, a dozen
    service-yard outlines — `references/osm-coverage.md`), so the gallery check
    is Berlin (`ROADS_CITY=berlin`, samples 4–6: `area:highway=traffic_island` at
    Rosenthaler Platz and the boulevards, `area:highway=primary|tertiary` outlines,
    signalized crossings with islands). The report counts `safety islands N + A areas,
    carriageway areas C, walkway areas W`.
