# Parsing — the geometry passes

The geometry half of the parse: the way readings and the passes that move or cut way
geometry (`parse/tags.rs::path_width`, `parse/rails.rs`, `parse/verges.rs`,
`parse/pavements.rs`, and the geometry passes of `parse.rs`, `parse/lots.rs`,
`parse/pockets.rs`). The seam they hang on — `ParseKnobs`, `read_elements`/`finish_parse`,
the pass order, how a parse is tested — is `parse-seam.md`; the building readings and
passes are `parse-buildings.md`.

## Reading

- **Path width** (`parse/tags.rs::path_width`, in `parse_way`, not a pass — it reads only
  the way's own tags). A path's width is not one class constant: a `width=1 surface=mud`
  trail beside a street is not a boulevard, and the park's alleys, trails and yard paths
  are not all one width. A plausible
  `width` tag decides (`PATH_WIDTH_READ` 0.5–12 m — `0` and `0.3` are notes, a larger
  value is the width of a square — clamped to `PATH_WIDTH_RANGE` 1–8 m, a thinner trail
  being a thread nobody sees); without it the kind and the surface: `pedestrian` 5,
  `track` 3, `footway=crossing` 3, `steps` and `footway=sidewalk` 2.5, `cycleway` 2, a
  trail surface (`dirt|ground|earth|mud|grass|sand|woodchips`) 1.5, other `path` 2,
  other `footway` 3. **Footbridges and arches keep the class 3.5**: their band is the
  corridor the navmesh carves (the navigation-deep skill) and a `width=1` footbridge
  would narrow a river crossing to a tile. Pinned by
  `parse/tests.rs::a_path_takes_its_width_from_the_tag_or_its_kind`.

## Finishing passes

- **Rail gaps stitched across a street** (`parse/rails.rs::stitch_rail_gaps`, step 1
  right after the drowned buildings) — OSM routinely breaks a spur exactly at the road it
  crosses: two `railway=rail` ways stop at the two kerbs and the crossing itself is not
  mapped (`level_crossing` nodes are not even in the query). Two **free** ends (no vertex
  of another track within 0.5 m — an end on a switch is not a break; the end of a track on
  a bridge, `RailLine::bridge`, is never one — a track bridge does not cross at grade) of
  tracks of the same `RailKind` (never a tram) are
  joined into one way when the gap is under `STITCH_GAP_MAX` 30 m, each end looks at the
  other within `STITCH_ANGLE_MAX` 12° (heading taken 8 m back, the last OSM link is noise)
  and the gap crosses the axis of a road the track can cross at grade — `rail::crossable`,
  the level crossing's own predicate: a street or a service drive, not a bridge, not a
  passage. Nearest pair first, one per
  loop, the first way hosts the joined points, `service` kept from either. The street
  condition is load-bearing: without it, station stubs facing each other over a platform
  get stitched (Berlin alone would stitch 37). Skipped in raw
  OSM; logged when non-zero.
- **Sidewalks of untagged streets** (`parse.rs::infer_sidewalks`, right after the drowned
  buildings, which must not count) — a residential / unclassified / living street with no
  `sidewalk*` key (`SidewalkSide::Inferred` — the tristate is the whole "who is asked")
  keeps its band only where the blocks along it average at least
  `SIDEWALK_STOREYS_MIN` 3 storeys (`cars::district::Districts::storeys_at`, probed every
  `SIDEWALK_PROBE_STEP` 40 m; nothing around counts as low) and loses it to `[None; 2]`
  otherwise; an unpaved one lost it already in `tags.rs::untagged_sidewalks`. The pass is
  `infer_sidewalks(&mut roads, &Districts)` — the measure is its explicit input
  (see the seam, `parse-seam.md`), and the three tests build the `Districts` straight from
  the scene's houses (`infer_sidewalks_drops_the_band_only_among_low_blocks`
  and its two neighbours). Before
  the house pull and the block pull, so both push and pull against the sidewalk that is
  drawn. The rule itself — `roads.md`,
  **Sidewalks**; logged as `N of M untagged residential streets left without sidewalks`.
