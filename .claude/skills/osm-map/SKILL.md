---
name: osm-map
description: Use when working on the OSM pipeline or map rendering in qwe — the Overpass query/mirrors/cache, map/osm/* parsing, the MapData model, building heights, entrance generation, tree planting and crowns, merged-mesh rendering, road/rail/tram/bridge layers, building and tree style resources. Deep detail behind CONTEXT.md's OSM section.
---

# OSM map pipeline — deep detail

This is the detail layer behind the **OSM map pipeline** summary in `CONTEXT.md`:
how the downloaded data becomes `MapData` and pixels.

This file is the **map and the invariants** every layer shares — the cache, the model,
the grid, the shapes, the shadow rules, the layer seam, the noise rule, zoom buckets.
The mechanisms live in the reference files next to it, read on demand — **read the
one your change is about, not all of them**:

- `references/parse-seam.md` — the parse seam (`ParseKnobs`, `read_elements` /
  `finish_parse`, pass contexts) and **Testing the parse**; the entry point of parse work.
- `references/parse-buildings.md` — the building readings and finishing passes (heights,
  building use, retail box, churches, the fortress, drowned buildings, squaring, houses
  pulled off the sidewalks).
- `references/parse-geometry.md` — the geometry passes (path width, rail gaps, the three
  sidewalk/pavement passes, blocks pulled to the roads with the lot paving, ground pockets,
  ring assembly).
- `references/model.md` — the **MapData field catalog**: per type, every field's parse
  provenance, consumers and the invariants it carries.
- `references/roads.md` — the roads hub: the phases of `build_roads` and the index to the
  subject files, each one subject — `roads-sidewalks.md` (sidewalks, verges, kerb
  pockets), `roads-pairs-medians.md` (divided streets and their medians, the tram band),
  `roads-streets-network.md` (streets, sections, tapers, the street axis, the ribbon,
  stitches), `roads-markings.md` (lane markings and their breaks), `roads-junctions.md`
  (the break sets, driveway crossings, turning circles, safety islands),
  `roads-junction-paint.md` (zebras, stop lines, who breaks), `roads-corners.md` (kerb
  returns), `roads-rings.md` (roundabouts), `roads-merges.md` (merges),
  `roads-bridges-wear.md` (bridge layers, asphalt wear, turn paths), `roads-gallery.md`
  (the junction gallery `examples/demos/roads`). `RoadStyle`/`RoadShape` ranges live in
  `CONTEXT.md`, the hub keeps the roads-side residue.
- `references/parking-layout.md` — the parking layout (stalls, aisles, the lot outline
  and its parse).
- `references/parking-big-lot.md` — the big lot: its kerb and the gores at a roundabout
  (the owner of the gore law).
- `references/parking-parked-cars.md` — the parked cars (`map/cars/`).
- `references/layer-seam.md` — the layer seam's per-layer stories: the converted
  modules, the report states, the checklist for converting the next module, and the
  surface material with its shader.
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
- `references/buildings-heights.md` — how a house is drawn: the height modes and their
  shadow sweeps, inferred storeys.
- `references/buildings-landmarks.md` — temples and the fortress, the arches.
- `references/buildings-material.md` — the roof and wall material with its shader.
- `references/buildings-clutter.md` — what stands on a roof.
- `references/buildings-garages.md` — the garage rows.
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

**Position comes from the screenshot's HUD** (the first line of the top-right **point
inspector** panel, `zoom/x/y cx/cy` — the panel under the telemetry, `ui/inspector.rs`),
never from `brp cam` — the live camera has moved since the picture was taken. The user's
screenshots are arbitrary screen regions from a macOS utility; `shot_coords` handles those
(the panel is its ruler) as long as the panel is in the crop. Details and caveats (window
size, zoom precision) — `.claude/live-app-project.md`, "Where a screenshot was taken". In
the **live app** the same question is one call: `brp probe <x> <y>` — its report lists the
drawn layers *and*, in the `features at the point (semantic index)` section, the map
features under the point (`road residential 8.0 m, sidewalks`, `building apartments,
12 m`, `waterway stream 2.5 m`…); the inspector panel shows the same list live under the
cursor. The semantic list answers for the **model**: beside the tagged features it
carries the bands a road draws itself — `sidewalk (auto, tagged)` / `sidewalk (auto,
inferred)` (why the parse put the sidewalk there) and `verge (auto)` — measured off the
OSM axis by the width law, so smoothing, kerb returns and driveway crossings are not
in them; for the drawn geometry `brp probe`'s layer section is the arbiter. Every
feature that came out of the extract carries its OSM id as a tail `#id` on the line —
the tags are one call away: `tools/osm_near --id <id>`. `osm_near` reads the newest
`assets/osm/<city>_*.json` and
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
  on the **country boundary** and everything inside inherits it. `out tags`, never
  `out geom` — a country's border geometry is megabytes.
  `is_in` needs areas, which not every mirror builds; an empty answer leaves
  `MapData::traffic_side` at `TrafficSide::Right` and logs it (`parse.rs::driving_side`,
  the innermost `admin_level` wins).
- **Mirrors** — `OVERPASS_URLS` in `download.rs` is tried in order (`maps.mail.ru` →
  `overpass-api.de` → `kumi.systems` → `private.coffee`). The VK/Mail.ru instance leads:
  full planet, current data, and the nearest pipe from here. On dense cities those answer
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
  `references/roads-gallery.md`.
- **One file per city** — every load first runs `prune_stale_caches()`: anything under a
  known city slug that is not that city's current `cache_path` is deleted. That is what
  retires extracts left by an old geo center, `MAP_SIZE` or `QUERY_VERSION` — tens of MB
  each. It sweeps **all** cities, not just the one being loaded, so junk under a city
  nobody visits still goes; the current file of each city survives, so a tour of the six
  is not six downloads per lap.

## MapData — the parsed model

`map/osm/model.rs`; the resource stays resident after spawn. The **field catalog** —
per type, every field's parse provenance, consumers and invariants — is
`references/model.md`. One line per type here:

- **The nine `Vec<PolyArea>`** are not `AreaKind` written twice — the vector is the
  layer, the kind is the member of that layer (the argument in `references/model.md`).
- **PolyArea** — polygon with holes; `AreaKind: Building | Kremlin | Water | Park | Wood
  | Grass | Sand | Residential | Industrial | Parking(LotKind) | Pitch(PitchKind)`;
  only Wood carries trees.
- **RoadLine** — centerline + width from its section; `highway`, `RoadClass`,
  `pavement`, `bridge`/`passage`, the marking fields (`oneway`, `roundabout`, `lanes`,
  `turns`, `sidewalks`, `parking`).
- **RoadNode / RoadArea** (v15) — the junction vocabulary: crossings, signals,
  stop/give-way, turning circles, islands, `area:highway` outlines.
- **RailLine** — `railway=*` centerline + width by value; `RailKind: Active | Tram |
  Disused`.
- **WallLine** — `barrier=city_wall`, 3 m, kremlin red, impassable.
- **FenceLine** — a plot boundary, `FenceKind: Fence | Wall | Hedge`.
- **Structure** — an industrial cylinder (`man_made=storage_tank|silo|chimney|
  water_tower|gasometer`), centre + radius + height + kind.
- **PipeLine** — an overhead heating main, `man_made=pipeline`.
- **WaterLine** — a *linear* watercourse, width by class; `tunnel` marks the piped part.
- **TreeRow / TreeNode** — avenues and surveyed trees, with the `spacing`/`radius` the
  data itself carries.
- **wood_trees / row_trees_kept / row_trees_slid** — the planting raw material, sorted
  by threshold.
- **trees (`TreeSet`)** — what the renderer reads; `compose_trees`, the prefix rule.

## The shadow rules — `map/shadow.rs`

Seven layers cast a shadow — buildings, fences, cars, wagons, industry, bridges, roof
clutter — and three things are the same for all of them.

**The crowns are the eighth caster and stay outside on purpose** (`trees/crown.rs`):
their length is a drawn length of its own (`CrownParams::shadow_height_base`, the conifer
cone's `3h`), multiplied by `sun_stretch()` right where it is written — `shadow::length`
would say something different about them, so they call the sun directly.

- **`length(height)`** is the one place `shadow_length_scale()` is applied — *"a shadow
  length written without `sun_stretch()` is a bug in the making: it will look right at
  the default and wrong at both ends of the slider."* Clamps stay with their owner
  (`SHADOW_LENGTH_RANGE` for buildings, the roof edge for clutter) and are applied
  **after** it.
- **`offset(height)`** is that length as a vector. It is the displacement of the far end
  of a sweep, **not** where a silhouette is moved to: the shadow starts *under* the object.
- **`penumbra(direction)`** — the soft edge's share at a vertex: zero where the shadow
  meets what casts it, full at the far end, growing along a lateral side.
- **`push_union(builder, contours, blur)`** — union the sweeps (`i_overlay`, NonZero — a
  translucent layer must never double on itself) and lay the tapered band outward from
  the outer ring and inward from each hole.

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

**The light stays a process global** (`map/sun.rs`, four `AtomicU32`), and that is a
decision, not an omission — the why and the stated price are in `CONTEXT.md`.

## The uniform grid — `map/grid.rs::Grid<T>`

Every "what is near this point" answer on the map comes from one type.

- **The step belongs to the grid** (`Grid::new(size)`) — an insert and a query cannot
  disagree about it. It stays an argument rather than a module constant because the
  number is about the domain, not about the grid: 60 m for the doors'
  road index and 30 for their footprints (`ROAD_CELL` / `FOOTPRINT_CELL` in
  `osm/entrances/index.rs`), 48 for the shadows, 120 for the car districts.
- **Two primitives and three conveniences.** `cell(IVec2)` — one cell; `near_each(min,
  max)` — everything in the touched cells **as is**, duplicates included, in a fully
  determined order (cells ascending by x then y, insertion order inside a cell). On top of
  them: `at(point)` (one cell by a point), `near(min, max)` (sorted and deduped, needs
  `T: Ord`), and `pairs()` (every pair that shares a cell, sorted and deduped).
- **`insert(min, max, value)` puts the value in every cell its box touches** — that
  invariant is what makes a one-cell `at` complete rather than approximate. An error
  there returns a silently incomplete answer —
  which is exactly why it lives in one place now.
  **A box that is not of this world is refused**: a non-finite corner is a
  `debug_assert`, and a box over `MAX_INSERT_CELLS` (2²⁰ — 30 × 30 km at the smallest
  30 m cell) is skipped with an `error!`. `as_ivec2` saturates a huge coordinate to
  `i32::MAX`, so one broken axis point had the double loop spread a single link over
  billions of cells — 12 GB and a load that never finished. The skip is the net, not the
  fix; the fix is wherever the geometry broke.
- **`insert_segment(from, to, pad, value)` is that insert for a link of a polyline**, and
  the box is the grid's arithmetic too: `from.min(to) - pad, from.max(to) + pad`. `pad` is
  `0.0` where the reach belongs to the query rather than to the value (`water.rs`,
  `entrances::RoadIndex`, the parse's sidewalk grid, `footprint::StreetEdges`), and it is
  a **scalar**: `Vec2 - f32` is glam's own, so no caller writes `Vec2::splat(reach)` any
  more.
- **`near` sorts because the mesh must not move between runs**, not for the caller's
  convenience. `near_each` is the escape hatch for values that are not `Ord` (`water.rs`
  indexes `(Vec2, Vec2)` edges) and for callers that already tolerate duplicates.
- **The map is `bevy::platform::collections::HashMap`, not `std`'s** — the same
  `hashbrown`, with `foldhash` in place of SipHash. A city-wide index is tens of thousands
  of inserts (the stitches alone index 14 642 buildings and water bodies), so at that
  traffic the hash *is* the price of the index, and an `IVec2` key has no use for a
  DoS-resistant one. **The swap also removes a nondeterminism rather than adding one**:
  std seeds SipHash per process, so a map iterated for its values handed out a different
  order every run, while bevy's `FixedHasher` has a fixed seed. Still on `std`: the tag
  maps of the parse and the Overpass reader, which serde deserializes into.
- **`cell_of` is public for one caller**, `entrances::RoadIndex`, which walks cells in
  **rings** outward from the point and stops as soon as what it found beats anything the
  next ring could hold. That strategy belongs to it, not to the grid.
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
product cost about a second per call against 19 ms with it — and `build` runs twice per
load, once for the grid fill and once for the mesh build. The decision itself is
deliberately NOT unified: the grid blocks curb tiles by a directional outward probe, the
mesh subtracts band polygons, and both survive the "primary's nominal 16 m swallows its
parallel sidewalk" trap in their own way — a shared point-coverage test reproduces
neither (with slack it opens the composite's outer barrier, without slack it crumbles
the curb chain into dashes). This was analysed and rejected during the footprint
extraction; re-attempting it means going through the curb pin tests
(`navmesh/fill/tests.rs`) and the parity tests.

## Rendering

- **One RNG and one point seed for the whole of `map/*`** (`map/seed.rs`) — everything a
  layer scatters must survive a rebuild: a zoom-bucket crossing, a height-mode switch, a
  restart. So the layers share two primitives instead of copying them. **`Lcg`** is the
  Park–Miller (Lehmer) generator of `Village.js` — `seed = 48271·seed mod 2³¹−1`, plus
  `range`, `gauss3` (a bell on (0,1)) and `bell4` (a bell on (−1,1)). **`seed_from_point(Vec2)`**
  is the seed itself: the object's **own reference point** in
  centimetres — the first vertex of a footprint, the first point of a street — through three
  mixing rounds, never the object's index in the extract, which a re-parse is free to move.
  Callers: `buildings::material::building_seed` (the roof material, the roof shape and, since
  the height inference, the storeys), `buildings::clutter`, `trees::crown`, `cars`, `wagons`.
  A copy of either primitive is the defect this module exists against.
  The **parse** stage is deliberately not on it — doors (`osm/entrances/`) and tree planting
  (`osm/planting.rs`) run on `rng::lcg_seeded_by`, a different point-seeded LCG, and rewiring
  them would move every door and every tree in every city.
- **Measuring the layer build** — `cargo run --example map_meshing -- [city slug]`
  (`examples/bench/map_meshing.rs`) prints vertices and milliseconds per layer for every
  height mode × clutter bucket, plus the car layer in the same `LayerCost` rows with its
  junction breaks split out, straight from the Overpass cache with **no window and no
  GPU**. That is the point of it: on macOS an invisible or minimised window is put under
  App Nap — the `building meshing:` log line is only trustworthy while the screen is
  awake. `map::measure_layers` / `map::measure_cars` are the entry points;
  `measure_layers` takes the two decisions the measurement actually reads (height mode +
  roof clutter), not a `BuildingPlan` — its `shadows` field would have been ignored —
  and they call exactly the builders `mesh_buildings` calls. **Absolute numbers still
  depend on the machine's power state** (with the display asleep everything is 2–3×
  slower), so compare runs, not runs against the log. **Shadows are measured at the
  default sun**: the sweep length and direction come from the process global of
  `map/sun.rs`, so the bench sets the sun itself —
  `map::apply_sun_style(SunStyle::default())` — and prints the azimuth/elevation in its
  header line; the live app builds with the sun from `settings.toml`, which is a second
  reason a log line and a run are not comparable. **Rails and tram get a row per zoom
  bucket** (`ZoomBucket::at(index)`, which exists for this) — their buckets differ in
  *what is drawn*, not in size — and **the tram is measured switched on**, though it
  ships off: the bench is about what the layer costs, not about whether it is shown.
  **Unmeasured: `fences`, `wagons`, `industry` and the trees** (both `mesh_trees` and
  `spawn::mesh_tree_row_band`). The first three are cheap single-pass layers with no
  zoom ladder worth a row per step, and the trees want a different row shape altogether
  (their build is a crown pool plus a scatter; the `LayerCost` row has nothing to say
  about 15 k entities) — a row for any of them is a `measure_*` plus a `row(...)` line
  in the bench.
- **Merged meshes** (`map/meshing.rs` + `map/spawn.rs`, water and waterways in
  `map/water.rs`, road layers in `map/roads.rs`,
  rail layers in `map/rail.rs`, the tram layer in `map/tram.rs`, building layers in
  `map/buildings/`) — **one merged `Mesh2d` per layer**: `MeshBuilder`
  triangulates polygons via `earcutr` (holes supported, degenerate contours skipped +
  counted) and emits per-vertex colors. Which layers ride the flat `ColorMaterial`, the
  roof material or the surface material is in `CONTEXT.md`; the parking **markings** are
  the exception that proves the rule — paint over asphalt, so that layer stays on the
  flat `ColorMaterial`, while the parking asphalt itself is `SurfaceKind::Street` (it
  *is* asphalt). ~7000 buildings cost a handful of entities. Trees are individual
  entities on the near zoom step and merged `tree_crowns` chunks on the far ones (see
  `references/trees.md`, **Crown detail by zoom**).
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
    `Name` in the live world, i.e. what a BRP query looks it up by; `spawn_layer` also
    puts the **`MapLayer`** marker on every layer entity — the selection
    `brp layers` / `brp hide` / `brp probe` work from, so a layer is found by name
    without dumping the world's `Mesh2d`s (`.claude/live-app-project.md`, "Which layer
    draws a glitch"). The near-step tree crowns (`CrownStream`, entity per tree) are not
    layer meshes and carry no marker.
  - **`MaterialSpec`** — `Flat` / `Blend` / `Surface(SurfaceKind)` / `Roof` / `Paint(PaintPass)`
    / `Crown` (the one app-wide `CrownMaterialHandle`, for the merged far crowns). It **names** the
    material instead of carrying a `Handle`, and a handle is the only thing that would
    have dragged Bevy into the build: with a spec the build needs neither `Commands` nor
    `Assets`, so the game, a test and the offline bench call one and the same function.
    Resolving spec → handle lives in `spawn_layers` and only there.

  The converted modules, the report states ("the layer is not drawn" is a state of the
  report, never a zero in a counter), the trees/cars/buildings shapes of the seam,
  `rebuilds_on()`, and the checklist for converting the next module — all in
  `references/layer-seam.md`.
- **The noise rule** — the zoom rule is one function,
  `visible(wavelength, px)` with `px = fwidth(world position)`: an octave shorter than 1.5 px
  contributes nothing and one longer than 4 px contributes fully — the noise is centred, so
  a faded octave shifts no brightness, and zooming out makes a surface smoother, never
  brighter or shimmering. That rule is **not this shader's own**: the shared library
  `assets/shaders/noise.wesl` holds all six helpers — `hash21`, `value_noise`, `visible`,
  `fbm4` (four octaves), `fbm3` (three) and `stripes` — and each shader imports only
  the names it calls (`import super::noise::{value_noise, visible, fbm3, fbm4};` here —
  the hash reaches it inside `value_noise`; `roof.wesl` takes its own
  subset, `crown.wesl` only `fbm3`), so a rule that must not drift is kept in one place.
  **Only what more than one shader calls moves there**: `dash_distance` (lane dashes)
  lives in `paint.wesl` and `band` (the drive between garage rows) in `roof.wesl`,
  each with a single consumer — a helper calling the library is not itself a reason to
  move it into the library. The surface material itself — the per-`SurfaceKind` noise
  params, the `Ribbon` attribute, the dropped sprite grain — is in
  `references/layer-seam.md`.
- **The layers themselves are in the references**, one file per subject, so a session
  reads only its own. A cross-reference «**X** below/above» inside any of them names a
  bullet by its bold title; this list says which file holds it.
  - `references/roads.md` — the roads hub and its subject files:
    `roads-sidewalks.md` — **Sidewalks** and **Kerb pockets** (the verges, the paved
    and unpaved streets, the lawn scraps); `roads-pairs-medians.md` — **Paired halves**
    (the median: asphalt and double solid, or a lawn with a kerb, the tram bed and
    band); `roads-streets-network.md` — **Streets, sections, tapers**, the **Ribbon**,
    **The drawn network** (pinned nodes, stitches, **The street axis**); `roads-markings.md`
    — **Markings** (with **Breaks** and `lane_count`); `roads-junctions.md` —
    **Junctions**, **Driveway crossings**, **Turning circles**, **Safety islands**;
    `roads-junction-paint.md` — **Junction paint**; `roads-corners.md` — **Kerb
    returns**; `roads-rings.md` — **Roundabouts**; `roads-merges.md` — **Merges**;
    `roads-bridges-wear.md` — **Bridge layers**, **Asphalt wear**, **Turn paths**;
    `roads-gallery.md` — **The junction gallery** (`examples/demos/roads`); the hub
    itself holds **RoadStyle and RoadShape**.
  - `references/parking-layout.md` — **Parking** (the layout, aisles, pockets);
    `references/parking-big-lot.md` — **A big lot shows the road through it** (the lot
    kerb, the medians and the **gores**); `references/parking-parked-cars.md` —
    **Parked cars** (rows, junction breaks, districts, bodies, `CarStyle`).
  - `references/layers.md` — **Rims**, **Shoal**, **Waterways**, **The yard**,
    **Pitches**, **Rail layers**, **Tram**, **Industry**, **Standing wagons**, **Fences**.
  - `references/buildings-heights.md` and its siblings (`buildings-landmarks.md`,
    `buildings-material.md`, `buildings-clutter.md`, `buildings-garages.md`) and
    `references/trees.md` — **Buildings** (below) and the
    trees.
- **Zoom buckets** (`map/zoom.rs`) — the one mechanism behind every zoom LOD. Each
  layer keeps its own table (`RAIL_LODS`, `TRAM_LODS`, `FENCE_LODS`; the cars', the
  wagons' and the roof clutter's are a single threshold each, the paint's are its three
  zoom maxima, the road verges' is one bound — wide verges drop off it, `VERGE_ZOOM_MAX`
  in `roads-sidewalks.md`) and names it with a marker type
  implementing `ZoomLods` (`RailLods`, `TramLods`, `FenceLods`, `CarLods`, `WagonLods`,
  `BuildingLods`, `PaintLods`, `VergeLods`, `TreeLods` — empty enums handing over the `max_zoom`s; the
  trees' table caps the density prefix, `references/trees.md`; the paint's
  and the trees' buckets only flip `Visibility`, the buildings' only a roof-material
  uniform — nothing is rebuilt — **Markings** in `roads-markings.md`,
  `references/buildings-material.md`). `ZoomBucket<T>` is the
  resource with the current index for that table;
  `for_zoom` is the single selection rule (first bucket whose bound is above the zoom,
  a zoom on the bound goes up — `zoom/tests.rs`). Three generic systems:
  `update_zoom_bucket::<T>` each Update via `set_if_neq`, so the `retuned`-gated
  `rebuild_*` runs only on a threshold crossing; `show_lod_layers::<T, F>` every
  frame, flipping the visibility of the prebuilt steps (the crossing paragraph
  below); and `seed_zoom_bucket::<T>` on world
  entry — in the `WorldInitSet::Spawn` chain right before the layer's first `rebuild_*`
  and after `camera::place_camera_on_world_ready` (hence that system is `pub(crate)`),
  written without change detection so the build that follows stays the only one. The
  seed exists because the camera opens on the saved zoom (`position: save` is the
  default) or, on a city switch, on `START_ZOOM`, and neither matches the index the
  resource happened to hold; the first build then went by the wrong bucket and the
  first Update redid it. The bucket is not
  persisted (the camera dictates it), and `Default` is the farthest, cheapest bucket.
  **A crossing itself costs what the layer decides to make it cost**, and the
  expensive layers decided to make it nothing: the rails (`rail::build_rails`) and
  the road verges (`spawn_map` + `roads::build_other_road_step`) build **every**
  step of their table once at world entry, each layer tagged
  `zoom::LodStep(index)`, only the camera's step visible — a crossing flips
  `Visibility` (`zoom::show_lod_layers`, idempotent, every frame) and pays no
  rebuild. That is where a near crossing used to cost the frame: rail 30–70 ms,
  the verge step a full `rebuild_roads` of 380–480 ms. The price is load time
  (the sum of all steps, ~90 ms rails + ~400 ms the second road step on Tula)
  and GPU memory for the hidden steps. The roads' style/sun/shape rebuild still
  builds **one** step and despawns the other — the first crossing after it
  rebuilds the missing step in `roads::show_road_steps`, so a style change pays
  the second build once, on the next crossing, not eagerly. The cheap ladders
  (tram, cars past the placement cache, fences, wagons) still rebuild per
  crossing — their builds cost 1–15 ms, under or about one frame, and
  precomputing them would buy load time and memory for nothing. The paint has no
  `LodStep`: its geometry does not depend on the verge step (identical vertex
  counts in both builds' reports), so it is spawned once by whichever build ran
  for the current step and `show_paint` owns its visibility alone — the reason
  `show_road_steps` filters `Without<paint::PaintTag>`.
- **Buildings** (`map/buildings/`) — the layer set a house is drawn as; the detail lives
  in the `references/buildings-*.md` files. Which file builds what:
  `mod.rs` picks the layers and builds them (`mesh_buildings`, `rebuild_buildings`,
  `spawn_building_meshes`), `layers.rs` the facade band, the roofs and the 2.5D walls,
  `shadows.rs` the two shadow layers, `roofs.rs` the pitched and the landmark roofs,
  `temples.rs` + `fortress.rs` the churches and the kremlin, `heights.rs` the storeys
  inferred where OSM carries no height, `material.rs` (shader `assets/shaders/roof.wesl`)
  the roof and wall material, `clutter.rs` what stands on a roof, `garages.rs` the garage
  rows, `arches.rs` the passage openings, `order.rs` the painter's order the whole thing
  is drawn in. Which of the layers exist at all is **`BuildingHeightMode`** (resource,
  BRP-writable, persisted; section `ui/buildings.rs`, Map tab below Trees and Tree rows,
  one cycling row) — `Facade`, `Shadows`, `ShadowsTint`, `Extrusion`,
  `ExtrusionShadowsTint` (the default); any change reruns `rebuild_buildings` (despawn
  `BuildingLayerTag` layers, respawn from the unchanged `MapData::buildings`).
  **The mechanism, the measurements and the rationale are in the `references/buildings-*.md`
  files**: heights (the height modes, their shadow sweeps and inferred storeys), landmarks
  (temples and the fortress, the arches), material (the roof and wall material with its
  shader and its two galleries), clutter (what stands on a roof), garages (the rows).
