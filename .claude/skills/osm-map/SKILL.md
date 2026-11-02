---
name: osm-map
description: Use when working on the OSM pipeline or map rendering in qwe — the Overpass query/mirrors/cache, map/osm/* parsing, the MapData model, building heights, entrance generation, tree planting and crowns, merged-mesh rendering, road/rail/tram/bridge layers, building and tree style resources. Deep detail behind CONTEXT.md's OSM section.
---

# OSM map pipeline — deep detail

This is the detail layer behind the **OSM map pipeline** summary in `CONTEXT.md`:
how the downloaded data becomes `MapData` and pixels.

This file is the **map and the invariants** every layer shares; the mechanisms live in
the reference files next to it, read on demand — **read the
one your change is about, not all of them**. A cross-reference «**X** below/above»
inside any of them (or inside this file) names a bullet by its bold title; this index
says which file holds it:

- `references/parse-seam.md` — the parse seam (`ParseKnobs`, `read_elements` /
  `finish_parse`, pass contexts) and **Testing the parse**.
- `references/parse-buildings.md` — the building readings and finishing passes (heights,
  building use, churches, the fortress, drowned buildings, squaring, houses pulled off
  the sidewalks).
- `references/parse-geometry.md` — the geometry passes (path width, rail gaps, the
  sidewalk/pavement passes, blocks pulled to the roads, ground pockets, ring assembly).
- `references/model.md` — the **MapData field catalog**: per type, every field's parse
  provenance, consumers and the invariants it carries.
- `references/roads.md` — the roads hub: the phases of `build_roads` and the index to
  the subject files — `roads-sidewalks.md` (sidewalks, verges, kerb pockets),
  `roads-pairs-medians.md` (divided streets and their medians, the tram band),
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
  with per-city counts, and how to regenerate them (`tools/osm_audit/`). Widening the
  query or adding a `parse_way` branch means updating it in the same change — a
  feature that already exists must not get "added" twice.
- `references/entrances.md` — the door generator: the measured statistics behind the
  cohort table, the pitch law, blocked walls, determinism.
- `references/buildings-heights.md` — how a house is drawn: the height modes and their
  shadow sweeps, inferred storeys. Its siblings: `buildings-landmarks.md` (temples and
  the fortress, the arches), `buildings-material.md` (the roof and wall material with
  its shader), `buildings-clutter.md` (what stands on a roof), `buildings-garages.md`
  (the garage rows).
- `references/trees.md` — planting (woods, standalone, rows, density/thresholds) and
  rendering (crowns, TreeStyle/TreeRowStyle, conifer stands).
- `references/tree-algo.md` — the watabou Village Generator crown algorithm (in
  Russian), reverse-engineered from `Village.js`; the ground truth `map/trees/crown.rs`
  is written against.

## From a screenshot to the OSM feature

A map bug usually arrives as a screenshot. Two scripts
turn it into OSM ids — before them, every session rewrote the projection by hand:

```bash
tools/shot_coords <png> [px py]        # the HUD camera line → centre / cursor / pixel in map metres
tools/osm_near <x> <y> [radius]        # ways, relations and nodes there, nearest first, d=0 inside
tools/osm_near --id 179102449          # one element: all tags, every vertex
```

**Position comes from the screenshot's HUD** (the camera line of the top-right **point
inspector** panel, `zoom/x/y cx/cy`), never from `brp cam` — the live camera has moved
since the picture was taken. The user's
screenshots are arbitrary screen regions from a macOS utility; `shot_coords` handles those
(the panel is its ruler) as long as the panel is in the crop. Details and caveats (window
size, zoom precision) — `.claude/live-app-project.md`, "Where a screenshot was taken". In
the **live app** the same question is one call: `brp probe <x> <y>` — its report lists the
drawn layers *and*, in the `features at the point (semantic index)` section, the map
features under the point. Beside the tagged features the list carries what the road
draws itself, in two halves: the sidewalk bands are model-measured, off the OSM axis by
the width law (smoothing and driveway crossings are not in them), while the verge zone
answers with the **drawn pieces** (`roads::DrawnPieces`) — kerb returns among them — so
a hit names the contour the last full build laid, at its verge zoom step rather than the
current zoom (a threshold crossing toggles layer visibility without a rebuild); for the
rest of the drawn geometry `brp probe`'s layer section is the arbiter. The label dictionary (including
the `#id` tail and its `tools/osm_near --id <id>` follow-up) is in `ui-panels`, the
inspector panel's skill.
**A translucent yellow shape in the shot is the inspector's highlight — the user's
pointer, not a map layer and not the question.** `tools/shot_coords` reads the hits
list together with the camera line and marks the row the user picked: the question is
about **that feature** — why it is there, why it looks or lies the way it does. **But
the picked row is the entry point, not the verdict on the culprit**: an artifact is a
*visual defect* — a seam, a kink, a wrong-coloured patch — and it lives where the
feature meets the layers under it as often as inside the feature itself, so read the
rows below the picked one too, in z order. The
overlay draws at `Z_INSPECTOR_HIGHLIGHT`, above every map layer, so it also outlines
parts the map itself hides under roads and paint — judge the feature on the map beneath
the overlay.
`osm_near` reads the newest
`assets/osm/<city>_*.json` and
projects with the centre and size from its name, i.e. the same metres as `SimPosition` and
`brp cam`; to confirm the pick live, `brp cam x y` then `brp shot`.
**With no app running, the same questions are answered offline:**
`cargo run --example pocket_probe -- x y [city]` prints the ground pockets, the
coverage bands and the **drawn road layers' z stack** at a point, plus a z-ordered
render of what `mesh_roads` draws with a layer legend — the drawn-geometry arbiter
when BRP is not an option. `tools/layer_colors [r g b | png px py]` maps a colour —
typed in or sampled from a screenshot pixel — to the layer colour constants it matches
(`ROAD_COLOR`, `SIDEWALK_COLOR`, …), which names the layer a strange-coloured patch on
a screenshot belongs to.

## Download & cache

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). The full tag list lives in the query
  itself (`overpass.rs`); the per-tag, per-city audit is `references/osm-coverage.md`.
  The bbox is `MAP_SIZE` around the selected `City`'s geo center. `QUERY_VERSION` is
  **15** and may only **grow**: caches stay on disk, so a reused number would silently
  serve extracts lacking the newer tags — bump it whenever the query gains tags. The
  version history (v3–v15, the skipped v12) is in the
  `QUERY_VERSION` doc comment and `references/osm-coverage.md`.
- **Driving side** — a second output after `out geom` (the `is_in` →
  `rel(pivot)["driving_side"]` → `out tags` chain): the tag is not on roads — OSM puts
  it on the **country boundary**, and everything inside inherits it. `out tags`, never
  `out geom` — a country's border geometry is megabytes.
  `is_in` needs areas, which not every mirror builds; an empty answer leaves
  `MapData::traffic_side` at `TrafficSide::Right` and logs it (`parse.rs::driving_side`,
  the innermost `admin_level` wins).
- **Mirrors** — `OVERPASS_URLS` in `download.rs` is tried in order; the VK/Mail.ru
  instance leads (full planet, current data, the nearest pipe from here). On dense
  cities those answer 504 "server too busy", or worse, a **200 with an HTML error
  page** — hence the "response must start with `{`" check before a mirror is
  considered successful.
- **Cache** — `assets/osm/tula_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored).
  Parameters live in the file name, so changing settings invalidates it. Written
  **only after successful parse**; a broken cache self-heals (deleted, re-downloaded).
  Second launch never touches the network.
- **The cache has a second reader** — the junction gallery (`examples/demos/roads`)
  reads it through `download::city_extract` and cuts its windows out with
  `map/osm/crop.rs`, so a query bump reaches the gallery with no step of its own
  (`references/roads-gallery.md`).
- **One file per city** — every load first runs `prune_stale_caches()`: anything under a
  known city slug that is not that city's current `cache_path` is deleted. That is what
  retires extracts left by an old geo center, `MAP_SIZE` or `QUERY_VERSION` — tens of MB
  each. It sweeps **all** cities, not just the one being loaded; the current file of
  each city survives, so a tour of the six is not six downloads per lap.

## MapData — the parsed model

`map/osm/model.rs`; the resource stays resident after spawn. The **field catalog** —
per type, every field's parse provenance, consumers and invariants, starting with why
the nine `Vec<PolyArea>` layers are not `AreaKind` written twice — is
`references/model.md`. The roster in one breath: `PolyArea` (the nine layer vectors of
`AreaKind`), `RoadLine`, `RoadNode`/`RoadArea` (the junction vocabulary), `RailLine`,
`WallLine`, `FenceLine`, `Structure` and `PipeLine` (industrial cylinders and overhead
heating mains), `WaterLine` (*linear* watercourses), `TreeRow`/`TreeNode`, the planting
raw material, and `trees` (`TreeSet`, what the renderer reads). **RoadLine** —
centerline + width from its section, its point order already normalized by the parse;
the rest of the fields are in `references/model.md`.

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
  disagree about it. It stays an argument, not a module constant: the number is about
  the domain — the doors' road index and footprints, the shadows, the car districts
  each pick their own.
- **The primitives and the conveniences on top.** `near_each(min, max)` — everything in
  the touched cells **as is**, duplicates included, in a fully determined order (cells
  ascending by x then y, insertion order inside a cell); `cell(IVec2)` / `at(point)` are
  the one-cell lookups; `near(min, max)` sorts and dedups (needs `T: Ord`); `pairs()`
  is every pair sharing a cell, sorted and deduped.
- **`insert(min, max, value)` puts the value in every cell its box touches** — that
  invariant is what makes a one-cell `at` complete rather than approximate. An error
  there returns a silently incomplete answer, which is why it lives in one place.
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
  a **scalar**: `Vec2 - f32` is glam's own, so no caller writes a `Vec2::splat` any
  more.
- **`near` sorts because the mesh must not move between runs**; `near_each` is the
  escape hatch for values that are not `Ord` (`water.rs`
  indexes `(Vec2, Vec2)` edges) and for callers that already tolerate duplicates.
- **The map is `bevy::platform::collections::HashMap`, not `std`'s** — the same
  `hashbrown`, with `foldhash` in place of SipHash. A city-wide index is tens of thousands
  of inserts (the stitches alone index 14 642 buildings and water bodies), so at that
  traffic the hash *is* the price of the index, and an `IVec2` key has no use for a
  DoS-resistant one. **The swap also removes a nondeterminism rather than adding one**:
  std seeds SipHash per process, so a map iterated for its values handed out a different
  order every run, while bevy's `FixedHasher` has a fixed seed. Still on `std`: the
  parse's and the reader's tag maps, which serde deserializes into.
- **`cell_of` is public for one caller**, `entrances::RoadIndex`, which walks cells in
  **rings** outward, stopping when what it found beats anything the next ring could
  hold — that strategy belongs to the caller, not the grid.
- **`spatial.rs` is not this grid and does not move here.** The pawn grid is a dense `Vec`
  over the whole map with a reverse entity→cell index and a per-tick move of one entity at
  a time; it shares nothing with a `HashMap` of boxes built once per load but the word.

## The shape vocabulary — `map/shapes.rs`

The map's one vocabulary for `i_overlay`: `Contour`
(`Vec<[f32; 2]>`) and `Shape` (outer ring, then holes), with `oriented` / `area_contours`
giving the CCW-outer, CW-hole winding NonZero needs; the offset rounding `ARC` 0.3, the
ring tolerance `RING_EPSILON` 0.01, and the rest — `stroke`, `push_shape`,
areas/bounds, `ring_of`, `point_in_shape` — beside them.

**`stroke` takes `ring` as an argument rather than asking `is_ring` itself**, and that is
the one place the two callers genuinely differ: a road ring is stroked closed, without
caps, while the parse strokes a run of road pieces open even when the run happens to come
back on itself.

## Footprint bands

`map/footprint.rs` — the strips linear geometry occupies on the ground, `Band { line,
width, role }`, one construction for every consumer (road decks, curbs and passages,
water channels (`None` for culverts), walls, fences with their `fence_gaps`). The width
policy (`casing_width`, `bridge_curb_width`) lives here too, its ranges deliberately
disjoint so a curb always out-sticks a casing — because a curb is not just paint: the
same band blocks the navmesh, and the drawn strip must match the blocked one by
construction. Bands carry
the **centerline**, not a ready outline: the grid fill needs the centerline for its
4-connected-chain rasterization guarantee, the mesh build turns it into an outline, and
the renderer draws its own smoothed copy and takes only the widths (smoothing must not
move what blocks).

`CurbCoverage` also lives here: the shared *inputs* of the composite-bridge curb
decision — the bridge and joining-roads lists, filtered by one `ways_joined` (moved
from `navigation`). `ways_joined` is reached only through an AABB prefilter
(`boxes_may_join`, bridge boxes precomputed once, both inflated by `JOIN_EPSILON`):
roads are tens of thousands and bridges hundreds, ~1% of them join, so the unfiltered
product cost about a second per call against 19 ms with it — and `build` runs twice per
load, once for the grid fill and once for the mesh build. The decision itself is
deliberately NOT unified: the grid blocks curb tiles by a directional outward probe, the
mesh subtracts band polygons, and both survive the "primary's nominal 16 m swallows its
parallel sidewalk" trap in their own way — a shared point-coverage test reproduces
neither (with slack it opens the composite's outer barrier, without slack it crumbles
the curb chain into dashes). Analysed and rejected once already; re-attempting it means
going through the curb pin tests
(`navmesh/fill/tests.rs`) and the parity tests.

## Rendering

- **One RNG and one point seed for the whole of `map/*`** (`map/seed.rs`) — everything a
  layer scatters must survive a rebuild — a zoom-bucket crossing, a height-mode switch,
  a restart — so the layers share two primitives instead of copying them. **`Lcg`** is the
  Park–Miller (Lehmer) generator of `Village.js` — `seed = 48271·seed mod 2³¹−1`, plus
  `range`/`gauss3`/`bell4` helpers. **`seed_from_point(Vec2)`**
  is the seed itself: the object's **own reference point** in
  centimetres — the first vertex of a footprint, the first point of a street — through three
  mixing rounds, never the object's index in the extract, which a re-parse is free to move.
  Callers: the building material and clutter, the crowns, the cars, the wagons.
  A copy of either primitive is the defect this module exists against.
  The **parse** stage is deliberately not on it — doors (`osm/entrances/`) and tree planting
  (`osm/planting.rs`) run on `rng::lcg_seeded_by`, a different point-seeded LCG, and rewiring
  them would move every door and every tree in every city.
- **Measuring the layer build** — `cargo run --example map_meshing -- [city slug]`
  (`examples/bench/map_meshing.rs`) prints vertices and milliseconds per layer for every
  height mode × clutter bucket, plus the car layer's rows with its junction breaks split
  out, straight from the Overpass cache, **no window and no GPU** — on macOS an
  invisible or minimised window is put under App Nap, so the `building meshing:` log
  line is only trustworthy while the screen is awake. The entry
  points `map::measure_layers` / `map::measure_cars` call exactly the builders
  `mesh_buildings` calls; `measure_layers` takes just the two decisions the measurement
  reads (height mode + roof clutter), not a `BuildingPlan` — its `shadows` field would
  have been ignored. **Absolute numbers still depend on the machine's power state**
  (with the display asleep everything is 2–3× slower), so compare runs, not runs against
  the log. **Shadows are measured at the default sun**: the sweep length and direction
  come from the process global of `map/sun.rs`, so the bench sets the sun itself
  (`map::apply_sun_style(SunStyle::default())`, azimuth/elevation in its header line);
  the live app builds with the sun from `settings.toml` — a second reason a log line and
  a run are not comparable. **Rails and tram get a row per zoom bucket**
  (`ZoomBucket::at(index)`, which exists for this) — their buckets differ in *what is
  drawn*, not in size — and **the tram is measured switched on**, though it ships off:
  the bench is about what the layer costs, not about whether it is shown. **Unmeasured:
  `fences`, `wagons`, `industry` and the trees** (both `mesh_trees` and
  `spawn::mesh_tree_row_band`): the first three are cheap single-pass layers with no
  ladder worth a row per step, the trees (a crown pool plus a scatter, ~15 k entities)
  want a different row shape — a row for any of them is a `measure_*` plus a `row(...)`
  line in the bench.
- **Merged meshes** (`map/meshing.rs` — `MeshBuilder` and its frame types, plus
  subjects `map/meshing/{ribbon,paint,geometry}.rs` (strokes and ribbons, paint
  strips, free geometry) and `map/spawn.rs`; per-layer geometry lives in the
  layer's own module) — **one merged `Mesh2d` per layer**: `MeshBuilder`
  triangulates polygons via `earcutr` (holes supported, degenerate contours skipped +
  counted) and emits per-vertex colors. Which layers ride the flat `ColorMaterial`, the
  roof material or the surface material is in `CONTEXT.md`; the parking **markings** are
  the exception that proves the rule — paint over asphalt, so that layer stays on the
  flat `ColorMaterial`, while the parking asphalt itself is `SurfaceKind::Street` (it
  *is* asphalt). ~7000 buildings cost a handful of entities. Trees are individual
  entities on the near zoom step and merged `tree_crowns` chunks on the far ones
  (**Crown detail by zoom** in `references/trees.md`).
- **The layer seam** (`map/surface.rs`) — building a layer and putting it in the world
  are two things, and this is the line between them. A **converted** module offers one
  pure function, `mesh_<layer>(data, style) -> (Vec<LayerMesh>, <Layer>Report)`, and
  its system is a thin adapter: despawn by tag, call it, hand the list to
  `surface::spawn_layers`, print the report.
  **A layer with a second caller gets one door for both** — `spawn_<layer>_meshes(...)`,
  taking what is already built so the build stays
  outside the world. `buildings` and `roads` have one each (`spawn_building_meshes`,
  `spawn_road_meshes`), because `spawn_map` spawns their layers at world entry and a
  `rebuild_*` respawns them on a style or bucket change. The trigger is the **number of
  callers, not the number of tags**: without the door `spawn.rs` would have to know the
  layer's tag and the "spawn, then `info!`" order. A module with one caller (the surfaces)
  needs none.
  - **`LayerMesh`** — `{ builder, z, offset, name, material: MaterialSpec }`, **one type for
    every layer of the map**, not a type per module. That is the point: a module read as
    `-> Vec<LayerMesh>` is read the same way as any neighbour. `name` is the entity's
    `Name` in the live world, i.e. what a BRP query looks it up by. `offset` is the
    layer's xy shift in the world — world = map + shift: map layers stand in map
    coordinates and carry `Vec2::ZERO`, and whoever draws a map-space layer in a
    shifted world sets it (`LayerMesh::offset`, the roads gallery's
    `map/inspect.rs::InspectOrigin`). `spawn_layer` also
    puts the **`MapLayer`** marker on every layer entity — what
    `brp layers` / `brp hide` / `brp probe` select from, so a layer is found by name
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
  the names it calls (`roof.wesl` its own subset, `crown.wesl` only `fbm3`), so a rule
  that must not drift is kept in one place.
  **Only what more than one shader calls moves there**: `dash_distance` (lane dashes)
  stays in `paint.wesl` and `band` (the drive between garage rows) in `roof.wesl`,
  each with a single consumer. The surface material itself — the per-`SurfaceKind` noise
  params, the `Ribbon` attribute, the dropped sprite grain — is in
  `references/layer-seam.md`.
- **Zoom buckets** (`map/zoom.rs`) — the one mechanism behind every zoom LOD. Each
  layer keeps its own table — a full ladder, a single threshold (cars, wagons, roof
  clutter), the paint's three zoom maxima, or the verges' one bound (wide verges drop
  off it, `VERGE_ZOOM_MAX` in `roads-sidewalks.md`) — and names it with a marker type
  implementing `ZoomLods`
  (`RailLods`, `PaintLods`, `TreeLods`, … — empty enums handing over the `max_zoom`s;
  the trees' table caps the density prefix, `references/trees.md`; the paint's
  and the trees' buckets only flip `Visibility`, the buildings' only a roof-material
  uniform — nothing is rebuilt — **Markings** in `roads-markings.md`,
  `references/buildings-material.md`). `ZoomBucket<T>` is the
  resource with the current index for that table;
  `for_zoom` is the single selection rule (first bucket whose bound is above the zoom,
  a zoom on the bound goes up — `zoom/tests.rs`). Three generic systems:
  `update_zoom_bucket::<T>` each Update via `set_if_neq`, so the `retuned`-gated
  `rebuild_*` runs only on a threshold crossing; `show_lod_layers::<T, F>` (the
  crossing paragraph below); and `seed_zoom_bucket::<T>` on world
  entry — in the `WorldInitSet::Spawn` chain, right before the layer's first
  `rebuild_*` and after `camera::place_camera_on_world_ready` (hence that system is
  `pub(crate)`), written without change detection so the build that follows stays the
  only one: the camera opens on the saved zoom (`position: save` is the default) or,
  on a city switch, on `START_ZOOM`, and neither matches the index the resource
  happened to hold — the first build went by the wrong bucket and the first Update
  redid it. The bucket is not persisted (the camera dictates it), and `Default` is the
  farthest, cheapest bucket.
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
- **Buildings** (`map/buildings/`) — the layer set a house is drawn as; the mechanism,
  the measurements and the rationale are in the `references/buildings-*.md` files (the
  header index names them), and the module names in `map/buildings/` say what each
  file builds (`order.rs` is the painter's order). Which of
  the layers exist at all is **`BuildingHeightMode`**
  (resource, BRP-writable, persisted; section `ui/buildings.rs`, Map tab) — `Facade`,
  `Shadows`, `ShadowsTint`, `Extrusion`,
  `ExtrusionShadowsTint` (the default); any change reruns `rebuild_buildings` (despawn
  `BuildingLayerTag` layers, respawn from the unchanged `MapData::buildings`).
