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
  than as a shoal). It used to be the rim above (`WATER_RIM`), and the rim failed twice on
  the Упа's southern arm, both reported from one screenshot: the arm is its own
  multipolygon (19415535) butting into the river's (19409693) with **a shared border
  across the mouth**, and each laid its light rim along it — a shoal line across open
  water; and the rim's thickness clamp (0.6 × area / perimeter) darkened the 18 m arm
  toward its middle faster than a real channel shallows, so the river's shoal stopped at
  the mouth instead of running on into the arm.
  - **Union first** (`i_overlay`, NonZero, outer rings oriented CCW and holes CW — OSM's
    order is arbitrary and NonZero only merges consistent windings): the shared border is
    gone before anything is laid. **The bands of the water gaps go into that same union**
    (`split_channels`, under **Waterways** below) — two polygons of one river that do
    *not* touch, because OSM cut the `riverbank` at a bridge, are joined by the channel
    between them, and their service edges across the river stop being banks the same way.
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
    `surface meshing:` line (it had its own `water meshing:` line until the surface
    layers went on the seam).
- **Waterways** (`map/water.rs::mesh_water_lines`, the `waterways` layer at `Z_WATERWAY` 2.02,
  `SurfaceKind::Water`) — the open channels, and two decisions, both from screenshots
  of the Упа's southern arm (`waterway=river` 221646296 at `cam 5366 4254`):
  - **Water lies over every road ribbon and under the bridge shadow — both layers**:
    area water at `Z_POND` 2.01, the channels a hair above it. They used to sit at
    1.0 / 1.05, under sidewalks, alleys and roads, and the embankment footways of that
    arm lay *on* the water for 1–3 m along it — not across it: none of them crosses,
    their axes run 2.6–4.5 m from the channel's. The rule now is the author's: **a road
    lies over water only as a bridge**. It is also what the navmesh says — area water
    and an open channel both block, a road carves nothing, only a bridge does — so a
    street drawn over water lied twice.
    **Both layers, because OSM maps a narrow arm either way**, and the first version of
    this fix moved only the channel and missed it: the arm at the arrow is *also* a
    multipolygon `natural=water` (relation 19415535, 18 m across), so the clipped channel
    left the polygon showing, still under the footways. Found by sinking the `alleys`
    layer's `Transform` over BRP (the water came back) and lifting `waterways` to z 10
    (nothing changed — the blue was not the ribbon).
    The price is stated, not hidden: water cuts an embankment sidewalk or footway mapped
    against the bank, and a street over a culvert OSM does not mark (Tula: one `path` ×
    `stream` at (3681, 70), one footway on the `weir` at (5251, 2546); every other
    crossing carries `bridge=yes`). Cutting the road at the crossing was rejected for
    that very screenshot — there was no crossing to cut.
  - **The mouth.** The channel ribbon has its own **shore** — the same distance field as
    the **Shoal** (`WATER_SHORE_COLOR`, `WATER_SHORE_WIDTH` 6 m, unclamped, so a 2.5 m
    stream is pale across its whole width like a narrow arm) — laid in
    `surface.wgsl` by the ribbon's `across`, since a ribbon has no ring to offset. That is
    not enough on its own: an OSM channel runs on **inside** the area water it flows into
    (13 of Tula's open channel ends lie inside a water polygon, most of them the Упа's
    own centreline inside its `riverbank`), and there its light edges would be two shoal
    lines across deep water, while across the bank rim its deep middle cut the shoal with
    a rectangle — the artifact reported. So `WaterIndex::open_runs` cuts the smoothed
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
    **discontinuity of the area water**, and the fade above is wrong for it in both
    halves. OSM cuts `riverbank` at a bridge — Tula, the Упа at (6157, 3397):
    `relation 19409693` ends before the bridge, `relation 19409692` starts after it,
    fifteen metres of nobody's land between them with the centreline `way 25857971`
    running through. Reported from a screenshot as «странно выглядящая вода», and it was
    two defects at one place: the ribbon's reach laid a rectangle of **deep** water over
    the pale shoal of a polygon only eleven metres wide (the reach fades to `WATER_COLOR`,
    which agrees with the water beside it only where that water is deep), and each polygon
    laid **its own shoal along the service edge** it is closed with across the river — a
    light band where there is no bank, the very artifact the union fixed for polygons that
    *touch*. So `split_channels` hands such a stretch over: its band (the axis spread by
    `miter_offsets` to the channel width, reaches included, so the overlap with both
    polygons is real and the union cannot leave a hairline) goes into
    `mesh_water_areas`'s union, and the ribbon does not draw it. The service edges are
    then interior, the shoal runs along the real banks and wraps into the channel, and the
    river visibly narrows to the channel's width under the deck — which is honest, and
    mostly hidden by the deck. The two doors share one cut for that reason: «где кончается
    полигон» must have one answer, not two. Tula: **one** gap, in the `surface meshing:`
    line as `N gaps`. An **island** in a river answers the same way, and rightly — there
    is water on both sides of it.
  - **Render-only.** The navmesh blocks the polygon and the whole channel band as before;
    the caps rule it shares with the drawing (`water_line_caps`) is untouched.
  - Cost: one pass per open channel at load — the `waterways` field of `SurfaceReport`,
    printed inside the one `surface meshing:` line (it had its own `waterways meshing:`
    line until the surface layers went on the seam).

