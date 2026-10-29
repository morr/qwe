# Roads — bridges, asphalt wear, turn paths

The three bridge layers and their shadows, the wear that keeps asphalt from being one
flat tone (lane ruts, turn paths, the rut overlay), and the lane arrows. Code:
`map/roads/bridges.rs`, `roads/aprons.rs`, `roads/footbridge.rs`, `roads/turns.rs`,
`surface.wgsl`, `roads/ruts.rs`. The hub is `roads.md`; the lane frame the wear rides
on is `roads-markings.md` (**Markings**), the leading roads whose ruts run through are
`roads-junction-paint.md` (**Junction paint**).

- **Turn paths** (`map/roads/turns.rs`, `Turns::new(&Drawn, junctions, side)` over
  `NodePaint::junctions()` (`roads-junction-paint.md`), on the ribbon axes) — the
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
    weaker wear laid crosswise reads as «both go through at half strength» — a light
    cross, not a light square.
  - **Curve**: a cubic Bézier from lane centre at one edge to lane centre at the other,
    tangent to both (control arm a third of the chord, 0.39 of it at a quarter turn), cut
    into links by **sagitta** — a link's chord at most `SAGITTA` 3 cm off the arc, no finer
    than 3° (one link for a plain straight, four for a lane shift; 15° per link read as
    facets on the turn). Plus a **tail** of `TURN_TAIL` 5 m straight
    into the lane, **one per lane end** (`JunctionWear::tails`), however many maneuvers
    start or end there: over the tail the rut fades to nothing while the lane rut, faded
    to nothing at the gap edge over the same 5 m (`WEAR_FADE`), fades in.
  - **Drawn like a shadow**: a rut over a rut is no lighter, as a shadow over a shadow is
    no darker — after the first version (a strip per curve, alpha-blended) lit every
    crossing of two ruts and the whole middle of a crossing of equals. A strip per curve
    and per tail (`Painter::paint_turn_wear`, kind 6), the two
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
    too, like the ruts. **An `i_overlay` union of the ruts was tried and dropped** — an
    order of magnitude slower than the two passes and a million vertices; what the mask
    cannot see is the asphalt's own ruts: a turn rut crossing
    the leading road's lane rut still adds to it.
  - **The wear stays on the asphalt** (`Painter::asphalt_pieces`): `paint_turn_wear`
    takes an asphalt test of its own — `PaintGround::under_wear`, the streets'
    triangles kept near the wear links themselves (`new`'s filter with the curves
    in place of the paint marks; a junction without paint still has wear, and the
    clip has to see the asphalt under it, while a ground over **all** the triangles
    is the whole street mesh and costs far more than the build) — and cuts
    each curve and tail into the pieces that lie on it: a
    link survives only when the asphalt holds both its ends **and both ruts
    beside them** (the normal × `RUT_OFFSET`), so the strip ends square at the
    kerb. Unclipped, the outer rut of a tight turn crossed the kerb return onto
    the sidewalk tile and read as a pale diagonal band along the kerb. Pinned by
    `roads/paint/tests.rs::wear_stops_at_the_kerb` and
    `::a_wear_segment_whose_rut_leaves_the_asphalt_is_dropped`.
  - **No guide dashes**: the 1.7 marking of a far turn was built from the same curves and
    dropped at the author's call — dashed arcs across the junction read as clutter.
  - **Lane arrows** (`Turns::arrows`, `LaneArrow`) — per incoming lane its
    maneuvers: `turn:lanes` when the tag matched the arm, otherwise — only on an approach
    with `ARROW_MIN_LANES` 2+ lanes of its direction — the maneuvers the rule grants it
    above (collected before the leading-road skip, so a straight along the leader still
    counts); none on a bridge, none for a lane with no maneuver, and **no rule arrows at a
    node with a ring arm** (`on_ring`, from `Axes::rings` — «straight» there means
    «into the ring»), and **none where the
    approach has no choice** (`has_choice`: the lanes together grant fewer than two of
    left / through / right — at the split of a divided road the other half leaves as a
    U-turn; `no_rule_arrows_where_the_approach_has_no_choice`); tagged ones stay.
    `Painter::paint_arrow`
    lays them as filled polygons in the **lanes** mesh, `LineKind::Arrow` (kind 9, cover
    1, fades with the lane lines at `LANE_ZOOM_MAX` — the near zoom only): a stem along
    the travel, a head if straight is allowed, a 45° branch with its own head per allowed
    turn; 5 m long, tip `ARROW_SETBACK` 4 m behind the farthest zebra or stop line crossing
    that lane's axis within `ARROW_MARK_REACH` 30 m of the edge (`Painter::
    arrow_setback`), or 4 m from the edge with none — a zebra stands off the edge by its
    crossing's position, and a fixed setback from the edge put arrows on it. The setback is measured **along the lane**: each arrow carries `LaneArrow::back`,
    its lane's centreline from the edge back against the travel for `ARROW_BACK` 60 m (the
    drawn axis offset by the lane, `turns::lane_back`), and `paint_arrow` puts tip and tail
    on it by arc length (`map::along::place_on_path`); a straight line back from the edge
    left the lane on a curved approach. A lane shorter than that falls back to the
    straight line. **A second row** stands
    `ARROW_REPEAT` 20 m behind the first (`Painter::repeat_setback`; ГОСТ 1.18 repeats
    them): only where
    the lane's centreline runs on `ARROW_REPEAT_CLEAR` 5 m past its tail, no zebra or
    stop line crosses the lane between the rows, and the row is clear of the approach
    road's own breaks (`LaneArrow::road` → `PaintBreaks::of(road).cut`) — the centreline is the
    whole way's, and without that the row lay in the previous junction. A short block
    therefore keeps one row. The marks sit in a `Grid`
    (`paint::ArrowMarks`) — a scan over all marks per arrow was measurable. Drawn under
    their own `RoadStyle::arrows` toggle, independent of `markings`.
  The report counts `turn paths W, arrows A, leading roads L`.
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
  **A track bridge goes through the same three layers** (`Bridges::push_track` —
  two `railway=rail` ways with `bridge=yes`, `layer=1` drawn as ballast on the asphalt
  before): `mesh_roads` pushes every non-tram
  `RailLine::bridge` after the streets — a concrete slab `TRACK_DECK_COLOR` as wide as
  the ballast's foot (`rail::deck_width`, bed × `SHOULDER_SCALE`) on the track's own
  smoothed axis (`rail::track_centerline`) into `bridges`, a parapet
  `bridge_curb_width(deck)` a side into `bridge_casings`, and a shadow band into the
  shadow union, so it merges with the street bridges' shadows and rebuilds on
  `SunOnMap` with the rest of the road layers. The track itself (ballast without a
  shoulder, ties, steel) is `map::rail`'s, in its own layers above `Z_BRIDGE`
  (layers.md, **Rail layers**). One simplification against the streets: **each track way
  is its own span** — no chain gluing, always casting, ramps at its two ends — because
  OSM maps a track bridge as one way per track across the whole span; the chain logic
  stays the streets'. A tram with `bridge` is skipped — it rides the street bridge. The
  same road build also lays the **level-crossing decks** (`rail_crossings`, layers.md,
  **Rail layers**) along the ribbon axes it has just drawn.
  The decisions below make the shadow read instead of lie, and every one of them was a
  bug report first:

  - **A bridge is a connected chain of ways, not one way** (`Bridges`, `BridgeSpan`).
    OSM cuts a bridge into pieces at every tag change, and per-way the shadow lied twice
    over: the ramp below fired at every *internal* node, where the deck is at full
    height, so the shadow dipped under the deck at a fork and at the joints of a long
    footway; and `SHORT_SPAN` was applied to the piece, so a 22 m middle section of a
    185 m bridge was interrogated as a footbridge and could lose its shadow outright.
    - **Glued end to end, not by `ways_joined`.** That predicate answers «do these two
      polylines touch anywhere» — it is what decides whether a road joins a bridge for
      the curb — and here it is wrong in both directions: it would fuse **two pairs of
      footbridges that merely cross**, and a way-end landing in the *middle* of
      another bridge would turn a branch into a continuation. What is matched is endpoint
      against endpoint, at the same `JOIN_EPSILON` (0.5 m) — the tolerance is the price
      of the projection, and that part is shared.
    - **The geometry is not glued — the arithmetic is.** Two reasons, and either alone
      is enough: pieces of one bridge may differ in width (one polyline cannot carry
      two decks), and **three way-ends may meet at one node** (a fork), where the three
      pieces are not a chain at all. So a
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
    answer. Proportional height was not enough on its own: multi-way approaches carry
    `bridge=yes` and `layer=1` and on the ground are solid fill, which no tag
    distinguishes from a span. A long way is never asked — there is no 100 m embankment —
    which also keeps the probe off the bridges that would cost the most to test.
    `Underneath` precomputes an AABB per outline for that: the probe is per 2 m of deck
    and a city carries up to a thousand water outlines.
  - **The height follows the span** (`bridge_height` = `SPAN_TO_HEIGHT` 1/8 of the length,
    capped at `BRIDGE_HEIGHT` 6 m, so a span over 48 m is at the ceiling). It was a flat
    6 m, and OSM hands `bridge=yes` to far more than spans: embankment steps, the pavement
    beside a flyover, a 4 m plank over a storm drain. A 6 m shadow under a 20 m path that
    stands on level ground is the loudest lie the map can tell, because **a shadow reads
    as height** and nothing else on the map states it.
  - **The centerline is densified to `SHADOW_STEP` (2 m) first** (`densify`). The ramp
    below lives in the vertices, and most bridge ways carry exactly two
    points — so every vertex was an end, the rise was zero
    everywhere, and the shadow landed exactly under the deck, i.e. nowhere. The ways that
    did have vertices got a shadow that stepped from vertex to vertex in visible teeth.
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
    the side the sun throws to. Pinned by `roads/bridges/tests.rs::the_shadow_never_runs_past_the_abutment`,
    which asserts that no vertex of
    the layer — core, penumbra or all — lies past either end of the deck.

  **The cores of all bridges are unioned** (`i_overlay`, NonZero — the buildings' and the
  fences' construction), and that is measured, not precautionary: OSM maps a road bridge's
  pavement as a **parallel way of its own** carrying the same `bridge=yes`, and dozens of
  pairs of different bridges per city have shadow cores that overlap. In a translucent layer
  that reads as a strip of double darkness running the whole length of the bridge.
  **The penumbra is laid per bridge, before the union**, and cannot be moved after it: its
  width is `rise`, the local height of the deck over the ground, and the union output is
  contours with no `rise` on them. Dropping the taper instead is the option that costs
  more (see the previous bullet). Two neighbours' bands may therefore overlap each other —
  the price the building shadows already state and accept, since both fade to zero. A penumbra
  buried in a neighbour's core is not laid — **and that probe is filtered by the cores'
  bounding boxes**, which is not a micro-optimisation but the difference between
  a fifth of the road layer's time and nothing: the probe runs
  per quad of the band (the path is densified to `SHADOW_STEP` 2 m), and unfiltered it
  walked the ring of **every** bridge in the city, each ring twice the band's own length.
  Only neighbours on one deck ever overlap, so all but a couple of those rings are
  answered by their box. Same reject as `probe_underneath`'s `Underneath` a few bullets
  up.

  About the deck itself: a light concrete **curb** (`BRIDGE_CURB_COLOR` 0.80, 12% of the width
  clamped 0.8–2 m) under the fill in the class color — a parapet over the asphalt-grey
  deck. The 2GIS look — the curb bands along both deck edges are what makes a bridge read
  as a bridge, so the curb draws **always** — it is the one casing-like layer left
  (`bridge_casings`).
  Curb caps are always `Butt` (`push_bridge_curb`) — the deck ends
  in a square cut; a `Round` half-disc would poke a curb
  tongue past the bridge end.
  **At a ground/bridge seam every ribbon ends square** (`bridges::seam_ends`: a node where
  a road of the other level ends or passes): the deck fill (its half-disc lay over
  the approach's asphalt, and a footbridge's as a pale semicircle on the kerb line), and
  on the approach the sidewalk band and the verge; the
  approach's asphalt stays round — it is under the deck and closes the notch between two
  square ends on a kink. A free deck end, or one meeting only decks, stays round.
  **The approach's sidewalk band runs into the deck curb** (`roads/ribbons.rs::push_sidewalk_to_seams`): the
  band (22 %, 1.2–3 m) is wider than the curb (12 %, 0.8–2 m) by
  construction, so its square end stood as a step outward of the curb. Over its last
  `BAND_SEAM_TAPER` 4 m (at most
  half the road) the band's outer edge now narrows to the deck curb's outer edge,
  `bridges::seam_curb_reach` — the approach's half width plus the widest curb of a street
  deck in the seam node — on the band's own sides (`push_taper_sided`), per band piece of
  a paired half. Not under a section taper's wedge (that end is not the body's), not on a
  deck. Pinned by `roads/bridges/tests.rs::an_approach_sidewalk_tapers_into_the_deck_curb`.
  **A seam with a kink leaves no thread of ground**: a square end stands across its
  own axis, the deck's across the deck's, and at a slightly kinked head the two can be
  under a degree apart — a thread of bare ground ran along the deck head from the
  curb across the outer lane. Two lips close it from below: the approach's band runs
  `BAND_SEAM_OVERLAP` 0.5 m under the deck, no wider than the curb's outer edge (the pin
  `an_approach_sidewalk_ends_square_at_the_bridge_head` now allows that lip and nothing
  beside the deck), and every street deck lays an **asphalt apron** `DECK_SEAM_APRON`
  0.5 m deep and its own width back onto the approach, into `roads` under their paint,
  at each seam end without a wedge — where the approach's asphalt is cut square (a
  junction or a merge in the seam node) instead of its round cap reaching under the
  deck — with the curb laid the same 0.5 m back in `bridge_casings`
  (`Bridges::push_curb_lip`) where the deck has it on both sides (not a paired half).
  Pinned by `roads/bridges/tests.rs::an_approach_sidewalk_runs_under_a_kinked_deck_head` and
  `no_ground_shows_between_a_kinked_approach_and_the_deck_head`.
  **A deck takes a taper** at a seam with a lane change like any seam
  (`tapers::takes_taper` excludes arches only): the fill runs through `tapers::split`, the
  wedges go into `Bridges::fills` with the lane frame, and the curb narrows with it
  (`push_deck`'s `wedges`, `push_taper_sided`) — before, the width jumped at the head,
  covered by the deck's half-disc (2 lanes on the ground, 3 on the bridge). The price: in
  the wedge the drawn deck is narrower than the navmesh
  corridor, which is carved by the OSM width.
  **A footbridge mapped beside a street deck gets a slit apron** (`roads/aprons.rs`): OSM
  draws a bridge's pavement as its own `bridge=yes` way some metres off the
  carriageway, and through that slit the ground under the bridge showed. `slit_aprons`
  probes every
  footbridge each metre against the street decks (box prefilter, `FOOTBRIDGE_ALONG` 0.9, the
  foot of the perpendicular on a link, never past a deck end), and where the clear gap
  between the deck curb and the footbridge's curb is 0.02–`SLIT_MAX` 3 m it lays the
  strip from the deck curb's outer edge to the footbridge's fill edge into
  `Bridges::fills` in the footbridge's colour (its inner curb goes under it): one
  pavement attached to the bridge. Render only — the curb stays the navmesh's edge, the
  slit is impassable anyway. **On the ground** the same: the path leading onto that
  footbridge and the sidewalk band of the street leading onto the deck converge at the
  head and left a lawn needle between them; from each
  footbridge end at a seam, the path is probed back up to 15 m and, while the gap to the
  street's band (`Drawn::band_half`) stays under `SLIT_MAX`, paved from the street's
  asphalt edge to the path's edge in the path's layer and colour, with a 1 m lip under
  the deck apron (their square ends stand across different axes). Only the run that
  starts at the head; an overlap with the band counts as part of it (the apron lies
  under both), so a footbridge **moved out** to the deck's edge — curbs overlapping, no
  slit on the deck — still gets the ground wedge closed where its path parts from the
  band. **Beside a street on
  the ground, without a deck**: the bridge's bank is
  longer than the deck, and along the approach the footbridge ran beside the street
  itself — the triple «sidewalk, strip of bare ground, footbridge» read along the whole
  approach; the footbridge is probed the same way against every non-bridge street
  ribbon, and the gap from the street's band edge (`Drawn::band_half`) to the
  footbridge's fill edge, up to `SLIT_MAX`, is paved in the path's layer and colour.
  Pinned by
  `roads/bridges/tests.rs::a_footbridge_beside_a_deck_gets_a_slit_apron`,
  `a_moved_footbridges_path_meets_the_band_without_a_wedge` and
  `a_footbridge_along_a_street_approach_gets_a_tile_apron`.
  **A path widens into its footbridge** (`roads/footbridge.rs::push_path_flare`): the footbridge is
  3.5 m plus a 0.8 m curb a side, the path onto it 2–3 m, so at the head the path's
  square end stood as a step inside the footbridge's curb on both sides. Over its last
  `PATH_SEAM_FLARE` 3 m the path widens to the footbridge's curb reach and runs a
  0.5 m lip under it.
  **A footbridge mapped inside a street deck is moved out** (`roads/axis.rs::
  place_bridge_sidewalks`): a vertex of a footbridge running along a deck
  (`FOOTBRIDGE_ALONG` 0.9) closer than `deck half + own half + own curb` goes out to that
  distance, and its moved end nodes join the pairs' `moved` list, so the ground path at
  the node follows with a fade (`Pairs::follow_moved_nodes`, `roads-pairs-medians.md`) — a footbridge that close
  covered the carriageway edge and lay on the approach's asphalt as a pale rectangle.
  The deck sits above `Z_ROAD` so an overpass covers the
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
  a shadow. The width of the curb comes from `footprint::bridge_curb_width` through
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
  **Markings — the paint layer** in `roads-markings.md`. So the ruts stand on the very grid the paint
  lines do, a taper included.
  - **The ruts are zero-mean**: the band's share of the lane (`2·σ·√(2π) / lane width`)
    is subtracted from it, so between the ruts the asphalt is a touch darker and the lane
    on average is exactly `ROAD_COLOR`. Added as a plain brightening, a marked street was
    a few percent lighter than an unmarked drive of the same colour, and the two read as
    different asphalt wherever one ran into the other.
  - **Kerb dirt was the second effect and is gone** (7 % darker over the outer 0.7 m).
    A junction gap comes only from two *carriageways* meeting; a service drive or a
    residential lane under 8 m joins a street with no gap, and the wider street, drawn
    on top, laid its dark kerb band straight across every drive mouth. Bringing it back
    needs a gap the drive can make without cutting the lane dashes.

  **Repair patches were the third and are gone.** A 6 m cell of the *world* grid was
  hashed and, above a threshold, darkened whole; the `smoothstep` softened the hash, not
  the shape, so the patch's edge was exactly the cell boundary — a chequerboard oriented
  to the compass across any street that is not axis-aligned. It is literally the
  construction **A cell grid places a feature, it never *is* the feature**
  was written against after the same mistake on the roofs. The way back in, if it is
  wanted: the cell must be the *lane's* cell (the
  ribbon frame, so the grid turns with the road), and inside it the patch must be a
  rectangle smaller than the cell with a jittered centre and size, a crisp saw-cut edge
  and a seam — not a fill of the cell.

  The ruts fade by `visible(...)` like the rest of the surface texture, by their lane
  pitch.

  **The rut lines overlay** (`roads/ruts.rs`, Debug → Overlays → **Rut lines**,
  `DebugRutLines`; gallery row `Ruts` / `ROADS_RUTS=1`) draws where both sources of wear
  lie: every lane's axis (orange) with its two wheel lines at `RUT_OFFSET` (pale orange) —
  the shader's ruts — and the junction turn curves and tails of `JunctionWear` (green) —
  the paint layer's. The lines come out of the same build: `build_roads` records
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
  stopped across the lane at the junction edge like a seam — the roads are independent
  overlapping ribbons, so each was drawing its own wear across the other, and neither
  effect is a thing that happens: traffic fans out over a crossing and polishes nothing.
  That is only half the story now: a junction's **leading** road keeps its ruts
  through
  the node (its asphalt breaks are dropped, `NodePaint::asphalt`), and what the rest of the
  traffic polishes — each maneuver's own pair of ruts — is laid by the **Turn paths**
  above in a layer of its own, not by this shader. The gate
  costs one `smoothstep` on the wear amplitude `w`, so anything added to the block later
  fades with the ruts. The block is
  gated on `high > low`: only a ribbon with a lane frame has that (`roads::road_lanes` —
  every carriageway, a one-lane street included, whatever the Markings toggle says; the
  wear has its own knob now). Water's ribbon carries `half width, 0` there and polygons
  zeros, so neither passes; an areal fill of the same `Street` material carries no
  ribbon and stays flat — the **parking lot** among them, which shares the material and
  would otherwise have grown ruts across its stalls.
