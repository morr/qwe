# Other layers — water, yard, pitches, rails, tram, industry, wagons, fences

The layers that are neither roads, parking, buildings nor trees. Each bullet is a
`SKILL.md` Rendering bullet as it stood, moved here so the skill itself stays a map.

## Water

- **Rims** (`map/spawn.rs::push_area` over `MeshBuilder::push_inset_band`) — each area
  polygon is followed, in the same builder, by a gradient band along its outer ring and
  along every hole: `edge` colour on the contour, the fill colour at the far edge. Park /
  wood / grass / sand get an edge a few
  percent darker than the fill (`*_RIM`, 2–3 m; the wood's the widest and darkest — shade
  under the canopy edge). The far edge is built from `miter_offsets` on the ring, with the
  side chosen by the ring's signed area (`outside` flips it for holes, whose band lies in
  the fill). Two guards: the band width is clamped to `RIM_THICKNESS_SHARE` (0.6) of the
  ring's thickness `|area| / perimeter` — a strip's thickness is half its width, so a
  2 m rim on a 1.5 m median never pokes out onto the road — and nothing under
  `MIN_RIM_WIDTH` (0.2 m) is pushed at all. Holes take the width the outer ring settled
  on. No z-slot: opaque 2D meshes test depth with `GreaterEqual`, so within one mesh the
  band pushed after the fill wins. **Water is not rimmed** — see **Shoal**.
