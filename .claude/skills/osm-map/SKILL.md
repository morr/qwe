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
  `amenity=parking` (way+rel),
  `barrier=city_wall`. The bbox is `MAP_SIZE` around the selected
  `City`'s geo center. `QUERY_VERSION` is **9** (v3 added `entrance` nodes, v4 `railway`,
  v5 `natural=tree_row`, v6 `natural=tree` nodes, v7 linear `waterway`, v8 `landuse`
  blocks, v9 `amenity=parking`).
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
  Industrial | Parking`. **Park** is the
  light base fill; **Wood** (`natural=wood` / `landuse=forest`) are the darker stands
  *inside* it and the **only** areas that carry trees; **Grass** (lawns, meadows) and
  **Sand** (beaches) also sit above the park fill, lighter green / sandy. Everything
  but Wood stays open ground — that is what makes the open half of a park read as a
  field, the way it does on OSM. **Residential** (`landuse=residential`) and
  **Industrial** (`landuse=industrial|garages`) are the *blocks* — `MapData::landuse`,
  **two** merged layers between the ground mesh and the parks — `landuse_works` at
  `Z_LANDUSE` (0.25) in the earth colour, `landuse_yards` a hair above it at
  `Z_LANDUSE_YARD` (0.26) in the muted green (see **The yard** below). The hair is not
  cosmetic: they are two opaque meshes with different materials, the two tags do overlap
  in OSM, and at an equal depth the phase queue would decide which fill wins rather than
  the map. `area_kind` tries them **last**: any green tag on the
  same polygon wins. They touch neither the navmesh nor tree planting. Tula v8: 264
  residential + 35 industrial/garages in the bbox (the audit table), 294 of them
  reach `MapData::landuse`; `commercial`/`retail` are not requested.
  **Parking** (`amenity=parking`) is the third such non-green class — `MapData::parking`,
  asphalt with stalls painted on it (see **Parking** below). It is tried after the greens
  and **before** the landuse blocks, but the branch is reached only for a polygon that is
  not a building at all: a multi-storey car park carries `building` *and*
  `amenity=parking`, and it must stay a building — and a lot whose asphalt is **not on the
  ground** is dropped by `parking=*` instead (`HIDDEN_PARKING`: `underground`,
  `multi-storey`, `rooftop`) — an underground car park is its own outline under a yard or
  a park, with no `building` on it, and drawing it striped would put asphalt on the lawn.
  Tula v9: 172 in the bbox, 170 reach `MapData::parking`.
  **Pitch** (`leisure=pitch|track|playground|sports_centre|stadium`) is the fourth —
  `MapData::pitches`, a surface plus markings (see **Pitches** below). It is tried after
  parking and before the landuse blocks, but **after `park`/`garden`**: a park with a
  pitch drawn on it stays a park, and the pitch arrives as its own way. Alone among the
  area kinds it **carries a payload**, `PitchKind`, because the sport decides both the
  colour and the marking and nothing else in the model has one — a field on `PolyArea`
  (the `building_use` pattern) would be meaningless for every other area kind and would
  touch all 33 literal constructions in the tests. Tula v10: 128 grounds in the bbox — 59
  playgrounds, 48 pitches, 13 tracks, 7 sports centres, 1 stadium — of which 118 reach
  `MapData::pitches`; the other 10 carry `building=*` too and stay buildings.
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
  layers** below). Three more fields feed the **markings** and the parked cars: `oneway`
  (`oneway=yes|1|true|-1`; `reversible`/`alternating` are not one-way), `roundabout`
  (`junction=roundabout|circular`, implies `oneway`) and `lanes: Option<u8>` (the `lanes`
  tag through `parse_measure`, floored, 1–8; `2;3` reads as 2, `0` and `12` as no tag).
  Coverage per city is in `references/osm-coverage.md` — Tula has `lanes` on 97 % of its
  streets ≥ 8 m, the European cities on about half.
  **The direction of a one-way way is load-bearing now**, and it did not use to be: the
  cars park on one side of it, the right-hand kerb, so `oneway=-1` — "the traffic runs
  against the order of the points" — is **normalized at parse by reversing the way**
  (`parse_way`, `is_oneway_backward`). One notion of "which way this street runs" instead
  of two, and nothing downstream has to remember the tag. It is not free: a street's seed
  is `seed::seed_from_point` of its **first** point, so such a way is seeded differently
  and its row stands
  elsewhere than it did — deterministic and reproducible, just not identical. Only the
  highway branch reverses; the rail and waterway branches of the same way ran earlier and
  keep the original order.
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
  LiDAR import) or else `building:levels` + `roof:levels` × `settings::STOREY_HEIGHT` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — and it is the majority
  everywhere but New York, so what fills it in matters: see **Inferred storeys** under
  Rendering. Coverage is logged per city on load (`N buildings (M with height)`).
- **Building use** (`parse/tags.rs::building_use`) — `BuildingUse: House | Apartments |
  Commercial | Industrial | Garage | GarageBlock | Church | Public | Other`, the class that
  picks two material tables — the **cladding** (`buildings/material.rs::wall_kind_of`) and
  the **roofing material** (`::kind_of`); neither colour comes from the class itself, both
  come from the chosen material's own palette (**Roof material** under Rendering, bullet
  **The pick**). Two sources in order: `building=*` when the value says something
  (`house`, `apartments`, `garages`, `church`,
  `school`, …), else `amenity=*` on the same outline (`school`, `hospital`, `police`,
  `place_of_worship`, …) — a school or a hospital in OSM is almost always `building=yes`
  + `amenity=…`. Anything outside the vocabulary is `Other`, and `Other` is read by shape
  rather than left flat: the wall takes the apartment-block table (height having spoken
  first — under `LOW_RISE_STOREYS` it is the low-rise one whatever the tag), the roof
  takes the private-sector table for a small footprint and the apartment-block one above
  it. The vocabulary covers what a city carries by the hundreds, not the OSM wiki.
  Tula: `yes` 4004 of 7465, `house` 2249, `apartments` 744, commercial/retail/office 165,
  garage(s) 74, industrial 31, church 17. **`garages` and `garage` are two different
  classes**, and that is not pedantry: the plural is how OSM maps a *whole cooperative*
  as one outline (Tula 43 of them, the largest 255 × 51 m), and drawing it as one shed is
  what made the ГСК look like a hangar — see **the garage row** under Rendering. The
  singular, together with `carport`/`shed`/`barn`/`roof`, stays one box.
  The Kremlin (`AreaKind::Kremlin`) keeps its
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

- **One RNG and one point seed for the whole of `map/*`** (`map/seed.rs`) — everything a
  layer scatters must survive a rebuild: a zoom-bucket crossing, a height-mode switch, a
  restart. So the layers share two primitives instead of copying them. **`Lcg`** is the
  Park–Miller (Lehmer) generator of `Village.js` — `seed = 48271·seed mod 2³¹−1`, plus
  `range`, `gauss3` (a bell on (0,1)) and `bell4` (a bell on (−1,1)); it started in the
  crown generator and was lifted out when the parked cars became its third caller.
  **`seed_from_point(Vec2)`** is the seed itself: the object's **own reference point** in
  centimetres — the first vertex of a footprint, the first point of a street — through three
  mixing rounds, never the object's index in the extract, which a re-parse is free to move.
  Callers: `buildings::material::building_seed` (the roof material, the roof shape and, since
  the height inference, the storeys), `buildings::clutter`, `trees::crown`, `cars`.
  The **parse** stage is deliberately not on it — doors (`osm/entrances/`) and tree planting
  (`osm/planting.rs`) run on `rng::lcg_seeded_by`, a different point-seeded LCG, and rewiring
  them would move every door and every tree in every city.
- **Measuring the layer build** — `cargo run --example map_meshing -- [city slug]`
  (`examples/bench/map_meshing.rs`) prints vertices and milliseconds per layer for every
  height mode × clutter bucket, plus the car layer in the same `LayerCost` rows with its
  junction breaks split out (milliseconds too — that is what the bench is for), straight
  from the Overpass cache
  with **no window and no GPU**. That is the point of it: on macOS an invisible or
  minimised window is put under App Nap, and a build that reports 116 ms on an awake screen
  reports five seconds on a locked one — the `building meshing:` log line is only
  trustworthy while the screen is awake. `map::measure_layers` / `map::measure_cars` are
  the entry points; `measure_layers` takes the two decisions the measurement actually reads
  (height mode + roof clutter), not a `BuildingPlan` — its `shadows` field would have been
  ignored — and they call exactly the builders `spawn_buildings` calls. **Absolute
  numbers still depend on the machine's power state** (with the display asleep everything
  is 2–3× slower), so compare runs, not runs against the log.
  **Shadows are measured at the default sun.** The sweep length and direction come from the
  process global of `map/sun.rs`, which in the app only `apply_sun` writes; the bench has no
  app, so it sets the sun itself — `map::apply_sun_style(SunStyle::default())` — and prints
  the azimuth/elevation in its header line. The elevation drives `sun_stretch`, i.e. the
  sweep length and the union's area, so a run that did not state its sun would not be
  comparable with the next one; the live app builds with the sun from `settings.toml`, which
  is a second reason a log line and a run are not comparable.
  **What it covers is the building layers and the cars, and nothing else yet.** The road,
  rail, tram and surface/tree layers are still measurable only from the app log
  (`road meshing:`, `rail meshing:`, `tram meshing:`) — the same log line App Nap lies
  about; there is no `measure_roads` / `measure_rails` / `measure_tram` / `measure_surface`,
  and adding one is the way to extend the bench when a road-style or surface comparison
  needs the same treatment.
- **Merged meshes** (`map/meshing.rs` + `map/spawn.rs`, road layers in `map/roads.rs`,
  rail layers in `map/rail.rs`, the tram layer in `map/tram.rs`, building layers in
  `map/buildings/`) — **one merged `Mesh2d` per layer** (ground, parks, water, waterways,
  sidewalks, alleys, roads, rail layers, tram, building layers, walls): `MeshBuilder`
  triangulates polygons via `earcutr` (holes supported, degenerate contours skipped +
  counted) and emits per-vertex colors. Facade, shadow, casing, rail and wall layers go
  over a single white `ColorMaterial`; every layer carrying a roof (`building_roofs`, and
  in 2.5D `building_extruded`, walls included) over the `RoofMaterial` of **Roof
  material**; the **surfaces** — ground, area fills, water, road and
  alley fills, sidewalks, parking asphalt (as `SurfaceKind::Street` — it *is* asphalt),
  the tree-row band — over the `SurfaceMaterial` below. The parking **markings** are the
  exception that proves the rule: paint over asphalt, so that layer stays on the flat
  `ColorMaterial`. ~7000
  buildings cost a handful of entities. Trees stay individual entities (see
  `references/trees.md`).
- **Surface material** (`map/surface.rs`, shader `assets/shaders/surface.wgsl`, a
  `Material2d` with its own vertex + fragment stage) — procedural texture without a single
  asset: the vertex colour is the base, and the fragment multiplies in noise sampled by
  **world position**, so two overlapping ribbons of one layer get the same pixel (the
  junction trick survives). Per `SurfaceKind` (`Ground | Yard | Park | Wood | Grass | Sand |
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
  brighter or shimmering. That rule is **not this shader's own**: the shared library
  `assets/shaders/noise.wgsl` holds all six helpers — `hash21`, `value_noise`, `visible`,
  `fbm4` (four octaves), `fbm3` (three) and `stripes` — and each shader imports by path
  only the names it calls (`#import "shaders/noise.wgsl"::{value_noise, visible, fbm3,
  fbm4}` here — the hash reaches it inside `value_noise`; `roof.wgsl` takes its own
  subset, `crown.wgsl` only `fbm3`), so a rule that must not drift is kept in one place.
  **Only what more than one shader calls moves there**: `dash_distance` (lane dashes)
  stays in `surface.wgsl` and `band` (the drive between garage rows) in `roof.wgsl`,
  each with a single consumer — a helper calling the library is not itself a reason to
  move it into the library.
  Materials are built once (`SurfaceMaterials`, `Startup`) and
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
    Don't reintroduce a straight door-to-road strip. `model::put_in_cells` / `grid_cell`
    stay — they are the door generator's own (`osm/entrances/index.rs`), extracted while
    this layer existed and its only surviving trace.
- **Pitches** (`map/pitch.rs`) — sports and children's grounds, the thing a courtyard is
  actually *made of* on an aerial photo. One surface layer at `Z_PITCH` 0.75 and one
  markings layer at 0.76, the parking pair's shape exactly: the paint is flat
  `ColorMaterial`, the surface carries `SurfaceKind::Ground` (a neutral mottle — a
  football field must not get the street's asphalt grain).
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
- **Parking** (`map/parking.rs`) — an `amenity=parking` area is drawn as asphalt
  (`Z_PARKING` 0.8, the `parking` surface layer) with the **stalls painted on it**
  (`Z_PARKING_LINES` 0.81). The markings go in a **flat-material** layer of their own,
  not through `SurfaceMaterial`: the procedural asphalt grain belongs under the paint,
  not on it, and a 12 cm line is the one thing on this map that must stay pure white.
  - **The layout is computed once per world load** into `ParkingLayout` (a resource,
    filled by `spawn_map` from `stalls(area)` per lot), and the paint and the cars both
    read it — two independent layouts would put a car across its own line, and recomputing
    it on every rebuild of the car layer was work the sun slider paid for by the frame.
    Rows run along the
    **long axis of `min_area_rect`**, the axis a real lot is striped along: `STALL_WIDTH`
    2.6 × `STALL_DEPTH` 5.2 m, two rows back to back, then an `AISLE` of 6 m, and
    `EDGE_MARGIN` 1.2 m in from the edge.
  - **A stall survives only if all four of its corners are inside the outline**
    (`fits`, the same test the roof clutter uses for its boxes) — an L-shaped lot gets
    nothing in the notch, and the OBB rows do not have to match the outline.
  - **`MIN_AREA` 120 m²** — under that the lot gets no paint at all. A yard for four cars
    is not striped in reality, and stripes on a 6 × 10 m patch read as a texture bug.
    **The stalls themselves stay**: `MIN_AREA` gates `push_markings` only, `fill_lots`
    reads `stalls()` unfiltered, so a small yard keeps its cars — on unmarked asphalt.
  - The paint is drawn as the **border between stalls** (one bar to the left of each
    stall, neighbours coinciding), not as a rectangle per stall: that is what a lot looks
    like, and it is cheaper than finding each stall's neighbour.
  - Tula: **170 lots** (172 in the bbox, less the one that is a building and the one
    `parking=multi-storey`). Parking touches neither the navmesh nor tree planting, like the
    landuse blocks.
- **Asphalt wear** (`surface.wgsl`, `SurfaceParams::wear`, on `SurfaceKind::Street` only)
  — two effects that keep a road from being one flat tone, both in the **ribbon frame**
  so they follow the lane rather than the compass:
  - **wheel ruts** — a polished band `RUT_OFFSET` 0.85 m either side of each lane's
    middle (a car's track is 1.5 m), `RUT_SIGMA` 0.32 m wide, +7.5 %. The lane is found
    from `fract` of `(across + half_width) / lane_width`, so **every** lane gets its own
    pair without knowing how many there are;
  - **kerb dirt** — 7 % darker over the outer `EDGE_DIRT_REACH` 0.7 m, where the sand
    and grit collect.

  **Repair patches were the third and are gone.** A 6 m cell of the *world* grid was
  hashed and, above a threshold, darkened whole; the `smoothstep` softened the hash, not
  the shape, so the patch's edge was exactly the cell boundary. What that draws is a
  chequerboard oriented to the compass: right angles, two chosen neighbours fused into one
  block, and a staircase of squares across any street that is not axis-aligned. It is
  literally the construction **A cell grid places a feature, it never *is* the feature**
  was written against after the same mistake on the roofs, and the claim standing here
  that wear "already follows" that rule was false. Taken out by the author's call on the
  picture. The way back in, if it is wanted: the cell must be the *lane's* cell (the
  ribbon frame, so the grid turns with the road), and inside it the patch must be a
  rectangle smaller than the cell with a jittered centre and size, a crisp saw-cut edge
  and a seam — not a fill of the cell.

  Both remaining effects fade by `visible(...)` like the rest of the surface texture — the
  ruts by their lane pitch, the kerb dirt by twice its reach
  (1.4 m), so a band under half a pixel does not flicker along the road edge.

  **And both fade out in a junction gap**, by the very `smoothstep(0, 1, to_break)` the
  lane dashes use — the second component of `ATTRIBUTE_RIBBON`, negative inside a gap.
  Reported from a screenshot of a four-way crossing: the roads are independent overlapping
  ribbons, so each was drawing its own wear across the other. The kerb dirt was the
  louder half — a dark band along a street's edge carried straight over the crossing
  street's asphalt, where there is no kerb — and the ruts the subtler, two lanes' polished
  bands meeting at right angles in the middle of the junction. Neither is a thing that
  happens: traffic fans out over a crossing and polishes nothing, and the grit collects
  where the kerb is. The gate costs one `smoothstep` on the amplitude that scales all of
  it, so the fade is shared. The block is
  gated on `lanes >= 2`, and `lanes` is decoded from the same `ATTRIBUTE_RIBBON.w` the
  markings ride on: `roads::road_markings` fills it only for a carriageway of two lanes or
  more, and only while `RoadStyle.markings` is on. So **wear reaches exactly the roads the
  lane lines reach** — a one-lane street gets none, turning Markings off turns wear off
  with it, and an areal fill of the same `Street` material carries no ribbon and stays
  flat — the **parking lot** among them, which shares the material and would otherwise
  have grown ruts across its stalls. The gate is `>= 2` rather than `>= 1` because
  `Markings::encode` never carries a single lane: `>= 1` read as a wider rule than the
  code could ever deliver.
- **Parked cars** (`map/cars/`, the layer in `mod.rs` and the drawing in `body.rs`) — the second most recognisable thing on an aerial photo
  after the roofs themselves: a street with not one car on it reads as a drawing whatever
  it is painted. A row goes along **both sides of every carriageway** — `roads::is_carriageway`,
  the very predicate that decides where a sidewalk and lane markings go, opened up for this
  — minus a bridge (nobody parks on one) and a roundabout (you drive it, you don't park on
  it), both excluded by `parkable` rather than by the predicate, which markings still need
  them in. The threshold that stood here before was `road.width >= 9 m`, and it was reading
  the wrong thing: `RoadLine::width` is a **drawing constant of the class**
  (`primary` 16, `tertiary` 10, `residential` 8, `service` 5), never a measured street
  width, so 9 m meant "not an arterial" and put every car on the avenues — while an aerial
  photo shows the housing blocks parked solid. `STREET_MIN_WIDTH` (8 m) lets
  `residential`/`unclassified`/`living_street` in and keeps `service` out, which is exactly
  the line wanted. On an 8 m street a sedan's row sits `8/2 − CURB_GAP − 1.8/2 = 2.6 m` off
  the axis (a van's own 1.95 m width narrows that to 2.53 m — the offset is per-body, not a
  constant, same as the length below), leaving 3.4 m of carriageway between the two rows for
  a sedan — a yard, and it is pinned by
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
    place within `Break::reach + JUNCTION_CLEARANCE` (5 m) of a break is dropped. A dead
    end arrives as a break of reach 0, so the clearance empties the same 5 m there; two way
    ends meeting are not a break at all, which is the half of the defect that tore the row.
  - **A one-way carriageway gets one row, on its right.** Traffic here is right-hand, so on
    each half of a divided avenue the kerb is on the right and the median on the left; two
    rows would put a column of cars down the median, and in Tula 145 of 218 `primary` ways
    are exactly such halves. The same rule is right for an ordinary one-way lane. `across`
    points left, so the right-hand side is `-1`; the direction it is right of is the way's
    own point order, which parse has already normalized (see **RoadLine** above).
  - **Not cached, and that is measured, not assumed**: on Tula `marking_breaks` is the
    `breaks` row's 1 ms against the 7 ms the layer costs at its far detail step and the 18 at
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
    `body::sweep`) — the hull of the outline and the outline moved by the light, i.e. the
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
    zero, exactly `buildings::layers::penumbra`: hard where the shadow meets the car,
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
    effect is one merged mesh — and `rebuild_cars` is gated on
    `retuned::<CarZoomBucket>.or_else(retuned::<CarStyle>).or_else(retuned::<RoadStyle>)`,
    one registration, since two in one schedule could both fire in a frame and spawn the
    layer twice; `RoadStyle` is in there because the row is walked along the **smoothed**
    centreline the ribbon is drawn from (`smooth_path(road.points, road.width,
    style.smoothing)`, never the raw OSM points), so Smoothing moves the cars with the
    asphalt. The invisible case
    goes through the same early return as the far zoom bucket: despawn the old layer, build
    no new one, so no second path can forget the despawn.
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
    machine, so compare runs against runs): **22 069 cars along the kerbs** — the bench
    does not fill the lots, and they add **5 934** more in the app — and per detail step
    **1 456 k verts / 27 ms** (Full), **485 k / 11 ms** (Silhouette), **220 k / 5 ms**
    (Block). **Those are the mesh rows
    alone**; the two steps in front of them do not depend on the detail and are measured
    once each — `breaks` 1 ms (`marking_breaks`) and `parking` 2 ms (`park_cars`) — so a
    rebuild is 30 ms at the near step and 8 ms at the far one.
    **The swept shadow is what most of the near step's growth bought** (971 k / 15 ms
    before it, on the same machine and the same run of the buildings' 785 k / 79 ms): the
    sweep's hull is two vertices *cheaper* than the translated copy was — which is why
    Silhouette went *down* from 529 k — and the whole of the +485 k is the soft edge, six
    band quads per car at the near step alone. Keep `parking` on its own
    timer: while it sat inside the mesh timer the layer's milliseconds compared with
    nothing — not with the `cars:` line the app logs (which has always included it), and
    not with the older single-row runs. Next to the building layer
    (785 k verts, 78 ms — the same run, see **What it costs** under Roof clutter) and above
    the rail layer's deepest bucket (673 k, 23 ms) — still a
    layer built once per rebuild that costs nothing per frame.
    - **The body outline goes through `MeshBuilder::push_convex`, not `push_polygon`**, and
      that is most of those milliseconds: `push_polygon` calls `earcutr`, which on a
      12-vertex contour costs several times the laying-out itself and runs twice per car
      (body and shadow), 22 k cars over. The fan is correct because the outline is convex by
      construction, and `cars/body.rs::the_outline_is_convex` — an inline `mod tests`, there
      is no `body/tests.rs` — is what keeps it that way.
      Measured: Full 40 → 15 ms, Silhouette 35 → 9 ms, vertices unchanged.
    - **The lots are outside every one of those numbers.** Measured on the avenues-only
      run that predates the current kerb rule: 5665 → 11 599 cars and 45 k → 93 k verts
      once the lots were filled, i.e. ~5 900 cars and ~48 k verts on Tula, at whatever the
      detail step of the moment costs per car.
  - **The lots are filled by the same pass** (`fill_lots`): every stall from
    `parking::stalls`, `LOT_OCCUPANCY` **55 %** of them taken — a lot is fuller than a
    kerb, and an empty one next to a painted grid reads as unfinished. Seeded per lot
    (`lot_seed`, its first point) exactly like a street. That share is a constant, not
    `CarStyle::occupancy`: the slider is about the ragged kerb row, and the half-empty
    lot is a different observation.
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
    footprint (edges whose outward normal faces the light — `map::shadow_dir()`,
    one source for building and tree shadows alike) one swept
    polygon `[chain, chain + offset reversed]`, offset = height ×
    **`map::shadow_length_scale()`** clamped to `SHADOW_LENGTH_RANGE` (3–45 m **at the
    default sun**, both ends multiplied by `map::sun_stretch()`). Not per-edge quads — on staircase
    facades those overlapped
    along the shadow axis and the translucency stacked into stripes; a chain sweep
    cannot self-intersect (a silhouette edge's perp-step equals `outward·d > 0`, so the
    chain is monotone along the shadow perpendicular). All sweeps of the map are then
    merged by a boolean union (`i_overlay`, NonZero — sweeps are winding-normalized
    first, in `push_contour`) into disjoint shapes-with-holes, so the translucent layer
    never overlaps itself anywhere: no double-darkening between wings of one block or
    neighbouring buildings (unlike tree shadows, which still stack).
    - **The sun is two knobs, not a constant** (`map/sun.rs`, `SunStyle`, section *Sun*).
      Azimuth (default 300°) and elevation (default 59°, Tula's summer noon) replace what
      used to be `SHADOW_DIR` and `SUN_ELEVATION_DEG`; the default of each is numerically
      the old constant, so nothing about the default picture moved. Everything reads it
      through the process global — `shadow_dir()`, `sun_light()`,
      `shadow_length_scale() = cot(elevation)` and `sun_stretch()`, the last being that
      cotangent *relative to the default*, by which every length that was calibrated at
      59° is multiplied. Those lengths are the point: the clamp above and the crowns'
      shadow heights are numbers picked by eye at `cot 59° = 0.6`, and a fixed 45 m ceiling
      would have made every building above 12 m cast the same shadow at 15°.
      **A shadow length written without `sun_stretch()` is a bug in the making** — it will
      look right at the default and wrong at both ends of the slider.
    - **The clamp on the elevation is on the read** (`SunStyle::elevation()`, the
      `PolymeshDebug::radius()` precedent): the setting is persisted, and `elevation = 0`
      from a hand-edited `settings.toml` or a BRP write gives an infinite cotangent, i.e.
      NaN geometry in `earcutr` and `i_overlay`.
    - **The slider is not the map's sun.** `SunStyle` is what the knob writes; **`SunOnMap`**
      is what the map is built with, and `settle_sun` (`PreUpdate`) copies one into the
      other after `SUN_SETTLE` (0.35 s) of quiet. Every rebuild and the prefs write are
      gated on `retuned::<SunOnMap>`, never on `SunStyle` — one division of the azimuth
      scale is a full building rebuild with its shadow union — most of it the union — plus
      15 k crowns plus the car layer, and there are seventy divisions on the scale. (The
      numbers that stood here, 77–86 ms on Tula with 47–56 of it the union, were read off the
      `building meshing:` line through the slider itself on an M1 Max: absolute milliseconds
      from the app are only comparable with each other, since App Nap decides them —
      `examples/bench/map_meshing` is what re-measures the same build offline.)
    - **The global is seeded in `Startup`, before `init_roof_material`.** The roof material
      is built once for the whole app and `apply_sun` runs in `PreUpdate`, which in the
      first `Main` pass is *after* `Startup`: without the seed the `light` uniform would
      keep the compile-time azimuth for the life of the process, and seam-metal ribs would
      be lit from 300° while the walls and shadows used the saved angle.
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
    - **The band is tapered, and that is what keeps the contact skirt from coming back.**
      A shadow meets the thing that casts it **hard** — there is no penumbra at the wall —
      and blurs as it runs away from it. In the union that difference is readable locally,
      because the body always lies on the `map::shadow_dir()` side of a contact edge: the
      band's own direction points *against* the light there, *along* it on the far edge, and
      across it on a lateral one. Hence `layers.rs::penumbra(direction) =
      direction·shadow_dir()` (clamped at zero) as the per-vertex share of the width, fed to
      `MeshBuilder::push_inset_band_tapered` — zero at the contact, the full metre at the
      far edge, and along a lateral side a growth from nothing at the building's corner to
      full width at the far end, which is what a real penumbra does. Untapered (the state
      the sun-shadows branch merged in) the metre also ran along the contact contour, and
      the mitred band at every convex corner of the silhouette chain left a soft dark blot
      a metre across **on the sunlit ground** — a stepped facade came out as a row of
      them, and the building read as outlined by the very contact skirt that had just
      been removed. A zero-width vertex degenerates its quad into a triangle (that *is*
      the hard edge); an edge zero at both ends is not emitted at all, so the taper also
      takes vertices off the most expensive layer here.
    - **The shadow layer rebuilds on its own schedule.** It carries `BuildingShadowTag`
      rather than `BuildingLayerTag`, and `rebuild_buildings` despawns it only when the
      **height mode or the sun** changed (`mode.is_changed() || sun.is_changed()`, the
      latter `Res<SunOnMap>`): it does not depend on the roof-clutter
      zoom bucket, and it is the single most expensive thing here — **60–80 % of the whole
      building build** on Tula (41–46 ms of a 52–70 ms build, by height mode and clutter
      bucket, in `examples/bench/map_meshing` runs on the `dev` profile — the share is lower
      the more the facades themselves cost;
      the 90 ms of 116 that used to stand here came off the `building meshing:` line in
      the app, where the power state sets the scale, so take the share, not the
      milliseconds). `BuildingPlan { mode,
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
    already use (`map::shadow_dir()`, from the upper left at the default sun) the west wall is lit and the south
    wall shaded — three tones, which is what makes a box read as a box in watabou and
    in 2GIS's 3D mode. The visible walls are the edges facing *against* the lift
    (`silhouette_edges(outer, -Lean::dir())`), courtyard walls the hole edges facing
    *along* it; each wall's tone comes from `layers.rs::wall_colors` through the shared
    `buildings/mod.rs::shade_by_light` — the facade colour mixed toward white by
    `outward · map::sun_light() × WALL_LIT_MIX` (0.18) when lit, toward black by
    `WALL_SHADED_MIX` (0.22) when not, plus the `WALL_TOP_LIGHTEN` vertical gradient.
    All building tone mixing — the palette, the `roof_color` ramp, walls, slopes — is
    done in sRGB, so the wall and slope constants compare directly. No facade band, no
    shadows. Depth is painter's algorithm *inside one mesh* (index-buffer order is raster
    order), and the order comes from **`buildings/order.rs::draw_order`** — a **pairwise**
    relation, not one key per building. In the lift's own basis (`u` across it, `v` along)
    a footprint is a set of `v`-segments at every `u`, and the building is drawn from a
    segment's start to its end plus the lift; at a shared screen point the nearer fragment
    is the one standing higher, and "higher" is `v` minus the nearest footprint edge below
    it. So neighbours whose drawn boxes meet (a 48 m grid finds them) are probed at up to
    16 `u` inside the overlap, whoever covers the other on more of the shared front gets
    an edge, and the graph is laid out by an iterative DFS seeded with
    `Lean::depth(centre)` — that key still decides every pair the relation left alone,
    which is most of the city. **One key per building cannot do this**: an L-shaped house
    has one wing in front of its neighbour and the other behind it, and its centre (or top
    point, or corner, or depth − lift) is dragged by the wrong wing — that is how a
    five-storey wall came to lie over a nine-storey roof (Tula 493864392 over 493864388,
    pinned by `the_order_puts_the_leaning_neighbour_over_the_wing_it_covers`). What the
    pairs still cannot do is a **cycle** — interlocked shapes each covering the other on
    their own piece; the back edge is ignored, so the order stays complete and someone in
    the cycle is still drawn wrong. The cure for that is a real depth test (`z` from the
    vertex's height), which the merged mesh does not have.
    - **The same question is asked once more inside a house** — `order.rs::wall_order`,
      and the walls used to be laid in the order the ring walks them. On a **stepped
      facade** (the sections of a block offset across the street) that is exactly the
      wrong order: two neighbouring walls overlap on screen by the width of the step, and
      the far section, walked later, covered the near one. The comparison here is exact,
      not sampled, because every wall of one house shares one lift: a wall is a band of
      constant thickness `|lift|` over its base, so at a shared `u` the wall whose base
      sits lower in `v` is in front. That relation cannot cycle (the edges of a simple
      ring do not cross, and three segments pairwise overlapping in `u` share a `u`, where
      they are strictly ordered), and it goes through the same `topological` as the
      houses, seeded by the depth of the base's midpoint. Courtyard walls are ordered in
      the same list. Tula: a wrongly ordered pair on 203 of 7524 buildings; pinned by
      `the_wall_order_puts_the_stepped_back_section_first`. The roof needs no ordering
      against the walls — it is drawn last and lies wholly above `base + lift` at every
      `u` it shares with a wall.
    `extrusion_lift` is the one door to that vector — the extrusion layer, the arch patch
    in the shadows and anything that wants to put a marker on the *drawn* building rather
    than its real outline all go through it. Known limits: units y-sort against
    flat z=5 and can draw over a tall roof they are "behind"; kremlin wall polylines
    (z 5.1) draw over nearby lifted roofs.
    - **`Lean` is a per-building value**, not a global function: direction,
      `lift(drawn)`, `ridge(rise)` and the draw order's base key `depth(centre)` all come from it,
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
    `is_pitched` (`BuildingUse::House` of any size, or `Other` with a footprint under
    `SMALL_FOOTPRINT_MAX` 250 m², never with a courtyard, never `AreaKind::Kremlin` —
    its towers and gates keep the flat roof, like they keep their colour) is in the
    **pitched cohort** and gets two slopes instead of a flat roof — unless `roofing`
    hands it to **Hip roofs** above, which is what happens on 4 houses in 10 by seed and
    on every outline a gable refuses. The ridge runs along the long axis of the footprint's minimum-area
    bounding rectangle (`min_area_rect`, edge directions of the ring tried as
    orientations — no hull needed at 4–20 vertices); the roof is drawn over that
    rectangle, not the outline (real roofs overhang), which is why it is only applied when
    the outline fills the rectangle to `RECT_FILL_MIN` 0.85 — an L-shaped house would
    wear a rectangle sticking out of it, so it takes a **hip** instead (it stayed *flat*
    until hip roofs existed). Slope tone: base roof colour mixed toward
    white/black by the slope's plan normal against `map::sun_light()` (`SLOPE_LIT_MIX` 0.14 /
    `SLOPE_SHADED_MIX` 0.11, through the same `shade_by_light` helper as the walls, in
    sRGB), softer than walls. In 2.5D the ridge is lifted a further
    `ridge_rise(width) = min(width/2 × ROOF_PITCH 0.8, ROOF_RISE_MAX 5 m)` real metres
    through `ridge_lift` (same `EXTRUDE_SCALE`, no `EXTRUDE_RANGE` clamp), and the two
    gable triangles are drawn on the visible end walls with the wall's top colour before
    the slopes. In flat modes the ridge lift is zero and the two shades are all that
    remains. Verified on Tula's western private sector: red-brown two-storey houses with a
    visible ridge — the L-shaped ones were flat there, which is the observation **Hip
    roofs** was written against.
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
    ≤ `LOW_FOOTPRINT_MAX` 300 m² at 2–4 (over 300 m²: the area test comes first, so a long
    thin shed stays low); everything else large is 2–5 — that last branch
    is the most populated one and being generous with it is what made the first version
    come out skyscraping (p90 27 m before the tower table was halved).
  - **Some uses are measured in metres, not storeys**: an industrial hall or a store has
    one tall span, a church has one storey to the cornice, a garage is one box and gets no
    spread at all (a row of garage boxes on a photo is all one height).
  - **A public building is measured by its use, not its shape** — school, clinic, office
    (`BuildingUse::Public`) at 2–5 storeys, the same table as «everything else large»
    but reached before the footprint is consulted: a 900 m² school with a squarish plan
    would otherwise be a tower.
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
    (`light` = `map::sun_light()`, `intensity` = `RoofStyle::texture`) and a per-vertex
    **`Roof` attribute** (`meshing::ATTRIBUTE_ROOF`, `[axis x, y, material code,
    seed]`). All four numbers are constant over a **face** — the roof's axis is the
    building's long axis, a wall's is its own direction — so the attribute is a
    *builder state* (`MeshBuilder::set_roof`), like the markings code, not an argument of
    every `push_*`; the fragment reads it `@interpolate(flat)`. Code `0` means **no
    texture** — roof clutter rides in the same mesh (2.5D is one
    painter's-order layer) and comes out with its vertex colour untouched. Walls rode
    at `0` too until they got codes of their own (`WallKind`, below), and so did the
    **gable**: it is the top of an end wall, takes the same `wall_frame` as the wall
    under it, and would otherwise break the pattern at the eaves.
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
    octave and every stripe grid fades by `visible(wavelength, px)`, the `noise.wgsl`
    rule, so nothing moirés when zoomed out; at the city zoom the texture is simply gone
    and only the material's colour is left. The noise helpers come from
    **`assets/shaders/noise.wgsl`** — its full set is named under **Surface material**
    above — and this shader imports by path the subset it calls
    (`#import "shaders/noise.wgsl"::{hash21, value_noise, visible, fbm3, stripes}`)
    — until this extraction each of the two shaders carried its own copy and the copies
    had already drifted in their comments; the `visible` rule in particular must not.
  - **The wall is the same mechanism on its own coordinates and its own codes**
    (`layers.rs::wall_frame`, `meshing::WallFrame` — the same frame the garage ribbon
    below is measured in). A 2.5D wall is a **parallelogram** —
    base edge `a→b`, side edges along the lean — and what the builder writes into
    `ATTRIBUTE_ROOF` at each of its vertices is where that vertex stands *in it*: **the panel
    number along the base and the storey up the lean**. Both linear in the point, so each is
    one dot product (the frame holds the rows of the inverted `[base, lean]` basis, already
    scaled by the counts), and the attribute is therefore **interpolated, not flat** — a
    roof's own numbers are equal on all three vertices of any triangle, so nothing changes
    for it.
    - **The counts are whole**: `round(wall length / PANEL_WIDTH 3.2 m)` panels and
      `round(building height / STOREY_HEIGHT 3 m)` storeys, at least one of each. That is
      the whole construction, and it is what the first version got wrong: it laid a **global
      metre grid** over every wall, and a grid that does not know where a wall ends cuts the
      edge panel and slices balconies in half at the corners. With whole counts the edge
      panel is whole, the top storey ends exactly at the eaves, a balcony inset inside its
      cell cannot reach the wall's edge, and at a corner both walls end on a whole panel and
      a whole storey — so their seams meet, which the global grid could not do either
      (its phase was arbitrary, and the two walls of one corner disagreed).
    - **The panel width is a target, not a divisor**: a 40 m wall takes 13 panels of 3.08 m,
      a 10 m one 3 panels of 3.33 m. A real panel block divides its facade evenly too.
    - **The shader needs no metres at all** — no panel width, no storey height, no lean:
      `fract` of what the vertex carries is the position inside the cell, and `fwidth` of the
      same number is cells per pixel, which is how each grid fades (`visible(1.0, px_cell)`).
      Foreshortening comes out of that derivative for free: a west wall, squeezed by the
      lean, loses its storeys earlier than a south one without a line of code about it.
    - **`WallKind` is to the wall what `RoofKind` is to the roof**, and the codes are one
      dictionary in one attribute slot: `0` no texture, `1…6` roofing, `7…8` the two garage
      runs, `9…14` cladding, `15` the door leaf (`WallKind::code` derives itself from
      `RoofKind::CODES`, the
      **last** roofing code rather than the length of `ALL` — the garage runs are outside
      `ALL` and would otherwise have been overwritten by the claddings — and `DOOR_CODE`
      derives from `WallKind::CODES` the same way, so a new roofing shifts the wall codes,
      a new cladding shifts the door, and the shader's mirror is edited whole).
      Six claddings —
      `Panel | Brick | Plaster | Shopfront | Shed | GarageDoors` — because panel seams with
      balconies are
      exactly **one** kind of building, and while the wall was one, a garage and a church
      wore them too. Five of them are chosen by the tables below; **`GarageDoors` is chosen
      by geometry**, like the garage runs on the roof — `layers::wall_of_run` puts it on
      every box of a run and nothing else can reach it, which is why
      `every_cladding_reaches_the_city` skips it.
    - **The pick** (`material::wall_look`) is `roof_look`'s twin — a ten-slot table per
      `BuildingUse`, the slot from the building's seed, the colour from the material's own
      palette plus ±3 % — with one difference: **height is consulted before the tag.**
      Anything under `LOW_RISE_STOREYS` (4) that is not already `House`, `Garage`, `Church`
      or `Industrial` drops into `LOW_RISE_WALLS`, because a low building is neither a panel
      block nor a curtain wall whatever OSM calls it. The seed is read from **other bytes**
      than the roof's (`>> 4`, `>> 12`, `>> 20` against the roof's raw, `>> 8`, `>> 16`):
      the two materials must be independent, or every panel block would also be under one
      bitumen. Kremlin is brick, `Church` whitewash — the same two exceptions the roof has,
      in the same order.
    - **Wall colours are calibrated the other way from roofs**: a wall is **lighter than its
      roof**, which is what holds the 2.5D box together (dark bitumen over light panel), so
      panel and plaster live in 0.66–0.86 and only `Shopfront` is deliberately darker — and
      it goes to shopping centres, of which a district has a couple. Spread follows the roof
      rule: tight in value and wide in hue for mass housing, bright and many-hued for the
      private sector, where ochre, whitewashed brick and blue plaster stand fence to fence.
      The **per-use facade colours are gone**; they painted half the city (`building=yes`)
      in one tone.
    - **What each cladding draws between the openings**: panel — floor seams, panel joints
      and a ±2 % tone jitter per *cell* (a chequerboard is the right answer here, the wall
      really is assembled from separately cast slabs); brick — courses at a twelfth of a
      storey, so they fade first and leave an even tone; plaster — `fbm3` streaks and no
      seam at all; shopfront — a spandrel band between the glazing strips; shed —
      corrugation ribs at a sixteenth of a panel plus a faint eaves line.
    - **A window is the only thing here that replaces the surface colour instead of
      correcting it.** Glass is not plaster some per cent darker, so `wall_shade` returns
      three numbers (`Wall { shade, glass, sky }`) and the fragment `mix`es toward
      `mix(GLASS_ROOM, GLASS_SKY, sky)` — both **linear** constants, because the vertex
      colour here is linear (`wall_colors` returns `LinearRgba`). `sky` rises up the pane
      (dark room below, reflected sky above), dips under the lintel for the reveal shadow,
      and is scaled by a per-window `tone`: a curtain, an open sash, dirty glass. Without
      that per-window draw a row of windows reads as a stencil.
    - **The opening is what a cladding is really about**: a two-sash window per panel (0.42
      of the panel, 0.30…0.72 of the storey), a narrower brick one, a small house window on
      plaster, a full-panel glazing strip on a shopfront, a high narrow ribbon on 55 % of a
      shed's panels. Mullions are placed **from the left edge of the opening**
      (`stripes(inside.x - 0.5 + half, wide / panes, …)`), so any pane count comes out
      right; centring them on the middle only works for even counts.
    - **The ground floor is its own case on every cladding**: never a balcony, a shopfront
      lower and taller there than the strip above it, and over it all a dark **plinth** band,
      the line that says where the building stops and the ground begins. The entrance is
      *not* drawn here — it comes as geometry from the data (the door bullet below); the
      shader rolls no doorway of its own on any cladding.
    - **The top of a wall needs room, not a stripe.** A wall is drawn exactly
      `storeys × 3 m` and a real building is not: above the last storey sit the ceiling, the
      roof slab and the parapet, and without them the top window butts straight into the roof.
      The first fix drew a **cornice** — a light coping with a dark seam — and changed
      nothing, because it painted inside the same `1 - WINDOW_HIGH` that was already there:
      the gap stayed identical to the pixel and one more line appeared. It was reported as
      exactly that and taken back out.
      What works is `meshing::PARAPET_CELLS` (0.15): the frame runs the storey coordinate to
      `storeys + PARAPET_CELLS`, and everything above the last whole storey is **plain wall
      with no openings** — nothing is drawn there at all. Whole cells survive, because the
      invariant is that *storey boundaries* are whole, not that the wall ends on one; the
      cost is the drawn storey shrinking by `1/(storeys + 0.15)`, 1.5 % on a nine-storey
      block. Measured on the frame the report came from: blank wall above the top window
      0.25 → 0.44 drawn metres, five screen pixels to nine at 0.05 m/px.
      The check that openings stop at `storeys` is not belt-and-braces: a window starts at
      0.30 of a storey and misses 0.15 on its own, but a **balcony** starts at 0.01, and its
      slab shadow and slab edge would climb into the cornice.
    - **Knowing where the top is takes the storey count**, and that rides **in the material
      slot** beside the code: `meshing::STOREY_STRIDE` (16) puts the code in the remainder and
      the storeys in the quotient, zero on a roof. That slot is the one field with a spare
      digit; a fifth float in the attribute would cost four bytes on every vertex of the
      building layer. `meshing::unpack_material` is the Rust mirror and exists only for the
      test that pins it — in the game the slot is written, and read by the shader alone.
    - **A *line* goes through `stripes`, never `cell_band`** — learned from the cornice seam
      before it was removed, and it still holds for every line here: `stripes` floors its
      width at one pixel (`max(width, px)`), `cell_band` does not, so a 0.05-cell seam (five
      drawn centimetres) vanishes at every zoom where the wall is visible at all. It took a
      probe run with the amplitude at 0.9 to tell "the branch never runs" from "the branch is
      too faint", and it was the second — worth remembering as the way to split those two.
      `cell_band` stays right for a *band* — plinth, balcony rail — which is thick enough to
      survive on its own.
    - **One opening per cell, on every cladding.** The shed briefly had two — a gate and the
      ribbon window, «because they sit at different heights» — and that stopped being true
      the moment the ribbon dropped from 0.60 to 0.45 to survive the fade: the window's
      sashes climbed onto the gate leaf and its bottom row came out as a stump over the dark
      rectangle. Different heights are not enough, because an opening owns its reveal and
      sill below it too, so the gap between two would have to allow for the frame; picking
      one of the two is the version that cannot drift.
    - **A door comes from the data, as geometry** (`layers::push_doors`, code `DOOR_CODE`
      12), and this is the one opening the shader does not place. It used to roll one on
      `DOOR_SHARE` (0.24) of the ground-floor columns and a gate on 0.30 of a shed's, which
      put drawn doors where `osm::entrances` has none and left the real entrance — the point
      the door gizmo marks and the pawn walks to — on blank wall; on a long London block the
      dice also put two doors in neighbouring panels.
      Two quads per door, both pushed **after** the wall they sit on:
      - a **patch** over the whole cells the leaf touches, carrying the wall's own frame and
        code so seams and courses run through it, marked `WallMark::Solid` (no openings).
        It is what stops the cell's window from peeking out beside the leaf: a window is
        centred in its cell, a door stands where the data put it, and they overlap. Whole
        cells, not the leaf's span — clipping a window in half looks worse than losing it;
      - the **leaf**, exactly on the entrance point, with `WallFrame::opening` — a frame
        with neither storeys nor parapet that maps the quad to `[0, 1]²`, so `doorway_of`
        draws the leaf at its centre and `DOOR_LEAF_WIDE/HIGH` are fractions of the opening
        rather than of a cell.
      **The metres are chosen on the CPU** (`layers::door_size`, by cladding: подъезд
      1.9 × 2.8 m, house door 1.3 × 2.4, shop leaves 2.4 × 3.0, shed gate 3.2 × 2.9) — a door
      is a scale ruler, and a gate that is wider than it is tall reads as a letterbox slot
      (the old `GATE_WIDE` 0.62 × `GATE_HIGH` 0.50 was exactly that, 2.0 by 1.5 m).
      A door at a ring **vertex** — which is where every real OSM `entrance` sits — is pushed
      inside the wall by half a leaf, and claimed by the edge it *starts*, so the two walls
      of a corner do not draw it twice. **The claim is by the nearest edge, not by every
      edge in tolerance** (`layers::door_edge`, ties to the earlier edge in the ring):
      `DOOR_ON_WALL` is half a metre and an OSM step
      can be twenty centimetres, so both edges took the door, each shifted its leaf inward
      to make it fit, and the wall came out with two doors side by side under a single
      gizmo. Cost: eight vertices per door, on drawn walls only.
      **Snapping the leaf to its panel instead was rejected**: it saves the geometry but
      leaves up to ±1.6 m between the drawn door and the entrance — the very gap this
      change exists to close — and packing a column index into the material slot caps out at
      twelve columns (≈38 m of wall), which is exactly where a second подъезд appears.
    - **A balcony is a stack of bands**, not a box: the slab's shadow on the wall, the bright
      slab edge, the parapet (its tone by its own draw, from light panel to dark sheet), and
      above it either glazing or an open recess in shade. 72 % of a panel, and along the wall
      **by the period of a section**: `(column + phase) % period < BALCONY_FILLED`, the
      period 4–6 panels (`BALCONY_PERIOD_MIN` + `BALCONY_PERIOD_SPAN`) and the phase both
      rolled from the **wall's** seed, two filled columns in it — a column the whole height of
      the wall, as on a real block, and a repeating step between columns.
      **A share is not a row**, and that is what the period fixes: the first version drew
      each column independently at 58 %, which on a ten-panel wall routinely gives three
      balconies in a row and then two blanks — a scattering with no step, exactly what the
      spec's «регулярным рядом» is not. The share is still a majority-ish (2 of 4–6), so
      nothing about the *density* changed; the dice survive only as the phase, so that two
      walls meeting at a corner do not start alike.
      Hashing the *cell* for **existence** is the
      thing that must not be done either: that is an independent draw per cell, i.e. a
      chequerboard; hashing it for *glazed or open* is fine and is what varies a column.
      A brick building's balconies are **recessed loggias** — no slab overhang, no bright
      edge, a deeper shade — and rarer: the same period with `BALCONY_FILLED_RECESSED` (1)
      column of it filled.
      **Two sashes, not four.** The lean squeezes the wall threefold vertically, so a
      balcony on screen is a ribbon four times wider than it is tall; cut into four it read
      as a scatter of dots. A balcony must read as *bands*, and anything chopping the ribbon
      crossways eats them.
    - **Who gets balconies is a CPU decision** (`layers::balconies_fit`): only `Panel` or
      `Brick`, never a `building=house` (the storey floor catches almost all of them, but a
      five-storey `house` does occur in the extract, and balconies on it would read as a
      parse error, which is what they would be), never under `BALCONY_STOREYS_MIN` 4
      storeys, never on a wall under `BALCONY_COLUMNS_MIN` 3 panels wide, and never on a
      **gable end**. The shader cannot
      decide any of it: it knows neither the use of the building nor how many storeys the
      wall has in total. Note what moved: the *use* filter now mostly lives in the cladding
      pick — plaster, shopfront and shed have no balconies by the meaning of the material,
      and they are exactly what the private sector, the mall and the warehouse get.
    - **A gable end is a direction, not a width** (`layers::plan_long_axis`,
      `WallSpan::gable_end`). «Глухие торцы» stood for a while as `BALCONY_COLUMNS_MIN`, and
      that threshold cannot say it: a real panel section is 12–14 m deep (the measurement is
      in `references/entrances.md`), which is four panels — over the threshold, so every
      gable end wore balconies like a facade, and the words «long facade» and «gable end»
      existed nowhere in the code. What the width threshold actually cuts off is a **step in
      the outline** — a three-metre sliver of wall where a row of projections could only read
      as a pattern — and it stays for that.
      The end is told from the facade by the **long axis of the plan**: `roofs::min_area_rect`
      (already computed for hipped roofs and the roof texture, so no second notion of "which
      way this building faces"), its first edge being the long one, and a wall counts as an
      end when its own direction is within 60° of the cosine of that axis
      (`GABLE_END_COS_MAX` 0.5, i.e. no more than 30° off the perpendicular).
      **The aspect-ratio guard is the load-bearing half**: on a plan that is not elongated a
      "short side" means nothing, so `plan_long_axis` returns `None` under
      `GABLE_PLAN_RATIO_MIN` (1.5) and *every* wall keeps its balconies — a tower
      (`heights::TOWER_MAX_RATIO` 1.7 the other way) would otherwise lose half its walls on a
      rounding, while a section (12–14 m by 35+, ratio 2.5 and up) is well clear of it. An
      oblique wall of a non-rectangular plan is likewise left a facade: when in doubt,
      nothing changes.
      The axis is computed **once per building and only when the building could carry
      balconies at all** (`balcony_house` — material, use, storeys), because `min_area_rect`
      is quadratic in the ring's vertices; it rides to each wall in `WallSpan::long_axis`.
      The end is marked `WallMark::Blank`, not `Solid` — it keeps its windows, since a blank
      wall and a windowless one are not the same thing (the bullet below).
    - **The verdict travels as the seed's own value**, not its sign: `WallMark`
      (`WallFrame::marked`) encodes `[0, 1)` balconies, `(-2, -1]` blank, `(-4, -3]` solid.
      Three states are needed because *blank* and *solid* are not the same thing — a blank
      wall (narrow, low, wrong material) still has **windows**, a solid one has none. Three
      surfaces are solid: the **gable**, where a window would be cut by the slope, and the
      two patches of one construction — **under a door** and **around an arch**
      (`WallCells`), where a cell is handed whole to an opening the shader knows nothing
      about. Gable and patch are unrelated: the old sign flip could say only one
      of the two, and needed an idempotency guard on top (the gable marks itself over an
      already-blank wall, and a second negation gave the balconies back); replacing the
      state is the guard.
    - **The seed is per wall**, `seed_from_point(a)` — the generator the roofs, doors and
      parked cars already share — not the building's: the balcony columns of two adjacent
      walls must not start alike. The **cladding**, on the contrary, is per building: one
      house does not have a panel end and a brick front.
    **The wall leaves the fragment before the common roof pass** — `wall_shade` is a branch
    of its own in `fragment`, not a case inside `roof_shade` — and that early exit is the
    whole of what the wall costs: two `stripes` and one hash, against the cheap path code `0`
    used to give it for free. Falling through into the common pass instead would hand every
    wall pixel `roof_age` and two more `fbm3` per frame, and with them a roof's
    fade-and-dirt: that octave is 8 m long and `visible` only kills it past ~5 m/px, so the
    *base colour* of every wall in the city would drift at the city zoom, where the texture
    is supposed to be gone. A wall therefore has **no age** — `roof_age` is the roof's — and
    no grime layer of its own either.
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
      the drive into its neighbour's. On Tula's Косая Гора cooperative ten parallel
      ribbons came out as **one** run of 349 × 72 m — whose axis was 7° off their own, so
      every bay seam stood askew to their walls, and which passed `BLOCK_MIN_WIDTH` on
      that bogus width and drew aisles across the whole stack. The real test is
      segment-to-segment distance between the two rings; garages do not nest, so
      containment need not be considered.
    - **Cutting the outline is what carries the geometry** (`split_rings`,
      `GarageRect`). The run used to hand every member **one** axis, taken from
      `min_area_rect` over the concatenation of all its rings, and that is wrong twice
      over: the contract of `min_area_rect` is a *ring*, and a ГСК is routinely an L or a
      comb whose own rectangle is 80 % air (Tula: 16 of 68 garage outlines fill theirs
      worse than 0.9, and they hold 29 % of all garage area). Everything downstream then
      inherited the lie — seams askew to the walls of both wings, aisles across the whole
      letter, and the ribbon/cooperative thresholds measured on air.
      So each outline is cut instead, and the axis, the grid and a share of the roof
      belong to the **piece**:
      - the cut is a **chord from a reflex vertex along one of its own two walls** to the
        nearest edge. Never an infinite line: on a comb that line runs on through the
        other teeth and leaves a "piece" made of several, joined by zero-width bridges
        (measured — a slab sweep in one frame shreds Tula's combs into 44 × 3 strips,
        the chord version does not). Along a wall, because the whole point is for the
        piece's axis to be parallel to its own walls;
      - candidates are scored by the **area-weighted fill** of the two halves, and a cut
        that does not improve on the uncut fill is not made. Recursion stops at
        `RECT_FILL` 0.90, at `SPLIT_DEPTH_MAX` 6, and never produces a piece under
        `PIECE_MIN_AREA` 8 m² — shaving a two-metre corner off is not "cutting into
        rectangles", it is making shavings, each with an axis and a seam of its own;
      - the pieces **tile the outline exactly** (a chord splits a simple ring into two
        simple rings), so the roof is laid one piece at a time, with no clipping and no
        gluing. A holed outline is never cut: a hole landing in the wrong piece is a
        courtyard under a roof;
      - on Tula this leaves 53 of 68 outlines whole, cuts 9 in two, 4 in three, 1 in four
        and 1 in seven, and the worst piece fills its rectangle 0.80 against the 0.18 the
        whole letter did.
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
      The Tula numbers are what forced the two-case split: 43 `building=garages` against
      32 single `building=garage`, and the plural ones are mostly **blobs** (255 × 51,
      183 × 69, 170 × 74 m) — whole cooperative territories, not rows.
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
      half at both ends — which is what a comb's teeth looked like. Reported from a
      screenshot, and the rule is the user's own: separate the crooked part and do not draw
      a garage on it.
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
      first — leaves a sliver of a bay at the other, and every Tula ribbon has one
      (74.9 m is 22.04 bays, 258.5 m is 76.03).
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
      south a ГСК is *read by its gates* the way it is read from above by its comb. Until
      it existed a garage box took the ordinary cladding tables: on a 75 m ribbon that
      came out as a brick apartment wall with two rows of windows and three подъезд doors.
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
      mark before it ever reaches `gate_of`. Nothing could say this before the outline was
      cut, because cladding is chosen for a whole *building*: an 8 m end took
      `round(8 / 3.9) = 2` cells and wore two gates, and at the corner they met the gates
      of the long facade head on.
      **And the wall is measured by the piece's bay, not by the constant `BAY`**
      (`layers::garage_cells`). "A gate stands under its own roof seam" was true only by
      coincidence before that: a wall divided *its own* length by `round(length / BAY)`,
      which agrees with the roof on a whole rectangle whose long side is that wall and
      nowhere else. On a cut outline the gates drifted off the comb the further from the
      wall's start you looked — reported from a screenshot, and now pinned by
      `a_gate_stands_under_its_own_roof_seam` (a 15 × 5 ribbon holds three 5 m bays, where
      the constant would have given four of 3.75).
      The **phase** is deliberately *not* taken from the piece, and that was tried first:
      a wall sitting on the piece's phase begins and ends mid-cell, and its end gates come
      out sliced in half — two half-gates on every tooth of a comb, which is exactly what
      the next screenshot showed. The wall keeps a whole number of cells ending on its own
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
    - Tula: **29 buildings** end up in runs.
  - **A cell grid places a feature, it never *is* the feature** (`repair_patch`). The
    bitumen patch started as `hash21(floor(uv / 6))` — a shade of its own for every 6 m
    cell — and that is not repair patches but a **chequerboard across the whole roof**:
    the edge is hard, the grid is aligned to the walls (`uv` is the building frame), and
    ±4 % of brightness on a big dark roof is plainly visible at the working zoom. The
    rule the fix follows — and the one the **asphalt patches** broke a second time, which
    is why they are gone (**Asphalt wear** above): only a minority
    of cells carry the feature (the `share` argument), and
    inside its cell the feature is smaller than the cell and jittered, so two neighbours
    never meet at a cell boundary. (The **wall balconies** above keep only the second half of
    that rule — a balcony is smaller than its cell — and deliberately break the first twice
    over: they are on about half the columns, because a panel block's balconies are a
    majority and a *column* of them is the pattern, not a scattering; and which columns is
    not a draw at all but the **period of a section**, because the pattern there is a step.
    A minority-by-dice is right for a feature that is an accident — a repair patch — and
    wrong for one that is construction.) Placement stays a grid (cheap, no extra octaves); the
    pattern does not.
    The garage bay above is the deliberate exception — there the seam grid *is* the
    feature, because a row of doors really is one.
  - **Roof age** (`roof.wgsl::roof_age`) — one number per building in [0, 1), **hashed from
    the same seed** the texture phase rides on, and with a fixed patch share that was the
    missing half of the patch fix: a minority of cells carried a patch, but *the same*
    minority on every bitumen roof, so the whole district read as re-roofed and repaired in
    one year. Age drives the patch share (`PATCH_SHARE_NEW` 0.04 → `PATCH_SHARE_OLD` 0.28,
    mixed by `age²` — age is uniform, repairs are not; with ⟨age²⟩ = 1/3 the mean share
    lands at 0.12, half the old fixed 0.22, so a patched roof is an event against clean
    neighbours instead of the district's baseline), the ponding amount (0.07 → 0.13) and,
    on **every roofing** material, the common fade-and-dirt amplitude (×0.75 → ×1.35) — the
    last one is what makes the age read as age rather than as a patch counter. **The wall is
    not one of them**: it returns from `roof_shade` before the age is ever hashed (see the
    wall bullet above), so a wall carries neither an age nor the common pass.
    It gets **no vertex attribute of its own**:
    the seed is already a per-building random number the shader hashes several ways
    (`seed·17`, `seed·11`, `seed·37`), the correlation between two patterns of one building
    is not visible, and a fifth float would cost four bytes on every vertex of the building
    layer. The consequence for the gallery: its `Seed` knob rolls the age too — that is how
    a new roof is compared against an old one there.
  - **There is no parapet, and putting one back needs a different construction.** A soft
    flat roof (bitumen / gravel / membrane, the old `RoofKind::has_parapet`) used to get a
    0.7 m inset band along its ring and every courtyard ring, lit by `shade_by_light` like
    a wall (0.24 / 0.20) — bright on the sunny edges, dark on the shaded ones. That is
    **the same construction as a hip roof's slopes** (an inset band on miter offsets,
    shaded by the edge's own outward normal) at a third of the width, so from the air every
    panel block wore a small hip, and the hip of a large private house — inset capped at
    `HIP_INSET` 2.2 m — was the same picture only wider. Two things a roof shape must never
    do, and it did both. The band's other cost is why the retreat is not a one-liner back:
    it wants to read as a vertical coping standing *above* the roof, and an inset band
    shaded by a plan normal cannot say that. `MeshBuilder::push_inset_band_with` (the
    per-edge-colour sibling of `push_inset_band`) survives as a primitive; nothing in the
    building layers uses it any more.
  - **One call lays every flat roof** — `layers.rs::push_flat_roof(builder, look, outer,
    holes, color)`: set the roof frame, fill the contour with its courtyards. Both callers
    use it — the flat modes with the real contour, 2.5D with the contour already lifted
    onto the walls. It is `pub(super)` now: the gallery reaches the same geometry through
    `push_house`, one level up. Slopes stay outside it: a gable
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
    the shape of `tree_gallery`), and it answers the two halves of "what is a roof" in two
    grids, because a material and a shape are chosen by different code from different
    inputs.
    - **Materials, below** — seven blocks, six materials plus the church palette, each
      with **a house per palette colour**, sized from a 30 m block down to an 8 m shed, so
      the two things a still picture cannot say are said at once: the palette's spread
      (tight in value, wide in hue) and that the texture is in **metres** and does not scale
      with the house. Their roofs are **flat by request** (`RoofShape::Flat`) and carry no
      clutter — a slope would take half the covering out of view and a shaft would stand on
      the rest.
    - **Shapes, above** (`shapes.rs`) — five outlines (rectangle, near-square, L, U, and a
      dumbbell: a big body on a thin neck) each under all three shapes **and** under the
      game's own choice, four columns. Under every house the shape that actually reached the
      mesh — a refused one says so instead of being quietly swapped, which is what
      `RoofShape` exists for — and the ridge rise in real metres; beside every row the two
      numbers the choice is made from, rectangle fill and hip inset, straight from
      `roofs::shape_facts`. That is the only way to tell a hip from a flat roof with a
      chamfer on a picture, and a gable's rise from a hip's.
    - **Every house is a house** — `push_house`, the per-building body of
      `extrusion_builder` (walls, roof, clutter), lifted out of that loop for exactly this
      reason: without walls under it a shape shows neither its ridge rise, nor its missing
      gables, nor the silhouette height two same-sized houses do not share. The gallery
      keeps its own painter's order by laying the grids top-down; it does not sort, because
      gaps keep its houses from overlapping at all.
    - Knobs are what the game reads off the building itself — wall height, long axis, phase
      seed, courtyard — plus two game sliders, `RoofStyle::texture` and **the sun**
      (`SunStyle` azimuth and elevation over the same process global, written straight into
      `SunOnMap` with no `settle_sun`: the settle exists for the city's rebuild, not the
      gallery's; the shapes grid always carries its clutter, so the elevation knob has a
      shadow to move). The readout at the bottom right
      prints metres per pixel and the two wavelengths `visible()` cuts at, since at city
      zoom "the texture is gone" and "the texture is off" look alike. Under the knobs the
      panel lists the **tuning constants** of both halves — texture from `roof.wgsl` and
      shape from `roofs.rs` (fill threshold, hipped
      share, inset and its clamp, pitch) — parsed out of those files by `constants.rs`
      (`include_str!`, lines of the form `const NAME: f32 = …;`) rather than mirrored as Rust
      numbers: a mirror would drift on the first edit and the gallery would then lie about
      exactly what it is opened for. They are text, not knobs: the numbers live in the code.
    - What the gallery may **not** do is roll its own geometry: every house goes through
      `push_house`, the shape numbers come from `shape_facts`, the shape that a caption
      reports is the one `push_house` returned. It picks material and colour directly
      (`RoofLook::new`) instead of
      through `roof_look`, because the seed cannot reach every combination — a membrane never
      lands on a private house — and it drops the ±3 % seeded jitter so the hex printed under
      a house is the constant in `material.rs`.
    - `ROOF_GALLERY_SHOT=path.png` takes one frame and exits. The example has no BRP, and a
      screen grab over another window comes out black, so this is the only way a session
      without the window in front of it can look at its own work.
    - **`roof.wgsl` holds two textures, and the two galleries split it by the section
      banner** (`─── стена`): the roof gallery parses everything **before** it, the wall
      gallery everything **after**. A name list would have to be extended on every new
      constant; the banner sits exactly where the meaning changes. Both halves are pinned by
      a test that the other half's constants did *not* come through — an empty or
      over-full group is a parser drift, not a fact about the code.
  - **The wall gallery** — `cargo run --example wall_gallery`
    (`examples/demos/wall_gallery/`), and it answers the two halves of "what is a wall" in
    two grids, because *what it is made of* and *who gets it* are decided by different code
    from different inputs.
    - **Claddings, left** — a row per `WallKind` and, in the row, the **storey ladder**
      2 / 4 / 5 / 9 / 16. The ladder is not evenly spaced: 2 is the low-rise that never has
      balconies, 4 is `BALCONY_STOREYS_MIN` exactly — the first storey count that does — 5
      and 9 are the mass housing, and 16 shows that the pattern *repeats by storey* rather
      than stretching. Colours cycle through the material's palette, so a row is also a look
      at its spread.
    - **Uses, right** — a row per `BuildingUse`, two houses in it (2 and 9 storeys), and the
      cladding is chosen by the **game** (`wall_of`), not ordered by the gallery. It is the
      only place the whole rule is visible at once: the tag picks the table, the height
      picks the branch inside it, and one `commercial` comes out brick at two storeys and
      glass at nine. The caption under a house is the material that actually came back.
    - **The readout prints the storey in pixels**, not only metres per pixel: a wall is
      measured in cells and every fade is keyed to them, and a storey is `STOREY_HEIGHT`
      metres already squeezed by the lean — so metres per pixel alone does not say when a
      window is due to vanish. It comes from `extrusion_lift` on a probe building rather
      than from a copy of `EXTRUDE_SCALE`, which is private and would drift.
    - Like the roof gallery, it may **not** roll its own geometry: every house goes through
      `push_house`. The cladding grid orders its material (`WallLook::new`) because the seed
      cannot reach every combination — a curtain wall never lands on a private house — and
      the use grid orders nothing at all.
    - `WALL_GALLERY_SHOT=path.png` takes one frame and exits, same as the roof gallery's.
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
  - **Placement** is the shared Park–Miller LCG — `map/seed.rs::Lcg`, one copy for the
    whole of `map/*` (crowns, this clutter, the parked cars), the crown generator's
    original lifted out of `map/trees` — seeded from the roof material's building seed
    (`seed::seed_from_point`, the same door), so the
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
    swept the item's own height × `map::shadow_length_scale()` along `shadow_dir()`. The
    sweep is
    the two silhouette edges plus the offset rectangle — the same construction the
    buildings' own shadows use, and for the same reason: for a convex base that *is* the
    missing part of the union, and no convex hull has to be built. (The first version
    swept all four edges; two of them were always inside the union.)
    The length is **cut at the edge of the roof** (`shadow_reach`: the nearest hit of the
    shadow ray with the outline, from each corner of the base, outer ring and courtyards
    alike). Physically the shadow would go over the edge, and on a photo it does — but this
    one is drawn opaque and in *this* roof's colour, inside the merged building mesh, so
    past the edge it would be a dark bar lying on the neighbour's roof and on the ground.
    The cut is not reserved for the low end of the slider: at the default 59° a 3 m plant
    room already wants `3 × cot 59° = 1.8 m`, more than the `EDGE_MARGIN` 1.6 m the frame
    keeps, and the margin shrinks further on a narrow building (1.08 m on a 6 m side), so a
    1 m shaft can be cut there too. Only the smallest boxes on a wide roof stay whole at
    59°. At 15° a 3 m lift penthouse wants 11 m, so the clipping the elevation knob exists
    for is what dominates at the end of the slider — but the default picture is not
    untouched by it.
  - **Zoom.** The clutter is the only thing zoom changes about the building layer, and
    it cannot be hidden without rebuilding, since it lives in the same merged mesh as
    the houses (painter's order is per building: walls, roof, then its own clutter). So buildings got a zoom bucket of their own — `BuildingLods` /
    `BuildingZoomBucket`, two steps at `ROOF_CLUTTER_MAX_ZOOM` (0.5 m/px), seeded on
    world entry before `spawn_map` and rebuilt on a threshold crossing through the same
    `retuned` gate the height mode uses (one registration with `or_else`, deliberately:
    two registrations of `rebuild_buildings` in one schedule could both fire in one
    frame and spawn the layer twice).
  - **What it costs** (Tula, 7643 buildings, 2.5D+shadows+tint, from
    `examples/bench/map_meshing` on the `dev` profile): 785 044 verts / 78 ms with clutter
    against 461 212 / 65 ms without — one hitch on the threshold crossing, in the same
    class as the rail layer's deepest bucket (673 k / 23 ms). Most of it is the shafts:
    every flat roof gets at least one, and a shaft is 6 quads. The 603 018 / 279 186 that
    stood here is an older build of the layer (the gap is 182 026 verts in **both** clutter
    buckets, so it sits in the walls and roofs, not in the clutter or the shadows), and the
    car section's aside — which name-drops this very layer — disagreed with it by a third.
    One bench run prints buildings and cars together, so
    **re-measuring one aside means writing down the other**; both now come off the same
    run.
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
  takes a `facing` (2.5D: `-Lean::dir()`, so south **and** west walls; facade band:
  south only) — because `push_wall_with_openings` matches an opening to its wall by exact
  edge endpoints, and a wall family the lookup does not know about would draw solid. In
  2.5D the wall is **really cut** (side pieces + a lintel above,
  `push_wall_with_openings`) so the layers beneath — the road running through, the
  ground — show through the hole, and `shadow_builder` patches the opening with
  `SHADOW_COLOR` (the lintel shades it; without the patch the hole glows).
  **The wall texture does not see that cut, so the cells the opening bites into are
  handed to it whole** (`WallCells`, `WallMark::Solid` — the very patch a door lays under
  its leaf; it is built by `layers::wall_cells`, so the panel and storey arithmetic stays
  with the wall and `arches` only cuts): a window sits in the middle of its cell and the
  shader knows nothing about the hole, so the opening's edge sliced a row of windows in
  half — on the pier beside the arch and on the lintel above it. Only the *partial* cells
  are blanked: the ones inside the opening are gone with it, and above and beside the
  patch the wall is whole, so its windows are complete and stay drawn. What is **not**
  done is snapping the opening itself to the cell grid: the navmesh is carved by the
  road's real width, and an arch narrower than its road brings back the very lie arches
  exist to fix — a pawn walking where a wall is drawn. In facade modes the facade band is
  one earcut polygon, so the opening is *painted* in shaded ground colour instead — a
  stated compromise. What the passage does to the navmesh is in the navigation-deep
  skill.
