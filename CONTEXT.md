# CONTEXT

Domain glossary for QWE. Use these terms verbatim in commit messages, hypotheses, test
names, and code identifiers. If a concept you need isn't here, that's a signal — either
you're inventing language the project doesn't use (reconsider) or the file has a real gap
(update it in the same change that introduces the concept).

**What belongs here, and how much.** This file holds *terms and invariants*; the
*mechanisms and their justifications* live in seven detail skills —
`.claude/skills/{osm-map, navigation-deep, sim-speed, ui-panels, determinism,
species-behavior, world-lifecycle}` — loaded on demand (see CLAUDE.md, "Skills").
Budget per entry: **~5–10 lines** — what the term is, the invariant(s) that hold, where it
lives, at most a one-line why, then a pointer to the skill that owns the mechanism. A
measurement's *conclusion* stays here ("25 m is the measured median pitch"); the
methodology, the tables and the derivations go to the skill. A "this was tried and was
wrong because…" guardrail is one line, not a paragraph. **An entry that wants a table, a
derivation or a war story is telling you it belongs in a skill.** When a change touches a
concept, update the summary here and the detail in the skill **in the same change** — a
stale glossary is worse than none.

## Project shape

**QWE** is a 2D real-time simulation prototype: a **demon invasion of the Tula city
center**. The map is generated from real OpenStreetMap data at first launch. 20 000
humans wander the streets; demons pour out of a portal, chase and devour them; humans
panic and flee off-map. Built on **Bevy 0.19 ECS** — one plugin per feature, registered
in `main.rs`.

## Coordinates & units

- World units are **meters**. Origin — **south-west corner** of the map, y grows north.
  All world coordinates are positive. `MAP_SIZE = 5600 × 3700` m.
- **Navtile** — navigation grid cell, **2 m by default, runtime-switchable to 1 m** via the
  `navtile:` cycler in the Debug tab. Grid size is derived as `MAP_SIZE / navtile_size()`
  (2800 × 1850 tiles at 2 m); the live value is a process-global atomic
  (`settings::navtile_size()`), written only in `OnEnter(Loading)`, and **a filled `Navmesh`
  carries its own `grid_size`/`tile_size` snapshot** so stale snapshots never index against
  the switched atomic — the fill and the navmesh-side queries convert through
  `Navmesh::to_tile` / `Navmesh::tile_center`, never the global pair. Switching reloads the
  world like a city switch, except the camera stays put. `grid.rs`: `world_to_tile` /
  `tile_center`, for callers with no `Navmesh` at hand. Costs and the chunk scaling —
  **navigation-deep skill**.
- **Post-processing** (`post.rs`) — the camera renders to an HDR target with **bloom**
  thresholded at 1.1, so only what draws itself above 1.0 glows — the portal vortex, demon
  halos, soul sparks — and the map's white markings and light roofs do not; tonemapping is
  off so the map palette is untouched, and `Msaa` stays off. A full-screen **vignette** is
  a UI node under the panels, `Pickable::IGNORE` (detail in the `ui-panels` skill).
- **Viewport** (`camera.rs`) — the piece of the world in frame, as a value: `centre`,
  `half_extent` (margin already applied), `zoom` (world m per logical pixel). `contains`
  — **the edge counts as inside**. Five visibility gates use it and **each keeps its own
  margin** (warmup 1.0, dispatcher/separation `VIEW_MARGIN` 1.2, movepath gizmos 3.0, door
  gizmos 1.5), because each asks a different question — the table is in the
  **navigation-deep skill**. Not Bevy's `Camera::viewport`, which is in pixels.
- **Geo anchor** — `GEO_CENTER_LAT/LON` (Tula, kremlin near frame center). Projection is
  local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW corner → (0,0),
  f64 math, `MAP_SIZE`-sized bbox derived from the center.
- **Z-layers** — constants in `settings.rs`, bottom to top: ground → landuse blocks →
  parks → woods → tree-row band casing → tree-row band → grass → sand → water → waterways → sidewalks →
  alley casings → alleys → road casings → roads → bridge casings → bridges → rail ballast
  → rail ties → rail steel → tram → cars → portal stain → corpses → portal → buildings (5) →
  units → souls (18) → tree shadows → trees (20). Three live in their own modules:
  `Z_BUILDING_SHADOW` 4.5, `Z_FACADE` 4.9 (`map/buildings/mod.rs`), `Z_WALL` 5.1
  (`map/roads.rs`). Units are y-sorted: `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR`
  (10 − y·0.002). **Invariant: the unit z range must stay above buildings (5) for any
  y ≤ MAP_SIZE.y** — a bigger map once sank northern units under roads.

## App lifecycle

Summary; the mechanism — **world-lifecycle skill** (states and the warmup hold,
`SimBootPlugin`, the load thread, the `WorldStarted` seam, restart slots, the city switch).

- **AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
  (progress, red error + **Retry**). **All world spawning happens in `OnEnter(Playing)`**,
  never in `Startup`.
- **PlayPhase** (sub-state of `Playing`) — `Warmup → Live`. During Warmup the world exists
  but `Time<Virtual>` is **paused** and the loader stays up counting pawns still routing
  *inside the camera view*; typical warmup **~0.15 s**, timeout 10 s. `Live` despawns the
  loader and reveals the game UI (`GameUiRoot`).
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. The navmesh must
  be filled before the population spawns, or humans land in the river.
- **SimBootPlugin** (`loading.rs`) — one world bring-up shared by the game and the replay
  app: the two states, the `WorldInitSet` chain, the warmup pause, the **WorldStarted**
  announcement on entering `Live` (chained before the unpause).
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` doing
  everything that needs no ECS: `Connecting → Downloading → Parsing → BuildingNavmesh →
  Pruning → Done | Failed`, every state a line on the loader screen. **Rule: heavy init
  belongs in this thread, not in `OnEnter(Playing)`** — no frame is drawn inside a schedule.
- **WorldStarted** (`loading.rs`, event) — "the world begins a new run", the single seam
  both lifecycle paths share, fired on entering `PlayPhase::Live` and on every restart. All
  run state (`SimClock` + `TickDebt`, `SimTick` + the frozen `Backend`, `Telemetry`,
  `DemonSpawner`, `SeparationStats`) is reset by observers of it, each in its owning
  module (`grep "On<WorldStarted>"`). **Membership is held from the outside** by
  `a_restart_replays_the_run`, not by hand. Map-derived state (`NorthstarGrid`,
  `PolyNavmesh`) is **not** run state — a restart keeps the map.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns pawns and corpses, fires
  **WorldStarted**, respawns the population; the navmesh persists. Under **Deterministic**
  this replays the previous run tick for tick.
- **RestartPending** (`restart.rs`, resource) — "a restart was ordered", the only way to ask
  for one from anywhere but R (a changed **world seed**, a flipped **Deterministic**).
  Consumed in `PreUpdate` after `InputSystems` — the same slot R uses, because a mass
  despawn may not happen in `Update` (CLAUDE.md). Always `to_portal: true`.
- **City** (`city.rs`, resource, persisted) — `Tula | NewYork | Paris | Berlin | London |
  Tokyo | DevilsLake`, each with its geo center, portal hint and cache slug. `MAP_SIZE` and
  therefore the derived `grid_size()` are shared, so switching city never resizes the
  navmesh. UI — a select at
  bottom centre (`ui/city.rs`).
- **City switch = full world reload** — writing `City` sends the app back to
  `AppState::Loading`; the scene is torn down, the new extract downloaded and parsed, the
  same navmesh refilled, the camera reset. Gated on `in_state(Playing)`: restarting a load
  on top of a running one would put two threads into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city. Every
  world entity must carry it; the rule and the list of spawn sites live in **CLAUDE.md**
  ("World entities"), and `loading.rs::warn_leftover_world_entities` warns when something
  survived.

## OSM map pipeline

Summary; the mechanism — **osm-map skill** (entrance statistics in
`references/entrances.md`, planting and crowns in `references/trees.md`, the tag coverage
audit in `references/osm-coverage.md`, the crown algorithm in `references/tree-algo.md`).

- **Overpass** — the Overpass API, queried once per city with `[out:json]` + `out geom`;
  bbox is `MAP_SIZE` around the `City` geo center. Mirrors in `OVERPASS_URLS` are tried in
  order. **Bump `QUERY_VERSION` in `overpass.rs` whenever the query gains tags**
  (currently 7), or existing caches keep serving extracts that lack them.
- **Cache** — `assets/osm/{slug}_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored);
  the parameters live in the file name, so changing them invalidates it. Written only after
  a successful parse; the second launch never touches the network. `prune_stale_caches()`
  keeps exactly one current file per city.