- **Sidewalks left to a separate footway** (`parse/verges.rs::measure_footways_beside_streets`,
  right after the pavements pass, whose answer it reads) — OSM maps a sidewalk either as a
  `sidewalk*` tag or as its own `footway` along the kerb, and the street then should say
  `sidewalk=separate`. Often it does not, and the rule band lay a metre from the mapped
  `footway=sidewalk` — two parallel sidewalks with a strip of grass between. The pass asks
  every **carriageway** (any class, the arterials
  too — `infer_sidewalks` asks only residential ones) for each **`Inferred`** side: probes
  every `SEPARATE_PROBE_STEP` 5 m, and a probe hits a side when a **paved** path
  (`RoadLine::is_paved_path` — a sand trail beside the street is not its sidewalk) runs
  parallel (`|cos| ≥ SEPARATE_PARALLEL` 0.85) on that side, its axis between
  `SEPARATE_INSIDE` 1 m inside the kerb and `SEPARATE_REACH` 4 m past the outer edge of
  the band (the nearest such footway per probe is kept). The side goes only if the
  **median lawn** — kerb to the footway's near edge (axis minus the path's half width) —
  is at least `SEPARATE_LAWN` 1.5 m. A footway closer than that keeps the band, which
  then lies under it (both are pavement, the overlap is invisible): taken away, it left a
  metre of bare ground between the kerb and the footway, and at a corner a hole down to
  the ground framed by the kerb returns. The same pass then gives every side
  of a paved carriageway whose probes found such a footway — the probe window reaching
  `VERGE_REACH` 10 m past the kerb for this (`VERGE_REACH_TWO_WAY` 16 m on a two-way
  street: its footways can sit 8–15 m out and only half the probes find them within
  10 m, so the side had no verge and bare ground lay between
  the band and the footway; a half of a divided street keeps 10 m, past which on its
  inner side lie the other half's footways), band or no band — a **verge**:
  `RoadLine::verges`, the median distance from the kerb to the footway's axis, and
  **its profile** `RoadLine::verge_profile` — `(metres along the points, verge)` at every
  probe that found a footway, a **slanted** link included (`VERGE_SLANT`, cos ≥ 0.5 —
  counted for the profile only, not as a footway alongside): the footway turns at the
  corner and drifts off the street, and a verge of one width left a wedge of ground
  between itself and the footway. `RoadLine::verge_at(side, along)`
  reads it — linear between probes, the end values past them, the constant median with
  no profile. Only the
  renderer reads it (**Sidewalks** in `roads.md`: sidewalk tile under the greens). At least `SEPARATE_SHARE` 60 % of the probes → the side becomes `None`. A
  `Tagged` side is never touched. **A piecewise drop was tried and rejected**: a side
  with a footway along part of it losing the band only there left bare ground between
  the kerb and the footway wherever the pass had set no verge (the verge is laid for a
  side only past the share) — drop the side whole. And what reads as a sidewalk beside a
  bridge approach may be the approach band's half-disc past the deck curb
  (`roads.md`, **Bridge layers**), not a footway at all. Before the house and block pulls
  like the other sidewalk
  pass, so both work against what is drawn. The consequences are the ones `sidewalk=separate`
  already had: no band means no kerb pocket on an arterial side (`pockets::kerb_parking`)
  and no rule zebra there. Logged as
  `N of M untagged sidewalk sides left to a separately mapped footway in T`; pinned by
  `parse/tests.rs::a_footway_along_the_kerb_takes_the_inferred_sidewalk_of_its_side`,
  `a_sidewalk_stays_unless_a_paved_footway_runs_beside_it` and, through the whole parse,
  `a_mapped_sidewalk_beside_an_untagged_street_replaces_its_band`.
- **Pavement of untagged footways** (`parse/pavements.rs::infer_pavements`, beside the
  sidewalks pass; its place is free — the greenery it reads never moves and only the
  renderer reads the answer). `RoadLine::pavement` leaves `parse_way` from
  `tags.rs::tagged_pavement`:
  `surface` decides (`asphalt|paving_stones|concrete|sett|paved|metal|wood|…` → `Paved`,
  the `untagged_sidewalks` dirt list plus `woodchips|grass_paver` → `Unpaved` — the two
  share `surface_pavement`); without it the kind does — `footway=sidewalk|crossing`,
  `steps`, `pedestrian`, `cycleway` paved, `path` and `track` unpaved; a bare `footway`
  stays `None`. The pass then probes each `None` path every `PAVEMENT_PROBE_STEP` 10 m
  against parks, woods and grass (a `Grid` of their boxes, `GREEN_CELL` 100 m): more than
  half the probes in green → `Unpaved`, else `Paved`. **A link between paved paths stays
  paved**: a footway the probes sent to the sand whose two ends (not a ring) are both
  vertices of paved paths — tagged, paved by kind (`steps`, `pedestrian`) or paved by the
  probes — is set back to `Paved`, in one pass (a link standing on another such link stays
  a trail). The spokes of a ring island run over
  the lawn from the paved ring footway to the steps of the central square; drawn as sand,
  they left the steps as paved stubs with round caps on it. Keys are the
  points in centimetres; logged as `…, N more are paved between paved paths`. Pinned by
  `a_footway_on_a_lawn_between_paved_paths_is_paved` and
  `parse/tests.rs::a_footway_is_paved_by_its_tag_its_kind_or_the_greenery_around`. How
  it is drawn — `roads.md`, **Paved paths**.
- **Blocks pulled to the roads** (`parse.rs::pull_areas_to_roads`, step 6) — the same mismatch
  as the house pull, read from the other side. The road's width is a class constant and its sidewalk is the
  renderer's, while a `landuse` block is traced along the red line or the plot fences, so
  between the yard and the drawn sidewalk a strip of bare ground is left showing. On a
  photo a yard runs up to the
  kerb, and a seam of ground beside the pavement reads as an unpainted layer.
  - **The vertex moves, not the block**, and it moves **under** the asphalt: a vertex whose
    gap to the road's **mapped edge** (`SidewalkProfile::mapped_edge(width / 2)`: half
    the width plus the band by class when the road has a sidewalk on at least one side —
    one reach for both sides, a one-sided `sidewalk=right` included) is within `LANDUSE_GAP_MAX` 5 m is pulled to the road's axis until it
    stands `LANDUSE_OVERLAP` 0.5 m inside the band. The overlap is not decoration: the
    ribbon is drawn from the *smoothed* centreline while the gap is measured on the raw OSM
    points, and without it a bend keeps a centimetre of seam. Beyond the limit nothing is
    done — that is a real gap (a front garden, a verge, a right of way), not a seam.
    The limit went up from 3 m on the author's look at the
    frame: at four and five metres the strip of ground along a street still reads as a
    seam rather than as a verge.
  - **A corner at a crossing goes under both streets** (`pull_corner`): a vertex just
    pulled under one band is slid along that street under the second one, when that one
    lies across (the slide gains at least half its length on the second's normal),
    outward from the fill and within `LANDUSE_CORNER_GAP_MAX` 8 m. Such a corner may start
    that far off — it stands back from the crossing on the bisector, farther than its
    edges do from either street — and a vertex beyond `LANDUSE_GAP_MAX` moves only if it
    finds the second street. Pulled toward the nearest street alone, the corner kept a
    triangle of bare ground by the kerb return. The grid of links is padded by the larger limit.
  - **Green only grows**, and that single rule is what keeps the pass safe: a vertex moves
    only when the move leads **outward from the fill**, which is read **locally off the
    ring** — the ring's own signed area gives the side, so the outer contour grows away
    from the block and a hole shrinks into itself (the green lies outside a hole's ring).
    A street running inside a block would otherwise drag its boundary inward, while a
    street through a courtyard correctly pulls the **hole's** edge to the asphalt — there is
    no green in the hole, and it is its rim that has to reach the road.
    Locally, rather than by asking whether the road lies outside the block: for a strip of
    lawn between two streets the nearest street is the one **beyond the far edge**, and that
    answer would squeeze the strip instead of stretching it. `point_in_area` also has
    nothing to say where the projection lands exactly on the outline — which is every road
    that ends against a block, and was two of the four corners of the courtyard test scene.
    Everything drawn on a block lies above it (`Z_LANDUSE` 0.25 against `Z_SIDEWALK` 1.6,
    parks and grass at 0.5–0.7), so the part that ends up under the road, under a park or
    on a neighbouring block is never seen; only the closed seam is.
    **One exception moves a vertex inward**: the nearest road is a **walkway**
    (`RoadClass::Alley` — a sidewalk mapped as its own footway) lying inside the fill
    within `SIDEWALK_TUCK_MAX` 3 m, and a carriageway lies outward within
    `LANDUSE_GAP_MAX`. The vertex then goes **to the footway's axis** — not
    `LANDUSE_OVERLAP` past its edge: the block edge is straight between its vertices
    while the footway bends, and half a metre of margin left a hair of yard along the
    footway. Berlin
    draws its blocks to the kerb and maps the sidewalk as a footway inside them, while
    our carriageway is narrower than the real one, so the yard stuck out from under the
    sidewalk as a dark crescent at every rounded corner. The strip between a sidewalk
    and the kerb is paving, not yard: it is left to the verge.
    **A vertex lying under another walkway** — the block's corner under the crossing
    path the sidewalk meets at the junction — is tucked the same way, to the nearest
    walkway it is *not* under: left where it was, it stayed off the sidewalk while
    its neighbours went under it, and the edge between them stuck out as a wedge of yard.
    A vertex under a
    carriageway is still left alone — under what is **drawn** of it, though.
    **A vertex in the bare side's verge** (`parse.rs::in_bare_verge`, `Edge::gap`): the
    mapped edge is one for both sides, so beside a street with a sidewalk on one side
    only, a vertex beyond the kerb of the side without one (`sidewalk=separate|no`, the
    `Edge::bare` flag) but within the other side's band counted as «under the road» and
    stayed. That strip is the verge up to the footway, and the verge is drawn **under**
    the blocks (`Z_ROAD_VERGE` < `Z_LANDUSE`), so the yard stuck out of its tiles. Such a
    vertex goes to the **nearer drawn rim**: the footway
    (tucked, when `tuck` allows) or the kerb (0.5 m under the asphalt, outward only). The
    gap to the drawn rim of a bare side (`Edge::gap`) is used there and in `tuck`'s
    «street beyond» — and nowhere else: measured everywhere, the nearest road flipped
    from street to footway on vertices outside the band as well, and the edges came out
    zigzag between the two. Pinned by
    `a_block_edge_in_the_verge_of_a_bare_side_goes_to_the_nearer_of_its_rims`. What the
    vertex rules still miss the area cut below takes.
  - **Slivers of yard on the verge tiles are cut out by area** (`parse/verges.rs::
    cut_verges_from_blocks`, right after the vertex pull, before the lots). The rules
    above decide **per vertex**, and the edge between two decided vertices is straight:
    at a footways' fork one vertex of the hook can stand outside the phantom band while
    its neighbours go under the footway, and a sliver of yard stays on the tiles. So the
    block is asked as an area: it is
    intersected (`i_overlay`, NonZero) with the **paved strip** of every verge — from
    the street's axis to `paved_verge(verge_at)` past the kerb (`roads.rs::paved_verge`,
    the drawing's own rule, now `pub(crate)`), less `VERGE_CUT_INSET` 0.5 m: the verge's
    rim is the footway's axis, so a cut edge lies under the footway ribbon, and the half
    metre absorbs the smoothing of the drawn axis. Of that intersection only the **thin**
    pieces are subtracted — mean width `2·area / perimeter` under `SLIVER_WIDTH_MAX`
    1 m (`is_sliver`). **Subtracting the whole strip was wrong**: a block drawn to the
    kerb lies on the verge as a band of yard for tens of metres, and it reads as the lawn
    between kerb and sidewalk it is; cut, it turned into a band of concrete. A wide piece
    is the data saying «lawn», a sliver or a hair along a footway is an artefact of the
    vertex rules. **Only the tiles** are asked: the lawn of a verge wider than 4 m is the
    yard's own grass (`VERGE_YARD_COLOR` = `RESIDENTIAL_COLOR`). The strip is built by
    `verge_rings` — the same raw-point construction the ground pockets take as their
    verge cover (at full width there). A block with no sliver keeps its rings as they
    were — no re-tracing, no drift; one cut in two becomes two `PolyArea`s of the same
    kind (a part under `MIN_BLOCK_PART` 1 m² goes). Nothing reads `landuse` by index, so
    the split is free. Blocks are cut across threads (`map/parallel.rs::in_parallel`); the log
    line says how many blocks lost a sliver and what the cut cost. Pinned by
    `a_sliver_of_block_on_the_verge_tiles_is_cut_back_under_the_footway`.
  - **The band is what is drawn**: a street's sidewalk counts in its reach only when it
    has one (`RoadLine::sidewalks`). `sidewalk=separate|no` used to count anyway, and the
    block was pulled under a sidewalk that is never drawn — its edge stood past the kerb.
  - **A long edge beside a road is split first** (`LANDUSE_STEP` 8 m, and only where the
    grid has a road near the edge): between its own two vertices an edge is straight while
    the road bends, and on the outside of a turn the seam would stay in the middle of the
    edge, where there is nothing to move. An inserted point that found nowhere to go is
    dropped again, so a ring does not collect vertices for nothing.
  - Bridges and passages give no segments: a block is drawn under a bridge anyway, and an
    arch through a house is not the edge of a yard. Everything else that is drawn does,
    alleys included — a footpath with a seam of ground beside it reads the same way.
  - **The parking lots reach their roads in the same step, by a different construction**
    — `parse/lots.rs::pave_lots`, a **polygon closing**, its own line of step 6 right
    after `pull_areas_to_roads` and the pilot
    of the **pass context**: it takes `PaveLotsCtx` — references to the eight fields it
    reads and the one `parking` it writes — not `&mut MapData`.
    - **The vertex pull was here first and was removed.** Each point of the outline
      looked for its own road — the farthest one outside within the limit — and
      neighbours moved by different amounts. On a big lot that left a **tooth of ground
      beside every aisle stub** where the drive was past the limit, **needles of asphalt**
      toward a far road, unfilled wedges where the ring crossed itself (`untangled` kept
      only the largest contour), and holes in the blob two thin lots and their drive
      merged into. None of
      it is a threshold to tune: a ring whose vertices move independently has no way to
      stay smooth. Do not bring the pull back for lots.
    - **The construction.** `base` = the lot ∪ the bands of the streets beside it
      (`road_pieces`: a road is cut every `ROAD_PIECE` 3 m and a piece is taken while its
      band is within two closing radii of the lot, so a long street does not drag asphalt
      along its whole way; pieces of one road are glued across links into one stroke;
      `RoadClass::Street` only — a footpath is not what a lot is entered from; the band
      of a carriageway includes its sidewalks — only those it draws, never a
      `sidewalk=separate|no` one). **A kerbside lot** (`parking=street_side`,
      `LotKind::Kerbside`) takes the carriageway band **without** the
      sidewalk: the closing then fills the sidewalk strip between it and the kerb, and the
      pocket is cut into the sidewalk instead of standing behind it — a lay-by separated
      from the lanes by a strip of drawn sidewalk has nothing to drive in from. Only the street the pocket **runs along** loses its
      sidewalk (`runs_along`: a link within `STREET_SIDE_ANGLE` 30° of the outline's
      longest side, `PolyArea::longest_side`); a cross street at the pocket's end, inside the closing radius,
      keeps its band whole, so the pocket stops at the edge of that sidewalk instead of
      paving its corner to the kerb (`a_street_side_lot_leaves_a_cross_street_sidewalk`). `closed` = `base` offset **out by the
      radius and back in** (`i_overlay` `outline`, round joins, `ARC` 0.3). `closed − base`
      is what the closing added, and a piece of it is kept only if it lies **between the
      lot and a road** (`between`: a vertex within `TOUCH` 0.25 m of the lot's rings — or
      of a **drive**, a street with no sidewalk, whose asphalt is the lot's own — and a
      vertex on some road band). A notch in the outline (the lawn in the corner of an
      L-shaped lot) touches no road; a wedge between two streets touches no lot. Kept
      pieces grow by `LANDUSE_OVERLAP` 0.5 m (bevel join) so the edge goes under the
      ribbon drawn from the smoothed centreline — **and only there**: the grown ring is
      intersected with the road bands. Grown on every side, a piece also stepped out
      half a metre along its *free* edge (the closing's arc, a house wall, a lawn), and
      wherever that edge met the lot's own there was a half-metre jog. The largest shape of lot ∪ pieces is
      the new outline, **holes included** — the island of a roundabout at the lot's edge
      stays ground.
    - **The apron** (`Around::walls`, `BUILDING_APRON` 1 m) — last, and on every lot
      whether it grew or not: **every** house near it, of any size, swollen by the apron
      and subtracted. OSM draws a lot overlapping a house or flush with its wall, and the
      closing pulls asphalt into the slit between them, so stalls stood in the wall.
      `KEEP_BUILDING_AREA` is a
      different question — what the *added* asphalt may not crawl over — and stays. A
      booth in the lot gets its island; a house across a lot cuts it, and then the parts
      of `MIN_LOT_PART` 30 m² and up all stay lots (the first in place, the rest appended
      to `MapData::parking`). `comes_near` keeps the boolean off lots no
      house reaches. Pinned by `a_lot_steps_back_from_the_houses_on_it`.
    - **`CLOSING_RADIUS` 7 m** — a gap under 14 m closes: a stall row with its aisle
      (`STALL_DEPTH` 5.2 + `AISLE` 6) and a little.
      **`GROUND_CLOSING_RADIUS` 12 m** on a big lot (`is_big`: the **drawn** outline ≥
      `GROUND_MIN_AREA` 8000 m² — the radius is chosen before there is a paved one): its
      perimeter drive stands 15–20 m off, and all of that strip is the lot's asphalt on a
      photo — exactly the pockets the vertex pull was written down as not reaching.
    - **The lot kind is settled last** (`settled_kind`): once every lot is paved, a lot
      that is not `Kerbside` becomes `LotKind::Ground` if its **paved** outline reaches
      `GROUND_MIN_AREA`, else `Yard` — the outline that is drawn is the one the big lot's
      kerb and the layout stand on, and a part a building cut off is judged by itself
      (`a_lot_paved_past_the_threshold_is_a_big_lot`). The render reads the kind
      (`parking::is_ground`) and measures nothing.
    - **What the asphalt does not crawl over** is subtracted from the kept pieces, and
      `between` is asked again: a building of `KEEP_BUILDING_AREA` 100 m² or more (a
      hardware shop has a yard behind it; a ticket booth *in* the lot does not), greenery,
      water — and **fences**, as a band of
      `FENCE_HALF` 0.75 m either side of the line. A fenced lot's outline lies along its
      `barrier`, so the band separates the added piece from the lot, the piece no longer
      touches it and is dropped; a fence standing a few metres off cuts the piece in two
      and each half fails one side of `between`. Both cases are pinned:
      `a_fenced_lot_stays_behind_its_fence`,
      `a_lot_does_not_step_over_a_fence_it_was_not_standing_on`.
    - **A piece an obstacle cut must still hold a band as wide as the fence**
      (`REMNANT_HALF_WIDTH` = `FENCE_HALF`, `fits_band` — the remnant shrunk by 0.75 m
      does not vanish). The subtraction runs **per piece**, and a piece that lost more than
      `CUT_AREA` 0.1 m² to it keeps only such remnants: what the cut leaves narrower than
      that is a crack between the fence and the road band, and it still touches both the
      lot and the road, so `between` passes it — a grey spike of asphalt on the sidewalk.
      `a_fence_pocket_at_a_lot_end_leaves_no_asphalt_sliver`. An **uncut** piece is not
      tested: a thin strip between the outline and the band is exactly what the closing is
      for.
    - **The fill as a whole goes through an opening** (`THIN_FILL_HALF_WIDTH` 1.125, so
      anything that cannot hold a 2.25 m band is cut) — the same sliver question as the
      remnant rule above, but **before the pieces are split**, because the sliver may be
      glued to an honest piece and no per-piece filter sees it. The closing's sliver in
      the corner between two bands — the **collar** at a drive's mouth (a hair of lot
      asphalt across the street's drawn tiles, past the kerb; the drive-loophole of
      `between` let it through, since its vertices stand on the drive band) — and the
      thin **tongue** along a kerb both read as asphalt butts over foreign paving. The honest
      lot-to-road strips are wider and live; a `street_side` pocket is **exempt** —
      reaching the kerb across the sidewalk is its contract
      (`a_street_side_lot_reaches_the_kerb_across_the_sidewalk`). Pinned by
      `a_fill_thinner_than_the_remnant_stays_unpaved`.
    - **The piece thresholds are only as good as `ring_area`**, and until the shoelace
      was taken from the ring's first vertex (`model::signed_ring_area`) they were not:
      in map metres `x · y` reaches 2·10⁷, where an `f32` step is 2, so a 0.07 m² crumb at
      the same lot's south corner measured exactly 1.0 and passed `MIN_PIECE_AREA`.
    - **The road band itself is not part of the lot** — a lot is entered *from* that
      road, and the layout stripes whatever the outline holds. **Except a drive with the
      lot's asphalt on both sides of it** (`sandwiched`, probed every 3 m at `BESIDE`
      0.4 m past either edge of the band, butt ends): without that a lot falls apart into
      strips along its own drives and no row fits any of them.
    - **What it changed besides the edge, and it is a correction**: the vertex pull went
      to the *farthest* road, so a pocket or a strip lying against a street was stretched
      **over the street** and striped there. Lots shrank to their honest area; what
      puts stalls back on such a strip is the layout's pocket rule (**Parking** in
      `references/parking-layout.md`).
    - **Cost, and why it is threaded.** A lot costs half a dozen `i_overlay` calls, and
      the price of a call is the call, not the geometry — ≈ 0.3 ms even on a four-vertex
      lot — so a city's lots cost seconds single-threaded. Lots are independent, so
      `pave_lots` hands them to the map's one fan-out
      (`map/parallel.rs::in_parallel` — `available_parallelism` scoped threads taking lots
      off a counter, results applied in order — the output does not depend on the
      split). **Release buys nothing here** — the time is
      `i_overlay`'s and a dependency is optimised in `dev` too; see the same point under
      **Cost** in `references/parking-big-lot.md`.
      Two things that were measured on the way: subtract
      obstacles from the *kept* pieces, not from everything added, and filter them by the
      pieces' bounds (the river's outline otherwise goes into a boolean for every lot on
      the embankment); glue road pieces into one stroke per road, not per link.
  - Order: after the houses are pulled off the sidewalks, before door generation. It could
    stand anywhere in the tail — neither `landuse` nor `parking` reaches the navmesh, the
    doors, tree planting or the parked cars' districts — and it is the only pass here whose
    effect is purely what is drawn. **After the squaring** (step 4) it must stay, though:
    `vertex_uses` counts a parking outline among the layers a house may share a vertex
    with, so a pulled edge would change which houses get squared.
