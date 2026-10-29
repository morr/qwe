# Parked cars

Detail behind `map/cars/` (the layer in `mod.rs`, the drawing in `body.rs`) — the
second most recognisable thing on an aerial photo after the roofs themselves: a street
with not one car on it reads as a drawing whatever it is painted. The stalls the lots
are filled from are laid out in `parking-layout.md`; the big lot's kerb and gores in
`parking-big-lot.md`.

## Where a row stands

- **A row goes along both sides of every carriageway** — `roads::is_carriageway`, the
  very predicate that decides where a sidewalk and lane markings go, opened up for
  this. **Which sides, and where on them, is `roads::pockets`**
  (`references/roads.md`, **Kerb pockets**): `parking:*=no` or a no-stopping
  restriction empties a side, a pocket side stands its cars `POCKET_WIDTH` further out
  inside the pocket's full-width part and nowhere else, and an untagged
  trunk/primary/secondary parks in pockets only — its lane stays clear. Bridges and
  roundabouts are excluded by `parkable` rather than by the predicate, which markings
  still need them in.
- **The gate is the class, not a width.** A width threshold (the first try was
  `road.width >= 9 m`) puts every car on the avenues while the housing blocks park
  solid; `RoadLine::width` comes from the lane section, so a width gate would have cut
  two-lane streets (7.6 m) and one-lane one-ways (4.3 m) off. `is_carriageway` asks
  `Highway::is_street`, which lets `residential`/`unclassified`/`living_street` (and
  everything above them) in and keeps `service` and paths out, whatever their width —
  exactly the line wanted (`a_residential_street_gets_a_row` pins it).
- Bodies of the size their **type** says (`CarShape`) at `CAR_PITCH` 6 m, offset
  `CURB_GAP` + half of **that** body in from the kerb — the type is therefore rolled
  before the place, so a van stands as close to the kerb as a sedan does — with
  `CarStyle::occupancy` (`CAR_OCCUPANCY_DEFAULT`, 45 %) of the places taken (a solid
  row from junction to junction looks like a dealership) and `END_MARGIN` 2 m clear of
  each end — that margin is only about the drawn ribbon's butt, so a car does not hang
  off it; a junction is a different question, answered below.
- **The pitch is walked along the arclength of the whole street**, not segment by
  segment: a city polyline's link is routinely shorter than two margins, and a
  per-segment walk dropped every such link whole and reset the step at every vertex,
  so the row tore or doubled across a bend. One extra rule handles curvature: a place
  closer than **half the two bodies' lengths together** to the last car **placed on
  that side** is skipped, measured in world distance so it catches a corner and any
  other bend alike. The half-sum, not the new body's own length, is where two bodies
  nose to tail actually stop overlapping. The place is recorded **before** the
  `PARK_SLOP` shift, because that shift is rolled after the check and pulling its roll
  forward would move the RNG stream and reposition every row in the city;
  `cars_never_overlap_on_a_sharp_bend` carries it as the tolerance on the half-sum.
  Both helpers live in **`map/along.rs`** — the walk is a shared primitive the way
  `map/seed.rs` is, and it moved out of `cars.rs` the moment the wagons became its
  second caller, since a second copy of this walk is exactly what it exists to
  prevent. `place_on_path` answers on **both** edges of `[0, total]`: the walk hits
  the end exactly, and a refusal on the boundary would drop the last object of the
  row. **A repeated vertex does not eat a place** — a zero-length link has no
  direction, and `binary_search` is free to return any of the equal keys, so the
  answer must not rest on an unspecified detail of `std`; the direction comes from the
  nearest link that has length (`direction_at`, forward first, then back). `None` is
  left only where there is no direction at all — under two points, or a polyline of
  zero length. `map/along/tests.rs` pins all four cases.
- **Nobody parks by a ruler**, and a row that does reads as warehouse markings rather
  than a yard: every car is turned by `PARK_SKEW_DEGREES` (2.5°) and shifted across by
  `PARK_SLOP` (0.12 m), both through the LCG's `bell4` — a bell, so most of the row is
  almost straight and the odd car is visibly askew. Both numbers are small on purpose:
  `CURB_GAP` (0.5 m) has to swallow the skew, or a corner of a body ends up on the
  lane markings.

## Where a row breaks