- **Overpass fixture** (`map/osm/fixture.rs::Overpass`) — a scene given in **map metres**,
  turned into an Overpass response and fed through the real `parse`, so a test states its
  scene in the same numbers it later asserts on. **Add a tag case here, not another
  literal.**
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes, rings open. `AreaKind: Building | Kremlin | Water |
    Park | Wood | Grass | Sand | Residential | Industrial`; **only Wood carries trees**;
    Residential/Industrial are the `landuse` **blocks** — a faint fill under everything
    else, no effect on navigation or planting. Buildings carry
    `height: Option<f32>`, `entrances: Vec<Vec2>` and `building_use: BuildingUse`.
  - **RoadLine** — centerline + width by highway class (primary 16 → footway 3.5);
    `RoadClass: Street | Alley`; `bridge` / `passage` flags (the navmesh carves by them);
    `oneway`, `roundabout` (`junction=roundabout|circular`, implies one-way) and
    `lanes: Option<u8>` (the tag, 1–8; the width default lives in `map/roads.rs`) — read
    by the markings only.
    Underground road is dropped (`is_road_underground`) — a **separate** predicate from
    `is_underground`, because the risk is asymmetric: an extra ribbon is cosmetic, an extra
    deletion is a hole in the navmesh.
  - **RailLine** — `railway=*` centerline; `RailKind: Active | Tram | Disused` *is* the
    drawing style; underground track is dropped. The rail branch of `parse_way` runs before
    `highway` and falls through — a way can be both street and track. A non-tram track is
    drawn as the **track** itself (`map/rail.rs`): ballast with a shoulder, ties across it
    and two steel rails on the gauge, thinned out by **rail zoom LOD** into osm-carto's
    dashed symbol on the city-wide view. Tram is `map/tram.rs`, with its own LOD, and is
    drawn only while `TramStyle::visible`.
  - **WallLine** — `barrier=city_wall` (the kremlin), 3 m, impassable.
  - **WaterLine** — a *linear* watercourse (`river` 8 m → `ditch` 1.5 m), falling through
    `highway` like rails. `tunnel: bool` marks a **culvert**: not drawn, and the only
    watercourse kind that does **not** block the navmesh.
  - **TreeRow** / **TreeNode** — `natural=tree_row` avenues and single surveyed
    `natural=tree` trees, with optional `spacing`/`radius` from tags.
  - **trees / tree_appears_at** — what the renderer reads; `compose_trees` merges forest +
    avenues of the selected layout, `composed_for` caches which.
- **Building height** (`parse/tags.rs::building_height`) — metres from `height` or
  `building:levels` × 3 m; outside 2–600 m counts as no tag. `None` is normal, and common:
  coverage varies wildly by city (NY 97 % … Tula 31 % … Tokyo 5 %) and is logged on load.
- **Inferred storeys** (`map/buildings/heights.rs`) — what a building without a `height`
  tag is drawn as, and it is **the shape of the footprint that decides**, the way an eye
  reads an aerial photo: a long thin box (≥ 35 m by ≤ 18 m) is a panel section (5 / 9 / 12
  storeys), a compact large one (≥ 500 m², sides within 1.7) a tower (mostly 9), a small
  one (≤ 300 m²) an old low building (2–4, over 300 m²: the area test comes first, so a
  long thin shed stays low), and industrial / commercial / church footprints are measured
  in **metres of span** rather than storeys.
  A **public** building (school, clinic, office — `BuildingUse::Public`) is measured in
  storeys, 2–5, but by its use and not by its shape: the use is asked first, so a large
  squarish school never comes out a tower.
  The slot inside each group comes
  from the building's own seed — the one that already picks its **Roof material** — so it
  is stable across rebuilds and modes. The tag always wins. Before this the whole 69 %
  took one of three numbers (3 / 6 / 15 m) and Tula's height distribution was median 15 m,
  p90 15 m; it is now median 8 m, p90 15 m, and the mix is printed in the `building
  meshing:` log line.
- **Building use** (`parse/tags.rs::building_use`) — the **drawing class** of a building,
  `BuildingUse: House | Apartments | Commercial | Industrial | Garage | Church | Public |
  Other`, from `building=*` and — whenever that value is outside the vocabulary, `yes`
  above all — from `amenity=*` on the same outline. Each class
  owns a (roof, wall) colour pair in `map/buildings/`; the Kremlin is coloured by `AreaKind`
  and ignores it. Not the bastion kind of `ROADMAP.md` — that is a separate concept.
- **Roofing** (`map/buildings/roofs.rs::roofing`) — the *shape* of a roof, **inferred**,
  not read (`roof:shape` is rare), in three kinds. A **gable** — two slopes with the ridge
  along the long axis of the minimum-area bounding rectangle — needs an outline that nearly
  fills that rectangle. A **hip** — a slope quad per outline edge, built by pushing the
  outline inward on miter offsets, with the leftover interior as the ridge plane — needs
  nothing but a footprint thicker than the inset, and so is what an **L-shaped house** gets
  (they used to stay flat among pitched neighbours). Which of the two a house takes is its
  own seed (4 in 10 hip). Everything else is **flat** — a real flat roof with its material
  and its clutter. Courtyard buildings and the Kremlin stay flat, outside use-based styling as
  with its colour. **`RoofShape`** is the same three as an *input*: the city never asks for
  one, `roof_gallery` does, to stand one outline under all three — and a refusal there stays
  a refusal instead of being swapped for another shape the way `roofing` swaps it.
  **`shape_facts`** hands out the numbers the choice is made from (rectangle fill, hip inset,
  either ridge rise) so the gallery prints them rather than restating them.
  Detail in the `osm-map` skill.
- **Roof material** (`map/buildings/material.rs`) — what a roof is *covered with*, and
  therefore what colour it is: `RoofKind: Bitumen | Gravel | Seam | Corrugated | Tile |
  Membrane`, picked deterministically from `BuildingUse` (+ footprint size for the untagged
  half) and a **seed hashed from the building's first vertex**, as the door generator is
  seeded. The colour comes from that material's own palette — **the per-use *roof* colours
  are gone**, `facade_color` is what `BuildingUse` still picks — and the texture from
  **`RoofMaterial`** (`assets/shaders/roof.wgsl`) reading the **`Roof` attribute**
  (`meshing::ATTRIBUTE_ROOF` = `[long axis x, y, material code, seed]`, **one value for the
  whole building**; code `0` is *not a roof* — walls and gables ride in the same
  mesh). **Roof age** is the second thing that seed carries (`roof.wgsl::roof_age`, hashed
  from it, no attribute of its own): one number per building that sets how many repair
  patches its bitumen carries (a young roof almost none, an old one a patch per second
  cell), how much water stands on it, and — on every material — how faded and dirty it is.
  Every flat roof of the city, in both flat modes and 2.5D, is laid by one call —
  **`push_flat_roof`**, a bare fill. A soft flat roof used to get a **parapet** on top of
  it, a 0.7 m inset band lit by the **Sun**; that is gone, because it is the same
  construction as a hip's slopes and only narrower — from the air every panel block wore a
  small hip, and a real hip could not be told from a flat roof. Strength — `RoofStyle::texture`
  (Buildings section, persisted), 0 = the flat fills of before. **A roof is now darker than
  the walls under it**, deliberately: that is the relation an aerial photo has, and the
  older "roof lighter than wall" rule is retired with the per-use roof palette. Every
  material and every palette side by side, with a house per colour from a 30 m block down
  to an 8 m shed, and above them every *shape* over five outlines:
  `cargo run --example roof_gallery` — whose houses are drawn by **`push_house`**, the
  per-building body of the 2.5D layer, walls included, because a roof shape does not read
  without them. Detail in the `osm-map` skill.
- **Roof clutter** (`map/buildings/clutter.rs`) — what stands *on* the roof: a lift
  penthouse, ventilation shafts, air-conditioning units, the skylight ribbons of an
  industrial shed, a chimney on a pitched ridge. Each is a small oblique box with its own
  **opaque** shadow (a translucent one could not blend inside the opaque building layer),
  placed by a Park–Miller LCG seeded from the same building seed the roof material uses,
  and every candidate is rejected unless all four corners fall inside the footprint —
  the placement frame is a rectangle, an L-shaped building is not. Which items a roof
  gets follows its **material**, not the building use: soft flat roofs carry the
  penthouse and the shafts, corrugated sheds the skylights, a gable a chimney.
  **The clutter is the only thing zoom changes about buildings** —
  `BuildingZoomBucket` (`ROOF_CLUTTER_MAX_ZOOM` 0.5 m/px) rebuilds the layer without it
  once a metre stops being worth two pixels, the way rail and tram rebuild themselves.
- **Lean** (`map/buildings/mod.rs`, `Lean`) — which way the *top* of a building is
  displaced, and the second thing (with the sun) that a 2.5D building answers to. One
  oblique skew for every building, which is what a **satellite** frame looks like: 5 km of
  city seen from 500 km up spans fractions of a degree, so the parallax is constant (an
  orthomosaic has none at all). `Lean` is a **per-building value** (metres of displacement
  per drawn metre of height, held as a vector) and carries the painter's key with it:
  **the far end of the skew is drawn first**, because the top of a far building is
  displaced onto a near one. **The sun is independent of it** — the lean is the camera,
  the shadow is the light. A radial lean away from the nadir — the signature of an
  *aircraft* frame — was tried and taken back out: the nadir is the centre of the **map**,
  not of the frame, so with a camera that pans the fan is only visible around the centre,
  and following the camera is out of reach while the layer is rebuilt on the CPU (tens of
  milliseconds). It belongs with a move of the skew into the vertex shader.
