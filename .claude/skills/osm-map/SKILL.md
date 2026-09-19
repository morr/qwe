---
name: osm-map
description: Use when working on the OSM pipeline or map rendering in qwe — the Overpass query/mirrors/cache, map/osm/* parsing, the MapData model, building heights, entrance generation, tree planting and crowns, merged-mesh rendering, road/rail/tram/bridge layers, building and tree style resources. Deep detail behind CONTEXT.md's OSM section.
---

# OSM map pipeline — deep detail

This is the detail layer behind the **OSM map pipeline** summary in `CONTEXT.md`:
how the downloaded data becomes `MapData` and pixels.

Five deep dives live next to this file and are read on demand:

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

### The parse seam: reading the elements, then finishing

`parse()` is two halves with a line between them, and the line is what makes a single
pass reachable:

- **`read_elements(response, bounds) -> (MapData, Vec<Vec2>, ReadReport)`** — the element
  loop and nothing else. What comes out is *raw*: houses still standing in water, churches
  without a faith, skewed outlines, no doors, no trees. The `Vec<Vec2>` is the entrances
  that have nowhere to go yet — Overpass hands out nodes before ways, so at that moment the
  buildings do not exist.
- **`finish_parse(&mut MapData, &[Vec2]) -> PassReport`** — the **eight** finishing passes
  in their one correct order, closed by a ninth step, `compose_trees` for the default
  layout (the parser knows nothing about the panels, but it must not hand out a `MapData`
  whose `trees` is empty, or every reader has to remember a separate compose step; the
  player's own layout is reported by `map::trees::recompose_row_trees`, and only when it
  differs). **That order is their interface**. It used to live as notes in three doc
  comments out of eight and was written down whole nowhere; now it is one numbered list of
  nine steps on that function, each step with its "why here", and a pass's own doc comment
  only points at its step number.
- **The reports are values**, not the ten `eprintln!` that used to make up forty-five of
  `parse`'s hundred and twenty-five lines. `parse` prints both of them at the end of the
  load, as one contiguous block; a test compares the counters, which before meant reading
  stderr. A pass that still prints on its own prints *before* that block — the door
  generator's two warnings (`entrances/mod.rs`) used to interleave with the summary and
  now precede it; moving them into a report is work inside `entrances/`.

Two facts the order carries, both pinned by tests that can only exist now that a pass can
be called alone:

- **Attaching the mapped doors comes before squaring the skewed houses.** An entrance holds
  the *node's* coordinate; while it is attached to the house the same centimetre key
  carries it onto the straightened outline. `squaring_before_attaching_loses_the_door` runs
  the two passes in both orders and shows the second one drops the door.
- **`vertex_uses` is computed twice on purpose.** Squaring (step 4) and pulling houses off
  the sidewalks (step 5) both ask "is this vertex shared?", and the outlines **move**
  between them — a count taken before squaring answers about the old map. This was
  reported as duplicated work; it is not.

### Readings and passes, one by one

- **Building height** (`parse/tags.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim (New York — 97%, a
  LiDAR import) or else `building:levels` + `roof:levels` × `settings::STOREY_HEIGHT` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — and it is the majority
  everywhere but New York, so what fills it in matters: see **Inferred storeys** in
  `references/buildings.md`. Coverage is logged per city on load
  (`N buildings (M with height)`).
- **Building use** (`parse/tags.rs::building_use`) — `BuildingUse: House | Apartments |
  Commercial | Retail | Industrial | Garage | GarageBlock | Church(Sacred) | Public | Other`, the class that
  picks two material tables — the **cladding** (`buildings/material.rs::wall_kind_of`) and
  the **roofing material** (`::kind_of`); neither colour comes from the class itself, both
  come from the chosen material's own palette (**Roof material** in
  `references/buildings.md`, bullet
  **The pick**). Three sources in order: `building=*` when the value names a *particular*
  building (`house`, `apartments`, `garages`, `church`,
  `school`, …), else a big-format **`shop=*`** (`is_big_format_shop`, **Retail box**
  below) — which also overrides the one generic value that does map to a class,
  `building=commercial`, so «Магнит» is a box and not an office — else `amenity=*` on the
  same outline (`school`, `hospital`, `police`,
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
  what made the ГСК look like a hangar — see **the garage row** in
  `references/buildings.md`. The
  singular, together with `carport`/`shed`/`barn`/`roof`, stays one box.
  The Kremlin (`AreaKind::Kremlin`) keeps its
  red regardless of class. `roof:shape` is **not** read (283 of 7465 in Tula carry it);
  the roof shape is inferred instead — see **Gable roofs** in `references/buildings.md` — with one
  exception below. The class is
  also one of the two inputs of **Inferred storeys** (the other is the footprint's shape),
  which is what fills in the height OSM does not carry.
- **Retail box** (`parse/tags.rs::is_big_format_shop`, `model::is_big_box`) —
  `BuildingUse::Retail`, the building that **is** a shop, and the one class where the
  **footprint decides as much as the tag**. Split off `Commercial` (which keeps the office,
  the kiosk and the pavilion) because an office block and a hypermarket share nothing from
  the air: the office has dwelling-height storeys and rows of windows, the box one tall
  trading hall, a blind facade under a brand band and a roof of skylights and plant.
  - **The tag.** `building=retail|supermarket|mall|department_store|shop`, and — under
    `building=yes`, which is where half of it lives — a **whitelist** of `shop=*`:
    `mall`, `supermarket`, `department_store`, `wholesale`, `doityourself`, `hardware`,
    `trade`, `garden_centre`, `furniture`, `car`. A whitelist for the reason every other
    one here is: `shop` has a hundred values and nearly all of them are a **point inside
    somebody else's house** — a bakery, a florist, «продукты» on the ground floor of a
    nine-storey block. Tula v14: 69 building outlines carry `shop=*` (67 ways and 2
    relations), 35 pass the list (17 `mall`, 7 `supermarket`, 4 `doityourself`,
    3 `department_store`, 2 `furniture`, `hardware`, `car`). **No `QUERY_VERSION` bump** —
    `out geom` returns every tag of the element, so `shop` has been in every cache since v1.
    **`building=shop` sits in the `Retail` arm, not the `Commercial` one**, because in OSM
    it is a plain synonym of `building=retail` — the building that is a shop — and the
    whitelist a line below already calls `shop=mall` and `shop=department_store` retail. The
    value is a *particular* one, so a small `shop=*` on the same outline no longer overrides
    it. Latent on Tula, which carries **no** `building=shop` at all; it shows on another
    city, where such an outline gets the cassette, the brand band and the box roof instead
    of an office's shopfront.
  - **The size, the storeys and the height** (`is_big_box`: `Retail` with a footprint ≥
    `BIG_BOX_AREA_MIN` 1200 m², **and** `PolyArea::storeys` at or under
    `BIG_BOX_MAX_LEVELS` 3, **and** a height at or under `BIG_BOX_MAX_HEIGHT` 17 m — each
    of the three can refuse on its own).
    OSM never marks crowd-format, so it is read off the pattern, exactly as
    `is_fortress_tower` reads a tower off its compactness. **The threshold is a chosen
    line, not a gap the data hands you**, and that is worth knowing before moving it: on
    the Tula v14 cache the class has 90 outlines — one per outer ring, the way
    `parse_relation` assembles them, not one per way-member of a relation — of which 63 are
    `building=retail` and 47 of those carry no `shop=*` at all; 24 are at or above 1200 m²
    and 66 below, and the distribution around the line is continuous —
    1265 m² («Торговые ряды») above it, 1198 m² of an unnamed `building=retail` below, and
    ТЦ «Триумф», a `shop=mall`, at 1187. What 1200 m² says is that big format starts at a
    hypermarket of 1300–1400 m² («ДА!» 1363, «Верный» 1628), while a mall block of a
    thousand-odd still reads as an infill from the air; the price is that two buildings of
    nearly the same footprint are drawn differently, which is the cost of any single
    number here.
    **The storey count is what actually separates a mall from a box, and that is why
    `PolyArea` carries it** (`storeys`, `building:levels` as the tag says — see **MapData**
    above). The threshold used to live in the parse's height alone, and everything else on
    this list hung on the footprint by itself: «Пятёрочка», a `shop=supermarket` on the
    ground floor of a nine-storey panel block mapped onto the whole block, got honest 27 m
    *and* a blind cassette, a 5.5 m tier, a skylight grid and entrance groups every 55 m.
    Moving a **height** ceiling into the predicate was the first attempt and it could not
    do the job: a four- or five-storey mall is measured in dwelling storeys and lands at
    12–15 m while a real two-level box stands at 12.5 m, so the ceiling separated two of
    the nine tall malls and left seven — no number in metres runs between «Империя» (4
    storeys, 12 m) and ТРЦ «Макси» (2 trading levels, 12.5 m). `BIG_BOX_MAX_LEVELS` 3 does,
    and the same constant is what `building_height` measures the shell by, so «measured as
    a box» and «drawn as a box» stay one statement rather than two that can part.
    Of the 24 outlines above the area line **15 are big boxes**. The nine that go are
    exactly the `building:levels ≥ 4` ones: ТРЦ «Гостиный двор» (6), «Парадиз» (5),
    «Троицкий» (5), «Заречье» (5), «УтюгЪ» (4), «Империя» (4), «Талисман» (4), an unnamed
    `building=retail` of 1408 m² (4) and «Пятёрочка» (9).
    **The height ceiling stays, as the answer for an unmapped storey count**
    (`BIG_BOX_MAX_HEIGHT` 17 m — the same three trading levels of `BIG_BOX_LEVEL_HEIGHT`
    4.5 m plus `BIG_BOX_SHELL_EXTRA` 3.5 m of shell, written in metres): a building with a
    verbatim `height` and no `building:levels` has nothing else to be judged by, and on
    Tula the two questions never disagree — no `Retail` outline there carries a `height`
    tag at all.
  - **What it decides.** Four things, each with its own bullet elsewhere: the **height**
    (below), the **roof material** (`BIG_BOX_ROOFS` — light membrane / bitumen / gravel —
    against `SHOP_ROOFS`), the **roof clutter** (the skylight grid and the roof plant, in
    `references/buildings.md`) and the **wall** (`WallKind::BigBox` with its brand band,
    same place). A small shop gets none of it and keeps a shopfront over a house window.
  - **The height** (`building_height`, and it is why that function takes the outline as
    well as the tags). A trading level is `BIG_BOX_LEVEL_HEIGHT` 4.5 m, not the 3 m
    dwelling storey, and over the top one sit `BIG_BOX_SHELL_EXTRA` 3.5 m of technical
    floor and parapet — so `building:levels=1` on a hypermarket is 8 m, not 3. That single
    number is the loudest half of the original report: a 122 × 103 m «Магнит»
    (`building=commercial` + `shop=supermarket`, `levels=1`) was drawn three metres tall
    and read as a giant one-storey house. Tula: «Магнит» 8 m, ТРЦ «Макси» (`levels=2`)
    12.5, ТЦ «Сарафан» (3) 17.
    **Only up to `BIG_BOX_MAX_LEVELS` 3 trading levels** (and only while the shell they
    produce fits under `BIG_BOX_MAX_HEIGHT` — the same statement in metres, which here
    catches a `roof:levels` the storey threshold does not count) **and only from
    `BIG_BOX_MIN_LEVELS` 1 up**. Both ends are
    load-bearing. Above: «Пятёрочка» is `building=retail` + `levels=9` mapped onto the
    whole panel block it occupies the ground floor of, and a trading level there would
    have made a 45 m tower of it — so the ordinary dwelling storey applies, which also
    happens to be right for the multi-floor malls (Гостиный двор 6 → 18 m, Парадиз 5 → 15).
    Below: `building:levels=0` is abandoned tagging rather than a building, and without
    the lower bound `BIG_BOX_SHELL_EXTRA` would lift that zero to a plausible 3.5 m — a
    hypermarket drawn as a slab one storey tall, which is the very defect this bullet is
    about. Out of the range at either end the tag is read as the ordinary storey count, and
    a zero there stays implausible and falls through to the inferred height.
    **Both thresholds are the constants the predicate reads**, and that is the point: a
    house measured in dwelling storeys because it has a fourth one is, by the same number,
    not a box downstream either (`a_shop_on_a_tall_block_is_not_a_big_box_either`,
    `four_storeys_of_mall_are_not_a_two_level_box`).
  - **The untagged half is inferred to the same numbers** (`heights.rs`: `BIG_BOX_HEIGHTS`
    8–11 m, `SHOP_HEIGHTS` 4.5–6.5), deliberately — «Верный» and ТЦ «Перспектива» carry no
    `building:levels` at all, and a box standing next to an identical box must not come out
    half as tall for want of a tag.
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
  `blue` on Свято-Никольский, `red` / `green` on the arms museum annex. **Two things read
  them**: the temples (`temples::tagged_wall` / `tagged_roof` / `tagged_dome`, below) and
  the retail box's **brand band** (`layers::brand_color` — ТРЦ «Макси» is tagged `orange`,
  which is its real colour). Reading them on every house is a palette decision the private
  sector has not made.
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
    Everything drawn on a block lies above it (`Z_LANDUSE` 0.25 against `Z_SIDEWALK` 1.6,
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

**Since the parse seam, a case need not go through the whole pipeline.** Three routes,
and the choice is what is under test:

- **The fixture through the real `parse`** — a tag rule, which is most cases. The route
  above.
- **`read(scene)`** (the helper in `tests.rs`) — the fixture's JSON through `read_elements`
  alone, so the *raw* map can be asserted on before any pass touches it
  (`reading_the_elements_leaves_the_passes_undone`), or `finish_parse` called on it as one
  value-returning step (`finishing_the_parse_reports_what_each_pass_did`).
- **A pass called by name on a `MapData` built by hand** — no JSON, no `GeoBounds`, none of
  the other passes (`a_pass_runs_on_a_hand_built_map`, `squaring_runs_on_its_own`,
  `pulling_houses_off_the_sidewalks_runs_on_its_own`,
  `pulling_the_blocks_to_the_roads_runs_on_its_own`, `resolving_the_faiths_runs_on_its_own` —
  every one of them on a scene that makes the pass's counter **non-zero**, since a pass
  that did nothing proves nothing about being called). This is
  also the only way to test the *order*: `squaring_before_attaching_loses_the_door` runs the
  same two passes both ways round.
  **The geometry under the passes has its own unit tests**, one step below the pass —
  `fit_rectangle`, `fit_ell` and `closest_between_segments` take rings and segments, not a
  `MapData`, so they are asserted on directly
  (`a_fitted_rectangle_keeps_the_area_the_centroid_and_the_winding`,
  `a_fitted_ell_squares_its_corners_and_refuses_what_is_not_an_ell`,
  `the_closest_pair_of_two_segments_is_none_only_when_they_cross`).

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
- **Sidewalks** (`map/roads.rs`, `sidewalks` layer at `Z_SIDEWALK` 1.6, `SurfaceKind::
  Sidewalk`, light concrete `SIDEWALK_COLOR` over the asphalt-grey `ROAD_COLOR` — the
  brightness step between them is what reads as the kerb) — a **carriageway**
  (`is_carriageway`: `RoadClass::Street`, width ≥ `STREET_MIN_WIDTH` 8 m, so `service`
  drives get none, and never a `passage`) gets a band `width + 2 · sidewalk_width` (22 % of
  the width, 1.2–3 m per side). It sits under the **street** ribbons (1.9 / 2.0) for the
  casing reason: a crossing street's fill covers it and the sidewalk ends at the junction
  the way a real one does. It sits **over the alley** ones (1.4 / 1.5), and that is the
  author's call from a screenshot of a yard footway running out onto улица: the path used
  to draw a sand ribbon straight across the light band and on to the kerb, while on a
  photo it stops at the pavement. The price is the other reading of the same rule — a
  `footway` mapped *alongside* a street (OSM's own way of mapping a pavement) now sinks
  into the band instead of lying on it, which is what the band already draws anyway; and a
  path crossing a street still reaches the asphalt, because a **driveway crossing**
  (`network::driveway_crossings`) is redrawn as a `Street` and a footway that is not one
  is simply covered by the carriageway at 2.0 as before.
  A **bridge is the exception**: `is_carriageway` says yes, so a deck keeps its
  lane markings, but the bridge branch of `mesh_roads` `continue`s into `bridge_casings`
  + `bridges` *before* the sidewalk block — a deck gets no band ever, at any width or
  `RoadStyle::sidewalks`. It would hang a metre or three past the deck edge over the
  water, and the deck already has its own kerb: `push_bridge_curb`, drawn unconditionally.
  The road fill went from osm-carto white to asphalt grey together with the
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
  every road, alley and kremlin wall is drawn. The `roads::push_ribbon` **wrapper** over
  it exists only to map a `RoadJoin` (the user's knob) onto the pair below, so the layers
  whose join is a constant — fences, rails, the tram — call `MeshBuilder::push_ribbon`
  themselves with `RibbonJoin::Round` / `RibbonCap::Round` and `closed: false`; going
  through the wrapper made the road's style read as theirs.
  Two knobs, both named after their SVG /
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
  - **smoothing** — Chaikin corner-cutting on the centerline (`map/smooth.rs::Smoothing`),
    `Off` / `Light` (default,
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
  and none of them may shift because the drawing changed. **The rule itself lives in
  `map/smooth.rs`**, not in `roads.rs`: `Smoothing`, `smooth_path`, the `pinned` variant
  `smooth_pinned` and the private `chaikin` with its two constants. Six modules read it —
  roads, rails, the tram, the waterways, the parked cars and the tree-row band — and it
  used to sit in the middle of `roads.rs`, between the road palette and the bridge
  shadows, which is why the enum was called `RoadSmoothing`. The **variant names**
  `Off`/`Light`/`Strong` and the field name `smoothing` are values in `settings.toml` and
  may not be renamed; the type and the module may. `centerline` stays in `roads.rs` — it
  is the road wrapper that adds the arch and shared-node pins.
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
    is not strictly half-open either (see **One RNG and one point seed** above), and over
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
    `retuned::<CarZoomBucket>.or_else(retuned::<CarStyle>).or_else(retuned::<RoadStyle>)
    .or_else(retuned::<SunOnMap>)`,
    one registration by the rule under **When a layer rebuilds** above; `RoadStyle` is in
    there because the row is walked along the **smoothed** centreline the ribbon is drawn
    from (`smooth_path(road.points, road.width,
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
