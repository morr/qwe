---
name: osm-map
description: Use when working on the OSM pipeline or map rendering in qwe — the Overpass query/mirrors/cache, map/osm/* parsing, the MapData model, building heights, entrance generation, tree planting and crowns, merged-mesh rendering, road/rail/tram/bridge layers, building and tree style resources. Deep detail behind CONTEXT.md's OSM section.
---

# OSM map pipeline — deep detail

This is the detail layer behind the **OSM map pipeline** summary in `CONTEXT.md`:
how the downloaded data becomes `MapData` and pixels.

Four deep dives live next to this file and are read on demand:

- `references/osm-coverage.md` — the **tag coverage audit** (in Russian): which OSM
  tags reach the map, which are downloaded and thrown away, which are never asked for,
  with per-city counts, and how to regenerate them (`tools/osm_audit/`). Read it
  before widening the Overpass query — a feature that already exists must not get
  "added" twice. Widening the query or adding a `parse_way` branch means updating it
  in the same change.
- `references/entrances.md` — the door generator: the measured statistics behind the
  cohort table, the pitch law, blocked walls, determinism.
- `references/trees.md` — planting (woods, standalone, rows, density/thresholds) and
  rendering (crowns, TreeStyle/TreeRowStyle, conifer stands).
- `references/tree-algo.md` — the watabou Village Generator crown algorithm (in
  Russian), reverse-engineered from `Village.js`; the ground truth `map/trees/crown.rs`
  is written against.

When a change here introduces or retires a concept, update the matching summary bullet
in `CONTEXT.md` and the detail here in the same change.

## Download & cache

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). Query covers: `building` (way+rel),
  `highway` (way), `natural=water` / `waterway=riverbank` (way+rel),
  `waterway=river|stream|brook|canal|ditch|drain|weir` (way — the *linear* watercourses),
  `leisure=park|garden`,
  `landuse=recreation_ground|forest` + `natural=wood`, `natural=tree_row` (way),
  `natural=tree` (node), `landuse=grass|meadow` / `natural=grassland|meadow`,
  `natural=sand|beach`, `landuse=residential|industrial|garages` (way+rel),
  `barrier=city_wall`. The bbox is `MAP_SIZE` around the selected
  `City`'s geo center. `QUERY_VERSION` is **8** (v3 added `entrance` nodes, v4 `railway`,
  v5 `natural=tree_row`, v6 `natural=tree` nodes, v7 linear `waterway`, v8 `landuse`
  blocks).
- **Mirrors** — `OVERPASS_URLS` in `download.rs` is tried in order (`maps.mail.ru` →
  `overpass-api.de` → `kumi.systems` → `private.coffee`). The VK/Mail.ru instance leads:
  full planet, current data, and the nearest pipe from here — Berlin took 19 s through it
  against ~2.5 min and two 504s through the European ones. On dense cities those answer
  504 "server too busy", or worse, a **200 with an HTML error page** — hence the
  "response must start with `{`" check before a mirror is considered successful.
- **Cache** — `assets/osm/tula_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored).
  Parameters live in the file name, so changing settings invalidates it; **bump
  `QUERY_VERSION` in `overpass.rs` whenever the query gains tags**, or every existing
  cache keeps serving an extract that lacks them. Written **only after successful
  parse**; a broken cache self-heals (deleted, re-downloaded). Second launch never
  touches the network.
- **One file per city** — every load first runs `prune_stale_caches()`: anything under a
  known city slug that is not that city's current `cache_path` is deleted. That is what
  retires extracts left by an old geo center, `MAP_SIZE` or `QUERY_VERSION` — tens of MB
  each. It sweeps **all** cities, not just the one being loaded, so junk under a city
  nobody visits still goes; the current file of each city survives, so a tour of the six
  is not six downloads per lap.

## MapData — the parsed model

`map/osm/model.rs`; the resource stays resident after spawn.

- **PolyArea** — polygon with holes; rings are open (no repeated last point).
  `AreaKind: Building | Kremlin | Water | Park | Wood | Grass | Sand | Residential |
  Industrial`. **Park** is the
  light base fill; **Wood** (`natural=wood` / `landuse=forest`) are the darker stands
  *inside* it and the **only** areas that carry trees; **Grass** (lawns, meadows) and
  **Sand** (beaches) also sit above the park fill, lighter green / sandy. Everything
  but Wood stays open ground — that is what makes the open half of a park read as a
  field, the way it does on OSM. **Residential** (`landuse=residential`) and
  **Industrial** (`landuse=industrial|garages`) are the *blocks* — `MapData::landuse`,
  one merged layer at `Z_LANDUSE` (0.25) between the ground mesh and the parks, half
  a tone off the ground colour (warmer/lighter for housing, greyer for industry) so the
  city stops being one flat sheet. `area_kind` tries them **last**: any green tag on the
  same polygon wins. They touch neither the navmesh nor tree planting. Tula v8: 264
  residential + 35 industrial/garages in the bbox (the audit table), 294 of them
  reach `MapData::landuse`; `commercial`/`retail` are not requested.
  `height: Option<f32>` — metres, buildings only (`None` on water/parks even if the
  tag is there). See **Building height** below. `building_use: BuildingUse` — the
  drawing class (`Other` on everything that is not a building), see **Building use**
  below. `entrances: Vec<Vec2>` — the OSM
  doors on this building's outline, empty for most buildings; see
  `references/entrances.md`.
- **RoadLine** — centerline polyline + width by highway class (primary 16 → footway
  3.5). `RoadClass: Street | Alley` (alleys = footways, park paths; different color and
  z). `bridge` and `passage` flags — the navmesh carves (see the navigation-deep
  skill); `bridge` also moves the road into the bridge deck layers (see **Bridge
  layers** below). Three more fields feed the **markings** only: `oneway`
  (`oneway=yes|1|true|-1` — direction is irrelevant, we draw, we don't route;
  `reversible`/`alternating` are not one-way), `roundabout`
  (`junction=roundabout|circular`, implies `oneway`) and `lanes: Option<u8>` (the `lanes`
  tag through `parse_measure`, floored, 1–8; `2;3` reads as 2, `0` and `12` as no tag).
  Coverage per city is in `references/osm-coverage.md` — Tula has `lanes` on 97 % of its
  streets ≥ 8 m, the European cities on about half.
  **Underground road is dropped** (`parse/tags.rs::is_road_underground`) — the same rule rails
  and watercourses have always had, and it was simply missing on the highway branch:
  metro concourses and stairs came out as ordinary alleys drawn over the city (Tokyo
  1053 of 12 859 ways — 8.2%; London 1808, Paris 1343, Berlin 787, Tula 34; counts in
  `references/osm-coverage.md`). It is a **separate predicate** from `is_underground`,
  and the reason is that **the risk is asymmetric here**: an extra ribbon is cosmetic,
  an extra deletion is a hole in the navmesh, because roads are exactly what carves it.
  So the rule stands down wherever the tag may not be describing the road — a `bridge`
  or an arch (`is_building_passage`), both of which exist at walking level by the very
  role that carves the navmesh, and `tunnel=culvert`, which belongs to the *stream*
  piped under the street (and for that stream it must keep meaning "underground", or the
  pipe walls off the navmesh). All three exclusions are pinned by tests; the blunt
  version cost Tokyo 331 arches and 17 bridges, London 177 arches. Verified live on
  Tula: `navmesh: pruned 9898` before and after — the navmesh did not move.
- **RailLine** — `railway=*` centerline + width by value (`rail` 5 → `light_rail` /
  `narrow_gauge` / `subway` 4 → the disused values 3.5 → `tram` 1.2).
  `RailKind: Active | Tram | Disused` — the
  kind *is* the drawing style, not a label: **Tram** is a thin line with cross ties
  (see **Tram** below — its width from parse is ignored, the zoom LOD picks it),
  **Disused** (`abandoned` / `disused` / `razed` / `dismantled`)
  is the `Active` track overgrown — same construction, weedy palette. A tram runs *on*
  the carriageway, so a gauge-wide ballast would cover its own street; `Active` and
  `Disused` get one (see **Rail layers** below — the parsed width is the bed).
  `parse/tags.rs::rail_class` is a
  **whitelist**, so the station vocabulary (`platform`, `station`, `switch`, `signal`,
  `construction`, …) never becomes a line. The rail branch in `parse_way` runs *before*
  the highway branch and deliberately **falls through**: an OSM way is routinely tagged
  both `railway=tram` and `highway=*`, and such a way is both a street and a track.
  **Underground track is dropped** (`parse/tags.rs::is_underground`) — a metro tunnel or a
  sunken through-line is invisible from above. Both markers are needed, neither alone
  suffices: of Tula's three underground ways two carry `tunnel=yes` *and* `layer=-1`,
  the third only `layer=-1`. `tunnel=no` is an explicit no, and elevated track
  (`layer` ≥ 0, no tunnel) still draws — that is what keeps an elevated subway on the
  map. **Rails never touch the navmesh** — see the navigation-deep skill.
- **WallLine** — `barrier=city_wall` (the Tula kremlin), 3 m wide, kremlin red,
  impassable.
- **WaterLine** — a *linear* watercourse: `waterway=river` 8 m → `canal` (and `weir`)
  6/4 m → `stream|brook` 2.5 m → `ditch|drain` 1.5 m, water blue, one merged ribbon at
  `Z_WATERWAY`. Widths are drawing widths, not hydrology: OSM draws as a line what is
  too narrow for a polygon, so a `river` line is narrower than the Упа (which is an
  area). A plausible `width` tag (`WATER_WIDTH_RANGE`, 0.5..50 m) overrides the class
  default. `parse/tags.rs::water_class` is a **whitelist** for the same reason `rail_class` is:
  `waterway=*` also carries `riverbank` (that one is an area, and `area_kind` claims
  it), `dam`, `dock`, `lock_gate`, `waterfall`. Like the rail and tree-row branches,
  the waterway branch in `parse_way` runs before `highway` and **falls through** — a
  culverted stream under a street shares its way with `highway=*`.
  **`tunnel: bool`** (`parse/tags.rs::is_underground`, the same test that drops subway track)
  marks a piped section: it is **not drawn at all** and, alone among
  watercourses, **does not block the navmesh** — the water runs under the ground and a
  pawn walks over it, so there is nothing to see and nothing to cross. Everything else
  about waterways *does* block; see the navigation-deep skill.
  A **culvert portal** — the node where an open way ends against the end of a piped one
  (`model::water_line_caps`) — is where the channel is cut **flat**. Everywhere else an
  open end is capped with a half-disk of half the channel width, because OSM splits one
  channel into several ways and the two caps meeting in a shared node fuse the joint;
  past a portal there is no more water, and the half-disk would jut into dry land and
  (the grid fill measures the same distance-to-segment) plug the culvert mouth with a
  semicircle of blocked tiles. One rule, both layers: `spawn::mesh_water_lines` and
  `Navmesh::fill_from_mapdata`.
- **TreeRow** — `natural=tree_row`: an avenue's centerline polyline plus what the data
  itself knows about the planting — `spacing: Option<f32>` (from `spacing`, or the row
  length spread over `count` / `tree:count`) and `radius: Option<f32>` (half
  `diameter_crown`). Both are rare, semi-standard tags, so almost every row is
  `None`/`None` and falls back to the density slider. Like the rail branch, the
  `tree_row` branch in `parse_way` runs before `highway` and **falls through**.
- **TreeNode** — `natural=tree` node: a single surveyed tree, position plus
  `radius: Option<f32>` (half `diameter_crown`, same parse as on rows). Raw input for
  `planting::plant_standalone`; see `references/trees.md`.
- **wood_trees / row_trees_kept / row_trees_slid** — `(pos, radius, appears_at)`,
  each sorted by threshold: the forest (with standalone surveyed trees at threshold 0
  in front), and the avenues under each placement policy.
  Raw material, not what the renderer reads.
- **trees** / **tree_appears_at** — what the renderer reads: `MapData::compose_trees`
  merges the forest with the avenues of the selected policy (a merge, not a sort — both
  inputs are already ordered). `composed_for` records which policy it was built for;
  it lives on `MapData` rather than in a system `Local` precisely because a city switch
  replaces the whole resource, and a `Local` would survive it and skip the rebuild.

## Parsing details

- **Building height** (`parse/tags.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim (New York — 97%, a
  LiDAR import) or else `building:levels` + `roof:levels` × `METERS_PER_LEVEL` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — and it is the majority
  everywhere but New York, so what fills it in matters: see **Inferred storeys** under
  Rendering. Coverage is logged per city on load (`N buildings (M with height)`).
- **Building use** (`parse/tags.rs::building_use`) — `BuildingUse: House | Apartments |
  Commercial | Industrial | Garage | Church | Public | Other`, the class that picks the
  wall colour (`map/buildings/mod.rs::facade_color`) and the **roofing material** the roof
  colour then comes from (**Roof material** under Rendering). Two sources in order:
  `building=*` when the value says something (`house`, `apartments`, `garages`, `church`,
  `school`, …), else `amenity=*` on the same outline (`school`, `hospital`, `police`,
  `place_of_worship`, …) — a school or a hospital in OSM is almost always `building=yes`
  + `amenity=…`. Anything outside the vocabulary is `Other`, the historical beige; the
  vocabulary covers what a city carries by the hundreds, not the OSM wiki. Tula: `yes`
  4004 of 7465, `house` 2249, `apartments` 744, commercial/retail/office 165,
  garage(s) 74, industrial 31, church 17. The Kremlin (`AreaKind::Kremlin`) keeps its
  red regardless of class. `roof:shape` is **not** read (283 of 7465 in Tula carry it);
  the roof shape is inferred instead — see **Gable roofs** under Rendering. The class is
  also one of the two inputs of **Inferred storeys** (the other is the footprint's shape),
  which is what fills in the height OSM does not carry.
- **Drowned buildings** (`parse.rs::drop_buildings_in_water`) — a building whose outline
  lies **entirely** inside a water polygon is dropped right after the element loop, before
  doors and trees. OSM tags floating restaurants and moored ships as buildings (`HMS
  Belfast`, `Café Barge`) and Tula carries a lone shed in the middle of Верхний пруд; the
  navmesh floods water impassable, so their doors are unreachable anyway and the box
  standing on the pond reads as a render bug. One vertex on land is enough to survive —
  piers and embankment houses stay. Counts: Tula 1, Berlin 6, NY 17, London 28, Paris 28,
  Tokyo 0; logged on stderr when non-zero.
- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.

## Testing the parse

`parse/tests.rs` is where a tag rule is pinned, and the fixture it uses is
`fixture.rs::Overpass` — a scene stated in **map metres**, turned into an Overpass
response, fed through the real `parse`:

```rust
let map = Overpass::new(CITY)
    .way(&[("railway", "rail")], vec![sw, ne])
    .way(&[("railway", "platform")], vec![sw, ne])       // dropped: whitelist
    .area(&[("natural", "wood")], square(CENTER, 110.0)) // area = closed way
    .node(&[("entrance", "main")], sw)
    .relation(&[("natural", "water")], &[("outer", …), ("inner", …)])
    .parse();
```

Four things about it worth knowing before writing a case:

- **Metres, not degrees.** Points are unprojected with `GeoBounds::unproject`, so a test
  builds its scene in the same numbers it asserts on (`CENTER`, `corners(half)`); the
  round trip is pinned by `unproject_returns_the_point_project_started_from`.
- **It goes through the JSON text**, not around it: deserialization of the Overpass DTO
  is part of what these tests cover. That is why the builder emits a string rather than
  handing `Element`s to an inner function.
- **`area` closes the ring, `way` does not, `relation` members are left as given** —
  OSM cuts an outer ring into several open ways, and assembling one from the pieces is
  itself under test.
- **Scenes are shared** where they repeat: `wood_scene()` (planting rules are checked as
  subtractions from a full wood), `tree_row(tags)`, `fixture()`.

A new tag reaching the map means a case here — a builder line and an assertion, not a
new JSON literal. Coverage of tags overall is the audit in `references/osm-coverage.md`.

## Footprint bands

`map/footprint.rs` — the strips linear geometry occupies on the ground, one construction
for every consumer: `Band { line, width, role }` built by `RoadLine::{deck_band,
curb_bands, passage_band}`, `WaterLine::channel_band` (`None` for culverts) and
`WallLine::band`. The width policy lives here too — `casing_width` (8%, 0.3–1 m) and
`bridge_curb_width` (12%, 0.8–2 m; ranges deliberately disjoint so a curb always
out-sticks a casing) — because a curb is not just paint: the same band blocks the
navmesh, and the drawn strip must match the blocked one by construction. Bands carry
the **centerline**, not a ready outline: the grid fill needs the centerline for its
4-connected-chain rasterization guarantee, the mesh build turns it into an outline, and
the renderer draws its own smoothed copy and takes only the widths (smoothing must not
move what blocks).

`CurbCoverage` also lives here: the shared *inputs* of the composite-bridge curb
decision — the bridge list and the joining-roads list, filtered by one `ways_joined`
(moved from `navigation`). `ways_joined` is reached only through an AABB prefilter
(`boxes_may_join`, bridge boxes precomputed once, both inflated by `JOIN_EPSILON`):
roads are tens of thousands and bridges hundreds, ~1% of them join, so the unfiltered
product cost 1.00 s per call on London (Paris 0.78 s, Tokyo 0.39 s) against 19 ms with
it — and `build` runs twice per load, once for the grid fill and once for the mesh
build. The decision itself is deliberately NOT unified: the grid
blocks curb tiles by a directional outward probe, the mesh subtracts band polygons, and
both survive the "primary's nominal 16 m swallows its parallel sidewalk" trap in their
own way — a shared point-coverage test reproduces neither (with slack it opens the
composite's outer barrier, without slack it crumbles the curb chain into dashes). This
was analysed and rejected during the footprint extraction; re-attempting it means going
through the curb pin tests (`navmesh/tests.rs`) and the parity tests.

## Rendering

- **Merged meshes** (`map/meshing.rs` + `map/spawn.rs`, road layers in `map/roads.rs`,
  rail layers in `map/rail.rs`, the tram layer in `map/tram.rs`, building layers in
  `map/buildings/`) — **one merged `Mesh2d` per layer** (ground, parks, water, waterways,
  sidewalks, alleys, roads, rail layers, tram, building layers, walls): `MeshBuilder`
  triangulates polygons via `earcutr` (holes supported, degenerate contours skipped +
  counted) and emits per-vertex colors. Facade, shadow, casing, rail and wall layers go
  over a single white `ColorMaterial`; every layer carrying a roof (`building_roofs`, and
  in 2.5D `building_extruded`, walls included) over the `RoofMaterial` of **Roof
  material**; the **surfaces** — ground, area fills, water, road and
  alley fills, sidewalks, the tree-row band — over the `SurfaceMaterial` below. ~7000
  buildings cost a handful of entities. Trees stay individual entities (see
  `references/trees.md`).
- **Surface material** (`map/surface.rs`, shader `assets/shaders/surface.wgsl`, a
  `Material2d` with its own vertex + fragment stage) — procedural texture without a single
  asset: the vertex colour is the base, and the fragment multiplies in noise sampled by
  **world position**, so two overlapping ribbons of one layer get the same pixel (the
  junction trick survives). Per `SurfaceKind` (`Ground | Park | Wood | Grass | Sand |
  Water | Street | Alley | Sidewalk`) a `SurfaceParams` uniform: **mottle** (four
  octaves of value noise from `mottle_scale` down to an eighth of it, with a per-channel
  `tint` shift so a lawn goes yellow-green ↔ blue-green, not just light ↔ dark), **grain**
  (three octaves from `grain_scale` down to a quarter), **speckle** (a thresholded noise
  field → sparse dark dots, grass tufts and undergrowth on Park/Grass/Wood), **drift** (the
  mottle slides with `globals.time` — only Water), and the **markings** block (Street —
  a bridge deck is the same kind and carries its street's lines; a footbridge in the same
  mesh has no markings code and stays bare). The zoom rule is one function,
  `visible(wavelength, px)` with `px = fwidth(world position)`: an octave shorter than 1.5 px
  contributes nothing and one longer than 4 px contributes fully — the noise is centred, so
  a faded octave shifts no brightness, and zooming out makes a surface smoother, never
  brighter or shimmering. Materials are built once (`SurfaceMaterials`, `Startup`) and
  shared by every city; `SurfaceStyle::texture` (section **Surfaces**, `ui/surfaces.rs`,
  persisted) rewrites the `intensity` uniform of each and rebuilds nothing.
  **This is the map's only grain.** A second one — `map/grain.rs`, a map-sized sprite
  tiled with a 256 px seamless simplex texture — was written in the building-look
  worktree and dropped when that branch was merged on top of this material; the commits
  describing it are history, not a missing file. Don't reintroduce a sprite grain: two
  noise fields over the same fills read as dirt, and the shader one already fades by
  pixel size.
  The material demands the **`Ribbon` vertex attribute** (`meshing::ATTRIBUTE_RIBBON`,
  `[across, to-break, half width, markings code]` in metres: *to-break* is the signed
  distance to the nearest marking break, the code is `Markings::encode`, `lanes·2 +
  oneway`, 0 for none) and a mesh gets it only from `MeshBuilder::with_surface_coords()`;
  `push_ribbon` / `push_ribbon_broken` fill it from the ribbon frame (quads: ±half width;
  join fans: the outer side; round caps: the projection onto the normal, with *to-break*
  extrapolated past the node along the last quad's slope), polygons get zeros. It costs
  16 bytes per vertex, which is why building, crown and overlay meshes are built without
  it. Where the breaks come from and why the mesher inserts a vertex at every kink of
  *to-break* is under **Markings → Breaks** below.
- **Rims** (`map/spawn.rs::push_area` over `MeshBuilder::push_inset_band`) — each area
  polygon is followed, in the same builder, by a gradient band along its outer ring and
  along every hole: `edge` colour on the contour, the fill colour at the far edge. Water
  gets a lighter **shore** (`WATER_RIM`, 3 m), park / wood / grass / sand an edge a few
  percent darker than the fill (`*_RIM`, 2–3 m; the wood's the widest and darkest — shade
  under the canopy edge). The far edge is built from `miter_offsets` on the ring, with the
  side chosen by the ring's signed area (`outside` flips it for holes, whose band lies in
  the fill). Two guards: the band width is clamped to `RIM_THICKNESS_SHARE` (0.6) of the
  ring's thickness `|area| / perimeter` — a strip's thickness is half its width, so a
  2 m rim on a 1.5 m median never pokes out onto the road — and nothing under
  `MIN_RIM_WIDTH` (0.2 m) is pushed at all. Holes take the width the outer ring settled
  on. No z-slot: opaque 2D meshes test depth with `GreaterEqual`, so within one mesh the
  band pushed after the fill wins.
- **Sidewalks** (`map/roads.rs`, `sidewalks` layer at `Z_SIDEWALK` 1.2, `SurfaceKind::
  Sidewalk`, light concrete `SIDEWALK_COLOR` over the asphalt-grey `ROAD_COLOR` — the
  brightness step between them is what reads as the kerb) — a **carriageway**
  (`is_carriageway`: `RoadClass::Street`, width ≥ `STREET_MIN_WIDTH` 8 m, so `service`
  drives get none, and never a `passage`) gets a band `width + 2 · sidewalk_width` (22 % of
  the width, 1.2–3 m per side). It sits under every road ribbon for the casing reason: a
  crossing street's fill covers it and the sidewalk ends at the junction the way a real
  one does. A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `spawn_roads` `continue`s into `bridge_casings`
  + `bridges` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`. It would hang a metre or three past the deck edge over the
  water, and the deck already has its own kerb: `push_bridge_curb`, drawn unconditionally.
  A `footway` mapped alongside draws over it as an alley — beige on grey, and
  tolerated. The road fill went from osm-carto white to asphalt grey together with the
  markings: a white line on white is invisible, and on grey the street grid also stops
  merging with the courtyards.
- **Markings** — the lane lines of a street are **not geometry**: `push_dashes` would
  alias and crawl at `Msaa::Off` (a 0.15 m line is under a pixel at the start zoom). The
  street (and bridge deck) fill is built with surface coords and
  `set_markings(Some(Markings { lanes, oneway }))` for every carriageway with two or more
  lanes — the code `lanes·2 + oneway` travels in the fourth `Ribbon` component. **Lane
  count** (`roads::lane_count`): the `lanes` tag, else the width default (two-way: a lane
  pair per 7 m → 8/10 m two, 12/16 m four; one-way: a lane per 4.5 m → 8 m one, 16 m
  three), never more than the width allows at `MIN_LANE_WIDTH` 2.5 m, and **always one on
  a roundabout** (a one-lane ring has no lines, and cutting a two-lane ring's line at every
  entry looks worse than none). The shader puts a line on every interior lane boundary
  (`round((across + half width) / lane width)`), dashed 3 m / 3 m, except the **axis** of a
  two-way road with 4+ lanes, which is solid; a one-way road has no axis, and an odd
  `lanes` on a two-way road (three: two one way, one the other) has none either — all its
  boundaries are dashed. Line width `MARKING_WIDTH` 0.15 m but never under 1.3 px,
  anti-aliased over ±0.7 px; faded out when a lane is under ~10 px on screen (`lane width
  / px`). `MARKING_COLOR` is white at 0.85 alpha over the asphalt grey.
  **Breaks** — the second `Ribbon` component is the signed distance to the nearest
  **marking break** (`meshing::Break { at, reach }`, passed as `RibbonBreaks::At` to
  `push_ribbon_broken`): negative inside a gap, so the line fades at the gap edge
  (`smoothstep(0, 1)`) and the dash phase is anchored there *with a gap first*, so no dash
  ever pokes into a junction. The mesher projects each break's world point onto its own
  (smoothed, merged) path — that is why a break is a point, not an arclength: the smoothed
  centreline and the OSM node may disagree — merges overlapping gaps, and inserts a vertex
  at every kink of the distance function (each gap centre and the crossover between
  neighbours) so the GPU's linear interpolation is exact. The round cap extrapolates the
  last quad's slope: a listed end goes negative, an unlisted one keeps growing — that is
  what carries the line through a seam between two ways of one road. A ribbon with no
  breaks at all gets `along + FAR_FROM_BREAKS` (dashes need a growing coordinate). The
  `Square` join falls back to `push_polyline`, which knows only the ends.
- **Ribbon** — a constant-width band along a polyline (`MeshBuilder::push_ribbon`), how
  every road, alley and kremlin wall is drawn. Two knobs, both named after their SVG /
  Mapnik counterparts: **join** (`Miter` — bisector offsets capped by `MITER_LIMIT`;
  `Round` — an arc of radius half-width on the **outer** side of the bend, the side where
  butt-ended segment quads leave a gap) and **cap** (`Butt` — cut at the last point;
  `Round` — a half-disc half-a-width past it). Arc tessellation is driven by
  `ARC_TOLERANCE` (5 cm of chord sagitta), so a 16 m primary gets more chords than a
  3.5 m footway; the **same tolerance decides whether a join fan is emitted at all** —
  a bend is skipped only when `half_width · turn` is under it. An angle threshold was
  tried first and was wrong: 5° on an alley still leaves a 15 cm slit, plainly visible
  as a pale cut across the road when zoomed in.
- **Junctions** (`map/roads/junctions.rs`) — computed for the markings only, and from
  **shared nodes**, not segment intersections: Overpass `out geom` gives no node ids, but
  a node shared by two ways projects to the same `Vec2` on both (quantised to 5 cm to be
  safe). Participants are carriageways (`is_carriageway`). A node with two or more
  distinct participants is a junction and hands every road there a `Break` of reach
  `half the widest other road + JUNCTION_MARGIN` (1 m); a node where exactly two ways
  *end* is a continuation (one road split by a tag change) and no break; a lone way end
  is a dead end (reach 0); a closed way's seam is not an end. Segment intersection was
  rejected on purpose: a bridge over a street shares no node with it and must not break
  either line, which the node rule cannot do wrong. A service drive or a footway joining a
  street is not a participant and leaves the street's line whole. Count and time are in
  the `road meshing:` log line (`junctions N`).
  **Junction geometry is still not computed**: roads are independent polylines drawn
  overlapping in one opaque layer, and `Round` caps are what makes a junction *look*
  joined — the caps of the ways meeting at a node overlap into a rounded blob, exactly
  how osm-carto gets its smooth junctions (`stroke-linejoin: round` + `stroke-linecap:
  round`). The fill order is **narrow first, wide last** (`spawn_roads` sorts by width), so
  the main road's fill and its gapped line lie over the side street's cap. This is why the
  road layer must stay opaque with a world-position colour: transparency or a per-way tint
  would expose every crossing.
- **RoadStyle** (resource, BRP-writable, persisted; section `ui/roads.rs` below Buildings)
  — how road ribbons are drawn; any change reruns `rebuild_roads` (despawn
  `RoadLayerTag` layers, respawn from the unchanged `MapData`). Five independent knobs —
  **sidewalks** and **markings** (both on by default) are described above, the three
  older ones:
  - **join** — `Square` (the historical `push_polyline`: an independent quad per segment
    with *both ends* extended by half a width; no joins at all, which is what produced
    the notches on bends and the wedges at junctions), `Miter`, `Round` (default).
  - **smoothing** — Chaikin corner-cutting on the centerline, `Off` (default) / 1 / 2
    iterations. Only bends over `MIN_SMOOTH_ANGLE` (10°) are cut and the cut length is
    clamped to the road width, so the drawn line never leaves the OSM data by more than
    a road width. `passage` roads are never smoothed — their endpoints are pinned to
    building outline vertices that `arch_openings` looks the arch up by. Off by default
    because OSM itself keeps its corners sharp.
  - **casing** — a darker outline, its own merged layer at `Z_ALLEY_CASING` (1.4) /
    `Z_ROAD_CASING` (1.9), width `+2·casing_width` (8% of the road, 0.3–1 m). Both fills
    (1.5 / 2.0) sit above both casings on purpose: otherwise a casing would cut every
    crossing in half. Off by default.

  Smoothing works on a **copy** — `RoadLine::points` and `width` are load-bearing for the
  navmesh (`bridge`/`passage` carves), arches, tree planting and the entrance generator,
  and none of them may shift because the drawing changed. `smooth_path` is shared with
  the rail layers; `centerline` is the road wrapper that adds the `passage` pin.
- **Bridge layers** (`map/roads.rs`, same `RoadLayerTag`) — a road with `bridge` leaves
  its class layers for the pair `bridge_casings` (`Z_BRIDGE_CASING` 2.1) + `bridges`
  (`Z_BRIDGE` 2.2): a light concrete **curb** (`BRIDGE_CURB_COLOR` 0.80, 12% of the width
  clamped 0.8–2 m) under the fill in the class color — a parapet over the asphalt-grey
  deck. The 2GIS look — the curb bands along both deck edges are what makes a bridge read
  as a bridge, so the curb draws **always**, independent of `RoadStyle::casing`, and is
  thicker than a casing and lighter where the casing is darker, so the two never blend.
  Curb caps are always `Butt` (`push_bridge_curb`) — the deck ends
  in a square cut; a `Round` half-disc or the `Square` end-extension would poke a curb
  tongue past the bridge end. The deck sits above `Z_ROAD` so an overpass covers the
  street it crosses, and below `Z_RAIL` so a track on the bridge stays visible; curbs
  below fills for the casing reason (a junction of two bridge ways is never cut by a
  curb band). Street and footbridge fills share one mesh — bridge-over-bridge overlap
  is push order, rare enough not to warrant four layers. Rails carry no bridge flag —
  rail bridges are out of scope. The curb is not just paint: the navmesh blocks the
  same bands (see **Bridge curbs are impassable** in the navigation-deep skill).
- **Rail layers** (`map/rail.rs`, its own module with its own zoom LOD, like the tram's;
  it left `map/roads.rs` when it stopped being a line style) — the **track**, not a map
  symbol: a ballast prism, ties across it and two steel rails on the gauge. Three merged
  meshes, `RailLayerTag`, all above `Z_ROAD` (2) so a track lies on its street: ballast
  `Z_RAIL` (2.4), ties `Z_RAIL_TIE` (2.5), steel `Z_RAIL_STEEL` (2.55) — the far-bucket
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
    hatching; from 0.65 m/px the ties go too and osm-carto's white dash pattern comes
    back, because a bare grey band reads as another street. `min_bed` floors the ballast
    width on the last two buckets — 5 m is a pixel at city scale, and the track would
    vanish before the roads it crosses. The numbers are derived from the screen size at
    the **worst** (far) edge of each bucket, and they hold on **every** parsed bed width
    (5 / 4 / 3.5 m), not only the mainline's: tie spacing never below ~6 px, no mark below
    ~1 px. The second number that must hold across buckets is the **tie duty cycle**,
    ~40% (the real 0.26 m in 0.65) — measured live: at 31% the ties stop being a texture,
    become sparse marks, and the two white rails outweigh them into a ladder. Both,
    plus the one-way progression (detail only ever falls away) and that ties and dashes
    never coexist, are pinned by `rail/tests.rs`.
  - **What a bucket costs** (Tula, 69 km of non-tram track inside the map, measured on
    an M1 Max from the `rail meshing:` log line): bucket 4 45 k verts / 1 ms, bucket 2
    131 k / 3 ms, bucket 1 298 k / 9 ms, bucket 0 673 k / 23 ms — a one-off hitch on the
    threshold crossing, and ~23 MB of buffer at the deepest bucket. Frame rate stays
    vsync-capped at 60 there. The 23 ms is what a denser tie step would multiply, so
    treat bucket 0's 65 cm as the floor.
  - **`RailZoomBucket`** is `ZoomBucket<RailLods>` — the same machinery as the tram's
    (see **Zoom buckets** below), a separate resource because the tables' thresholds
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
- **Parked cars** (`map/cars.rs`) — the second most recognisable thing on an aerial photo
  after the roofs themselves: a street with not one car on it reads as a drawing whatever
  it is painted. A row goes along **both sides of every `RoadClass::Street` at least
  `PARKED_MIN_WIDTH` (9 m) wide** that is neither a bridge nor an arch — 4.4 × 1.8 m bodies
  at `CAR_PITCH` 6 m, offset `CURB_GAP` + half a body in from the kerb, with `OCCUPANCY`
  45 % of the places taken (a solid row from junction to junction looks like a dealership)
  and `END_MARGIN` 8 m clear of each end, where the junction is. Colours are a ten-slot
  palette in the shares a photo shows. Every car casts a shadow through the same
  `map::shadow_length_scale()` as the buildings, and the mesh draws **all shadows first,
  then all bodies** — otherwise a car's shadow lands on top of the neighbour drawn before
  it. The layer is one merged **blended** mesh (the shadow is translucent, the body is not)
  at `Z_CAR` 2.7, above the tram and the rails (a car parks on the asphalt over the tracks)
  and below the portal stain.
  - **Decoration, and deliberately so**: cars touch neither the navmesh nor the simulation
    and pawns walk through them. A parked row along every street would otherwise eat the
    pavements the entire crowd walks on.
  - **Its own zoom bucket** (`CarLods` / `CarZoomBucket`, `CAR_MAX_ZOOM` 0.8 m/px, so a
    4.4 m car is never under ~6 px): past the threshold the layer is not drawn at all, which
    is cheaper than any LOD of the drawing itself. Seeded per street (its first point,
    like doors and roofs), so the row is the same across rebuilds.
  - Tula: **5665 cars, 45 k verts** — next to the building layer's 925 k, free.
- **Tram** (`map/tram.rs`, its own module so a zoom-LOD step never rebuilds the
  road/rail meshes) — a thin blue line with perpendicular cross ties, the
  Yandex/2GIS convention; `TRAM_COLOR` is the only thing separating the two (Yandex dark
  red, 2GIS blue) and we take 2GIS's blue, since red on this map already means kremlin
  wall. Line and ties share one colour, so both go in one mesh (`TramLayerTag`, `Z_TRAM`
  2.6 — above the rail steel at crossings, name `tram`) — self-overlap costs nothing,
  and there is no white dash layer for a tram. The tie primitive is
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
  is `ZoomBucket<TramLods>` (see **Zoom buckets** below), so `rebuild_tram` fires only
  on an actual threshold crossing, never per frame. The tram centerline is smoothed with a
  fixed `TRAM_SMOOTH_WIDTH` (1.2 m) clamp rather than the bucket's line width, so the
  path itself is identical across buckets and LOD switches don't wiggle the track.
  `RailLine::width` from parse is ignored for trams.
- **Zoom buckets** (`map/zoom.rs`) — the one mechanism behind both zoom LODs. Each
  layer keeps its own table (`RAIL_LODS`, `TRAM_LODS`) and names it with a marker type
  implementing `ZoomLods` (`RailLods`, `TramLods`, empty enums handing over the
  `max_zoom`s). `ZoomBucket<T>` is the resource with the current index for that table;
  `for_zoom` is the single selection rule (first bucket whose bound is above the zoom,
  a zoom on the bound goes up — `zoom/tests.rs`). Two generic systems:
  `update_zoom_bucket::<T>` each Update via `set_if_neq`, so the `retuned`-gated
  `rebuild_*` runs only on a threshold crossing; and `seed_zoom_bucket::<T>` on world
  entry — in the `WorldInitSet::Spawn` chain right before the layer's first `rebuild_*`
  and after `camera::place_camera_on_world_ready` (hence that system is `pub(crate)`),
  written without change detection so the build that follows stays the only one. The
  seed exists because the camera opens on the saved zoom (`position: save` is the
  default) or, on a city switch, on `START_ZOOM`, and neither matches the index the
  resource happened to hold; the first build then went by the wrong bucket and the
  first Update redid it — for rails up to 23 ms / 673 k vertices. The bucket is not
  persisted (the camera dictates it), and `Default` is the farthest, cheapest bucket.
- **BuildingHeightMode** (resource, BRP-writable, persisted) — how a building's OSM
  height is drawn; any change reruns `rebuild_buildings` (despawn `BuildingLayerTag`
  layers, respawn from the unchanged `MapData::buildings`). The section lives in
  `ui/buildings.rs`, in the Map tab below Trees and Tree rows, one cycling row. A building
  with no height uses the default of its `BuildingUse` in every mode (15 m, a house 6 m,
  a garage 3 m — see **Building use**). Modes:
  - **Facade** (the historical look) — pseudo-3D: the footprint polygon shifted
    straight down in a darker color at z just below the roof (`Z_FACADE` 4.9), visible
    only along south edges. Shift = height × `FACADE_SCALE` (0.2) clamped to 1.5–12 m, so
    a five-storey block keeps the historical 3 m band. Facades sit *under* every roof on
    purpose — that is what stops a tower's wide band from painting over its low neighbour.
  - **Shadows** — facade band plus a long shadow: one translucent merged mesh at
    `Z_BUILDING_SHADOW` (4.5 — *below* every building layer, so a neighbour's roof or
    wall masks the shadow and a shadow never lands on a same-height roof: the cheap
    stand-in for real height-aware casting; still above the portal and corpses, which
    are outdoors and in shadow by meaning). Per contiguous **silhouette chain** of the
    footprint (edges whose outward normal faces the 30° light — `map/mod.rs::SHADOW_DIR`,
    one source for building and tree shadows alike) one swept
    polygon `[chain, chain + offset reversed]`, offset = height ×
    **`map::shadow_length_scale()`** clamped to 3–45 m. Not per-edge quads — on staircase
    facades those overlapped
    along the shadow axis and the translucency stacked into stripes; a chain sweep
    cannot self-intersect (a silhouette edge's perp-step equals `outward·d > 0`, so the
    chain is monotone along the shadow perpendicular). All sweeps of the map are then
    merged by a boolean union (`i_overlay`, NonZero — sweeps are winding-normalized
    first, in `push_contour`) into disjoint shapes-with-holes, so the translucent layer
    never overlaps itself anywhere: no double-darkening between wings of one block or
    neighbouring buildings (unlike tree shadows, which still stack).
    - **The sun has an elevation now.** `SUN_ELEVATION_DEG` (59°, Tula's summer noon)
      gives `shadow_length_scale() = cot 59° = 0.601` — numerically the old bare 0.6, but
      derived, and the elevation is what you change to move it. The azimuth stays
      `SHADOW_DIR`; together they are the one light every tone on the map answers to
      (`shade_by_light`, the roof shader's `light` uniform, the clutter shadows).
    - **Only the sweeps go into the union.** A **contact skirt** was tried and taken back
      out: every footprint also entered the union expanded outward by 1.1 m, to bind the
      building to the ground with a dark rim the way a photo does and to darken the ground
      under a lifted 2.5D roof. It reads as a grubby outline around every house, the
      courtyards it had to subtract (reversed contours, or the skirt filled them) cut
      other buildings' cast shadows out of every yard, and it cost half again as many
      vertices. Don't reintroduce it without solving the yards.
    - **Soft edge** — after the union each contour gets a **`PENUMBRA_WIDTH` (1 m) band
      fading to zero alpha**: outward from the outer ring, and into the gap from a hole
      (`push_inset_band` picks the side from the ring's own signed area, so a hole needs
      `outside: false` — with `true` the band lands on the already-filled body and rings
      the gap with double darkness instead of blurring it). This is not the
      geometric penumbra: the sun's angular size would give ~10 cm at these lengths. It is
      what actually softens a shadow edge on a photograph — the frame's resolution and the
      sky's fill light — so the width is chosen by look (1 m is 2–10 screen px at the zooms
      where shadows read). Bands of neighbouring shapes may overlap, but both fade to
      zero, so the doubling is weaker than the shadow itself.
    - **The shadow layer rebuilds on its own schedule.** It carries `BuildingShadowTag`
      rather than `BuildingLayerTag`, and `rebuild_buildings` despawns it only when the
      **height mode** changed (`mode.is_changed()`): it does not depend on the roof-clutter
      zoom bucket, and it is the single most expensive thing here — 90 ms of a 116 ms
      build on Tula. `BuildingPlan { mode,
      bucket, shadows }` is how that decision reaches `spawn_buildings` (and what keeps it
      at seven arguments).
  - **Shadows+tint** — shadows plus a roof color ramp: `t = sqrt(height / 60 m)` mixes
    the roof toward `ROOF_TALL_COLOR` (0.20, a near-black neutral — it must be darker in
    luminance than every palette colour, saturated tile included, or the ramp inverts),
    max 0.3 (a 27 m block: 0.55 → 0.48); no-height buildings and the Kremlin keep their
    material colour.
  - **2.5D (Extrusion)** — watabou-style: roof lifted by `lift = height ×
    EXTRUDE_SCALE (0.35) × (EXTRUDE_SKEW, 1)`, the vertical part clamped to 2.5–30 m.
    The lift is **oblique** (`EXTRUDE_SKEW` 0.4 — 0.4 m right per metre up): a
    strictly vertical lift showed one south wall and a block read as a roof with a dark
    band under it; the skew exposes two wall families, and with the light the shadows
    already use (`SHADOW_DIR`, from the upper left) the west wall is lit and the south
    wall shaded — three tones, which is what makes a box read as a box in watabou and
    in 2GIS's 3D mode. The visible walls are the edges facing *against* the lift
    (`silhouette_edges(outer, -extrusion_dir())`), courtyard walls the hole edges facing
    *along* it; each wall's tone comes from `layers.rs::wall_colors` through the shared
    `buildings/mod.rs::shade_by_light` — the facade colour mixed toward white by
    `outward · −SHADOW_DIR × WALL_LIT_MIX` (0.18) when lit, toward black by
    `WALL_SHADED_MIX` (0.22) when not, plus the `WALL_TOP_LIGHTEN` vertical gradient.
    All building tone mixing — the palette, the `roof_color` ramp, walls, slopes — is
    done in sRGB, so the wall and slope constants compare directly. No facade band, no
    shadows. Depth is painter's algorithm *inside one
    mesh*: buildings sorted by `Lean::depth`, far first (index-buffer order is raster
    order), so a south-western building correctly overlays its north-eastern neighbour.
    `extrusion_lift` is the one door to that vector — the extrusion layer, the arch patch
    in the shadows and anything that wants to put a marker on the *drawn* building rather
    than its real outline all go through it. Known limits: units y-sort against
    flat z=5 and can draw over a tall roof they are "behind"; kremlin wall polylines
    (z 5.1) draw over nearby lifted roofs.
    - **`Lean` is a per-building value**, not a global function: direction,
      `lift(drawn)`, `ridge(rise)` and the painter's key `depth(centre)` all come from it,
      and `layers.rs` builds one per building. It holds **metres of displacement per drawn
      metre as a vector** rather than a `(direction, length)` pair on purpose: the vector
      is exactly `(0.4, 1)`, and a round trip through `normalize` × `length` moves it by
      one ulp — enough to break the `EXTRUDE_RANGE` clamp assertion in the arch tests.
      One oblique skew for the whole city is what a **satellite** frame gives: 5 km of
      city from 500 km up spans fractions of a degree, so the parallax is constant across
      it (a true orthomosaic has none at all).
    - **A radial lean was tried and taken back out.** A frame from an *aircraft* leans
      every building away from the nadir, harder the farther it stands, and that fan is
      the most recognisable signature of aerial photography. It does not work here,
      because the nadir would be the centre of the **map** and not of the frame: with a
      camera that pans, the fan is only visible around the map centre, and everywhere else
      it is an ordinary skew pointing somewhere else. Making the nadir follow the camera
      is what would be honest, and it is out of reach while the lean is baked into the
      merged mesh — a rebuild is tens of milliseconds against a 16 ms frame. The way back
      in is the vertex shader: the lean is linear in height, so basis + height as vertex
      attributes and the nadir as a uniform would cost nothing per frame. Three things
      would have to move with it — the painter's sort (to a depth test writing the same
      key), the choice of visible walls (build every edge, collapse the invisible ones in
      the shader) and the arch patch in the shadow layer.
    - **The sun does not follow the lean.** The lean is the camera, the shadow is the
      light, and their being independent is itself a realism cue. `wall_colors` takes the
      true outward normal; the lean only picks which walls are visible.
  - **2.5D+shadows+tint (ExtrusionShadowsTint, the default)** — everything at once: the extruded
    geometry with the tint ramp on lifted roofs plus the long-shadow layer.
  - **Hip roofs** (`buildings/roofs.rs::hip_roof`) — the other pitched roof, and the one
    that does not need a rectangle. The outline is pushed inward by `HIP_INSET` (2.2 m,
    clamped to `HIP_INSET_SHARE` 0.38 of the outline's own thickness, exactly as a rim is
    clamped — a narrow shed would otherwise turn its slopes inside out) using the same
    `miter_offsets`; each outline edge becomes a slope quad from the eave to its shifted
    pair, and what is left inside is the **ridge plane**, drawn `RIDGE_LIGHTEN` lighter
    because it faces straight up. Slope tone is `shade_by_light` on the edge's outward
    normal, so a hip roof shows four or more tones instead of one. On a convex house that
    construction *is* a hip roof; on an L-shaped one it is a hip roof with a flat top —
    which is what the photo shows anyway, and what a straight skeleton would have cost an
    order of magnitude more to produce. `roofing` is the single door: gable when the seed
    says so and the rectangle fits, hip otherwise, flat when the building is not in the
    pitched cohort at all — and `HIPPED_SHARE` (4 in 10) is the split among houses that
    could take either. **The L-shaped case is why this exists**: those houses were flat
    among pitched neighbours, which is the one thing an aerial photo of a private sector
    never shows.
  - **Gable roofs** (`buildings/roofs.rs`) — in every mode, a building that
    `is_gabled` (`BuildingUse::House` of any size, or `Other` with a footprint under
    `SMALL_FOOTPRINT_MAX` 250 m², never with a courtyard, never `AreaKind::Kremlin` —
    its towers and gates keep the flat roof, like they keep their colour) gets two
    slopes instead of a
    flat roof. The ridge runs along the long axis of the footprint's minimum-area
    bounding rectangle (`min_area_rect`, edge directions of the ring tried as
    orientations — no hull needed at 4–20 vertices); the roof is drawn over that
    rectangle, not the outline (real roofs overhang), which is why it is only applied when
    the outline fills the rectangle to `RECT_FILL_MIN` 0.85 — an L-shaped house stays
    flat rather than wearing a rectangle. Slope tone: base roof colour mixed toward
    white/black by the slope's plan normal against `−SHADOW_DIR` (`SLOPE_LIT_MIX` 0.14 /
    `SLOPE_SHADED_MIX` 0.11, through the same `shade_by_light` helper as the walls, in
    sRGB), softer than walls. In 2.5D the ridge is lifted a further
    `ridge_rise(width) = min(width/2 × ROOF_PITCH 0.8, ROOF_RISE_MAX 5 m)` real metres
    through `ridge_lift` (same `EXTRUDE_SCALE`, no `EXTRUDE_RANGE` clamp), and the two
    gable triangles are drawn on the visible end walls with the wall's top colour before
    the slopes. In flat modes the ridge lift is zero and the two shades are all that
    remains. Verified on Tula's western private sector: red-brown two-storey houses with a
    visible ridge, the L-shaped ones flat.
- **Inferred storeys** (`buildings/heights.rs`) — the height of the 69 % of Tula (95 % of
  Tokyo) that OSM leaves untagged. It used to be three numbers — house 6 m, garage 3 m,
  everything else 15 m — and the measurement of that is its own argument: the city's
  height distribution came out **median 15 m, p90 15 m**, i.e. no distribution at all.
  Every shadow was the same length and every 2.5D lift the same size.
  - **The footprint decides**, the way it does for an eye reading an aerial photo, and
    only then the use: a **section** is a long thin box (≥ `SLAB_MIN_LENGTH` 35 m by
    ≤ `SLAB_MAX_WIDTH` 18 m) at 5 / 9 / 12 storeys with the weights a Russian city has
    (half of them five); a **tower** is compact and large (≥ `TOWER_FOOTPRINT_MIN` 500 m²,
    sides within `TOWER_MAX_RATIO` 1.7) and is mostly nine; a **low** building is
    ≤ `LOW_FOOTPRINT_MAX` 300 m² at 2–4; everything else large is 2–5 — that last branch
    is the most populated one and being generous with it is what made the first version
    come out skyscraping (p90 27 m before the tower table was halved).
  - **Some uses are measured in metres, not storeys**: an industrial hall or a store has
    one tall span, a church has one storey to the cornice, a garage is one box and gets no
    spread at all (a row of garage boxes on a photo is all one height).
  - **The slot inside a group is the building's own seed** — the same
    `material::building_seed` that picks the roofing material, so heights survive a mode
    switch, a zoom rebuild and a restart, and two identical footprints in different places
    still come out different. **A tag always wins**; the inference runs only where
    `PolyArea::height` is `None`.
  - **`height_mix`** puts the result in the `building meshing:` log line
    (`31% tagged, median 8 m, p90 15 m, max 82 m`) — a height distribution is exactly the
    thing a screenshot cannot show, and that line is how this was tuned.
- **Roof material** (`buildings/material.rs`, shader `assets/shaders/roof.wgsl`) — what
  the roof is *covered with*. The goal is the aerial photo: from above, a roof is a
  **material** first (rolled bitumen with its seams and repair patches, gravel ballast,
  standing-seam metal, corrugated sheet, tile, PVC membrane) and a colour second, and the
  old per-`BuildingUse` roof colour plus a ±3 % tint by list index made a district read as
  a dress pattern.
  - **The pick** (`roof_look`) — a **seed hashed from the building's first vertex**
    (three xorshift-multiply rounds over the centimetre coordinates; the door generator
    is seeded from the same point, so both survive a rebuild and neither depends on the
    order of `MapData::buildings`) chooses a slot in a **ten-slot table per
    `BuildingUse`** — ten slots so a table reads as percentages: `House` is 5 tile / 2
    seam / 2 corrugated / 1 bitumen, `Apartments` 7 bitumen, `Industrial` 5 corrugated,
    and so on. `Church` is always seam metal (its green is now green *metal*), the
    Kremlin too (and keeps `KREMLIN_ROOF_COLOR`), and `Other` — half the city — splits by
    footprint at the same `SMALL_FOOTPRINT_MAX` 250 m² the gable rule uses: a small box is
    a private house, a big one a block.
  - **The colour** comes from that material's own palette (3–7 plausible shades, picked
    by another slice of the same seed, then ±3 % of value). **Value is calibrated against
    the aerial photo at the city zoom**, where the texture has faded and the base colour
    is all that is left: a panel block's bitumen is a mid grey there (~0.55 sRGB, palette
    0.50–0.63), not the 0.38–0.50 of the first version, which on the map's ~0.9 ground
    read as dirty-dark boxes — fine close up, unreal zoomed out. The spread differs by
    what the roof covers: the **flat roofs of blocks** (bitumen, gravel, membrane) are
    **tight in value and wide in hue** — neighbouring panel blocks on a photo differ in
    shade, not in brightness, and a wide value spread there reads as confetti — while the
    **private sector** (tile, and the seam / corrugated it also draws) is **bright and
    many-hued**: red and brown metal tile, green, blue, silver and dark slate stand fence to
    fence, and a palette of greys made the whole quarter one colour.
  - **The texture** is a `Material2d` in the shape of `SurfaceMaterial`: one material for
    the whole app (`RoofMaterialHandle`, built at `Startup`), a `RoofParams` uniform
    (`light` = `-SHADOW_DIR`, `intensity` = `RoofStyle::texture`) and a per-vertex
    **`Roof` attribute** (`meshing::ATTRIBUTE_ROOF`, `[long axis x, y, material code,
    seed]`). All four numbers are constant over a building, so the attribute is a
    *builder state* (`MeshBuilder::set_roof`), like the markings code, not an argument of
    every `push_*`; the fragment reads it `@interpolate(flat)`. Code `0` means **not a
    roof** — walls, gables and parapets ride in the same mesh (2.5D is one painter's-order
    layer) and come out with their vertex colour untouched.
  - **What the shader draws**, by world position rotated into the building's long axis
    (`min_area_rect`'s first edge), phase-shifted by the seed so neighbours' seams do not
    line up: bitumen — 0.95 m roll seams, scattered repair patches (as many as the roof's
    **age** below), ponding stains; gravel —
    strong fine grain and bright specks; seam metal — a lit rib and its shadow every
    0.62 m; corrugated — a 0.30 m wave plus 1.05 m sheet laps; tile — 0.32 m rows with a
    shadow line and per-tile jitter; membrane — 2 m sheet seams. **Rolls and tile rows run
    along the ridge (constant `v`), seams and corrugation run down the slope (constant
    `u`)** — the first version had the corrugation along the ridge, and water would have
    run along the rib rather than down it. Rib lighting is scaled by
    `|axis · light|`, so ribs pointing at the sun neither highlight nor shade. Every
    octave and every stripe grid fades by `visible(wavelength, px)`, the `surface.wgsl`
    rule, so nothing moirés when zoomed out; at the city zoom the texture is simply gone
    and only the material's colour is left. The noise helpers are a **copy** of
    `surface.wgsl`'s — there is no shader library in the project yet, and importing one
    for four functions costs more than the copy.
  - **A cell grid places a feature, it never *is* the feature** (`repair_patch`). The
    bitumen patch started as `hash21(floor(uv / 6))` — a shade of its own for every 6 m
    cell — and that is not repair patches but a **chequerboard across the whole roof**:
    the edge is hard, the grid is aligned to the walls (`uv` is the building frame), and
    ±4 % of brightness on a big dark roof is plainly visible at the working zoom. The
    rule the fix follows, and the same one the asphalt wear already followed: only a
    minority of cells carry the feature (the `share` argument), and inside its cell the
    feature is smaller than the cell and jittered, so two neighbours never meet at a cell
    boundary. Placement stays a grid (cheap, no extra octaves); the pattern does not.
  - **Roof age** (`roof.wgsl::roof_age`) — one number per building in [0, 1), **hashed from
    the same seed** the texture phase rides on, and with a fixed patch share that was the
    missing half of the patch fix: a minority of cells carried a patch, but *the same*
    minority on every bitumen roof, so the whole district read as re-roofed and repaired in
    one year. Age drives the patch share (`PATCH_SHARE_NEW` 0.04 → `PATCH_SHARE_OLD` 0.28,
    mixed by `age²` — age is uniform, repairs are not; with ⟨age²⟩ = 1/3 the mean share
    lands at 0.12, half the old fixed 0.22, so a patched roof is an event against clean
    neighbours instead of the district's baseline), the ponding amount (0.07 → 0.13) and,
    on **every** material, the common fade-and-dirt amplitude (×0.75 → ×1.35) — the last
    one is what makes the age read as age rather than as a patch counter.
    It gets **no vertex attribute of its own**:
    the seed is already a per-building random number the shader hashes several ways
    (`seed·17`, `seed·11`, `seed·37`), the correlation between two patterns of one building
    is not visible, and a fifth float would cost four bytes on every vertex of the building
    layer. The consequence for the gallery: its `Seed` knob rolls the age too — that is how
    a new roof is compared against an old one there.
  - **Parapet** (`layers.rs::push_parapet`) — a soft flat roof (bitumen / gravel /
    membrane) gets a 0.7 m inset band along its ring and every courtyard
    ring, lit by `shade_by_light` like a wall (0.24 / 0.20): bright on the sunny edges,
    dark on the shaded ones. Tile, seam and corrugated get none — they end in an eave, not
    a parapet. *Soft* is a property of the material, so the rule lives on it —
    **`RoofKind::has_parapet`** (`material.rs`), not on the layer that happens to draw the
    band. The band carries **no** roof frame: a roll seam crossing a concrete coping
    would read as a crack. `MeshBuilder::push_inset_band_with` (the per-edge-colour
    sibling of `push_inset_band`) exists for exactly this.
  - **One call lays every flat roof** — `layers.rs::push_flat_roof(builder, look, outer,
    holes, color)`: set the roof frame, fill the contour with its courtyards, add the
    parapet if the material has one. Both callers use it — the flat modes with the real
    contour, 2.5D with the contour already lifted onto the walls — and it is `pub` because
    the `roof_gallery` example builds its houses with it. Slopes stay outside it: a gable
    roof is computed by the caller, which needs the same `GableRoof` for the gables it
    draws *with the walls*, before the roof.
  - **A roof is now darker than the walls under it.** That inverts the old "roof lighter
    than wall, so the wall reads as a band under it" rule, which is retired: on a photo a
    dark bitumen roof over light panel walls is the normal relation, and the 2.5D box is
    held together by the two visible walls' own tones. The relation is "as a rule": the
    light materials (membrane, gravel, silver tile) sit above their walls, as they do on
    the photo. The height ramp (`ShadowsTint`) keeps its direction only while
    `ROOF_TALL_COLOR` is darker than every base — 0.71 with the near-white per-use roofs,
    0.34 with the first dark bitumen, and now a near-black 0.20 with a short mix (0.3): a
    mid-value neutral would *lighten* a saturated red or green tile, and the old 0.34 / 0.7
    pair would have pulled a nine-storey bitumen roof from 0.55 back to 0.45, undoing the
    brightening at the very zoom it was made for.
  - **`RoofStyle::texture`** (section Buildings, row `Roof texture`, persisted, BRP) is
    the amplitude of all of it; 0 leaves flat material colours. It rewrites the uniform
    only, so dragging the slider rebuilds nothing.
  - **The gallery** — `cargo run --example roof_gallery` (`examples/demos/roof_gallery/`,
    the shape of `tree_gallery`): seven blocks — six materials plus the church palette —
    each with **a house per palette colour**, sized from a 30 m block down to an 8 m shed,
    so the two things a still picture cannot say are said at once: the palette's spread
    (tight in value, wide in hue) and that the texture is in **metres** and does not scale
    with the house. Knobs are what the game reads off the building itself — long axis,
    phase seed, courtyard — plus `RoofStyle::texture`; the readout at the bottom right
    prints metres per pixel and the two wavelengths `visible()` cuts at, since at city
    zoom "the texture is gone" and "the texture is off" look alike. Under the knobs the
    panel lists the shader's own **tuning constants** (patch cell, patch size, the two
    share ends), parsed out of `roof.wgsl` itself by `constants.rs` (`include_str!`, lines
    of the form `const NAME: f32 = …;`) rather than mirrored as Rust numbers — a mirror
    would drift on the first edit and the gallery would then lie about exactly what it is
    opened for. They are text, not knobs: the numbers live in the shader. What the gallery may
    **not** do is roll its own quad: houses go through `push_flat_roof`, the parapet
    marker in a block's caption comes from `RoofKind::has_parapet`. It picks material and
    colour directly (`RoofLook::new`) instead of through `roof_look`, because the seed
    cannot reach every combination — a membrane never lands on a private house — and it
    drops the ±3 % seeded jitter so the hex printed under a house is the constant in
    `material.rs`.
- **Roof clutter** (`buildings/clutter.rs`) — the boxes that stand on the roof, and the
  second half of the same argument: a photographed roof is never empty, and it is the
  small equipment with its short shadows that reads as "photo" rather than "fill".
  - **What stands where follows the material**, not the building use, because the
    material already encodes the kind of building: a soft flat roof (bitumen / gravel /
    membrane) over 400 m² gets a **lift penthouse** (5 × 3.5 m, 3 m tall; a second one
    over 1600 m²), every flat roof gets **ventilation shafts** (1.1 m cubes, one per
    300 m², at most ten), a corrugated shed over 700 m² gets one or two **skylight
    ribbons** (2.2 m wide, 62 % of the length, along the long axis; the second only if
    the building is at least 22 m across), commercial and public buildings get
    **air-conditioning units** in addition, and a gabled roof gets a **chimney** on the
    ridge — `ridge_of` reads the ridge back out of `GableRoof`'s first slope, since the
    two far corners of `[eave, eave, ridge, ridge]` are exactly it.
  - **Placement** is a Park–Miller LCG (the crown generator's, copied — `map/trees`
    keeps its own `pub(super)`) seeded from the roof material's building seed, so the
    equipment survives a mode switch, a zoom-bucket rebuild and a restart in the same
    place. Positions are rolled in the building's own frame (long axis × its
    perpendicular, extent projected from the outline — no second `min_area_rect`),
    inset by `EDGE_MARGIN` 1.6 m or 18 % of the short side, whichever is smaller. Every
    candidate is accepted only if **all four corners are inside the footprint**
    (`point_in_area`, holes included) — the frame is a rectangle and an L-shaped block
    is not — and a miss is retried `PLACE_TRIES` (6) times before the item is dropped.
    One try was the first version and it was wrong: a 5 × 3.5 m penthouse fits a 12 m
    slab only in a narrow band, so most blocks came out with no penthouse at all.
  - **Shadows are opaque.** The building layer draws without blending, so a translucent
    shadow would not mix; each item's shadow is the roof colour mixed 30 % toward black,
    swept the item's own height × `SHADOW_LENGTH_SCALE` along `SHADOW_DIR`. The sweep is
    the two silhouette edges plus the offset rectangle — the same construction the
    buildings' own shadows use, and for the same reason: for a convex base that *is* the
    missing part of the union, and no convex hull has to be built. (The first version
    swept all four edges; two of them were always inside the union.)
  - **Zoom.** The clutter is the only thing zoom changes about the building layer, and
    it cannot be hidden without rebuilding, since it lives in the same merged mesh as
    the houses (painter's order is per building: walls, roof, parapet, then its own
    clutter). So buildings got a zoom bucket of their own — `BuildingLods` /
    `BuildingZoomBucket`, two steps at `ROOF_CLUTTER_MAX_ZOOM` (0.5 m/px), seeded on
    world entry before `spawn_map` and rebuilt on a threshold crossing through the same
    `retuned` gate the height mode uses (one registration with `or_else`, deliberately:
    two registrations of `rebuild_buildings` in one schedule could both fire in one
    frame and spawn the layer twice).
  - **What it costs** (Tula, 7643 buildings, 2.5D+shadows+tint, M1 Max): 603 018 verts /
    69 ms with clutter against 279 186 / 58 ms without — one hitch on the threshold
    crossing, in the same class as the rail layer's deepest bucket (673 k / 23 ms). Most
    of it is the shafts: every flat roof gets at least one, and a shaft is 6 quads.
- **Arch rendering** (`buildings/arches.rs::arch_openings` + `push_wall_with_openings`) —
  a building `passage` (арка) is also cut out of the *drawn* building. The opening is a
  rectangle **in the wall plane**, found from the passage's **endpoints**, not by segment
  intersection: an OSM arch is typically mapped outline-vertex to outline-vertex (Tula
  way 485488257), so the road lies inside the building and only its ends touch walls. At
  such a shared vertex the opening is laid across **every** wall within `ARCH_WALL_TIE`
  (0.5 m) of the nearest one — clamped to a single edge it came out half a road wide.
  Width = the road's own width × |sin| of the entry angle, trimmed to the edge; height =
  `ARCH_HEIGHT` (6 real metres — 3 is physical but read as 2 px on a tall slab) as a
  fraction of *that building's* height, `band × 6/height`, never taller than the wall.
  Openings are looked up only on the walls the mode actually draws — `arch_openings`
  takes a `facing` (2.5D: `-extrusion_dir()`, so south **and** west walls; facade band:
  south only) — because `push_wall_with_openings` matches an opening to its wall by exact
  edge endpoints, and a wall family the lookup does not know about would draw solid. In
  2.5D the wall is **really cut** (side pieces + a lintel above,
  `push_wall_with_openings`) so the layers beneath — the road running through, the
  ground — show through the hole, and `shadow_builder` patches the opening with
  `SHADOW_COLOR` (the lintel shades it; without the patch the hole glows). In facade
  modes the facade band is one earcut polygon, so the opening is *painted* in shaded
  ground colour instead — a stated compromise. What the passage does to the navmesh is
  in the navigation-deep skill.