- **Sun** (`map/sun.rs`, `SunStyle`) — one light for the whole map, as **two knobs**:
  **azimuth** (clockwise from north; the default 300° puts the shadow down-right at 30°,
  the old `SHADOW_DIR` exactly) and **elevation** (default 59°, the summer noon of Tula's
  latitude — the hour a city is photographed from the air), from which
  **`shadow_length_scale()` = cot(elevation) = 0.60** metres of shadow per metre of height
  and **`sun_stretch()`** — the same number relative to that default, which is what every
  length calibrated at 59° is multiplied by (the buildings' `SHADOW_LENGTH_RANGE`, the
  crowns' shadow heights). Section *Sun*, persisted, and the same two knobs stand in
  `roof_gallery`. **It is read through a process global**, not a `Res`, for the
  same reason the navtile size is (`settings::navtile_size`): `shade_by_light`, the shadow
  sweep, the roof clutter and the cars are pure functions deep inside mesh building. What
  the global holds is the ready shadow vector and cotangent, not the two angles: it is read
  hundreds of thousands of times per layer build, and `sin`/`cos` from under an atomic are
  not hoisted out of a loop. **Only `apply_sun` writes it** — once in `Startup` after
  `seed_sun` (so that `init_roof_material`, which bakes the light into the roof material's
  uniform for the life of the process, reads the *saved* sun and not the compile-time one),
  and then every frame in `PreUpdate`. That makes the global readable on the main thread in
  `Startup` after that pair and between `PreUpdate` and the end of `Update`, and nowhere
  else (a reader in `PostUpdate`, in the render world or on a worker thread must take
  **`SunOnMap`** as a resource instead).
- **`SunOnMap`** — the sun the map is *built* with, as against `SunStyle`, the sun on the
  slider. `settle_sun` moves one into the other after `SUN_SETTLE` (0.35 s) of quiet, and
  it is `SunOnMap` that both the global and every rebuild follow (`retuned::<SunOnMap>`:
  building layers with their shadows, tree crowns, cars, the roof material's `light`
  uniform) and that the settings file is written from. One division of the slider costs a
  full building rebuild with its shadow union, so a drag across the scale would otherwise
  be seventy of them.
- **Soft shadow** (`map/buildings/layers.rs::shadow_builder`) — a building's shadow is no
  longer a hard silhouette: every contour of the union carries a **1 m band fading to zero
  alpha** (`PENUMBRA_WIDTH` — the photographic soft edge, which comes from the frame's
  resolution and the sky's fill light, not from the sun's angular size, and is therefore
  chosen by look), outward from the outer ring and into the gap from a hole. Its width is
  **tapered per vertex by `penumbra()` = the band direction projected on `shadow_dir()`**: a
  shadow meets its own building hard and blurs with distance, so the contact edge gets no
  band at all, the far edge the full metre, and a lateral edge grows from one to the other.
  Untapered, the metre also ran along the contact contour and left a soft dark blot on the
  sunlit side of every convex corner — the building came out ringed exactly like the
  **contact skirt** that was taken back out of the union. What goes into
  the union is still the silhouette sweeps and nothing else. The shadow layer now
  carries its own **`BuildingShadowTag`** and is rebuilt only when the height mode changes:
  it is the most expensive thing the building layers build, and it does not depend on the
  roof-clutter zoom bucket.
- **Map seed** (`map/seed.rs`) — one Park–Miller LCG (`Lcg`) and one point hash
  (`seed_from_point`) shared by everything the map *layers* scatter: crowns, roof clutter,
  the roof material, parked cars, standing wagons. **The seed is the object's own reference
  point** — the
  first vertex of a footprint, the first point of a street — never its index in the extract,
  so a zoom rebuild, a height-mode switch and a restart move nothing. The parse stage
  (doors, tree planting) keeps its own point-seeded `rng::lcg_seeded_by`.
- **Arclength walk** (`map/along.rs`) — `arclengths` + `place_on_path`, the one walk along a
  polyline's **whole** length, shared by every layer that places objects along linear
  geometry (parked cars, standing wagons) the way `map/seed.rs` shares the RNG. The defect
  it exists against is a per-link `points.windows(2)` walk: a city polyline's link is
  routinely shorter than two end margins, so such a walk drops the link whole, resets the
  step at every vertex and keeps a margin clear of every interior bend. Curvature stays the
  caller's problem — checked by **world** distance to the last object placed, never by the
  arc coordinate.
- **Entrances** — real `entrance=*` nodes are attached to building outlines by exact vertex
  lookup; coverage is thin everywhere, so `map/osm/entrances/` **generates** doors for the
  ~98 % of buildings without one. Doors face the street, the count follows building
  *length* at a measured pitch (`ENTRANCE_SPACING` 25 m, floor `ENTRANCE_MIN_SPACING` 12 m),
  walls a neighbour stands against get none, and the result is deterministic per building
  (LCG seeded by its first vertex). **Real doors always win.** The `doors` debug toggle
  draws them.
- **Trees** (`map/osm/planting.rs`) — planted **only inside Wood polygons** plus standalone
  surveyed trees and `tree_row` avenues; deterministic LCGs seeded by geometry. **Planting
  runs once at the density ceiling**; the density slider shows a monotone *prefix*
  (`tree_appears_at`), never a replant. The ceiling (`TREE_DENSITY_MAX` 6.5×) is derived
  from `TREE_MIN_SPACING` (6 m) saturation, not chosen. Health check — the `osm parse: N
  trees planted of M asked …` log line. **Crown geometry** is all in `CrownParams`
  (`map/trees/crown.rs`), built by `crown_variant`; **the city is drawn with
  `CrownParams::default()`**, whose `seed` picks the **crown set** (the city: **set 5**) —
  a whole `TREE_VARIANTS` of silhouettes at once, since **a single variant cannot be
  re-rolled**. Every crown side by side, knobs live: `cargo run --example tree_gallery`.
- **Standing wagons** (`map/wagons.rs`) — a station throat with empty rails reads as a
  diagram; half the area of a real one is taken by standing stock. Same trick as the
  parked cars, aimed at where they stand: wagons go **only on service track**
  (`RailLine::service`, from `service=siding|yard|spur` — `crossover` is a link between
  running lines and nobody parks on it), never on the running line, where a train is
  either moving or absent. They stand in **rakes** — several coupled 13.9 × 3.1 m cars
  with `COUPLED_GAP` 0.9 m between them, then an empty stretch of 12–90 m; an even row at
  a fixed pitch would read as a fence. The rakes are stepped along the **whole track's**
  arclength (**Arclength walk** above), so the end margin is kept clear of the track's ends
  — where the switch is — and not of every bend in it. A 3.8 m body throws a long shadow by the same
  `shadow_length_scale()` as everything else. Decoration only, like the cars, but with no
  style knobs of its own: the layer comes off by zoom alone. They have a zoom bucket of
  their own (`WAGON_MAX_ZOOM` 2.0 — a wagon is three times a car's length, and
  2.0 m/px leaves it the same ~7 screen pixels at which a car is already dropped, so the
  yards survive 2.5× further out than the rows) at `Z_WAGON` 2.65, above the steel and
  below the cars. **No
  `QUERY_VERSION` bump was needed**: `out geom` already carries every tag of the element,
  so `service` was in the cache all along.
- **Parked cars** (`map/cars.rs`) — a row of cars along every **carriageway**: the same
  `roads::is_carriageway` that decides where a sidewalk and lane markings go (so a
  `residential` street at 8 m parks and a `service` drive at 5 m does not), minus bridges
  and roundabouts. The pitch is walked along the **whole street's arclength** (**Arclength
  walk** above), not segment
  by segment, and the row **breaks at the junctions the lane markings already know**
  (`junctions::marking_breaks`, plus a 5 m clearance) rather than at the ends of an OSM way.
  A **one-way** carriageway gets a single row, on its right-hand kerb — which is what stops
  the two halves of a divided avenue from parking a column down their median, and is why
  `oneway=-1` is now normalized at parse by reversing the way.
  4.4 × 1.8 m bodies at a 6 m pitch,
  45 % of the places taken so the row comes out ragged, half a metre in from the kerb, in a
  ten-slot palette in the shares a photo of a Russian city shows — white / silver / grey two
  fifths, black a fifth, the rest coloured. Each casts its own shadow, by the same `shadow_length_scale()` the
  buildings use. **Decoration only** — cars are in no navmesh and no simulation, and pawns
  walk through them, deliberately: a parked row along every street would eat the pavements
  the whole crowd walks on. One merged blended mesh at `Z_CAR` (2.7), seeded per street, and
  a zoom bucket of its own (`CarZoomBucket`, `CAR_MAX_ZOOM` 0.8 m/px) drops the layer
  entirely when a car stops being worth six pixels. Tula: 22 022 cars, 176 k verts, 5.4 ms
  to build (5665 / 45 k while only the avenues parked). Every street shape the row broke on,
  side by side: `cargo run --example car_gallery`.
