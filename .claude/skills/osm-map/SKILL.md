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
  `man_made=pipeline` (way only). The bbox is `MAP_SIZE` around the selected
  `City`'s geo center. `QUERY_VERSION` is **14** (v3 added `entrance` nodes, v4 `railway`,
  v5 `natural=tree_row`, v6 `natural=tree` nodes, v7 linear `waterway`, v8 `landuse`
  blocks, v9 `amenity=parking`, v10 the `leisure` pitches and playgrounds, v11 the
  industrial `man_made` cylinders and pipelines, v13 `driving_side`, v14 the fences;
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
  Commercial | Industrial | Garage | GarageBlock | Church(Sacred) | Public | Other`, the class that
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
  the roof shape is inferred instead — see **Gable roofs** under Rendering — with one
  exception below. The class is
  also one of the two inputs of **Inferred storeys** (the other is the footprint's shape),
  which is what fills in the height OSM does not carry.
- **Places of worship** (`parse/tags.rs::faith`, `sacred_form`; `parse.rs::resolve_faiths`)
  — `BuildingUse::Church(Sacred { faith, form })`. `building=bell_tower|campanile|minaret`
  and `tower:type=bell_tower|minaret` are churches too (`form: Tower`); a part with
  `roof:shape=onion|dome` is `form: Dome` — the only reading of `roof:shape` in the parse.
  **Tagged colours** (`tags::area_colours` → `PolyArea::colours`, every building, not only
  churches): `building:colour` and `roof:colour` as sRGB bytes. `tags::colour` takes a hex
  value as is — the mapper picked it off a photo — and a **CSS name as the map's own paint**
  (`CSS_COLOURS`: `white` is whitewash, `blue` the roof palette's blue, `darkgray` darker
  than `gray` as a mapper means it, not lighter as CSS has it), because `#0000FF` on the map
  is a marker, not paint; an unknown name is `None`, never a guess. Tula: 30 `building:colour`
  and 91 `roof:colour` on 7.7 k buildings, 51 of them `blue`; on the churches — `#FFD700` on
  both kremlin-cathedral drums, `#5D948F` on the All Saints cathedral and its bell tower,
  `blue` on Свято-Никольский, `red` / `green` on the arms museum annex. **Only the temples
  read them so far** (`temples::tagged_wall` / `tagged_roof` / `tagged_dome`, below);
  reading them on every house is a palette decision the private sector has not made.
  Faith: `religion=christian` + an Orthodox-family `denomination` → `Orthodox`, any other
  denomination → `Western`, none → `Unknown`; `muslim|jewish|buddhist|hindu|shinto|…` by
  religion, else by `building=mosque|synagogue|temple`. Tula v14: 27 places of worship —
  17 `russian_orthodox`, 3 `orthodox`, 1 catholic, 1 evangelical, 1 `christian` bare, 4 with
  no religion (the kremlin cathedral's parts: 9×9 onion 35 m, 11×11 dome, 26×25 onion 30 m,
  the 70 m bell tower). **`resolve_faiths`** runs right after the drowned-building pass and
  assembles churches from parts: a part's **host** is the largest larger church holding its
  centre, else the nearest larger one within `CHURCH_PART_REACH` 30 m (a bell tower stands
  beside, not inside). Hosts are followed **up the chain**: a bell tower whose nearest larger
  church is a part of a cathedral (a part sticking out past the outline, or tied with it on
  distance) belongs to the cathedral. The part takes the top host's first-vertex seed as
  `Sacred::complex` — what its colours are picked by — and, when untagged, the faith of the
  nearest host up the chain that has one; the rest take the
  city majority (Orthodox vs Western, ties → Western). `Sacred::floor_dm` is `min_height`,
  else `building:min_level` × 3 m (`tags.rs::part_floor`).
  **Annexes** (`parse.rs::absorb_annexes`, same pass, after hosting): a `BuildingUse::Other`
  building of `AreaKind::Building` that lies on a church **or an annex** — its centre in the
  host, the host's centre in it, or `ANNEX_OVERLAP_SHARE` 25 % of its own footprint
  shared (`overlap_area`, an `i_overlay` intersect) — at most `ANNEX_AREA_RATIO` 2.5× the
  host, becomes `SacredForm::Annex` with the largest such host's faith and complex, in up
  to `ANNEX_ROUNDS` 3 rounds (the apse part below shares under a quarter with the
  cathedral itself and lies on the museum instead).
  Only `Other`: a house, a school or a shop keeps its class however it lies. Tula: the arms
  museum mapped over the Epiphany cathedral (45×39 vs 37×35, `building=yes`) and the
  cathedral's apse part (20×24, centre outside, half inside) were two apartment boxes with
  windows the cathedral's cupolas stuck out from behind.
- **Fortress** (`parse/tags.rs::is_fortification`) — `AreaKind::Kremlin` from
  — on an outline carrying `building` only (`area_kind` asks nothing else) —
  `historic=citywalls|castle|city_gate|fort`, `barrier=city_wall`,
  `man_made=tower` + `tower:type=defensive`, or `building=wall` at ≥ 6 m
  (`FORTRESS_WALL_MIN_HEIGHT`; lower is a garden wall). Tula carries **no** `historic` on
  its kremlin — before this every tower and the wall were plain buildings with windows.
- **Drowned buildings** (`parse.rs::drop_buildings_in_water`) — a building whose outline
  lies **entirely** inside a water polygon is dropped right after the element loop, before
  doors and trees. OSM tags floating restaurants and moored ships as buildings (`HMS
  Belfast`, `Café Barge`) and Tula carries a lone shed in the middle of Верхний пруд; the
  navmesh floods water impassable, so their doors are unreachable anyway and the box
  standing on the pond reads as a render bug. One vertex on land is enough to survive —
  piers and embankment houses stay. Counts: Tula 1, Berlin 6, NY 17, London 28, Paris 28,
  Tokyo 0; logged on stderr when non-zero.
- **Squared houses** (`parse.rs::square_skewed_houses`) — a small house outlined as a
  **skewed quad** is replaced by a rectangle. The private sector is traced by eye off
  imagery, and a rectangular house comes out a rhombus (Tula way 968419942, corners
  79°–100°): in 2.5D its ends stand askew to its front and no gable fits it. Taken: a
  4-vertex convex outline, `AreaKind::Building`, no holes, an `Other` of at most
  `SQUARE_AREA_MAX` 250 m² (the pitched-cohort threshold) or a `House` of at most
  `SQUARE_HOUSE_AREA_MAX` 400 m² (the tag already says private house, and a house is
  pitched at any size — Tula way 968378335, 348 m² at 21°), with its worst corner
  `SQUARE_SKEW_MIN` 2° … `SQUARE_SKEW_MAX` 35° off square — under 2° the trace is
  already straight, over 35° it is a trapezoid by the plot. The ceiling was 20° first and
  a screenshot of Tula's private sector (around `cam 641 3539`) showed seven lone houses
  left crooked at 20.4°–32.3°; measured on the cache, **every** lone small quad above 20°
  was such a trace (10 of them, vertex shift ≤ 1.7 m), none a real trapezoid — the
  shift cap is what guards the rest. The rectangle keeps the
  **centroid and the area**: the axis is the length-weighted mean of the edge directions
  with the angle ×4 (so both axes vote for one), the sides the mean lengths of opposite
  edges along it, scaled to the area; vertex `i` becomes corner `i` with the winding kept.
  **A skewed L is straightened too** — a six-vertex outline with exactly one reflex corner
  (a house with a wing, Tula ways 968378349 / 968378329, corners up to 17°–20° off), under
  the same area, skew and shift gates. `fit_ell` uses the same `outline_axis`; every edge
  goes to the nearer axis (they must alternate, or it is not an L) and gets a **level** —
  the mean of its two ends across that axis — and vertex `i` stands where its two edges'
  levels cross. So each wall lands halfway between its traced ends; the area is not
  rescaled, and a fit whose area drifts over `ELL_AREA_DRIFT` 15 % or that does not come
  out with exactly one reflex corner is left alone. `corner_skew` measures a reflex corner
  against 270°. Tula: 27 of ~600 lone six-vertex small houses (561 are already square
  within 2°).
  Skipped when any vertex is **shared** with another outline or line (terraced houses, a
  fence along the wall, an arch — squared, they would open a gap) or when a vertex would
  move over `SQUARE_SHIFT_MAX` 3 m (2.5 m first; on Tula that left exactly one lone
  crooked house, way 968378327 at 27° and 2.84 m). **Which outlines count as sharing is
  an explicit list, not "every outline on the map"** (`vertex_uses`): the buildings, all
  the line layers, and **three of the eight area layers** — `parking`, `pitches`,
  `water`. Those three are drawn as a surface of their own with markings on it, and a lot
  carries parked cars as well, so a house that steps off that edge reads on the frame,
  which is the whole subject of issue #27; `parks`, `grass`, `woods` and `sand` lie
  *under* the house and show no seam, and a `landuse` block's outline is not counted
  because private houses are routinely traced onto the block's boundary. Tula, buildings
  with a vertex on an area layer (counted on the cache, so an upper bound — most of them
  fail the area, skew or shift gates anyway): `landuse` 35, `parks` 29, `grass` 15,
  `pitches` 13, `parking` 10, `woods` 5, `water`/`sand` 0. So the three counted layers put
  at most 23 buildings of 7975 out of reach and the other 43 stay eligible.
  `Obstacles` (the pull below) carries its own, narrower list and does not close this
  hole — it is about what a moving house bumps into, not about a vertex two outlines share.
  Runs after `attach_entrances` (they match by the same centimetre
  `vertex_key`, and an attached door is carried to the straightened outline by that
  same key — an entrance holds the *node's* coordinate, not the vertex's, so an exact
  `==` would silently leave the door behind while the house moved) and before door
  generation and tree planting. The price: the render seed is the first vertex, so a squared house rolls
  its material and inferred storeys anew. Tula: 207 (193 under the 20° / 250 m² / 2.5 m
  thresholds; python estimate from the cache, the exact count is the `osm parse:` line).
