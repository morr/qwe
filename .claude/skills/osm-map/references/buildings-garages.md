# Buildings — garage rows

Siblings: `buildings-material.md` (the wall frame and the cladding codes the run is
measured in, the cell-grid rule), `buildings-heights.md` (heights — a garage is one box,
no spread). `CONTEXT.md` carries the **Garage rows** glossary entry.

- **The garage row is the same mechanism keyed to *geometry*, codes `GarageRow` and
  `GarageBlock`** (`buildings/garages.rs`). Every other kind is chosen by `BuildingUse`
  and the building's own seed; these two are chosen by the shape of a **run** of
  garages, and it is the only place where one building's frame comes from its
  neighbours:
  - **Stitching.** Garage footprints whose **outlines** come within `JOIN_GAP` 2 m of
    each other are joined
    (union-find over a spatial hash of `CELL` 32 m — each footprint is registered in
    every cell its inflated box touches, so two close boxes always meet in at least one
    cell, and the pair test is inside a cell rather than across the city).
    **The AABB is a prefilter, never the test.** Measuring the gap between bounding
    boxes is what the first version did, and a ГСК's ribbons run diagonally: the box of
    a 258 × 8.6 m ribbon at 80° is three times wider than the ribbon and reaches across
    the drive into its neighbour's. The real test is
    segment-to-segment distance between the two rings; garages do not nest, so
    containment need not be considered.
  - **Cutting the outline is what carries the geometry** (`split_rings`,
    `GarageRect`). The run used to hand every member **one** axis, taken from
    `min_area_rect` over the concatenation of all its rings, and that is wrong twice
    over: the contract of `min_area_rect` is a *ring*, and a ГСК is routinely an L or a
    comb whose own rectangle is mostly air. Everything downstream then
    inherited the lie — seams askew to the walls of both wings, aisles across the whole
    letter, and the ribbon/cooperative thresholds measured on air.
    So each outline is cut instead, and the axis, the grid and a share of the roof
    belong to the **piece**:
    - the cut is a **chord from a reflex vertex along one of its own two walls** to the
      nearest edge. Never an infinite line: on a comb that line runs on through the
      other teeth and leaves a "piece" made of several, joined by zero-width bridges.
      Along a wall, because the whole point is for the
      piece's axis to be parallel to its own walls;
    - candidates are scored by the **area-weighted fill** of the two halves, and a cut
      that does not improve on the uncut fill is not made. Recursion stops at
      `RECT_FILL` 0.90, at `SPLIT_DEPTH_MAX` 6, and never produces a piece under
      `PIECE_MIN_AREA` 8 m² — shaving a two-metre corner off is not "cutting into
      rectangles", it is making shavings, each with an axis and a seam of its own;
    - the pieces **tile the outline exactly** (a chord splits a simple ring into two
      simple rings), so the roof is laid one piece at a time, with no clipping and no
      gluing. A holed outline is never cut: a hole landing in the wrong piece is a
      courtyard under a roof.
  - **Qualifying, and into which of the two.** A **cooperative** (`GarageBlock`) is a
    piece made of `building=garages` outlines at least `BLOCK_MIN_WIDTH` 14 m wide and
    `BLOCK_MIN_AREA` 400 m² — wide enough to
    hold rows *and* the drive between them. That test comes first, and it is restricted
    to the plural tag on purpose: a 30 × 20 m `building=shed` would otherwise get drive
    aisles invented across it. Otherwise a **ribbon** (`GarageRow`): at least
    `ROW_MIN_LENGTH` 12 m long (about four boxes — fewer and a comb does not read) and
    `ROW_MIN_ASPECT` 2.2 times longer than wide (a square patch has no axis, and the
    cross seam would go at random).
    Whether the **run** is a garage at all is a separate question, and it is answered by
    the same two tests applied either to the group as a whole or to any one piece. Both
    halves are load-bearing: a stitched ribbon of twenty 3 × 6 boxes reads as a ribbon
    only *together* (no box passes on its own), and a letter Г reads only by its
    *wings* (the whole never passes). Anything else is left alone and drawn as an
    ordinary small building.
  - **What the run hands out**: one seed (the **minimum**
    of the members' seeds, so it does not depend on their order), and nothing else.
    The shared seed is the whole point — with per-building seeds every box picked
    its own material and its own texture phase, and twenty boxes came out as confetti of
    tile, bitumen and corrugated sheet.
    **A piece that does not read as a garage on its own is answered by whether its
    outline was cut.** An uncut one **borrows a grid** — from the largest qualifying
    piece of its own outline, failing that from the run as a whole. That is the stitched
    ribbon: a 3 × 6 box's own long axis runs *across* the ribbon it stands in, so its
    seams would cross the comb and its gates would land on the party wall between
    neighbours. Sharing the axis is what a run is *for*; owning one is earned by being a
    ribbon or a blob yourself.
    An **offcut of a cut outline** — a four-metre tooth of a comb, a wedge beside a wing
    — gets no garage drawn on it at all (`GarageRect::plain`, `layers::plain_garage_roof`):
    the roof takes the ordinary world-axis frame with `RoofKind::Corrugated`, the walls
    are `WallMark::Solid`, and the colour and texture phase stay the run's, so it reads
    as a corner of the same shed rather than as a garage of its own. A near-square scrap
    has no axis to give, and the borrowed one turns its comb across its own walls; its
    four-metre walls then hold a fraction of a bay, and the end gates came out sliced in
    half at both ends — which is what a comb's teeth looked like. The rule is the
    user's own: separate the crooked part and do not draw a garage on it.
  - **The grid is the piece's own, in whole cells** — the wall's construction
    (`meshing::WallFrame::run`, `layers::garage_frame`), reached here for the same reason.
    The bay is `length / floor(length / BAY 3.9 m)` and the row
    `width / floor(width / ROW_PITCH 18 m)`, so the pitch is fitted to the piece rather
    than the piece to the pitch: a seam lands on **both** ends of the ribbon and an aisle
    on both edges of the blob.
    **Divided down, not rounded to the nearest**, and that is the user's rule for what to
    do with the remainder: rounding up makes the bay *narrower* than the measure — a
    22 m ribbon came out as six bays of 3.67 m, a 5.9 m one as two of 2.95 — and neither
    a car nor the gate drawn above it stands in such a cell. Dividing down spreads the
    remainder evenly over **all** the bays of that piece (five of 4.4; one of 5.9)
    instead of leaving a stub at the end; a row of identical gates is what a ГСК is read
    by. Pinned by `a_bay_is_never_narrower_than_a_car_needs`.
    A fixed metre pitch phased to one end — what this was
    first — leaves a sliver of a bay at the other.
    **`BAY` and `ROW_PITCH` are targets, not divisors**, exactly as `PANEL_WIDTH` is on a
    wall, and neither number is mirrored in `roof.wgsl` any more: the shader is handed
    cells and takes `fract` of them.
  - **What is drawn**: a seam on every bay boundary — the only thing that survives to
    city zoom, and the thing that makes the ribbon a comb — plus ±10 % of paint tone per
    bay (hashed from the bay index, so it changes exactly at the seams, like a row of
    doors), the corrugation of an ordinary garage, and more rust than any other
    roof gets. A **cooperative** adds to that a **darkened drive** on every row boundary
    (`ROW_FILL` 0.667 of the row is the two rows of boxes back to back, the rest the
    aisle) and the back-to-back seam in the
    middle of each pair, so the blob comes out as a grid of boxes rather than a
    hangar; the tone hash then keys on the row as well as the bay. The aisle is
    **shading, not a hole cut in the roof**: cutting it for real means a boolean on the
    outline, and the walls (built from the outer ring) and the shadow sweep would then
    disagree with the roof.
    The `band(coord, period, start, width, px)` helper is the wide-stripe sibling of
    `stripes` and exists for this drive.
    Like the wall, the ribbon **leaves the fragment before the common roof pass**
    (`garage_shade`): it has no roof age, and the one thing it still reads in metres is
    the rust, which lives on the roof rather than in the run's cells.
  - **The wall of a run is the ribbon's other half** (`WallKind::GarageDoors`,
    `layers::wall_of_run`, `roof.wgsl::gate_of`) — and it has to be, because from the
    south a ГСК is *read by its gates* the way it is read from above by its comb.
    The cell of that wall is the **bay** rather than the 3.2 m panel
    (`layers::cell_width`), so a gate stands under its own roof seam; a gate is
    `GATE_WIDE` 0.78 × `GATE_HIGH` 0.80 of it — a garage door is as wide as a car and
    nearly as tall as the wall, and the ordinary door beside it is a slit — it starts at
    the ground (no threshold, you drive in), carries the sections of a roller door and a
    lintel over it, and is painted per bay by its owner. There are no windows and no
    balconies on it at all, and `push_doors` returns early: an OSM entrance would
    otherwise put a second, differently sized leaf over a gate that is already there.
    **Only the walls running along their piece's axis get gates**
    (`layers::garage_cells`, `GarageRect::faces_the_drive`, the split at 45°): you drive
    in from the drive, and the cross wall is a party wall between two boxes. The end is
    measured in the piece's **rows** rather than its bays — one cell on a single-row
    ribbon, the aisle boundaries on a cooperative — and is marked `WallMark::Solid`, i.e.
    drawn with the material's pattern and no opening at all; `wall_shade` returns on that
    mark before it ever reaches `gate_of`.
    **And the wall is measured by the piece's bay, not by the constant `BAY`**
    (`layers::garage_cells`). "A gate stands under its own roof seam" was true only by
    coincidence before that: a wall divided *its own* length by `round(length / BAY)`,
    which agrees with the roof on a whole rectangle whose long side is that wall and
    nowhere else. On a cut outline the gates drifted off the comb the further from the
    wall's start you looked — pinned by
    `a_gate_stands_under_its_own_roof_seam` (a 15 × 5 ribbon holds three 5 m bays, where
    the constant would have given four of 3.75).
    The **phase** is deliberately *not* taken from the piece, and that was tried first:
    a wall sitting on the piece's phase begins and ends mid-cell, and its end gates come
    out sliced in half — two half-gates on every tooth of a comb. The wall keeps a whole
    number of cells ending on its own
    corners, as every other wall does (`a_garage_wall_holds_whole_gates`); the piece's
    length *is* the wall's length on a long facade, so the same pitch from the same
    corner gives the same boundaries anyway.
    For the same reason the visible walls of a cut outline come **piece by piece**
    (`layers::garage_walls`): a chord cuts an outline edge in two — the letter Г's 52 m
    west wall is 8 m of one piece and 44 m of another — and a whole wall can only carry
    one grid. A piece's ring is already cut where it should be, so its silhouette *is*
    the list of walls; the chord is dropped from it by not lying on the original outline.
  - **The bay is measured against the cars the game draws** (`CarShape`, 1.72–1.95 m
    wide): the widest one plus half a metre each side for the doors plus a pier is
    `1.95 + 2 × 0.55 + 0.4 ≈ 3.9 m`, which is `BAY`. The 3.4 m it was first is the
    physical floor — a van fits with nothing to spare — and on the map that read as too
    fine a comb.
  - **No clutter**: `flat_roof_items` and `ridge_chimney` both refuse both kinds. A
    ventilation shaft or a chimney on a garage is the generator showing through — and
    the blobs used to collect *skylight ribbons*, since a 12 000 m² corrugated roof is
    exactly what that rule looks for.