- **Footprint bands** (`map/footprint.rs`) — the strips linear geometry occupies on the
  ground, as **(centerline, width, role)** values (`deck_band` / `curb_bands` /
  `passage_band` / `channel_band` / `wall.band()`) plus the width policy. One construction,
  three consumers: the grid fill rasterizes, the mesh build outlines, the renderer draws its
  own smoothed copy. **A drawn band and a blocking band match by construction, not by
  discipline.**
- **Merged meshes** (`map/meshing.rs`, `map/spawn.rs`, `map/roads.rs`, `map/rail.rs`,
  `map/tram.rs`, `map/buildings/`) —
  one merged `Mesh2d` per layer: earcut triangulation, per-vertex colors over one white
  `ColorMaterial` (facades, shadows, casings, rails, walls), the **surface material** below
  (everything that is ground) or the **roof material** above (every layer that carries a
  roof — in 2.5D that is the walls' layer too); ~7000 buildings cost a handful of entities. Trees stay
  individual entities; tree and building **shadows** are each one merged mesh. **Ribbon**
  (`push_ribbon`) — constant-width band along a polyline with join/cap knobs. **Junction
  geometry is not computed** — overlapping `Round` caps in one opaque layer are what makes
  them look joined; **keep the road layer opaque, and its colour a function of world
  position only** (a flat colour or the surface shader, never a per-way tint). What *is*
  computed are **junction nodes** (`map/roads/junctions.rs`): a node shared by two or more
  carriageways, found by coordinate match on a 5 cm grid — Overpass gives no node ids, but
  a shared node projects to the same point on every way. They feed the markings only.
- **Surface material** (`map/surface.rs`, `assets/shaders/surface.wgsl`) — the ground,
  the area layers, water and the road fills are drawn by **`SurfaceMaterial`** instead of
  `ColorMaterial`: the vertex colour stays the base, the shader multiplies in procedural
  noise by **world position** (large mottle with a warm/cool tint shift, fine grain, grass
  speckle, drifting ripple on water) — no textures, no assets, and identical in any two
  overlapping ribbons. **Every octave fades by pixel size** (`fwidth`), so nothing shimmers
  when zoomed out. One material per **`SurfaceKind`** (`SurfaceMaterials`, built once at
  startup); **`SurfaceStyle::texture`** (panel *Surfaces*, persisted) scales all amplitudes,
  0 = the old flat fills, and retunes uniforms without rebuilding a mesh. A mesh for it is
  built with **`MeshBuilder::with_surface_coords`** — the **`Ribbon` attribute**
  `[across, to-break, half width, markings code]` in metres (*to-break* = signed distance
  to the nearest **marking break**, negative inside a gap; code = `lanes·2 + oneway`, 0 =
  none), zeros on polygons. This shader grain is what the map has instead of a **ground
  grain sprite** — a map-sized tiled noise sprite, proposed and then dropped in the merge
  that brought the building look; there is no `map/grain.rs`, and none is wanted.
- **Rims** (`map/spawn.rs::push_area`, `MeshBuilder::push_inset_band`) — every area
  polygon carries a gradient band along its contour, holes included: water a lighter
  **shore** (3 m), park / grass / wood / sand an edge a few percent darker (2–3 m). Same
  mesh as the fill, pushed after it (opaque 2D depth is `GreaterEqual`, so later wins —
  no z-slot). **Width is clamped to 0.6 × area / perimeter** of the outer ring, so a thin
  median strip never bleeds its rim onto the road.
- **Sidewalks & markings** (`map/roads.rs`) — a **carriageway** (`Street`, ≥ 8 m, not a
  passage; bridges included) is asphalt grey and gets a light **sidewalk band** at
  `Z_SIDEWALK` under every road ribbon (a crossing street's fill covers it, like a
  casing), width `sidewalk_width` (22 %, 1.2–3 m per side) — **never a bridge deck**,
  which leaves for its own layers before the band is pushed and has its curb instead.
  A carriageway also gets white **lane markings drawn by the surface shader** from the
  `Ribbon` coordinates (a bridge keeps those): a line on every lane boundary
  (`lane_count`: the `lanes` tag, else by width — two-way 8/10 m → 2, 12/16 m →
  4; one-way 8 m → 1, i.e. none; a roundabout always 1), dashed, the axis of a two-way road
  with 4+ lanes solid; anti-aliased, never thinner than ~1.3 px, gone when a lane is under
  ~10 px on screen. **Marking breaks**: at every junction node each carriageway's lines
  stop `half the widest other road + 1 m` short of the node — the through road gets a gap,
  the side street ends before the carriageway edge; a way end shared with exactly one
  other way end is a **continuation** (the line runs through the seam), any other way end
  a dead end. Wider fills are pushed after narrower ones, so a junction shows the main
  road's gap rather than the side street's stub. Both are `RoadStyle` knobs, on by default.
- **Style resources** — each is BRP-writable, persisted, and a change rebuilds only its own
  layers from the unchanged `MapData`: **RoadStyle** (join / smoothing / casing /
  sidewalks / markings — smoothing works on a *copy*, since `RoadLine::points`/`width` are
  load-bearing for navmesh, arches, planting and entrances), **BuildingHeightMode**,
  **TreeStyle**, **TreeRowStyle**, **ConiferNoiseStyle**, **SurfaceStyle** and
  **RoofStyle** (the last two: uniforms only, no rebuild). **`CrownParams` is deliberately not one of them** — a plain struct, no BRP,
  no prefs; only the `tree_gallery` example varies it. **Bridge / rail / tram layers** have
  their own z-slots and primitives (`push_dashes`, `push_ticks`, `push_rails`). Rail and
  tram answer to **no style resource** for their *geometry* — it is a function of the camera
  zoom (a **zoom bucket** each — `ZoomBucket<T>` over the layer's own LOD table,
  `map/zoom.rs`; seeded from the camera on world entry, then recomputed every frame), so a
  smoothing knob that moved the centerline would slide the track against its own ballast.
  The tram's one resource is **TramStyle** — `visible` alone, **off** by default (the blue
  line lies on the carriageway and at city zoom reads as another street layer), the `Tram`
  row of the Roads section (the track runs on the carriageway, so it is read with the roads);
  a change goes through `rebuild_tram`, so toggling the tram never remeshes the roads.
  **CarStyle** sits in the same section for the same reason and with the same shape —
  `visible` (**on** by default) and `occupancy` (the share of parking places taken, 0.45),
  the `Cars` and `Occupancy` rows; a change goes through `rebuild_cars` alone.

## Navigation

Summary; the mechanism and the measurements — **navigation-deep skill** (polymesh in
`references/polymesh.md`, separation & slots in `references/crowd.md`).

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * grid_size.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** → buildings block → walls block → **building passages carve back through
  them**. Without bridges the Упа river bisects the map and no cross-river path exists.
- **Bridge curbs are impassable** — the same two bands the renderer draws; on dry spans they
  stop a pawn stepping off the deck sideways.
- **Linear waterways block, unlike rails** — water is crossed by bridge, not waded;
  **culverts do not block at all**. The health check after any change here is the
  **pruned-tile count** in the log: a jump of thousands means a watercourse severed a
  district.
- **A rasterized polyline is a 4-connected chain, by construction** — `set_polyline` walks
  the centerline tile by tile on top of the capsule test, because a thin slanted band
  otherwise degenerates into corner-touching tiles that northstar and `line_of_sight` slip
  through. Pinned by `tests/navigation.rs`.
- **Ordinary roads do not touch the navmesh** — the grid starts all-passable and the fill
  only subtracts; roads enter it solely through the `bridge`/`passage` carves. **Rails do
  not touch it either, deliberately** (pinned): blocking an unbroken cross-city line would
  let `prune_unreachable` amputate half the map.
- **Building passage** (арка) — `tunnel=building_passage` / `covered=…` sets
  `RoadLine::passage`; carved passable **last**, width capped by `PASSAGE_MAX_WIDTH`.
  Without it, arch-only courtyards get sealed by the prune.
- **prune_unreachable** — BFS flood from the portal; unreachable pockets become impassable,
  because an A* to an unreachable target floods the whole region (a 12 000 request backlog
  once "froze" the crowd).
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>`; async tasks read it off-thread. Filled and pruned
  by the map-load thread while the loader is up.
- **PortalPos** (resource) — the actual portal position; `PORTAL_POS` is only a hint,
  `snap_portal_position` spirals to the nearest tile with clearance, between fill and prune.
  The spiral is **capped at `PORTAL_SEARCH_METERS`** (400 m, `settings.rs`); past the cap
  the load thread warns and keeps the raw hint.
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable: A* / Dijkstra /
  Fringe / BFS / **HPA*** (28× cheaper than flat A* at ~10 % longer paths) / Theta*.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid`, built
  lazily (~12 s) **only when a northstar algorithm is selected**; until it lands the
  dispatcher falls back to flat A*.
