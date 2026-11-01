# Roads — kerb returns and corner law

The rounded corner of a junction: the fillet between two arms, the outer corner fan,
the nose of a sharp fork, the bend, the obtuse-corner wedge, the fork gore, the small
paved islands, and the sidewalk that turns with the kerb. Code: `corners.rs` (module
root), `corners/gather.rs` (the `KerbReturns` value and `kerb_returns`),
`corners/arm.rs` (arms, radii), `corners/shapes.rs` (fillet, bend, obtuse corner,
kerb pad), `corners/nose.rs` (nose, fork gore), `corners/islands.rs` (small islands,
tile overlaps). The hub is `roads.md`; the straight edges the arcs are clamped by come
from the street axis (`roads-streets-network.md`, **The street axis**).

- **Kerb returns** (`corners/gather.rs::kerb_returns(&Drawn, scale)`, with `corners/islands.rs::small_islands(&Drawn)` beside
  it — both read everything they need off `Drawn`: the nodal axes, the drawn sidewalk,
  the pair runs, the tapers, the merges) — the rounded corner of a junction. At every shared
  node, arms are collected from the **nodal** paths (pinned, so the node is a vertex of
  each): a direction to the first vertex at least 0.5 m away and the straight **run** —
  to that vertex and on through every next one within `STRAIGHT_TOLERANCE` 0.15 m of
  the arm's line (OSM puts vertices on a straight drive wherever it likes, and a run
  cut at the first of them clipped the tangent to nothing). **Both are measured
  on the vertices the ribbon keeps** (`meshing::ribbon_vertices`, the mask behind
  `merge_ribbon_points`, at the ribbon's own `ribbon_merge_distance` — a quarter of the
  road's width): the ribbon merges axis points closer than that, and its edge runs
  straight to the next vertex it kept, not to the merged one — a direction aimed at a
  merged vertex parted from the drawn edge by degrees (a sliver of sidewalk beside the
  kerb; a run carried through a merged vertex ended the arc as a tongue of asphalt in
  the sidewalk). A node the ribbon merged away itself falls back to
  the axis vertices. Arms of one class are sorted by angle, and between neighbours
  25°–155° apart the
  corner of the two facing edges is found, a circle is fitted tangent to both — its
  radius **by the minor class of the pair** (`corners/arm.rs::kerb_radius`): `MAJOR_RADIUS` 10 m between
  avenues (`trunk`…`secondary` and their links),
  `STREET_RADIUS` 6 m with a street (`tertiary`, residential, `unclassified`),
  `DRIVE_RADIUS` 2.5 m with a drive, a living street or a driveway crossing,
  `PATH_RADIUS` 2 m between footways, and never more than `DIRT_RADIUS` 3 m between
  two dirt roads (**Unpaved streets** in `roads-sidewalks.md`). It used to follow the widths (`0.6 ×
  (half + half)`), and on a divided avenue, where halves of different lane counts meet
  in one node, every corner came out a metre or two. The wedge `[corner, tangent, arc…,
  tangent]` goes into that class's fill builder **before any ribbon** — ribbons and their
  markings then lie over it, and since it is pushed with no ribbon coords it carries no
  wear or markings of its own. One clamp: the tangent never runs past an arm's straight
  run (past the next vertex the edge has turned) **nor into a taper** (`Drawn::taper_ends`
  — the run ends where the wedge begins, since the edge there is already
  closer to the axis; lengths as the ribbon cuts them,
  `tapers::fit`) — **by the fitted length**: a raw seam wedge can outgrow the road
  itself, and a cap by the raw length went negative and wiped the arm's run to zero,
  so the mouth corner was not built and the node's asphalt spiked through the tile
  band (`corners/tests.rs::a_taper_longer_than_the_road_keeps_the_mouth_corner`). **A node under the
  wedge** — a side street joining the wider way inside
  its taper, not at its end — is read at the wedge's width there, `narrow + (own − narrow) · from
  seam / length` on each narrowed side; the arm away from the seam runs straight to the
  wedge's end and widens to its own half, the arm **toward** the seam runs to the seam
  and narrows to the neighbour's (it used to get a run of zero, and no corner at all),
  and both edges are **slanted** — `Arm::slope`, metres off the axis per metre of arm —
  so `fillet_arc` meets them on the edge the wedge ribbon lays, not on a line of the
  way's own half (that line stood half a metre off the kerb, the sidewalk showing
  through as a light wedge). Its vertices are the wedge ribbon's
  (`wedged_vertices`: each piece of `tapers::split` merged from its own ends, a wedge at
  `narrow width / 4` like `push_taper_sided`), or a crack opened along the slanted kerb.
  The run clamp is why the street axis keeps
  `KERB_STRAIGHT` next to a pinned node and pins only on carriageway nodes, and the
  paired halves keep `PIN_STRAIGHT` 16 m unshifted there (**The street axis** in
  `roads-streets-network.md`,
  **Paired halves** in `roads-pairs-medians.md`). The old `r ≤ 3.4 × the narrower sidewalk` cap is gone: the sidewalk
  fillet below is concentric and shares the kerb's tangent lines, so the asphalt wedge
  lies inside it at any radius over the sidewalk width. With one sidewalk or none, a
  flare over the ground is exactly what a drive's kerb return looks like. It is pushed as a **fan from
  the corner** (`push_convex`), which is correct although the wedge is concave: the arc
  between the tangent points is precisely the part of the circle visible from the corner.
  Its straight sides reach `OVERLAP` 5 cm under both ribbons — a side lying exactly on
  a ribbon edge without sharing its vertices rasterizes with dropouts.
  Mixed-class arms get nothing: a grey wedge over a sand
  footway would read as asphalt spilled onto the path — and so do a dirt street and an
  asphalt one (**Unpaved streets** in `roads-sidewalks.md`). Bridges and passages give no arms
  (their paths go in as `None`). The radii are scaled by `RoadShape::corner_radius`
  (0.5–2, `kerb_returns(..., scale)`).
  **The output is one `KerbReturns` value, and the module lays it itself**
  (`KerbReturns::lay`, taking a `KerbLayers` of the seven layer builders, `Meadows`
  and the lawn `Scraps`): the order — lawn scraps first (paving around them is what
  cuts them), then the fanned fills street/alley → dirt → sidewalks with tile overlaps
  → verges → wide-verge lawns, then the noses and bends triangulated whole — used to
  live in `build_roads`' hand-rolled loops and is `corners`' invariant now; the
  fork gores (`fork_gores()`) alone are laid at their own call site, into the hatch
  (`Gores::add_forks`), because `Gores` is built from the drawn order after it.
  Geometry readers take typed views instead of fields: `paving()` (what cuts the lawn
  scraps — street asphalt, sidewalks, verges, paved noses and bends; **tile overlaps
  are not paving**, they are the repair itself; split into `flat()` and `curved()` so
  the caller chains its own gore outlines between them and the insertion order of
  `Scraps` keeps), and `streets()` (street-class asphalt only, for the gore mouths).
  The report's counters come from `counts()` (`KerbCounts`: roundings without outer
  corners, tile overlaps, noses, bends, outer corners) — and the fields are closed
  inside the module, read directly by nothing but its own tests. **`lay` also feeds the
  semantic index**: every piece it pushes — corner fills by class, the dirt arcs, the
  sidewalk turns and tile overlaps, the verge corner fillets and pads, the corner
  lawns/yards, noses and bends — is collected into `DrawnPieces` as `road corner (auto)`
  / `sidewalk corner (auto)` / `verge paving (auto)` / `verge lawn (auto)` /
  `verge yard (auto)`, with no owner road (a corner belongs to the junction, not to one
  way), so the point inspector highlights the exact corner piece.
  - **An arm that ends in a junction ends square** (`KerbReturns::butt`). A
    junction here is a class group of three arms or more, or of two meeting at an angle
    (25°–155° either side); two nearly collinear ends are one road continued and keep
    their round caps. Every arm of a junction that is an **end** of its drawn path gets a
    `Butt` cap in the fill and the sidewalk band (`trimmed` in `mesh_roads`,
    the same flag the taper ends use; a stitched end is not in a node and stays round) —
    except an **alley arm whose cut no corner piece would cover**: one continuing through
    the node (an opposite within `MAX_ANGLE` — its pairs' pieces are skipped, **Paved
    paths** in `roads-sidewalks.md`) or one whose every neighbour continues; such a cut stood out as a wedge and a step between ribbons of different widths, and the round cap keeps the
    merge reading as bands crossing — **unless the cap pokes past them**: an ending arm
    wider than a band it lands on (an arm in the cap's half-plane, ahead of the node) is
    cut square anyway, for the half-disc of the cap sticks out past the narrower band's
    far edge (a grey wedge on the grass, the zebra's first pale stripe on the neighbour
    band). Streets keep the square cut: the crossing street's asphalt covers
    the seam (a round cap of a wide road ending on a narrow one stuck out past its far
    edge as a half-disc).
    The butt end lies on the node, inside the crossing road.
  - **The outer corner of a junction is a fan** (`corners/shapes.rs::outer_corner`): between two neighbour
    arms more than 180° apart — two streets meeting at a corner with nothing running
    through, a fork seen from outside — a fan from the node with the radius sliding from
    one half width to the other, what the round caps used to give for free. Pushed like a
    return, before the ribbons; its centre sits `OVERLAP` behind the node and its sides
    reach `OVERLAP` into the butt ends. The same fan in the sidewalk layer on `half +
    sidewalk`. Counted apart in the log line (`outer corners`). "More than 180°" means
    past `MIN_OUTER` 0.05° — rounding noise, no more: a street split into two ways at a
    junction with a slight kink leaves
    a wedge between the two butt ends on the side away from the crossing road, and at
    the old 1° threshold it showed as a light hairline. The fan's straight
    sides follow the **butt edges of the ribbons** (`Arm::butt` — the direction of the
    axis's own end link, the same one `meshing::butt_normal` cuts the cap across), not
    the arms' directions: an arm direction aims at the first vertex the ribbon kept,
    and on a street split into two ways at a kinked node the two part by degrees — a
    fan built on the arm direction stood turned from the cap edge, and between the two
    a wedge slit crossed the whole carriageway, the verge tile showing through the
    asphalt and the layer below through the lawn. The >180° test stays on the arm
    directions — `MIN_OUTER` judges the
    drawn axis's kink, not the caps'. Where an arm has no butt end the side falls back
    to the arm direction, which is what still keeps the `OUTER_OVERLAP` 0.3 m reach
    (not `OVERLAP`): the through arm's side runs along the nodal chord while the
    ribbon edge on a smoothed bend has already turned, and on the wider sidewalk fan a
    degree is fifteen
    centimetres at the band's far side. **In the
    sidewalk layer the fan is also laid between exactly two banded arms** when both are
    way ends and the streets at the node make a junction — a street split into two ways
    where a drive without a sidewalk joins it: its band ends are butt (the street node is
    a junction), yet two banded arms alone read as a continuation and got no corner, and
    a hairline crossed the sidewalk.
  - **A sharp fork gets a nose** (`corners/nose.rs::nose`, `KerbReturns::noses`): between two
    neighbour arms under 25° (`MIN_ANGLE`, down to `NOSE_MIN_ANGLE` 2°), and between
    arms up to `NOSE_MAX_ANGLE` 60° whose fillet did not fit (the straight run shorter
    than the tangent), the two facing edges used to meet in a mathematical point: a
    spike of sidewalk
    and ground between the carriageways. A fillet cannot help
    — its tangent `r / tan(θ/2)` runs tens of metres, far past the straight run — so the
    nose is a different figure: an arc of a **small** radius (`NOSE_SHARE` 0.4 of the
    pair's kerb radius, capped at `NOSE_RADIUS` 1.5 m × the corner knob — 1.5 on
    streets, 1 with a drive, 0.8 on footways) where the edges have parted by two radii.
    It is found on the **axes** (`Arm::trail`, the nodal path from the node out to
    `NOSE_REACH` 100 m, built only for a nose), not on the arms' first directions: the
    tip lies 30–60 m from the node, where a ring exit or a curving branch has long left
    the straight line. `NOSE_STEP` 0.5 m stations along the first arm: its edge (with
    the taper — `Arm::half_at`, linear over the fitted wedge the ribbon lays: the one
    under the node, and one further along the arm — a wedge whose seam lies tens of
    metres out narrows the kerb long before the nose zone, and on the nominal half
    width the nose stood past the real parting of the edges, its hatched gore on the
    verge tile), the nearest
    point of the second axis (searched in a `NOSE_WINDOW` of links around the previous
    hit) and the gap between the edges; the tip is the last station where they still
    crossed, the centre starts in the middle of the first gap of two radii and is then
    **settled** (`NOSE_SETTLE` 8 passes) until it stands one radius off both edges — a
    ring's edge bulges towards the island, and the midpoint's arc stopped short of the
    straight edge with a notch. The outline — tip, the first edge, the arc between the
    feet, the second edge back — follows bent edges, so it is **not** a fan: it goes into
    `noses` with its `Fill` (street/alley layer, unpaved, sidewalk) and is triangulated
    whole (`push_polygon`); the narrow part of the wedge keeps every `NOSE_THIN` 4th
    station only. The sidewalk gets its own nose on the band edges (half + sidewalk),
    with the same small radius — the concentric `r − sidewalk` would go negative, and the
    tip of an island is all paving anyway. (The first cut built the trail for every arm
    and searched the whole second axis at every station — measurably slower; the window
    and the per-nose trail are the fix.)
  - **A curved edge gets a bend** (`corners/shapes.rs::bend`, `KerbReturns::bends`): where
    the straight fillet does not fit because an arm's straight run is shorter than the
    tangent. A ring is redrawn on an ellipse in chords of centimetres of sag
    (`roads/rings.rs`), so its arm's run is the couple of metres a chord stays within
    `STRAIGHT_TOLERANCE` — a ring entry met the ring with square corners, the ring's
    sidewalk band ending in a sharp wedge
    in the carriageway on both sides. The bend is the same arc seated on the **real**
    edges: both arms' `Arm::trail` (cut to `BEND_REACH` 40 m) offset by the half widths,
    the corner is their first crossing, and the centre starts on the bisector of the
    two edges' first directions and is **settled** `BEND_SETTLE` 8 times to stand one
    radius off both polylines (the nose's trick); when a foot lands past the end of its
    kerb — the pocket is shallower than the class radius — the
    radius shrinks by 0.8 down to `MIN_RADIUS` 0.5 until both feet sit inside, the way
    `fillet_arc` shrinks its tangent under a short run; a foot at the corner, or an arc
    off an edge by more than `BEND_FIT` 5 cm even at the floor, gives nothing. The
    outline — corner, the first edge to the foot, the arc, the second edge back — is
    concave where a ring's edge bulges into the corner, so it is triangulated whole
    (`bends`, pushed with the noses). Its sides go `BEND_OVERLAP` 0.3 m under the
    ribbons, not `OVERLAP`: the edge is computed on the dense axis while the ribbon
    lays its curved edge in chords between the vertices it keeps, and 5 cm left a dotted
    crack along the side. Tried after the fillet in the street group (not
    for a mixed dirt/asphalt pair) and in the sidewalk group (on `half + sidewalk`,
    radius less the sidewalk, the concentric rule); the verge corner and the kerb pad
    stay on the straight `fillet_arc`. **Only 60°–155°** (`NOSE_MAX_ANGLE`..`MAX_ANGLE`):
    sharper is a nose's, and at a ring an approach bent in at 25° (`bend_approach`)
    meets the ring at ~30° — a bend there poured asphalt twenty metres into the
    island's gore. `kerb_returns` itself did not move past the noise (a trail and two
    offsets only where the fillet failed).
  - **An obtuse corner with a change of width gets the wide kerb's wedge**
    (`corners/shapes.rs::obtuse_corner`): two neighbour arms of a junction
    `MAX_ANGLE`..`OBTUSE_MAX` (155°–178°) apart are nearly one road continued, but at a
    junction their ends are square, and when the widths differ the wide arm's edge,
    carried on past the node, meets the narrow arm's edge only **behind** the wide arm's
    butt end (along the narrow one). No fillet sits there — its corner lies behind the
    wide end — and between the square end of the wide ribbon and the narrow one's edge a
    wedge of ground showed, the asphalt ending in a straight cut. The outline: from the wide end along
    its edge carried past the node to an arc tangent to both edges, its tangent as long
    as the room allows — no further back than the wide end, no further out than the
    narrow arm's straight run — so the width change reads as one gentle sweep. A fan from a point just inside the wide ribbon at the
    node, pushed with the returns into the class's fill; the same call on `half +
    sidewalk` in the sidewalk group and on the verge corners (tried after the fillet).
    Streets only, and never between two ring arcs: a ring is one closed fill with its
    own sidewalk (**Roundabouts** in `roads-rings.md`), and a wedge on the arcs' differing sidewalk widths
    ran a tile strip across the lawn of the big ring island.
    Skipped when both arms are one axis (a road running through), under a taper wedge
    (`Arm::slope`) and when the edges meet ahead of the node on both arms (equal
    widths — nothing to close). Past `OBTUSE_MAX` the arms are one street continued and
    a width step there is the section taper's (`roads/tapers.rs`).
  - **A sharp fork of two streets gets a hatched gore ahead of its nose**
    (`corners/nose.rs::fork_gore`, `KerbReturns::fork_gores` → `Gores::add_forks`, painted like
    every gore by `road_paint_islands`). The nose stands where the ribbons' edges have
    parted, and before it the two ribbons overlap for tens of metres: at a shallow fork
    one plain tongue of asphalt
    runs tens of metres from the node to the nose. On the ground the lanes part earlier
    and the space between them is hatched. The gore is built off the nose's own stations
    (`Station`: both facing edges, the gap, the half widths, the inward normals): it
    **starts** at the first station where the overlap of the edges is down to the
    narrower half width — the narrower street's axis has left the wider one's
    carriageway — widens linearly to the gap at the station where the nose was found
    (two radii), and **ends square** on the line through the nose arc's apex (running it
    along the arc laid the outline as a white bracket over the kerb, with hooks at its
    feet). Where the gore is wider than the gap, its sides go into both ribbons, each
    by its share of the half widths, so both carriageways narrow together; past the
    edge crossing it is exactly the nose's asphalt. Only at a **fork** — a node of
    exactly three street arms, the two under `MIN_ANGLE` 25° (not the nose of a fillet
    that did not fit, up to 60°): on a six-arm node the gores lay as hatched islets in
    the middle of its asphalt. Only between two `Highway::is_street` arms, neither of
    them a ring arc (a
    ring hatches its own wedges — `Gores::add_forks` also skips a gore that touches
    one), not on footways, drives or dirt, and not shorter than `FORK_GORE_MIN` 8 m —
    a fork under 25° gives at least 12 m (a lane and two nose radii over the sine), and
    the shorter ones came from bent arms inside a complex node.
    The asphalt under it is the ribbons and the nose: only the hatching is added
    (`corners/tests.rs::a_sharp_street_fork_hatches_a_gore_ahead_of_its_nose`).
  - **A small island of three nodes is paved** (`corners/islands.rs::small_islands`): three shared nodes
    pairwise joined by pieces of streets (a fork's triangle) whose inradius, less the
    widest half width of the three, is under
    `ISLAND_FILL` 2 m, with a perimeter under `ISLAND_PERIMETER_MAX` 120 m. Of such an
    island only a lens between the ribbons is left, and the ground showed in it as a
    light crescent; the whole triangle is pushed as asphalt under the ribbons, like a
    ring's web. A larger triangle is a real island and keeps its fill. Counted in the log
    line (`small islands`).
  - **The junction is not unioned into one polygon.** An early plan asked for the
    asphalt of a node as an `i_overlay` union of its ribbons; the pieces above give the
    same picture lying under the ribbons of their layer (a ribbon covers every seam
    between them), and a union per node — thousands of them per city — would
    have cost hundreds of milliseconds of loading. The junction's paint (stop lines,
    zebras) is placed along its arms and needs no outline.
  - **The sidewalk turns with the kerb** (`KerbReturns::sidewalks`), and it is an
    *addition*, not the subtraction this doc used to call impossible: the corner between
    two sidewalk bands is a **concave** notch exactly like the asphalt one, so the same
    `fillet` fills it — laid on the band edges (`half + sidewalk` instead of `half`) with
    a radius smaller by the sidewalk width. That single subtraction is what makes the two
    arcs **concentric**: a fillet's centre sits at `corner + bisector · r/sin(α/2)`, and
    pushing the corner out by `s/sin(α/2)` while taking `s` off the radius leaves it
    where it was. So the band keeps a constant width all the way round the corner, which
    is what a photo shows.
    - **The pairing is its own**, over the arms that carry a sidewalk rather than over
      the class group: a drive without one must not break the band of the street it comes
      out of (the street runs straight past it), while two streets with a drive between
      them still get their corner — the drive's asphalt is drawn over it.
    - **A radius under the sidewalk width leaves the corner square**, and that is the
      geometry, not a fallback: a drive with a sidewalk into an avenue (2.5 m against a
      3 m sidewalk) has no arc for the outer edge to follow on the ground either. Same
      for a run too short for the tangent, and for a fillet the run clamp brings under
      `MIN_RADIUS` 0.5 m — it would not show.
    - **The asphalt wedge lands on pavement at any radius**: the two arcs are concentric
      and share their tangent lines (the foot of the perpendicular from the centre to an
      edge is the same point for the road edge and the band edge), so the wedge between
      the road edges and the kerb arc lies inside the one between the band edges and the
      sidewalk arc. The cap `r ≤ 3.4 × sidewalk` that used to guard this is gone.
    - Arms with **different** sidewalk widths cannot share one concentric arc; the
      radius then takes the wider of the two (the conservative side — a smaller radius
      keeps the asphalt wedge inside).
    - **The side of a paired half has no sidewalk corner**: an arm carries a sidewalk
      per side (`Arm::sidewalk`, left and right of its heading), and where the arm lies in
      a pair run (`Pairs::beside`, the runs of **Paired halves** in `roads-pairs-medians.md` with two probes of slack) the
      partner's side is `None`. The corner is taken from the first arm's left to the
      second's right, so a corner facing the median gets none — it put light arcs into
      the median opening of a divided avenue.
    - **A crossing street's piece between two halves carries no sidewalk at all**
      (`Pairs::across_median`, asked once per road by `Drawn::new`, under
      `MEDIAN_CROSSING_MAX` 40 m, one end in a
      node with a half and the other in a node with its partner): it lies in the median
      opening, and its band showed as a light disc in the middle of the junction
      (sample 15).
    - Load-time only, like the rest of this module.
