# Roads — paired halves and their medians

A divided street as OSM maps it — two opposite one-way ways side by side — and the
median between them: asphalt, lawn or tram bed, plus the laid tracks and the tram band.
The hub is `roads.md`; the sidewalk pieces a pair cuts on its own side are described in
`roads-sidewalks.md` (**Sidewalks**), the paint break sets the medians open on live in
`roads-junctions.md` (**Junctions**) and `roads-junction-paint.md` (**Junction paint**).

- **Paired halves** (`map/roads/network/pairs.rs`, drawing in `map/roads/medians.rs`) — a
  divided street is two opposite one-way ways side by side, and each used to be drawn as
  a street of its own: its own sidewalk on both sides, a hairline of sidewalk under a
  half-metre gap, two pavements down the middle of a lawn, and where the mapper drew the
  halves closer than their width, one ribbon over the other with its lane lines through
  the neighbour's.
  - **Finding** (`Pairs::new`, on the drawn axes of `roads/axis.rs::street_axes`, so once for the
    roads and once for the cars) — from every `PROBE_STEP` 2 m of a `pairable` road (a
    one-way `Street`-class way: a street **or a `service` drive** — never a parking
    aisle, a ring or an arch; a bridge is a half like any, below) the nearest other one
    of the **same `Highway`** running **against** it (`PAIR_PARALLEL` 0.9) and beside it
    (`PAIR_SKEW` 0.35) with `-PAIR_OVERLAP` 3.3 … `PAIR_MAX_GAP` 15 m between the kerbs.
    The overlap is a lane, not the lot's old 1 m: opposite halves never merge like lanes
    do, so an overlap is sloppy mapping and the alignment fixes it. The distance is to
    the **foot on the neighbour segment's line**, allowed `END_OVERHANG` 3 m past its
    end: to the clamped closest point, a probe near the neighbour's end saw that end
    shifted along the axis and lost the pair metres before every node and seam.
    Consecutive probes with one partner are a **run** (`PairRun`, `PAIR_MIN` 8 m), and a
    run's gap is its median. **The partner holds from probe to probe** while it is
    within `PARTNER_SLACK` 0.5 m of the nearest: at a seam of the opposite half its two
    ways stand end to end, and inside `END_OVERHANG` the nearest flipped between them
    every other probe, cutting the pair into pieces under `PAIR_MIN`. **A short way can
    be a half too** (`RunKind::Short`): a run under `PAIR_MIN` but at least half of it
    and `PAIR_COVER` 0.75 of the way's own length is taken in a second pass if the way
    shares an end with a half that found a pair — a street cut into ten-metre ways at a
    bridge; two slips meeting are no continuation of a pair and stay unpaired. One
    `Median` per pair of runs, from the half with the lower index.
  - **Queries** — what a half differs by is asked of `Pairs`, not read off its runs in
    each consumer: `beside(road, at, slack)` (the kerb returns pass two probes of
    slack; the sidewalk wedge of a half, bare on the partner's side, takes none),
    `partners(road)` as `Partner { road, paved }` (the junction paint's zebra plank),
    `is_paired(half, other)` (the merges, `roads-merges.md`), `across_median(road, path, nodes)` (the
    cross-street piece in the median's opening, **Kerb return** in `roads-corners.md`),
    `band_pieces(road, sides, stitch, total)` (the sidewalk band cut into pieces
    without the pair side, **Sidewalks** in `roads-sidewalks.md`), `has_runs(road)`, `medians()`.
    `Drawn` answers none of them itself: it hands out `pairs()`, one owner.
    **The fields are closed** — `PairRun`, `Median` and `Pairs` keep them
    `pub(super)`, so only `network/*` reads them. A median is read through
    `roads()`, `gap()`, `width()`, `midline()`, `inner()`, `is_paved()`,
    `carries_tram()`, and the one change it takes
    from outside is `Median::extend(end, along)` — the midline and both inner edges
    lengthened at one end, each along its own last link, which `medians::reach_breaks`
    uses to carry a median to the junction (the tip and its heading are
    `along::tip_of`, shared with the gores). Tests build runs with
    `PairRun::for_test`, `Pairs::of_runs` and `Drawn::with_pairs` (`Pairs::set_runs`).
  - **One door** (`medians::draw`) — every median of every pair goes through one call
    from `mesh_roads`, and every other function of `medians.rs` is private to it. It
    **paints nothing**: the double solids come back in
    `MedianDrawing::painted`, ready (midline + breaks), and `mesh_roads` hands them to
    `Painter::paint_median` — otherwise `medians` would drag in `paint.rs`; what it
    needs from neighbours comes in `MedianInputs` (three closures:
    `Merges::is_pure_node`, `Gores::reach`, `RoadNetwork::street_of`), so `medians`
    pulls in neither `gores` nor `merges`' logic. The rest of `MedianDrawing` is what
    the neighbours read further down `mesh_roads`: `paved` (the tram band, the big
    lot's edge), `lawn_kerbs` (the merge nose, `merges::nose_fill`), `ends` (the merge
    axis meeting a median, `merges::merge_axis`) and `bed_caps()` (the asphalt past a
    tram bed's end). The push order into each builder is pinned by
    `roads/medians/tests.rs::the_median_loop_lays_the_same_vertices`; `draw` itself is tested on a
    bare avenue in `medians.rs`.
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
    a vertex of the drawn axis. (Pinning every node with a carriageway instead let the
    axis fall back to OSM at each yard exit, and the kerb waved by a metre and a half.)
    A node moved by two halves — a half's end and its continuation's start — is put at
    the **mean** of the two moves, so both drawn ends stay one vertex (apart, they
    missed each other and the node was lost). **The axes are snapshotted before any
    move**: `follow_moved_nodes` walks copies of all axes taken up front — a short
    street spanning two moved nodes had each end placed by its own node, and the second
    node's fading tail, walking a *live* axis, met the first node's *new* place before
    `RoadNodes::alias` registered it and dragged that end out of the node's key cell;
    the road left the node (`RoadNodes::is_shared` is an exact cell) and the junction
    stopped seeing it (`roads/network/pairs/tests.rs::a_short_street_between_two_moved_nodes_keeps_both_node_vertices`).
    A **continuation** is a way of the same street *or* the coaxial way
    past the end (`RoadNodes::next_way`): at a crossing the network's street ends while
    the half goes on, and a fade on both sides narrowed the avenue at every crossing.
    **A bridge is a half like any** (`pairable` excludes only arches): the navmesh
    carves the bridge by the OSM points, and the alignment moves the halves *outward*,
    so the walkable part stays inside the drawn deck. Such a bridge is also **smoothed
    with its street** (`roads/axis.rs`, the `excluded` rule keeps out only unpaired bridges
    and arches), and a footway bridge mapped beside it follows it
    (`follow_bridge_sidewalks`: every vertex takes the shift of the nearest deck
    point): bridge decks lie over the streets, and the footway deck left at its OSM
    place put its tail over the moved approach as a pale tooth. **The
    median of two paired decks lies on the deck** (`medians::draw` pushes a paved median
    or tram bed whose both halves are bridges into `Bridges::fills`, its double solid into
    `bridge_paint_axes`, its share of the one tram band cover into the deck layer too
    (**Tram band** below, `split_cover`), and each deck draws **no
    curb on its paired side** where that median is paved — `push_deck` takes
    `Pairs::curb_pieces`, the sidewalk band's pieces over paved runs only (over a lawn the
    curb is the bridge's edge and stays) — and casts its shadow there only to the
    asphalt edge, `ShadowBand::reach` being
    per side **and per point of the band** (`deck_shadow_reach`: the nearest curb piece
    decides, so a half deck over a mixed median keeps the curb's shadow over the lawn
    part), else the band lay as dark plates on the asphalt between the halves. At a node
    where two halves meet (the mean above), both ends are put on **one tangent**
    (`align_seam_ends`, a vertex `SEAM_TAIL` 0.5 m in along the bisector): the ribbons
    end square to their own axis, and a 2° kink between a bridge and its approach showed
    as a notch in the kerb. **A run is judged by its chain** in `Pairs::new`: pieces of
    probes that change partner with no gap count their length together (a piece under
    `PAIR_MIN / 2` inside a chain is a sliver at a seam and is dropped) — a 16 m way at
    a seam of the opposite half saw neither piece reach `PAIR_MIN`, and the half went
    unpaired into the bridge. **Pinned** is now only a node
    the other road cannot follow to: one with another aligned half, a ring, a bridge or
    arch, or a foreign carriageway street passing straight through the crossing — on it
    lie the crossing's kerb rounds and zebras, and the half kerbs stay at their OSM
    place while the pair's gap passes from piece to piece (without the hold the
    transition dragged the axis across the crossing and took its node along with the
    half). A butt-end side street still moves with
    the half; a street or a drive — a footway pins nothing, as on **The street axis**
    in `roads-streets-network.md`. Next to
    such a node the axis is not moved at all for `PIN_STRAIGHT` 16 m and the fade begins beyond it: a
    kerb return is laid only on a straight edge, and a 10 m return to a crossing avenue
    needs its half width plus the tangent (before it the fade bent the edge from the
    node on). **Runs of one half
    closer than `RUN_BRIDGE` 12 m are one span** (`spans`): the partner changes at every
    seam of the *opposite* half, and a few probes at that seam find nothing, so a fade
    at each run end let the axis fall back to OSM for tens of metres at every seam — on
    halves mapped over each other the carriageway "breathed" by 2–3 m. Inside a
    span the target gap passes from run to run over `ALIGN_TRANSITION` (`span_gap`), and a
    point near a partner seam measures against whichever of the partner ways is nearer.
    A continuation "carries a run at the same node" when that run reaches within
    `RUN_BRIDGE` of it, and a span whose end is continued runs
    to the very end of the drawn axis without fading. The seam is looked up by the
    way's **OSM end point**, never by the end of the drawn axis: the smoothed axis is
    cut into ways at the point of the arc nearest the seam (**The street axis** in
    `roads-streets-network.md`), a few
    millimetres off the node, and `RoadNodes` found nothing there — every seam of the
    own half counted as a run end, and the axis fell back to OSM at each one;
    `a_seam_of_the_own_half_on_a_smoothed_axis_does_not_let_the_axes_go` runs
    `street_axes` with the default curve.
    **At a seam with a taper the gap changes along the wedge, not across the node**
    (`seam_blend`, `SeamWedge`): the narrow way keeps its own gap up to the node, the
    wide one goes from the narrow one's gap at the node to its own at the end of the
    fitted wedge, linearly, like the wedge itself — so both kerbs of the wedge are
    straight lines. The `ALIGN_TRANSITION` blend centred on the node used to fall on the
    same seam as the wedge, and where a lawn median ends at a change of section the
    outer kerb first went in with the closing gap and then out with the wedge — a dogleg
    (`a_gap_changing_at_a_taper_seam_changes_along_the_wedge_and_keeps_the_kerbs_straight`).
    A seam without a taper keeps the centred blend.
    **The distance is measured with the half widths that face each other, and on a
    taper that is the tapered one** (`facing_half`, fed by `street_axes` with a
    `Tapers::new` over the roads as parsed — the same joints `Drawn` finds later): a
    wedge that narrows the partner's side gives the half, at `d` metres from the seam,
    `narrow + (wide − narrow)·d/L` over the wedge length `L` (`tapers::fit`), both for
    the half being moved and for the partner at its nearest point, and the median's
    inner edges use the same halves. With the full width the wide way's axis stood
    half the width difference further from the middle than the narrow one's, so at a
    2 → 4 seam the two axes of one half missed each other: a step on the outer
    kerb and a hole to the ground at the inner corner, between the two medians and
    the wedge. Now the axes
    meet (within ~0.3 m where the two ends measure against different partner ways) and
    the wedge's inner edge runs on straight: only the outer kerb narrows
    (`a_widening_seam_of_the_own_half_meets_and_keeps_the_inner_kerb_straight`).
    Then the path is thinned back by
    Douglas–Peucker at `SIMPLIFY_TOLERANCE` 3 cm keeping every shared node, and the
    median's midline and the two inner kerbs are sampled off the aligned axes and thinned
    the same way. Ends of two medians closer than `pairs::JOIN_GAP` 5 m are
    drawn together (`join_ends`): a half of two ways is two runs, and the gap at the seam
    was a hole in the double line. **At a pure
    seam of a half** (only its two ways among the streets at the node, drawn where the
    alignment put it) the ends join up to `SEAM_JOIN_GAP` 8 m apart, with the seam within
    8 m of their midpoint: a lawn measured along the partner half ends a few metres short
    of the own half's seam. `JOIN_GAP` itself stays: raised to 6 m, it joined two
    medians across a six-arm node and grew its asphalt
    (`median_ends_meet_further_apart_at_a_seam_of_a_half_than_elsewhere`). For the same
    reason a half's sidewalk is not drawn in a gap shorter than `pairs::PAIR_SIDE_REACH`
    12 m between two runs on the same side (`Pairs::band_pieces`) — whatever the runs are,
    paved, lawn or tram bed: their medians are drawn tip to tip, and the sidewalk lay
    between them as a pale patch — **nor between the end run and the
    road's end** when that stretch is under the same 12 m: the probes lose the partner a
    few metres before the node where the halves converge, and both halves' paired-side
    sidewalks lay there as a pale wedge poking into the junction. A half with a taper takes the same pieces, shifted by the head wedge's
    length (the body starts past it), and its wedge lays no sidewalk on the paired side
    either (`Pairs::beside` at the wedge's middle).
  - **Paved median** (gap ≤ `RoadShape::median_gap`, 1–6 m, default 3; the flag is
    stored on `Median` at construction — `Pairs::new(roads, paths, median_gap, rails)` — and
    `Median::is_paved` reads it; the pair tests take the knob's default) — `push_paved` lays a ribbon down the
    midline as wide as the axes are apart into the `roads` layer **before** the halves
    (no lane frame, so no ruts; the halves lay theirs over it) **plus the contour between
    the inner kerbs** widened `FILL_OVERLAP` 2.5 m under each half (`between_edges`; at
    1 m, where a half bends hard at a node the drawn ribbon rounds the bend while the
    median's kerb runs a chord, and a half-metre sidewalk spike showed between them).
    Each kerb is widened **away
    from the kerb across**, not away from the midline: the wider half's kerb crosses
    the midline, and pushed away from it that kerb went under the *narrower* half, the
    whole contour lay under the narrow ribbon and the gap between the kerbs showed the
    ground. The same contour is
    the lawn median's asphalt and the tram bed. The
    midline is measured between the *axes*, so between halves of different widths it
    lies near the narrower one's kerb and the ribbon fell short of the wider one's where
    the gap widens toward a lawn. The
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
    drawn grass inside it. Drawn
    whatever `RoadStyle::sidewalks` says: a lawn is still a lawn. **The pieces are cut
    on a densified midline** (`lawn_outlines` over `densified`: every link split into
    parts of at most `LAWN_STEP` 1 m, alike on the midline and both edges — on the OSM
    vertices alone a straight avenue lost its lawn span by span: the first vertex past
    the junction lay inside the break and the next one sixty metres on, so the whole
    span was bare asphalt), a station
    counting while it is more than `NOSE_CLEARANCE` past every break **measured along
    the midline** (a break lies on a half's axis, off to the side, and a circle round
    it covered the midline a couple of metres short; a break more than `BREAK_ASIDE`
    12 m past the kerb is no break of this pair); the
    contour keeps only the piece's two ends and the original vertices. The
    breaks are the base ones **and the paint's** facing across both halves
    (`crossing_breaks` over `paint().of(half).cut`), **but a zebra does not end a lawn**:
    `split_zebras` takes every paint break a zebra passes through out of that list and
    turns it into a **crossing** — a break `ZEBRA_LENGTH` wide where the zebra meets the
    half's axis — and `push_lawn` cuts a **passage** (`passages`: a quad across the whole
    median, the zebra's length along the midline) out of the grass only. The kerb runs
    through it, so the pedestrian crosses the median on the island's paving between two
    pieces of lawn; a paint break with no zebra in it (a stop line at a node) still ends
    the lawn with a nose. (Two `crossing:island=yes` zebras set six metres apart along
    the axis, plus the node's break — the full paint break round a zebra is half the
    zebra, the signal's stop line and the paint clearance, nine to eleven metres —
    once left no piece at all.) A piece shorter than `MIN_LAWN_RUN` 3 m is dropped (not `PAIR_MIN`:
    a five-metre island between a zebra and a node is a normal lawn), and the nose of a
    piece shorter than the gap is rounded by `NOSE_SHARE` of its **length**, not of the
    gap — the opening erases everything under two radii. **Everything between
    the inner kerbs that is not lawn is asphalt** (`push_lawn` → `uncovered`): the
    contour between the kerbs (`FILL_OVERLAP` under the halves) minus the drawn kerbs
    inflated by `CUT_MARGIN` 5 cm, into the `roads` layer before the halves. The
    opening erases whatever is narrower than two nose radii — at a run's end, where the
    alignment only spreads the gap to the lawn's width, or where the halves converge to
    a node, that is metres of wedge — and under it lay nothing without this fill. The
    margin matters: flush with the kerb the
    difference left hairlines of asphalt, which the roads layer (above the grass) drew
    as dashes along the kerb.
  - **Pocket at a median end** (`medians::end_caps`, any median but a tram bed, which has
    `bed_caps`) — a median ends where the pair's probes ran out, and when the gap ahead
    is closed by another carriageway — a U-turn or a link between the halves — rather than
    opened by a junction, the few metres between the median's end and that road were
    nobody's. Now an end with **no
    break on its line** (the `reach_breaks` test, breaks behind the tip included — those
    ends are opened by a junction) whose ray meets, within `MEDIAN_EXTEND` 12 m, the axis
    of a carriageway from a node of either half near the tip (`MedianInputs::closer`,
    `closing_reach` — the first axis crossed, so it never paves past a cross street into
    the next median) gets a rectangle of asphalt from the inner kerbs, `CAP_WIDER` wider
    on each side — `FILL_OVERLAP` 2.5 m under the halves, like the fill between the
    kerbs; at 1 m the half's sidewalk band still lay under its ribbon there — up to that
    axis, minus the median's own
    lawn kerb, into the `roads` layer before the ribbons. Rounding the stub into an island was
    the alternative and was not taken: the island is a few metres of kerb between three
    ribbons and reads as a crumb either way. Pinned by
    `roads/medians/tests.rs::a_pocket_between_the_median_end_and_a_closing_link_is_asphalt` and
    `the_closing_reach_is_the_first_axis_across_the_ray`.
  - **Tram bed** (`Median::carries_tram`, found in `Pairs::new(roads, paths,
    median_gap, rails)`) — a run whose gap is at most `TRAM_BED_MAX_GAP` 8 m and at
    least `TRAM_SHARE_MIN` half of whose probes have a `RailKind::Tram` link within
    half the gap (never less than `TRAM_REACH_MIN` 2 m) of the midpoint between the
    axes. The rails come through a `Grid` of tram links (`Tracks`), so the probes do not
    walk every track of the city. Wider than 8 m it is a reserved track on grass
    and stays a lawn. A bed is **always paved**, the
    knob notwithstanding, and `PairRun::tram` / `paved` carry it to the zebras (one plank
    across both halves, like any paved pair). **Each bed run keeps its own gap** (its
    median, like any pair): a shared gap per chain of streets was tried and removed —
    between junctions the halves were pulled toward the middle and sprang back at every
    pinned node, the avenue narrowing and widening; the step at a seam it was written
    against is now `span_gap`'s job (**Alignment** above).
    **Drawing** — each half is widened to the middle by its own inner lane, without
    marking: `push_bed` lays the asphalt from inner kerb to inner kerb (`FILL_OVERLAP`
    1 m under each ribbon, like the paved and lawn fills — 5 cm left the ground showing
    where a half's ribbon wobbles at a seam) **as a contour, not a ribbon**, so it follows the kerbs
    where the gap wanders and ends **square** — the round cap of the old ribbon lay over
    the nose of the lawn next to it; the lawn beside a bed takes the bed's
    ends as breaks (`bed_ends`), so its nose stands `NOSE_CLEARANCE` short of them.
    Between the square end and the rounded nose nothing lay — the half's sidewalk showed
    through as a pale square (a half with a wedge keeps its paired-side sidewalk), so
    `bed_caps` carries the bed `BED_CAP` further on **minus the lawn's kerb contour**
    (`push_lawn` returns it): the grass (`Z_ROAD_MEDIAN` 1.7) lies *under* the streets
    (2.0), and a plain extension would have eaten the nose. An end with **no lawn
    kerb near** is carried on too, whole, `BED_CAP` plus the bed's width and `BED_WIDER`
    1 m wider than the kerbs on each side. (The sidewalk in a short gap
    between two runs is the general rule of **Paired halves**, **Alignment** above.) No
    lane frame, so no ruts over the tram lane. The double solid runs **down the middle**,
    between the tracks (as 2GIS draws it). What makes the tram lane read is the
    **tram band** below, not paint. (The first version
    drew the bed as dark asphalt with a double solid along **each edge** — the GOST
    reserved track — and read as «a dark corridor with no rails»; the Yandex picture —
    plain asphalt with one double solid — was chosen instead.)
    **Why the axes are not moved.** Shifting each half's axis Δ/2
    towards the middle with the width grown by Δ, so the outer kerb stays put, was
    prototyped and dropped: everything that finds a node by a vertex of the
    drawn path — `corners/gather.rs::kerb_returns` (`is_shared` at 5 cm), the zebras' crossing
    marks, the ends of `node_paint` and the stitches — would have lost the nodes on
    every half, and pinning the axis at each cross street would have made
    the tram lane die and be born every 150–300 m. The contour between the inner kerbs
    is the same picture — outer kerb, sidewalk, cars and lane frame all untouched by
    construction — with the model and the drawn axis left alone. `RoadReport::drawn.medians`
    is `[paved, lawn, tram beds]` (`Pairs::count`).
  - **Laid tracks** (`roads/tram_lay.rs`) — the tram tracks **as drawn**: OSM puts
    them where the mapper did, while the street is drawn smoothed and a pair is pulled
    apart from its middle, so the OSM track wandered off its street. Each
    `RailKind::Tram` track is probed every 2 m (`densify`); a probe **on a tram bed**
    (`Median::carries_tram`, the reached-to-breaks `MedianDrawing::paved`, within the bed's
    half width + `BED_SLACK` = one `TRACK_SPACING`) moves to the median's middle, the very
    line the double solid is painted on, ± `TRACK_SPACING` 3.2 m / 2 on its own side of
    the other track (the nearest one across, more than `PARTNER_MIN` 0.5 m away; alone — on
    the middle). A probe **along a single street** (drawn nodal axis link along it,
    `ALONG_MIN` 0.8, inside half width + 0.6 m) moves by what the drawn axis moved there
    (foot on the drawn link − nearest point of the OSM axis), so the mapper's offset from
    the axis stays. Off the asphalt the shift fades over `TRANSITION` 20 m; a gap under
    `BRIDGE_MAX` 40 m between two shifted pieces (a junction where the bed breaks) is
    crossed with the shift blended from one edge to the other; the result is thinned at
    5 cm. `build_roads` returns them as `RoadBuild::tram` (`TramTracks`, one per tram track, map
    order; `as_mapped` in the raw-OSM mode) — the tram band is laid over **them**, and
    `rebuild_roads` / `spawn_map` insert them as a resource the tram layer draws
    (`tram::rebuild_tram`, gated on `resource_changed::<TramTracks>` too, so a shape knob
    moves the tram with the road). Pinned by
    `roads/medians/tests.rs::tram_tracks_in_a_bed_lie_symmetric_about_the_drawn_middle`.
  - **Tram band** (`roads/tram_band.rs`) — a lighter strip of asphalt (`TRAM_BAND_COLOR`,
    `ROAD_COLOR` lighter by about 8 %) `TRAM_BAND_WIDTH` 3.3 m wide along every tram
    track that lies **under a street's asphalt**: on a bed and on a single street with
    the tram down its axis alike. A track is
    probed every 2 m against a `Grid` of the drawn street links (street decks included,
    no arches) and of the paved medians; a probe is covered when a link runs **along** it
    (`ALONG_MIN` cos 0.8 — a tram crossing a street is a level crossing, not a lane),
    the foot falls on the link (±`LINK_SLACK` 1 m) and the strip fits inside that
    asphalt (`EDGE_SLACK` 0.6 m). An uncovered stretch between two covered probes of one
    track is **bridged** when it is under `BRIDGE_MAX` 40 m and every probe of it lies within
    `BRIDGE_SLACK` 4 m of some link's asphalt in any direction: tracks between the halves
    of a divided street cross a junction where the median has ended and each half's
    ribbon is 3 m short of them. Open
    ground between two pieces of street is farther than that from any link and still cuts
    the band. Covered runs shorter than `MIN_RUN` 12 m are dropped.
    Laid into `roads` **after every ribbon and carriageway area**, butt-ended, without a
    lane frame; the paint is its own layer above, so the lane lines and the double solid
    lie on top of it. The pieces are laid as **one cover** (`band_cover`): each stroked
    `CLOSE_GAP` 3 m wider and half of it longer at each end, unioned (`i_overlay`,
    NonZero) and inset back by half the gap — a closing, so edges and ends stay where they
    were while any gap under 3 m between neighbouring bands fills (plain asphalt
    left between a turning track and the straight one, between two
    tracks' pieces cut at different probes, read as dark holes in the light band).
    **The cover is one across a ground/bridge seam and is cut by
    the decks that carry it**: `deck_cuts` strokes, butt-ended, every street
    deck (`2 × curb_reach`) and every paved median of two decks (its width + 0.2 m, so
    the halves' inner lanes are cut by their own decks) along which a band piece runs
    (`ALONG_MIN` — an overpass across the tram cuts nothing, or the ground band under it
    would be lifted over it), and `split_cover` intersects / subtracts them: the ground
    part goes into `roads`, the deck part into `Bridges::fills`, and one line — the deck
    head — cuts both. Built apart, the two covers overlapped with
    a shift, and the deck cover's closing ran back over the approach, hiding the
    end of its double solid under `Z_BRIDGE`; on a street deck without a median the
    band stopped at both heads over the double solid. `RoadReport::tram_bands` counts the pieces. Only the
    streets whose box touches a cell of a tram link are indexed, and a covered run is
    thinned back by Douglas–Peucker at 5 cm — one vertex per 2 m probe is far too many.
  - **Where it opens** — `crossing_breaks`: only a junction break of one half **facing** a
    break of the other (within the axes' distance plus both reaches; the axes stand
    `Median::width` — the gap between the **inner edges**, not between the axes — plus
    both halves' half-widths apart; reading it as the axes' distance left a narrow cross
    street between two three-lane halves not opening the median) — a crossing
    street, a U-turn link, a zebra's footway. A street into one half does not open the
    median: the far half runs past, and the double solid runs past with it. At such a break the lawn stops `NOSE_CLEARANCE` 1 m short of it, and
    `reach_breaks` carries the midline and the kerbs on to the break centre (at most
    `MEDIAN_EXTEND` 12 m) so that the double solid dies at the break edge like the lane
    lines do. Toward a gore at a ring the line is trimmed and
    reached by `Gores::reach` (`MEDIAN_GORE_GAP` 0.6 m short of the hatching).
  - **Cars** need nothing: a one-way half parks one row on its driving-side kerb, i.e.
    away from the partner, and the row stands on the aligned axis. **Navmesh** does not
    see any of it — `RoadLine::points` never move.