- **Backend / Walkable** (`navigation/backend.rs`) — the active backend as one cheap-clone
  `Send` snapshot, and **the resource the whole simulation reads** (`Res<Backend>`). The two
  modes differ not in type but in **who writes it**: live re-takes it every frame, under
  determinism it is frozen for the run. **It has no `Default` on purpose**, so **every
  system taking `Res<Backend>` must sit in a `SimPipeline` set**. `walkable()` is the
  passability view: `allows`/`nearest_free_point` are backend-strict, `sift_target` /
  `line_of_sight` / `coast_allows` stay deliberately grid-only. **Invariant: outside
  `navigation/` and `ui/`, the names `PolymeshBuild` / `PolymeshDebug` /
  `PathfindingAlgorithm` do not appear** — the door for "run this world on the flat grid" is
  **`navigation::use_flat_grid(&mut World)`**.
- **NavMode** (`navigation/mode.rs`) — which backend is active **right now**, as one value:
  `Grid(Flat | HierarchyPending{wanted} | Hierarchy(g))` and `Mesh(Pending | Ready(b))`,
  computed in exactly one place (`Pathfinder::mode`). A *value*, not a resource, and taken
  fresh by each consumer — the loader gate must see the live situation even in a
  deterministic run, where `Backend` is frozen.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) — requests become
  async tasks with **visibility gating** (peaceful wanderers off-screen or at zoom ≥
  `WANDER_DISPATCH_MAX_ZOOM` wait; **`UrgentPath` always dispatches**) and **priority**
  (urgent first, nearest-to-camera, cap `MAX_PATHFINDING_IN_FLIGHT` 1024).
- **UrgentPath** (`movement/components.rs`) — "this pawn may not wait for the camera". The
  species own it: a demon and the test walker carry it always, a human only while panicking;
  `strip_movement` takes it off a corpse. **Movement asks `Has<UrgentPath>` and names no
  species at all.**
- **BodyScale** (`movement/components.rs`) — how many times this pawn's body is bigger than
  a human's (`HUMAN` 1.0, `DEMON` the `DEMON_RADIUS_RATIO` 2×), a **ratio** because the
  radius itself is a live slider (`HumanStyle::body_radius`). Required by `Movable`, so
  every movable pawn has one; the demon writes `BodyScale::DEMON` at spawn, everyone else
  takes the human default. `move_moving_entities` reads the rest distance off it
  (`BodyScale::rest`) instead of asking `Has<Human>` — separation still derives its own
  radius from `Has<Demon>`, both from the same ratio.
- **Repath on the move** — `to_pathfinding` keeps the current path; a pawn walks the old one
  while the new is computed. `MovableStateMovingTag` means "has a path **or is coasting**".
  **Coasting** — a pawn whose path ran out mid-repath keeps walking `last_direction` over
  passable tiles; on reply, up to `REPATH_TRIM_LIMIT` leading waypoints are trimmed.
- **Rescue** (`movement::rescue_from_impassable`) — a pawn on an impassable tile is moved to
  the nearest passable one. **Trigger is a failed search**, not a clock; what counts as free
  is the *active backend*. A full pass runs only after every completed polymesh build, and
  it runs **ahead of that frame's target picking and dispatch** (`movement/mod.rs`).
- **find_passable_tile_near** — target tile or its 8 neighbors only; callers tolerate `None`.
- **Poly navmesh** (`navigation/polymesh/`) — a polygonal polyanya mesh from the same vector
  sources the grid rasterizes, ring **holes subtracted** as the grid subtracts them; **the
  default pathfinding backend**, the grid serving as fallback while it builds (~5–20 s,
  async, cancellable). polyanya is **vendored** (`vendor/polyanya`, edits marked `QWE:`);
  `bounded_path` is the only door to it, an exhausted budget **panics** by design, and so
  does a task older than `PATHFINDING_TASK_HANG_SECS`.
- **Chunk graph** (`polymesh/stitch.rs::stitch_chunks`, `ChunkGraph`) — the level-1 graph of
  the chunked poly navmesh: a **node is a connected component of one chunk**, an edge joins
  components of two *neighbouring* chunks that share a seam **segment**. `find_path_polymesh`
  A*s over it and hands polyanya the corridor of chunks. It is also **the reachability
  answer** — polyanya's island check is off whenever there is more than one layer, so
  `astar → None` means "unreachable"; on a flat mesh the baked islands answer instead.
- **Polygonal routing** (`find_path_polymesh`) — paths are **world-space polylines**
  (`VecDeque<Vec2>`, start point included); **the goal stays a tile** (identity for
  stale-answer filtering and arrival). A missed goal is `PathfindingError`, not a fallback —
  watch `answers: N/frame, X % failed` on the speed panel. Endpoint tolerance is 0.75 m
  (`POLYMESH_SEARCH_DELTA · (POLYMESH_SEARCH_STEPS − 1)`), which also sets the agent-radius slider ceiling
  (0.6 m). **A start that had to be snapped stays in the polyline as its own second point**,
  so every segment after that snap hop is one the funnel or `smoothed` vouched for. Coasting
  and lunge `line_of_sight` stay grid tests.
- **tiny_city / parity tests** (`map/osm/fixture.rs`, `navigation/parity_tests.rs`) — the
  shared `MapData` fixture from which **both fills** are built and must agree probe by probe:
  the executable form of "one rule for both fills". Run after touching either fill; **new
  fill rules add a zone here, not a hand-built `MapData`.**
- **pathfinding_bench** (`examples/bench/pathfinding_bench.rs`) — offline comparison of all
  six algorithms over a seeded wander-shaped task list. Run it after touching `successors`,
  costs, or the navmesh fill.

## Determinism

Summary; the mechanism — **determinism skill** (seed derivation, the decision stream, the
`SimPipeline` sets, the deterministic dispatcher, the replay yards and what they pin).

- **World seed** (`rng.rs::WorldSeed`, persisted, panel row *Seed*) — the one number every
  simulation draw descends from. It governs the **simulation**, not the map: trees and
  entrances are seeded by their own coordinates and are reproducible without it. Capped at
  `i64::MAX` (`MAX_SEED`).
- **Seed derivation** — `seed_for(world_seed, domain, key)`, two rounds of splitmix64.
  `RngDomain: Population | Human | Demon`. **Nothing stores live RNG state**, so a restart
  has no RNG to reset — every stream is re-derived.
- **Placement stream** (`rng.rs::stream`, `RngDomain::Population`) — the one shared
  generator, held by `spawn_population` across every spawn. Legal precisely because its
  consumer is a fixed `0..count` loop rather than a query traversal, and everything
  personal (colour, `Pace`, heading) still comes from the pawn's own decision stream.
- **Decision stream** (`rng.rs::WanderIndex::next`, humans *and* demons) — a `SimRng` is
  built **per decision** and dies with it, seeded from `(PawnId, decision number)`: the
  pawn's observable identity plus which choice this is, never the history of a stream. So
  draws do not depend on query iteration order, on neighbours, or on how many draws the
  previous decision consumed. **Position is deliberately not an input** — it would close
  every pawn's trajectory into a cycle. It also advances on **transitions**, not only on
  ladder rungs — a kill costs the demon one decision number, rolled for the devour pause in
  the kill observer (the sites are listed in the determinism skill).
- **Species** (`rng.rs`, component) — the other half of a pawn's personal number. `PawnId` is
  unique only *within* a species, so every mixed ordering (`movement::order::pawn_key`) puts
  species first. **Variant order is part of the replay contract** — `Demon` is declared
  first.
- **PawnId** (`rng.rs`) — a pawn's spawn ordinal within its species and run. Used wherever a
  stable "personal number" is needed: the RNG seed key, the flee-fan angle, the separation
  axis, the dispatcher tiebreak, the spatial-grid tiebreak. **Never `Entity`** — entity
  indices are recycled in a different order after a restart.
- **SimTick** (`determinism.rs`) — the step counter, incremented at the head of the
  `FixedUpdate` chain. **The unit of replay**: world state is a function of `(seed,
  settings, SimTick)`. Not `SimClock`, which counts virtual seconds and loses whatever
  `max_delta` discarded. **Compare states by tick, never by wall clock.**
- **Deterministic** (`determinism.rs::Determinism`, panel toggle) — gates *scheduling*, not
  the dice. On: *human* target picking moves to `FixedUpdate` (the demons' already runs there
  in both modes), answers land on a fixed tick, the dispatcher stops looking at the camera,
  the backend is frozen, separation is off. A run is
  deterministic or not from tick 0, so flipping it (like changing the seed) orders a restart
  via `RestartPending`, `to_portal: true`.
- **SimPipeline** (`determinism/mod.rs`) — the toggle in the schedule: **three** system sets
  — `Live`, `Deterministic`, `BothModes` — gated once for every schedule they appear in. A
  system declares its branch with `.in_set(..)` and **never reads the mode**. **The sets
  also carry the world gate** (`in_world`), so "no set" reads as "lives outside the world",
  not "both modes".
- **Retire tick** (`RetireAt`, `PATHFINDING_RETIRE_TICKS = 8`) — the deadline is stamped **at
  dispatch, not when the request was filed**: a request that leaves the queue on tick `D` is
  applied on exactly `D + 8`, waiting on the search if it has not finished. The queue wait
  before `D` is on top of that (a long queue is normal here — see *Dispatch rate*), so what
  replays identically is not one fixed end-to-end latency but the deadline itself: `D` and
  `D + 8` both follow from integer state. That wait removes "when did the OS get around to
  it" from the simulation. **The constant must not scale with `SimSpeed`.**