- **The row breaks at real junctions, not at way ends.** A way cut mid-street by a
  tag change would tear the row for no reason, and a way running straight through a
  crossing would park cars in the middle of it. `roads/junctions.rs::marking_breaks`
  already answers this question for the lane markings — the module opens up and the
  layer calls it with `is_carriageway` over the **whole** `map.roads` slice (its
  `breaks` are indexed by the road's position in the input, and the participants must
  be every real street, not only the parkable ones: a residential street joining
  another has to break the row too). A place whose **body** comes within
  `Break::reach + JUNCTION_CLEARANCE` (6 m) of a break, measured **along** the street
  (the place sits at the kerb, and the straight line to the node overstated the
  distance), is dropped. Six metres, not five: a rule zebra stands 1–5 m past the
  junction edge, and a car measured by its centre five metres out stood on it nose
  first. A dead end arrives as a break of reach 0, so the clearance empties the same
  metres there; two way ends meeting are not a break at all, which is the half of the
  defect that tore the row. The breaks are `roads::pockets::row_breaks`, not the bare
  `marking_breaks`: they add the **service drives** as participants and the **marked
  OSM crossings** — see `references/roads.md`, **Kerb pockets**. They arrive as
  `pockets::RowBreaks`, a type only `row_breaks` produces, and `park_cars` and
  `all_kerbsides` take nothing else — the ribbon's `Junctions::row()` is the same
  type, so the row and the pockets cannot be fed the base or the paint's breaks by
  mistake (`references/roads.md`, **Junctions**).