- **Houses pulled off the sidewalks** (`parse.rs::pull_houses_off_sidewalks`) — the street's
  width is a class constant and the sidewalk is added by the renderer
  (`roads::sidewalk_width`), so an old house standing at the kerb in OSM came out with its
  wall on the drawn sidewalk and, in 2.5D, its roof on the asphalt (Tula way 179102449 on
  улица Бундурина: the wall 4.7 m from the axis against a 5.76 m band). The game does not
  need metre accuracy, and a house on the pavement reads as a bug, so the house moves.
  - **The band** is every non-bridge `roads::is_carriageway` link at `width / 2 +
    sidewalk_width + SIDEWALK_CLEARANCE` 2 m, in a `SIDEWALK_CELL` 32 m grid; raw OSM
    points, not the smoothed centreline — the difference is centimetres. The clearance was
    0.3 m first, and the author's look said the houses stood right on the pavement edge: the
    2.5D roof leans toward the street by another half metre to a metre.
  - **The row moves, not the house** (`Front`, union-find): each eligible building's deepest
    band intrusion names its street way and the side of it; fronts with `need >
    -ROW_SETBACK_TOLERANCE` 2 m on the same (way, side) whose bounding boxes are within
    `ROW_GAP` 20 m join a row. The row's shift is its deepest `need`, and every member whose
    own `need` is within `ROW_SETBACK_TOLERANCE` of it takes that same shift along its own
    away-direction — a neighbour standing on the line but not quite on the band moves too,
    one standing deeper in the block stays. The author's ask: a lone house pulled back broke
    the facade line into a step. Rows do not cross a way split (a junction usually splits the
    way anyway).
  - **The shift** translates the whole outline, holes and attached entrances by the row
    shift, then by the deepest remaining intrusion (closest points of wall edge and axis
    link, pushed out along their difference) for up to `SIDEWALK_SHIFT_ROUNDS` 4 so a
    corner house settles against the other street too.
  - **The cap is a cap, not a refusal**: `SIDEWALK_SHIFT_MAX` 6 m. A front needing more stays
    **out of the row union** (a row of itself) and moves by 6 m; the correction rounds clamp the
    total to the same 6 m, so a house that cannot fully clear the band stops part of the way.
    It was 4 m and a refusal first, and the row rule made that loud: the row's shift is its
    deepest member's, so one house over the limit left its whole row standing — Tula's улица
    Громова east side (13–35) stayed on the pavement because house 17 needed 4.01 m.
  - **Collisions** (`Obstacles`, built once per pass): other buildings (their current, possibly
    already moved, outlines; a grid of bboxes grown by the cap) and segments of every road and
    rail (reach = half width), wall, fence, pipe, open watercourse, water-area ring and
    industrial cylinder (a zero-length segment of its radius), each plus `SHIFT_CLEARANCE`
    0.5 m. The rule is **relative**: a shift is blocked only by an object the house comes
    **closer** to and within reach of — a footway along the wall or a fence on the plot line
    in OSM does not forbid moving away from it. A blocked shift is retried at
    `SHIFT_FRACTIONS` ¾ / ½ / ¼; blocked at ¼ too, the house stays. Not a sweep test: a house
    thinner than its shift could jump a line, which a ≤ 6 m move of a house does not.
  - **Left in place**: a street axis crossing the outline or ending inside it (`None` from
    `Front::of` / `sidewalk_push` — there is no "away"), every fraction blocked, any **shared**
    vertex (the `square_skewed_houses` rule, same `vertex_uses`: terraces, arches, fences on
    walls), `AreaKind::Kremlin`, `BuildingUse::Church` (parts stand on each other).
  - The log line reports moved (rows included), how many of the intruding ones stopped only
    part of the way (cap or obstacle), and how many were left.
  - Order: after squaring, before door generation and tree planting — the navmesh, doors
    and trees see the moved outline. Price: the first vertex moves, so the seed, material
    and inferred storeys roll anew. Tula: **1121 moved (rows included), 56 of them only part
    of the way, 38 left**, 33 ms at load (`examples/bench/map_meshing`'s parse; it was 993
    moved / 160 left / 18 ms at the 4 m refusal with no collision test).