- **Dispatch rate** (`PATHFINDING_WANDER_UNITS_PER_TICK` 128 / `_URGENT_` 64) — how much
  leaves the queue each tick, measured in *predicted search cost* (an integer), not in
  requests. **Never reuse `MAX_*_IN_FLIGHT` here** — those cap concurrent searches behind
  the visibility gate. **A long queue is the normal state of this mode**; at 30× it settles
  around 2–5×.
- **RequestedAt** — the tick a request was filed; the deterministic dispatcher's FIFO key is
  `(requested_at, species, pawn_id)`, all integers. **The camera does not appear in it at
  all.**
- **Frozen backend** — in this mode `Backend` is written once, on **WorldStarted**, and
  never refreshed; warmup waits for the wanted backend instead (~11–14 s on first entry into
  a city on HPA, deliberately; restarts do not pay it). **No pawn warmup in this mode.**
- **NeedsWanderTarget** (`movement/components.rs`) — marker held exactly on `Idle` and
  `PathfindingError`; without it each `FixedUpdate` run would scan all 17 000 wanderers.
- **Replay check** (`determinism/replay.rs`, `tests/determinism.rs`,
  `examples/acceptance/determinism_replay.rs`) — three claims: the same seed replays tick for
  tick, a ragged frame rate does not change the run, a different seed does. Two conditions
  make it bite, both learned the hard way: **the world must actually move** and **the scene
  must be crowded**. `a_restart_replays_the_run` runs its second half in the *same* `App` —
  that is what catches state outliving the reset.
- **Frame rate does not matter.** `Time<Fixed>`'s step is constant regardless of fps and of
  `SimSpeed`; a slow machine replays the same run more slowly.
- **The replay contract** — 1:1 holds only while `DemonStyle` / `HumanStyle` /
  `SeparationStyle` / the algorithm / the navtile size are left alone mid-run. Sliders are
  simulation input. Not enforced by code. **Not claimed**: float reproducibility across
  machines, or replaying a run made with the toggle *off*.

## Simulation

Summary; species behaviour — **species-behavior skill**; the crowd (separation, slots) —
**navigation-deep skill**, `references/crowd.md`.

- **SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`): `SpatialRebuild →
  DemonBehavior → HumanBehavior`. **Demons act before humans so a kill lands before
  `escape`** — a human is never counted both killed and escaped in one tick.
- **SimPosition / PreviousSimPosition** — simulation-space positions; `Transform` is
  interpolated between them in `RunFixedMainLoop`. Systems mutate `SimPosition`, **never
  `Transform.translation.xy`**. `snapshot_previous_sim_positions` runs **before**
  `SimSet::SpatialRebuild` (as does the demon spawner) and `move_moving_entities` **after**
  `SimSet::HumanBehavior`, because behavior may move `SimPosition` itself (the demon lunge).
- **Movable** — `{speed, path: VecDeque<Vec2>, state, last_direction}` with `MovableState:
  Idle | Pathfinding(goal) | Moving(goal) | PathfindingError(goal)`. **Waypoints are in
  world metres, the goal is a tile** (*Polygonal routing*); `last_direction` is the heading
  of the last step, i.e. the coasting vector (*Repath on the move*). `to_pathfinding`
  queues the search and keeps the current path; **`to_idle` is the only transition that
  stops movement** — and it stops it whole: path, `MovableStateMovingTag`, and a
  `PathfindingRequest` that has not been dispatched yet, with its `RequestedAt`.
  **`to_pathfinding` re-files a queued request instead of overwriting it** (remove +
  insert, with `RequestedAt`): both consumers filter on `Added<PathfindingRequest>`,
  and an overwrite arms only `Changed`.
- **SpatialGrid<T>** — uniform grid per marker type (`Demon`, `Human`), 60 m cells (≥ the
  largest search radius, so a radius query is a 3×3 cell walk). Cells hold **entities
  only** — positions are read live through the `pos_of` closure. **A tie in distance is
  broken by `PawnId`, not by traversal order** (`order_of`, called only on exact equality).
  **The human grid is incremental** (observers + `SpatialGrid::moved` across cell
  boundaries), **the demon grid is rebuilt** each tick.
- **Decision ladder** (`human/decide.rs`, `demon/decide.rs`) — a species' rules live in a
  pure `decide(&…Sense, …) -> …Action`: plain values in, one enum variant out, no
  `Commands`, no queries, tested without an `App`. `behavior.rs` only applies the answer, so
  the *order of the rungs* is readable in one place. Deliberately outside: **expensive
  senses** (asked lazily, at most once), **anything touching the world** (its *terms* still
  come out of `decide`), and **the RNG rolls** (the decision stream must advance on exactly
  the ticks it did before).
- **Wander skeleton** (`movement/wander.rs`) — the *order* of one target-picking step,
  shared by both species: `ready_to_pick` → the species' policy → `point_in_cone` /
  `clamp_to_map` → `request_wander_path` → `heading_towards`. Only **where** a pawn wants to
  go is per-species. It deliberately does **not** open the `SimRng`.
- **PopulationSize** (`human/components.rs`, resource) — how many humans
  `spawn_population` settles. **No knob, and not a `settings.rs` value**: its `Default`
  *is* `HUMAN_COUNT` (20 000) and the game never changes it; the resource exists so a
  headless scene can run a small crowd. Read at two sites that must stay in step —
  `human::spawn_humans` under `WorldInitSet::Spawn` and `restart::on_restart`, so a
  restart respawns the same number. Today the only non-default user is the replay yard
  (`determinism::replay::replay_app`'s `population` argument; `tests/determinism.rs`
  runs 64). **It is the right-hand side of the telemetry invariant** — see Telemetry.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s, rolled per
  arrival and drawn by only `HUMAN_WANDER_PAUSE_SHARE` 20 % of them — the rest pick the
  next target the same frame; 80 % a building errand anywhere in the city — the real
  pathfinding load — and 20 % a 20–40 m stroll) ⇄ **Flee** (a demon within
  `HUMAN_PANIC_RADIUS` 60 m; the first repath on the panic tick itself, then every
  0.7–1.2 s, stepping 40–60 m away), calm-down at ×1.5
  radius hysteresis. The Wander → Flee
  check is **inverted** — demons collect neighbours from the human grid, so its cost tracks
  the crowd near demons, not the city population. **Flee fan** — a non-chased fleeing human
  rotates its away-vector by a deterministic per-entity angle (±0.6 rad) so crowds spread;
  chased humans flee straight. **Escape** — a fleeing human within `ESCAPE_MARGIN` of the
  border despawns, `telemetry.escaped += 1`.
- **WanderHeading** — the direction a human is walking, kept between walks; every next
  target is picked inside a `WANDER_CONE` (±60°) around it. Without it pawns wobbled in
  place. `flee` rewrites it to the away-vector on every repath.
- **PanicRecoil** — a unit vector *toward* the demon, written on **every flee repath** and
  **never queried live** (`pick_wander_targets` must stay off the demon grid). While it is
  on, the next target must be an errand outside `RECOIL_CONE` (±45°) and farther than
  `RECOIL_MIN_ERRAND` (90 m); nothing acceptable → re-roll next frame, **never a stroll** —
  except on a map with no buildings at all, where the fallback stroll is filtered by the
  cone alone, with no distance floor.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near* stroll,
  never an errand. Measured: errands first routed the on-screen pawns in 3.9 s, strolls
  first in 0.15 s. `PanicRecoil` overrides it.
- **Pace** — a human's personal speed multiplier, rolled once at spawn and stored
  **normalized** (−1…+1), applied to *both* bases through `Pace::speed`. Normalized storage
  is what lets the **Speed spread** slider widen the ordering the crowd already rolled
  instead of re-dealing it. Ceiling 35 % is derived: above it the fastest humans outrun the
  slowest demon setting.
- **CorpseTag** — a killed human: behavior/movement components removed, the body drawn
  as a lying human figure at `Z_CORPSE` (the **corpse look**, see Look: pose, heading and
  mirror by `Entity` bits — cosmetics, not run state; **blood** under the chest, see
  Look), not in the human spatial grid. The transition is **`human::to_corpse`**, one
  entry point; the kill observer in `demon/` only reports that it happened. It calls
  **`movement::strip_movement`**, so `Movable`'s `#[require]` stays the single record of
  what a movable entity drags along.
