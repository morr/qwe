# Roads — merges

Where a divided street becomes an ordinary one: the merge wedges, the ramps, the
merge axis, the nose asphalt, and the mouths an impure merge is split into. Code:
`map/roads/merges.rs`, `merges/mouths.rs`. The hub is `roads.md`; the halves that merge
are found by the pairs (`roads-pairs-medians.md`, **Paired halves**), the junction
paint the merge keeps out of the node is `roads-junction-paint.md`.

- **Merges** (`roads/merges.rs`) — a node
  where a divided street becomes an ordinary one: a one-way half ends there flowing
  **in**, its pair partner starts there flowing **out**, both leave the node within 40°
  of each other (`MERGE_ALIGN`, a 20 m chord — OSM's first link may be half a metre),
  and a **two-way** way of the same `Highway` ends there leaving it the other way —
  compared by `Highway::family`, which folds `X_link` into `X`: the entry and exit
  `primary_link` off the end of a two-way primary are its fork (without the family
  test it was drawn as a crossroads with a zebra, a stop line and the entry lane
  running into the hatched island's solid).
  Bridges and arches take no part as halves; the continuation may be a bridge when the
  node is pure. **A Y-fork is a merge without a pair**: a two-way way splitting
  into an entry and an exit that `Pairs` never pairs (the branches part at once)
  counts when the node is **pure** (no other carriageway), none of the three is unpaved
  and the branches part within `FORK_ALIGN` 70° (they reach the ring's nodes 48–58°
  apart on the 20 m chord). As a junction both branches' ribbons started on the
  continuation's axis and the carriageway pinched to one branch's width with a kerb step
  on both sides; as a merge each branch continues its side of the continuation over the
  merge wedge and the island opens between them. A oneway grid corner (90°) and a fork
  with a third carriageway at the node stay junctions. At a pure merge an **OSM
  crossing** on the node is kept as a mid-block zebra across the continuation, pulled
  whole onto it (`ZEBRA_LENGTH / 2 + PAINT_CLEAR` from the node), since the rule zebras
  and stop lines go.
  **The pair is checked by streets** (`paired`, over
  `Pairs::is_paired` and `Pairs::partners`): a
  run of **Paired halves** in `roads-pairs-medians.md` between the two ways themselves, or between any way of the
  one's street and the other's street — OSM cuts a half into short ways at the node,
  and on such a piece no run forms (the first version asked the ways and found almost
  none).
  `merges(drawn, paths, nodes, runs, network)` finds them on the drawn axes, once per
  load.
  - **At a crossing the pair goes up to the crossing**: an impure merge — a carriageway
    through the node besides
    the three — is taken apart right after the pair alignment (`roads/axis.rs::street_axes` →
    `merges/mouths.rs::split_mouths`, over `merges()` on the aligned axes). Each half's
    end moves to a **mouth** on the arm of the cross street on its side: `continuation
    half − own half` off the pair's axis (the bisector of the halves' 20 m chords, not
    the continuation — that one may meet the pair at an angle, and halves forced parallel
    to it bent past the straight run), on the cross street's axis, where a vertex is
    inserted and `RoadNodes::insert` makes it a node of the half and the cross street.
    The half's axis is pushed **outward only**, up to that offset, for `MOUTH_STRAIGHT`
    16 m from the node and fading over `MOUTH_FADE` 20 m (densified at 2 m). The halves
    are then **paired up to the mouths** (`Pairs::join_mouth`, a run of zero gap — no
    sidewalk between them; their zebras in one cluster, lined up) and their medians are
    relaid
    on the moved axes (`Pairs::relay_medians`; the old midline crossed the lanes). The
    node is no merge any more — no wedge, no ramp, no nose; the three nodes (two mouths,
    the continuation's) fall into one paint cluster. Only drawn axes: the navmesh and
    the parse keep OSM. Skipped when the offset is under `MOUTH_MIN_OFFSET` 0.5 m or no
    cross arm lies within 60° of the half's side.
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
    moves a piece's end by a metre), so a short last way does not cut the wedge short.
    The side is geometric — outward is away from the partner's axis 20 m out —
    not a run's `left`, which a short last way does not have. The path is resampled
    every `MERGE_STEP` 2 m from the node and the band runs from the half's kerb
    (`MERGE_OVERLAP` 5 cm under its ribbon) outward to a reach that falls by smoothstep from the continuation's half width at the node
    to the half's own at the wedge's end, into `streets` **before** the ribbons; the
    same band plus the half's sidewalk width goes into `sidewalks` where the half has a
    tagged sidewalk on its outer side. Each side has its own step, so a kerb already in
    line with the continuation gets nothing — the wedge is **by kerbs**, not about the
    axis. The merges are `RoadReport::drawn.merges`, the bands `RoadReport::merge_edges`.
  - **The paint runs through** — three pieces:
    - **No breaks at a pure merge** (`Merge::pure` — no other carriageway at the node).
      `node_paint` saw three streets at a node (each half is a street of its own, the
      continuation a third), so all three yielded and broke by the neighbour's half width
      plus a metre. `NodePaint::new` reads `Drawn::merges`: a pure merge
      node is dropped from the junction list, its base breaks leave all three roads and a
      `solid` break goes on each — the continuation's axis is solid there. No zebra, stop
      line or turn wear. A merge on a junction (five roads) breaks as any
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
      a vertex every `RAMP_STEP` 2 m on the ramp.
    - **The axis** (`merge_axis` → `Painter::paint_merge_axis`, double from four lanes,
      single solid below): from the node up the middle between the halves, to the nearest
      `MedianEnd` of the pair's median within `AXIS_REACH` 60 m — a paved median's
      midline tip (the median's own double solid no longer breaks at a pure merge node, so
      the two meet), or a lawn kerb point less `NOSE_CLEARANCE` 1 m. With no median it
      stops where the halves' kerbs part (axes further apart than the two half widths).
      **It leaves the continuation along its tangent** (`LeadIn::of`): it starts at
      the end of the continuation's *painted* axis (the node shifted by
      `paint::axis_offset`) and heads along its last link, blending into the middles
      by smoothstep over `AXIS_LEAD_BLEND` 12 m — from the bare node straight to the
      middles it broke at the node by the angle between the continuation and the
      halves' fork, kinking the double solid right on a deck seam.
    - **A deck seam at the merge node** (the continuation on the other level): the
      halves leave the node at their own angle to the deck, so a
      dash lying on the node broke into two pieces with a hook. Each such half gets a
      **quiet** break at the node (`seam_gaps` in `mesh_roads`, added to
      `LineBreaks::cut` and `quiet`, so no solid approach before it): the deck's dash
      ends at the deck head, the half's starts past the node. Its reach is
      `SEAM_DASH_GAP` 1 m plus the continuation's half width × sin of the half's angle
      to it — the ramp starts the half's outer line on the continuation's lane grid but
      across the half's own axis, so at 20° it begins over a metre back under the deck,
      and a bare 1 m break still left a hook right at the deck head.
    - **Asphalt up to the lawn's nose** (`nose_fill`): where the halves' kerbs have parted
      and the lawn has not begun, the ground between them was bare. The outline between
      the halves' axes, from the node
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
      the ruts run between the paint lines through the node instead of fading metres
      short.
      The asphalt's profile is the paint's with one difference: on the pair side it is
      never narrower than the body. The half's ribbon runs on to the node over its
      partner, and past the paint frame squeezed toward the axis it was asphalt with no
      ruts — a smooth patch by the continuation's axis; near the node the
      grid carried on is the continuation's own, so the ruts fall where the partner's do.
  - **A four-way node of one-ways with a lawn pair is not a merge**: all four ways
    one-way — the pair turns and goes on as 3 + 4 lanes with a
    lawn, while the tram leaves. The bed ends there square at the lawn's
    nose (`bed_caps`, `roads-pairs-medians.md`), and that is the whole of it.