- **Blocks pulled to the roads** (`parse.rs::pull_landuse_to_roads`) — the same mismatch
  read from the other side. The road's width is a class constant and its sidewalk is the
  renderer's, while a `landuse` block is traced along the red line or the plot fences, so
  between the yard and the drawn sidewalk a strip of bare ground is left showing — reported
  from a screenshot of Tula's Воздухофлотская улица (block 185117817: the edge 6.1 m from
  the axis against a 5.76 m band, i.e. a 34 cm seam). On a photo a yard runs up to the
  kerb, and a seam of ground beside the pavement reads as an unpainted layer.
  - **The vertex moves, not the block**, and it moves **under** the asphalt: a vertex whose
    gap to the drawn edge (half the class width plus `roads::sidewalk_width` on a
    carriageway) is within `LANDUSE_GAP_MAX` 5 m is pulled to the road's axis until it
    stands `LANDUSE_OVERLAP` 0.5 m inside the band. The overlap is not decoration: the
    ribbon is drawn from the *smoothed* centreline while the gap is measured on the raw OSM
    points, and without it a bend keeps a centimetre of seam. Beyond the limit nothing is
    done — that is a real gap (a front garden, a verge, a right of way), not a seam.
    Measured on the Tula cache: of 2756 block vertices, 1135 already lie under the asphalt
    and 752 are within five metres (261 / 174 / 130 / 88 / 99 by the metre), with 200-odd
    more beyond that. The limit was 3 m first and went up on the author's look at the
    frame: at four and five metres the strip of ground along a street still reads as a
    seam rather than as a verge.
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
    Everything drawn on a block lies above it (`Z_LANDUSE` 0.25 against `Z_SIDEWALK` 1.2,
    parks and grass at 0.5–0.7), so the part that ends up under the road, under a park or
    on a neighbouring block is never seen; only the closed seam is.
  - **A long edge beside a road is split first** (`LANDUSE_STEP` 8 m, and only where the
    grid has a road near the edge): between its own two vertices an edge is straight while
    the road bends, and on the outside of a turn the seam would stay in the middle of the
    edge, where there is nothing to move. An inserted point that found nowhere to go is
    dropped again, so a ring does not collect vertices for nothing.
  - Bridges and passages give no segments: a block is drawn under a bridge anyway, and an
    arch through a house is not the edge of a yard. Everything else that is drawn does,
    alleys included — a footpath with a seam of ground beside it reads the same way.
  - Order: after the houses are pulled off the sidewalks, before door generation. It could
    stand anywhere in the tail — `landuse` reaches neither the navmesh, nor the doors, nor
    tree planting, nor the parked cars' districts — and it is the only pass here whose
    effect is purely what is drawn.
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
  material**; the **surfaces** — ground, area fills, water, road and
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

    Either is safe, because `surface::spawn_layer` skips an empty builder anyway. That is
    the very thing the converted modules now say in their own doc comments —
    «второй дороги, на которой можно забыть деспавн, нет» (`cars/mod.rs`, `industry.rs`)
    — a property of the shape rather than a thing to remember. Do not confuse it with the
    three comments in `map/mod.rs`: those are about **double registration spawning a layer
    twice**, which the seam neither removes nor touches. It also makes the toggle testable:
    it used to live behind a `return` inside a Bevy system, where no test could reach it.
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
    `roads` (9 layers, `mesh_roads`), all of `spawn.rs` (the 13 surface and paint layers
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
      (`visible_count`), the species resolve off the conifer field, the tint slot and the
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
  - **Converting a module** means: lift the build to `mesh_*` returning
    `Vec<LayerMesh>`, derive `Clone, Copy` on its `*LayerTag` (`spawn_layers` hands the
    tag to every layer), move any cutoff or toggle into the build, drop its
    `materials.add(...)` and its now-redundant `is_empty` guard, and write the tests the
    seam has just made possible. Do not add a `MaterialSpec` variant before a module
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
  **Waterways**), and the **markings** block (Street —
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
  distance to the nearest break — a marking break on a street, a mouth on a channel —
  the code is `Markings::encode`, `lanes·2 + oneway`, 0 for none) and a mesh gets it only from `MeshBuilder::with_surface_coords()`;
  `push_ribbon` / `push_ribbon_broken` fill it from the ribbon frame (quads: ±half width;
  join fans: the outer side; round caps: the projection onto the normal, with *to-break*
  extrapolated past the node along the last quad's slope), polygons get zeros. It costs
  16 bytes per vertex, which is why building, crown and overlay meshes are built without
  it. Where the breaks come from and why the mesher inserts a vertex at every kink of
  *to-break* is under **Markings → Breaks** below.
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
    gone before anything is laid.
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
  of the Упа's southern arm (`waterway=river` 221646296 at `cam 4366 3254`):
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
  - **Render-only.** The navmesh blocks the polygon and the whole channel band as before;
    the caps rule it shares with the drawing (`water_line_caps`) is untouched.
  - Cost: one pass per open channel at load — the `waterways` field of `SurfaceReport`,
    printed inside the one `surface meshing:` line (it had its own `waterways meshing:`
    line until the surface layers went on the seam).
- **Sidewalks** (`map/roads.rs`, `sidewalks` layer at `Z_SIDEWALK` 1.2, `SurfaceKind::
  Sidewalk`, light concrete `SIDEWALK_COLOR` over the asphalt-grey `ROAD_COLOR` — the
  brightness step between them is what reads as the kerb) — a **carriageway**
  (`is_carriageway`: `RoadClass::Street`, width ≥ `STREET_MIN_WIDTH` 8 m, so `service`
  drives get none, and never a `passage`) gets a band `width + 2 · sidewalk_width` (22 % of
  the width, 1.2–3 m per side). It sits under every road ribbon for the casing reason: a
  crossing street's fill covers it and the sidewalk ends at the junction the way a real
  one does. A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `mesh_roads` `continue`s into `bridge_casings`
  + `bridges` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`. It would hang a metre or three past the deck edge over the
  water, and the deck already has its own kerb: `push_bridge_curb`, drawn unconditionally.
  A `footway` mapped alongside draws over it as an alley — beige on grey, and
  tolerated. The road fill went from osm-carto white to asphalt grey together with the
  markings: a white line on white is invisible, and on grey the street grid also stops
  merging with the courtyards. At a junction the band turns the corner on the kerb's own
  arc — **The drawn network → Kerb returns → The sidewalk turns with the kerb** below.
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
  as a pale cut across the road when zoomed in. **A bend that gets no fan is joined by
  shared vertices** — both quads end on the bisector (`miter_offsets`) there. With each
  quad on its own normal the two ends meet only on the axis, and even at 0.4° the
  rasteriser left a hairline across the whole ribbon: a drive in Tula (way 2065, three
  almost collinear points by a parking lot, `cam 4300 2282`) showed two pale lines of the
  sidewalk under it, and the darker asphalt made them loud. The same holds for the
  vertices `GapProfile::split_path` inserts, which are collinear by construction.
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
  **Junction geometry is still not computed as a union**: roads are independent polylines
  drawn overlapping in one opaque layer, and `Round` caps are what makes a junction *look*
  joined — the caps of the ways meeting at a node overlap into a rounded blob, exactly
  how osm-carto gets its smooth junctions (`stroke-linejoin: round` + `stroke-linecap:
  round`). The fill order is **narrow first, wide last** (`mesh_roads` sorts by width), so
  the main road's fill and its gapped line lie over the side street's cap. This is why the
  road layer must stay opaque with a world-position colour: transparency or a per-way tint
  would expose every crossing.
- **The drawn network** (`map/roads/network.rs`, `map/roads/corners.rs`) — what the ribbons
  are laid *from* is not quite `MapData::roads`, and the difference is four render-only
  corrections, all built on **`RoadNodes`** (every node two roads of any class share, same
  5 cm key as the junctions, `junctions::node_key`). None of them moves `RoadLine::points`:
  the navmesh, doors, trees, arches and the parked cars still read OSM as it is. All four
  are counted in the `road meshing:` line, with the time spent before the first ribbon.
  - **Pinned nodes.** `centerline` smooths with `smooth_pinned`, and Chaikin leaves every
    shared node in place. Before, a bend of the through road at a junction was cut by a
    chord up to a road width long, and the side street's end — which sits on the OSM node
    — hung beside the drawn asphalt or stuck out past its far edge. `smooth_path` (rails,
    tram, tree-row band, waterways, cars) pins nothing, as before. **The cars do not pin**, so near a
    bent junction a row walks a chord the ribbon no longer draws; the junction clearance
    (`reach + 5 m`) covers most of it, and making the cars read `RoadNodes` is the way to
    close the rest.
  - **Driveway crossings** (`driveway_crossings`) — an `Alley` way under
    `CROSSING_MAX_LENGTH` 20 m whose **both** ends are ends of (non-bridge) streets is drawn
    as a `Street` at the narrower street's width. Found from a screenshot on проспект Ленина
    (Tula ways 4175 → 4176 → 80, `cam 3265 357`): a service drive, ten metres of `footway`
    across the pavement, the drive again — the sand ribbon lay under the asphalt and cut a
    strip of ground across the entry. A crosswalk is safe from the rule by construction: its
    ends are on pavement footways and it crosses the carriageway with an interior node.
    The substitution is `drawn: Vec<&RoadLine>` in `mesh_roads`, and everything below reads
    `drawn`, not `map.roads`. Tula: 8.
  - **Stitches** (`stitches`) — a loose end (not closed, not a bridge or a passage, no other
    road at its node that **carries** it: a street is carried only by a street, an alley by
    anything) looks ahead for the nearest centreline of a road it may join: the closest
    point on each segment within 60° of its heading, and the heading's ray hit, scored by the
    gap to that road's **drawn edge**, ≤ `STITCH_MAX_GAP` 6 m. The drawn edge is the outer
    edge of the **sidewalk** when the road carries one (`drawn_sidewalk`, so it follows
    `RoadStyle::sidewalks`): OSM maps a drive «to the pavement footway», which sits ~9 m off
    a 12 m street's axis, and measured to the asphalt the drive stayed 7.4 m short — it
    butted into the sand ribbon with the street showing again beyond the sidewalk (Tula way
    1309163271 at Первомайская, `cam 2420 1726`). If the end already lies inside a
    carrying ribbon, nothing is done. The stitched point is pulled back by
    `own half − target half` when the own ribbon is wider, so its round cap does not poke
    past the far edge; the segment is probed every metre against buildings and water (a grid
    of their AABBs, 32 m), and a drive that ends at a garage wall stays ended. The point is
    appended to the drawn path (`Stitches::apply`), so the sidewalk band follows too. The
    markings see nothing of it: the end's dead-end break stands and the extension is
    past it. Tula: 39.
  - **Kerb returns** (`kerb_returns`) — the rounded corner of a junction. At every shared
    node, arms are collected from the **drawn** paths (pinned, so the node is a vertex of
    each): a direction to the first vertex at least 0.5 m away and the straight **run** —
    to that vertex and on through every next one within `STRAIGHT_TOLERANCE` 0.15 m of
    the arm's line. OSM puts vertices on a straight drive wherever it likes (the node
    where the pavement footway crosses it, 2 m off the street), and a run cut at the
    first of them clipped the tangent to nothing: the drive met the street with square
    corners (reported from a screenshot; 8220 → 8710 returns on Tula). Arms of one class are sorted by angle, and between neighbours 25°–155° apart the
    corner of the two facing edges is found, a circle is fitted tangent to both — of
    radius `0.6 × (half + half)` clamped 1.5–9 m between roads of one width, but only
    `MINOR_RADIUS_SHARE` 0.4 × the narrower half width when the half widths differ by
    more than `MINOR_WIDTH_STEP` 0.5 m (1 m for a 5 m drive into a street, 1.6 m for a
    residential street into an avenue). The author's call from a screenshot: at the
    shared-sum radius every drive entered its street as a wide funnel, which is not how
    a minor road meets a main one — and the wedge `[corner, tangent, arc…,
    tangent]` goes into that class's fill builder **before any ribbon** — ribbons and their
    markings then lie over it, and since it is pushed with no ribbon coords it carries no
    wear or markings of its own. Two clamps: the tangent never runs past an arm's straight
    run (past the next vertex the edge has turned), and when **both** roads carry a
    sidewalk, `r ≤ 3.4 × the narrower sidewalk` — the arc's nearest point to the corner is
    `r·(1 − 1/√2)` inside it, and beyond `s·√2/(√2 − 1)` the wedge would show past both
    sidewalks on the ground. With one sidewalk or none, a flare over the ground is exactly
    what a drive's kerb return looks like and is left alone. It is pushed as a **fan from
    the corner** (`push_convex`), which is correct although the wedge is concave: the arc
    between the tangent points is precisely the part of the circle visible from the corner.
    Its straight sides reach `OVERLAP` 5 cm under both ribbons: a side lying exactly on a
    ribbon edge without sharing its vertices rasterizes with dropouts, a dotted light
    crack along the drive edge.
    Mixed-class arms get nothing: a grey wedge over a sand
    footway would read as asphalt spilled onto the path. Bridges and passages give no arms
    (their paths go in as `None`), and under `RoadJoin::Square` no returns are built at
    all — that join is kept for comparison with the old picture. Tula: 8710.
    - **The sidewalk turns with the kerb** (`KerbReturns::sidewalks`), and it is an
      *addition*, not the subtraction this doc used to call impossible: the corner between
      two sidewalk bands is a **concave** notch exactly like the asphalt one, so the same
      `fillet` fills it — laid on the band edges (`half + sidewalk` instead of `half`) with
      a radius smaller by the sidewalk width. That single subtraction is what makes the two
      arcs **concentric**: a fillet's centre sits at `corner + bisector · r/sin(α/2)`, and
      pushing the corner out by `s/sin(α/2)` while taking `s` off the radius leaves it
      where it was. So the band keeps a constant width all the way round the corner, which
      is what a photo shows. Reported from a screenshot of улица Кооперативная × 2-й проезд
      Мясново (`cam 931 3189`): the asphalt rolled out into the corner on its arc, shaving
      the light band to a sliver, and past it the band's square step stuck out onto the
      grass.
      - **The pairing is its own**, over the arms that carry a sidewalk rather than over
        the class group: a drive without one must not break the band of the street it comes
        out of (the street runs straight past it), while two streets with a drive between
        them still get their corner — the drive's asphalt is drawn over it.
      - **A radius under the sidewalk width leaves the corner square**, and that is the
        geometry, not a fallback: a minor entry (a residential street into an avenue,
        radius 1.6 m against a 3 m sidewalk) has no arc for the outer edge to follow on the
        ground either. Same for a run too short for the tangent.
      - **The asphalt wedge still lands on pavement.** `SIDEWALK_COVER` keeps the kerb
        radius under 3.4 sidewalk widths, and under that bound the sidewalk fillet's disk
        is nested in the kerb's, so the asphalt wedge lies inside the sidewalk bands and
        their fillet whatever the angle. On Tula's streets the bound never binds (1.76 m
        sidewalk → 5.98 m ceiling against a 4.8 m radius); it is load-bearing for the
        nesting, not for the radius.
      - Arms with **different** sidewalk widths cannot share one concentric arc; the
        radius then takes the wider of the two (the conservative side — a smaller radius
        keeps the asphalt wedge inside), and the two differ by at most ~0.2 m, since a
        wider gap between the half widths sends the pair to the minor branch anyway.
      - Tula: **903** of them against the asphalt's 8710 (the pairs need two sidewalks and
        a radius over the sidewalk width), ~8 k of the road layers' 387 k vertices, in the
        `road meshing:` line. Load-time only, like the rest of this module.
- **RoadStyle** (resource, BRP-writable, persisted; section `ui/roads.rs` below Buildings)
  — how road ribbons are drawn; any change reruns `rebuild_roads` (despawn
  `RoadLayerTag` layers, respawn from the unchanged `MapData`). Five independent knobs —
  **sidewalks** and **markings** (both on by default) are described above, the three
  older ones:
  - **join** — `Square` (the historical `push_polyline`: an independent quad per segment
    with *both ends* extended by half a width; no joins at all, which is what produced
    the notches on bends and the wedges at junctions), `Miter`, `Round` (default).
  - **smoothing** — Chaikin corner-cutting on the centerline, `Off` / `Light` (default,
    1 iteration) / `Strong` (2). Only bends over `MIN_SMOOTH_ANGLE` (10°) are cut and the
    cut length is clamped to the road width, so the drawn line never leaves the OSM data by
    more than a road width. `passage` roads are never smoothed — their endpoints are pinned
    to building outline vertices that `arch_openings` looks the arch up by — and a node
    shared with another road is never cut (see **The drawn network** above).
  - **casing** — a darker outline, its own merged layer at `Z_ALLEY_CASING` (1.4) /
    `Z_ROAD_CASING` (1.9), width `+2·casing_width` (8% of the road, 0.3–1 m). Both fills
    (1.5 / 2.0) sit above both casings on purpose: otherwise a casing would cut every
    crossing in half. Off by default.

  Smoothing works on a **copy** — `RoadLine::points` and `width` are load-bearing for the
  navmesh (`bridge`/`passage` carves), arches, tree planting and the entrance generator,
  and none of them may shift because the drawing changed. `smooth_path` is shared with
  the rail layers; `centerline` is the road wrapper that adds the `passage` pin.
- **Bridge layers** (`map/roads.rs`, same `RoadLayerTag`) — a road with `bridge` leaves
  its class layers for the **three** `bridge_shadows` (`Z_BRIDGE_SHADOW` 2.05) +
  `bridge_casings` (`Z_BRIDGE_CASING` 2.1) + `bridges`
  (`Z_BRIDGE` 2.2). The **shadow** is the deck's own band, offset along `shadow_dir()` by
  the deck height through the usual `shadow_length_scale()`, on a blended material of its
  own (the flat white one would eat the vertex alpha). Nothing else produced it, because
  the ground shadow layer only knows buildings, and a bridge over the river is the most
  visible thing on the water. It sits **under** the deck and **over** what the bridge
  crosses — except what the z ladder draws above bridges: the rails (a tram on a bridge
  must stay visible), the tram line, wagons, parked cars, fences.
  Eight decisions in `map/roads.rs` (`Bridges`, `probe_underneath`, `bridge_height`,
  `bridge_shadow_path`, `bridge_penumbra`, `push_bridge_shadows`) make it read instead of
  lie, and every one of them was a bug report first:

  - **A bridge is a connected chain of ways, not one way** (`Bridges`, `BridgeSpan`).
    OSM cuts a bridge into pieces at every tag change: Tula's **61 bridge ways are 56
    bridges**, and 8 of those ways are glued into 3 — the Оружейный мост interchange
    424 + 95 + 299 m (818 m, a fork: three ways meeting at one node), a footway
    34 + 129 + 22 m (185 m), and 39 + 4 m. Per-way the shadow lied twice over: the ramp
    below fired at every *internal* node, where the deck is at full height, so the shadow
    dipped under the deck at the interchange's fork and at both joints of the footway;
    and `SHORT_SPAN` was applied to the piece, so a 22 m middle section of a 185 m bridge
    was interrogated as a footbridge and could lose its shadow outright.
    - **Glued end to end, not by `ways_joined`.** That predicate answers «do these two
      polylines touch anywhere» — it is what decides whether a road joins a bridge for
      the curb — and here it is wrong in both directions: on Tula it would fuse **two
      pairs of footbridges that merely cross**, and a way-end landing in the *middle* of
      another bridge (Tula has none, but nothing in the data forbids it) would turn a
      branch into a continuation. What is matched is endpoint against endpoint, at the
      same `JOIN_EPSILON` (0.5 m) — the tolerance is the price of the projection, and
      that part is shared.
    - **The geometry is not glued — the arithmetic is.** Two reasons, and either alone
      is enough: pieces of one bridge may differ in width (one polyline cannot carry
      two decks), and **three way-ends meet at one node** on Tula's own fork, the
      Оружейный мост interchange above, where the three pieces are not a chain at all. So a
      way keeps its own points and its own width, and takes from its bridge two numbers:
      the **span** (the sum over the whole connected component) and, per end, the
      distance to the nearest **free end** through the neighbouring ways. A fork costs
      that model nothing: the distance runs along the shortest of the three branches,
      the node is not a free end, and the deck there stays up.
    - Distances come from relaxing over the way-graph until it converges (tens of edges
      — a priority queue would be ceremony). A component with **no** free end at all — a
      ring flyover — comes out at infinity, i.e. fully raised everywhere, which is right:
      it never sits down on the ground.
  - **A span under `SHORT_SPAN` (35 m) has to prove there is a gap under it**
    (`probe_underneath` over `Underneath` every 2 m — water outlines, watercourse
    channels, rails; **never roads**, because a road is exactly what an approach
    embankment runs along). The question is put to the **whole chain** and answered once
    for it, so a piece over dry land next to a piece over the river keeps the river's
    answer. Proportional height was not enough on its own: the western
    approach to the Упа bridge on Советская улица is four ways of 23–30 m carrying `bridge=yes` and
    `layer=1`, and on the ground it is solid fill, which no tag distinguishes from a
    span. A long way is never asked — there is no 100 m embankment — which also keeps the
    probe off the bridges that would cost the most to test. `Underneath` precomputes an
    AABB per outline for that: the probe is per 2 m of deck and a city carries up to a
    thousand water outlines.
  - **The height follows the span** (`bridge_height` = `SPAN_TO_HEIGHT` 1/8 of the length,
    capped at `BRIDGE_HEIGHT` 6 m, so a span over 48 m is at the ceiling). It was a flat
    6 m, and OSM hands `bridge=yes` to far more than spans: embankment steps, the pavement
    beside a flyover, a 4 m plank over a storm drain. A 6 m shadow under a 20 m path that
    stands on level ground is the loudest lie the map can tell, because **a shadow reads
    as height** and nothing else on the map states it.
  - **The centerline is densified to `SHADOW_STEP` (2 m) first** (`densify`). The ramp
    below lives in the vertices, and **42 of Tula's 61 bridge ways carry exactly two
    points** — the 137 m Упа bridge on Советская among them — so every vertex was an end, the rise was zero
    everywhere, and the shadow landed exactly under the deck, i.e. nowhere. The ways that
    did have vertices (a 571 m flyover with 42 of them, one per ~14 m) got a shadow that
    stepped from vertex to vertex in visible teeth.
  - **The offset ramps from zero at each free end of the chain** (smoothstep over
    `RAMP_SHARE` 0.25 of the **span** capped at `RAMP_MAX` 25 m) — at the abutment the
    deck is on the ground and casts nothing, and the constant-offset version poked a dark
    band past the deck onto the street that joins it, which is exactly where a bridge
    meets a road and where the eye is. The distance a point measures is its own arclength
    **plus what lies beyond its way's joint** (`BridgeSpan::from_start` / `from_end`), so
    an internal joint never ramps and the rise runs continuously across it — both ways
    read the same number at the node they share.
  - **The rise is clamped by the span left ahead of the shadow** (`room / |offset·t|`,
    `room` measured through the chain to the end the offset points at). The ramp is not
    enough on its own: a
    smoothstep climbs faster than the arc advances, so on a short bridge the shadow
    overtook its own end anyway and lay past the abutment as a wedge — the artifact that
    survived two rounds of fixing the ramp.
  - **The band is `SHADOW_SPREAD` (1 m) wider than the deck on each side**, tapering with
    the same rise. A plate's shadow is its silhouette *translated*, so the offset splits
    into a crosswise part (the visible strip beside the deck) and a lengthwise one (the
    band slides along itself and stays under the deck) — a bridge running along the sun
    azimuth has no crosswise part at all and showed nothing, which is what the footbridges
    over the ponds did. The fringe is the water the deck cuts off from the sky plus the
    railing shadow and the slab's thickness, and unlike the offset it barely depends on
    the height.

  - **The edge is soft, and the width comes from the span** (`bridge_penumbra`). Every
    other shadow on the map fades at its edge — buildings `PENUMBRA_WIDTH` 1 m, cars and
    fences `SHADOW_BLUR` 0.35 — and the deck's was a hard cut. The law is the one the car
    already states: «a house's is a metre, three times ours, **because its shadow is three
    to ten times longer**» — so the width is a share (`PENUMBRA_SHARE` 0.3) of the
    shadow's *own* length, i.e. of `bridge_height(span) × shadow_length_scale()`, not a
    constant. A bridge is the tallest thing casting a shadow here and the most varied (a
    2 m plank over a pond against a 6 m flyover), which is exactly why a constant is
    wrong. The ends of the clamp are the two numbers already on the map:
    `PENUMBRA_MIN` 0.35 (nothing here has a softer edge than a parked car) and
    `PENUMBRA_MAX` 1.0 (nothing has a softer one than a house). So a 16 m footbridge gets
    0.36 m, a 40 m bridge 0.9, anything over 44 m the ceiling. The ends do **not** ride
    the sun — they are constants of neighbouring layers — while the share does, through
    `shadow_length_scale()`: a low sun lengthens the shadow and softens its edge.
    - **It tapers by `rise`, not by direction.** Buildings, cars and fences taper theirs
      by `direction · shadow_dir()` because those objects stand *on the ground*: their
      shadow is hard where it meets the object and soft at the far end. A deck is a plate
      *in the air*, so every point of its outline casts from the same height and the
      penumbra is uniform all the way round — except at the abutments, where the deck
      sits down and `rise` is zero. A directional taper would put a metre of soft shadow
      past one deck end onto the street, which is the contact skirt the buildings removed.

  Because the width varies along the band it is **not** a `push_ribbon`: the rails come
  from `miter_offsets` scaled per point (`shadow_edges`), and the core is the closed
  contour of those rails — the left one forward, the right one back — handed to the union
  below and pushed as `push_polygon` per union shape. The penumbra is two quad strips along
  those same rails
  (`push_quad_gradient`, opaque on the rail, alpha 0 at the outer lip), and a segment
  whose rise is zero at both ends emits none.

  **The cores of all bridges are unioned** (`i_overlay`, NonZero — the buildings' and the
  fences' construction), and that is measured, not precautionary: OSM maps a road bridge's
  pavement as a **parallel way of its own** carrying the same `bridge=yes`, and on Tula
  **28 pairs of different bridges** have shadow cores that overlap. In a translucent layer
  that reads as a strip of double darkness running the whole length of the bridge.
  **The penumbra is laid per bridge, before the union**, and cannot be moved after it: its
  width is `rise`, the local height of the deck over the ground, and the union output is
  contours with no `rise` on them. Dropping the taper instead is the option that costs
  more (see the previous bullet). Two neighbours' bands may therefore overlap each other —
  the price the building shadows already state and accept, since both fade to zero. A penumbra
  buried in a neighbour's core is not laid.

  About the deck itself: a light concrete **curb** (`BRIDGE_CURB_COLOR` 0.80, 12% of the width
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
    Don't reintroduce a straight door-to-road strip. The uniform-grid index stays — it is
    the door generator's own (`osm/entrances/index.rs`), extracted while this layer
    existed and its only surviving trace; it has since become `map/grid.rs::Grid` (above).
- **Pitches** (`map/pitch.rs`) — sports and children's grounds, the thing a courtyard is
  actually *made of* on an aerial photo. One surface layer at `Z_PITCH` 2.003 and one
  markings layer at 2.006, the parking pair's shape exactly: the paint is flat
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
- **Parking** (`map/parking.rs`) — an `amenity=parking` area is drawn as asphalt
  (`Z_PARKING` 2.001, the `parking` surface layer) with the **stalls painted on it**
  (`Z_PARKING_LINES` 2.002). The markings go in a **flat-material** layer of their own,
  not through `SurfaceMaterial`: the procedural asphalt grain belongs under the paint,
  not on it, and a 12 cm line is the one thing on this map that must stay pure white.
  - **Road asphalt, no rim.** `PARKING_COLOR` is `roads::ROAD_COLOR` and the fill is a bare
    `push_polygon` — no **Rim**, unlike parks, woods, grass, sand and pitches (the landuse
    blocks have none either, and water has its shoal instead). It had a darker one,
    then a narrow lighter one; both drew a band across every drive where it runs into the
    lot, reported from screenshots (`cam 4301 2270`): the lot lies over the roads, so its
    outline crosses the drive's asphalt, and anything laid along it is a seam.
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
  - **The stall layout knows nothing of roads**, and must not. While the road ribbons
    were still drawn over the lot, stalls under a crossing road were dropped (Tula's big
    lot by the eastern roundabout has a one-way `highway=service`, way 498649803, mapped
    through it); with the lot on top that road is hidden, and the dropped stalls read as
    an unexplained empty band across the rows — the author's call from a screenshot, and
    the rule came out together with the `RoadLine::parking_aisle` flag it needed. The
    layout's own `AISLE` gaps stand in for the OSM aisles.
  - Tula: **170 lots** (172 in the bbox, less the one that is a building and the one
    `parking=multi-storey`). Parking touches neither the navmesh nor tree planting, like the
    landuse blocks.
- **Asphalt wear** (`surface.wgsl`, `SurfaceParams::wear`, on `SurfaceKind::Street` only)
  — what keeps a road from being one flat tone, in the **ribbon frame** so it follows the
  lane rather than the compass: **wheel ruts** — a polished band `RUT_OFFSET` 0.85 m
  either side of each lane's middle (a car's track is 1.5 m), `RUT_SIGMA` 0.32 m wide,
  +7.5 %. The lane is found from `fract` of `(across + half_width) / lane_width`, so
  **every** lane gets its own pair without knowing how many there are.
  - **The ruts are zero-mean**: the band's share of the lane (`2·σ·√(2π) / lane width`)
    is subtracted from it, so between the ruts the asphalt is a touch darker and the lane
    on average is exactly `ROAD_COLOR`. Added as a plain brightening, a marked street was
    ~3 % lighter than an unmarked drive of the same colour, and the two read as different
    asphalt wherever one ran into the other — reported from a screenshot as a colour
    step at the junction.
  - **Kerb dirt was the second effect and is gone** (7 % darker over the outer 0.7 m).
    A junction gap comes only from two *carriageways* meeting; a service drive or a
    residential lane under 8 m joins a street with no gap, and the wider street, drawn
    on top, laid its dark kerb band straight across every drive mouth — the same report.
    Bringing it back needs a gap the drive can make without cutting the lane dashes.

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

  The ruts fade by `visible(...)` like the rest of the surface texture, by their lane
  pitch.

  **And they fade out in a junction gap**, by `smoothstep(0, WEAR_FADE, to_break)` over
  the same `to_break` the lane dashes use — the second component of `ATTRIBUTE_RIBBON`,
  negative inside a gap. `WEAR_FADE` is 5 m, not the dashes' 1 m: over a metre the ruts
  stopped across the lane at the junction edge like a seam.
  Reported from a screenshot of a four-way crossing: the roads are independent overlapping
  ribbons, so each was drawing its own wear across the other. The kerb dirt (since removed,
  above) was then the louder half — a dark band along a street's edge carried straight
  over the crossing street's asphalt, where there is no kerb — and the ruts the subtler,
  two lanes' polished bands meeting at right angles in the middle of the junction. Neither
  is a thing that happens: traffic fans out over a crossing and polishes nothing. The gate
  costs one `smoothstep` on the wear amplitude `w`, so anything added to the block later
  fades with the ruts. The block is
  gated on `lanes >= 2`, and `lanes` is decoded from the same `ATTRIBUTE_RIBBON.w` the
  markings ride on: `roads::road_markings` fills it only for a carriageway of two lanes or
  more, and only while `RoadStyle.markings` is on. So **wear reaches exactly the roads the
  lane lines reach** — a one-lane street gets none, turning Markings off turns wear off
  with it, and an areal fill of the same `Street` material carries no ribbon and stays
  flat — the **parking lot** among them, which shares the material and would otherwise
  have grown ruts across its stalls. The gate is `>= 2` rather than `>= 1` because
  `Markings::encode` never carries a single lane: `>= 1` read as a wider rule than the
  code could ever deliver.
- **Industry** (`map/industry.rs`) — the industrial belt, added in `QUERY_VERSION` **11**.
  Five layers from two sources ([`Structure`] and [`PipeLine`] above), rebuilt on
  `retuned::<SunOnMap>.or_else(retuned::<BuildingHeightMode>).or_else(retuned::<IndustryStyle>)`
  and on nothing else — the settled sun, never `SunStyle`, like every other rebuild — and
  the system stands on its own rather than in the zoom-bucket chain, because there is no
  zoom bucket here: a cylinder is visible exactly as far as its shadow is.
  **One registration carrying all three conditions, never three registrations**: the layer
  arrived with its `rebuild_industry` listed twice in `Update`, and two copies of one
  system in one schedule can both fire in a frame — the second despawns by a query taken
  before the first one's commands were applied, so the layer is spawned twice. That is the
  same trap the buildings' `or_else` chain is written against.
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
    truncating cast ate the 16-car rake the docs promised.
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
    of it. `Z_WAGON` 2.65 — above the rail steel (a wagon stands *on* the rail),
    below the cars.
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
    below, not an exception to it. Heights are constants of the kind (`FENCE_HEIGHT` 2 m,
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
  Both helpers live in **`map/along.rs`** — the walk is a shared primitive the
  way `map/seed.rs` is, and it moved out of `cars.rs` the moment the wagons became its
  second caller, since a second copy of this walk is exactly what it exists to prevent.
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
  - **Nor does a row stand where a street crosses a bridge** (`BridgeDeck`, pinned by
    `a_street_crossing_a_bridge_clears_the_row_under_the_deck`). A street under a bridge,
    or one butting into its side, shares no node with it, so `marking_breaks` sees no
    junction — and `Z_CAR` lies above `Z_BRIDGE`, so the car was drawn on the deck. A place
    within the deck's half width + `bridge_curb_width` + `JUNCTION_CLEARANCE` of a bridge
    centreline is dropped, the way a junction's is; the bridges are prefiltered per street
    by their AABB (grown by the street's width), so a place tests only the decks near it.
    Past the gap the RNG stream differs, exactly as past a junction.
  - **A one-way carriageway gets one row, on the kerb of the driving side**
    (`MapData::traffic_side`, see **Driving side** above; `TrafficSide::kerb`). With
    right-hand traffic, each half of a divided avenue has the kerb on the right and the
    median on the left; two rows would put a column of cars down the median, and in Tula
    145 of 218 `primary` ways are exactly such halves. London and Tokyo mirror it. The same
    rule is right for an ordinary one-way lane. `across` points left, so the right-hand
    side is `-1`; the direction it is right of is the way's own point order, which parse
    has already normalized (see **RoadLine** above).
  - **A car faces the traffic of its own kerb.** The row on the driving-side kerb points
    along the way, the opposite row against it — before this every car on the map faced
    the way's point order, so half of every two-way street was parked nose to the traffic.
    The side order of a two-way street (`[-1, 1]`) does **not** depend on the driving side,
    because it decides the RNG stream: the traffic side turns a row, it never moves one
    (`the_traffic_side_turns_the_row_without_moving_it`).
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
    `retuned::<CarZoomBucket>.or_else(retuned::<CarStyle>).or_else(retuned::<RoadStyle>)
    .or_else(retuned::<SunOnMap>)`,
    one registration, since two in one schedule could both fire in a frame and spawn the
    layer twice; `RoadStyle` is in there because the row is walked along the **smoothed**
    centreline the ribbon is drawn from (`smooth_path(road.points, road.width,
    style.smoothing)`, never the raw OSM points), so Smoothing moves the cars with the
    asphalt. The invisible case
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
    once each — `breaks` 1 ms (`marking_breaks`), `districts` 3 ms (the index) and
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
    (792 k verts, 101 ms — the same run, see **What it costs** under Roof clutter) and above
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
  is `ZoomBucket<TramLods>` (see **Zoom buckets** below), so `rebuild_tram` fires only
  on an actual threshold crossing, never per frame. The tram centerline is smoothed with a
  fixed `TRAM_SMOOTH_WIDTH` (1.2 m) clamp rather than the bucket's line width, so the
  path itself is identical across buckets and LOD switches don't wiggle the track.
  `RailLine::width` from parse is ignored for trams.
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
- **BuildingHeightMode** (resource, BRP-writable, persisted) — how a building's OSM
  height is drawn; any change reruns `rebuild_buildings` (despawn `BuildingLayerTag`
  layers, respawn from the unchanged `MapData::buildings`). The section lives in
  `ui/buildings.rs`, in the Map tab below Trees and Tree rows, one cycling row. A building
  with no height takes its **Inferred storeys** in every mode. Modes:
  - **Facade** (the historical look) — pseudo-3D: the footprint polygon shifted
    straight down in a darker color at z just below the roof (`Z_FACADE` 4.9), visible
    only along south edges. Shift = height × `FACADE_SCALE` (0.2) clamped to 1.5–12 m, so
    a five-storey block keeps the historical 3 m band. Facades sit *under* every roof on
    purpose — that is what stops a tower's wide band from painting over its low neighbour.
  - **Shadows** — facade band plus a long shadow: one translucent merged mesh at
    `Z_BUILDING_SHADOW` (4.5 — *below* every building layer, so a neighbour's roof or
    wall masks the shadow; what a **taller** neighbour owes a lower roof is no longer
    dropped along with it — that piece is the separate `Z_ROOF_SHADOW` layer, see
    **Shadows on lower roofs** below. Still above the portal and corpses, which
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
      15 k crowns plus the car, wagon, fence and industry layers plus the road layers,
      and there are seventy divisions
      on the scale. (The numbers that stood here, 77–86 ms on Tula with 47–56 of it the
      union, were read off the `building meshing:` line through the slider itself on an
      M1 Max: absolute milliseconds from the app are only comparable with each other, since
      App Nap decides them — `examples/bench/map_meshing` is what re-measures the same build
      offline. The road layers' 230–460 k verts in 5–12 ms come off the `road meshing:`
      line the same way.)
      **The road layers are in that list because of the bridge shadow**, and it is the
      only thing in them the sun moves: its offset is baked into the merged mesh, so
      `rebuild_roads` is gated on `retuned::<RoadStyle>.or_else(retuned::<SunOnMap>)`.
      With `RoadStyle` alone that one shadow kept the sun the city loaded with while every
      other shadow on the map followed the knob.
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
    - **Shadows on lower roofs** (`roof_shadow_builder`, `Z_ROOF_SHADOW` 5.05) — the one
      place this model used to lie outright. The ground layer is **under** every building
      layer, so a nine-storey block did not darken the five-storey roof next to it, and in
      a dense block that is the first thing an eye checks. A second, small layer sits
      **over** the building layers and carries exactly the missing piece:
      - **not on a pitched roof** (`is_pitched` targets are skipped). The layer lays a
        flat patch at the eave lift, and a pitched roof rises to its ridge: the patch slid
        off the slopes as a dark rectangle across them — reported from a screenshot as
        soon as private houses became one storey and every two-storey neighbour was the
        `SHADOW_MIN_DROP` taller. The price: a tall block no longer darkens the private
        houses' roofs next to it (its ground shadow still lies under them);
      - for each building, the union of its **taller** neighbours' sweeps
        **intersected with its own footprint** — one `overlay(Intersect, NonZero)` call,
        so overlapping shadows on one roof merge instead of stacking into double darkness;
      - filled with a plain `push_polygon`, **no `PENUMBRA_WIDTH` band** like the ground
        sweeps get (see **Soft edge** above) — deliberately: part of the intersection
        contour is not the shadow's edge but the cut along the roof outline, and a
        feathered band there would draw a light rim around every roof;
      - a neighbour counts only if it is `SHADOW_MIN_DROP` (3 m) taller. Below that the
        shadow reaches the eaves at most, and the pair test would run for nearly every
        pair in a city whose median height is one storey (Tula: 3 m);
      - casters come from a grid of sweep boxes (`SHADOW_CELL` 48 m, just over the longest
        shadow `SHADOW_LENGTH_RANGE` gives **at the default sun** — a low sun stretches
        sweeps past a cell, and that costs selectivity, not correctness: a sweep box is
        registered in every cell it spans); the pairwise version would be 58 million tests;
      - in 2.5D the result is **lifted by the target's own `Lean`**, so it lands on the
        roof where that roof is drawn, not where its footprint is;
      - and then, in 2.5D, the **drawn bodies of the neighbours the extrusion layer paints
        *after* the target** are subtracted from it — a third boolean pass,
        `overlay(Difference, NonZero)` against `DrawnBodies`. This layer is one flat mesh
        above every building layer, while depth in 2.5D is painter's order *inside one
        mesh*: without the subtraction a shadow computed for a far roof is drawn over the
        body of the nearer building that visually hides that roof — a dark blot on a
        sunlit wall. Three parts of it are load-bearing:
        - **"after" is the extrusion layer's own order** — a later place in
          `order::draw_order`, the very list `extrusion_builder` lays the mesh by, and
          **not** the base key `Lean::depth(centre)` under it. The pairs the two disagree
          on are exactly the ones `draw_order` exists for — an L-shaped house with one
          wing in front of its neighbour and the other behind it — and they are exactly
          the pairs whose drawn bodies overlap, i.e. the ones this pass is asked about.
          `roof_shadow_builder` therefore needs that very list, and is **handed** it: the
          order is built once per layer build by the caller (`mesh_buildings`, and
          `measure_layers` as its own bench row) and passed to `extrusion_builder` and to
          this layer alike — the sweeps travel the same way (`ShadowSweeps`). The
          `Option<&[usize]>` it arrives in *is* the 2.5D flag: no order, no lift, no drawn
          bodies, nothing to subtract. A
          *taller* neighbour is not automatically an earlier one: the caster is usually
          drawn first (at the default azimuth 300°), but for a sun anywhere in
          **(111.8°, 291.8°)** the caster itself is the nearer body and eats its own
          shadow on the roof, which is correct — you are looking at its wall there.
        - **a drawn body is the Minkowski sum of the footprint with the lift segment** —
          the ground contour, the lifted contour, and the sweeps of the silhouette chains
          along `Lean::dir()`. That is exactly the patch `extrusion_builder` fills with
          walls and roof, and it is built from the same primitives as a shadow sweep.
          A courtyard goes into the body whole: it has walls of its own, and
          over-subtracting the sliver of shadow that would show through the gap is
          cheaper than leaving a stain on a drawn wall.
        - **covers are prefiltered by the same `SHADOW_CELL` grid** as the casters, over
          body boxes instead of sweep boxes (`Grid::near`); the pass runs only for a
          target that has both a shadow and a cover.
        In the flat modes there is nothing to subtract — a building is drawn on its own
        contour, and `DrawnBodies` is empty there.
      It rides the same `BuildingShadowTag`, so it rebuilds and despawns with the ground
      shadows, and it is reported separately both in the `building meshing:` line
      (`shadows 2ms sweeps + 41ms on ground + 20ms on roofs`) and as its own `roof shadows`
      row in `examples/bench/map_meshing`, where the two shared steps in front of it —
      `order` and `sweeps` — have rows of their own, the way the car layer's `breaks` and
      `parking` do.
      **What it costs** (Tula, 7723 buildings, `dev` profile, one machine — compare runs
      against runs): **~2 k verts** and **20 ms of a 101 ms** build in
      2.5D+shadows+tint, **14–15 ms of ~78** in the flat shadow modes, where there are no
      bodies to subtract. The five milliseconds between the two are the drawn bodies; the
      fifteen underneath them are the intersections.
      **Two things it used to compute a second time, and no longer does** — both are built
      once per layer build by the caller and handed to every layer that needs them:
      - the **order** (`draw_order`), **9 ms**, shared with `extrusion_builder`. This is
        the larger half of the saving: the roof-shadow row went **31 → 20 ms** in 2.5D;
      - the **sweeps** (`ShadowSweeps`), **2 ms** for all 7723 buildings, shared with
        `shadow_builder`. It came off both rows — ground shadows 44 → 41 ms, roof shadows
        17 → 15 in the flat modes. The claim that stood here, that rebuilding the sweeps
        was two thirds of this layer's cost, was **measured wrong**: they are cheap, and
        what the row is actually made of is the intersections.
      Together, on the default mode: **114 → 101 ms**, with the vertex count identical to
      the digit — this was a move of the computation, not a change of the rules. Load-time
      only — nothing here runs per frame.
      Its length clamp rides `map::sun_stretch()` exactly as the ground sweeps do — two
      halves of one shadow may not be measured differently.
    - **The shadow layer rebuilds on its own schedule.** It carries `BuildingShadowTag`
      rather than `BuildingLayerTag`, and `rebuild_buildings` despawns it only when the
      **height mode or the sun** changed (`mode.is_changed() || sun.is_changed()`, the
      latter `Res<SunOnMap>`): it does not depend on the roof-clutter
      zoom bucket, and the two of them together are the single most expensive thing here —
      **60–80 % of the whole building build** on Tula (in `examples/bench/map_meshing` runs
      on the `dev` profile — the share is lower the more the facades themselves cost; the
      roof half is the smaller one, see its own numbers above;
      the 90 ms of 116 that used to stand here came off the `building meshing:` line in
      the app, where the power state sets the scale, so take the share, not the
      milliseconds). `BuildingPlan { mode,
      bucket, shadows }` is how that decision reaches `mesh_buildings` (and what keeps it
      at seven arguments).
  - **Shadows+tint** — shadows plus a roof color ramp: `t = sqrt(height / 60 m)` mixes
    the roof toward `ROOF_TALL_COLOR` (0.20, a near-black neutral — it must be darker in
    luminance than every palette colour, saturated tile included, or the ramp inverts),
    max 0.3 (a 27 m block: 0.55 → 0.48); no-height buildings and the Kremlin keep their
    material colour.
  - **2.5D (Extrusion)** — watabou-style: roof lifted by `lift = height ×
    EXTRUDE_SCALE (0.35) × (EXTRUDE_SKEW, 1)`, the vertical part clamped to 1–30 m. The
    floor was 2.5 m — seven real metres — and a one-storey house, a shed and a two-storey
    cottage were all drawn one height; at 1 m a 3 m house wall is drawn as it is.
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
    `extrusion_lift` is the one door to that vector **for a building** — the extrusion
    layer, the arch patch in the shadows and anything that wants to put a marker on the
    *drawn* building rather than its real outline all go through it. It is a thin wrapper
    over **`drawn_lift(height, mode)`**, the height-only core, which exists because the
    lean is not the house's alone: the industry cylinders (`map/industry.rs`) have
    neither an outline nor a `BuildingUse` and take the same scale and the same
    `EXTRUDE_RANGE` clamp through it. Known limits: units y-sort against
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
    order of magnitude more to produce. It was the L-shaped house's roof and 4 houses in 10
    by seed, and a private sector of hips was the author's report against it («в частном
    секторе домов с плоской или вальмовой крышей практически нет»): now `roofing` hands a
    hip only to `HIP_SHARE_OF_100` (3) of the houses of at least `HIP_HOUSE_AREA_MIN`
    120 m² and to outlines no **gable form** fits (Tula: 3 % of the pitched cohort, from
    13 % before the cross gable learned T and П).
  - **Gable forms** (`buildings/roofs.rs`, `GableRoof` + `GableForm`) — one structure for
    every roof with wall above the eaves: `gables` are `((a, b), face)` — the rectangle edge
    at the eave that picks frame and visibility, and any convex face on that wall plane —
    `slopes` are convex polygons (pushed through `push_convex`: planar and convex, and the
    affine lean keeps them convex), `dormers`, `ridge: Option` (the chimney's; a lean-to
    has none). A rectangular house's form is `house_roof`, from a **mixed** seed
    (`shape_seed` — the raw bits already pick material, colour, jitter and height, and a
    form read from them would travel with them): `HOUSE_FORMS` gable 8 / gambrel 1 /
    half-hip 1, `SHED_FORMS` (untagged ≤ `SHED_FOOTPRINT_MAX` 40 m²) lean-to 7 / gable 3.
    - **Half-hip** — `HALF_HIP_SPLIT` 0.55 of the gable is wall, the rest a hip triangle of
      the side pitch; the ridge is shortened by `(1 − t) × half width` at each end. Under
      `HALF_HIP_MIN_WIDTH` 4.5 m wide, or too short for the two cuts, it is a plain gable.
    - **Gambrel** — knee at `GAMBREL_KNEE` 0.32 of the half width, lower pitch 2.0, upper
      0.5, rise ≤ 6 m; the gable is a pentagon, the steep slope shaded ×1.5, the upper ×0.6.
      Under `GAMBREL_MIN_WIDTH` 5.5 m wide the seed's gambrel is a plain gable.
      Kept under 1 on the screen: a pitch whose `pitch × lean` passes 1 folds the far slope
      under the ridge.
    - **Lean-to** — rise `width × 0.3` ≤ 2 m, the high side by seed; the gables are two
      triangles and the high long wall.
    - **Cross gable** (`cross_gable`) — the outline is cut into rectangles by every
      combination of chords from its reflex vertices (`rect_splits` over
      `garages::reflex_cuts`, ≤ 4 pieces, ≤ 24 splits — the cap stops the enumeration
      itself, not just its output), tried from the largest main body down;
      each piece must fill its `min_area_rect` to `RECT_FILL_MIN`. The main body takes a
      plain gable, every other piece attaches (`Wing::attach`, tolerance 0.6 m) to an
      already covered one: at its **long side** its ridge runs across and into the
      neighbour's slope to a valley at `rise / neighbour pitch`, at its **end** it
      continues the ridge and drops the gable against the neighbour. A wing wider than its
      neighbour, or overhanging it, refuses — the ridge would stand above the neighbour's.
      Both chords of an L must be tried: the "wrong" one leaves the wing at the body's end
      overhanging it (Tula: 409 L's fell to a hip with a single split). Wing slopes are
      pushed after the body's: inside the valley triangle the wing is the upper surface.
    - **Dormer** — one per house (a row of dormers is an apartment block's, the author's
      note), on a plain gable ≥ 8 × 7 m, `DORMER_SHARE` 3 in 10, on the slope and at the
      place (0.35–0.65 of the length) the seed picks, 2.5D only. A front wall with a glass
      pane, two cheeks, a small gable roof into the slope; faces turned away from the
      camera are not pushed, which is the painter's order inside it.
    - `RoofMix` counts what the extrusion layer actually drew into the `roofs:` part of the
      `building meshing:` line. A separate `roofing` pass was 14 ms per rebuild.
  - **Gable roofs** (`buildings/roofs.rs`) — in every mode, a building that
    `is_pitched` (`BuildingUse::House` of any size, or `Other` with a footprint under
    `SMALL_FOOTPRINT_MAX` 250 m², never with a courtyard, never `AreaKind::Kremlin` —
    its towers and gates keep the flat roof, like they keep their colour) is in the
    **pitched cohort** and gets two slopes instead of a flat roof — or another of the
    **Gable forms** above; a hip only where none fits, or on the rare seeded large house
    (**Hip roofs**). The ridge runs along the long axis of the footprint's minimum-area
    bounding rectangle (`min_area_rect`, edge directions of the ring tried as
    orientations — no hull needed at 4–20 vertices); the roof is drawn over that
    rectangle, not the outline (real roofs overhang), which is why it is only applied when
    the outline fills the rectangle to `RECT_FILL_MIN` 0.85 — an L-shaped house would
    wear a rectangle sticking out of it, so it takes a **cross gable** instead (it stayed
    *flat* until hip roofs existed, and a hip until the cross gable did). **Fill alone is
    not enough**, so the rectangle's corners
    must also lie on the walls — no corner farther than `GABLE_OVERHANG_MAX` 0.6 m from
    the outline (`gable_rect`, reported as `ShapeFacts::gable_overhang`). A skewed quad
    fills its rectangle well and still leaves a corner of the roof over nothing: Tula way
    968419942 (79°–100° corners) fills 0.91 with a corner 2.2 m off the wall, and in 2.5D
    no wall came down from under that corner, so the gable end read as cut off (reported
    from a screenshot). About 550 of Tula's ~4 800 gable candidates exceed 0.6 m (measured
    before the gable forms) and go past the rectangle forms: to a **cross gable** when the
    outline cuts into rectangles, else to a hip, which follows the outline itself. Slope
    tone: base roof colour mixed toward
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
- **Temples and the fortress** (`buildings/temples.rs`, `buildings/fortress.rs`) — the two
  kinds of building drawn by their meaning rather than by the house rules; the author's
  report was a kremlin wall with apartment windows under a dark-orange outline and churches
  with dwelling windows.
  - **The roof is assigned** (`roofs::LandmarkRoof`, `landmark_roof` before `is_pitched` in
    `roofing`): Orthodox / Jewish / Eastern nave → hip, Western → `SteepGable` (pitch 1.3,
    ≤ 12 m; a non-rectangle falls back to hip), mosque → flat; a Jewish / Eastern tower →
    `Tent { rise: 0.9 }` in plan sides (≤ 40 m); an Orthodox or Western tower → flat, and
    moot — it has no box (**A standalone bell tower** below); a minaret → flat under its
    `Crown::Minaret`; a drum part ≤ 14 m wide
    → flat (the cupola hides it). Fortress: tower (`area/perimeter² ≥ 0.03`) → tent 0.9, wall
    → flat. `landmark_rise` is the same decision in metres — what a cupola stands on.
  - **The crown** (`Crown: Dome | Tower | Minaret`) is laid on `min_area_rect` with the long
    axis turned east. Orthodox: a chapel (< 120 m²) one small onion; a "ship" (L ≥ 1.6 W and
    ≥ 22 m) a bell tower with a cap onion at the west end and the cupolas over the
    eastern core; five cupolas on a core ≥ 14 m by seed (6 in 10). **Colours come from the
    tags first** (`tagged_wall` / `tagged_roof` / `tagged_dome` over `PolyArea::colours`,
    consumed in `material::wall_look` / `roof_look` and `temples::dome_color`; a tagged colour
    gets no ±3 % jitter): on a `Dome` part — a drum, or a nave whose whole `roof:shape` is an
    onion — `roof:colour` is the **cupola's**, not the roof's, since there is no roof there to
    paint; on a tower it is the spire's and the cap's. **Otherwise colours ride the church's
    seed** (`Sacred::complex`, `material::look_seed`): the kremlin cathedral's parts each
    picked their own and came out pink, white and teal side by side; a size rule for gold was
    dropped with it, since a part does not know its host's area — and the gold the tag now
    gives those drums is what the palette had rolled as silver. `ORTHODOX_DOMES` is half
    gold (Tula's gold: kremlin, All Saints, Николо-Зарецкая), green, blue and the black of
    the red-brick Успенский; silver is gone — a grey cupola read as a shed's galvanised
    roof. Western
    ≥ 20 m: a spire tower at the west front. Mosque: hemisphere (R 0.3 of the short side) and
    1 / 2 / 4 minarets by area (500 / 2000 m²). Synagogue ≥ 300 m²: a low dome. Eastern: roof
    only. A crown ignores clutter gating — it is the silhouette, not equipment — and roof
    clutter (vents, penthouses, chimneys) is **refused** on churches and fortresses.
  - **A bell tower is tiers** (`Crown::Tower { tiers, top }`, `push_tower`), and the photos
    are the reason: every Tula bell tower — the kremlin's, All Saints', Флора и Лавра's — is
    a stack of receding storeys with a white cornice between them and open arches on the top
    one, and a plain box with a tent read as a water tower. `tier_count` gives a tier per
    `TIER_ASPECT` 1.6 narrow sides of pillar height, 1–3 (`TIERS_MAX`): the kremlin tower
    (~50 m of pillar) three, a ship's 6 m tower two, a chapel's turret one. Each
    tier is `TIER_SHRINK` 0.8 of the one below **and never wider than `SHAFT_SIDE_MAX`
    12 m** (`next_tier`; `tier_count` measures by that shaft too): a standalone tower's
    outline is its ground storey *with* the porches and side chambers — the kremlin tower
    is a 28 × 24 m cross, All Saints' 24 × 24 — and the shaft above it is ten to thirteen
    metres, while an unclamped second tier was the same block again. When the base is
    wider than the shaft its tier is capped at `BASE_TIER_MAX` 14 m and the rest goes to the
    shaft (a 24 m white cube otherwise). Heights follow `TIER_SHARES`
    1 / 0.8 / 0.65; between tiers a cornice `CORNICE_REACH` 0.35 m out and `CORNICE_HEIGHT`
    0.5 m high, `CORNICE_LIGHTEN` 0.3 toward white (`push_cornice` — visible sides plus the
    top slab, so the next tier stands on a ledge). Walls take `wall_colors` like a house's;
    **openings are metres, not shares of the wall** (`push_tier_walls`): a belfry arch
    `BELFRY_ARCH` 2 × 5 m, one per `BELFRY_PITCH` 4.5 m of face up to `BELFRY_ARCHES_MAX` 3,
    a window `TIER_WINDOW` 1.1 × 2.4 m, two from `TWO_WINDOWS_FROM` 9 m of face, none
    wider than `OPENING_SHARE_MAX` 0.28 of its wall — as shares a 12 m wall wore black
    gates. **`TowerTop`** ends it: `Tent` — `push_cone` from the top tier to
    the apex, the cap onion on the point; `Spire` — a top cornice, a lantern (`LANTERN_RADIUS`
    0.28 / `LANTERN_HEIGHT` 0.45 of the top tier's narrow side, with slits when ≥ 1.5 m) and
    a thin cone from `SPIRE_FOOT` 0.8 of the lantern, with a ball `SPIRE_BALL` half the cap
    onion. Orthodox towers are tents 6 in 10 and spires 4 (`SPIRE_SHARE_OF_10`, by the
    church's seed); Western towers are spires always. `top_tier` and `cap_radius(size, top)`
    size the cap off the **top** tier — on the bottom one it overhung the tent's faces.
  - **A standalone bell tower is boxless** (`is_standalone_tower`, `Sanctuary::boxless`): an
    Orthodox or Western `Tower` outline — `building=bell_tower`, `tower:type=bell_tower` —
    is drawn **entirely as its crown** (`standalone_tower`) on the plan's rectangle from the
    ground, at eave `Vec2::ZERO`, and the layers lay neither walls, roof, box shadow nor
    neighbours' roof shadows on it (the four `boxless` sites in `layers.rs`, where `raised`
    alone used to decide). Before this the box went up the whole height with church windows
    over all of it and a tent on top: the All Saints bell tower (relation 7064811, 82 m) was a
    pink nine-storey tower block. The OSM `height` is the height **with** the spire, so the
    pillar takes `TOWER_PILLAR_SHARE` 0.72 of it and the spire the rest; an inferred height
    (`BELL_TOWER_HEIGHTS`, to the belfry cornice) keeps the spire on top of it. Minarets keep
    their box under `Crown::Minaret`, Jewish and Eastern towers their box under a tent.
    **And the church it belongs to grows no ship tower of its own** (`Sanctuary::towered`,
    `Own::tower` into `crowns_with`, `Own::default()` being a church standing alone):
    the kremlin cathedral's plan is just long enough for a "ship", and its seeded tent
    stood ten metres from the real bell tower — two tents over one cathedral. The cupolas
    stay; this is the twin of `domed` / `Own::domes`.
  - **A bell tower sits on the church's own west projection first** (`Plan::west_piece`, tried
    by `Plan::tower_seat` before the search below) — the porch, the narthex, the mapped tower
    base. Then its walls **are** the church's walls carried upward and there is no junction
    with the roof to draw at all, which is the only version of that junction that reads: a
    tower seated anywhere else ends its wall in the middle of the roof, and a tower a few
    tens of centimetres inside a wall leaves a sliver of roof along it — both reported off
    Свято-Никольский (way 234273451). The outline is cut by a chord from every reflex vertex
    along its own wall (`garages::reflex_cuts`, the cross-gable trick), the pieces reaching the
    plan's west end within `TOWER_PIECE_REACH` 1 m are kept, and of those the **smallest**
    that fills its `min_area_rect` to `TOWER_PIECE_FILL` 0.9, is no thinner than
    `TOWER_SIDE_MIN` and no wider than `TOWER_PIECE_SIDE_MAX` 16 m wins. Smallest, because at
    Двенадцати Апостолов one chord cuts off the 7.5 × 5.4 m porch and another the porch
    together with the neck. The rect's **own** axis is used, not the plan's: at Свято-Никольский
    the west chapel's walls stand a couple of degrees off the plan, and a pillar on the plan's
    axis poked out of them by centimetres — that was the sliver. Hence `Crown::Tower` carries
    a `size: Vec2` rather than a side: a church's projection is never square.
  - **Otherwise a tower stands on its church all four corners** (`Plan::west_tower`) — the roof
    clutter's own rule, and it did not hold by itself: `min_area_rect` describes the porches
    and the apse along with the church, so at an end with a porch the rectangle is longer
    than the building, and a tower flush with its west end hung over the ground (Tula way
    496756343, the evangelical church on улица Кабакова — 2.5 m in the air, and the same on
    ten of the city's twenty-seven places of worship). So the square is slid east in
    `TOWER_SEAT_STEP` 0.5 m steps, narrowed at each step to `TOWER_SIDE_MIN` 4 m, and nudged
    **across** the axis at each width (`nudges`, from the middle of the end outward), until
    all four corners are on the footprint (`point_in_area`, holes included). The order of the
    search **is** the layout rule: the tower holds the west front, so the westernmost step
    wins, on it the widest tower, and on that the smallest nudge off the middle. Each of the
    three freedoms answers its own case: narrowing keeps the west face where it is, so it
    does not help against a straight facade and does help against a narrow porch; the nudge
    is there because a mapped porch is rarely on the plan's centreline — Двенадцати Апостолов
    (way 42066388) has a real 7.5 × 5.4 m bell-tower base 0.9 m off it, and with nothing to
    shift by, the tower slid off that base onto the church's neck and stood jammed against
    its wall, which is where the crooked junction with the roof came from. With the nudge
    every Tula church but three seats its tower at `slide 0`, i.e. on the west front itself.
    The test square
    is shrunk by `TOWER_SEAT_SLACK` 0.05 m — on a rectangular church the rect's corner lies
    exactly on the wall, and without the slack every tower would slide off its own end — and
    that slack is also the worst overhang left, under a pixel at any zoom. A seat that never
    fits leaves the church without a tower (its Orthodox "ship" falls back to the plain nave
    layout), which is the honest answer for a plan that has no room for one.
  - **And so do the cupolas** (`Plan::dome_seat`) — the same rule broken by the same
    rectangle from the other end: the Orthodox core is the *eastern* slice of
    `min_area_rect`, and on a cross plan that slice is the apse and the air beside it, so
    four of five cupolas grew out of the walls (Tula way 234273451, Свято-Никольский на
    Ржавце). The cluster at full spread is moved **west from the altar** in the same 0.5 m
    steps, no further than the bell tower's east face (`room` — past it the cluster would
    crown the tower, not the church); failing that the spread shrinks (never under
    `DOME_SPREAD_MIN` 1.1 central radii, where the minor cupolas merge into the main one),
    and last of all one cupola is left. The order is the layout rule: five cupolas outrank
    the place, the place outranks the east end. A disc is probed at `DOME_SEAT_PROBES` 8 rim
    points, at the **belly** radius rather than the drum's, so the whole cupola stays over
    the roof — **plus `roofs::landmark_inset`**, because a cupola stands on the roof's own
    platform and not on the outline: a hip's platform is pulled in from the eaves by the
    slope's overhang, and a drum at the outline's edge stood on the slope and hung off the
    roof (the two east cupolas of way 234273451). `landmark_inset` is `landmark_rise`'s twin —
    that one says at what height the cupola stands, this one how far in — and both are
    computed from the same `hip_plan`, so neither can drift from the roof that is drawn. If
    even one cupola never stands, it is left where the plan put it — a church with no cupola
    at all reads worse than one over the eaves.
  - **A cupola is a stack of 32 slices** (`DOME_SLICES`; sixteen showed as bands across a
    6 m cupola stretched 2.6×), each a `push_fan_gradient` disc lifted by the lean
    and coloured per rim vertex by the 3D normal against the sun (`AMBIENT` 0.52 +
    `DIFFUSE` 0.62 × Lambert, a metal highlight `SPECULAR` 0.55 — at 0.35 gold read as paint).
    Upper slices cover lower ones, leaving the
    near crescent — what a real dome shows. The onion profile is a sine to the belly (0.32)
    then the Hermite fall `1 − 3u² + 2u³`: concave at the tip; a `cos^1.6` fall read as an
    egg. **Heights are stretched** (`ONION_STRETCH` 2.6, `HEMISPHERE_STRETCH` 1.4): the 2.5D
    lift is 0.35 m per metre, and an honest onion came out a ball. Drums carry window slits
    and a **cornice ring** under the cupola (`DRUM_CORNICE_REACH` 1.12 × the drum,
    `DRUM_CORNICE_HEIGHT` 0.35 m, whitened) — without it the onion grew out of a pipe; bell
    towers dark belfry arches (above). In the flat modes slices lie concentric.
  - **Shadows** reach the crown's real top: `Sanctuary::shadow_casters` hands a convex base
    per element and its height, swept by `sweep_convex` into `ShadowSweeps`. Stretch is
    drawing only. **Roof shadows are not cast between parts of one church**
    (`layers::same_church`): the roof-shadow layer lies over the building layer, and the
    bell tower and drums of Tula's kremlin cathedral laid translucent wedges over its own
    cupolas.
  - **Parts of a church are assembled at draw time** (`temples::Sanctuary`, built once per
    layer build from the whole list). A **raised part** — a `Dome` part with
    `Sacred::floor_dm`, or a drum (≤ 14 m) on another church of its `complex` (floor = the
    host's height) — **draws no box**: its crown is one drum from the floor (not stretched —
    the height is real) and a cupola, seated at eave `ZERO`. Before this the kremlin
    cathedral's drums (`min_height=20`, 30–35 m) grew from the ground as columns with
    windows through its walls. A church whose cupolas are mapped as parts grows **none of
    its own** (its minor cupolas used to pair up with the real ones). The raised part is
    skipped as a roof-shadow target and casts no box sweep. **All crowns of the layer are
    pushed after all buildings** (`push_crowns` over `(Crown, eave)` pairs): an annex laid
    after its cathedral hid the cupolas' base — cupolas sticking out from behind walls. The
    price: a crown can overdraw a nearer taller neighbour's wall, rare since a cupola tops
    everything around it.
  - **Tower against wall is the ordinary pairwise order, nothing forced.** Tula's
    `building=wall` sections end at the tower outlines (measured: cutting them by the towers
    removes nothing), so at a joint the section on the camera side covers the tower's foot
    and the one behind goes under it — exactly what `order::upper` decides. Forcing «tower
    over wall» was tried and reverted by the author's report: the tower then covered the near
    section's end.
  - **Merlons** (`clutter::merlons`) along every edge of a fortress wall's outer ring, pitch
    2.6 m, 1.3 × 0.7 × 1.9 m, through `push_items` (clutter zoom bucket).
  - **`temple_gallery`** (`examples/demos/temple_gallery`) — faith × (ship, square, chapel,
    bell tower, drum) plus a kremlin row, built by the real `mesh_buildings`;
    `TEMPLE_GALLERY_SHOT` and `TEMPLE_GALLERY_FOCUS=row,column,m/px` for a close-up.
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
  - **A private house is one storey with an attic in the roof**: `HOUSE_HEIGHTS` is
    3–3.4 m in 8 slots of 10 and 6–6.4 m in 2, for `building=house` and for an untagged box
    up to `roofs::SMALL_FOOTPRINT_MAX` 250 m²; an untagged box up to `SHED_FOOTPRINT_MAX`
    40 m² is a yard shed at 2.4–3 m. It was 5–8 m, i.e. two rows of windows on every house,
    and an untagged 60 m² box took the 2–4-storey low table — the author's report: «большая
    часть "двухэтажных" домов на самом деле одноэтажные, но с чердаком». An untagged box
    also takes the house wall table (`material::wall_kind_of`), not the low-rise one with
    its shopfront — at the same footprint ≤ `SMALL_FOOTPRINT_MAX` and under
    `LOW_RISE_STOREYS` (= `BALCONY_STOREYS_MIN` 4). **One constant for roof, walls and
    height**, owned by `roofs.rs` and imported by `material.rs` and `heights.rs`: the height
    rule had its own `COTTAGE_FOOTPRINT_MAX` 150 m² for a while, and an untagged 150–250 m²
    box came out with a gable roof and house walls over 2–4 storeys — the two-storey-house
    look this rule exists to remove (`an_untagged_box_under_the_pitched_cohort_border_is_a_private_house`).
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
    (Tula: `32% tagged, median 3 m, p90 15 m, max 82 m`) — a height distribution is exactly
    the thing a screenshot cannot show, and that line is how this was tuned. **Re-read it
    after every change to a table here**: the median was 8 m while a private house was
    5–8 m, and one storey plus the untagged-box border moved it to the house's own 3 m —
    the median is now the private sector, and it is p90 that carries the blocks.
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
    and so on. `Church` takes its material and palette from its faith
    (`temples::roof_kind` / `roof_palette` — Orthodox seam metal, Western and Eastern tile,
    mosque gravel), the Kremlin from `fortress` (seam on a tower, gravel on the wall's
    walkway), and `Other` — half the city — splits by
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
      runs, `9…15` cladding, `16` the door leaf (`WallKind::code` derives itself from
      `RoofKind::CODES`, the
      **last** roofing code rather than the length of `ALL` — the garage runs are outside
      `ALL` and would otherwise have been overwritten by the claddings — and `DOOR_CODE`
      derives from `WallKind::CODES` the same way, so a new roofing shifts the wall codes,
      a new cladding shifts the door, and the shader's mirror is edited whole).
      Seven claddings —
      `Panel | Brick | Plaster | Shopfront | Shed | GarageDoors | Sacred` — because panel seams with
      balconies are
      exactly **one** kind of building, and while the wall was one, a garage and a church
      wore them too. Five of them are chosen by the tables below; `Sacred` by the class
      alone (every `Church`, before any table); **`GarageDoors` is chosen
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
      bitumen. Kremlin is brick (every wall `WallMark::Solid` — no window, and `push_doors`
      skips it), `Church` is `WallKind::Sacred` coloured by faith — the same two exceptions
      the roof has, in the same order.
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
      slot** beside the code: `meshing::STOREY_STRIDE` (32 — raised from 16 when
      `WallKind::Sacred` pushed the door to code 16) puts the code in the remainder and
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
      dumbbell: a big body on a thin neck) each under every shape — flat, gable, gable with
      a dormer, half-hip, gambrel, lean-to, cross gable, hip — **and** under the game's own
      choice, nine columns (`shapes.rs::COLUMNS`). Under every house the shape that
      actually reached the
      mesh — a refused one says so instead of being quietly swapped, which is what
      `RoofShape` exists for — and the ridge rise in real metres; beside every row the
      three numbers the choice is made from — rectangle fill, the gable's worst corner
      overhang and hip inset — straight from
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
      shape from `roofs.rs` (fill threshold, the hip and dormer shares, inset and its
      clamp, pitch) — parsed out of those files by `constants.rs`
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
    ridge — `GableRoof::ridge`, an `Option` because a lean-to has no ridge and so no
    chimney (the ridge used to be read back out of the first slope, which stopped being
    `[eave, eave, ridge, ridge]` once half-hips and gambrels existed).
  - **Placement** is the shared Park–Miller LCG — `map/seed.rs::Lcg`, one copy for the
    whole of `map/*` (crowns, this clutter, the parked cars), the crown generator's
    original lifted out of `map/trees` — seeded from the roof material's building seed
    (`seed::seed_from_point`, the same door), so the
    equipment survives a mode switch, a zoom-bucket rebuild and a restart in the same
    place. Positions are rolled in the building's own frame (long axis × its
    perpendicular, extent projected from the outline — no second `min_area_rect`),
    inset by `EDGE_MARGIN` 1.6 m or 18 % of the short side, whichever is smaller. Every
    candidate is checked **twice**: **all four corners inside the footprint**
    (`fit`, `point_in_area`, holes included) — the frame is a rectangle and an L-shaped
    block is not — and **the place is free** (`clear`), i.e. no already-placed item
    within `CLUTTER_GAP` 0.5 m. A miss is retried `PLACE_TRIES` (8) times before the
    item is dropped.
    One try was the first version and it was wrong: a 5 × 3.5 m penthouse fits a 12 m
    slab only in a narrow band, so most blocks came out with no penthouse at all.
    **The occupancy test came second, and without it a box sat on a box**: the places are
    rolled independently, so on Tula's kindergarten way 234273437 (9 × 17 m, `Public`, i.e.
    exactly one air-conditioning unit and one shaft) the unit landed on the shaft — the
    author's report from a screenshot. It is a cheap check because **every item of a flat
    roof is laid in one frame** (`axis`/`perp`, the skylight ribbons included), so an
    overlap is two interval tests, not a polygon intersection; the six tries went to eight
    with it, since the tenth shaft on a dense roof now has somewhere to miss. Pinned by
    `equipment_never_sits_on_equipment`, and the shape of that test is the lesson: it
    **walks the same house across the map**, because the seed is its first vertex, so one
    position is one roll of the dice and the first version of the test — four houses at
    the origin — passed with the check commented out. It compares by separating axis, so
    it does not lean on the shared frame the check itself uses. The chimney and the
    merlons are laid outside
    `flat_roof_items`, one per ridge and one per wall edge, and have nothing to collide
    with.
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
  - **What it costs** (Tula, 7723 buildings, 2.5D+shadows+tint, from
    `examples/bench/map_meshing` on the `dev` profile): 792 147 verts / 101 ms with clutter
    against 468 867 / 89 ms without — one hitch on the threshold crossing, in the same
    class as the rail layer's deepest bucket (673 k / 23 ms). Most of it is the shafts:
    every flat roof gets at least one, and a shaft is 6 quads. The 78 / 65 ms that stood
    here came off a run that predates the roof-shadow layer in its current shape (and the
    sharing of the sweeps and the draw order, which took 13 ms back off these very
    numbers — see **Shadows on lower roofs**); the vertex counts moved only by the 80
    buildings the parse gained. The 603 018 / 279 186 that
    stood here before that is an older build of the layer (the gap is 182 026 verts in **both** clutter
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
  **The passage has a side wall** (`arches::tunnel_walls` + `push_tunnel_walls`): the
  opening is skewed by the lean while the road goes straight in, so a sight line
  through the top of the hole runs sideways and in a real arch hits the passage's side
  wall — without it that wedge of the hole showed the grass beside the road (reported
  from a screenshot). The wall is the passage offset by half its width, only its runs
  **inside** the outline (there the body covers it wholly, since the sill is under the
  lift, and it shows only through the hole), only the side facing `-Lean::dir()`, flat
  `wall_colors` darkened by `TUNNEL_SHADE` 0.45 (0.35 blended with the facade beside it,
  0.55 was too dark), no cladding code, and pushed **before**
  the house's walls so the piers and lintel lie over it.
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