- **Demon** states (`demon/behavior.rs`): **Wander** (a point in the `DEMON_WANDER_CONE`
  (1.3 rad half-angle) around the away-from-portal vector, `DEMON_WANDER_RANGE` 40–120 m;
  no `WanderPause` analogue and no stored `WanderHeading` — the next target is picked the
  same frame the demon goes idle) → **Chase** (nearest human within `DEMON_AGGRO_RADIUS`
  45 m with a free claim slot; give-up at ×1.5 radius hysteresis, 67.5 m) → **Devour** →
  Wander. **Chase claims** —
  **max 2 chasers per target**
  (`ChaseClaims`, `demon/claims.rs`), a value rebuilt each tick; there is no standing claim
  between ticks, only **GaveUp** releases a slot, and a switch *transfers* one. Repath
  throttle 0.4 s (`DEMON_CHASE_REPATH`), and on that tick the demon may **switch** target — **a rung of the ladder,
  not a tail after it**. **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with
  `line_of_sight`, the demon drops its path and steps `SimPosition` straight at the target;
  without it a chase never converts. Kill at `KILL_DISTANCE` triggers
  `DemonCaughtHumanEvent`. **Devour** — pause 1.5–2 s with a sine pulse ×1 → ×1.5; the pause
  is rolled in the kill observer from the demon's **decision stream**, so a kill advances its
  `WanderIndex`.
- **DEMON_SPEED** — one base for every state, `HUMAN_FLEE_SPEED × 1.35`. **Do not
  reintroduce per-state demon speeds**: the only multipliers are the two user ones,
  `DemonStyle::speed` and `DemonStyle::lunge`.
- **DemonSpawner** — initial burst at the portal rim, then one demon per interval up to the
  cap; cap and interval live in **`DemonStyle`**, `DEMON_CAP` / `DEMON_SPAWN_INTERVAL` are
  only its `Default`. Lowering the cap never despawns demons already out. **The spawner runs
  only in `PlayPhase::Live`, and that is an invariant**: it hands out `PawnId`s from a
  counter `WorldStarted` resets, so a burst fired before the announcement deals the same
  numbers twice. Matching precondition: **no demon may be alive when a run starts**. It runs
  **before `SimSet::SpatialRebuild`**, so a demon enters the demon grid and acts on the tick
  it is born on.
- **Separation** (`movement/separation/`, Nav tab, persisted) — soft pairwise
  anti-overlap, **on-screen only, cosmetic by charter**: pawns keep their body radii
  apart (a resting human pair at 1.8 m, against a 1.0 m `HUMAN_SIZE`). The radius is a
  knob — **`HumanStyle::body_radius`**, and `HUMAN_BODY_RADIUS` 0.9 m is only its
  `Default`; the demon's is never a separate knob, always `2 ×` it
  (`separation::demon_radius`, `DEMON_RADIUS_RATIO`, default `DEMON_BODY_RADIUS` 1.8 m).
  Runs **only on the
  polymesh backend and never under determinism** (grid waypoints re-collapse any push), once
  per rendered frame, below `SEPARATION_MAX_ZOOM`; the run is **dt-invariant** — overlap
  decays as `exp(-rate · t)` of virtual time under a `max_speed · dt` ceiling
  (`max_step` is only a teleport guard), so one frame at 30x equals thirty frames
  at 1x. Lunging demons are exempt; the one
  deliberate breach of "cosmetic" is `SeparationHolds` + rest-distance arrival forgiveness.
  Measured on demand by `examples/demos/crowd_demo/`.
- **Destination slot** (`movement/destination.rs`) — the reservation that stops two pawns
  from being aimed at the same point: a `k × k` navtile block per claimed goal, goal
  strictly the block's **centre** tile; a taken slot ring-searches outward. Claims are
  released on next target selection, despawn, or corpse strip — **not on arrival** (a
  standing pawn *is* the occupancy). **Chase and flee are excluded** by design. Runs in
  **both** modes — it is simulation, not cosmetics.
- **Telemetry** — `{killed, escaped}`, BRP-readable; `killed` is the **Souls reaped** HUD
  counter (`ui/stats.rs`), *not* a row of the Sim tab's **World** section — that section
  holds the seed and the determinism row. Invariant (check paused):
  `killed + escaped + alive == PopulationSize` — the number the spawn actually read, not
  the constant: in the game that is the default `HUMAN_COUNT`, in a replay run whatever
  `replay_app` was given. At high sim speed BRP reads are skewed — pause before asserting.

## Look

How pawns are drawn over the map — the map's own rendering is the **osm-map skill**; the
pawn half of the mechanism, `silhouette/` and `portal.rs` included, is the
**species-behavior skill**, and the camera's bloom the **ui-panels skill**. There are no
art assets and no artist: every shape here is a formula, every colour a constant beside
its draw call.

- **Silhouette** (`silhouette/`) — a pawn's on-map shape: one procedural **atlas**
  (`Silhouettes` resource, `Glyph::COUNT` cells — `Disc`, `Ember`, `Halo`, ten blood
  pools and ten spatters of `silhouette/blood.rs`, and the four corpse figures of
  `silhouette/figure.rs`; the three families carry a variant number, and `Glyph::cell()`
  is the atlas index) plus the `Silhouette { body, min_px }`
  component — the body in metres and a **screen-size floor** in logical px
  (`HUMAN_MIN_PX` 2, `DEMON_MIN_PX` 5), so at city zoom a human stays a grain and a demon
  a point instead of vanishing. Two invariants: **`Sprite::custom_size` belongs to this
  module** — spawn sets the body, the module's two LOD systems write the size — and in an
  app without a renderer (replay, tests) the resource stays `None` and pawns are plain
  squares, which is not an error (`HumanPlugin` / `DemonPlugin` / `PortalPlugin` only
  `init_resource` it, `SilhouettePlugin` in `main.rs` fills it). Rasterisation, the mip
  chain, the one-image batching argument and the two LOD passes — **species-behavior
  skill**.
- **Attire** (`human/components.rs`, palette in `human/look.rs`) — a human's own colour,
  the three spawn draws of its decision stream, cool and muted (hue 170–290°): **warm on
  the map means demons and panic**. Separate from `Sprite::color` because of the **panic
  tint** — `On<Add, HumanFleeTag>` paints the sprite `PANIC_COLOR` (amber, one for all,
  so the panic front reads as a spreading stain), `On<Remove, HumanFleeTag>` restores the
  attire. Transitions only, never per frame.
- **Corpse look** (`human/look.rs`, figures in `silhouette/figure.rs`) — a killed human
  is a **lying figure**, not an ellipse: head, torso and limbs as capsules on a skeleton
  in metres of `CORPSE_HEIGHT`, four poses (sprawled, prone, curled, crumpled), sixteen
  headings and a mirror, all from the `Entity` bits (`corpse_pose`). Limbs are drawn
  thicker than anatomy so the pose survives crowd zoom. The tint is the pawn's own
  attire **drained** (`corpse_tint`), so the dead keep their clothes; under the chest the
  **blood** below.
- **Blood** (`silhouette/blood.rs`, spawned by `human/look.rs`) — what a corpse leaves on
  the ground, as **two** children, because they are two events: a **pool** (`BloodPool`,
  lobes welded into one stain plus a rivulet) that **spreads** from `SPREAD_START` to full
  over `SPREAD_SECS` of sim time (`spread_blood` in `Update`; `BloodSpread` comes off when
  full, so the pass costs the last seconds' kills, not the map's corpses), and a
  **spatter** (`BloodSpatter`) — a cast-off fan of drops, small ones far and drawn out
  into commas, that lands whole and never grows, in a cell 1.8× wider. Ten glyphs of
  each — **the count costs nothing per frame**: cells are rasterised once at startup and
  the batch is still one texture — shape seeded from the variant number; which pair a body gets, plus its own spin,
  size and tint, comes from the `Entity` bits (`blood_look`). **Thickness sets both alpha
  and shade**: the sprite colour is the *thin film* and the glyph darkens the deep middle,
  so a small drop is thin because it is small. Every stain side by side on three grounds:
  `cargo run --example blood_gallery`.
- **Demon look** (`demon/look.rs`) — the `Ember` glyph in a five-shade crimson → orange
  ring (`demon_tint`) plus a **halo**: a child entity (`DemonHalo`, the `Halo` glyph,
  three bodies wide, z −0.01) that inherits the devour pulse and dies with its parent.
- **Portal** (`portal.rs`, `assets/shaders/portal.wgsl`) — a `Mesh2d` quad with a
  `PortalMaterial` (`Material2d`): a log-spiral vortex computed per pixel from
  `globals.time` — no spritesheet, no frames. Violet on purpose: red is demons, amber is
  panic, blue is water and tram. Its rim is **HDR** (> 1.0) so it blooms; under it a
  **portal stain** (the `Halo` glyph, dark violet, 2.6 portals wide) at `Z_PORTAL_STAIN`
  — above roads, below corpses.
- **Soul** (`human/soul.rs`) — a golden HDR spark (`SoulMote`) released by the kill
  observer at the victim's position, rising 6 m over 1.4 s of sim time and fading; it is
  the visible side of `Telemetry::killed`. Lives at `Z_SOUL` above every unit. Stepped
  and despawned in `FixedUpdate` (`rise_souls`, after `SimSet::HumanBehavior`) — a world
  entity may not be despawned from `Update`. Not the `Souls { earned, spent }` currency of
  `ROADMAP.md` — that is a separate concept, a resource the same kill observer will
  increment; the mote counts nothing and is only the visible side of the kill.
