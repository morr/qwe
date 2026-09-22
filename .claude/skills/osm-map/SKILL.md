---
name: osm-map
description: Use when working on the OSM pipeline or map rendering in qwe — the Overpass query/mirrors/cache, map/osm/* parsing, the MapData model, building heights, entrance generation, tree planting and crowns, merged-mesh rendering, road/rail/tram/bridge layers, building and tree style resources. Deep detail behind CONTEXT.md's OSM section.
---

# OSM map pipeline — deep detail

This is the detail layer behind the **OSM map pipeline** summary in `CONTEXT.md`:
how the downloaded data becomes `MapData` and pixels.

This file is the **map and the invariants** every layer shares — the cache, the model,
the grid, the shapes, the shadow rules, the layer seam, the surface material, zoom
buckets. The mechanisms live in nine deep dives next to it, read on demand — **read the
one your change is about, not all of them**:

- `references/parse.md` — the parse seam and every reading and finishing pass (heights,
  building use, retail box, churches, squaring, houses pulled off the sidewalks, blocks
  and lots pulled to the roads), and how the parse is tested.
- `references/roads.md` — how a street is drawn: sidewalks, divided streets and their
  medians, lane markings and their breaks, the ribbon, junctions and the drawn network (stitches, kerb returns),
  `RoadStyle`, bridge layers, asphalt wear; the junction gallery `examples/demos/roads`.
- `references/parking.md` — the parking layout and the big lot (its kerb, the medians,
  the gores at a roundabout), and the parked cars.
- `references/layers.md` — water (rims, shoal, waterways), the yard, pitches, rails,
  tram, industry, standing wagons, fences.
- `references/osm-coverage.md` — the **tag coverage audit** (in Russian): which OSM
  tags reach the map, which are downloaded and thrown away, which are never asked for,
  with per-city counts, and how to regenerate them (`tools/osm_audit/`). Read it
  before widening the Overpass query — a feature that already exists must not get
  "added" twice. Widening the query or adding a `parse_way` branch means updating it
  in the same change.
- `references/entrances.md` — the door generator: the measured statistics behind the
  cohort table, the pitch law, blocked walls, determinism.
- `references/buildings.md` — how a house is drawn: the height modes and their shadow
  sweeps, temples and the fortress, inferred storeys, the roof and wall material with its
  shader, the roof clutter, the arches.
- `references/trees.md` — planting (woods, standalone, rows, density/thresholds) and
  rendering (crowns, TreeStyle/TreeRowStyle, conifer stands).
- `references/tree-algo.md` — the watabou Village Generator crown algorithm (in
  Russian), reverse-engineered from `Village.js`; the ground truth `map/trees/crown.rs`
  is written against.

When a change here introduces or retires a concept, update the matching summary bullet
in `CONTEXT.md` and the detail in the reference that owns it (or here, for a shared
invariant) in the same change.

## From a screenshot to the OSM feature

A map bug usually arrives as a screenshot ("the house is missing a corner"). Two scripts
turn it into OSM ids without writing a projection by hand — every past session wrote
its own `find_building.py` with the `GeoBounds` formula copied in:

```bash
tools/shot_coords <png> [px py]        # the HUD camera line → centre / cursor / pixel in map metres
tools/osm_near <x> <y> [radius]        # ways, relations and nodes there, nearest first, d=0 inside
tools/osm_near <x> <y> 30 --key building --verts   # + vertices in metres, for measuring
tools/osm_near --id 179102449          # one element: all tags, every vertex
```

**Position comes from the screenshot's HUD** (the last line of the top-right panel,
`zoom/x/y cx/cy`), never from `brp cam` — the live camera has moved since the picture was
taken. The user's screenshots are arbitrary screen regions from a macOS utility;
`shot_coords` handles those (the panel is its ruler) as long as the panel is in the crop.
Details and caveats (window size, zoom precision) — `.claude/live-app-project.md`,
"Where a screenshot was taken". `osm_near` reads the newest `assets/osm/<city>_*.json` and
projects with the centre and size from its name, i.e. the same metres as `SimPosition` and
`brp cam`; to confirm the pick live, `brp cam x y` then `brp shot`.

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
  `leisure=pitch|track|playground|sports_centre|stadium` (way+rel),
  `barrier=city_wall`, `barrier=fence|wall|retaining_wall|hedge` (way — the plot
  **fences**),
  `man_made=storage_tank|silo|chimney|water_tower|gasometer` (way+node),
  `man_made=pipeline` (way only),
  `highway=crossing|traffic_signals|stop|give_way|mini_roundabout|turning_circle|turning_loop`
  (node — the **road nodes**), `traffic_calming=island` (node+way), `area:highway` (way).
  The bbox is `MAP_SIZE` around the selected
  `City`'s geo center. `QUERY_VERSION` is **15** (v3 added `entrance` nodes, v4 `railway`,
  v5 `natural=tree_row`, v6 `natural=tree` nodes, v7 linear `waterway`, v8 `landuse`
  blocks, v9 `amenity=parking`, v10 the `leisure` pitches and playgrounds, v11 the
  industrial `man_made` cylinders and pipelines, v13 `driving_side`, v14 the fences, v15
  the road nodes, islands and `area:highway`;
  **v12 is skipped** — the fence branch held that number while it waited its turn and
  `driving_side` reached master first, and the number may only ever **grow**: a v13
  cache is already on disk without `barrier`, and reusing the gap would have served the
  fence layer an extract that has none, silently).
- **Driving side** — a second output after `out geom`: `is_in(lat,lon)` at the city's geo
  center → `rel(pivot)["driving_side"]` → `out tags`. The tag is not on roads: OSM puts it
  on the **country boundary** and everything inside inherits it (Tula's v11 bbox cache
  carries zero). `out tags`, never `out geom` — a country's border geometry is megabytes.
  `is_in` needs areas, which not every mirror builds; an empty answer leaves
  `MapData::traffic_side` at `TrafficSide::Right` and logs it (`parse.rs::driving_side`,
  the innermost `admin_level` wins). Verified on `maps.mail.ru`: UK and Japan `left`,
  Russia `right`.
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
- **The cache has a second reader** — the junction gallery (`examples/demos/roads`)
  reads it through `download::city_extract` (the cache, or the loader when there is none)
  and cuts its windows out of it with `map/osm/crop.rs`, which then go through
  `parse::parse_response` — the `parse` door for an already deserialized answer. So a
  query bump reaches the gallery with no step of its own; details in
  `references/roads.md`, **The junction gallery**.
- **One file per city** — every load first runs `prune_stale_caches()`: anything under a
  known city slug that is not that city's current `cache_path` is deleted. That is what
  retires extracts left by an old geo center, `MAP_SIZE` or `QUERY_VERSION` — tens of MB
  each. It sweeps **all** cities, not just the one being loaded, so junk under a city
  nobody visits still goes; the current file of each city survives, so a tour of the six
  is not six downloads per lap.

## MapData — the parsed model

`map/osm/model.rs`; the resource stays resident after spawn.

- **The nine `Vec<PolyArea>` are not `AreaKind` written twice**, and this has been
  reviewed and closed once — do not re-open it as "the same fact encoded twice". The
  vector says which **layer** an area is drawn in; the kind says which **member of that
  layer** it is, and three of the nine hold more than one member:
  - `buildings` holds `Building` **and** `Kremlin`, and the difference is read in nine
    production places — the wall ribbon (`roads::Fortresses`), the fortress roofs and
    merlons, the brick cladding, the tint ramp;
  - `landuse` holds `Residential` **and** `Industrial`, which are two surface textures
    (yard grass against trodden works ground);
  - `Pitch(PitchKind)` carries a **payload** — the sport — that a vector cannot hold at
    all.

  Collapsing them into one filtered vector would therefore lose nothing of the kind (it
  would all still be needed) and would add a pass over tens of thousands of areas per
  layer. The `match` in `parse::push_area` (one arm per vector) is a dispatch that
  exists **once**; it is not repeated on the read side — what `spawn::mesh_surfaces`
  matches is the *sub-class within* a vector, which is exactly what the vector cannot say.
- **PolyArea** — polygon with holes; rings are open (no repeated last point).
  `AreaKind: Building | Kremlin | Water | Park | Wood | Grass | Sand | Residential |
  Industrial | Parking | Pitch(PitchKind)`. **Park** is the
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
  Tula, cache v14: 355 in the bbox, 349 reach `MapData::parking` — five carry
  `building` and stay buildings, one is `parking=multi-storey` with no building on it.
  Recount with `tools/osm_audit/cache_audit.py` on the cache in `assets/osm/`.
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
  tag is there). See **Building height** below.
  `storeys: Option<f32>` — `building:levels` as the tag says (`area_storeys`), buildings
  only, **without** `roof:levels`, which by S3DB describes the roof and not the floors;
  the height alone cannot be asked how many floors a building has — a four-storey mall
  and a two-level retail box are both about twelve metres — and `is_big_box` is what
  reads it. Not to be confused with the *inferred* storeys of `buildings/heights.rs`,
  which invent a height where OSM is silent. `building_use: BuildingUse` — the
  drawing class (`Other` on everything that is not a building), see **Building use**
  below. `entrances: Vec<Vec2>` — the OSM
  doors on this building's outline, empty for most buildings; see
  `references/entrances.md`.
- **RoadLine** — centerline polyline + width **from its section**: lanes × 3.3 m (3.0 on a
  service drive) + 0.5 m of edge each side, set by the first parse pass
  (`map/roads/network/sections.rs`; footways keep 3.5 by class) — streets, sections and
  the tapers between them are in `references/roads.md`, **Streets, sections, tapers**.
  `highway: Highway` is the `highway` value (the five `*_link` are classes of their own);
  `Highway::is_street` — not a service drive, not a path — is what `roads::is_carriageway`
  asks, **not the width**. `RoadClass: Street | Alley` (alleys = footways, park paths;
  different color and z). `bridge` and `passage` flags — the navmesh carves (see the navigation-deep
  skill); `bridge` also moves the road into the bridge deck layers (see **Bridge
  layers** below). Three more fields feed the **markings** and the parked cars: `oneway`
  (`oneway=yes|1|true|-1`; `reversible`/`alternating` are not one-way), `roundabout`
  (the `junction=roundabout|circular` tag, implies `oneway` — but **nothing asks the field
  directly**, they ask `RoadLine::is_roundabout`: tag **or** shape, a closed one-way way
  being a ring too. In the Tula cache the two never coincide — 12 tagged ways, none of
  them closed, against 7 closed one-way ways, the mall's big ring among them — so a
  consumer reading the bare field is a consumer that misses every untagged ring) and
  `lanes: Option<u8>` (the `lanes`
  tag through `parse_measure`, floored, 1–8; `2;3` reads as 2, `0` and `12` as no tag;
  without `lanes`, `lanes:forward` + `lanes:backward` — then **overwritten** by the
  section pass with the inferred count on every street and drive, so after the parse it
  is `None` on paths only). Coverage per city is in `references/osm-coverage.md` — Tula has `lanes` on 97 % of its
  streets ≥ 8 m, the European cities on about half. `turns: [Vec<LaneTurn>; 2]` — the
  `turn:lanes` per direction of flow (`parse/tags.rs::tagged_turns`: a one-way road reads
  the plain tag or its flow's `:forward`/`:backward`, a two-way one only the directional
  ones; each lane left to right as `LaneTurn { left, through, right }`, `slight_`/`sharp_`
  folded into the turn, an unknown word — Tula has `throught` — into through). Only the
  turn paths and their lane arrows read it (`references/roads.md`, **Turn paths**);
  Tula 62 ways. `sidewalks: [bool; 2]` and `parking: [KerbParking; 2]` — `[left, right]`
  along the points, from `sidewalk=*` and `parking:*` (swapped with the points on
  `oneway=-1`); the sidewalk band and the kerb pockets read them (`references/roads.md`,
  **Sidewalks**, **Kerb pockets**).
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
- **RoadNode** / **RoadArea** (v15) — what the data says about a junction beyond its
  lines. The **junction paint** (`roads/node_paint.rs`, `references/roads.md`) reads the
  crossings (zebras), signals and the stop / give-way signs (who breaks, stop lines);
  a turning circle on a dead end is a disc of asphalt (`references/roads.md`, **Turning
  circles**); mini-roundabouts, islands and the `RoadArea` outlines are still drawn by
  nothing — later stages of the roads plan consume them. `MapData::road_nodes` — a point on a way's axis with
  `RoadNodeKind`: `Crossing { signals, island, marked }` (`crossing=traffic_signals` /
  `crossing:signals=yes`; `crossing:island=yes` / `crossing=island`; `marked` is cleared
  only by an explicit `crossing=unmarked` or `crossing:markings=no` — Tula has 111 of 801
  crossings with no kind at all, and plausibility wants a zebra there), `TrafficSignals`,
  `Stop`, `GiveWay`, `MiniRoundabout`, `TurningCircle` (both `turning_circle` and
  `turning_loop`), `Island` (`traffic_calming=island` as a node). `parse/tags.rs::
  road_node_kind` is a whitelist, the `rail_class` reason: `highway=bus_stop|street_lamp`
  sit on nodes too. `MapData::road_areas` — a closed outline with `RoadAreaKind`:
  `Carriageway` / `Walkway` by the **road class** its tag names, read by the same
  `road_class` as a line (`area:highway=<class>`, or `highway=<class>` + `area=yes`),
  `Island` for `traffic_calming=island` / `area:highway=traffic_island`; a value outside
  the road vocabulary (`emergency`, `platform`) is skipped. The branch is the **first** in
  `parse_way` and falls through: `highway` + `area=yes` still arrives as a `RoadLine` as
  it did before v15, and whether the square should stop being a line is the consuming
  stage's call, not the parse's. Tula v15: 801 crossings, 206 signals, 80 give-way, 28
  stop, 1 mini-roundabout, 34 road areas (15 carriageway, 19 walkway); six-city counts in
  `references/osm-coverage.md`, «v15».
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
  impassable. **The ribbon is drawn only off fortress buildings** (`roads.rs::Fortresses`,
  the axis probed every `WALL_PROBE_STEP` 2 m against `AreaKind::Kremlin` outlines): Tula
  also maps the wall as `building=wall` (relation 13342793, 12.7 m) and each tower as a
  building, and the ribbon laid over the lifted 2.5D wall read as a dark-orange outline and
  as red rings over the towers — the user's report. A leftover piece under `WALL_STUB_MAX`
  40 m is dropped too, but only on a line that was cut at all: the axis and the outlines
  drift apart by 15–30 m pieces at Tula's corner towers. Navmesh untouched.
- **FenceLine** — a plot boundary: `points` + `FenceKind: Fence | Wall | Hedge`, from
  `parse/tags.rs::fence_kind` (`barrier=fence` → `Fence`, `wall|retaining_wall` → `Wall`,
  `hedge` → `Hedge`). `MapData::fences`, drawn by `map/fences.rs` (**Fences** under
  Rendering). **It blocks the navmesh, with gaps**: `FenceLine::band` is the physical
  `FENCE_BAND_WIDTH` 0.3 m, and `footprint::fence_gaps` opens it where a road passes
  through — including a **loose end dropped a few metres short of it and aimed at it**,
  the very gap the drawn network closes with a stitch, so the stitched asphalt never runs
  through an uncut fence; `gates` holds the **default gates** the load thread adds — empty
  after parse.
  Both are the navigation-deep skill's (**Fences block, with gaps**).
- **Structure** — an industrial cylinder: `man_made=storage_tank|silo|chimney|
  water_tower|gasometer` as centre + radius + height + kind (`StructureKind`). A
  **whitelist**, for `rail_class`'s reason and more so: `man_made` is OSM's most mixed
  key (`surveillance`, `street_cabinet`, `works`, even `bridge` as an outline), and only
  those five read as a round spot from the air. Comes from a node **and** from a way:
  a node has no outline, so its radius is the `diameter`/`width` tag or the kind's
  default (`structure_size` — 2.5 m and 60 m for a chimney, 8 × 12 for a tank, 20 × 30
  for a gasometer); a way's radius is the **mean distance from the vertex mean**, which
  is exact on the near-circular ring OSM actually draws and overestimates a rectangular
  silo block by √2. The way branch **returns** — the opposite of the pipeline one below —
  because a chimney tagged `building=yes` is *the same object*, and falling through
  would put a box under the circle; it also keeps doors and the navmesh off it, which a
  chimney has no use for. Height comes from `height` only (`structure_height`): a
  chimney has no storeys, so `building:levels` is not consulted. Tula: 8 chimneys (4 of
  them nodes, one carrying a size tag) and 2 water towers; Berlin 2846 cylinders.
- **PipeLine** — an overhead heating main: `man_made=pipeline` centerline + bundle width
  from `count` (`pipe_width`, 0.7 m per pipe clamped 0.9–4 m; Tula runs pairs on twelve
  ways and fours on six). **The above-ground test is inverted** relative to rails and
  waterways: there underground has to be proven (`is_underground`), here *above* ground
  does — `location=overground|overhead|bridge` and nothing else. An untagged pipeline in
  OSM is buried, and drawing a silver line across the city over a buried pipe is a
  bigger lie than losing a trestle whose tag somebody forgot. The branch **falls
  through** like the rail and tree-row ones: a pipeline crossing a street on a trestle is one way
  carrying both tags. Tula: 22 of 24 ways kept, 1.2 km.
- **WaterLine** — a *linear* watercourse: `waterway=river` 8 m → `canal` (and `weir`)
  6/4 m → `stream|brook` 2.5 m → `ditch|drain` 1.5 m, water blue, one merged ribbon at
  `Z_WATERWAY` (see **Waterways** under Rendering). Widths are drawing widths, not
  hydrology: OSM draws as a line what is too narrow for a polygon, so a `river` line is
  narrower than the Упа (which is an area). A plausible `width` tag
  (`WATER_WIDTH_RANGE`, 0.5..50 m) overrides the class default.
  `parse/tags.rs::water_class` is a **whitelist** for the same reason `rail_class` is:
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
  semicircle of blocked tiles. One rule, both layers: `water::mesh_water_lines` and
  `Navmesh::fill_from_mapdata`. The **mouth** — where the drawing cuts the channel at an
  area-water outline — is the opposite case: render-only, and the grid keeps its caps
  (the polygon blocks those tiles anyway).
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
- **trees** (**`TreeSet`**) — what the renderer reads: `MapData::compose_trees`
  merges the forest with the avenues of the selected policy (a merge, not a sort — both
  inputs are already ordered). `composed_for` records which policy it was built for;
  it lives on `MapData` rather than in a system `Local` precisely because a city switch
  replaces the whole resource, and a `Local` would survive it and skip the rebuild.
  The set is **one value**, not the two `pub` arrays it used to be: positions and
  thresholds are private, the only door in is `TreeSet::push`, which takes a position and
  its threshold together, so "same length, same order" can no longer be broken from
  outside. The **prefix rule** lives on it too — `visible(density)` /
  `visible_count(density)` return the beginning of the set, never a filter, so a step up
  the density slider only adds trees and never moves the standing ones.

## Parsing details

In `references/parse.md`: the parse seam (`read_elements` / `finish_parse` and why the
order of the passes is their interface), every reading and finishing pass — building
height and use, the retail box, places of worship, the fortress, drowned buildings,
squared houses, houses pulled off the sidewalks, blocks pulled to the roads with the lot
paving (`pave_lots`), ring assembly — and **Testing the parse** (`fixture.rs::Overpass`,
the three routes a case can take).

## The shadow rules — `map/shadow.rs`

Seven layers cast a shadow — buildings, fences, cars, wagons, industry, bridges, roof
clutter — and three things are the same for all of them. Each used to be written out
wherever it was needed.

**The crowns are the eighth caster and stay outside on purpose** (`trees/crown.rs`):
their length is not an object's height run through `shadow_length_scale()` but a drawn
length of its own (`CrownParams::shadow_height_base`, the conifer cone's `3h`), and every
one of those is multiplied by `sun_stretch()` right where it is written. `shadow::length`
would say something different about them, so they call the sun directly.

- **`length(height)`** is the one place `shadow_length_scale()` is applied. The expression
  `shadow_dir() * height * shadow_length_scale()` existed in ten places, and the rule this
  file states — *"a shadow length written without `sun_stretch()` is a bug in the making:
  it will look right at the default and wrong at both ends of the slider"* — is now a call
  rather than something to remember. Clamps stay with their owner (`SHADOW_LENGTH_RANGE`
  for buildings, the roof edge for clutter) and are applied **after** it.
- **`offset(height)`** is that length as a vector. It is the displacement of the far end
  of a sweep, **not** where a silhouette is moved to: the shadow starts *under* the object.
  Cars, fences and the bridge each shipped the translated-copy version first, and at 15°
  a 2 m fence "moved" 7.4 m and read as a second fence.
- **`penumbra(direction)`** — the soft edge's share at a vertex: zero where the shadow
  meets what casts it, full at the far end, growing along a lateral side. It was written
  three times, once as a named function (buildings) and twice as a closure (fences, cars).
- **`push_union(builder, contours, blur)`** — the eighteen lines that were duplicated
  verbatim between `fences.rs` and `buildings/shadows.rs`, differing only in the blur
  constant: union the sweeps (`i_overlay`, NonZero — a translucent layer must never
  double on itself) and lay the tapered band outward from the outer ring and inward from
  each hole.

**Only buildings and fences go through `push_union`; the other five stay outside it, and
each for a measured reason.** The cars do not union at all (a 6 m pitch against a metre
of sweep, and `i_overlay` over 22 k cars would cost more than the layer); the bridge
tapers by `rise` rather than by direction, because a deck hangs in the air and its
penumbra is uniform all the way round; the roof clutter's shadow is **opaque**, drawn in
the roof's own colour inside the merged building mesh, so it has no band to lay; the
wagons get a translated quad, the cheap version the card asked for (a 13.9 m body at a
6 m pitch, and at 15° the copy does detach — the stated price); the industry writes its
circle sweep out by hand and lays it as a plain polygon, since a disc has no contour to
hand `i_overlay`. They call `length`/`offset` like everyone else — what differs is the
policy above them.

**The light stays a process global** (`map/sun.rs`, four `AtomicU32`) and that is a
decision, not an omission. Making it an argument would thread a parameter through every
`mesh_*` — the very functions the layer seam made callable from the game, a test and the
offline bench with one and the same call — and the global is what lets a build run on the
load thread, where there is no ECS at all. The price is known and written down: a test
with lit geometry takes the `default_sun()` / `sun_at()` guard and serialises on a mutex.

## The uniform grid — `map/grid.rs::Grid<T>`

Every "what is near this point" answer on the map comes from one type. The doors, tree
planting, the parse's sidewalk pull, landuse blocks and shift obstacles, water outlines,
road stitches, the fence gaps and street edges of `footprint.rs`, standing stock, garage
runs, house draw order, building shadows, the car districts and the bridge bands all
index the same way, and before `Grid` existed each of them wrote the arithmetic again:
the double loop "put it in every cell the box touches" existed in **nine** copies, the
key was `(i32, i32)` in ten places and `IVec2` in seven, and the cell size travelled as
an argument on every call — so an insert and a query could disagree about it and nothing
would say so.

- **The step belongs to the grid** (`Grid::new(size)`), which is what makes that
  disagreement impossible. It stays an argument rather than a module constant because the
  number is about the domain, not about the grid: 60 m for the doors' road index and 30 for
  their footprints (`ROAD_CELL` / `FOOTPRINT_CELL` in `osm/entrances/index.rs`), 48 for the
  shadows, 120 for the car districts.
- **Two primitives and three conveniences.** `cell(IVec2)` — one cell; `near_each(min,
  max)` — everything in the touched cells **as is**, duplicates included, in a fully
  determined order (cells ascending by x then y, insertion order inside a cell). On top of
  them: `at(point)` (one cell by a point), `near(min, max)` (sorted and deduped, needs
  `T: Ord`), and `pairs()` (every pair that shares a cell, sorted and deduped).
- **`insert(min, max, value)` puts the value in every cell its box touches**, and that
  invariant is what makes a one-cell `at` complete rather than approximate: the caller
  inflates the box by the reach it cares about, so any point the value has business with
  falls inside one of those cells. An error there returns a silently incomplete answer —
  which is exactly why it lives in one place now.
- **`insert_segment(from, to, pad, value)` is that insert for a link of a polyline**, and
  the box is the grid's arithmetic too: `from.min(to) - pad, from.max(to) + pad`. Twelve
  of the map's indexes wrote that line by hand — the doors (two of them), tree planting,
  water outlines, road stitches, the wagon fan, three of the parse's grids, the bridge
  bands and `footprint.rs`'s fence gaps and street edges — and a `pad` that drifts from
  the reach the query cares about is the same silent incompleteness as a disagreeing cell
  size. `pad` is `0.0` where the reach belongs to the query rather than to the value
  (`water.rs`, `entrances::RoadIndex`, the parse's sidewalk grid,
  `footprint::StreetEdges`), and it is a **scalar**: `Vec2 - f32` is glam's own, so no
  caller writes `Vec2::splat(reach)` any more.
- **`near` sorts because the mesh must not move between runs**, not for the caller's
  convenience: a value sits in several cells, so without `dedup` a neighbour comes back
  several times, and without the sort the `HashMap` iteration order leaks into the
  geometry. `near_each` is the escape hatch for values that are not `Ord` (`water.rs`
  indexes `(Vec2, Vec2)` edges) and for callers that already tolerate duplicates.
- **The map is `bevy::platform::collections::HashMap`, not `std`'s** — the same
  `hashbrown`, with `foldhash` in place of SipHash. A city-wide index is tens of thousands
  of inserts (the stitches alone index 14 642 buildings and water bodies), so at that
  traffic the hash *is* the price of the index, and an `IVec2` key has no use for a
  DoS-resistant one. It cost one line and about **100 ms of a world load**: trees 286 →
  246 ms, entrances 62 → 40, `pave_lots` 116 → 98, the sidewalk pull 64 → 52, `mesh_roads`
  89 → 80 — fifteen indexes ride on this type. The map's own keyed maps followed
  (the shared road nodes, the junction and corner nodes, the parking grid, `Occupied`,
  the arches and the garage runs): trees to 206, `mesh_roads` to 76, the parking layout
  29 → 24. **The swap also removes a nondeterminism rather than adding one**: std seeds
  SipHash per process, so a map iterated for its values handed out a different order every
  run, while bevy's `FixedHasher` has a fixed seed. Still on `std`: the tag maps of the
  parse and the Overpass reader, which serde deserializes into.
- **`cell_of` is public for one caller**, `entrances::RoadIndex`, which walks cells in
  **rings** outward from the point and stops as soon as what it found beats anything the
  next ring could hold. That strategy belongs to it, not to the grid.
- **`planting::Occupied` keeps a grid of its own**, on the opposite convention: it takes a
  bare point and the asker carries the radius, so a query walks the 3×3 neighbourhood
  (`osm/planting/index.rs`). A "neighbourhood" method for its one caller would be a
  hypothetical seam, not a seam.
- **`spatial.rs` is not this grid and does not move here.** The pawn grid is a dense `Vec`
  over the whole map with a reverse entity→cell index and a per-tick move of one entity at
  a time; it shares nothing with a `HashMap` of boxes built once per load but the word.

## The shape vocabulary — `map/shapes.rs`

Everything the map says to `i_overlay` it says in one vocabulary, and it lives in one
module the way the walk lives in `map/along.rs` and the RNG in `map/seed.rs`: `Contour`
(`Vec<[f32; 2]>`) and `Shape` (outer ring, then holes), the offset rounding `ARC` 0.3,
the ring tolerance `RING_EPSILON` 0.01 with `is_ring`, `oriented` / `area_contours` (the
CCW-outer, CW-hole winding NonZero needs), `contour_area` / `shape_area` /
`contour_bounds`, `ring_of` and `point_in_shape`, `stroke` and `push_shape`.

Three modules speak it — the parse's lot paving (`osm/parse/lots.rs`), the big lot's kerb
(`roads/lots.rs`) and the gores (`roads/gores.rs`) — and **each of the last two arrived
with its own copy of the set**, `oriented` byte for byte identical and the two `stroke`s
already parted in their return type. That is the defect `map/seed.rs`, `map/grid.rs` and
`shadow::push_union` exist against, stated once more for polygons. It also unknots
`roads/`: eight of these names were `pub(super)` in `lots.rs` for the sole benefit of
`gores.rs`, so the two modules imported each other in a circle although not one of the
names is about a parking lot. The one direction left is `lots → gores` (the kerb subtracts
the islands), which is what both modules' doc comments already claim.

**`stroke` takes `ring` as an argument rather than asking `is_ring` itself**, and that is
the one place the two callers genuinely differ: a road ring is stroked closed, without
caps, while the parse strokes a run of road pieces open even when the run happens to come
back on itself.

## Footprint bands

`map/footprint.rs` — the strips linear geometry occupies on the ground, one construction
for every consumer: `Band { line, width, role }` built by `RoadLine::{deck_band,
curb_bands, passage_band}`, `WaterLine::channel_band` (`None` for culverts) and
`WallLine::band`, `FenceLine::band` (plus `fence_gaps`, the openings in it). The width policy lives here too — `casing_width` (8%, 0.3–1 m) and
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
through the curb pin tests (`navmesh/fill/tests.rs`) and the parity tests.

## Rendering

- **One RNG and one point seed for the whole of `map/*`** (`map/seed.rs`) — everything a
  layer scatters must survive a rebuild: a zoom-bucket crossing, a height-mode switch, a
  restart. So the layers share two primitives instead of copying them. **`Lcg`** is the
  Park–Miller (Lehmer) generator of `Village.js` — `seed = 48271·seed mod 2³¹−1`, plus
  `range`, `gauss3` (a bell on (0,1)) and `bell4` (a bell on (−1,1)); it started in the
  crown generator and was lifted out when the parked cars became its third caller.
  `next_f32` ends on a **clamp to the largest `f32` below one**: the plain
  `self.0 as f32 / 2_147_483_647.0` rounds up to exactly `1.0` on 63 of the two-odd billion
  states (p ≈ 2.9·10⁻⁸), and that is enough for `range` to land on `to` itself. The clamp
  leaves every other value bit for bit what it was, so no city is reseeded; dividing in
  `f64` was tried and **rejected** — it still yields `1.0` (on 64 states) and shifts the low
  bit of 1.3 % of the rest. **`range` is therefore not half-open, only almost**:
  `from + t·(to − from)` rounds up to `to` on its own, so a caller that needs `< to` clamps
  at the call site — the wagon rake is the one that does.
  **`seed_from_point(Vec2)`** is the seed itself: the object's **own reference point** in
  centimetres — the first vertex of a footprint, the first point of a street — through three
  mixing rounds, never the object's index in the extract, which a re-parse is free to move.
  Callers: `buildings::material::building_seed` (the roof material, the roof shape and, since
  the height inference, the storeys), `buildings::clutter`, `trees::crown`, `cars`, `wagons`.
  A copy of either primitive is the defect this module exists against, and the wagon layer
  arrived with both — a private `Lcg` and a `track_seed` two rounds short of
  `seed_from_point`, under a doc comment claiming it was the streets' own seed.
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
  ignored — and they call exactly the builders `mesh_buildings` calls. **Absolute
  numbers still depend on the machine's power state** (with the display asleep everything
  is 2–3× slower), so compare runs, not runs against the log.
  **Shadows are measured at the default sun.** The sweep length and direction come from the
  process global of `map/sun.rs`, which in the app only `apply_sun` writes; the bench has no
  app, so it sets the sun itself — `map::apply_sun_style(SunStyle::default())` — and prints
  the azimuth/elevation in its header line. The elevation drives `sun_stretch`, i.e. the
  sweep length and the union's area, so a run that did not state its sun would not be
  comparable with the next one; the live app builds with the sun from `settings.toml`, which
  is a second reason a log line and a run are not comparable.
  **What it covers besides the buildings and the cars**: `measure_surfaces`,
  `measure_roads`, `measure_rails` and `measure_tram` — six measurements against eleven
  `mesh_*` doors. **Unmeasured: `fences`, `wagons`, `industry` and the trees** (both
  `mesh_trees` and `spawn::mesh_tree_row_band`). The first three are cheap single-pass
  layers with no zoom ladder worth a row per step, and the trees want a different row
  shape altogether (its own sub-bullet below); none of the four is a decision recorded as
  final — a row for any of them is a `measure_*` plus a `row(...)` line in the bench.
  Those four that exist are of a different kind from the two above, and the
  difference is the whole payoff of the seam: they have **no build of their own**. Each
  calls the game's `mesh_*` once and lays its layers out through
  `surface::layer_costs(&layers, report.elapsed)` — the helper *and* its `LayerCost` row
  both live in `map/surface.rs`, i.e. on the seam rather than in `buildings`, where the
  row was first needed — a first row named `build`, carrying the module's
  milliseconds (the build is one pass; there is nothing to split them between) and then a
  row of vertices per layer, under the same `name` the layer wears in the live world.
  `measure_layers` and `measure_cars` repeat their build's steps deliberately, because a
  bench row per step is exactly what they exist for.
  - **Rails and tram get a row per zoom bucket** (`ZoomBucket::at(index)`, which exists
    for this). Their buckets differ in *what is drawn*, not in size — rails run 45 k
    vertices on the far step against 663 k on the near one (44 563 and 662 939, Tula,
    `dev`) — so a single number would be a number about nothing. Those are the **bench's**
    numbers; the 673 k / 23 ms that appears elsewhere in this skill under **What a bucket
    costs** and beside it comes off the app's `rail meshing:` line, which App Nap decides,
    and the two are not comparable — that is the whole reason the bench exists.
  - **The tram is measured switched on**, though it ships off: the bench is about what
    the layer costs, not about whether it is shown.
  - Trees are the gap left: their build is a crown pool plus a scatter, and the
    `LayerCost` row (a name and a vertex count per layer) has nothing to say about
    15 k entities. Measuring them wants its own shape, not a sixth `measure_*`.
- **Merged meshes** (`map/meshing.rs` + `map/spawn.rs`, water and waterways in
  `map/water.rs`, road layers in `map/roads.rs`,
  rail layers in `map/rail.rs`, the tram layer in `map/tram.rs`, building layers in
  `map/buildings/`) — **one merged `Mesh2d` per layer** (ground, parks, water, waterways,
  sidewalks, alleys, roads, rail layers, tram, building layers, walls): `MeshBuilder`
  triangulates polygons via `earcutr` (holes supported, degenerate contours skipped +
  counted) and emits per-vertex colors. Facade, shadow, casing, rail and wall layers go
  over a single white `ColorMaterial`; every layer carrying a roof (`building_roofs`, and
  in 2.5D `building_extruded`, walls included) over the `RoofMaterial` of **Roof
  material** (`references/buildings.md`); the **surfaces** — ground, area fills, water, road and
  alley fills, sidewalks, parking asphalt (as `SurfaceKind::Street` — it *is* asphalt),
  the tree-row band — over the `SurfaceMaterial` below. The parking **markings** are the
  exception that proves the rule: paint over asphalt, so that layer stays on the flat
  `ColorMaterial`. ~7000
  buildings cost a handful of entities. Trees stay individual entities (see
  `references/trees.md`).
- **The layer seam** (`map/surface.rs`) — building a layer and putting it in the world
  are two things, and this is the line between them. A **converted** module offers one
  pure function, `mesh_<layer>(data, style) -> (Vec<LayerMesh>, <Layer>Report)`, and
  its system is a thin adapter: despawn by tag, call it, hand the list to
  `surface::spawn_layers`, print the report.
  **A layer with a second caller gets one door for both** — `spawn_<layer>_meshes(commands,
  meshes, materials, mesh_<layer>(...))`, taking what is already built so the build stays
  outside the world. `buildings` and `roads` have one each (`spawn_building_meshes`,
  `spawn_road_meshes`), because `spawn_map` spawns their layers at world entry and a
  `rebuild_*` respawns them on a style or bucket change. The trigger is the **number of
  callers, not the number of tags**: without the door `spawn.rs` would have to know the
  layer's tag and the "spawn, then `info!`" order. A module with one caller (the surfaces)
  needs none.
  - **`LayerMesh`** — `{ builder, z, name, material: MaterialSpec }`, **one type for
    every layer of the map**, not a type per module. That is the point: a module read as
    `-> Vec<LayerMesh>` is read the same way as any neighbour. `name` is the entity's
    `Name` in the live world, i.e. what a BRP query looks it up by.
  - **`MaterialSpec`** — `Flat` / `Blend` / `Surface(SurfaceKind)` / `Roof`. It **names** the
    material instead of carrying a `Handle`, and a handle is the only thing that would
    have dragged Bevy into the build: with a spec the build needs neither `Commands` nor
    `Assets`, so the game, a test and the offline bench call one and the same function.
    Resolving spec → handle lives in `spawn_layers` and only there.
  - **`FlatMaterials`** (`Startup`, beside `SurfaceMaterials`) holds the two flat
    `ColorMaterial`s the spec names. An **unconverted** module did
    `materials.add(...)` on every rebuild — a per-rebuild allocation of a material that
    never changes — and the conversion is what retired the last of them. Both resources
    reach an adapter as one **`LayerMaterials`** system param (`#[derive(SystemParam)]`,
    the `ui/debug/mod.rs::DebugValues` idiom), which is also where `resolve` lives. Two
    separate `Res` were tried first and pushed `rebuild_tram` and `rebuild_industry` to
    eight arguments, past clippy's limit; bundling them left every adapter shorter than
    it had been before the seam.
  - **A zoom cutoff or a visibility toggle belongs in the build, not in the system.** A
    far-bucket fence, an invisible tram, an invisible industry layer all return **nothing
    to draw**, so the despawn in the adapter is unconditional — and that comes in two
    shapes, both pinned by tests:
    - an **empty list**, when the cutoff also saves work before the build — the fence's
      far bucket returns `Vec::new()` and never calls `fence_gaps` (8.7 ms on Tula), and
      `mesh_cars` off or past the last step does neither breaks nor districts
      (`fences/tests.rs::the_far_bucket_draws_nothing`);
    - the module's **usual layers with empty builders**, when there is nothing to save and
      the layer order is worth seeing in the test — the tram, the industry layer
      (`tram/tests.rs` and `industry/tests.rs::the_toggle_off_draws_nothing`).

    **A stepped layer takes the `ZoomBucket` itself, never a value already unrolled from
    the LOD table.** `mesh_rails`, `mesh_tram`, `mesh_wagons`, `mesh_cars` and — last to
    follow — `mesh_fences` all take the bucket and read their own table inside the door.
    Handing the build a bare width instead (`FENCE_LODS[bucket.index].width`, as
    `rebuild_fences` did) leaves half the cutoff in the system: the test then has to index
    the table by hand to say which step it means, and the door can be called with a width
    no step of the ladder ever produces. With the bucket a test says `for_zoom(MAX_ZOOM)`
    and asserts on the report (`FenceReport::width == 0.0`), which is where the layer
    carries `hidden`.

    Either is safe, because `surface::spawn_layer` skips an empty builder anyway. That is
    the very thing the builds' own doc comments now say from the other side —
    «второй дороги, на которой можно забыть деспавн, нет» (`cars::mesh_cars`,
    `industry::mesh_industry`) — a property of the shape rather than a thing to remember.
    Do not confuse it with **«одно условие — одна регистрация»**, which each layer's
    `rebuilds_on` states and `map/mod.rs` refers to: that one is about **double
    registration spawning a layer twice**, which the seam neither removes nor touches. It
    also makes the toggle testable: it used to live behind a `return` inside a Bevy
    system, where no test could reach it.
    Whichever shape a module takes, it says so in its **report** — see **"The layer is not
    drawn" is a state of the report** below; the shape decides what is in the list, the
    report decides what the log line says.
  - **The report is a value, not a log line.** `FenceReport`, `RailReport`: the counters
    `info!` used to be made of, returned so a test can assert on them. `info!` is also
    the thing App Nap mismeasures on macOS, so a returned `elapsed` is the only honest
    one. A module that logs nothing gets no report — `spawn::mesh_tree_row_band` returns
    a bare `Vec<LayerMesh>`, deliberately; inventing a report for symmetry would invent
    a number nobody reads. **The reports are not symmetrical with each other, and are
    not meant to be.** `FenceReport` is `Clone, Copy, PartialEq, Debug` — every field of
    it is a number, so the derives cost nothing; `BuildingReport` derives nothing, and
    cannot: `Copy` is out (two `String` fields) and no test compares it. Add a derive
    when something uses it, not for the symmetry; a failure message wants `Display`
    anyway, which every report has and which prints the log line itself.
  - **"The layer is not drawn" is a state of the report, never a zero in a counter.** A
    build that the toggle or the zoom step took away must not print what an empty city
    prints: `industry: 0 structures, 0 pipes` was the line a map with a chimney on it
    logged, and nothing in it said which of the two had happened. So the state is a
    **field**, and the counters keep saying what came in:
    - the field is `hidden: bool` (`IndustryReport`, `TramReport`, `WagonReport`) or an
      existing one that already carries the fact — `CarReport::detail: Option<CarDetail>`,
      `FenceReport::width` (zero *is* "not drawn at this step"). A second `hidden` beside
      `width` would be two sources of one fact;
    - `Display` then prints `<layer>: hidden`, with the free input counters after it
      (`industry: hidden (1 structures, 0 pipes)`, `tram meshing: hidden (1 tracks)`,
      `fences: hidden (429 lines)`) — that parenthesis is what tells the hidden layer
      from the empty city at a glance;
    - **free** means the count is a slice length or a filter over the input
      (`mesh_tram` walks `rails` and counts `RailKind::Tram` whatever the toggle says;
      `mesh_industry` counts its own `structures`/`pipe_lines`, and only the loops move
      to the substituted `drawn_*` slices). A counter that would need the build to run
      stays zero, and that is honest rather than a stub — `WagonReport::standing` and
      `CarReport::cars` are zero because the placement genuinely does not run, which is
      the whole point of taking the cutoff before the build (`fences::pieces` likewise:
      `fence_gaps` is 8.7 ms).

    Pinned by `industry/tests.rs` and `tram/tests.rs::the_toggle_off_draws_nothing` (the
    input counter stays 1 and `hidden` is true) and by
    `fences/tests.rs::the_far_bucket_draws_nothing`, each asserting the log line itself.
  - **Converted — ten modules, eleven layer doors.** `fences`, `rail`, `tram`, `wagons`,
    `industry`, `cars`,
    `roads` (20 layers, `mesh_roads` — ten of its own with the median lawn
    `road_medians`, the eight paint layers of `roads/paint.rs` (the turn paths' wear mask
    and apply among them, and the roundabout islands' hatching above a lot's asphalt) and
    the two over a big lot,
    `roads/lots.rs`), all of `spawn.rs` (the 13 surface and paint layers
    as `mesh_surfaces(map, parking_layout) -> (Vec<LayerMesh>, SurfaceReport)` — the
    parking layout arrives ready, because the car rows are drawn off the same one — plus
    the tree-row band, its **second** door), `buildings` and `trees`.
    `surface::spawn_layer` (one layer, a
    ready `LayerMaterial`) survives only as the primitive `spawn_layers` is built on.
    The two counts are different numbers and both are worth having: `grep 'pub fn mesh_'
    src/map/` gives eleven doors, the module list gives ten — `spawn.rs` carries two
    (`mesh_surfaces`, `mesh_tree_row_band`). **Count the modules when asking "is anything
    left".** The tree-row band lives in `spawn.rs` and is not
    `trees` — that mistake is what once made the list read "all ten" with `trees.rs`
    still spawning by hand.
  - **`trees` is a scatter, and the seam takes a different shape there.** A crown is an
    **entity per tree** — its own tint, its own micro-step of z, its own scale — so it
    does not fit a `LayerMesh` at all, and `mesh_trees(style, params, planted, field)`
    returns `TreeMeshes { pools, tints, crowns, shadows }` instead:
    - **`pools`** — the crown meshes, `TREE_VARIANTS` of them per concrete shape (`Mixed`
      has two pools, every other shape one), as plain `Mesh` **values**. A
      `Handle<Mesh>` would be the world, which is exactly what `MaterialSpec` keeps out
      of a build; the adapter uploads the pool to `Assets` and nothing else changes.
    - **`crowns`** — `CrownPlacement { at, radius, z, pool, variant, tint }`, one per
      drawn tree. This is what the conversion actually bought: the density prefix
      (`TreeSet::visible_count`), the species resolve off the conifer field, the tint slot and the
      z micro-step were all inside a Bevy system and unreachable from a test.
    - **`shadows`** — the one merged shadow mesh, an ordinary `LayerMesh` at
      `Z_TREE_SHADOW`. Its colour moved **into the vertices** (`shadow_template` pushes
      `SHADOW_COLOR`) so the layer can be a plain `MaterialSpec::Blend`, the way every
      other shadow on the map already was; before that the layer allocated a coloured
      `ColorMaterial` on every rebuild. `tree_gallery` lays its own grid and therefore
      does not go through `spawn_tree_meshes`, but it had to follow the colour: its
      shadow material is now a blended white one.
    So `spawn_tree_meshes` is the adapter, and it is the **one** place on the map that
    still writes `DespawnOnExit` by hand — for the crowns. Every merged layer gets it
    from `spawn_layer`.
  - **`cars` is the one whose build is a layer rather than a mesh.** Every other
    `mesh_*` takes the data it draws; `mesh_cars(bucket, style, smoothing, map, layout)`
    takes the whole `MapData` (as `mesh_roads` does) and does the **assembly** as well —
    junction breaks, `Districts`, `park_cars`, `fill_lots` — because that assembly is
    exactly what the cutoff and the toggle gate. Off, or past the last zoom step, none
    of it runs and the list is empty, so a hidden layer still costs what the old
    `return` inside the system cost. The private `mesh_bodies(&[Car], CarDetail)`
    underneath is only the mesh; it carried the name `mesh_cars` until the layer
    function took it. `CarReport::detail` is an `Option<CarDetail>`, and `None` is what
    the `cars: hidden` log line prints — every other counter is then zero, because the
    assembly those counters would count is exactly what did not run. That is the one
    shape of the rule above (**"The layer is not drawn" is a state of the report**); the
    four modules whose input counters are free print theirs beside the word.
    **The bench and the gallery still assemble on their own, deliberately**:
    `measure_cars` times `breaks` / `districts` / `parking` as separate rows and meshes
    all three detail steps, which one call cannot report — the same reason
    `buildings::measure_layers` repeats the steps `mesh_buildings` takes; `cars_mesh` is
    the gallery's one door and builds with neither lots nor districts on purpose.
  - **Buildings was last, and not for being big.** Two things are peculiar to it and
    worth knowing before touching it:
    - it is the only module needing **`MaterialSpec::Roof`**, and the variant was added
      exactly when it arrived — before that nothing could construct it;
    - it spawns under **two tags** (`BuildingLayerTag`, and `BuildingShadowTag` on its
      own rebuild schedule — the zoom bucket does not touch the shadows), while
      `spawn_layers` takes one tag per call. Hence `BuildingMeshes { layers, shadows }`
      and two calls, wrapped in `spawn_building_meshes` so `rebuild_buildings` and
      `spawn_map` share one door. Carrying the tag *inside* `LayerMesh` was rejected: a
      tag is what the **adapter** despawns by, not a property of what was drawn.
    The local closure in `spawn_buildings` that shadowed the name `spawn_layer` is gone
    with the conversion; it now just pushes into a `Vec<LayerMesh>`.
  - **The 13 surface layers still spawn with tag `()`**, and after the conversion that
    is visible as one line in the list rather than a silent argument. They are not
    rebuilt by anything, so they have nothing to be found by; giving them a tag is worth
    doing together with a reason to rebuild them, not before.
  - **When a layer rebuilds is the layer's own business** — `rebuilds_on()`, a run
    condition next to its `rebuild_*`, and `map/mod.rs` only wires it
    (`roads::rebuild_roads.run_if(roads::rebuilds_on())`). The reason a gate lists what it
    lists is a fact about the layer: `roads` carries `SunOnMap` **because the bridge
    shadow is baked into its mesh**, and that is something you need to know while editing
    `roads.rs`, not while reading the plugin. Before this it was a ten-line comment in
    `map/mod.rs`, a file the layer's author has no reason to open.
    **One layer takes its condition from another module**: `trees::rebuilds_on()` gates
    the whole four-system chain — `recompose_row_trees`, `retune_conifer_field`,
    `spawn::rebuild_tree_row_band` and `rebuild_trees` — so the tree-row band, which
    lives in `spawn.rs` and is not `trees` (see **Converted** above), is gated from
    `trees.rs`. What rebuilds is the chain, so the chain is what the condition belongs to.
    - **One condition, one registration**, and it is written on every one of them. Two
      copies of one system in one schedule can both fire in a frame: the second one's
      despawn runs against data taken before the first one's commands were applied, and
      the layer spawns twice. That is not theory — the industry layer arrived with
      `rebuild_industry` listed twice. So conditions are summed with `or_else` rather
      than split across registrations, and the rule now has a single place to live
      instead of the three comments that used to repeat it.
    - `rail::rebuilds_on` goes through `IntoSystem::into_system` because a bare
      function-condition carries its own type marker; the other layers' `or_else` erases
      it. That is the only oddity in the shape.
    - **A full layer registry was considered and rejected.** Declaring a layer as data —
      tag, triggers, build, z, material — and letting one generic system register it
      would close the double-spawn trap by construction, and the trap is real. It would
      also cost an associated-type-per-layer trait and one indirection between "what is
      drawn" and "when", to replace eleven adapters of about ten lines each. Most of what
      that card was written against is already gone: the per-rebuild material
      allocations, the redundant `is_empty` guards and the two modules writing
      `DespawnOnExit` by hand all went with the seam itself. What was left was the gates'
      prose living away from its layer, and that is what `rebuilds_on` fixes.
  - **Converting a module** means: lift the build to `mesh_*` returning
    `Vec<LayerMesh>`, derive `Clone, Copy` on its `*LayerTag` (`spawn_layers` hands the
    tag to every layer), move any cutoff or toggle into the build — a zoom ladder as the
    `ZoomBucket` itself, not as a width read out of the table by the adapter — drop its
    `materials.add(...)` and its now-redundant `is_empty` guard, write its `rebuilds_on()`
    beside the `rebuild_*` (**When a layer rebuilds** above), and write the tests the seam
    has just made possible. Do not add a `MaterialSpec` variant before a module
    needs it — the `Surface` one sat unconstructed until the tree-row band arrived, and
    the compiler said so. A module whose entities are **not** one merged mesh per layer
    (so far only `trees`) returns its own struct instead of a bare `Vec<LayerMesh>`, and
    the rule that survives is the division, not the return type: the build says what is
    drawn, the adapter says where it goes.
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
  mottle slides with `globals.time` — only Water), **shore** (`shore_color` /
  `shore_width` — only Water: the channel ribbon's shoal by its `across`, see
  **Waterways**), and the **wear** block (Street — wheel ruts on the lane frame; a bridge
  deck is the same kind and carries its street's ruts, a footbridge in the same mesh has
  no lane frame and stays bare). The lane **lines** are not this shader's any more: they
  are the paint layer (`roads/paint.rs`, `paint.wgsl`, `references/roads.md`). The zoom
  rule is one function,
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
  lives in `paint.wgsl` and `band` (the drive between garage rows) in `roof.wgsl`,
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
  in metres: `[across, to-break, half width, 0]` on a ribbon without lanes, `[across,
  to-break, low, high]` from the lane grid node on a carriageway with a **lane frame**
  (`MeshBuilder::set_lanes` / `set_lane_taper`, `meshing::LaneFrame`); *to-break* is the
  signed distance to the nearest break — a marking break on a street, a mouth on a
  channel) and a mesh gets it only from `MeshBuilder::with_surface_coords()`;
  `push_ribbon` / `push_ribbon_shaped` fill it from the ribbon frame (quads: ±half width;
  join fans: the outer side; round caps: the projection onto the normal, with *to-break*
  extrapolated past the node along the last quad's slope), polygons get zeros. It costs
  16 bytes per vertex, which is why building, crown and overlay meshes are built without
  it. Where the breaks come from and why the mesher inserts a vertex at every kink of
  *to-break* is under **Markings → Breaks** below.
- **The layers themselves are in the references**, one file per subject, so a session
  reads only its own. A cross-reference «**X** below/above» inside any of them names a
  bullet by its bold title; this list says which file holds it.
  - `references/roads.md` — **Sidewalks**, **Paired halves** (the median: asphalt and
    double solid, or a lawn with a kerb), **Markings** (with **Breaks** and
    `lane_count`), **Ribbon**, **Junctions**, **The drawn network** (pinned nodes,
    driveway crossings, stitches, **Kerb returns**), **RoadStyle**, **Bridge layers**,
    **Asphalt wear**, and **The junction gallery** (`examples/demos/roads`).
  - `references/parking.md` — **Parking** (the layout, aisles, pockets, **A big lot shows
    the road through it** with the lot kerb, the medians and the **gores**) and
    **Parked cars** (rows, junction breaks, districts, bodies, `CarStyle`).
  - `references/layers.md` — **Rims**, **Shoal**, **Waterways**, **The yard**,
    **Pitches**, **Rail layers**, **Tram**, **Industry**, **Standing wagons**, **Fences**.
  - `references/buildings.md` and `references/trees.md` — **Buildings** (below) and the
    trees.
- **Zoom buckets** (`map/zoom.rs`) — the one mechanism behind every zoom LOD. Each
  layer keeps its own table (`RAIL_LODS`, `TRAM_LODS`, `FENCE_LODS`; the cars' and the
  roof clutter's are a single threshold each) and names it with a marker type
  implementing `ZoomLods` (`RailLods`, `TramLods`, `FenceLods`, `CarLods`,
  `BuildingLods` — empty enums handing over the `max_zoom`s). `ZoomBucket<T>` is the
  resource with the current index for that table;
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
- **Buildings** (`map/buildings/`) — the layer set a house is drawn as, and the largest
  subject of this section; the detail lives in its own reference. Which file builds what:
  `mod.rs` picks the layers and builds them (`mesh_buildings`, `rebuild_buildings`,
  `spawn_building_meshes`), `layers.rs` the facade band, the roofs and the 2.5D walls,
  `shadows.rs` the two shadow layers, `roofs.rs` the pitched and the landmark roofs,
  `temples.rs` + `fortress.rs` the churches and the kremlin, `heights.rs` the storeys
  inferred where OSM carries no height, `material.rs` (shader `assets/shaders/roof.wgsl`)
  the roof and wall material, `clutter.rs` what stands on a roof, `garages.rs` the garage
  rows, `arches.rs` the passage openings, `order.rs` the painter's order the whole thing
  is drawn in. Which of the layers exist at all is **`BuildingHeightMode`** (resource,
  BRP-writable, persisted; section `ui/buildings.rs`, Map tab below Trees and Tree rows,
  one cycling row) — `Facade`, `Shadows`, `ShadowsTint`, `Extrusion`,
  `ExtrusionShadowsTint` (the default); any change reruns `rebuild_buildings` (despawn
  `BuildingLayerTag` layers, respawn from the unchanged `MapData::buildings`).
  **The mechanism, the measurements and the rationale are in `references/buildings.md`**:
  the height modes and their shadow sweeps, temples and the fortress, inferred storeys,
  the roof and wall material with its shader and its two galleries, the roof clutter, the
  arches.
