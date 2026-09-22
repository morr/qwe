# Parking lots and parked cars

Detail behind `map/parking.rs`, `map/roads/lots.rs`, `map/roads/gores.rs` and
`map/cars/`. The lot outline reaching its road is a parse pass (**Blocks pulled to the
roads → `pave_lots`** in `parse.md`); everything here reads the outline it produced.

## Parking

- **Parking** (`map/parking.rs`) — an `amenity=parking` area is drawn as asphalt
  (`Z_PARKING` 2.001, the `parking` surface layer) with the **stalls painted on it**
  (`Z_PARKING_LINES` 2.004 — above the two big-lot road layers, see **A big lot shows
  the road through it** below). The markings go in a **flat-material** layer of their own,
  not through `SurfaceMaterial`: the procedural asphalt grain belongs under the paint,
  not on it, and a 12 cm line is the one thing on this map that must stay pure white.
  - **Road asphalt, no rim.** `PARKING_COLOR` is `roads::ROAD_COLOR` and the fill is a bare
    `push_polygon` — no **Rim**, unlike parks, woods, grass, sand and pitches (the landuse
    blocks have none either, and water has its shoal instead). It had a darker one,
    then a narrow lighter one; both drew a band across every drive where it runs into the
    lot, reported from screenshots (`cam 5301 3270`): the lot lies over the roads, so its
    outline crosses the drive's asphalt, and anything laid along it is a seam.
  - **The layout is computed once per world load** into `ParkingLayout` (a resource,
    filled by `spawn_map` from `ParkingLayout::new(&map.parking, &map.roads)`, which lays
    the lots out largest first through `stalls_beside(lot, aisles, through, drives)` — the
    production door; the arity-two `stalls` beside it is `#[cfg(test)]`), and the paint and
    the cars
    both read it — two independent layouts would put a car across its own line, and
    recomputing it on every rebuild of the car layer was work the sun slider paid for by
    the frame. `STALL_WIDTH` 2.6 × `STALL_DEPTH` 5.2 m, `AISLE` 6 m, `EDGE_MARGIN` 1.2 m
    in from the edge.
    **And it is measured, by a timer of its own** — `ParkingLayout::new` runs *before*
    `mesh_surfaces` at both call sites, so the `SurfaceReport` timer never saw it: the app
    prints a `parking layout: N lots, M stalls in …` line beside `surface meshing:`, and
    `measure_surfaces` carries a `parking layout` row (milliseconds, no vertices — it is
    not a mesh) inside the bench's `surfaces` line. Tula, cache v14, `dev` profile, three
    interleaved runs per side: the layout costs **21 ms** against **2 ms** before the
    pocket rule, and the surface build **192 ms** against **181** (the markings' extra
    `fits_with` per bar), i.e. **+29 ms once per world load** on a load measured in
    seconds. The tenfold is mostly **not** the extra probes: `point_in_area` is an
    even-odd raycast over the whole ring with no AABB rejection, and pulling a lot to the
    road densified its outline — the `parking` layer went 2 k → 10 k vertices, so every
    probe walked about five times the ring it used to. **That edge index now exists**
    (`Outline`, below — it came with the polygon paving, whose fillets took the mall lot
    to 1220 vertices); what must not happen is the next probe being added without
    re-reading that row.
  - **Which way the rows run is read out of OSM, not guessed** — `service=parking_aisle`
    (`RoadLine::parking_aisle`, `tags::is_parking_aisle`), the only thing the layout takes
    from the road network. Tula's ТРЦ «Макси» lot carries **50** aisles, 44 along the
    long axis at 102–105° and 6 across at 59–61°, neighbours 16–19 m apart; the hospital
    lot (way 344589378) carries 4, spaced **14.8**. How they are read:
    - **Stalls go by the pocket between neighbouring aisles, not by an offset off each
      centreline.** A pair of rows back to back down the pocket's middle, noses out, and
      what is left is the drive to either side. The fixed offset (`AISLE/2 +
      STALL_DEPTH/2`) needs 16.4 m between aisles — fine at the mall, and at the hospital
      every second row came out overlapping and `Placed` dropped it, leaving one row per
      aisle. **One row belongs at the edge of a lot and nowhere else**, which is exactly
      what the author reported.
    - **The stall's depth is a field of `Stall`, not a constant**, because the pocket
      sets it: `(gap - AISLE)/2` clamped to `STALL_DEPTH_MIN` 4.8 … `STALL_DEPTH` 5.2, and
      a pair is laid only while the drive keeps `PAIR_AISLE` 5 m. At the hospital's 14.8
      that is 4.8 + 4.8 of stalls and 5.2 of drive. Below 4.8 a car (3.9–4.6 m; the
      "Газель" is 5.3 and sticks out of any of them) would poke into the drive, and the
      room won back would be lost to the car anyway.
    - **The main direction is read in two passes** (`main_of`): a rough
      length-weighted direction over every aisle link inside the lot, then the same
      answer again over the links within `AISLE_SPREAD` 30° of it, so the cross aisles
      stop voting. Thirty degrees is more than the aisles' own spread (Макси 102–105°)
      and less than the turn to a cross aisle (59–61°, i.e. 40° off). `rows_of` then
      keeps only the straight links, joining consecutive ones of a single OSM way into
      one run while the next begins where the last ended (`JOIN_SLACK` 1 cm — the links
      come from one polyline and meet exactly; the tolerance is for `f32` at map scale).
    - **The lane grid is continued by its own step out to the outline**, and a row spans
      the lot rather than the aisle. Aisles stop short of the edge in OSM, so without it
      the hospital lot had a broad band of bare asphalt along two sides and stalls to the
      kerb along the other two — a lot is not laid out with margins like that. The step
      is the **mean spacing** of the lot's own aisles (`lanes_of`), or `2 × STALL_DEPTH +
      AISLE` where there is only one to measure — a lone aisle means a pair of rows back
      to back and a drive, which is how such a lot is striped.
    - **Two lanes closer than `LANE_MERGE` (`STALL_DEPTH` 5.2 m) are one lane.** A long
      aisle in OSM is routinely cut into two ways with a slight kink — at Макси **all
      forty-four** lie that way, in pairs half a metre apart — and counted as two lanes
      they would open a half-metre pocket between them.
    - **A pocket too narrow for a pair gets a single row down its middle**, provided
      `MIN_AISLE` 3 m is left on **both** sides of it (`gap ≥ STALL_DEPTH + MIN_AISLE`
      after the pair's `gap ≥ 2 × depth + PAIR_AISLE` has refused). Three metres is the
      narrowest strip a car still squeezes through to the stall; below it the pocket is
      left as bare asphalt.
    - **`Placed` stays** — a grid at `OVERLAP_CELL` and a separating-axis test with
      `OVERLAP_SLACK` 5 cm of give (without the give the fp error between two neighbours
      touching exactly would drop every second stall). Pockets do not overlap by
      construction, so it now only guards odd data.
    - **No invented `ROW_BLOCK` along an aisle-driven row, but the block break OSM does
      draw is honoured.** OSM draws a cross drive as a **gap** — one lane's aisle cut into
      two collinear runs with an aisle-wide gap between them — and a gap repeated in at
      least a second lane is kept clear across every row (`cross_drives_of`); the row
      otherwise ends where the outline ends it. Tula's mall lot carries exactly one such
      drive, visible in 21 of its 37 lanes, 5.1–6.0 m wide. In the 7600 × 5700 v14 cache
      the whole city holds **22** gaps between collinear runs of one lane; 21 are that one
      drive (midpoints within 1.5 m), and the twenty-second is 2.8 m and is filtered out by
      `MIN_AISLE`. The corridor costs **155 stalls of 3231** on the mall lot and nothing on
      the other 46 aisle-driven lots. **Spurs and perimeter loops that `rows_of` throws
      away never become corridors**: 337 stalls in the city stand over such a link, but not
      one of them is a cut into blocks — at the hospital lot the chain of spurs lies exactly
      where the rows already begin, and at the mall 200 of those stalls lie over a diagonal
      field of aisles with a direction of its own. Taking them was measured and rejected;
      `the_elbow_that_leads_to_an_aisle_gets_no_rows` pins it.
    - **The strictest tag, not any `service`.** `driveway`, `alley` and `drive-through`
      lead *to* a lot, not along its rows; taking them would turn the rows 90°.
      Tula's v14 cache: 2846 `highway=service`, of which 209 `parking_aisle`, 99
      `driveway`, 15 `alley`.
  - **A lot with no aisle in it gets an invented layout** (`generated_rows`) — the
    majority of them: yard patches. Rows run along the **longest side of the outline**,
    not the long axis of `min_area_rect`: on a lot pulled to the road the outline is
    ragged and the minimal rectangle turns on whichever tooth happens to be longest in
    projection, so the stripes end up at an angle to the side the lot reads by.
  - **One law decides that invented layout: a car has to be able to drive to every
    stall**, and both directions of it answer the same report — a lot striped wall to wall
    reads as hatching, not as a place cars are parked in. **Which way a row faces is read
    off the gaps around it** (`row_noses`), one answer per row and never one for the whole
    lot: a row drives out into whichever side has the wider gap — the wall-side row into
    its own aisle, the first row of a pair back into the aisle in front of it, the second
    forward into the aisle behind it. With one nose per lot the first row of every pair
    stood nose to the second one's back: on `rect(20, 40)` that is 14 of 28 stalls facing
    a place there is no asphalt at, and turning `reachable` on without the fix deletes
    that row whole (28 → 14) instead of turning it round.
    - **Across, `row_bands`: `row — aisle — pair — aisle — pair`.** The field starts with a
      *single* row at the edge and only then pairs rows back to back; each pair has an
      aisle on either side of it, and the edge row takes the one behind it. It used to
      start with a pair (`row row aisle` repeating), and then the very first row had its
      back in the second row and its nose in the lot's own edge: nothing could reach it.
      The shift costs no stalls — the module is the same 2 × 5.2 + 6.
    - **A pair's second row is dropped where the lot ends right behind it** (less than an
      aisle of room left): its back is in its pair and its nose in the kerb. The strip
      stays as asphalt. The one row that legitimately has no aisle at all is a lot **one
      row wide** — a strip along a street, entered from the street, and that exception is
      what `every_row_has_an_aisle_to_drive_in_from` skips.
    - **Along, `row_places`: `ROW_BLOCK` 50 m of stalls, a cross aisle, 50 m more.** The
      breaks are computed once per lot and shared by every row, so they line up into a
      drive across the field rather than a scatter of empty stalls. Fifty metres is 19
      stalls, an ordinary block between drives; without it **199 lots take the invented
      layout, 97 of them would run a row longer than 50 m and 33 longer than 100, the
      longest 258 m** (`w605838911`; cache 7600 × 5700, v14), where the aerial photo (2GIS,
      Tula's mall lot, zones A–H) shows blocks with drives between them. The mall lot is
      the picture the rule was read off, not a case it applies to — that one is striped by
      `aisle_rows` and never reaches `row_places`.
  - **A stall survives three tests**, and all three came out of one report: single bars
    left along the edge of a lot with nothing between them.
    - **All four corners inside the outline** (`fits`, the test the roof clutter uses for
      its boxes) — an L-shaped lot gets nothing in the notch, and the rows do not have to
      match the outline. **The extra `EDGE_MARGIN` of clearance** (`fits_with`), rather
      than merely falling inside, is a rule of the **aisle-driven** row only, where the
      row is placed by an OSM aisle and is tied to the outline in no way: a stall whose
      corner sits on a skewed kerb there reads as a half stall. The invented layout gets
      that clearance from its own grid — `generated_rows` insets by `EDGE_MARGIN` from the
      bounding box on all four sides — and asks only `fits`. **Demanding more of it was
      measured and refused**: the grid is counted off the bounding box while the clearance
      would be to the real outline, so on a skewed quadrilateral the wall-side row dies
      whole instead of shifting — −13.1 % of stalls over Tula's 277 yard lots and 16 lots
      emptied outright (way 775607821, 129 × 16.5 m, 44 → 0; ways 797465631 and 1272152742
      likewise; cache 7600 × 5700, v14). `fits` already puts all four corners inside, so
      no body hangs over the kerb, and `push_markings` tests the **bar** with
      `fits_with(EDGE_MARGIN)` on both paths anyway, so the stroke into nothing is already
      suppressed.
    - **A run of at least `MIN_ROW_RUN` 2 stalls.** At a wedge a row shrinks to one
      stall, which is a bar, a gap and a bar on empty asphalt; the whole run is dropped.
      A rule of **any** stall — `Frame::push_row` and `generated_rows` cut their runs the
      same way, and on the invented layout a break by `row_places` (a cross drive)
      deliberately does not end a run.
    - **Asphalt in front of the nose** (`reachable`, `PAIR_AISLE/2` ahead, probed at
      *both* front corners because at a skewed corner the middle is still on asphalt when
      half the exit is off it). A rule of **any** stall too, and on the invented layout it
      is meaningful only together with `row_noses` above — with one nose per lot it would
      delete the wall-side row rather than turn it. **A lot striped one row wide is not
      asked**: it is a strip along a street, entered from the street, and the outline knows
      nothing of the street — the same exception the bullet above states and
      `every_row_has_an_aisle_to_drive_in_from` skips. The lane grid is continued past the
      outermost aisle, so without this a row lands where its drive is already outside the
      lot. Cost of the two rules on the invented layout: **−180 stalls of 8981 (−2.0 %)
      over Tula's 277 yard lots, 5 of them emptied** — exactly the lots that hold one
      stall today, which is the lone bar on empty asphalt `MIN_ROW_RUN` exists against.
  - **`MIN_AREA` 120 m²** — under that the lot gets no paint at all. A yard for four cars
    is not striped in reality, and stripes on a 6 × 10 m patch read as a texture bug.
    **The stalls themselves stay**: `MIN_AREA` gates `push_markings` only, `fill_lots`
    reads the `ParkingLayout` unfiltered, so a small yard keeps its cars — on unmarked
    asphalt.
  - The paint is drawn as the **border between stalls** (one bar to the left of each
    stall, neighbours coinciding), not as a rectangle per stall: that is what a lot looks
    like, and it is cheaper than finding each stall's neighbour. Plus a **closing** bar
    where a run of stalls begins — at a cross aisle — found by asking
    whether the *previous* stall in the list stands one stall width away, since the stalls
    of a row are generated in order. Without it every block ended in a stall open to the
    drive, which is exactly what the cross aisles were added to stop looking like.
    **Any bar is drawn only where a stall would fit on its other side** (`fits_with` at
    `EDGE_MARGIN`). A bar divides two stalls; at the end of a row it divided a stall from
    the kerb, which is a line nobody paints and which read as a stroke into nothing —
    the author's report, three of them left on the hospital lot. Probing merely for
    asphalt half a stall past the bar did **not** settle it: the wedge beyond the last
    stall of a skewed kerb is still asphalt, so the bar survived the test.
    A bar also stops `LINE_GAP` 0.5 m short of a stall's back, or the two rows of a pair
    read as one line straight through both of them.
    **"In order" is a contract on the list, not an observation**: a row has to be emitted
    along `-perp(Stall::along)`. The invented layout satisfies it by construction; along
    an aisle only the near side does, so `aisle_rows` walks the far side **back to
    front**. Emitting it forwards is not a crash — it silently doubles every bar, two
    coincident quads per stall.
  - **The lot lies over every road ribbon and sidewalk** (`Z_PARKING` above `Z_ROAD`,
    under the pitch and water). OSM runs aisles, entries and footways into and through
    a lot, and a light ribbon over the lot's asphalt cut the stall rows — reported from
    screenshots. With the lot on top its own outline clips every ribbon exactly, and
    the lot's asphalt *is* the aisle.
    **Cutting the ribbon's axis at the outline was tried first and removed**: a ribbon
    has width, so the cut left a round cap poking into the lot where an aisle ends on
    the outline, a square end beside a pointed lot corner, a step where an aisle running
    along the edge crosses it, and the light ribbon of a road that crosses the lot. The
    price of the lot on top is stated: an outline mapped over a real carriageway hides
    that stretch of asphalt, markings and sidewalk — the kerb cars lie above (`Z_CAR`)
    and stay. Render-only: the navmesh, doors and tree planting still see every road.
  - **On a small lot the stall layout leaves no gap for a road, and must not.** While the
    road ribbons were still drawn over every lot, stalls under a crossing road were
    dropped; with the lot on top that road is hidden, and the dropped stalls read as an
    unexplained empty band across the rows — the author's call from a screenshot. The
    layout's own `AISLE` gaps — and, since the cross aisles, its own drives across the
    field — stand in for the OSM aisles. **The rule that a gap and a visible road go
    together is the same one the next bullet obeys from the other side.**
  - **A big lot shows the road through it** — the author's later call, on ТРЦ «Макси»:
    hiding the roads is right for a yard and wrong for eight hectares, which came out as
    a field of hatching with no landmark in it, while 2GIS and Yandex draw the boulevard.
    - **Big** is `parking::is_ground`, the outline at `GROUND_MIN_AREA` 8000 m² or more
      (six lots in Tula). **Through** is `parking::is_through`: a `Street` that is not a
      `parking_aisle`, a bridge or a passage, and is **one-way, a roundabout or a
      carriageway**. That is the one thing tags say about it: the mall's boulevard is
      `highway=service` + `oneway=yes` with three mini-roundabouts, while the plain
      `service` ways inside the other five big lots are their aisles, merely untagged —
      not one of them one-way (measured on the cache: 838 m of one-way road inside lots,
      all of it at the mall). Showing every `service` way would have cut those lots' rows
      along every aisle.
    - **Drawn by `roads/lots.rs`, inside `mesh_roads`** (so it follows `RoadStyle` and
      the very centreline the ribbon underneath is drawn from): `Grounds::push` hands
      every street's drawn path to the big lots it reaches, `Grounds::layers` builds two
      layers from them. **The road's asphalt is not laid a second time** — it is the
      lot's own tone; what shows the road is its **kerb**, `lot_sidewalks`
      (`Z_LOT_SIDEWALK` 2.002, `SurfaceKind::Sidewalk`, `SIDEWALK_COLOR` — measured
      pixel for pixel the same as a street's sidewalk, 210/208/204; a thin light strip
      on dark asphalt only *looks* whiter). `RoadStyle::sidewalks` off takes it off.
    - **The kerb is a polygon** (`kerbs`, `i_overlay`): the bands of the through roads
      whose axis enters the lot (`width + 2 · parking::kerb_width` — its sidewalk, or
      `LOT_KERB` 1.2 m on a drive) plus the **island of every roundabout** (the ring's
      own polygon), **minus the asphalt of every street on the lot** — an aisle cuts its
      **mouth**, so along the boulevard the kerb comes out as the islands at the row
      ends — **and minus the gores** at the roundabouts (next bullet but two), where
      there is no kerb between the carriageways at all. The result is cut to the lot's
      outline **grown by `KERB_OVERHANG` 2 m**, **opened** by `KERB_OPENING` 0.3 m
      (slivers under 0.6 m go, corners round) and scraps under `MIN_KERB_AREA` 6 m² are
      dropped. The overhang is what joins the kerb to the sidewalk of the street the road
      enters the lot from: cut exactly on the outline it met that sidewalk with a jog —
      the outline is rounded there by the paving's closing and the sidewalk is not.
    - **Closedness comes from the raw OSM points, never from the drawn path**
      (`Grounds::push`, `GoreRoad::new`): whether a way is a ring is a property of the
      way, not of a style knob. It used to be load-bearing for a second reason —
      `centerline` smoothed a closed way as an open one and cut the corner at its
      **seam**, so the two ends of a drawn ring stood metres apart (8.6 m on a 12 m test
      ring) and `is_ring` on it was false unless the seam node happened to be shared and
      therefore pinned. That is fixed at the source (a closed way is smoothed round the cycle —
      **The street axis** in `roads.md`), so the
      re-closing both modules do is now a belt: a drawn ring already comes back closed.
      The original was found the hard way — the unit test passed on a tagged ring, the
      city showed nothing.
    - **Ribbons were here first and were removed** (a `lot_roads` layer of every
      street's asphalt over the kerb layer, runs clipped to the outline by bisection).
      Each road's asphalt lay over every *other* road's kerb, so where slip lanes fan
      out at a roundabout the kerbs were chopped into ragged scraps, the island stayed a
      ring, and the two kerbs of the boulevard's halves fused into a light thread
      between them — the author's report. Do not bring the ribbons back.
    - **The median is a double solid line, not a kerb** (layer `lot_lines`,
      `Z_LOT_LINES` 2.003, flat paint, gated on `RoadStyle::markings`). A boulevard in
      OSM is two one-way ways side by side with half a metre of asphalt between them —
      **paired halves**, found and aligned for the whole city by the network
      (`roads/network/pairs.rs`, `roads.md` → **Paired halves**; the lot looked for them
      itself until then, on its own streets only). The lot takes each **paved** median's
      midline, cuts out the stretch over the lot by probes every `PROBE_STEP` 2 m
      (`on_lot` — a straight midline is two points, both often outside the lot) no
      shorter than `MEDIAN_MIN` (= `PAIR_MIN` 8 m), lays it with `push_rails`
      (`DOUBLE_LINE_WIDTH` 0.2 m at `DOUBLE_LINE_GAUGE` 0.5 m — wider than the real
      0.15 / 0.3, which fuse into a hair at lot zoom) and subtracts a strip as wide as
      the axes are apart from the kerb. Its own line is needed because the street paint
      layer (which draws the same double solid everywhere else) lies under the lot's
      asphalt. The median is computed **once per pair**: from both sides at once (the
      lot's first try) two double lines lay on each other a few centimetres off — the
      author's report. The «Макси» boulevard is two `service` drives, which is why a
      `service` one-way is `pairable`. **At a gore** (`Gores::reach`, shared with the
      paint layer) the ends of the midline lying inside the hatching are trimmed — the
      median exists while the gap is under 3 m, the gore from 0.6 m up, so they overlap
      — and the line is then carried on along its heading up to `MEDIAN_REACH` 8 m,
      stopping `MEDIAN_GORE_GAP` 0.6 m short of the hatching: flush, its end fused with
      the island's outline.
    - **Gores — the splitter islands at a roundabout** (`roads/gores.rs`, the author's
      ask: «как у Яндекса»). An approach in OSM is two one-way ways, entry and exit,
      fanning out to two nodes of the ring; the wedge between them and the ring is flat
      asphalt with diagonal hatching, nobody drives or walks on it. We drew a triangle of
      **sidewalk** there (the two arms' bands overlapping), and on the big lot a kerb
      triangle — outlined, then filled, with a crescent of asphalt left in it. It is a
      property of the **network at a ring, not of a lot**, so it is computed for every
      roundabout in the city and the lot only subtracts it from its kerb.
      - **A roundabout is the tag or the shape** — `RoadLine::is_roundabout`, the one
        notion, on the model: `junction=roundabout|circular`, or a closed one-way way.
        The big ring at ТРЦ «Макси» (way 397005605) is plain `oneway=yes` closed on
        itself; by the tag alone none of its three islands was found. The shape test
        started here and was lifted to the model when `lane_count` and the parked cars
        turned out to need the same answer — see **Markings → lane_count**.
      - **An arm** is a one-way, non-ring street with an end within `ARM_SNAP` 1 m of a
        ring vertex, taken for `ARM_REACH` 40 m from the ring — an avenue's carriageway
        runs for hundreds of metres and would hatch the whole median with its opposite.
      - **The wedge** is what a closing by `GORE_CLOSING` 6 m of the rings' and arms'
        asphalt pulls shut, minus the asphalt of every street nearby and the island of a
        small ring. The same closing fills the fillet on the **outer** side of an arm,
        and arms meet a ring at a shallow angle, so that fillet is not small: what tells
        them apart is the neighbours, not the area — a gore **touches two arms**
        (`ARM_TOUCH`), a fillet one. `GORE_MIN_AREA` 12 m² on top.
      - **The subtraction is one wedge's business.** The closing has already broken the
        city into separate shapes, and asphalt from the other end of town touches none of
        them, so the difference runs per closed shape against the clip contours whose
        bounds overlap it, and is skipped outright where the bounds say fewer than two
        arms can reach (the same question the exact test asks below, asked of a rectangle
        first). One boolean for all of them cost two to three times as much — half the
        work went on the road bands intersecting each other where there is no wedge at
        all. **A street reaches a wedge by its kerb, not by its axis** — eight metres on
        an avenue — so the bounds it is tested against are grown by its half width;
        by the axis alone one bit of asphalt was missed, and a gore kept a 3-vertex
        sliver of road under it.
      - **Two shapes, not one.** The whole wedge, grown `ASPHALT_PAD` 0.5 m under the
        road edges, is **asphalt** (pushed into the `roads` layer — above the sidewalks,
        so it covers their triangle); the wedge **opened** by `GORE_OPENING` 0.3 m —
        without tips and necks too thin for a stripe — is what gets the outline
        (0.2 m) and the hatching (0.35 m every 1.6 m, at 45° to the wedge's long axis).
        One shape for both (tried first) showed ground wherever the opening had cut.
      - **The paint is the road paint layer's** (roads plan, stage 6): `Gores::islands`
        hands each hatched shape and the direction across its stripes to
        `Painter::paint_island`, which lays the outline as a closed paint strip
        (`LineKind::Edge`) and the shape itself as a paint area (`LineKind::Hatch`,
        `MeshBuilder::push_paint_area` — the ribbon attribute carries the world
        coordinate across the stripes), and `paint.wgsl` draws the stripes and fades them
        into their mean share when they get finer than a few pixels, with the zebras'
        zoom (`PaintTag::Zebras`). Its own mesh, `road_paint_islands` at
        `Z_ROAD_ISLANDS` 2.0035: above a lot's asphalt and its double line, where the
        rest of the road paint would be covered. Before, the stripes were real quads
        clipped by one boolean for the city, in `lot_lines`, with no LOD at all. Gated on
        `RoadStyle::markings`; the count is `gores N` in the `road meshing:` line. Tula:
        **11** after stage 6 (10 before it) — three at the mall's big ring, three at the
        boulevard's mini-roundabouts, the rest at the two big rings.
      - **Splitters by rule** (`gores::splitters`): an approach mapped as **one two-way
        way** has no fan, so the closing finds no wedge. A carriageway of two lanes or
        more ending at a node of a drawn ring (`roads/rings.rs`) of radius ≥ 10 m gets a
        teardrop on its axis — from 1 m past the ring's kerb, `0.6 × radius` long (6–20 m),
        `0.06 × radius` half wide at the base (0.6–1.5 m) — hatched like a gore; the
        approach is widened around it by an asphalt flare so each lane keeps its width,
        and its paint and ruts break over the island's length. Tula has none (its big
        rings are fed by one-way fans, its two-way approaches are service drives);
        `roads/tests.rs::a_two_way_approach_gets_a_splitter_island` holds it.
    - `Z_PARKING_LINES` lies **above** both layers, so a stall bar is never covered.
    - **Cost — real, and per road rebuild, not per frame.** `mesh_roads` on Tula is
      **76 ms** (`map_meshing`, **release**, one machine; the bench swings by ±10 %), of
      which the two big-lot layers and the gores are about **36**: `Gores::of` 13.6 (2.2
      of strokes, 10.5 the closing and the subtraction — 0.65 the simplify, 3.4 the grow,
      3.3 the shrink, 3.1 the differences; the grow and the shrink are what is left to
      cut, and splitting *them* per cluster made them slower, the offset having a fixed
      cost per call), the kerb polygon 17 (the mall lot alone is ~90 round-joined
      strokes through the booleans and an opening), the medians 2 (every sample against every other carriageway;
      bounds-filtered — measured before the search moved into the network, see
      `roads.md` → **Paired halves** for its cost now), the `enters` probes and `GoreRoad::new` together 2.4, the gores'
      asphalt and hatching 0.7. The other ~40 are older work the lots did not add —
      the ribbons 16, `network::stitches` 10 (of which the *search* is 1: the rest is
      building the two grids it asks, and both got cheaper with the hasher — see **The
      uniform grid**), the kerb returns 7, the bridge shadows 5, the
      nodes and centrelines 4. It is paid at world load and whenever
      `roads::rebuilds_on` fires (`RoadStyle`, the settled `RoadShapeOnMap`, the settled
      sun).
      **`dev` is only ~1.3× slower than `release` here, and that is the measurement to
      know before optimising**: what these steps spend is `i_overlay`, a dependency, and
      a dependency is built optimised in `dev` too (`opt-level = 1` applies to our code
      alone). `pave_lots` said it plainest — 119–133 ms release against 129–137 dev,
      i.e. nothing. So a `dev` bench number here is a real number, not one to be
      discounted by an imagined release factor.
      Not optimised: the kerb and the gores depend on the map, the settled `RoadShape`
      (curve tolerance) and two `RoadStyle` toggles only (`sidewalks`, `markings`) and on
      the sun not at all, so caching them
      per world load is the obvious cut — but the hitch on a sun change is **not** theirs
      to fix: the building layer rebuilds on the same `SunOnMap` and costs 145–190 ms,
      against these 76.
    - **No stall under a through road or its kerb** (`Surroundings::cover`, the band
      `width / 2 + kerb + THROUGH_CLEARANCE` 0.5 m, four corners and the centre; a cheap
      centre-distance reject first). 250 stalls on the mall lot.
    - **The perimeter drive gets no kerb**, by construction rather than by a rule: a road
      band is not part of the paved outline (see **Blocks pulled to the roads**), so its
      axis is outside the lot (`enters` is false) and it only cuts, never adds. The lot's
      asphalt simply runs into the drive's.
    - A carriageway crossing a big lot loses its lane markings on it (the lot's asphalt
      covers its ribbon) — the stated price; Tula has no such road.
  - **But the outline itself reaches the road**, in the parse (**Blocks pulled to the
    roads** above, `pave_lots`). That is not the layout reading roads: the lot grows to
    the drive it is entered from, and everything below — the stalls, the paint, the cars —
    reads the grown outline knowing nothing of why it grew.
  - **What the layout does read of the roads around a lot** (`Surroundings`, built per lot
    in `ParkingLayout::new` from the streets whose bounds come within `AISLE` of it):
    the through roads above, and the **drives** — every street band beside the lot is
    asphalt a stall can be entered from (`reachable` asks the lot **or**
    `Surroundings::paved`). Without it a strip along a drive lost every stall: its nose
    is in the drive, and the drive is not in the outline. On a big lot a through road is
    left out of the drives — there is a kerb in the way.
  - **Aisle fields** (`fields_of`). A big lot is not always one grid: the mall's 44
    aisles run at 102–105° and the six of its east wing at 59–61°, along their own edge.
    One direction per lot striped that wing with rows of the main field, across the
    wing's own aisles. Fields are peeled off in turn by the same `main_of` over the links
    not yet taken; a second field needs `FIELD_MIN_LANES` 3 distinct lanes, which is what
    keeps the hospital lot's chain of spurs (one line, one lane) from becoming one. The
    lane grid of each field still runs out to the outline, so **whose ground it is** is
    decided per stall: the nearest aisle run must be of the stall's own field
    (`Frame::territory`). A lot with one field is never asked.
  - **Aisle blocks** (`blocks_of`) — a field is split once more, and each block is a
    `Field` with its **own lane grid**. A lane is one number, an offset across the lot,
    and the grid used to be one per direction. At the mall the aisles north and south of
    the boulevard are separate ways that do not stand opposite each other — by the
    roundabout a northern lane falls between two southern ones — so in the shared list
    they cut an honest 17 m pocket into 6 and 11: **one** row stood in the eleven,
    nothing in the six, and single rows with a double drive ran across the middle of the
    lot (the author's report). Runs join a block by two rules:
    - **neighbours** — within `BLOCK_LANE_REACH` 40 m across (two steps of the grid: a
      pocket may carry a footway instead of an aisle) and overlapping along by
      `BLOCK_OVERLAP_SHARE` 0.5 of the shorter. The share matters: the boulevard crosses
      the aisles ~26° off square, so a northern run and the southern run one lane over
      overlap by a couple of metres without being neighbours;
    - **pieces of one lane** — collinear within `BLOCK_COLLINEAR` 1 m, up to two `AISLE`
      apart, and **no through road between them** (`Surroundings::crossed`): the gap on
      the boulevard is as wide as a cross drive's, and only the road tells them apart.
      Without this rule the cross drive would split its block and `cross_drives_of` would
      find nothing.

    A block stripes `BLOCK_REACH` 30 m past the ends of its own runs, not the whole lot —
    the rest is another block's by `Frame::territory` anyway — and `push_row` asks the
    outline before `owns`, which walks every run of the lot. Layout 38 → **30 ms** on
    Tula. Pinned by `aisles_across_a_through_road_keep_their_own_lane_grids`.
  - **A pair of rows is laid as a rectangle** (`Frame::push_pair`). The two rows of a
    back-to-back pair were tested apart, and in a skewed block — between two roads
    crossing the aisles at an angle — each was cut its own way: three stalls on the
    left, five on the right and shifted. It read as «a row and a scrap of a second one
    beside it» (the author's report), not as a pair. Two rules:
    - **whose ground it is is asked once per pair**, at the pair's centre (`cells`'s
      `anchor`), not per stall: on the seam of two blocks the left row went to one and
      the right to the other, each from its own grid;
    - **a short overhang is trimmed**: stalls of one row with no partner behind them, up
      to `PAIR_OVERHANG` 2 in a run, go. A longer run stays — that is a row along a
      skewed edge where the second one has nowhere to stand, and a single row at the
      edge is legitimate.

    `push_row` is now `lay(cells(..))`: `cells` answers per slot of the lane's grid,
    `lay` applies `MIN_ROW_RUN` and the list order `push_markings` expects. Pinned by
    `a_pair_of_rows_cut_askew_is_trimmed_to_a_rectangle`.
  - **Pockets** (`pocket_depth`, `pocket_rows`) — a one-row strip along a street (way
    702257069, 354 × 5.7 m). With `EDGE_MARGIN` either side no stall fits it, and it was
    a dark band by the pavement; before the polygon paving it was striped only because
    the outline had been stretched over the street. Recognised by thickness,
    `2 · area / perimeter`, from `STALL_DEPTH_MIN` up to a stall with both margins; the
    stall takes the strip's own depth with `STRIP_MARGIN` 0.2 m (clearance for the
    corner test, not a design gap). The row is laid **along the sides of the outline**,
    longest first, not on the bounding-box grid — a strip a third of a kilometre long
    bends with its street, and one degree of bend walks a straight grid out of it;
    the row along the opposite side lands on the first and is dropped by `Placed`. One
    nose per side, toward the street if `paved` finds it in front. The bar test follows
    (`push_markings`): it asks for `EDGE_MARGIN` beyond the bar **along the row** and for
    as much margin in depth as the stall itself has, or a pocket would get no paint.
  - **Overlapping lots do not share stalls.** Two lots paved to one drive both hold the
    gap between them, and both layouts striped it — cars of two lots parked across each
    other (the mall's north-west lot and the two pockets along its drive). Lots are laid
    out from the largest down against one shared `Placed`, and a stall landing on one
    already standing is dropped; results are stored in map order, so
    `ParkingLayout.0[i]` is still `MapData::parking[i]`.
  - **`Outline`** — every point test of the layout and the markings goes through an index
    of the ring's edges by horizontal bands of `OUTLINE_BAND` 4 m (same even-odd answer
    as `point_in_area`, pinned by `the_outline_index_agrees_with_the_full_ring`, which
    checks it against the full walk on the three shapes the paving makes: a concave
    outline with a hole, a lot with a hole out to the edge of its bounding box, and a
    thin part cut off by the apron). A paved
    outline carries its fillets: the mall lot is 1220 vertices, and walking the ring for
    each of tens of thousands of probes cost **87 ms on that lot alone**. With the index
    the whole city's layout is **24 ms** (`map_meshing`'s `parking layout` row,
    release; 29–30 before its grid went off SipHash, 38 before the aisle blocks, 21
    before the paving, 2 before the pocket rule)
    — two fields, the through-road test and the shared `Placed` are the rest.
    **Every** point test means it: `aisle_rows` asked `point_in_area` for its three
    probes per aisle link long after the index was built and handed to it, and that is
    now the index too. It bought nothing measurable — 30 → 29 ms, inside the bench's
    swing, because fifty aisles against 1220 vertices is a fraction of the tens of
    thousands of probes the stalls make — and it is here so that there is one way to
    ask the outline a question rather than two.
  - Tula, cache v14: **349 lots** (355 in the bbox, less the five that carry `building`
    and the one `parking=multi-storey`). Parking touches neither the navmesh nor tree
    planting, like the landuse blocks.

## Parked cars

- **Parked cars** (`map/cars/`, the layer in `mod.rs` and the drawing in `body.rs`) — the second most recognisable thing on an aerial photo
  after the roofs themselves: a street with not one car on it reads as a drawing whatever
  it is painted. A row goes along **both sides of every carriageway** — `roads::is_carriageway`,
  the very predicate that decides where a sidewalk and lane markings go, opened up for this
  — minus a bridge (nobody parks on one) and a roundabout (you drive it, you don't park on
  it), both excluded by `parkable` rather than by the predicate, which markings still need
  them in. **Which sides, and where on them, is `roads::pockets`** (stage 7,
  `references/roads.md`, **Kerb pockets**; `parkable` moved there too): `parking:*=no` or
  a no-stopping restriction empties a side, a pocket side stands its cars `POCKET_WIDTH`
  further out inside the pocket's full-width part and nowhere else, and an untagged
  trunk/primary/secondary parks in pockets only — its lane stays clear. The ring is `RoadLine::is_roundabout`, **tag or shape**: by the bare tag a
  column of parked cars stood right round the mall's big ring, which is a closed one-way
  way with no `junction` tag. The threshold that stood here first was `road.width >= 9 m`,
  and it put every car on the avenues — while an aerial photo shows the housing blocks
  parked solid. The gate is **the class, not a width**: `is_carriageway` asks
  `Highway::is_street`, which lets `residential`/`unclassified`/`living_street` (and
  everything above them) in and keeps `service` and paths out, whatever their width —
  exactly the line wanted. `RoadLine::width` comes from the lane section
  (`network::sections`: lanes × lane width + two edges), so a width threshold would have
  cut two-lane streets (7.6 m) and one-lane one-ways (4.3 m) off. On a two-lane residential
  (7.6 m at the default `LANE_WIDTH_DEFAULT` 3.3) a sedan's row sits
  `7.6/2 − CURB_GAP − 1.8/2 = 2.4 m` off the axis (a van's own 1.95 m width narrows that to
  2.33 m — the offset is per-body, not a constant, same as the length below), leaving 3 m of
  carriageway between the two rows for a sedan — a yard, and it is pinned by
  `a_residential_street_gets_a_row`. Bodies of the size their **type** says (`CarShape`,
  4.4 × 1.8 m for a sedan up to 5.3 × 1.95 for a van)
  at `CAR_PITCH` 6 m, offset `CURB_GAP` + half of **that** body in from the kerb — the
  type is therefore rolled before the place, so a van stands as close to the kerb as a
  sedan does — with
  `CarStyle::occupancy` (`CAR_OCCUPANCY_DEFAULT`, 45 %) of the places taken (a solid row
  from junction to junction looks like a dealership)
  and `END_MARGIN` 2 m clear of each end — that margin is only about the drawn ribbon's
  butt, so a car does not hang off it; a junction is a different question, answered below.
  The pitch is walked along the **arclength of the whole street**, not segment by segment:
  a city polyline's link is routinely shorter than two margins, and the old
  `points.windows(2)` walk dropped every such link whole (51 % of Tula's segments, 35 % of
  its length) and reset the step at every vertex, so the row tore or doubled across a bend.
  `arclengths` + `place_on_path` (binary search, then interpolation) replace it, and one
  extra rule handles curvature: a place closer than **half the two bodies' lengths together**
  to the last car **placed on that side** is skipped, measured in world distance so it catches
  a corner and any other bend alike. The half-sum, not the new body's own length, is where two
  bodies nose to tail actually stop overlapping — while every car was the same 4.4 m the two
  were the same number, and with five `CarShape`s they part: a hatchback behind a van passed
  the own-length check overlapping it by up to 0.7 m. So `last` carries the length of the car
  it points at, not only its point. The place is recorded **before** the `PARK_SLOP` shift,
  because that shift is rolled after the check and pulling its roll forward would move the
  RNG stream and reposition every row in the city; it is 0.12 m across the row, and
  `cars_never_overlap_on_a_sharp_bend` carries it as the tolerance on the half-sum.
  Both helpers live in **`map/along.rs`** — the walk is a shared primitive the
  way `map/seed.rs` is, and it moved out of `cars.rs` the moment the wagons became its
  second caller, since a second copy of this walk is exactly what it exists to prevent.
  `place_on_path` answers on **both** edges of `[0, total]`: the walk hits the end exactly,
  and a refusal on the boundary would drop the last object of the row. A **repeated vertex
  does not eat a place** — a zero-length link has no direction, and `binary_search` is free
  to return any of the equal keys, so the answer used to rest on an unspecified detail of
  `std`; the position does not depend on that choice (coinciding vertices are one point),
  and the direction comes from the nearest link that has length (`direction_at`, forward
  first, then back). `None` is left only where there is no direction at all — under two
  points, or a polyline of zero length. `map/along/tests.rs` pins all four cases.
  - **Nobody parks by a ruler**, and a row that does reads as warehouse markings rather than
    a yard: every car is turned by `PARK_SKEW_DEGREES` (2.5°) and shifted across by
    `PARK_SLOP` (0.12 m), both through the LCG's `bell4` — a bell, so most of the row is
    almost straight and the odd car is visibly askew. Both numbers are small on purpose:
    `CURB_GAP` (0.5 m) has to swallow the skew, or a corner of a body ends up on the lane
    markings.
  - **The row breaks at real junctions, not at way ends.** It used to break at the ends of
    the OSM way, which is wrong in both directions at once: a way cut mid-street by a tag
    change tore the row for no reason, and a way running straight through a crossing parked
    cars in the middle of it. `roads/junctions.rs::marking_breaks` already answers this
    question for the lane markings — the module opens up and the layer calls it with
    `is_carriageway` over the **whole** `map.roads` slice (its `breaks` are indexed by the
    road's position in the input, and the participants must be every real street, not only
    the parkable ones: a residential street joining another has to break the row too). A
    place whose **body** comes within `Break::reach + JUNCTION_CLEARANCE` (6 m) of a break,
    measured **along** the street (the place sits at the kerb, and the straight line to the
    node overstated the distance), is dropped. Six metres, not five: a rule zebra stands
    1–5 m past the junction edge, and a car measured by its centre five metres out stood
    on it nose first (the author's report, Чапаева at Кирова, 7375 3707). A dead end
    arrives as a break of reach 0, so the clearance empties the same metres there; two way
    ends meeting are not a break at all, which is the half of the defect that tore the row.
    The breaks are `roads::pockets::row_breaks`, not the bare `marking_breaks`: they add the
    **service drives** as participants (a car used to stand across a driveway, Ф. Энгельса
    at 3976 1236) and the **marked OSM crossings** (half a zebra, spilled onto the next way
    when the crossing is near its way's end) — see `references/roads.md`, **Kerb pockets**.
  - **Nor does a row stand where a street crosses a bridge** (`BridgeDeck`, pinned by
    `a_street_crossing_a_bridge_clears_the_row_under_the_deck`). A street under a bridge,
    or one butting into its side, shares no node with it, so `marking_breaks` sees no
    junction — and `Z_CAR` lies above `Z_BRIDGE`, so the car was drawn on the deck. A place
    within the deck's half width + `bridge_curb_width` + `JUNCTION_CLEARANCE` of a bridge
    centreline is dropped, the way a junction's is; the bridges are prefiltered per street
    by their AABB (grown by the street's width), so a place tests only the decks near it.
    Past the gap the RNG stream differs, exactly as past a junction.
  - **A one-way carriageway gets one row, on the kerb of the driving side**
    (`MapData::traffic_side`, see **Driving side** in `SKILL.md`; `TrafficSide::kerb`). With
    right-hand traffic, each half of a divided avenue has the kerb on the right and the
    median on the left; two rows would put a column of cars down the median, and in Tula
    145 of 218 `primary` ways are exactly such halves. London and Tokyo mirror it. The same
    rule is right for an ordinary one-way lane. `across` points left, so the right-hand
    side is `-1`; the direction it is right of is the way's own point order, which parse
    has already normalized (see **RoadLine** in `SKILL.md`).
  - **A car faces the traffic of its own kerb.** The row on the driving-side kerb points
    along the way, the opposite row against it — before this every car on the map faced
    the way's point order, so half of every two-way street was parked nose to the traffic.
    The side order of a two-way street (`[-1, 1]`) does **not** depend on the driving side,
    because it decides the RNG stream: the traffic side turns a row, it never moves one
    (`the_traffic_side_turns_the_row_without_moving_it`).
  - **Not cached, and that is measured, not assumed**: on Tula the `breaks` row
    (`pockets::row_breaks` since the bench took the game's breaks; the 1 ms was measured
    on the bare `marking_breaks`, see the rows below) is 1 ms against the 7 ms the layer costs at its far detail step and the 18 at
    its near one, and the layer itself is well under the building layer's 79 — a resource
    cached per world load would not pay for itself. The rows come from `measure_cars` in
    `examples/bench/map_meshing`; the 0.76 ms of 5.4 ms that stood here came off the `cars:`
    log line, which App Nap decides, and lumped the parking in with the mesh.
  Colours are a ten-slot
  palette in the shares a photo shows. Every car casts a shadow through the same
  `map::shadow_length_scale()` as the buildings — its length by the **type's own height**
  (1.5 m for a saloon, 2.3 for a van), which is why the van's shadow is visibly the longer
  one — and the mesh draws **all shadows first,
  then all bodies** — otherwise a car's shadow lands on top of the neighbour drawn before
  it. The layer is one merged **blended** mesh (the shadow is translucent, the body is not)
  at `Z_CAR` 2.7, above the tram and the rails (a car parks on the asphalt over the tracks)
  and below the portal stain.
  - **The shadow is a swept silhouette, the way a building's is** (`body::push_shadow` +
    `meshing::sweep_convex`, shared with the fences since) — the hull of the outline and the outline moved by the light, i.e. the
    Minkowski sum with the segment `[0, offset]`, so the shadow starts **under** the car and
    runs out from beneath it. What stood here before was the silhouette *translated* by the
    same offset, and at a low sun that copy detaches completely: a van at 15° is moved
    8.6 m, four times its own length, leaving the car and a separate dark patch beside it.
    At the default 59° the offset is 0.9 m and the two constructions differ only in the two
    notches at the flanks — which is why the defect was invisible until the elevation slider
    existed. The hull is built by an O(n) walk (an edge whose outward normal faces the light
    moves, the rest stay, and the two vertices in between carry both copies), never a sort;
    convexity is what `push_convex` needs and `the_shadow_sweep_is_convex` pins over the
    whole azimuth × elevation grid, `the_shadow_stays_under_the_car` pins the attachment.
  - **The edge is soft, by the buildings' own taper** — a `SHADOW_BLUR` (0.35 m) band
    fading to zero alpha, its width at each vertex `direction · shadow_dir()` clamped at
    zero, exactly `map/shadow.rs::penumbra`: hard where the shadow meets the car,
    full width at the far end, growing along the flanks. A metre there against a third of
    one here, because a building's shadow is three to ten times longer. Two consequences
    of that taper are load-bearing: the near edges collapse and **are not emitted at all**,
    which halves the band's vertices, and the car does **not** end up ringed by a soft
    fringe — that ring is the "contact skirt" the building shadows took out for reading as
    a grubby outline, and twenty-two thousand outlined cars would read the same way.
    The band is `Full` only: past `CAR_DETAIL_MAX_ZOOM` 0.35 m is under two pixels, and it
    costs four times the vertices of the shadow itself.
  - **The shadow's own contour is coarser than the body's** — `SHADOW_CORNERS`, the body's
    six-per-side outline with the two middle vertices dropped. Dropped rather than
    recomputed: a subset of a convex polygon's vertices is convex and lies inside it, so the
    shadow cannot poke out from under the body it should be hidden by.
  - **Shadows of neighbouring cars are not unioned**, unlike the buildings' — at the
    default sun the sweep is about a metre and `CAR_PITCH` is six, so there is nothing to
    overlap, and `i_overlay` over 22 k cars would cost more than the whole layer. At a low
    sun a row's shadows do overlap and stack into double-dark patches; that is the stated
    price.
  - **What a car is drawn as** (`cars/body.rs`) — from above a car is **not a rectangle**:
    it is a rounded silhouette with a dark cabin across the middle — windscreen, roof,
    backlight. Those three cross bands are what make the patch on the asphalt read as a car;
    the colour is second, and the flat coloured rectangle that stood here read as a crate
    precisely because it had none of them. Six points a side make the outline (nose and tail
    narrower than the midships by `Profile::nose` / `tail`), then the cabin is three quads —
    windscreen in `GLASS_FRONT` (lighter: the sky is in it), roof in the body colour
    lightened by `ROOF_LIGHTEN` (it faces straight up, so it is the brightest place on the
    car), backlight in the darker `GLASS_BACK` — and a pair of mirrors, the cheapest sign
    that the patch has a front. Painter's order inside the one merged mesh, as everywhere
    in `map`: the cabin is pushed after the body it lies on.
  - **The type is `CarShape`**, a ten-slot table in the shares a Russian yard shows (three
    sedans, three hatchbacks, a wagon, two crossovers, a van), and it decides both the
    metres (length, width, height) and the layout of the cabin **in fractions of them** —
    a sedan has a long boot, a hatchback only the overhang behind its backlight, a van's roof
    starts right behind the windscreen. Fractions rather than metres, so a body is described
    once and scales with its own size.
    **What the table actually delivers, in metres of boot** (`(backlight + 0.5) × length`):
    sedan 0.66, hatch 0.43, crossover 0.41, wagon 0.23, van 0.16 — so «a hatchback has none
    at all», which stood here and in `body.rs`'s module doc, overstated it: the hatchback's
    boot is two thirds of the sedan's, not zero, and `the_cabin_runs_from_nose_to_tail_in_order`
    forbids a zero (it requires `backlight > -0.5`). **It is a near-zoom difference**: the
    `Full` step runs from `MIN_ZOOM` 0.05 m/px out to `CAR_DETAIL_MAX_ZOOM` 0.18, and sedan
    against hatchback is 13 px of boot against 9 at the near end but 3.7 against 2.4 at the
    far one — at the far edge of `Full` only the van and (weakly) the wagon are told apart by
    the cabin, and the rest of the row differs by its colour and its length.
  - **Decoration, and deliberately so**: cars touch neither the navmesh nor the simulation
    and pawns walk through them. A parked row along every street would otherwise eat the
    pavements the entire crowd walks on.
  - **`CarStyle`** (resource, BRP-writable, persisted, settings group `cars`) is the whole
    style surface: `visible` (**on** by default) and `occupancy`. It is not a `RoadStyle`
    field for the tram's reason — that would remesh every road layer on a knob whose only
    effect is one merged mesh — and `rebuild_cars` is gated on `cars::rebuilds_on()`,
    `retuned::<CarZoomBucket>.or_else(retuned::<CarStyle>).or_else(retuned::<RoadShapeOnMap>)
    .or_else(retuned::<SunOnMap>)`,
    one registration by the rule under **When a layer rebuilds** in `SKILL.md`; the settled
    `RoadShapeOnMap` is in there because the row is walked along the **same street axis**
    the ribbon is drawn from (`axis::street_axes(.., &shape)`, never the raw OSM points)
    and breaks at the same taper clearings (`pockets::row_breaks(.., shape.taper())`), so
    the curve tolerance and the taper move the cars with the asphalt
    (`mesh_cars(bucket, style, shape, map, layout)`). The invisible case
    takes the same road as the far zoom bucket, and since the seam both of them live in
    `mesh_cars` rather than in the system: the adapter despawns the old layer
    unconditionally and is handed an empty list, so no second path can forget the
    despawn — and it is testable, which behind a `return` it was not.
  - **Its own zoom bucket** (`CarLods` / `CarZoomBucket`), and since the body has detail in
    it the table is no longer one threshold but four: `CAR_DETAIL_MAX_ZOOM` (0.18 m/px, a
    24-px car — glass and mirrors still read) → `CarDetail::Full`, `CAR_SILHOUETTE_MAX_ZOOM`
    (0.4) → `Silhouette` (the outline alone, since a windscreen there is under a pixel),
    `CAR_MAX_ZOOM` (0.8, a 4.4 m car at ~6 px) → `Block`, the plain rectangle, and past it
    no layer at all — still cheaper than any LOD of the drawing. `detail_for` is the one
    place the bucket index becomes a drawing, and the ladder only ever drops vertices.
    **The layer is built for the whole city, not for the frame**, so the detailed bucket
    pays for all 22 k cars at once: a one-off hitch on the threshold crossing, of the same
    nature as `RAIL_LODS`'s deepest bucket, which is what `CAR_DETAIL_MAX_ZOOM` is chosen to
    keep rare. Seeded per street (its first point, like doors and roofs), so the row is the
    same across rebuilds.
  - **The gallery** — `cargo run --example car_gallery` (`examples/demos/car_gallery/`, the
    shape of `roof_gallery`): eight cells, and they are **not** pretty streets but the list
    of shapes the row used to break on — straight, a ten-link polyline, a 90° bend, a T and
    a four-way crossing, a divided avenue, an 8 m residential street, a `service` drive and
    a bridge (both empty). Under each one, in the caption, what it is there to show. It may
    not roll its own geometry: `cars_mesh` is the one door out of `map/cars/` and the
    cells are described with the very `osm::fixture` the parse tests use, so a cell and a
    test talk about the same object. Its own is only the asphalt underneath, drawn with
    `MeshBuilder::push_ribbon` in the game's `ROAD_COLOR` — the brightness step between a
    body and the surface is half of how the row reads. `CAR_GALLERY_SHOT=path.png` takes one
    frame and exits, the way the roof gallery does and for the same reason. It has already
    earned its keep once: the divided-avenue cell was built with the two carriageways
    swapped (left-hand traffic), and the picture said so at a glance.
    The `panel.rs` and `params.rs` modules repeat identically across galleries — intentionally,
    so examples read top-to-bottom as self-contained units. The auto-shot logic is shared in
    `examples/demos/gallery_shot.rs`: it holds the frame counts and window-raise logic, both
    debugged facts (commit 21853a3), and fixes apply there to all galleries at once.
  - **The ninth cell is the stand** (`car_gallery/stand.rs`) — five body types × three
    detail steps, and it answers the other question: not *where* a row stands but *what*
    stands in it. Neither is readable off a street — the type falls out of the LCG and a van
    may simply not turn up — so this is the one place in the gallery where the type is
    **ordered** rather than rolled, exactly as the roof gallery orders a material
    (`RoofLook::new`) because the seed cannot reach every combination. The geometry is still
    the game's (`body::push_body` / `push_shadow`, the very calls the city layer makes) and
    still in metres; only the transform magnifies it (`stand::SCALE`), since a 4.5 m car
    next to streets hundreds of metres long is otherwise invisible. The `Detail` knob drives
    the street cells, never the stand — the stand shows all three steps at once.
  - Tula at the default occupancy, from `examples/bench/map_meshing` (`dev` profile, one
    machine, so compare runs against runs): **14 669 cars along the kerbs** — the bench
    does not fill the lots — and per detail step
    **968 k verts / 18 ms** (Full), **322 k / 7 ms** (Silhouette), **146 k / 3 ms**
    (Block). **Those are the mesh rows
    alone**; the three steps in front of them do not depend on the detail and are measured
    once each — `breaks` 1 ms (measured on the bare `marking_breaks`, before the bench
    took the game's `pockets::row_breaks` with its tapers and crossings — re-measure
    before quoting it), `districts` 3 ms (the index) and
    `parking` 4 ms (`park_cars`) — so a rebuild is 26 ms at the near step and 11 at the far
    one.
    **The district multiplier paid for itself and then some**, measured before and after on
    one machine: 21 929 → 14 669 cars (−33 %), and the row went **45.7 → 36.1 ms** — the
    3 ms index and the one millisecond the queries added to `parking` against 8 ms of mesh
    that is no longer laid (Full 26 → 18, Silhouette 11 → 7, Block 5 → 3). Fewer cars is
    not a cost here, it is the correction.
    **The swept shadow is what most of the near step's growth bought** (971 k / 15 ms
    before it, on the same machine and the same run of the buildings' 785 k / 79 ms): the
    sweep's hull is two vertices *cheaper* than the translated copy was — which is why
    Silhouette went *down* from 529 k — and the whole of the +485 k is the soft edge, six
    band quads per car at the near step alone. Keep `parking` on its own
    timer: while it sat inside the mesh timer the layer's milliseconds compared with
    nothing — not with the `cars:` line the app logs (which has always included it), and
    not with the older single-row runs. Next to the building layer
    (792 k verts, 101 ms — the same run, see **What it costs** under Roof clutter in
    `references/buildings.md`) and above
    the rail layer's deepest bucket (673 k, 23 ms) — still a
    layer built once per rebuild that costs nothing per frame.
    - **The body outline goes through `MeshBuilder::push_convex`, not `push_polygon`**, and
      that is most of those milliseconds: `push_polygon` calls `earcutr`, which on a
      12-vertex contour costs several times the laying-out itself and runs twice per car
      (body and shadow), 22 k cars over. The fan is correct because the outline is convex by
      construction, and `cars/body.rs::the_outline_is_convex` — an inline `mod tests`, there
      is no `body/tests.rs` — is what keeps it that way.
      Measured: Full 40 → 15 ms, Silhouette 35 → 9 ms, vertices unchanged.
    - **The lots are outside every one of those numbers**, and the way to read them off is
      the app's own `cars:` log line minus the bench's kerb count: **1 994** on Tula
      (16 663 in the app against 14 669 in the bench), at whatever the detail step of the
      moment costs per car. It was ~5 900 before the district multiplier reached the lots
      too — most of Tula's are in low- or mid-rise quarters.
  - **The lots are filled by the same pass** (`fill_lots`): every stall from
    `ParkingLayout`, a share of them taken **that falls with the lot's size**
    (`lot_occupancy`): `LOT_OCCUPANCY_SMALL` 50 % up to `LOT_SMALL_STALLS` 20 stalls,
    `LOT_OCCUPANCY_LARGE` 12 % from `LOT_LARGE_STALLS` 400, by the log of the stall count
    in between. It was a flat 55 %, and on a mall lot of hundreds of stalls that solid
    field of cars read as a dealership — the author's call from a screenshot. A yard
    is still fuller than a kerb, and an empty one next to a painted grid reads as
    unfinished. Seeded per lot
    (`lot_seed`, its first point) exactly like a street. That curve is built from
    constants, not from `CarStyle::occupancy`: the slider is about the ragged kerb row, and the half-empty
    lot is a different observation.
  - **How densely a place parks at all is decided by the district** (`cars/district.rs`,
    `Districts`), and it multiplies **both** shares — the kerb row's `CarStyle::occupancy`
    and the lot's `lot_occupancy`. Until it existed the layer knew only the width of the
    street and the size of the lot, and on the photo that is wrong twice over: a quarter of
    private houses parks in its own yards and shows singles on the street, while a
    microdistrict is solid because there is nowhere else. Tula measured off the cache:
    **98.7 of 188 km** of parkable street and **46 of 159 lots** are low-rise, i.e. half the
    city was parked to microdistrict norms.
    - **The reading is the area-weighted mean height** of the buildings within `REACH`
      120 m, in storeys, and the weighting is the load-bearing half. By count, twenty
      garages beside a nine-storey slab outvote it, though the cars belong to the slab; by
      area, a hundred small houses read the same as ten, which is right — a quarter does
      not get taller for being denser. A median reads worse here than a mean: the boundary
      between quarters has to be soft, and the weighted mean ramps — one section on the edge
      of a private sector already lifts it a little, a row of them lifts it all the way.
    - **The factor** is `LOW_FILL` 0.25 at `LOW_STOREYS` 2 and under, `HIGH_FILL` 1.15 from
      `HIGH_STOREYS` 5, linear between, and exactly **1** where nothing is in reach. Low is
      "far thinner" — a car about every fifty metres of kerb, one per two or three plots;
      high is deliberately only +15 %, because past that the row closes into a solid band
      and reads as the very dealership `lot_occupancy` exists to avoid.
    - **Every contour of `MapData::buildings` counts**, churches and the kremlin included:
      the measure is "how high is what stands here", which is what the eye reads off the
      photo, and a kremlin tower really does make its surroundings not a private sector.
      Each exclusion would need its own argument and is worth fractions of a metre on Tula.
    - **The height is `buildings::height_or_default`** — the same inference the building is
      *drawn* with (it went `pub(crate)` for this), never the raw `PolyArea::height`: 69 %
      of Tula has no tag, and a second notion of "how tall is this" would put a drawn
      nine-storey block in a quarter the cars treat as private sector.
    - **The index is a grid** of building centroids, `CELL` = `REACH`, each registered in
      every cell its radius touches, so a query reads one cell — the wagons' `Fan`
      construction. Built **per rebuild**, not cached per world load, for the junction
      breaks' reason: it is milliseconds on 7.6 k buildings against a layer that is
      percentages of the building one, and `measure_cars` prints it as its own `districts`
      row so that decision stays measured.
    - **Along a street the reading is refreshed every `DISTRICT_STEP` 48 m**, not per place:
      a query per each of 22 k places would cost more than the whole layer, and a quarter
      does not change from car to car. Forty-eight metres is a couple of private plots or
      the end of a section — the scale at which the city does change — and it is *arclength*
      of the street, so a way running out of the private sector into a microdistrict changes
      density where the city changes rather than where the way ends.
    - **It is RNG-stream-neutral.** Both call sites already rolled one `next_f32` per place
      and compared it against a share; the factor only scales the share, so the draws and
      their order are untouched and nothing else in the layer moves. The product is clamped
      to `[0, 1]`, so `occupancy = 1` still means "every place taken" on the slider.
    - **An empty `Districts` is a factor of 1 everywhere**, which is not a stub but the
      honest "no quarter to read": that is what `car_gallery` (no buildings at all) and the
      row-layout tests in `cars/mod.rs` build with, so they check the laying-out rule
      unmixed with the density rule. The density rule has its own tests in `district.rs`
      plus one end-to-end in `cars/mod.rs`
      (`the_same_street_parks_thinner_in_a_private_sector`).