- **Bloom** (`post.rs`) — what makes every HDR colour above glow: the portal rim, the
  demon halos, the souls, and later spells. The setup and its threshold are under
  **Post-processing** in App lifecycle; the tuning — **ui-panels skill**.

## UI & debug

Summary; panel internals — **ui-panels skill**; the speed regulator — **sim-speed skill**.

- **UI input never reaches the world** — the panel sits over the map, so a click, drag or
  scroll that lands on it must not also drive the camera or anything in the world.
  `camera.rs::drag_pan` decides *in the press frame* whether the gesture belongs to the UI
  (`pointer_over_ui` over `HoverMap`) and holds that verdict until release; `zoom_to_cursor`
  runs under `not(hovering_ui)`. The rule and the idiom — **CLAUDE.md**.
- **Two layers.** **HUD**, always on screen: run counters (`ui/stats.rs`), telemetry +
  **Speed button** (`ui/speed.rs`), the **City** select bottom centre (`ui/city.rs`), hotkey
  help (`ui/hotkeys.rs`), the agent **BRP** badge (`ui/brp.rs`). And **one settings panel**
  with four tabs (`ui/shell.rs`): **Map**, **Nav**, **Sim**, **Debug**. `Tab` (or the `-`/`+`
  button, or a click on the open tab) collapses it to the tab strip; the open tab and the
  collapsed flag are a persisted settings group (`UiShellState`). **Section order inside a
  tab is `SectionSlot`'s declaration order** (`sort_sections`), because the sections are
  spawned by one system per section plugin.
- **Knob** (`ui/knob.rs`) — a panel row **bound to one field of one resource**, in two
  shapes: `spawn_knob` (slider) and `spawn_cycle_row` (button that cycles a value).
  `app.add_knobs::<R>()` registers the drag observer and the label/thumb sync **once per
  resource**, however many knobs it has. **Use it for any panel row driven by a resource**;
  the Nav tab's rows are the deliberate exception, since their text is computed
  from several resources at once.
- **Widgets & theme** (`ui/theme.rs`) — the controls are first-party **`bevy_feathers`**,
  themed by `create_dark_theme()` with colour overrides only. The plaques are **translucent**
  over the map and the text is **brighter** than feathers': the panel keeps its legibility
  with type, not with an opaque fill. `PanelWidgetsPlugin` installs `FeathersCorePlugin` —
  **not** the `FeathersPlugins` group, since `TabNavigationPlugin` would let Tab+Space both
  press a button and pause the sim. Everything that is not a widget is coloured by **design
  tokens**; **no hand-written UI colours are left**. A **value row rests transparent**;
  **"active" is `ButtonVariant::Primary`**. Every container node comes from `ui_node` /
  `ui_row` / `ui_column`, which carry the `ThemedText` marker — every container *between a
  font source and a label*, that is; a root is the source itself (`InheritableFont` requires
  the marker) and stays a bare `Node`.
- **Shared kits** — `ui/slider.rs` (the layer under the knob kit) and `ui/rows.rs`
  (`spawn_value_row`; a row whose click does nothing gets `bevy::ui::InteractionDisabled`).
  **Use these, don't hand-roll a panel row.**
- **Debug tab** (`ui/debug/`) — the grid / doors / movepath / noise overlay rows, the
  `Camera start` and `Navtile` cyclers (global settings, deliberately not under a backend
  section) and **`reset`** (`prefs::ResetSettings`). The navmesh overlay is **one merged
  mesh** (per-tile entities once cost 330 k); the noise overlay is one CPU-built texture
  sprite.
- **Camera start view** (`camera.rs`) — `CameraPositionMode` (`reset | save`, persisted):
  where the camera stands when the world comes up. **RR** (double R within 0.5 s) and
  `RestartEvent { to_portal: true }` go to the portal at `START_ZOOM` regardless of mode. A
  city switch always resets to the new portal.
- **Sim speed** (`sim_time.rs`) — Space pauses; `=`/`-` walk `SPEED_LADDER` (1-2-5-10-20-30);
  **`MAX_SIM_SPEED` 30× is a deliberate product cap**. **SimSpeed** `{requested, pipeline,
  affordable, effective, actual}` — a regulator throttles `effective` to what the machine
  carries, and `guard_frame_budget` hard-stops a frame's fixed loop at `SIM_FRAME_BUDGET_MS`,
  booking stripped ticks into **TickDebt**. **`actual` is the only honest reading.** Set speed
  over BRP with `res set SimSpeed .requested N`, never `brp speed`. The regulator's memory is
  reset by **WorldStarted**. **SimClock** — virtual seconds of the current world, zeroed by
  the same observer; compare states by `SimTick`, not by it.
- **Tunable resource** (`prefs.rs`) — a resource the user retunes from a panel, a hotkey or
  a BRP write. Two things are asked of one: **`app.track_pref::<T>()`** persists it (a
  `bevy::settings::SettingsGroup` in `settings.toml`), and the run condition
  **`retuned::<T>`** (`changed && !added`) gates whatever rebuilds on it. **Registration
  sits with the resource's own plugin**, next to its `init_resource`; `PrefsPlugin` is
  registered **last**. **`ResetSettings`** puts every group back to its `Default`, found
  through the type registry, **never a list**, so a new tunable is covered the day it is
  declared.
- **dev.rs** — `TakeScreenshotEvent` (BRP-triggerable) → `screenshot.png` (gitignored);
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
- **BRP** — `RemoteHttpPlugin` on port 15702; drive it via the `live-app` skill's `brp`
  script only.

## Naming conventions worth preserving

- **`*Tag`** — marker/state components (`HumanFleeTag`, `DemonDevourTag`, `CorpseTag`).
  **`*Plugin`** — one per feature module. **`on_*`** — event/observer handlers.
- **Пешки** — the user's word for humans/units in feedback; maps to `Human`.
- **Behavior module** — per-species state machine lives in `behavior.rs`
  (`demon/behavior.rs`, `human/behavior.rs`), separate from `systems.rs` (spawning,
  wander targets).
- **Hysteresis** — every enter/exit radius pair uses `RADIUS_HYSTERESIS = 1.5` on exit;
  keep new radii consistent with this pattern.
- **macOS occlusion throttling** — a fully covered window parks the main thread; fps and
  BRP timings are only meaningful with the window visible. Not a perf bug.

## Cross-references

- All tuning constants: `src/settings.rs` (sizes, speeds, radii, spawn rates, z-layers,
  geo anchor). Not there: a number that *is* a rule of a decision ladder rather than a knob
  over it stays beside its `decide.rs` — `MAX_CHASERS_PER_TARGET` and the ×1.5/×0.7 switch
  factors in `demon/decide.rs`, `FLEE_STEP`/`FLEE_SPREAD`/`ESCAPE_MARGIN` in
  `human/decide.rs`. A constant both species declare moves to `settings.rs`
  (`WANDER_MAP_MARGIN`). Same split in the polymesh: the world-scale metres are in
  `settings.rs` under the `POLYMESH_` prefix (agent radius, endpoint tolerance, map-edge
  margin, chunk sides), while the rules of the algorithm stay in `navigation/polymesh/` —
  `MAX_CHUNKS` (polyanya's layer-index width), the f32 tolerances (`SEAM_EPSILON`,
  `SEAM_QUANTUM`, `SIMPLIFY_EPSILON`, `WALK_*`), the search budget and `COST_SCALE`.
  Also not there: what a gizmo *looks like* and each visibility gate's own view margin —
  `MOVEPATH_COLOR`/`MOVEPATH_ARROW_TIP`/`MOVEPATH_VIEW_SCREENS` (`movement/systems.rs`),
  `DOOR_*` (`ui/debug/overlays.rs`), `VIEW_MARGIN` (`movement/mod.rs`) — they live beside
  the draw call or the gate, see `camera::Viewport::of`.
  Detail — **species-behavior** and **navigation-deep** skills.
- OSM pipeline: `src/map/osm/{overpass,download,parse,model}.rs`; rendering:
  `src/map/{meshing,spawn}.rs`. Detail — **osm-map skill** (its `references/` also carry
  the tag coverage audit and the crown-algorithm write-up).
- Navigation: `src/navigation/{navmesh,astar,northstar,polymesh}`; movement/interpolation:
  `src/movement/`. Detail — **navigation-deep skill** (crowd and slots in
  `references/crowd.md`).
- World bring-up, restart, city switch: `src/{loading,restart,city}.rs`,
  `src/map/osm/download.rs`. Detail — **world-lifecycle skill**.
- Determinism and replay: `src/rng.rs`, `src/determinism/`. Detail — **determinism skill**.
- Species behaviour: `src/human/`, `src/demon/`, `src/movement/wander.rs`, `src/spatial.rs`.
  Detail — **species-behavior skill**.
- Speed & regulator: `src/sim_time.rs`. Detail — **sim-speed skill**.
- UI: `src/ui/`, camera: `src/camera.rs`. Detail — **ui-panels skill**.
- Tests: `tests/navigation.rs` (synthetic navmesh + hand-built `MapData`),
  `tests/spatial.rs`, `tests/movement.rs`, `tests/determinism.rs`, unit tests inside
  `map/osm/*` and `map/meshing.rs`.
