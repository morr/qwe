# Roads — junction paint

What the paint layer does at a junction: the clusters, who breaks and who leads, the
zebras and stop lines on the arms, the ring entries and exits, and the breaks the
asphalt keeps. Code: `map/roads/node_paint.rs`. The hub is `roads.md`; the break sets
it rewrites live in `roads-junctions.md` (**Junctions**), the line geometry and the
transverse paint rules in `roads-markings.md` (**Markings**), the turn wear the leading
road exempts in `roads-bridges-wear.md` (**Turn paths**).

- **Junction paint** (`map/roads/node_paint.rs`, `NodePaint::new(&Drawn, base, map, paved,
  style)` — the roads, the ribbon axes, the stitches, the merges, the mapped sidewalk, the
  pair partners and the ring arcs all come off `Drawn`; built by `Junctions::new` in
  `roads-junctions.md`
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
    between (the case: two streets joining from opposite sides 17 m apart).
  - **Corner nodes** (`is_corner`): two ways of **different streets**
    ending end to end in one node are a seam to `is_junction`, and their lines met there
    at an angle — the kerb rounded by `corners.rs` and the axis and the dashes
    drawn into a right-angled corner. Such a node is a paint node too when the drawn
    axes leave it (`PASS_CHORD` 2 m chords) at `corners::MIN_ANGLE`..`MAX_ANGLE`
    (25°–155°, the angle at which `corners.rs` rounds the kerb there — one notion of a
    corner for the kerb and the paint). Both streets pass nowhere there, so both break,
    but a corner is not a crossroads: its breaks are **quiet** (`NodePaint::quiet`,
    `LineBreaks::quiet` — no solid approach before them, `paint::Painter::paint` takes the
    approach off the profile without them), and its arms get no `ArmPlan` — no zebra, no
    stop line; an OSM crossing on the arm stays a mid-block zebra. The turn paths still
    round the corner. The 2 m chord matters: a 10 m chord read the
    smooth bend of two ways meeting mid-curve as a corner too, and the solid approaches
    then ran round a plain street bend. A collinear seam is no corner
    (`roads/node_paint/tests.rs::a_collinear_seam_of_two_streets_stays_unbroken`).
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
    at a oneway change, which once left dashes running straight through a four-way
    crossing of equals. **A crossroads of any class breaks the higher road too**: no
    road paints its lanes through the field of a real crossroads — a double solid through
    the node reads as «no left turn». A crossed main road does not
    only break its lines, it **yields** — a deliberate choice, kept on review: `crossed`
    drops its `leads`, so its own arms get stop lines and rule zebras, `Junction::leading`
    is empty (it is no longer laid over the arms in `fill_order`) and `turns.rs` gives it
    no through ruts. The bent through road is the other case: `bent_roads` breaks
    its lines and keeps its priority. **A staggered crossroads is
    one too**: two foreign arms from two nodes of the cluster, closer than the road's
    own width and heading apart on one line (within `STAGGER_ALIGN` 40°, a 10 m chord);
    two joinings farther apart than the width stay two. A **joining** street of any
    rank — one foreign arm at the node, a T, a slip — never cuts it — pinned by
    `roads/node_paint/tests.rs::a_crossing_of_any_class_breaks_the_primary_but_a_joining_street_does_not`. Signals
    aside, such a road **leads** the junction
    (`Junction::leading`), and so does a ring road (an arc of `on_ring` or a closed way —
    whether it passes or not, since OSM ends an arc at every entry) whatever the
    approaches' class — a ring has priority; an approach never leads a ring node. The leading road loses its asphalt breaks there
    (`NodePaint::asphalt`): the ruts run through, signals or not.
    **A leading road whose drawn axis bends at the node leads with its lines broken**
    (`bent`): a two-way road (every arm of its street two-way — a one-way
    has no axis to show a corner, and a divided half fans out of its fork gently) whose
    two arms do not leave the node on one line within `PASS_ALIGN` 15°
    (the echo of `SHARED_MAX_BEND`), or whose drawn path has a **kink** under the break
    (a vertex within `reach` of the node where the `PASS_CHORD` 2 m chords back and ahead
    part by more than 15°). The chord is short on purpose: it reads a corner, not a turn —
    an axis arc of `ThroughBend` (radius from 30 m) parts by a few degrees on 2 m, while a
    10 m chord from the node read the arc too and cut a secondary in the middle of its
    smooth bend; an axis that passes the pinned node straight and turns sharply a few
    metres past it also breaks. Such a road
    breaks like a yielding one (not `main through`), but it keeps its priority: no zebra and
    no stop line across it (its arms get no `ArmPlan`), the side street still yields. Pinned by
    `roads/node_paint/tests.rs::a_through_street_turning_at_a_t_node_breaks_its_lines` and
    `a_slight_bend_or_a_one_way_turn_stays_through`.
  - **Zebras and stop lines on the arms that break**: an OSM crossing on the arm (a
    `Crossing { marked: true }` node on the road, between the node and
    `ARM_CROSSING_REACH` 35 m past the junction edge — measured from the edge, since a
    wide junction's edge is itself tens of metres from the node) becomes the zebra;
    without one, `CrossingMode::Generated` puts a zebra `ZEBRA_SETBACK` 1 m past the
    edge — if the cluster joins two streets with sidewalks (by the `sidewalk=*` tag —
    `RoadLine::sidewalks`, like `kerb_parking`; the Sidewalks toggle only hides the
    band, `crossings` is the zebra's own knob), or the cluster is **signalized** and the
    arm itself has a sidewalk on both sides by its tags — one of
    the cluster's streets is at least `tertiary` (`RULE_ZEBRA_RANK`) or the cluster is
    signalized, no road of the cluster is a ring arc (`on_ring`), the
    arm is not a `*_link` and the next junction node on the same road
    lies at least `RULE_ZEBRA_ROOM` 30 m past the edge (`nodes_along`). A shorter arm is
    a link between two nodes — the branches of a fork's triangle had a rule zebra at both
    ends and a stop line between them within fifteen metres;
    the crossing is left to the outer arms. An OSM crossing ignores the room. **A
    street already crossed by the data gets no rule zebra**: a marked OSM crossing on any
    road of the same street (`street_of`) visiting the cluster, within
    `RULE_ZEBRA_DATA_REACH` (= `ARM_CROSSING_REACH`, 35 m) of the arm's node point, drops
    the rule zebra from every arm of that street — the mapper said where this junction is
    crossed, and a second zebra twenty metres from the first read as a mistake. The stop line stays;
    the other street's arms are untouched. A zebra is `ZEBRA_LENGTH` 4 m along the
    road, across the carriageway less 0.3 m at each kerb. The stop line is called by the
    same things as the zebra — a zebra on the arm, signals, a stop / give-way sign, or a
    street of at least `tertiary` in the cluster; two residential streets with no sign get
    neither. The stop line (0.4 m) stands
    `STOP_GAP` 1 m behind the zebra (or 1 m past the edge without one), across the lanes
    **coming to the node** — axis to kerb on the traffic side (`MapData::traffic_side`)
    for a two-way road, the full width for a one-way one that flows toward the node, none
    on a one-way arm leaving it. **With a zebra on the arm the line is parallel to its
    plank** (`stop_line_at`'s `align`) — on a bent mouth they parted by an angle; the
    line takes the plank's direction and keeps its own setback. **The line is trimmed to
    the laid asphalt** (`paint::trim_stop_line` — **Transverse paint** in `roads-markings.md`). A
    `give_way` sign without signals makes it dashed. **A
    stop line needs room for a queue behind it**: at least `STOP_QUEUE_ROOM` 10 m (two
    cars) of free road from its back to whichever comes first upstream — the asphalt of
    the next junction node on the road (its node point less the half width of its widest
    other road) or an OSM crossing's zebra. Shorter, and the line is dropped (the zebra
    stays): it lay on the throat between the halves of a divided street, where a waiting
    car would stand on the neighbouring junction or on the zebra across the median; 1–3 %
    of the lines per city. Ring entry lines are exempt. The
    two halves of a divided street cross on **one line**: `align_pair` moves the second
    zebra onto the first's line across the street (to the OSM one if there is one, else to
    the farther one); when **both** are OSM crossings — a `highway=crossing` node on each
    half, which mappers place a metre apart — both move to the
    line halfway between them. Over a **paved** median (`PairRun::paved`, read through
    `Pairs::partners` as `pairs::Partner { road, paved }`) the two aligned zebras then become **one
    plank** kerb to kerb (`join_zebras`: parallel within `JOIN_PARALLEL`, on one line
    within `JOIN_OFFSET` 1 m, the gap between them at most `node_paint::JOIN_GAP` 8.8 m —
    `TRAM_BED_MAX_GAP` plus `EDGE_INSET` 0.3 m off both kerbs plus `OVERLAP_SLACK` 0.2 m,
    since the widest paved median is a tram bed): the shader
    counts the bars from the plank's end, so two planks put the bars out of step at the
    seam. The median's double solid is broken there
    anyway (its breaks are the halves'). A lawn median keeps two zebras, each to its
    kerb. **Paint inside another road's asphalt is dropped**: a stop
    line, or a rule zebra, whose point on the arm lies within another cluster road's half
    width (less `EDGE_INSET`) of its axis. **The edge of an arm is where its
    cross-section leaves the other roads' asphalt** (`node_paint::clear_reach`): from the break reach (half the widest other road + 1 m) outward in
    `EDGE_STEP` 0.5 m steps until both points `half width − EDGE_INSET` off the arm's axis
    are outside every other road of the cluster (its half width off its axis; the
    arm's own street and its paired half are no rivals) **and** outside every paved
    island of a node triangle (`corners/islands.rs::small_islands`, handed to `NodePaint::new` as
    `paved`), up to `EDGE_SEARCH` 25 m. The rule zebra, the stop line and the turn paths
    and arrows (`JunctionArm::edge`) measure from it, and the lane lines break from the
    node to the outermost paint — and on an arm with **no paint at all** (a one-way
    leaving the node, an arm no zebra or stop line is called for) to the edge itself:
    at a shallow crossing the break reach is the neighbour's half width, 3 m for a
    one-lane street, while its asphalt runs a dozen metres along the arm. A **link**
    with no paint — an arm whose section never left within
    `EDGE_SEARCH` — breaks up to where its **lines** leave the other roads' asphalt
    (`lines_edge`: the same walk with the half width of `line_half`, the outermost lane
    line's offset — the lane frame less one lane, plus a two-way axis shift; none on a
    one-lane road): a branch peeling off a wide primary at a shallow angle never
    clears its section within 25 m, so its edge stayed at the neighbour's half width and
    its lane line started in the middle of the primary's lanes. The primary itself is not
    touched — its section's two side points clear the narrow branch at once
    (`roads/node_paint/tests.rs::a_fork_branch_keeps_its_line_off_the_lanes_of_the_main_road`). The lines' walk
    goes out to `LINES_EDGE_SEARCH` 60 m, not the section's 25: a shallow slip off a
    wide street takes ~36 m to get its axis out of the wide lanes
    (`roads/node_paint/tests.rs::a_shallow_slip_off_a_wide_street_starts_its_line_past_that_street`). The turn paths keep
    the arm's edge as it was. An **OSM crossing keeps its place** — it is measured
    from the reach as before: pushed past the new edge, it no longer fitted a short arm
    with its `ARM_TAIL` and was lost. **Not at a ring**: an approach
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
    section has left (the old line, at the break reach, ran over the ring's lanes up to
    the island's kerb on a tangential entry). The walk goes out to
    `RING_EDGE_SEARCH` 60 m (`leaves_ring`), not the arm's `EDGE_SEARCH` 25 m: a long
    tangential entry runs along the ring's asphalt for about thirty
    metres, the entry was not found at all, and its lane line ran over the ring toward
    its axis as a merge would, solid for the last 25 m
    (`roads/node_paint/tests.rs::a_long_tangential_ring_entry_yields_and_keeps_its_lines_off_the_ring`).
    **A ring exit has a throat** (`node_paint::Throat`, `throat_on`): for a one-way arm
    leaving a ring node, both kerbs of its section are walked out of the ring's asphalt
    the same way, and the ring road's stretch from half the exit's width before the
    node's projection (the exit's ribbon end already lies on the outer lane there, and a
    dash cut by the node was left as a stub) to the projection of the point that left
    last (the shorter way round a closed ring) breaks
    the ring's lane lines **on the exit's side of the ring's axis only** (`side`, the sign
    of the path's left normal; `Painter::paint` keys the profile by it, bits 2 and 3 of
    the pocket mask). The outer lane is the exit there, and its dashes ran straight
    across the mouth out of step with the exit's solid line; the inner lines run through.
    The throat is no approach: the ring's lines
    stay dashed before it (the solid approach spans are taken off the profile without
    the throat). An arc that ends at the node gets only that stretch before it — both
    projections fall on its end (`roads/node_paint/tests.rs::a_ring_exit_breaks_the_outer_ring_line_across_its_throat`). **A ring road never gets a
    stop line** and **always leads** its node — OSM cuts a ring into arcs at every entry,
    so an arc "passes" by street identity nowhere: every arc broke at every entry with a
    stop line across all its lanes and 25 m of solid approach lines. And an approach never leads a ring node, even where the network
    carries its street on into an arc.
    **An arm that never leaves the junction's asphalt is a link** (`JunctionArm::link`,
    `ArmPlan::link`) — a throat of a complex junction, not an approach to it: no rule
    zebra, no stop line, no arrows (the fork triangle's branches run from node to node
    across the paved island, and their stop line and arrows lay on it). A length rule (under
    30 m to the next node) was tried first and took the stop lines off every short
    approach. Zebras that land on one another — the two branches of a fork at
    one node, an OSM crossing beside a rule one — are reduced to one (`without_overlaps`,
    the last step of `NodePaint::new`, after the mid-block crossings): the OSM zebra
    stays, of two generated the first; a plank counts as inside another when a 9-point
    probe of its line falls within the other's box less `OVERLAP_SLACK` 0.2 m, so the
    halves of a divided street standing side by side keep both. The report's `zebras N`
    is the count after it. Just before it, **two OSM zebras on different carriageways
    touching end to end become one plank** (`join_touching`): their nearest
    ends within `TOUCH_GAP` (= `ZEBRA_LENGTH` 4 m), both planks within 30° of the chord
    between their far ends (`TOUCH_ALIGN`, so not side by side along the street and not at
    an angle), and not the two halves of a pair over a lawn (`Partner::paved`, read through
    `NodePaint::zebra_roads`, the road of each zebra kept beside it) — the plank runs from
    the far end of one to the far end of the other, one direction for both (a crossing
    over both halves of a divided street was two planks, each across its own
    half, offset and touching end to end — a step). The joined planks go first, so the
    duplicate zebra a seam node puts on both ways of the seam is what `without_overlaps`
    drops, not the plank (`two_osm_zebras_touching_end_to_end_become_one_plank`). The
    lines' breaks and the stop lines stay each half's own. An arm whose way ends less than `ARM_TAIL` 8 m past the paint is a
    link inside a complex junction and gets nothing. The paint break covers the edge to
    the outermost stroke plus `PAINT_CLEAR` 1 m (it was 0.5 with the metre-long fade, which
    ended the visible line about a metre out anyway; the cut is sharp now).
  - **A paint gap spills over a way end** (`node_paint::spill_over_ends`): OSM splits a
    street at a signal or a crossing, and a zebra at the very end of a short way cut the
    lines of that way only — the next way's double solid started flush with the zebra.
    Whatever
    `Walk::gap` clamps at a path end is kept as a spill (`Walk::spills`) and handed, as a
    break from that end, to the one other carriageway ending at the same point — only at a
    plain continuation, never at a junction node, where every road has its own break and a
    leader keeps its lines.
  - **Mid-block crossings**: every marked OSM crossing on a carriageway not taken by an
    arm is a zebra with a gap in the lines around it, and stop lines on both approaches
    if it is signalized. A crossing inside a break already there is skipped.
  - **The plank cuts other carriageways too** (`breaks_across_zebras`):
    the gap a zebra gets was its own road's only, and where another carriageway's
    section (its axis ± half width) passes through the plank rectangle inflated by
    `PAINT_CLEAR` — the branches of a Y-merge at a node the crossing stands a few
    metres from — their first dashes ran
    straight over the plank as short oblique stubs. Every such carriageway gets a gap
    over the interval its section spends inside; the ordinary short-runs pass closes
    what is left of a line under `MIN_RUN` after that. Arm planks do not reach the
    other arms' asphalt — they stand a metre past the arm's edge, and `PAINT_CLEAR` is
    exactly that margin — and a bridge is never cut (its asphalt is another layer).
    The pass runs before the zebras are joined and thinned: a plank later removed
    lies inside the kept one to `OVERLAP_SLACK`, a joined one covers both, and
    `PAINT_CLEAR` swallows the difference.
  - **Pocket**: when a street passes without breaking and its arm on the far side has
    fewer lanes, the lines of the wide arm that lie outside the narrow one's lane frame
    end at the junction edge (`Pocket`, one extra break for those lines only —
    `meshing::break_distances` re-measures to-break on the already cut path) — solid for
    the approach, as a turn pocket reads — instead of running into the junction.
  - **Short runs**: a run of lines under `MIN_RUN` 6 m between two breaks is closed. A
    way end that is a pure seam into a way with **fewer lanes** (`narrowing_ends`) counts
    as a break here: the lines the neighbour lacks die in the taper anyway, and a short
    street between a junction and its taper left a two-metre dash at the kerb.
  - **The median's double solid** breaks on the paint breaks as well (both halves'
    zebras and stop lines, `medians::crossing_breaks` over them), not only on the asphalt
    ones. Those breaks lie on the halves' axes, off to the side of the midline, so a long
    one covers the midline short of its end; a piece of the double solid under
    `MEDIAN_MIN_RUN` 6 m between two breaks or between a break and the run's end is
    closed as well (`medians::bridge_short_pieces`). A midline no break touches stays as
    it is. **The paved
    median keeps the full paint breaks, zebra and stop lines together** — unlike the lawn,
    which a zebra only cuts a passage through. Narrowing them to the zebra was tried
    where four crossings within thirty metres leave no piece of double solid: the line
    came back there, but the neighbouring pair's double solid ran on past a stop line
    into the junction, so it stays by rule.
  - Not drawn from data: `footway=crossing` ways are not parsed (the crossing node is
    what Tula maps). Islands and `RoadArea` outlines are drawn by **Safety islands** in
    `roads-junctions.md`.
  The report counts `junctions N (C clusters, main through T), zebras Z (O from OSM),
  stop lines S, pockets P`.