- **Ground pockets sown with grass** (`parse/pockets.rs::fill_ground_pockets`, into
  `MapData::pockets`) — its own step right after the block and lot pulls, with its own
  `osm parse:` line and timing. What it closes: a scrap of bare ground **enclosed** by what
  is drawn, which no tag describes and the vertex pull cannot reach — the wedge's corner
  stands on the footways' junction, where the block has no vertex to move — no vertex rule
  can fill it.
  - **The rule is a hole of the union.** Covers: every road but bridges and arches as a
    band of its mapped edge (`SidewalkProfile::mapped_edge`, so a one-sided sidewalk
    counts on both sides — over-cover is the safe error) in pieces of `RUN` 16 links,
    **bevel joins and square caps** (a round join is a dozen points, and the union's price
    is points: round → bevel took the whole pass from 229 to ~120 ms on Tula); each
    verge side (`RoadLine::verge_at`) as a band from the axis to the kerb plus the verge,
    every `VERGE_STEP` 5 m (the profile's own probe step); the outlines of blocks,
    parks, woods, grass, sand, water, lots and pitches; and **every ring island** — the
    raw loop of a roundabout (`roads::rings::islands`: the same chains of arcs and the
    same figure test `rings::reshape` fits, so exactly the islands the render fills with
    `ring_island_lawns`). The island is lawn whole, and its interior was a hole of the
    union that touches the ring's band: sown as yard grass, it lay over the lawn as a
    dark ragged blot almost the size of the island. A loop the figure test rejects (a
    long road round a square) gets
    no lawn and stays an ordinary cover-less interior. Pinned by
    `a_ring_island_is_not_a_ground_pocket`. **Houses are left out**: a hole
    that runs under a house only gets grass under the house.
  - **A hole is sown** when it is at most `POCKET_AREA_MAX` 400 m² (above that it is a
    plot of its own — a waste plot, a building site), touches
    a paved road's band (`TOUCH` 0.25 m — a hole cut into a block by its own
    multipolygon has no road on its rim) and touches **no** dirt path or unpaved street
    (a waste plot crossed by trails). The ring grows
    `LANDUSE_OVERLAP` 0.5 m (bevel) so the seam goes under the smoothed ribbons.
  - **Which grass** (`Scene::grass_near`, the verge lawn's rule, not a third one): a
    vertex within `POCKET_NEAR` 4 m of a **block's** ring gives that block's kind (a
    block's edge often lies *under* the footway, and the wedge beyond touches the footway,
    not the block); else one within 4 m of a mapped **park or lawn** (`parks`, `grass`)
    gives `Grass` — the meadow, drawn without a rim in the lawn layer, as the verge beside
    them (`roads.rs::Meadows`) — and **merges into that very green** (`merge_into_green`):
    the grown hole and the green's ring are unioned, and the result replaces the green.
    Sown beside it, the pocket lay on the green's rim kaima with its half-metre growth,
    and where the sowing ended the cut kaima read as a seam — two light lines converging
    in a V; the merged green's kaima
    stays only along its true outer edge, against the block and the road. The merge goes
    to the one green whose ring the nearest pocket vertex answers for (`Rim::near`, parks
    before lawns); a pocket whose nearest vertex stands farther from the ring than the
    growth reaches (`LANDUSE_OVERLAP`) stays a separate sowing — the union would come out
    two shapes, and that is the test. else `Residential`, the yard grass every other verge lawn
    is — but only from `LONE_POCKET_MIN` 10 m² up: a smaller lone hole is a gap between
    paving bands (a sidewalk corner at a crossing), where yard grass lay on the
    tiles as a dark stain and bare ground nearly matches them. (A green pocket in the
    tiles that *is* drawn — what the kerb pad and the crossings leave of a **verge lawn** —
    is not a hole of this union at all: the verge covers it here, and the render tiles it
    over, `roads/scraps.rs`, `references/roads.md` **A lawn scrap is tiled**.) The tiles
    are those touched by a road link (± `MARGIN`), since the road is what
    every pocket has.
    Pinned by `a_meadow_pocket_touching_a_lawn_is_merged_into_it`.
  - **A green apron** (`Scene::green_aprons`) — the same question's second half,
    for strips that are never holes: a band of bare ground between a mapped green's
    **outer** edge and the kerb of a paved band, up to `GREEN_GAP_MAX` 2.5 m wide, is
    sown meadow from the green's edge to the very kerb. At its ends such a strip *leaks*
    into the open, so "what is enclosed"
    cannot see it. The band belongs to a paved **path** (`RoadClass::Alley`, kerb = the
    band's edge) or to a **street** — and a street's own verge decides: the strip along
    it starts at the edge of what the street draws itself, its sidewalk band or its
    verge lawn, whichever reaches further (`Link::street` carries the street and its
    arclength; `RoadLine::verge_at` is asked at the probe's side and place), so the
    apron is sewn to the band only where that edge falls short of the green's edge —
    the question is "does a covering verge lawn reach the strip", not "is there a
    verge": a square left a 1 × 7 m strip of bare ground
    between its lawn and a sidewalk band, open into the crossing
    at the south end — a spill the enclosure question cannot see, with a verge measured
    on the street's next segment but never reaching the strip. Asking streets without
    the verge gate measured ~3000 aprons lying on their own verge lawns' colour (the
    restriction's reason); a service drive has no sidewalk band and is not asked. The
    apron asks the green's edge **after** the merges: a merged pocket grows the green
    toward the band, and the strip is open from the grown edge — the pre-merge edge could
    stand past `GREEN_GAP_MAX`, and an apron sewn against it would have left the
    strip — so the `Scene` owns its street copies and outlives the merge loop. A street
    candidate whose probe projects past the link's end is dropped: the square cap of
    the cover is an enclosure fiction (the drawn band is trimmed at the node), and an
    apron sewn to it left a notch of bare ground where the crossing street's
    cap edge pointed into the strip. The
    green's edge is probed every `APRON_STEP` 1 m, the kerb is the nearest qualifying
    link's, and a jump of the kerb over `APRON_BREAK` 3 m (the probe switched roads)
    breaks the strip. A dirt path gets no apron — ground by it is a waste plot, as for
    a pocket. Pinned by `a_slit_between_a_paved_path_and_the_park_edge_is_sown_as_meadow`,
    `a_strip_by_a_street_band_is_sown_where_no_verge_lawn_covers_it` and
    `a_green_apron_follows_the_lawn_edge_a_merged_pocket_grew`.
  - **Why the errors are safe**: the block layer lies below everything drawn on it, so a
    cover the parse misses only makes a hole bigger (a missed fill) or puts grass under
    something drawn (unseen); a cover the parse invents only closes a hole that is then
    not filled.
  - **Tiles**: `TILE` 400 m cells touched by a road link, a window of `MARGIN` 30 m around each;
    a hole counts only when it lies wholly in the window (then every cover touching it is
    in the window) and its bbox centre lies in the tile (one tile owns it). Tiles and the
    road bands go across threads through a counter (`in_parallel`) — a centre tile costs
    tens of times an outer one — and the results are put back in tile order.
  - **A pocket is not a block** (`MapData::pockets`, drawn in the block layer by
    `spawn.rs`): pushed into `landuse` first, it made the verge lawn's yard test of the
    time (`roads.rs::Yards::beside`) find a yard beside every verge next to a sown
    sliver, and meadow verges turned
    to yard grass. The verge now asks the other way round — yard grass by default, a
    meadow only beside `parks` / `grass` (`Meadows::beside`, `references/roads.md`,
    **Sidewalks**) — so a pocket does not reach that choice at all.

## During the read

- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.
- **Every area ring is cleaned of needles** (`parse.rs::collapse_slivers` in `push_area`,
  the one choke point every parsed area flows through): consecutive repeats and
  p → q → p backtracks closer than `RING_SLIVER` 0.05 m collapse to one point. OSM
  contours like to touch themselves, and a zero-width needle is invisible as a fill but
  the area's rim band draws both its sides — a bright V of two thin lines over whatever
  the area lies on (`a_needle_in_an_area_ring_is_collapsed`). Only local needles collapse:
  a self-touching ring whose halves meet far from the touching vertex still needs the door
  snapped at the assembly step.