## Ground

- **The yard** — what stops the city from being a beige sheet with buildings on it; it
  came from looking at the first offscreen shot (#27):
  - **The residential block is the yard.** `RESIDENTIAL_COLOR` went from half a tone off
    the ground to a muted green, and the `landuse` layer split in two (`landuse_yards`
    with `SurfaceKind::Yard`, `landuse_works` with `Ground`) — one kind for both would
    have put grass speckle on a concrete yard. **`Yard` is its own kind** rather than
    `Grass`: the mottle is 0.105 against 0.06 of amplitude on a 22 m wavelength against
    30 (and the speckle is sparser and higher-threshold), which is
    exactly the difference between a meadow and ground people walk over — bare patches by
    the doors, grass in the corners. With plain `Grass` the block came out as a golf
    course. Nothing else changed: the roads, the
    parking, the buildings and the pitches are all drawn *over* the block, so the green
    only shows where nothing else is, which is exactly where the grass is.
  - **Worn paths were tried here and removed.** A straight 1.1 m strip of bare earth from
    every OSM entrance to the nearest kerb of the nearest road (`map/paths.rs`,
    `Z_WORN_PATH` 0.72) — the desire lines. On the city it looked bad and was taken out
    by the author's call; the commits describing it are history, not a missing file.
    Don't reintroduce a straight door-to-road strip. The uniform-grid index stays — it is
    the door generator's own (`osm/entrances/index.rs`), extracted while this layer
    existed and its only surviving trace; it has since become `map/grid.rs::Grid` (above).
- **Pitches** (`map/pitch.rs`) — sports and children's grounds, the thing a courtyard is
  actually *made of* on an aerial photo. One surface layer at `Z_PITCH` 2.005 (2.003 until the two
  big-lot road layers took 2.002–2.004 — a step under 0.001 is not worth trusting to the
  depth buffer) and one markings layer at 2.006, the parking pair's shape exactly: the paint is flat
  `ColorMaterial`, the surface carries `SurfaceKind::Ground` (a neutral mottle — a
  football field must not get the street's asphalt grain).
  - **Over every road ribbon, under water.** It sat at 0.75, under the sidewalks and
    alleys, and OSM routinely runs yard footways straight across a field — reported
    from a screenshot of a Tula courtyard where the paths cut the pitch into pieces. On
    a photo the field is whole and the path stops at its edge. Render-only: a pitch
    blocks nothing on the navmesh, so pawns still walk the path across it. Parking lies
    over the roads too, one hair lower (see **Parking**), so where a lot and a pitch
    overlap the pitch wins.
  - **The kind is decided in three steps** (`parse/tags.rs::pitch_kind`), because `sport`
    is missing on a quarter of Tula's pitches: `leisure` first (`track` → `Track`,
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

- **Rail layers** (`map/rail.rs`, its own module with its own zoom LOD, like the tram's;
  it left `map/roads.rs` when it stopped being a line style) — the **track**, not a map
  symbol: a ballast prism, ties across it and two steel rails on the gauge. Three merged
  meshes, `RailLayerTag`, all above `Z_ROAD` (2) so a track lies on its street at a level
  crossing, and all **below `Z_BRIDGE_SHADOW` (2.05)** so a road bridge over the tracks
  covers them (Tula, 5896 4324: the track used to run over the deck — it sat at 2.4–2.55,
  above every bridge, for the tram's sake, and the tram has its own layer above the deck
  since): ballast `Z_RAIL` (2.03), ties `Z_RAIL_TIE` (2.035), steel `Z_RAIL_STEEL` (2.04)
  — the far-bucket
  dash rides in the tie mesh, it never coexists with ties. Three rather
  than one, for the casing reason inverted — coplanar geometry z-fights, and a tie must
  sit above *every* ballast, or a junction of several ways delaminates. Inside the
  ballast mesh the same rule is push order: **all** shoulders first, then all beds.
  - **The prism** — the OSM width (rail 5 m, light_rail/narrow_gauge/subway 4, disused
    3.5) is the bed; the tie length and the far-bucket dash width scale off it, so a
    disused track is drawn narrower — but the **gauge is absolute** (1.5 m on every
    bed: light_rail, subway and disused track are physically the same 1520 mm), because
    a 30% share on a 4 m / 3.5 m bed put the two rails 3.6 / 3.0 px apart at the far
    edge of bucket 1, under the 4 px floor at which they still read as two; the shoulder
    under it is `SHOULDER_SCALE` (1.22) of that, darker. It is
    the slope that separates the track from the ground it runs on; without it the track
    is a flat ribbon again.
  - **`RAIL_LODS`** — five buckets over the camera zoom range, and they change the
    *drawing*, not its size: close up the real thing (ties 2.6 × 0.26 m every 65 cm,
    a 1.5 m gauge on every bed with 12 cm rails); by 0.26 m/px the two
    rails no longer separate on screen and are dropped, ties thicken and thin out into
    hatching; from 0.65 m/px the ties go too and a coarse dash pattern comes
    back, because a bare grey band reads as another street. That dash was osm-carto's
    **white** until the first offscreen shot showed what it does to a photo — a white
    ladder across the whole station throat, the most map-like thing in the frame — and it
    is now **darker than the ballast**: the ties are already gone on these buckets, so
    from the air a track is a grey band with a dark dash running **along** its middle —
    a mark of the track itself, never a white one. Its colour is exactly
    `RailPalette::tie` of its own palette (active 0.243/0.196/0.157, disused
    0.400/0.361/0.302) — the dash *is* the ties that stopped being drawn, so the mark
    does not brighten in a step at the threshold, and the tone is all the sign has: on
    the last bucket the dash is `min_bed` 9 m × `width_scale` 0.5 ÷ `MAX_ZOOM` 4.5 =
    exactly 1 px wide. That is why the contrast was bought back with the colour rather
    than by widening `width_scale` — the pixel arithmetic of this paragraph stays put.
    The pin runs both ways: `MAX_ZOOM` cannot be raised without re-tuning this rung's
    `min_bed` and `width_scale` and the last rung of `TRAM_LODS` with them. Fitting the
    widened map by height would need 5.28, and that order breaks them one after another —
    the ballast first (2.0 px against a 1.8 floor), then this dash (exactly 1 px against a
    1.0 floor), then the tram ribbon (1.16 px against the same floor).
    Since the tone is all the sign has, "darker" is not enough and the test pins a
    **floor on the contrast**: WCAG `(L + 0.05)` ratio of ballast to dash over bevy's
    linear `luminance()`, at least `MIN_DASH_CONTRAST` 1.75. A plain luminance
    difference is blind here — ~0.143 on both palettes — while the ratio shows the
    disused track's real, thinner margin: active 2.69, disused 1.90. The floor lets the
    disused tie lighten by 0.02 per sRGB channel (1.76) and fails at 0.03 (1.69), so
    greying that palette further has to move its ballast with it.
    `min_bed` floors the ballast width on the last two buckets — 5 m is a pixel at city
    scale, and the track would vanish before the roads it crosses. The numbers are
    derived from the screen size at the **worst** (far) edge of each bucket, and they
    hold on **every** parsed bed width (5 / 4 / 3.5 m), not only the mainline's: tie
    spacing never below ~6 px, no mark below ~1 px. The second number that must hold
    across buckets is the **tie duty cycle**, ~40% (the real 0.26 m in 0.65) — measured
    live: at 31% the ties stop being a texture, become sparse marks, and the two white
    rails outweigh them into a ladder. Both, plus the one-way progression (detail only
    ever falls away), that ties and dashes never coexist, and that the dash of **every**
    palette clears that contrast floor against its own ballast, are pinned by
    `rail/tests.rs`. That the dash *is* the tie colour is not a test but a construction:
    both fields of a palette are one `*_TIE` constant.
  - **What a bucket costs** (Tula, 69 km of non-tram track inside the map, measured on
    an M1 Max from the `rail meshing:` log line): bucket 4 45 k verts / 1 ms, bucket 2
    131 k / 3 ms, bucket 1 298 k / 9 ms, bucket 0 673 k / 23 ms — a one-off hitch on the
    threshold crossing, and ~23 MB of buffer at the deepest bucket. Frame rate stays
    vsync-capped at 60 there. The 23 ms is what a denser tie step would multiply, so
    treat bucket 0's 65 cm as the floor.
  - **`RailZoomBucket`** is `ZoomBucket<RailLods>` — the same machinery as the tram's
    (see **Zoom buckets** in `SKILL.md`), a separate resource because the tables' thresholds
    have nothing in common: `RailLods` is the marker that hands `RAIL_LODS`'s
    `max_zoom`s to `map/zoom.rs`.
  - **`MeshBuilder::push_rails`** is the new primitive: two ribbons offset from the
    centerline by half the gauge, using the very `miter_offsets` that build a ribbon's
    edge, so the rails hold the gauge through a bend instead of drifting outward at the
    corner. `push_dashes` (far buckets) and `push_ticks` (ties) are as before.
  - **No style resource.** Like the tram (whose only resource is a visibility toggle),
    rails ignore `RoadStyle` and hardwire
    `Round` + `Light` with a fixed `RAIL_SMOOTH_WIDTH` (5 m), so the centerline is
    identical on every bucket — a smoothing knob would slide the track against its own
    ballast, and an LOD switch would wiggle it.
  - **`RailKind` is the palette**: `Active` is ballast grey-brown, creosote ties, bright
    steel; `Disused` is the same track overgrown — weedy ballast, grey ties, rust.
    `Tram` is skipped here, it has its own module.
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

  **Tram zoom LOD** (`TRAM_LODS`) — the mesh is rebuilt at discrete zoom thresholds,
  pseudo-gizmo style: five buckets over the camera zoom range, each with its own line
  width (targeting ~1.8 screen px, so the line neither fattens close up nor vanishes far
  out) and tie length/thickness/spacing (on-screen tie spacing never drops below ~10 px);
  the farthest bucket drops ties entirely, as 2GIS does at city scale. `TramZoomBucket`
  is `ZoomBucket<TramLods>` (see **Zoom buckets** in `SKILL.md`), so `rebuild_tram` fires only
  on an actual threshold crossing, never per frame. The tram centerline is smoothed with a
  fixed `TRAM_SMOOTH_WIDTH` (1.2 m) clamp rather than the bucket's line width, so the
  path itself is identical across buckets and LOD switches don't wiggle the track.
  `RailLine::width` from parse is ignored for trams.

## Industry, standing stock, fences

- **Industry** (`map/industry.rs`) — the industrial belt, added in `QUERY_VERSION` **11**.
  Five layers from two sources (`Structure` and `PipeLine`, see **MapData — the parsed
  model** in `SKILL.md`), rebuilt on
  `industry::rebuilds_on()` — `retuned::<SunOnMap>.or_else(retuned::<BuildingHeightMode>)
  .or_else(retuned::<IndustryStyle>)`
  and on nothing else — the settled sun, never `SunStyle`, like every other rebuild — and
  the system stands on its own rather than in the zoom-bucket chain, because there is no
  zoom bucket here: a cylinder is visible exactly as far as its shadow is.
  **One registration carrying all three conditions, never three registrations** — this is
  the layer the rule is written from (it arrived with its `rebuild_industry` listed twice
  in `Update`); the rule itself lives once, on `roads::rebuilds_on` and under **When a
  layer rebuilds** above.
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
    below. Tula ships only chimneys and towers, so the pair never occurs on the shipped
    data.
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
    (`silhouette_edges(outer, -lift_dir)`). The far half was drawn first and left the
    near end of the silhouette open, and through that hole the cylinder's own base
    shadow showed as a dark half-disc under the chimney. The geometry in one line: the
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
  - **But service track is not a station, and density follows the place.** The first
    version stood rakes at one rate on every service track — ~73 % of its length, ~3.4 k
    wagons on Tula — and the author's verdict was "far too many; many where the trains
    stand, at stations, almost none on an ordinary track". The tag cannot say it: a spur to
    a plant, a lone dead end and a park of sidings carry the same `service`, and Tula's
    spurs are the longest group (72 ways, 24 km). What does say it is **the fan**: a station
    is a bundle of parallel tracks metres apart, a spur runs alone. So every rake first asks
    `Fan::width_at` how many **other** `Active` tracks (running lines included — a passing
    loop beside a double-track main *is* a station) pass within `FAN_REACH` 12 m of its
    middle, and stands with the share `FAN_FILL[width.min(3)]` = 1.4 % / 7 % / 28 % / 52.5 %,
    times `class_fill` (`Spur` 0.5, the others 1). The row was 2 / 10 / 40 / 75 % first
    (1195 wagons on Tula) and was scaled by 0.7 whole on the author's "30 % fewer" — the
    ratio between a park and a lone track was right, the total was not. 12 m and not 9 because the spacing in a
    park is 5.3–6.5 m, and the edge track of a fan must see its *two* neighbours (5.3, 10.6).
    One neighbour does not make a station — it is a loop beside the main or two parallel
    plant spurs. A rake that does not stand leaves its own span empty, so the phase of the
    rakes along a track does not depend on which of them stood.
    Measured on the Tula cache (service track inside the map, km by neighbour count
    0/1/2/3+): sidings 0.0/1.1/3.1/15.9, yard tracks 1.1/2.6/5.3/8.8, spurs 5.3/3.7/2.9/3.8.
    **The index is a grid**, `FAN_CELL` 32 m, each segment registered in every cell its box
    inflated by `FAN_REACH` touches, so a query reads one cell; it is rebuilt with the layer,
    queried once per rake. Pinned by `a_lone_track_stands_almost_empty`,
    `a_spur_stands_thinner_than_a_siding`, `the_fan_counts_other_stock_tracks_within_reach`.
    **Rejected: `railway=station` / `landuse=railway`** — neither is in the query (a
    `QUERY_VERSION` bump), and a station node does not say where the station ends.
  - **Rakes, not rows**: `RAKE_MIN..=RAKE_MAX` (3–16) cars coupled at `COUPLED_GAP` 0.9 m,
    then `GAP_MIN..GAP_MAX` (12–90 m) of empty track, seeded from the track's first point
    through the shared `seed::seed_from_point`. An even row at a fixed pitch reads as a
    fence. The rake length is a **count**, so `RAKE_MIN`/`RAKE_MAX` are `u32` and the roll
    is `range(MIN, MAX + 1)`: as `f32` bounds they described a half-open interval and the
    truncating cast ate the 16-car rake the docs promised. That is half the answer: `range`
    is not strictly half-open either (see **One RNG and one point seed** in `SKILL.md`), and over
    3..17 it rounds up to `17.0` on another 127 generator states, which the cast then turned
    into a rake of `RAKE_MAX + 1`. So the roll carries a `.min(RAKE_MAX)` of its own.
  - **Walked along the whole track's arclength**, the same `along::{arclengths,
    place_on_path}` the cars use (see below), with `TRACK_MIN` 40 m and `END_MARGIN` 12 m
    measured from the **ends of the track**, not of a link. The `points.windows(2)` walk
    that stood here first is the very one the cars had already given up, and a yard is not
    the exception it was assumed to be: on Tula's service track half the links are shorter
    than `TRACK_MIN` and carry a fifth of the length, 11 tracks of 159 came out empty for
    that reason alone, and the margin was being kept clear of every interior bend, where
    there is no switch. The rake phase reset at every vertex on top of that. Curvature is
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
    `CarZoomBucket` would have hidden the yards early. By the cars' own 5.5 px criterion
    the wagon threshold would sit at 13.9/5.5 = 2.53 m/px; 2.0 is the conservative side
    of it. `Z_WAGON` 2.045 — above the rail steel (a wagon stands *on* the rail),
    below the bridges, like the track under it.
  - **No style resource, unlike the cars.** The layer is decoration and still has no
    `visible`: it comes off by `WagonZoomBucket` alone. That is a difference from
    `map/cars/` the summary used to deny.
  - **No `QUERY_VERSION` bump**: `out geom` returns every tag of the element, so `service`
    has been sitting in every cache since v4.
- **Fences** (`map/fences.rs`) — `barrier=fence|wall|retaining_wall|hedge`, added in
  `QUERY_VERSION` **14**. What is drawn is a thin ribbon **and its shadow**, and the
  shadow is the point: from above a fence is a 25 cm hair, and on a photo it is the dark
  thread beside it that you actually see. In a private-house district that grid of plot
  boundaries *is* the texture of the district.
  - **`FenceLine` is a separate type, not `WallLine` with a flag.** The kremlin wall is
    impassable end to end; a fence blocks the navmesh **with gaps** — a road through it,
    a default gate — at its physical 0.3 m, not at the drawn width below. It used to be
    pure decoration, on the argument that 429 lines across the courtyards would strand
    the crowd; that argument is exactly what the gaps and the default gates answer (the
    measurement is in the navigation-deep skill). **The gaps are drawn as gaps**, by the
    author's call: `mesh_fences` cuts every fence with `footprint::fence_pieces` — the
    polyline minus the gap discs, the very discs the polygonal mesh subtracts — and lays
    both the shadow and the ribbon from the pieces, so no fence is drawn across a path a
    pawn walks. A leftover under `MIN_FENCE_PIECE` 0.5 m at a gap edge is not drawn. The
    gaps are recomputed on every rebuild (`fence_gaps`, 8.7 ms on Tula) rather than
    cached: the layer rebuilds on a zoom crossing or a settled sun, not per frame.
  - **And the stretch under a bridge is not drawn at all** (`footprint::BridgeDecks`,
    the third argument of `fence_pieces`), which is the one place the drawn fence and the
    blocking one part on purpose. `Z_FENCE` 2.75 lies above `Z_BRIDGE` 2.2, so a fence
    crossing under a span was drawn **over** the deck — a dark thread with its own shadow
    across the bridge, reported from a screenshot. The cut is the same idiom by which the
    car layer keeps off a deck (`cars::BridgeDeck` — everything the z ladder puts above
    the bridges yields to them), but the shape differs: a car is dropped whole, a fence is
    cut at the edge the way a gap cuts it.
    - **What covers is the drawn ribbon, not a crossing of centrelines.** `fence_gaps`
      measures a road's opening as a disc at the crossing precisely so that a street
      *along* a fence does not erase it; a deck is the opposite case — it hides exactly
      what lies beneath, along or across. Hence a **capsule**: the polyline inflated by
      `RoadLine::curb_reach` (deck + curb, the whole drawn width of a bridge), with round
      ends. `segment_in_capsule` unions the band and the two end discs, which is a single
      interval because a capsule is convex.
    - **The shadow is cut by a boolean, not by the piece**, and cutting only the pieces is
      what the first version did — reported again from the same bridge, with the line gone
      and the grey band still running onto the deck. A shadow is an **area**: the sweep of
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
      `a_bridge_hides_the_stretch_of_fence_it_covers` (no gap, and the pieces end on the
      curb edge), `a_fence_running_under_a_deck_is_hidden_along_its_whole_length`, and —
      the one that states the whole rule — `fences/tests.rs::
      nothing_of_a_fence_is_drawn_on_a_bridge_deck`: **no vertex of the layer**, line,
      shadow or penumbra, lies within `curb_reach` of a bridge centreline.
    - Cost: a grid of bridge segments (`GAP_CELL` 32 m, hundreds of segments per city)
      built once per rebuild beside the 8.7 ms `fence_gaps` already paid there, and one
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
    carries `barrier=fence` alongside another feature's tags, and it has to become both.
    With a `return` there Tula lost a block and a park to the fence branch — caught by the
    counts in the `osm map:` line, not by any test, which is why there is a test now
    (`a_fenced_block_becomes_both_a_fence_and_a_quarter`).
  - **And it stands *above* the road branch**, beside the rail, tree-row and waterway
    ones — the road branch does `return`, and a fence mapped along a path rides the very
    same way as its `highway=*` (Paris and London carry one such way each). Below the
    road branch that `return` ate the fence whole; pinned by
    `a_fenced_path_becomes_both_an_alley_and_a_fence`. The industry cylinder is the
    opposite case and sits lower, where its own `return` is what is wanted.
  - **The shadow is cast by the map's own sun**, so `rebuild_fences` is gated on
    `retuned::<FenceZoomBucket>.or_else(retuned::<SunOnMap>)` — the general Sun rule
    (**The slider is not the map's sun**, under **Height modes** in `buildings.md`), not an
    exception to it. Heights are constants of the kind (`FENCE_HEIGHT` 2 m,
    `HEDGE_HEIGHT` 1.4 m) run through the same `shadow_dir()` / `shadow_length_scale()`
    the buildings and the cars use.
  - **The shadow is a sweep, not a shifted copy** (`fences::push_shadows`). The copy that
    stood here first — the ribbon moved by `shadow_dir() × height × cot(elevation)` — is
    the defect the cars and the bridge shadow had already been through: at 15° a 2 m fence
    is moved 7.4 m, and against a 0.25–1.3 m ribbon that reads as a second fence beside the
    first. The sweep is the Minkowski sum of the ribbon with `[0, offset]`, so the shadow
    starts **under** the fence. The ribbon is not convex, so it is swept piecewise: each
    link's rectangle and each vertex's octagon (the `Round` joins and caps the ribbon is
    drawn with) through **`meshing::sweep_convex`** — the cars' hull walk, lifted out of
    `cars/body.rs` when this became its second caller. It is swept at the **drawn** width
    of the bucket: a fence parallel to the light casts a shadow exactly as wide as the line.
  - **The sweeps are unioned** (`i_overlay`, `simplify_shape` NonZero), unlike the cars'.
    Two reasons, both about translucency: a link's and a joint's sweep overlap at every
    bend, and plot fences stand back to back, so at a low sun neighbouring shadows lie on
    each other along their whole length. The union removes both; the cars can skip it
    because their pitch is six metres and their count is 22 k. After the union each
    contour gets the cars' `SHADOW_BLUR` (0.35 m) band, tapered by `direction ·
    shadow_dir()` — hard at the fence, soft at the far edge. **Shadows first, lines last**
    still holds: the whole union goes into the mesh before any ribbon.
  - Tula: **429 lines** — 356 `fence`, 71 `wall` + 1 `retaining_wall` (both drawn as a
    wall, so 72 walls), 1 `hedge` (the audit was right that live hedges are mapped as
    `barrier=hedge`, not `natural=hedge` — the latter is zero in all six cities). The
    audit's `barrier` row says 428 for Tula and is not in conflict: it counts
    `fence|wall|hedge` and leaves `retaining_wall` out, as its own caption says.