- **At a junction of carriageways the row keeps further off.** `park_cars` takes a
  second set of breaks, `junctions::marking_breaks(roads, is_carriageway)` — the
  carriageway nodes where the paint lays a stop line and a solid approach; dead ends
  (reach 0) are skipped — and a place is dropped, measured along the street from the
  body as above, when
  - its body is within `CROSSED_EDGE_CLEARANCE` 10 m of the **crossed carriageway's
    edge** (`reach − JUNCTION_MARGIN`): ПДД 12.4 says five, a car at five read as
    parked in the mouth;
  - or it stands **alongside the solid approach** (`paint::APPROACH` 25 m past the
    break's reach — one number with the paint) closer than `SOLID_LINE_CLEARANCE` 3 m
    to it (ПДД 12.4 again). The line is `SolidLine::of(road)`: none on a one-lane
    street, an unpaved one or `lane_markings=no`; `sections::EDGE_WIDTH` 0.5 m + one
    lane from the kerb otherwise — the **axis** of a two-way street of 2–3 lanes,
    solid on both sides of the node, else a **lane line**, solid only on the approach
    side (`ahead · kerb.heading > 0`). The gap is that distance minus `CURB_GAP` and
    the body width, plus `POCKET_WIDTH` in a pocket — a 1+1 street leaves 1.5 m, so
    its row starts ~30 m from the node on both kerbs; a car in a kerb pocket clears
    3 m and may stay.

  It takes a node on the main road too (its axis runs solid through the node,
  `near_spans(to_through)`), and it is geometry, not the paint's own breaks: the
  through road's lane lines, which the paint leaves dashed at a node it passes, are
  still cleared on the approach side. Crossings keep the plain 6 m. The gallery shows
  it with `ROADS_CARS=full`. Pinned by
  `cars/tests.rs::no_car_stands_along_the_solid_approach_of_a_narrow_street`.
- **Nor does a row stand where a street crosses a bridge** (`BridgeDeck`, pinned by
  `a_street_crossing_a_bridge_clears_the_row_under_the_deck`). A street under a
  bridge, or one butting into its side, shares no node with it, so `marking_breaks`
  sees no junction — and `Z_CAR` lies above `Z_BRIDGE`, so the car was drawn on the
  deck. A place within the deck's half width + `bridge_curb_width` +
  `JUNCTION_CLEARANCE` of a bridge centreline is dropped, the way a junction's is;
  the bridges are prefiltered per street by their AABB (grown by the street's width),
  so a place tests only the decks near it. Past the gap the RNG stream differs,
  exactly as past a junction.
- **No car stands on a railway or by its ballast** (`cars/rails.rs::RailKeepout`,
  pinned by `no_car_parks_on_a_track_crossing_the_street`). None of the three
  placements knows the tracks, and a station track crossing a street had the kerb row
  parked on its rails. So the keepout is **one last step of `park_on`**, after the
  kerb rows, the yards and the lots alike: a car whose body comes closer than
  `RAIL_CAR_GAP` 2.5 m to the ballast shoulder (half of `rail::deck_width`, the
  ballast's foot — 3 m on a 5 m mainline, so 5.5 m from the axis) is dropped. The
  body's reach toward the track is its half length × |cos| + half width × |sin| of
  its angle to the link (`CAR_HALF_LENGTH` 2.7 / `CAR_HALF_WIDTH` 1.0, the largest
  silhouette), so a car across the track keeps farther back than one alongside it.
  Filtering rather than breaking the row keeps the RNG stream of every other car
  where it was. Trams keep no zone — they run on the carriageway, and a zone would
  empty every tram street. A track on a bridge keeps its zone on purpose — a car at
  `Z_CAR` would be drawn over the slab (`layers.md`, **Track bridges**; pinned by
  `a_track_on_a_bridge_keeps_its_zone`). A rail gap stitched at parse
  (`references/parse-geometry.md`, **Rail gaps**) is a track like any other here.
- **A one-way carriageway gets one row, on the kerb of the driving side**
  (`MapData::traffic_side`, see **Driving side** in `SKILL.md`;
  `TrafficSide::kerb`). With right-hand traffic, each half of a divided avenue has
  the kerb on the right and the median on the left; two rows would put a column of
  cars down the median. London and Tokyo mirror it. The same rule is right for an
  ordinary one-way lane. `across` points left, so the right-hand side is `-1`; the
  direction it is right of is the way's own point order, which parse has already
  normalized (see **RoadLine** in `SKILL.md`).
- **A car faces the traffic of its own kerb.** The row on the driving-side kerb
  points along the way, the opposite row against it. The side order of a two-way
  street (`[-1, 1]`) does **not** depend on the driving side, because it decides the
  RNG stream: the traffic side turns a row, it never moves one
  (`the_traffic_side_turns_the_row_without_moving_it`).

## Caching and cost

- **Cached per placement** — `cars::CarPlacement` keeps the placed `Vec<Car>` (with
  the junction count) keyed on `(occupancy, RoadShape)`, reset on world entry by
  `forget_parked_cars`; a zoom-bucket crossing is `mesh_parked` alone. The skeleton
  under the placement is what a crossing still pays for: `Drawn::nodal` alone is
  25 ms on Tula and 49 on Kaluga, paid on every crossing of a car threshold (and even
  past `CAR_MAX_ZOOM`, where the layer is hidden). The rows come from
  `measure_cars` in `examples/bench/map_meshing`, which follows the game's split:
  `Drawn::nodal` as its `drawn` row, the placement once, then `mesh_parked` once per
  detail step — so a `cars *` row is exactly what a zoom crossing costs. Re-measure
  before quoting any number against those rows.
- The layer is one merged **blended** mesh (the shadow is translucent, the body is
  not) at `Z_CAR` 2.7, above the tram and the rails (a car parks on the asphalt over
  the tracks) and below the portal stain. The mesh draws **all shadows first, then
  all bodies** — otherwise a car's shadow lands on top of the neighbour drawn before
  it.
- **The bodies are built by threads** (`cars::mesh_bodies`): the cars are cut into
  one chunk per core (`available_parallelism`, no fewer than `BODY_CHUNK_MIN` 1024
  cars a chunk), each chunk lays its shadows and its bodies into two builders of its
  own, and `MeshBuilder::concat` glues them back **in the old order** — all the
  chunks' shadows, then all the chunks' bodies, indices shifted — so the mesh is the
  single-thread one byte for byte
  (`cars/tests.rs::the_threaded_mesh_is_the_sequential_one`). The glue is threaded
  too: done in one thread it cost as much as the chunks themselves (copying 50 MB and
  first-touching fresh pages, not arithmetic). The chunks go through the map's one
  fan-out, `map/parallel.rs::in_parallel` (one chunk runs on the calling thread with
  no spawn); the glue splits its slots by hand, since it writes into disjoint `&mut`
  slices, which `in_parallel` does not hand out. The threads are
  `std::thread::scope` rather than `ComputeTaskPool`, which the game gives a third of
  the cores (`main.rs` hands half to A*): a crossing holds the frame anyway. Two
  single-thread savings came with it — the body paints (body, roof, mirror in linear
  space) are computed once per palette slot (`body::Paint`), not three `powf`-heavy
  conversions per car, and the contours are arrays instead of a `Vec` per car.
- **The body outline goes through `MeshBuilder::push_convex`, not `push_polygon`**:
  `push_polygon` calls `earcutr`, which on a 12-vertex contour costs several times
  the laying-out itself and runs twice per car (body and shadow). The fan is correct
  because the outline is convex by construction, and `cars/body.rs::the_outline_is_convex`
  — an inline `mod tests`, there is no `body/tests.rs` — is what keeps it that way.

## The drawing

- **The shadow is a swept silhouette, the way a building's is** (`body::push_shadow` +
  `meshing::sweep_convex`, shared with the fences) — the hull of the outline and the
  outline moved by the light, i.e. the Minkowski sum with the segment `[0, offset]`,
  so the shadow starts **under** the car and runs out from beneath it. What stood here
  before was the silhouette *translated* by the same offset, and at a low sun that
  copy detaches completely — a van at 15° is moved 8.6 m, four times its own length,
  leaving the car and a separate dark patch beside it. The hull is built by an O(n)
  walk (an edge whose outward normal faces the light moves, the rest stay, and the
  two vertices in between carry both copies), never a sort; convexity is what
  `push_convex` needs and `the_shadow_sweep_is_convex` pins over the whole azimuth ×
  elevation grid, `the_shadow_stays_under_the_car` pins the attachment.
- **The edge is soft, by the buildings' own taper** — a `SHADOW_BLUR` (0.35 m) band
  fading to zero alpha, its width at each vertex `direction · shadow_dir()` clamped
  at zero, exactly `map/shadow.rs::penumbra`: hard where the shadow meets the car,
  full width at the far end, growing along the flanks. A metre there against a third
  of one here, because a building's shadow is three to ten times longer. Two
  consequences of that taper are load-bearing: the near edges collapse and **are not
  emitted at all**, which halves the band's vertices, and the car does **not** end up
  ringed by a soft fringe — that ring is the "contact skirt" the building shadows
  took out for reading as a grubby outline. The band is `Full` only: past
  `CAR_DETAIL_MAX_ZOOM` 0.35 m is under two pixels, and it costs four times the
  vertices of the shadow itself.
- **The shadow's own contour is coarser than the body's** — `SHADOW_CORNERS`, the
  body's six-per-side outline with the two middle vertices dropped. Dropped rather
  than recomputed: a subset of a convex polygon's vertices is convex and lies inside
  it, so the shadow cannot poke out from under the body it should be hidden by.
- **Shadows of neighbouring cars are not unioned**, unlike the buildings' — at the
  default sun the sweep is about a metre and `CAR_PITCH` is six, so there is nothing
  to overlap, and `i_overlay` over 22 k cars would cost more than the whole layer. At
  a low sun a row's shadows do overlap and stack into double-dark patches; that is
  the stated price.
- **What a car is drawn as** (`cars/body.rs`) — from above a car is **not a
  rectangle**: it is a rounded silhouette with a dark cabin across the middle —
  windscreen, roof, backlight. Those three cross bands are what make the patch on the
  asphalt read as a car; the colour is second, and the flat coloured rectangle that
  stood here read as a crate precisely because it had none of them. Six points a side
  make the outline (nose and tail narrower than the midships by `Profile::nose` /
  `tail`), then the cabin is three quads — windscreen in `GLASS_FRONT` (lighter: the
  sky is in it), roof in the body colour lightened by `ROOF_LIGHTEN` (it faces
  straight up, so it is the brightest place on the car), backlight in the darker
  `GLASS_BACK` — and a pair of mirrors, the cheapest sign that the patch has a front.
  Painter's order inside the one merged mesh, as everywhere in `map`: the cabin is
  pushed after the body it lies on.
- **The type is `CarShape`**, a ten-slot table in the shares a Russian yard shows
  (three sedans, three hatchbacks, a wagon, two crossovers, a van), and it decides
  both the metres (length, width, height) and the layout of the cabin **in fractions
  of them** — a sedan has a long boot, a hatchback only the overhang behind its
  backlight, a van's roof starts right behind the windscreen. Fractions rather than
  metres, so a body is described once and scales with its own size. In metres of boot
  (`(backlight + 0.5) × length`): sedan 0.66, hatch 0.43, crossover 0.41, wagon 0.23,
  van 0.16 — «a hatchback has none at all» overstated it: the hatchback's boot is two
  thirds of the sedan's, not zero, and
  `the_cabin_runs_from_nose_to_tail_in_order` forbids a zero (it requires
  `backlight > -0.5`). The difference is near-zoom only: at the far edge of `Full`
  only the van and (weakly) the wagon are told apart by the cabin, and the rest of
  the row differs by its colour and its length.
- **Its own zoom bucket** (`CarLods` / `CarZoomBucket`): `CAR_DETAIL_MAX_ZOOM`
  (0.18 m/px, a 24-px car — glass and mirrors still read) → `CarDetail::Full`,
  `CAR_SILHOUETTE_MAX_ZOOM` (0.4) → `Silhouette` (the outline alone, since a
  windscreen there is under a pixel), `CAR_MAX_ZOOM` (0.8, a 4.4 m car at ~6 px) →
  `Block`, the plain rectangle, and past it no layer at all. `detail_for` is the one
  place the bucket index becomes a drawing, and the ladder only ever drops vertices.
  **The layer is built for the whole city, not for the frame**, so the detailed
  bucket pays for all 22 k cars at once: a one-off hitch on the threshold crossing,
  of the same nature as `RAIL_LODS`'s deepest bucket, which is what
  `CAR_DETAIL_MAX_ZOOM` is chosen to keep rare. Seeded per street (its first point,
  like doors and roofs), so the row is the same across rebuilds.
- **`CarStyle`** (resource, BRP-writable, persisted, settings group `cars`) is the
  whole style surface: `visible` (**on** by default) and `occupancy`. It is not a
  `RoadStyle` field for the tram's reason — that would remesh every road layer on a
  knob whose only effect is one merged mesh — and `rebuild_cars` is gated on
  `cars::rebuilds_on()`,
  `retuned::<CarZoomBucket>.or_else(retuned::<CarStyle>).or_else(retuned::<RoadShapeOnMap>)
  .or_else(retuned::<SunOnMap>)`, one registration by the rule under **When a layer
  rebuilds** in `SKILL.md`; the settled `RoadShapeOnMap` is in there because the row
  is walked along the **same street axis** the ribbon is drawn from and breaks at the
  same taper clearings: the placement (`park_all`, cached in `CarPlacement` until the
  occupancy or the shape moves) builds `roads::Drawn::nodal(map, shape)` — the
  prepared roads without the stitches and merges the row has no use for — and takes
  its `Axis::Nodal` axes, its `tapers()` (`pockets::row_breaks`) and its `lots()`;
  the ribbon reads the very same values off its own `Drawn`
  (`references/roads.md`, **The drawn network**), so the curve tolerance and the
  taper move the cars with the asphalt. The invisible case takes the same road as the
  far zoom bucket, and since the seam both of them live in `mesh_cars` rather than in
  the system: the adapter despawns the old layer unconditionally and is handed an
  empty list, so no second path can forget the despawn — and it is testable, which
  behind a `return` it was not.

## The gallery

- **`cargo run --example car_gallery`** (`examples/demos/car_gallery/`): eight street
  cells, and they are **not** pretty streets but the list of shapes the row used to
  break on — straight, a ten-link polyline, a 90° bend, a T and a four-way crossing,
  a divided avenue, an 8 m residential street, a `service` drive and a bridge (both
  empty). Under each one, in the caption, what it is there to show. It may not roll
  its own geometry: `cars_mesh` is the one door out of `map/cars/` and the cells are
  described with the very `osm::fixture` the parse tests use, so a cell and a test
  talk about the same object. Its own is only the asphalt underneath, drawn with
  `MeshBuilder::push_ribbon` in the game's `ROAD_COLOR` — the brightness step between
  a body and the surface is half of how the row reads. `CAR_GALLERY_SHOT=path.png`
  takes one frame and exits, the way the roof gallery does and for the same reason.
  It has already earned its keep once: the divided-avenue cell was built with the two
  carriageways swapped (left-hand traffic), and the picture said so at a glance.
  Gallery infra facts: `panel.rs` and `params.rs` repeat identically across galleries
  — intentionally, so examples read top-to-bottom as self-contained units; the
  auto-shot logic is shared in `examples/demos/gallery_shot.rs` (frame counts,
  render-to-texture capture — a window surface shoots black on a locked screen), and
  fixes apply there to all galleries at once. Every gallery's `<VAR>` also reads
  `<VAR>_SCALE`: the image is the window's logical size × the scale, with the same
  `scale_factor` on the `ImageRenderTarget` — and the shot is spawned as
  `Screenshot(RenderTarget::Image(<the camera's target>))`: `Screenshot::image(handle)`
  names the target with scale 1 while `prepare_screenshots` swaps the output
  attachment keyed by the **whole** target, so the camera drew past the swap.
- **The ninth cell is the stand** (`car_gallery/stand.rs`) — five body types × three
  detail steps, and it answers the other question: not *where* a row stands but *what*
  stands in it. Neither is readable off a street — the type falls out of the LCG and a
  van may simply not turn up — so this is the one place in the gallery where the type
  is **ordered** rather than rolled, exactly as the roof gallery orders a material
  (`RoofLook::new`) because the seed cannot reach every combination. The geometry is
  still the game's (`body::push_body` / `push_shadow`) and still in metres; only the
  transform magnifies it (`stand::SCALE`). The `Detail` knob drives the street cells,
  never the stand — the stand shows all three steps at once.

## Filling the lots

- **The lots are filled by the same pass** (`fill_lots`): every stall from
  `ParkingLayout`, a share of them taken **that falls with the lot's size**
  (`lot_occupancy`): `LOT_OCCUPANCY_SMALL` 50 % up to `LOT_SMALL_STALLS` 20 stalls,
  `LOT_OCCUPANCY_LARGE` 12 % from `LOT_LARGE_STALLS` 400, by the log of the stall
  count in between. It was a flat 55 %, and on a mall lot of hundreds of stalls that
  solid field of cars read as a dealership. A yard is still fuller than a kerb, and an
  empty one next to a painted grid reads as unfinished. Seeded per lot (`lot_seed`,
  its first point) exactly like a street. That curve is built from constants, not
  from `CarStyle::occupancy`: the slider is about the ragged kerb row, and the
  half-empty lot is a different observation.
- **Yard rows** (`cars/yard.rs`, `park_yards`, called by `mesh_cars` between the kerb
  row and the lots) — the cars along `highway=service` drives, which `parkable` keeps
  out of the kerb row and OSM never maps. Without them a courtyard of nine-storey
  slabs was an empty grey loop on a lawn. Generated, deterministic, seeded from the
  drive's first point like a street:
  - **which drives** (`is_yard_drive`): `Highway::Service`, not a `parking_aisle`
    (the lot has its stalls), not a bridge, an arch or a ring, and at least
    `MIN_DRIVE_LENGTH` 20 m — a shorter one is a driveway to an entrance or a
    garage;
  - **one side**, the right of the points: a 4 m drive with cars on both sides is not
    passable, and which side does not matter as long as it is the same every build;
  - **half on the lawn**: no kerb, so the body stands with `ON_ASPHALT` 0.6 m of its
    width on the asphalt edge — offset `half + width/2 − 0.6` — and faces along the
    way;
  - **only among flats**: the share is the district's storeys (`Districts::storeys_at`)
    ramped from 0 at `LOW_STOREYS` 2 to 1 at `HIGH_STOREYS` 5, × `YARD_SHARE` 0.8 ×
    `CarStyle::occupancy`; nothing where no house stands within 120 m. A private-house
    quarter parks behind its fences, not along the lane;
  - **every body probed** (`Blocked::fits` — centre and four corners inflated by
    `CLEARANCE` 0.4 m): not inside a building, a lot, water or a pitch, not within
    the drawn half-width (+ sidewalk band) of any other road. A yard drive runs 2–4 m
    from a facade and along footpaths, and without the probe the row lay in the
    houses. Drives meeting this one end-to-end (`RoadNodes::roads_at` on its ends,
    `service` only) are not "other": OSM splits one drive into several ways, and the
    continuation's ribbon would otherwise wipe the row out for a car length each side
    of the seam;
  - breaks at junctions exactly as the kerb row does (`RowBreaks::of`, the one
    `cars::clear_of_break` test both rows call); the probe index (`Blocked`, a 40 m
    `Grid` of areas and road links) is built once per car rebuild;
  - **the occupancy roll comes before the probe**: the probe is the dearest step, and
    a slot that stays empty — every slot of a private-sector lane, where the share is
    zero — does not need it. Probing first cost the rebuild 3–6× more.

  Pinned by `a_yard_drive_among_towers_gets_a_row_on_its_right`,
  `a_yard_row_never_stands_in_a_house`, `a_yard_row_keeps_off_a_footway_beside_it`,
  `no_yard_row_in_the_private_sector_or_on_a_lot`. The roads gallery draws the car
  layer on request (`ROADS_CARS=1`, `map::mesh_map_cars`) — that is how the yards are
  checked by eye; `ROADS_CARS=full` (or a number) sets the row's occupancy, so a rule
  that forbids a place shows on the shot rather than on a roll of the dice.
- **How densely a place parks at all is decided by the district** (`cars/district.rs`,
  `Districts`), and it multiplies **both** shares — the kerb row's
  `CarStyle::occupancy` and the lot's `lot_occupancy`. Until it existed the layer
  knew only the width of the street and the size of the lot, and on the photo that is
  wrong twice over: a quarter of private houses parks in its own yards and shows
  singles on the street, while a microdistrict is solid because there is nowhere
  else.
  - **The reading is the area-weighted mean height** of the buildings within `REACH`
    120 m, in storeys, and the weighting is the load-bearing half. By count, twenty
    garages beside a nine-storey slab outvote it, though the cars belong to the slab;
    by area, a hundred small houses read the same as ten, which is right — a quarter
    does not get taller for being denser. A median reads worse here than a mean: the
    boundary between quarters has to be soft, and the weighted mean ramps — one
    section on the edge of a private sector already lifts it a little, a row of them
    lifts it all the way.
  - **The factor** is `LOW_FILL` 0.25 at `LOW_STOREYS` 2 and under, `HIGH_FILL` 1.15
    from `HIGH_STOREYS` 5, linear between, and exactly **1** where nothing is in
    reach. Low is "far thinner"; high is deliberately only +15 %, because past that
    the row closes into a solid band and reads as the very dealership
    `lot_occupancy` exists to avoid.
  - **Every contour of `MapData::buildings` counts**, churches and the kremlin
    included: the measure is "how high is what stands here", which is what the eye
    reads off the photo, and a kremlin tower really does make its surroundings not a
    private sector. Each exclusion would need its own argument and is worth fractions
    of a metre on Tula.
  - **The height is `buildings::height_or_default`** — the same inference the
    building is *drawn* with, never the raw `PolyArea::height`: a second notion of
    "how tall is this" would put a drawn nine-storey block in a quarter the cars
    treat as private sector.
  - **The index is a grid** of building centroids, `CELL` = `REACH`, each registered
    in every cell its radius touches, so a query reads one cell — the wagons' `Fan`
    construction. Built **per placement** (`park_all`), i.e. once per world load and
    per occupancy or road-shape edit, not per zoom crossing — it is not kept after
    the placement.
  - **Along a street the reading is refreshed every `DISTRICT_STEP` 48 m**, not per
    place: a query per each of 22 k places would cost more than the whole layer, and
    a quarter does not change from car to car. Forty-eight metres is a couple of
    private plots or the end of a section — the scale at which the city does change —
    and it is *arclength* of the street, so a way running out of the private sector
    into a microdistrict changes density where the city changes rather than where the
    way ends.
  - **It is RNG-stream-neutral.** Both call sites already rolled one `next_f32` per
    place and compared it against a share; the factor only scales the share, so the
    draws and their order are untouched and nothing else in the layer moves. The
    product is clamped to `[0, 1]`, so `occupancy = 1` still means "every place
    taken" on the slider.
  - **An empty `Districts` is a factor of 1 everywhere**, which is not a stub but the
    honest "no quarter to read": that is what `car_gallery` (no buildings at all) and
    the row-layout tests in `cars/mod.rs` build with, so they check the laying-out
    rule unmixed with the density rule. The density rule has its own tests in
    `district.rs` plus one end-to-end in `cars/mod.rs`
    (`the_same_street_parks_thinner_in_a_private_sector`).