- **Shoal** (`map/water.rs::mesh_water_areas`, the `water` layer at `Z_POND`) — the
  light shallows of area water, as a **distance field to the nearest bank**: the colour at
  a point depends only on how far the nearest bank is, `WATER_SHORE_COLOR` on it and
  `WATER_COLOR` at `WATER_SHORE_WIDTH` (6 m — at 3 m it read as the polygon's edging rather
  than as a shoal). It replaced the rim above (`WATER_RIM`), which laid a light line
  wherever two water multipolygons share a border across open water, and whose thickness
  clamp darkened a narrow arm toward its middle faster than a real channel shallows.
  - **Union first** (`i_overlay`, NonZero, outer rings oriented CCW and holes CW — OSM's
    order is arbitrary and NonZero only merges consistent windings): the shared border is
    gone before anything is laid. **The bands of the water gaps go into that same union**
    (`split_channels`, under **Waterways** below) — two polygons of one river that do
    *not* touch, because OSM cut the `riverbank` at a bridge, are joined by the channel
    between them, and their service edges across the river stop being banks the same way.
    **The union is an intersection with the map** (`within`, inflated by
    `WATER_CLIP_MARGIN` 50 m — wider than the shoal, so the shoal along the cut lies off
    the map): Overpass hands a lake's multipolygon over whole, and meshing every shoal
    band of a 38 km lake mostly outside the map hung `spawn_map` for minutes; clipped,
    30 ms. Never drop the clip.
  - **Then nested inward offsets** (`OutlineOffset::outline`, negative offset, `Round`
    joins, `SHOAL_STEP` 0.5 m, twelve levels), each always taken from level 0 rather than
    from the previous one, so errors do not accumulate. The band between depth `d` and
    `d + step` is one earcut polygon whose **outer ring carries colour(d) and holes
    colour(d + step)** (`MeshBuilder::push_polygon_graded`), so the GPU interpolates the
    gradient across the band. A narrow place's offset vanishes on its own, and the
    innermost level left is filled flat with its own depth's colour — the middle of an
    8 m arm is 4 m deep, never full depth.
  - **The band is assembled from rings, not by boolean difference**: a difference would
    lose which vertex came from which level, and the level *is* the colour. An inner shape
    lies inside exactly one outer shape (not in its holes — a lake on an island is someone
    else's); its outer ring is a hole of the band, and each of its holes surrounds an
    island, making a separate band piece with that island as its hole.
  - A triangle whose three vertices sit on one ring is flat-coloured; on a 0.5 m band that
    error is under one step.
  - Cost: the `water` field of `SurfaceReport`, printed inside the one
    `surface meshing:` line.
- **Waterways** (`map/water.rs::mesh_water_lines`, the `waterways` layer at `Z_WATERWAY` 2.02,
  `SurfaceKind::Water`) — the open channels, two decisions deep:
  - **Water lies over every road ribbon and under the bridge shadow — both layers**:
    area water at `Z_POND` 2.01, the channels a hair above it — **CONTEXT.md's z-layers
    invariant** owns the rule, its reason and its price (a road lies over water only as
    a bridge; a narrow arm is mapped as a ribbon or as a polygon; the navmesh blocks
    both, and only a bridge carves). Guardrail: stray blue over the roads can be the
    polygon, not the ribbon — lifting the `waterways` layer alone proves nothing.
  - **The mouth.** The channel ribbon has its own **shore** — the same distance field as
    the **Shoal** (`WATER_SHORE_COLOR`, `WATER_SHORE_WIDTH` 6 m, unclamped, so a 2.5 m
    stream is pale across its whole width like a narrow arm) — laid in
    `surface.wgsl` by the ribbon's `across`, since a ribbon has no ring to offset. That is
    not enough on its own: an OSM channel typically runs on **inside** the area water it
    flows into (a river's own centreline lies inside its `riverbank`), and there its light edges would be two shoal
    lines across deep water, while its deep middle cut the shoal with
    a rectangle. So `WaterIndex::open_runs` cuts the smoothed
    axis at every water outline (edges in a 32 m grid; inside/outside asked once per
    stretch between crossings, not per link) and keeps the dry stretches; each **cut end
    reaches `WATER_SHORE_WIDTH` past the bank** (along the axis, straight on past its
    end) with a `Butt` cap and a `Break` of that reach. *To-break* then runs 0 → −6 m
    over the reach, and the shader fades the channel shore by it — linearly, exactly as
    the shoal fades from the bank inwards, so at every depth the channel edge and
    the water beside it have one colour and the shoal turns into the channel without a seam.
    Two guards: water narrower than two reaches between two dry stretches does not cut
    (the reaches would overlap into a seam mid-channel), and an uncut ribbon still goes
    through `RibbonBreaks::At(&[])`, never `Ends` — `Ends` measures to the ribbon's own
    ends and would fade the shore at every channel end on dry land too.
  - **The gap.** A stretch cut at **both** ends is not a channel on land at all: it is a
    **discontinuity of the area water** — OSM cuts a `riverbank` at a bridge, and
    fifteen metres of nobody's land remain between the two polygons, the centreline
    running through. The fade above is wrong for it in both halves: the ribbon's reach lays a rectangle of **deep** water over
    the pale shoal of a narrow polygon (the reach fades to `WATER_COLOR`,
    which agrees with the water beside it only where that water is deep), and each polygon
    lays **its own shoal along the service edge** it is closed with across the river — a
    light band where there is no bank, the very artifact the union fixed for polygons that
    *touch*. So `split_channels` hands such a stretch over: its band (the axis spread by
    `miter_offsets` to the channel width, reaches included, so the overlap with both
    polygons is real and the union cannot leave a hairline) goes into
    `mesh_water_areas`'s union, and the ribbon does not draw it. The service edges are
    then interior, the shoal runs along the real banks and wraps into the channel, and the
    river visibly narrows to the channel's width under
    the deck — honest, and mostly hidden by the deck. The two doors share one cut for that reason: «где кончается
    полигон» must have one answer, not two. The count prints in the `surface meshing:`
    line as `N gaps`. An **island** in a river answers the same way, and rightly — there
    is water on both sides of it.
  - **Render-only.** The navmesh blocks the polygon and the whole channel band as before;
    the caps rule it shares with the drawing (`water_line_caps`) is untouched.
  - Cost: one pass per open channel at load — the `waterways` field of `SurfaceReport`,
    printed inside the one `surface meshing:` line.

## Ground

- **The yard** — what stops the city from being a beige sheet with buildings on it.
  - **The residential block is the yard.** The `landuse` layer splits in two
    (`landuse_yards` with `SurfaceKind::Yard`, `landuse_works` with `Ground`) — one kind
    for both would have put grass speckle on a concrete yard. **`Yard` is its own kind** rather than
    `Grass`: the mottle is 0.105 against 0.06 of amplitude on a 22 m wavelength against
    30 (and the speckle is sparser and higher-threshold), which is
    exactly the difference between a meadow and ground people walk over — bare patches by
    the doors, grass in the corners. With plain `Grass` the block came out as a golf
    course. Nothing else changed: the roads, the
    parking, the buildings and the pitches are all drawn *over* the block, so the green
    only shows where nothing else is, which is exactly where the grass is.
  - **Worn paths were tried here and removed.** A straight 1.1 m strip of bare earth from
    every OSM entrance to the nearest kerb — the desire lines — looked bad on the city
    and was taken out by the author's call; the commits describing it (`map/paths.rs`,
    `Z_WORN_PATH`) are history, not a missing file.
    Don't reintroduce a straight door-to-road strip. The uniform-grid index stays — it is
    the door generator's own (`osm/entrances/index.rs`), extracted while this layer
    existed and its only surviving trace; it has since become `map/grid.rs::Grid` (above).
- **Pitches** (`map/pitch.rs`) — sports and children's grounds, the thing a courtyard is
  actually *made of* on an aerial photo. One surface layer at `Z_PITCH` 2.005 and one
  markings layer at 2.006 — a whole 0.001 apart, since a z-step under that is not worth
  trusting to the depth buffer — the parking pair's shape exactly: the paint is flat
  `ColorMaterial`, the surface carries `SurfaceKind::Ground` (a neutral mottle — a
  football field must not get the street's asphalt grain).
  - **Over every road ribbon, under water.** OSM routinely runs yard footways straight
    across a field, cutting it into pieces on the map where on a photo the field is
    whole and the path stops at its edge. Render-only: a pitch
    blocks nothing on the navmesh, so pawns still walk the path across it. Parking lies
    over the roads too, one hair lower (see **Parking**), so where a lot and a pitch
    overlap the pitch wins.
  - **The kind is decided in three steps** (`parse/tags.rs::pitch_kind`), because `sport`
    is often missing on a pitch: `leisure` first (`track` → `Track`,
    `playground` → `Playground`, `sports_centre`/`stadium` → `Ground`), then `sport` on a
    `pitch` (soccer/rugby/athletics/… → `Soccer`, everything else → `Hard`), then
    `surface` (`grass`/`dirt` → `Soccer`, `sand` → `Playground`, else `Hard`). The last
    fallback is `Hard` on purpose: a nameless yard pitch in a Russian city is an asphalt
    box far more often than a lawn.
  - **Colour by kind**, not by tag: the football green is more saturated than a lawn's,
    the hard court is a blue-grey darker than the street, the track is tartan orange, the
    playground sandy. One rim (`PITCH_RIM`) for all of them — from above that edge is the
    shadow of a kerb or a board, not paint.
  - **Only `Soccer` and `Hard` are marked.** A playground has no markings; a track's run
    along its oval and would have to follow the outline rather than the frame; a sports
    centre contains the already-marked pitches. What is drawn: the perimeter, the centre
    line, the centre circle (`CIRCLE_SHARE` 0.087 of the long side — 9.15 m on a 105 m
    field, and the same fraction happens to work for a basketball box), and on a field
    over `PENALTY_MIN_LENGTH` 45 m the two penalty areas.
  - **The frame gate.** Markings are laid out in the `min_area_rect` frame, and only if
    the outline fills it to `RECT_FILL_MIN` 0.85 (the gable roof's number, same meaning:
    "this is a rectangle, not a blob") and the footprint is over `MIN_AREA` 150 m². On an
    L-shaped patch the centre line would otherwise run across the lawn beside it.
  - The circle is a 32-gon of quads (`ring`), 1.8 m a side at a 9 m radius — a fraction of
    a pixel at any zoom where it is visible at all. There is no arc primitive in
    `MeshBuilder` and this is the only caller that wants one.

## Rails and tram

- **Rail layers** (`map/rail.rs`, its own module with its own zoom LOD, like the tram's) —
  the **track**, not a map
  symbol: a ballast prism, ties across it and two steel rails on the gauge. Three merged
  meshes for a ground track (plus the level-crossing deck between ties and steel, and three
  more for a track on a bridge — both below), `RailLayerTag`, all above `Z_ROAD` (2) so a track lies on its street at a level
  crossing, and the ground ones all **below `Z_BRIDGE_SHADOW` (2.05)** so a road bridge over the tracks
  covers them (the tram, which must run over a street deck, has its own layer above it):
  ballast `Z_RAIL` (2.03), ties `Z_RAIL_TIE` (2.035), steel `Z_RAIL_STEEL` (2.04)
  — the far-bucket
  dash rides in the tie mesh, it never coexists with ties. Three rather
  than one, for the casing reason inverted — coplanar geometry z-fights, and a tie must
  sit above *every* ballast, or a junction of several ways delaminates. Inside the
  ballast mesh the same rule is push order: **all** shoulders first, then all beds.
  - **The prism** — the OSM width (rail 5 m, light_rail/narrow_gauge/subway 4, disused
    3.5) is the bed; the tie length and the far-bucket dash width scale off it, so a
    disused track is drawn narrower — but the **gauge is absolute** (1.5 m on every
    bed: light_rail, subway and disused track are physically the same 1520 mm), because
    a proportional share drops the two rails under the ~4 px floor at which they still
    read as two on the smaller beds; the shoulder
    under it is `SHOULDER_SCALE` (1.22) of that, darker. It is
    the slope that separates the track from the ground it runs on; without it the track
    is a flat ribbon again.
  - **`RAIL_LODS`** — five buckets over the camera zoom range, and they change the
    *drawing*, not its size: close up the real thing (ties 2.6 × 0.26 m every 65 cm,
    a 1.5 m gauge on every bed with 12 cm rails); by 0.26 m/px the two
    rails no longer separate on screen and are dropped, ties thicken and thin out into
    hatching; from 0.65 m/px the ties go too and a coarse dash pattern comes
    back, because a bare grey band reads as another street. That dash is **darker than
    the ballast** — a white dash reads as a map symbol, not a track — and the ties are
    already gone on these buckets, so from the air a track is a grey band with a dark
    dash running **along** its middle. Its colour is exactly `RailPalette::tie` of its
    own palette — the dash *is* the ties that stopped being drawn, so the mark does not
    brighten in a step at the threshold. Since the tone is all the sign has, the dash
    width stays put and the contrast is bought with the colour; the test pins a **floor
    on the contrast**: WCAG `(L + 0.05)` ratio of ballast to dash over bevy's linear
    `luminance()`, at least `MIN_DASH_CONTRAST` 1.75, and **every** palette must clear it
    against its own ballast (a plain luminance difference is blind here).
    The pin runs both ways: `MAX_ZOOM` cannot be raised without re-tuning this rung's
    `min_bed` and `width_scale` and the last rung of `TRAM_LODS` with them.
    `min_bed` floors the ballast width on the last two buckets — 5 m is a pixel at city
    scale, and the track would vanish before the roads it crosses. The numbers are
    derived from the screen size at the **worst** (far) edge of each bucket, and they
    hold on **every** parsed bed width (5 / 4 / 3.5 m), not only the mainline's: tie
    spacing never below ~6 px, no mark below ~1 px. The second number that must hold
    across buckets is the **tie duty cycle**, ~40% (the real 0.26 m in 0.65) — measured
    live: at 31% the ties stop being a texture, become sparse marks, and the two white
    rails outweigh them into a ladder. Both, plus the one-way progression (detail only
    ever falls away) and that ties and dashes never coexist, are pinned by
    `rail/tests.rs`. That the dash *is* the tie colour is not a test but a construction:
    both fields of a palette are one `*_TIE` constant.
  - **What a bucket costs**: meshing cost rises by an order of magnitude from the
    coarsest bucket to the deepest (measured on the `rail meshing:` log line) — a one-off
    hitch on the threshold crossing, and the cost a denser tie step would multiply, so
    treat bucket 0's 65 cm as the floor.
  - **`RailZoomBucket`** is `ZoomBucket<RailLods>` — the same machinery as the tram's
    (see **Zoom buckets** in `SKILL.md`), a separate resource because the tables' thresholds
    have nothing in common: `RailLods` is the marker that hands `RAIL_LODS`'s
    `max_zoom`s to `map/zoom.rs`.
  - **`MeshBuilder::push_rails`** — two ribbons offset from the
    centerline by half the gauge, using the very `miter_offsets` that build a ribbon's
    edge, so the rails hold the gauge through a bend instead of drifting outward at the
    corner.
  - **No style resource.** Like the tram (whose only resource is a visibility toggle),
    rails ignore `RoadStyle` and hardwire
    `Round` + `Light` with a fixed `RAIL_SMOOTH_WIDTH` (5 m), so the centerline is
    identical on every bucket — a smoothing knob would slide the track against its own
    ballast, and an LOD switch would wiggle it.
  - **`RailKind` is the palette**: `Active` is ballast grey-brown, creosote ties, bright
    steel; `Disused` is the same track overgrown — weedy ballast, grey ties, rust.
    `Tram` is skipped here, it has its own module.
  - **Track bridges** — `RailLine::bridge` (`bridge=*` except `no`, read in
    `parse_way`). Such a track is drawn in three layers of its own, `rail_bridge_ballast` /
    `_ties` / `_steel` at `Z_RAIL_BRIDGE` 2.3 / 2.305 / 2.31 — above the street bridges'
    deck (`Z_BRIDGE` 2.2) and paint, below the tram — and **without a shoulder** (the
    ballast lies in the bridge's trough), its bed capped at `rail::deck_width` so a far
    bucket's `min_bed` does not hang it over the parapet. **The bridge ballast ends with
    the trough** (`RibbonCap::Butt` in `push_tracks`): a ground track's ballast keeps the
    round cap — a track dying mid-city ends rounded, not sheared — but a round cap on the
    bridge layers buries the ground continuation's ties and steel for ~2.4 m past every
    seam. The slab and
    parapet edges of a *neighbouring* deck stay visible where they overhang the
    neighbour's ramp past the seam — that is the decks' real outline, which the cap
    painted over. The slab, parapet and shadow
    under it are the road layer's (`roads.md`, **Bridge layers**). A bridge track never
    forms a **level crossing**, is never a **rail gap** end (`parse/rails.rs::free_ends`),
    and holds no **standing wagons** (they would lie under the slab at `Z_WAGON`). It does
    keep its **car keepout** (`cars/rails.rs`): a car at `Z_CAR` would be drawn on top of
    the slab, so the row under a track bridge is cleared the way a street bridge clears
    the row beneath it.
  - **Level crossings** (`rail::mesh_level_crossings`, layer `rail_crossings` at
    `Z_RAIL_CROSSING` 2.0375 — over the ties it hides, under the steel that runs over it).
    Where the axis of a ground track crosses the axis of a road that can be crossed at
    grade (`rail::crossable`: `RoadClass::Street` — streets *and* service drives — not a
    bridge, not a passage — the same predicate the **rail gap** stitch asks, so a spur
    broken at a yard drive is stitched across it where the deck would lie), the ballast and ties inside the road and its sidewalks are
    covered by a light concrete deck (`CROSSING_COLOR`) and the rails run on over it, the
    way a tram's run over asphalt; past the kerb the track is ballast and ties again.
    **Found by geometry, not by tag**: `railway=level_crossing` is not in the query.
    The deck is the
    **parallelogram** where the two straight bands of a link pair overlap — across the
    track the ballast's foot (`deck_width`), across the road the drawn carriageway plus
    each side's drawn sidewalk (`Drawn::sidewalk_on`) — so its ends follow the kerbs and an
    oblique crossing leaves no ballast triangle on the asphalt. Below `CROSSING_MIN_SIN`
    0.34 (~20°) the track runs *along* the road and gets no deck. Track links go into a
    `Grid` (`CROSSING_CELL` 32 m) and the road links query it: one pass over the roads.
    **The layer is the road layer's, not the rail's** (`mesh_roads` pushes it, tagged
    `RoadLayerTag`, counted in `RoadReport::level_crossings`), because the road has to be
    taken **as drawn** — the deck rides the ribbon axis (`Axis::Ribbon`), not the OSM
    centerline, which lies up to the curve tolerance (3 m) off the asphalt. The rail layer has no `RoadShape` and should not grow one; the road layer has it
    and already rebuilds on it. The price: the deck stays on every zoom bucket, over the
    dash sign too (on the last bucket the 9 m `min_bed` ballast sticks out of the 6.1 m
    deck by ~1.5 m a side, about a pixel there).
- **Tram** (`map/tram.rs`, its own module so a zoom-LOD step never rebuilds the
  road/rail meshes) — a thin blue line with perpendicular cross ties, the
  Yandex/2GIS convention; `TRAM_COLOR` is the only thing separating the two (Yandex dark
  red, 2GIS blue) and we take 2GIS's blue, since red on this map already means kremlin
  wall. Line and ties share one colour, so both go in one mesh (`TramLayerTag`, `Z_TRAM`
  2.6 — above the rail steel at crossings, name `tram`) — self-overlap costs nothing,
  and there is no dash layer for a tram. The tie primitive is
  `MeshBuilder::push_ticks`: the same arclength walk as `push_dashes`, but each mark is
  a perpendicular bar rather than a piece of the path, and the first one is offset half
  a step so a bar never lands exactly on a way endpoint and pairs into a cross at joins.
  The style is fixed, and the panel has exactly one row: on a line 1.5–2 px wide a join
  style is invisible and Strong smoothing is indistinguishable from Light, so it is
  hardwired to `Round` + `Light` (`TRAM_JOIN` / `TRAM_SMOOTHING`), and the sparse tie
  spacing is baked into the LOD table.

  **`TramStyle`** (resource, one field `visible`, **off** by default — the blue line lies on
  the carriageway and at city zoom reads as another street layer — BRP-writable, persisted,
  settings group `tram`) is therefore the whole style surface: the `Tram` row at the bottom
  of the **Roads** section (`ui/roads.rs`) — the track runs on the carriageway, so it is
  read together with the roads rather than given a section of its own for one row. It is
  **not** a field of `RoadStyle`, and that is the point: a `RoadStyle` edit reruns
  `rebuild_roads`, and hiding the tram would then remesh every road layer for nothing.
  `rebuild_tram` is gated on `retuned::<TramZoomBucket>.or_else(retuned::<TramStyle>)` and
  the invisible case goes through it like any other — despawn the old layer, build no new
  one — so there is no second path that could forget the despawn.

  **Tram zoom LOD** (`TRAM_LODS`) — the mesh is rebuilt at discrete zoom thresholds:
  five buckets over the camera zoom range, each with its own line
  width (targeting ~1.8 screen px, so the line neither fattens close up nor vanishes far
  out) and tie length/thickness/spacing (on-screen tie spacing never drops below ~10 px);
  the farthest bucket drops ties entirely, as 2GIS does at city scale. `TramZoomBucket`
  is `ZoomBucket<TramLods>` (see **Zoom buckets** in `SKILL.md`), so `rebuild_tram` fires only
  on an actual threshold crossing, never per frame. The tram centerline is smoothed with a
  fixed `TRAM_SMOOTH_WIDTH` (1.2 m) clamp rather than the bucket's line width, so the
  path itself is identical across buckets and LOD switches don't wiggle the track.
  `RailLine::width` from parse is ignored for trams.
  **The tracks are the laid ones, not `MapData::rails`**: `rebuild_tram` reads the
  `TramTracks` resource the road build writes — on a tram bed symmetric about the drawn
  middle, along a single street moved with its drawn axis (`roads.md`, **Laid tracks**) —
  and rebuilds on `resource_changed::<TramTracks>` as well as on the bucket and the toggle.
  `mesh_tram` itself still takes `&[RailLine]` (the bench measures the OSM ones), and the
  roads gallery draws the layer on `ROADS_TRAM=1`.

## Industry, standing stock, fences

- **Industry** (`map/industry.rs`) — the industrial belt, added in `QUERY_VERSION` **11**.
  Five layers from two sources (`Structure` and `PipeLine`, see **MapData — the parsed
  model** in `SKILL.md`), rebuilt on
  `industry::rebuilds_on()` — `retuned::<SunOnMap>.or_else(retuned::<BuildingHeightMode>)
  .or_else(retuned::<IndustryStyle>)`
  and on nothing else — the settled sun, never `SunStyle`, like every other rebuild — and
  the system stands on its own rather than in the zoom-bucket chain, because there is no
  zoom bucket here: a cylinder is visible exactly as far as its shadow is.
  **One registration carrying all three conditions, never three registrations** — this
  is the layer the rule is written from; the rule itself lives once, on `roads::rebuilds_on`
  and under **When a layer rebuilds** above.
  - **`IndustryStyle::visible` is the whole style surface, and it is off by default** —
    the `Industry` row of the **Buildings** section (`ui/buildings.rs`), the tram's
    arrangement exactly, and for the tram's reason: its own resource rather than a
    `BuildingHeightMode` case, so a toggle rebuilds this layer instead of remeshing every
    building layer. Off, because a city carries a dozen cylinders standing on its edges
    while a chimney's shadow runs fifty metres — at the city zoom that is a dark streak
    from nothing visible. The invisible case goes through the same rebuild — despawn the
    old layer, build no new one — so there is no second path that could forget the
    despawn. It is read with the buildings and not with the roads because a cylinder
    stands on the ground and leans by the very `drawn_lift` a house does.
  - **"Like a house" is literal, and shared in code**: the cylinder's shadow is drawn in
    exactly the three modes a house's is (`BuildingHeightMode::casts_shadows()` — the
    mode list lives there once and both layers ask it; in `Facade` and `Extrusion` a
    2.5 m chimney is a small circle and nothing else), and its lean comes from
    `buildings::drawn_lift`, the height-only core of `extrusion_lift`, so the
    `EXTRUDE_RANGE` clamp is one for roofs and cylinders alike. Without that clamp a
    60 m chimney lay across the map as an 80 m tube.
  - **A cylinder is three layers, like a house**: `industry_shadows`
    (`Z_INDUSTRY_SHADOW` 4.55, beside the building shadow), `industry_walls` (5.06) and
    `industry_tops` (5.07) — **above** the houses, because a works chimney is taller than
    anything around it and on a photo it covers the neighbouring shed, not the other way
    round. **One rung for all five kinds**, from a 12 m tank to a 60 m chimney, so a low
    tank covers a tall block too — accepted deliberately: the buildings have no height
    sort of their own either (one `Z_BUILDING` for the whole layer, order inside the mesh
    by `Lean::depth`), so a height threshold would split the layer into two meshes and pop
    at the threshold without solving the general case. That one is the joint painter's
    sort of buildings and cylinders — separate work, like the cylinder-to-cylinder sort
    below.
  - **The shadow is a sweep, not a shifted disc** (`sweep` — the convex hull of the disc
    and its copy, written out by hand as two half-arcs; the buildings get theirs from
    `i_overlay`). A cylinder is solid from the ground to the top, so every height in
    between casts too; a shifted disc would leave the strip between base and shadow
    empty, which on a 60 m chimney is 36 m of missing shadow. Its length is the
    buildings' `SHADOW_LENGTH_RANGE` **multiplied by `sun_stretch()` at both ends**, as
    every calibrated length here must be: with the bare 3–45 m a 60 m chimney stopped at
    45 m at 15° while a house of the same height threw 168.
  - **The wall is one quad per facet, each shaded on its own** (`shade_by_light`, mixes
    0.26/0.26 — stronger than a flat house wall's 0.18/0.22, since the gradient has to
    span the whole visible half). That gradient *is* what makes the circle read as a
    cylinder; a single flat tone reads as a faceted prism. **Only the half turned
    *away* from the `Lean` is emitted** (`outward · lift < 0`) — the near one. The
    camera sits at the nadir and the top leans away from it, so what it sees is the
    near side of the wall, exactly as the building extrusion picks its edges
    (`silhouette_edges(outer, -lift_dir)`); drawing the far half instead leaves the
    near end of the silhouette open, and the cylinder's own base shadow shows through
    the hole. The geometry in one line: the
    silhouette of a leaning cylinder is a stadium — the top circle covers
    `[|lift|−r, |lift|+r]`, the near half of the wall covers `[−r, |lift|]`, and the
    two overlap into the whole stadium, with no foot piece and no seam. With no lean at
    all (the flat height modes) nothing is emitted — a cylinder standing straight up
    shows no wall.
  - **The rim** (`RIM_SHARE` 10 % of the radius, 0.25–1 m, 28 % toward black) is the
    tank's coaming or the chimney's wall thickness. Without it the top reads as a sticker.
  - **The pipeline is a line one storey up**: line plus shadow, `PIPE_HEIGHT` 3 m,
    `Z_PIPE_SHADOW`/`Z_PIPE` 2.76/2.77 — over everything standing on the ground (the
    parked cars at 2.7), because a heating main on trestles steps over it, and under
    everything taller than those trestles. **All the shadows first, then all the lines**,
    the parked cars' rule: otherwise one main's shadow lands on the main drawn before it.
  - Both are drawn as **one merged layer each and no painter's sort between structures**:
    a taller cylinder's wall can therefore be covered by a shorter neighbour's top. With
    ten of them per city they never meet; a city where they do wants the buildings' sort.
- **Standing wagons** (`map/wagons.rs`) — the same generator as the cars, aimed at the one
  place that stayed empty: a station throat. On a photo half of it is standing stock, and
  without that the yard reads as a track diagram.
  - **Only service track carries them** (`RailLine::service: Option<ServiceTrack>`,
    `Siding | Yard | Spur`, parsed from `service=siding|yard|spur` by `service_track`).
    Stock on the running line is either moving or absent.
    `crossover` is deliberately out of the whitelist — it links two running lines.
    **`RailKind::Active` on top of that**: a `Disused` track is taken up and a `Tram` one
    belongs to its own module, so neither holds stock (`a_disused_track_stands_empty`).
  - **But service track is not a station, and density follows the place.** One rate on
    every service track stands wagons far too densely — many where the trains stand, at
    stations, almost none on an ordinary track — and the tag cannot say it: a spur to
    a plant, a lone dead end and a park of sidings carry the same `service`. What says it is **the fan**: a station
    is a bundle of parallel tracks metres apart, a spur runs alone. So every rake first asks
    `Fan::width_at` how many **other** `Active` tracks (running lines included — a passing
    loop beside a double-track main *is* a station) pass within `FAN_REACH` 12 m of its
    middle, and stands with the share `FAN_FILL[width.min(3)]` = 1.4 % / 7 % / 28 % / 52.5 %,
    times `class_fill` (`Spur` 0.5, the others 1). 12 m and not 9 because the spacing in a
    park is 5.3–6.5 m, and the edge track of a fan must see its *two* neighbours (5.3, 10.6).
    One neighbour does not make a station — it is a loop beside the main or two parallel
    plant spurs. A rake that does not stand leaves its own span empty, so the phase of the
    rakes along a track does not depend on which of them stood.
    **The index is a grid**, `FAN_CELL` 32 m, each segment registered in every cell its box
    inflated by `FAN_REACH` touches, so a query reads one cell; it is rebuilt with the layer,
    queried once per rake. Pinned by `a_lone_track_stands_almost_empty`,
    `a_spur_stands_thinner_than_a_siding`, `the_fan_counts_other_stock_tracks_within_reach`.
    **Rejected: `railway=station` / `landuse=railway`** — neither is in the query (a
    `QUERY_VERSION` bump), and a station node does not say where the station ends.
  - **Rakes, not rows**: `RAKE_MIN..=RAKE_MAX` (3–16) cars coupled at `COUPLED_GAP` 0.9 m,
    then `GAP_MIN..GAP_MAX` (12–90 m) of empty track, seeded from the track's first point
    through the shared `seed::seed_from_point`. An even row at a fixed pitch reads as a
    fence. The rake length is a **count**: `RAKE_MIN`/`RAKE_MAX` are `u32` and the roll is
    `range(MIN, MAX + 1)` with a `.min(RAKE_MAX)` of its own — as `f32` bounds the pair
    was half-open and the truncating cast ate the 16-car rake, and `range` itself rounds
    up to the outer bound on some generator states (see **One RNG and one point seed** in
    `SKILL.md`).
  - **Walked along the whole track's arclength**, the same `along::{arclengths,
    place_on_path}` the cars use (see below), with `TRACK_MIN` 40 m and `END_MARGIN` 12 m
    measured from the **ends of the track**, not of a link. The `points.windows(2)` walk
    is the very one the cars had already given up, and a yard is no exception: links
    shorter than `TRACK_MIN` are common on service track, the margin would have kept
    every interior bend clear where there is no switch, and the rake phase reset at
    every vertex on top of that. Curvature is
    handled the cars' way — a place closer than `WAGON_LENGTH` to the last wagon **placed**
    is skipped, measured in world distance, since a 13.9 m body has a rigid wheelbase.
    Pinned by `short_links_carry_the_same_rakes`.
  - **The shadow is the point**: a 3.8 m body against a 13.9 × 3.1 m footprint throws its
    shadow by the same `shadow_length_scale()` as the buildings — 3.8 × 0.6 ≈ 2.3 m under
    the default 59° sun, half a wagon's length once the sun drops to 30° and a whole one at
    the 15° minimum — and that is what makes a rake read as solid objects rather than paint.
  - Its own bucket (`WagonZoomBucket`, `WAGON_MAX_ZOOM` 2.0): a 13.9 m wagon is three
    times a 4.4 m car, and at 2.0 m/px it is the same ~7 screen pixels at which the cars
    are already dropped — 2.5× further out than `CAR_MAX_ZOOM` 0.8, so sharing
    `CarZoomBucket` would have hidden the yards early. `Z_WAGON` 2.045 — above the rail
    steel (a wagon stands *on* the rail),
    below the bridges, like the track under it.
  - **No style resource, unlike the cars.** The layer is decoration and still has no
    `visible`: it comes off by `WagonZoomBucket` — and by **raw OSM's drawing level**
    (`RawOsm::Draw`, `references/parse-seam.md`), since a rake is our placement, not data:
    `mesh_wagons` takes the `RawOsm` and reports the layer `hidden`, the cars' rule
    (`raw_osm_draw_stands_no_wagons`).
  - **No `QUERY_VERSION` bump**: `out geom` returns every tag of the element, so `service`
    has been sitting in every cache since v4.
- **Fences** (`map/fences.rs`) — `barrier=fence|wall|retaining_wall|hedge`, added in
  `QUERY_VERSION` **14**. What is drawn is a thin ribbon **and its shadow**, and the
  shadow is the point: from above a fence is a 25 cm hair, and on a photo it is the dark
  thread beside it that you actually see.
  - **`FenceLine` is a separate type, not `WallLine` with a flag.** The kremlin wall is
    impassable end to end; a fence blocks the navmesh **with gaps** — a road through it,
    a default gate — at its physical 0.3 m, not at the drawn width below (pure decoration
    would strand nobody, but the gaps and the default gates answer that argument; the
    measurement is in the navigation-deep skill). **The gaps are drawn as gaps**, by the
    author's call: `mesh_fences` cuts every fence with `footprint::fence_pieces` — the
    polyline minus the gap discs, the very discs the polygonal mesh subtracts — and lays
    both the shadow and the ribbon from the pieces, so no fence is drawn across a path a
    pawn walks. A leftover under `MIN_FENCE_PIECE` 0.5 m at a gap edge is not drawn. The
    gaps are recomputed on every rebuild (`fence_gaps`, 8.7 ms on Tula) rather than
    cached: the layer rebuilds on a zoom crossing or a settled sun, not per frame.
  - **And the stretch under a bridge is not drawn at all** (`footprint::BridgeDecks`,
    the third argument of `fence_pieces`), which is the one place the drawn fence and the
    blocking one part on purpose: `Z_FENCE` 2.75 lies above `Z_BRIDGE` 2.2, so a fence
    crossing under a span would be drawn over the deck. The cut is the same idiom by which the
    car layer keeps off a deck (`cars::BridgeDeck` — everything the z ladder puts above
    the bridges yields to them), but the shape differs: a car is dropped whole, a fence is
    cut at the edge the way a gap cuts it.
    - **What covers is the drawn ribbon, not a crossing of centrelines.** `fence_gaps`
      measures a road's opening as a disc at the crossing precisely so that a street
      *along* a fence does not erase it; a deck is the opposite case — it hides exactly
      what lies beneath, along or across. Hence a **capsule**: the polyline inflated by
      `RoadLine::curb_reach` (deck + curb, the whole drawn width of a bridge), with round
      ends.
    - **The shadow is cut by a boolean, not by the piece** — cutting only the pieces
      leaves the grey band running onto the deck. A shadow is an **area**: the sweep of
      the piece that ends at the curb flows out from under its round cap **sideways**, the
      whole length of the shadow (7.4 m at a 15° sun), and no interval on the fence's own
      axis takes it away. So `push_shadows` subtracts `BridgeDecks::outlines` — the same
      capsules as polygons — from the sweeps before the union (`i_overlay`, Difference /
      NonZero). The polygon is **circumscribed** about the cap circle (`radius / cos(π/2n)`,
      `CAP_SIDES` 8 per end): an inscribed one leaves a crescent of shadow at each end.
    - **The cut is wider than the deck by a reserve** (`BridgeDecks::build`'s second
      argument, `(width / 2).max(SHADOW_BLUR)`). What is cut is geometry — an axis, a
      contour — while what is drawn is wider than it: the ribbon's round cap by half its
      width, the shadow's soft band by `SHADOW_BLUR`. Cut flush with the curb and both land
      on the deck. One number for the two, so the line and its shadow break off on the same
      line — the drawn edge of the bridge.
    - **Navigation is untouched**: `fence_gaps` still skips bridges (a span runs over the
      fence, not through it), so the pawn's fence and the polygonal mesh's fence are the
      same as before — only the picture is shorter. Pinned by
      `fences/tests.rs::
      nothing_of_a_fence_is_drawn_on_a_bridge_deck`: **no vertex of the layer**, line,
      shadow or penumbra, lies within `curb_reach` of a bridge centreline.
    - Cost: a grid of bridge segments (`GAP_CELL` 32 m) built once per rebuild beside
      the `fence_gaps` pass already paid there, and one
      cell lookup per fence link.
  - **The drawn width grows with the zoom** (`FENCE_LODS`: 0.25 → 0.5 → 1.3 m, then
    nothing past 0.9 m/px). A true 25 cm line is under a pixel from 0.3 m/px, which is
    exactly the scale a fence has to be visible at; aiming for ~1.5 screen px is the
    tram's trick and the honest one. The far bucket draws nothing at all, because at city
    scale the plot grid turns into dirt.
  - `fence_kind` is a whitelist for the reason every other one is: `barrier=*` also
    carries `kerb`, `gate`, `bollard`, `block` — points and street furniture, not lines —
    and `city_wall`, which the branch above already took.
  - **The branch falls through**, like the rail and tree-row ones: a way in OSM routinely
    carries `barrier=fence` alongside another feature's tags, and it has to become both —
    a `return` there loses whole blocks to the fence branch
    (`a_fenced_block_becomes_both_a_fence_and_a_quarter`).
  - **And it stands *above* the road branch**, beside the rail, tree-row and waterway
    ones — the road branch does `return`, and a fence mapped along a path rides the very
    same way as its `highway=*`; below the
    road branch that `return` ate the fence whole; pinned by
    `a_fenced_path_becomes_both_an_alley_and_a_fence`. The industry cylinder is the
    opposite case and sits lower, where its own `return` is what is wanted.
  - **The shadow is cast by the map's own sun**, so `rebuild_fences` is gated on
    `retuned::<FenceZoomBucket>.or_else(retuned::<SunOnMap>)` — the general Sun rule
    (**The slider is not the map's sun**, under **Height modes** in `buildings-heights.md`), not an
    exception to it. Heights are constants of the kind (`FENCE_HEIGHT` 2 m,
    `HEDGE_HEIGHT` 1.4 m) run through the same `shadow_dir()` / `shadow_length_scale()`
    the buildings and the cars use.
  - **The shadow is a sweep, not a shifted copy** (`fences::push_shadows`) — the defect
    the cars and the bridge shadow had already been through: a copy moves a 2 m fence
    7.4 m at a 15° sun, and against a 0.25–1.3 m ribbon that reads as a second fence
    beside the first. The sweep is the Minkowski sum of the ribbon with `[0, offset]`, so the shadow
    starts **under** the fence. The ribbon is not convex, so it is swept piecewise: each
    link's rectangle and each vertex's octagon (the `Round` joins and caps the ribbon is
    drawn with) through **`meshing::sweep_convex`** — the cars' hull walk, lifted out of
    `cars/body.rs` when this became its second caller. It is swept at the **drawn** width
    of the bucket: a fence parallel to the light casts a shadow exactly as wide as the line.
  - **The sweeps are unioned** (`i_overlay`, `simplify_shape` NonZero), unlike the cars'.
    Two reasons, both about translucency: a link's and a joint's sweep overlap at every
    bend, and plot fences stand back to back, so at a low sun neighbouring shadows lie on
    each other along their whole length. The union removes both; the cars can skip it
    because their pitch is six metres. After the union each
    contour gets the cars' `SHADOW_BLUR` (0.35 m) band, tapered by `direction ·
    shadow_dir()` — hard at the fence, soft at the far edge. **Shadows first, lines last**
    still holds: the whole union goes into the mesh before any ribbon.
  - **A see-through fence casts no shadow** — `FenceKind::Railing`, a `barrier=fence`
    whose every `fence_type` value is in `parse/tags.rs::SEE_THROUGH_FENCES` (`metal`,
    `bars`, `chain_link`, `mesh`, `wire`, `pole`, …; a mix such as
    `barbed_wire;concrete` stays a solid `Fence`). It is drawn as a grey-metal line and
    skipped by `push_shadows`. The case that brought it: a metre-high `metal` median
    railing swept a solid 2 m shadow onto the asphalt and read as a dark bar between the
    carriageways (`fences/tests.rs::a_railing_casts_no_shadow`). Navigation does not look at the kind — the band is the
    same 0.3 m.
