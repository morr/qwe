# CONTEXT

Domain glossary for QWE. Use these terms verbatim in commit messages, hypotheses, test
names, and code identifiers. If a concept you need isn't here, that's a signal — either
you're inventing language the project doesn't use (reconsider) or the file has a real gap
(update it in the same change that introduces the concept).

## Project shape

**QWE** is a 2D real-time simulation prototype: a **demon invasion of the Tula city
center**. The map is generated from real OpenStreetMap data at first launch. 20 000
humans wander the streets; demons pour out of a portal, chase and devour them; humans
panic and flee off-map. Built on **Bevy 0.19 ECS** — one plugin per feature, registered
in `main.rs`.

## Coordinates & units

- World units are **meters**. Origin — **south-west corner** of the map, y grows north.
  All world coordinates are positive. `MAP_SIZE = 3000 × 2250` m.
- **Navtile** — navigation grid cell, `NAVTILE_SIZE = 2` m. `GRID_SIZE = 1500 × 1125`
  tiles, hand-maintained as `MAP_SIZE / NAVTILE_SIZE` (they can silently desync — keep
  them in step). `grid.rs`: `world_to_tile` / `tile_center` / `tile_in_bounds`.
- **Geo anchor** — `GEO_CENTER_LAT/LON` (Tula, kremlin near frame center). Projection is
  local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW corner → (0,0),
  f64 math, `MAP_SIZE`-sized bbox derived from the center.
- **Z-layers** — constants in `settings.rs`: ground 0 → parks 0.5 → water 1 → alleys 1.5
  → roads 2 → corpses 3 → portal 4 → buildings 5 → units → trees 20. Units are y-sorted:
  `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR` (10 − y·0.002). **Invariant: the unit z
  range must stay above buildings (5) for any y ≤ MAP_SIZE.y** — a bigger map once sank
  northern units under roads.

## App lifecycle

- **AppState** (`loading.rs`) — `Loading → Playing`. `Loading` shows the loader screen
  (progress text, red error + **Retry** button on failure). All world spawning happens in
  `OnEnter(Playing)`.
- **PlayPhase** (sub-state of `Playing`) — `Warmup → Live`. During **Warmup** the world
  exists but `Time<Virtual>` is **paused** and the loader screen stays up reading
  "Routing pawns... N left"; `poll_warmup` counts pawns *inside the camera view* that
  still hold a `PathfindingRequest`/`PathfindingTask` and flips to `Live` when none are
  left (or after `WARMUP_TIMEOUT` = 10 s, logged as a warning). Reason: all 20 000 humans
  queue a path in the same frame, and without the hold the visible ones stood still for
  the first seconds. Typical warmup **~0.15 s** — see `HumanFirstWanderTag`.
  `Live` is what despawns the loader and reveals the game UI (`GameUiRoot`).
- **WorldInitSet** — ordering inside `OnEnter(Playing)`: `Navmesh → Spawn`. Navmesh must
  be filled before population spawns, or humans land in the river.
- **MapLoadJob / JobState** (`map/osm/download.rs`) — background `std::thread` that
  prepares everything not needing ECS: `Connecting{attempt} →
  Downloading{bytes,total,bytes_per_sec} → Parsing → BuildingNavmesh → Pruning →
  Done(LoadedWorld{map, portal}) | Failed(msg)`, polled via
  `Arc<Mutex<_>>` by `poll_job`, every state a line on the loader screen. `total` is
  `None` in practice — Overpass answers chunked and ureq strips `content-length` when it
  decompresses gzip — so the screen shows downloaded MB plus a rate (smoothed over
  `SPEED_WINDOW`, 250 ms) instead of a percentage. `bytes` counts *decompressed* JSON, so
  it matches the cache file size, not the wire. `Connecting` is mostly **Overpass
  computing the query**, not TCP — measured on Paris: 0.05 s connect, 0.23 s TLS, then
  62 s of server-side compute before the first byte — so the screen says "Waiting for
  Overpass", not "Connecting", and `poll_job` ticks seconds next to it off `Time<Real>`,
  keyed on `attempt` (the 1-based mirror index) so the count restarts on failover instead
  of looking frozen. The thread cannot tick it itself: it is blocked inside `send()` until
  the first byte. Minutes here are normal, not a hang; a mirror that is actually broken
  ends the wait with a 502/504 and the loop moves on. It writes the
  navmesh through the `ArcNavmesh` handle it is given and returns the snapped portal
  position. **Rule: heavy init belongs in this thread, not in `OnEnter(Playing)`** — no
  frame is drawn inside a schedule, so work there freezes the loader on its last message.
- **RestartEvent** (`restart.rs`, R key or BRP) — despawns humans/corpses/demons/walkers,
  resets `DemonSpawner` + `Telemetry`, respawns population. The navmesh persists — it is
  filled once per city.
- **City** (`city.rs`, resource, remembered by `prefs.rs`) — which city the map is built
  from: `Tula | NewYork | Paris | Berlin | London | Tokyo`. Each carries its **geo center** (bbox
  center of the Overpass extract), its **portal hint** and its **cache slug**; `MAP_SIZE`
  and therefore `GRID_SIZE` are shared, so switching city never resizes the navmesh.
  Panel — bottom centre (`ui/city.rs`), the current city's button is highlighted.
- **City switch = full world reload.** Writing `City` (button or BRP) sends the app back
  to `AppState::Loading`: leaving `Playing` despawns the scene, the load thread downloads
  / re-parses the new extract, refills the same navmesh (`fill_from_mapdata` resets it
  first), re-snaps the portal, and `OnEnter(Playing)` rebuilds map and population and
  resets the camera (`camera.rs::reset_camera_to_portal` — onto the new portal, back to
  `START_ZOOM`). `DemonSpawner`, `Telemetry`, `NorthstarGrid` and `WarmupProgress` are
  reset on the way. The switch is gated on `in_state(Playing)` — restarting a load on top
  of a running one would put two threads into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city.
  Every world entity must carry it; the list of spawn sites and the rule live in
  `CLAUDE.md` ("World entities"), and `loading.rs::warn_leftover_world_entities` warns on
  every entry into `Loading` if something survived.

## OSM map pipeline

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). Query covers: `building` (way+rel),
  `highway` (way), `natural=water` / `waterway=riverbank` (way+rel), `leisure=park|garden`,
  `landuse=recreation_ground|forest` + `natural=wood`, `landuse=grass|meadow` /
  `natural=grassland|meadow`, `natural=sand|beach`, `barrier=city_wall`. The bbox is
  `MAP_SIZE` around the selected `City`'s geo center.
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
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn:
  - **PolyArea** — polygon with holes; rings are open (no repeated last point).
    `AreaKind: Building | Kremlin | Water | Park | Wood | Grass | Sand`. **Park** is the
    light base fill; **Wood** (`natural=wood` / `landuse=forest`) are the darker stands
    *inside* it and the **only** areas that carry trees; **Grass** (lawns, meadows) and
    **Sand** (beaches) also sit above the park fill, lighter green / sandy. Everything
    but Wood stays open ground — that is what makes the open half of a park read as a
    field, the way it does on OSM.
    `height: Option<f32>` — metres, buildings only (`None` on water/parks even if the
    tag is there). See **Building height** below. `entrances: Vec<Vec2>` — the OSM
    doors on this building's outline, empty for most buildings; see **Entrances**.
  - **RoadLine** — centerline polyline + width by highway class (primary 16 → footway
    3.5). `RoadClass: Street | Alley` (alleys = footways, park paths; different color and
    z). `bridge` and `passage` flags — see navmesh.
  - **WallLine** — `barrier=city_wall` (the Tula kremlin), 3 m wide, kremlin red,
    impassable.
  - **trees** — `(pos, radius)` pairs, precomputed at parse.
- **Building height** (`parse.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim (New York — 97%, a
  LiDAR import) or else `building:levels` + `roof:levels` × `METERS_PER_LEVEL` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — every consumer owns a
  default. Coverage is logged per city on load (`N buildings (M with height)`).
- **Entrances** (`parse.rs::parse_entrance` + `attach_entrances`) — `entrance=*` **nodes**
  (the only nodes the Overpass query asks for), minus `NON_WALKABLE_ENTRANCES`
  (`no` = not a door at all, `garage`, `emergency`). An entrance in OSM is normally a
  *shared node of the building outline*, so attachment is an exact vertex lookup on a
  1 cm grid, not a nearest-neighbour search — it lands 82% of Tula's, 79% of Berlin's,
  65% of Paris's. The rest are orphans (porch nodes, buildings outside the bbox) and are
  dropped with a count on stderr. Overpass emits nodes before ways, so entrances are
  buffered through the element loop and attached after it. Coverage is thin everywhere
  (Tula 431 doors / 6946 buildings; NY and Tokyo ~300 city-wide) — hence the generator
  below. Real OSM doors always win: generation only runs on buildings that got none.
- **Generated entrances** (`map/osm/entrances.rs`) — synthetic doors for the ~98% of
  buildings OSM leaves without one, with every parameter measured off the buildings that
  *do* have them (5 cities, 14 941 attached doors; Tokyo's mirror 502'd and it carries
  ~310 doors city-wide, so it is absent from the sample). Two measurements drive the
  whole algorithm:
  - **A door faces the street.** Median angle between the outline edge's outward normal
    and the bearing to the nearest road: **0.0–0.9°** per city; 95.6% of real doors are
    within 45°, 98% within 90°. Distance from door to nearest road: median **0.7 m**,
    p90 5.8 m. So each outline edge is scored `road_distance + angle × 20 m/rad`
    (`ENTRANCE_FACING_PENALTY`), best-first, and doors go on the winning edges. A
    building with no road in reach still gets a door — it would otherwise vanish as a
    wander target.
  - **Door count follows length, not area.** Doors per 100 m of perimeter run 4.4 (shed)
    down to 0.65 (station), so a linear density is wrong. And **length beats area as the
    axis**: across 10 358 mapped buildings the mean runs 1.22 → 1.26 → 1.53 → 2.04 →
    2.96 over the length bands, and length keeps separating *inside* a single area band
    (at 800–2500 m²: 1.90 at 40–70 m against **3.11** at ≥ 120 m). By residual variance,
    length alone (1.073) is no worse than area alone (1.078), and length + area + height
    is the best combination tried (1.036 against an ungrouped 1.229).
  - **Building length** (`equivalent_length`) — the long side of the rectangle with the
    same area *and* perimeter: `L = (P + √(P² − 16A)) / 4`. Derived from those two rather
    than from an AABB, which doubles the reading for a diagonally-oriented slab, and
    which measures size rather than elongation. A shape more compact than any rectangle
    (`P² < 16A`) has no elongation and falls back to `P / 4`.
  - **Abandoned mapping, and why the naive mean is unusable.** Averaging over *every*
    building with ≥ 1 mapped door gives 2.73 doors at 120–200 m and 3.70 at ≥ 200 m —
    that is **one door per 94 m and per 133 m of building**. No such building exists.
    The cause is mappers who tag one door and stop; those buildings drag the mean down,
    and the longer the building the worse the damage. Restricting to buildings with
    **≥ 2** mapped doors removes exactly that failure and gives 3.35 / 4.18 / 4.42 for
    the 70–120 / 120–200 / ≥ 200 m bands. That is the estimator the cohorts use above
    40 m. It is not unbiased either — selecting on ≥ 2 cannot produce a 1-door result —
    so below 40 m the full-sample means are kept, where "one door" is the true answer
    and mapping is complete anyway. For reference the ≥ 3-door selection (mapping
    unambiguously enumerated) reads 4.40 / 5.48 / 5.67, so the values below are the
    conservative end of the plausible range, not the middle.
  - **Entrance cohorts** — length × height, with area as a demotion guard. Height only
    separates the long bands (at 70–120 m: 1.86 low vs 2.23 tall; at ≥ 120 m: 2.64 vs
    3.07 — about ±10% either side of the band); on short buildings it is noise (1.27 vs
    1.22). `p10 = 1` in every cohort, so the floor is always 1 door; `max` is the p90 of
    the same selection the mean comes from.

    | cohort | length | height | what it is | mean | max |
    |---|---|---|---|---|---|
    | hut | < 20 m | any | garage, kiosk, small detached house | **1.2** | 2 |
    | house | 20–40 m | any | terrace section, small block, shop | **1.35** | 3 |
    | row | 40–70 m | any | long shop, school wing | **2.0** | 4 |
    | block | 70–120 m | < 12 m | wide low corpus | **3.0** | 6 |
    | block, tall | 70–120 m | ≥ 12 m | apartment/office corpus | **3.7** | 6 |
    | slab | 120–200 m | < 12 m | long low corpus | **3.8** | 8 |
    | slab, tall | 120–200 m | ≥ 12 m | the *dom-korabl* | **4.6** | 8 |
    | quarter | ≥ 200 m | < 12 m | a whole block under one outline | **4.0** | 8 |
    | quarter, tall | ≥ 200 m | ≥ 12 m | — | **4.9** | 8 |

    A building with **no** height lands in the low branch (measured "unknown" tracks the
    low rows). **Area guard:** below `COHORT_SMALL_AREA` (800 m²) a long building is
    demoted to `row` — a 100 × 4 m garage row is long but has no podyezdy, and the
    measured 120–800 m² × 70–120 m cell is 1.46, not the long cohort's 3+.
  - **The pitch law beats the cohort mean, and drives the count.** The two measurements
    contradicted each other and the cohort table lost. Independent evidence for a fixed
    pitch: (a) the median gap between adjacent real doors is 26.7 m pooled, 22.6 m in
    Tula; (b) in the only exhaustively-enumerated sample anywhere — Tula's
    `entrance=staircase` *podyezdy*, which mappers list by convention — metres of length
    per door holds at **21.8–27.4 m in every length band**, from a 40 m house to a 200 m
    slab (1.43 → 3.26 → 3.58 → 5.75 doors). A cohort mean of 4 doors on a 200 m building
    means one door per 60 m and is incompatible with both. So `entrance_count` takes
    `length / ENTRANCE_SPACING`, and the cohort supplies only the **ceiling** (a 200 m
    factory is not a dom-korabl) and the small-building case where the pitch law yields
    zero.
  - **Spacing** — `ENTRANCE_SPACING` 25 m is the measured median gap between adjacent
    doors (26.7 m pooled; 22.6 m in Tula, where *podyezdy* are the best-enumerated doors
    anywhere in the sample). `ENTRANCE_MIN_SPACING` 12 m is the hard floor — the measured
    p10 is 4.5 m, but a navtile is 2 m, so doors closer than ~10 m resolve to the same
    tile and are not distinct targets. `facade_capacity` caps how many doors one edge
    absorbs so a long facade cannot hoard them.
  - **Blocked walls** (`FootprintIndex`) — a wall a neighbour stands against carries no
    door. OSM buildings routinely touch, share an outline edge, or overlap outright, and
    a door placed there sits *inside* the neighbour: invisible from the street and
    unreachable. Every candidate point is probed `ENTRANCE_CLEARANCE` (= `NAVTILE_SIZE`,
    2 m) along the edge's outward normal, and a probe that lands inside another
    `AreaKind::Building` kills that slot; the facade simply yields fewer doors and the
    next one by score picks them up. The probe distance is also the smallest gap worth
    a door — less than a navtile of free space in front and nobody can stand there.
    Lookups go through a 30 m uniform grid over the building outlines, built once per
    map. A building **walled in on every side** (usually a corpus that a mapper also
    traced over as a block-wide outline) still gets one door on its best facade — it
    would otherwise vanish as a wander target — and the count of those is logged
    (`N buildings have no free wall for a door`).
  - **Determinism** — the count is the only random draw (`floor(mean)` plus one more with
    probability `frac(mean)`, which reproduces the cohort mean exactly), and its LCG is
    seeded from the building's own first vertex — same family as tree planting. A given
    building therefore gets the same doors on every launch, independent of extract order
    or of which buildings were parsed before it. Neighbours matter only through their
    geometry (the blocked-wall test above), never through parse order.

  - **Seeing them** — the `doors` toggle in the debug row (bottom-left, `DebugDoors`,
    remembered by `prefs`) draws a gizmo circle on every entrance, real and generated
    alike. Same shape as `movepath`: per-frame gizmos culled to `DOORS_VIEW_SCREENS`
    around the camera, because ten thousand ungated gizmos cost a frame.

  **Measurement bias, on the record:** the means come only from buildings that have at
  least one door mapped, and a mapper who tags one door often stops there. So these are
  lower bounds, most trustworthy for `hut`/`house` (a house really does have one door)
  and least for `complex`.
- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.
- **Trees** — planted **only inside Wood polygons**, never across a whole park:
  deterministic LCG seeded per wood polygon, density ~1 / 1230 m², rejection sampling
  inside the polygon, never on buildings or within `TREE_CLEARANCE` (1.5 m) of a road
  edge (park alleys count as roads). Also rejected inside water or within
  `TREE_SHORE_CLEARANCE` (3 m) of a shoreline — a pond is drawn *over* the park fill, so
  an unfiltered tree grew out of the water — and anywhere inside a Grass or Sand polygon
  (a lawn is a lawn; overhang from a neighbouring tree is fine).
- **Rendering** (`map/meshing.rs` + `map/spawn.rs`, building layers in
  `map/buildings.rs`) — **one merged `Mesh2d` per layer** (parks, water, alleys, roads,
  building layers, walls): `MeshBuilder` triangulates polygons via `earcutr` (holes
  supported, degenerate contours skipped + counted) and emits per-vertex colors over a
  single white `ColorMaterial`. ~7000 buildings cost a handful of entities. Trees stay
  individual entities (see tree crowns below).
- **BuildingHeightMode** (resource, BRP-writable, persisted) — how a building's OSM
  height is drawn; any change reruns `rebuild_buildings` (despawn `BuildingLayerTag`
  layers, respawn from the unchanged `MapData::buildings`). The panel lives in
  `ui/buildings.rs`, bottom-right above the Trees panel, one cycling button. A building
  with no height uses `DEFAULT_BUILDING_HEIGHT` (15 m) everywhere. Modes:
  - **Facade** (default, the historical look) — pseudo-3D: the footprint polygon shifted
    straight down in a darker color at z just below the roof (`Z_FACADE` 4.9), visible
    only along south edges. Shift = height × `FACADE_SCALE` (0.2) clamped to 1.5–12 m, so
    a five-storey block keeps the historical 3 m band. Facades sit *under* every roof on
    purpose — that is what stops a tower's wide band from painting over its low neighbour.
  - **Shadows** — facade band plus a long shadow: one translucent merged mesh at
    `Z_BUILDING_SHADOW` (4.5 — *below* every building layer, so a neighbour's roof or
    wall masks the shadow and a shadow never lands on a same-height roof: the cheap
    stand-in for real height-aware casting; still above the portal and corpses, which
    are outdoors and in shadow by meaning). Per contiguous **silhouette chain** of the
    footprint (edges whose outward normal faces the 30° tree-shadow light) one swept
    polygon `[chain, chain + offset reversed]`, offset = height × `SHADOW_LENGTH_SCALE`
    (0.6) clamped to 3–45 m. Not per-edge quads — on staircase facades those overlapped
    along the shadow axis and the translucency stacked into stripes; a chain sweep
    cannot self-intersect (a silhouette edge's perp-step equals `outward·d > 0`, so the
    chain is monotone along the shadow perpendicular). All sweeps of the map are then
    merged by a boolean union (`i_overlay`, NonZero — sweeps are winding-normalized
    first) into disjoint shapes-with-holes, so the translucent layer never overlaps
    itself anywhere: no double-darkening between wings of one block or neighbouring
    buildings (unlike tree shadows, which still stack).
  - **Shadows+tint** — shadows plus a roof color ramp: `t = sqrt(height / 60 m)` mixes
    the roof toward a darker muted tone (max 0.7); no-height buildings and the Kremlin
    keep their base color.
  - **2.5D (Extrusion)** — watabou-style: roof lifted up by height × `EXTRUDE_SCALE`
    (0.35) clamped to 2.5–30 m, south-facing wall quads (vertical gradient) fill the
    gap; courtyard north walls included. No facade band, no shadows. Depth is painter's
    algorithm *inside one mesh*: buildings sorted north-first (index-buffer order is
    raster order), so a southern building correctly overlays its northern neighbour.
    Known limits: units y-sort against flat z=5 and can draw over a tall roof they are
    "behind"; kremlin wall polylines (z 5.1) draw over nearby lifted roofs.
  - **2.5D+shadows+tint (ExtrusionShadowsTint)** — everything at once: the extruded
    geometry with the tint ramp on lifted roofs plus the long-shadow layer.
- **Tree crowns** (`map/trees.rs`, algorithm write-up — `TREE_ALGO.md`) — Watabou-style
  procedural trees: a jittered 12-gon **bloated** into a cloud outline (recursive
  outward midpoint extrusion), ink outline, dashed inner **bands** shaded away from the
  light, and a **long shadow** — the crown silhouette stretched ×1.4 along the 30°
  shadow axis on `Z_TREE_SHADOW`. `TREE_VARIANTS` unit-radius mesh pairs (crown+shadow)
  are reused across all trees; per tree — variant, quantized brightness tint (material
  multiplies vertex colors, so ink stays ink) and radius as `Transform::scale`.
  Geometry RNG is a deterministic Lehmer LCG (same family as tree planting).
- **TreeStyle** (resource, BRP-writable) — the watabou «Style settings → Trees» tab:
  `foliage`, `details` (ink), `variance` (brightness spread), `shape`. **TreeShape** is
  `Cotton | Conifer | Palm` — cloud outline (`bloat`), spiky cone (`Spiker::simple`),
  bent fronds (`Spiker::bent`). Any change reruns `rebuild_trees` (despawn `TreeTag`,
  respawn from the unchanged `MapData::trees` positions); the panel lives in
  `ui/trees.rs`, bottom-right, one cycling button per field.

## Navigation

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * GRID_SIZE.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water blocks → **bridge corridors carve
  passable strips back** (`bridge=yes` roads) → buildings block → walls block →
  **building passages carve back through them**. Without bridges the Упа river bisects
  the map and no cross-river path exists.
- **Building passage** (арка) — a road that runs *through* a building: OSM
  `tunnel=building_passage`, or `covered=building_passage|yes` (both tag styles occur;
  `tunnel=yes` is an underground tunnel and is **not** one). `parse::is_building_passage`
  sets `RoadLine::passage`; the navmesh carves those centerlines passable **last**, after
  buildings and walls, since the whole point is to punch through a block that was just
  filled. Carve width is `min(road width, PASSAGE_MAX_WIDTH)` — the way is usually tagged
  `service` (5 m) but the arch itself is narrower, and an uncapped corridor would eat a
  tile of facade on each side. Tula has ~70 of them, London ~1700; without the carve,
  courtyards reachable only through an arch get sealed off by `prune_unreachable`.
- **Arch rendering** (`buildings.rs::arch_openings` + `push_wall_with_openings`) — the
  passage is also cut out of the *drawn* building. The opening is a rectangle **in the
  wall plane**, found from the passage's **endpoints**, not by segment intersection: an
  OSM arch is typically mapped outline-vertex to outline-vertex (Tula way 485488257), so
  the road lies inside the building and only its ends touch walls. At such a shared
  vertex the opening is laid across **every** wall within `ARCH_WALL_TIE` (0.5 m) of the
  nearest one — clamped to a single edge it came out half a road wide. Width = the road's
  own width × |sin| of the entry angle, trimmed to the edge; height = `ARCH_HEIGHT`
  (6 real metres — 3 is physical but read as 2 px on a tall slab) as a fraction of *that
  building's* height, `band × 6/height`, never taller than the wall. In 2.5D the wall is
  **really cut** (side pieces + a lintel above, `push_wall_with_openings`) so the layers
  beneath — the road running through, the ground — show through the hole, and
  `shadow_builder` patches the opening with `SHADOW_COLOR` (the lintel shades it; without
  the patch the hole glows). In facade modes the facade band is one earcut polygon, so
  the opening is *painted* in shaded ground colour instead — a stated compromise.
- **Row-span rasterization** (`row_spans`): an area is filled row by row — one pass over
  the ring per tile row yields the x-crossings, and the tiles between crossing pairs are
  set. Holes are subtracted as intervals, *not* merged into one even-odd list, so a hole
  poking outside its outer ring still subtracts instead of filling. Replaced a
  point-in-polygon test per tile of the AABB, which on London's Thames (huge bbox × long
  ring) cost 6.3 s of a 6.5 s fill; now 30 ms.
- **prune_unreachable** — BFS flood from the portal tile; passable-but-unreachable
  pockets (enclosed courtyards, islands) become impassable. Reason: an A* request to an
  unreachable target floods the whole reachable region (tens of ms each); before pruning
  this once piled up a 12 000-request backlog and humans "froze". 4-connectivity matches
  A* reachability because of the no-corner-cutting rule.
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>` resource; async A* tasks read it off-thread.
  Starts empty (all passable), filled and pruned by the map-load thread while the loader
  screen is still up (`JobState::BuildingNavmesh` / `Pruning`).
- **PathfindingAlgorithm** (`navigation/astar.rs`) — runtime-switchable resource, cycled
  by the bottom-left button: A* / Dijkstra / Fringe / BFS (all from the `pathfinding`
  crate over the navmesh) plus **HPA*** and **Theta*** (hierarchical, from
  `bevy_northstar`). IDA*/IDDFS are deliberately excluded (never finish on open grids).
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/pathfinding_bench.rs`
  (1.3 ms vs 36.4 ms mean, 15 ms vs 450 ms worst case) at ~10% longer paths. The other
  five stay switchable for comparison.
- **NorthstarGrid** (`navigation/northstar.rs`) — `bevy_northstar` `OrdinalGrid` built
  once from the final navmesh (after pruning; chunk 25), wrapped in `Arc`, called directly
  from async tasks — the crate's plugin is not used. Long paths cost ~0.5 ms vs ~40 ms for
  flat A*. The build takes **~12 s** on the 5600 × 3700 map, so it runs as an
  `AsyncComputeTaskPool` task started on `OnEnter(PlayPhase::Live)` and picked up by
  `poll_northstar_build`; until it lands, `NorthstarGrid::get()` is `None` and the
  dispatcher **falls back to flat A\*** for HPA*/Theta* requests. Doing it inline cost
  11 s of frozen loader screen; starting it before the warmup ends made it fight the
  warmup's A* for cores through rayon (85 ms per search instead of 36 ms).
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
  `Movable::to_pathfinding` only queues a `PathfindingRequest`;
  `dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks
  (polled with `check_ready`). **Visibility gating**: peacefully wandering humans
  OUTSIDE the camera view (×1.2 margin) are never dispatched — their requests wait
  until the camera arrives; demons and fleeing humans are always dispatched.
  **Priority** (`priority::` in `movement/systems.rs`): demons and fleeing humans
  (`URGENT`) go before wandering humans in frame (`WANDER_ON_SCREEN`), within a
  priority nearest-to-camera-center first, capped at `MAX_PATHFINDING_IN_FLIGHT`
  (512). The order only bites when the cap binds — in normal play in-flight sits
  around 100 of 512. The speed panel shows in-flight / queued / avg ms.
- **Repath on the move** — `to_pathfinding` keeps the current path and the
  `MovableStateMovingTag`, so an entity walks its old path while the new one is
  computed; `MovableStateMovingTag` therefore means "has a path", *not* "state is
  `Moving`". Dispatch and pickup both live in `Update`, so a reply costs at least a
  frame, and a fleeing human repaths every ~1 s: stopping for that frame left a
  quarter of all panicking humans standing at any instant at 10× (measured). When the
  reply lands, up to `REPATH_TRIM_LIMIT` (2) leading waypoints are dropped while the
  next one is no further than the first — the entity has moved off the tile the
  search started from, and without the trim its first step would be backwards.
- **find_passable_tile_near** — the target tile or its 8 neighbors only; callers must
  tolerate `None`.
- **pathfinding_bench** (`examples/pathfinding_bench.rs`) — offline comparison of all six
  algorithms without booting Bevy: reads the OSM cache, rebuilds the navmesh exactly as
  the map-load thread does (fill → `snap_portal_position` → prune), generates one
  seeded task list mirroring human wander (80% random building, 20% short stroll) and
  replays that *same* list per algorithm across a shared atomic work cursor. Reports
  wall / cpu / avg / p50 / p95 / max and mean path length. Run it after touching
  `successors`, costs, or the navmesh fill.
- **PortalPos** (resource) — actual portal position. `PORTAL_POS` in settings is only a
  **hint**; `snap_portal_position` spirals out to the nearest tile with clearance derived
  from `PORTAL_DIAMETER`. The map-load thread snaps it between fill and prune (the flood
  starts from the snapped position) and hands it back in `LoadedWorld`; `poll_job` inserts
  the resource before switching to `Playing`.

## Simulation

- **SimSet** (`spatial.rs`, `FixedUpdate`, gated on `Playing`):
  `SpatialRebuild → DemonBehavior → HumanBehavior`. Demons act before humans so a kill
  lands before `escape` — a human is never counted both killed and escaped in one tick.
- **SimPosition / PreviousSimPosition** — simulation-space positions; `Transform` is
  interpolated between them in `RunFixedMainLoop` (after the fixed loop). Systems mutate
  `SimPosition`, never `Transform.translation.xy` directly. Fixed-step order is explicit:
  `snapshot_previous_sim_positions` **before** `SimSet::SpatialRebuild`,
  `move_moving_entities` **after** `SimSet::HumanBehavior` — behavior may move
  `SimPosition` itself (demon lunge), and a snapshot taken after that would flatten one
  tick of interpolation.
- **Movable** — `{speed, path: VecDeque<IVec2>, state}`;
  `MovableState: Idle | Pathfinding(goal) | Moving(goal) | PathfindingError`.
  `to_pathfinding` queues the search and keeps the current path (see *Repath on the
  move*); `to_idle` is the only transition that stops movement.
- **SpatialGrid<T>** — uniform grid of `(Entity, Vec2)` per marker type (`Demon`,
  `Human`), 60 m cells (≥ the largest search radius), fully rebuilt every tick.
  `nearest_in_range_where` — nearest entity passing a filter.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s *between*
  walks, zero at spawn so nobody stands around after launch; then 80%
  head to a random building anywhere in the city — long routes, the real pathfinding
  load — and 20% stroll 20–40 m nearby) ⇄ **Flee** (demon within `HUMAN_PANIC_RADIUS`
  60 m; repath every 0.7–1.2 s, step 40–60 m away from the nearest demon). **Flee fan** — a
  non-chased fleeing human rotates its away-vector by a deterministic per-entity angle
  (±0.6 rad) so crowds spread instead of forming a column; actively chased humans flee
  straight. Calm-down at ×1.5 radius hysteresis. **Escape** — a fleeing human within
  `ESCAPE_MARGIN` of the map border despawns, `telemetry.escaped += 1`.
- **WanderHeading** — the direction a human is walking, kept between walks. Every next
  target, near stroll or cross-city errand, is picked inside a `WANDER_CONE` (60°)
  cone around it — a building errand samples `WANDER_BUILDING_TRIES` (8) random
  buildings and takes the first one inside the cone. Without the heading each pick was
  uniformly random and pawns wobbled in place instead of walking somewhere.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near*
  stroll, never a building errand; the tag is dropped when that target is picked. All
  20 000 humans queue their first path in the same frame, and cross-city A* costs
  hundreds of ms per request: with errands first the on-screen pawns took 3.9 s to route
  (the whole `PlayPhase::Warmup`), with strolls first — 0.15 s.
- **CorpseTag** — a killed human: behavior/movement components removed, dark lying
  sprite at `Z_CORPSE`. Not in the human spatial grid (grid filters on `Human`).
- **Demon** states (`demon/behavior.rs`): **Wander** (target biased away from portal) →
  **Chase** → **Devour** → Wander. Chase claims: **max 2 chasers per target**
  (`ChaserCounts`); a demon sharing a target opportunistically **switches** to an
  unclaimed human no farther than ×1.5 its current distance. Repath throttle 0.4 s.
  **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight` to the victim,
  the demon drops its path and steps `SimPosition` straight at the target. Without it a
  chase never converts: a tile path aims at the *center* of the victim's tile while the
  victim keeps moving inside it, so the last ~1.4 m — more than `KILL_DISTANCE` — is
  never closed and the demon "almost catches" forever. The line-of-sight check is what
  keeps the lunge from cutting through a building when the victim rounds a corner.
  A lunging demon carries **`DemonLungeTag`** (set/cleared in `chase`) — it has no tile
  path left, so the movepath gizmo would show nothing; `draw_lunge_paths` draws its arrow
  straight at the victim's live `SimPosition` instead.
  Kill at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); `killed_this_tick`
  HashSet dedupes double kills within one command flush. **Devour** — pause 1.5–2 s with
  a sine **pulse** ×1 → ×1.5 (0.5 s period), scale reset on exit.
- **DEMON_SPEED** — single constant, always `HUMAN_FLEE_SPEED × 1.35`, both wandering
  and chasing. Do not reintroduce per-state demon speeds.
- **DemonSpawner** — initial burst of 8 at the portal rim, then one per 5 s up to
  `DEMON_CAP = 100`. Runs in `FixedUpdate` so restart re-fires the burst for free.
- **Telemetry** — `{killed, escaped}`, BRP-readable. Invariant (check paused):
  `killed + escaped + alive == HUMAN_COUNT`. At high sim speed BRP reads are skewed —
  pause before asserting.

## UI & debug

- **Telemetry panel** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight /
  avg ms, entity count, camera. Fixed width + right-padded digits (no jitter).
- **Speed button** (`ui/speed.rs`) — left of that panel, a `Speed <value>` row-button in
  the Buildings-panel style. Left click walks the ladder up and wraps to 1x past
  `SPEED_CYCLE_MAX` (15x — the 60 fps ceiling), right click steps down; green while
  paused. It reads `Pointer<Click>` itself instead of `Activate`, which fires for *any*
  mouse button and would make one right click move both ways.
- **Tree style panel** (`ui/trees.rs`) — bottom-right: shape / foliage / crown details /
  color variance, one button per row cycling through a fixed palette (`bevy_ui` has no
  text input, so hex fields became cycles). Writes `TreeStyle`; `map::trees::rebuild_trees`
  picks the change up. Also settable over BRP: `res set TreeStyle .shape '"Conifer"'`.
- **Debug toggles** (`ui/debug.rs`) — grid / navmesh / movepath buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay is **one merged mesh** — per-tile entities once cost 330 k entities.
- **sim_time.rs** — Space pauses, `=`/`-` walk the speed ladder (unbounded, unlike the
  button's `cycle_time_scale`).
  - **SimSpeed** — `{requested, effective, actual}`. `requested` is what the ladder says;
    `effective` is the regulator's command, what reaches `Time<Virtual>` after **fps
    throttling**; `actual` is measured — virtual seconds per real second, averaged over
    `ACTUAL_SPEED_WINDOW` (0.5 s of *real* time, so long frames weigh what they cost).
    `actual` is the only honest one: Bevy clips a frame's virtual delta at `max_delta`, so
    a stall eats simulated time behind the regulator's back. The panel and `is_throttled`
    read `actual`.
  - **Speed ceiling** — Bevy hands `FixedUpdate` at most `Time<Virtual>::max_delta`
    (`MAX_FRAME_DELTA` = 0.25 s, pinned explicitly at startup) of virtual time per frame,
    so a speed of S is only real if `S ≤ fps × MAX_FRAME_DELTA` — 15 at 60 fps, 10 at
    40 fps. Above the ceiling the ticks pile into frames, `Update` (path dispatcher,
    input, UI) starves, and humans that finish a route just stand there.
    `throttle_speed_to_fps` closes the loop on measured fps and eases `effective` toward
    the ceiling (`SPEED_SETTLE_RATE` up, the faster `SPEED_DROP_RATE` down). It throttles
    **below 1× too** — under 4 fps even real time is unaffordable — down to
    `MIN_SIM_SPEED` (0.1). The button shows `15x → 8.6x` when limited, and
    `1x → 0.42x` while something (the async northstar build, say) is starving the
    frame.
  - Set the requested speed over BRP with `res set SimSpeed .requested N` — `brp speed`
    writes `Time<Virtual>` directly and the throttle overwrites it on the next frame.
  - **SimClock** — `elapsed`, virtual seconds the *current world* has lived, zeroed on
    entering `PlayPhase::Live` (so map load and warmup don't count, and a city switch
    restarts it). Not wall-clock: it stops on pause and runs `actual`× faster on speedup.
    The panel's first line shows it as plain seconds (`T+8130`), and it is readable
    over BRP as `SimClock`.
  - **Per-tick cost** (`sim/*_ms` diagnostics, 20 000 humans / 100 demons): `panic`
    ~1.8 ms ≫ `spatial` ~0.7 ms > `move` ~0.16 ms ≈ `flee` ~0.14 ms ≫ `chase` ~0.01 ms.
    `panic` scans every wandering human against the demon grid every tick — that single
    system is what sets the speed ceiling.
- **Remembered UI options** (`prefs.rs`) — every UI-settable resource (`DebugGrid`,
  `DebugNavmesh`, `DrawMovePaths`, `PathfindingAlgorithm`, `TreeStyle`) is a
  `bevy::settings::SettingsGroup`, so a click survives a restart. `SettingsPlugin` reads
  `settings.toml` from the OS settings dir (macOS:
  `~/Library/Preferences/com.github.morr.qwe/`) while the `App` is still being
  built, before any schedule; `PrefsPlugin` is registered **last** because that scan needs
  the other plugins' `register_type` calls to have run. Any change to those resources —
  click, key P, BRP — triggers `SaveSettingsSync::IfChanged`. Delete the file to reset.
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
  geo anchor).
- OSM pipeline: `src/map/osm/{overpass,download,parse,model}.rs`; rendering:
  `src/map/{meshing,spawn}.rs`.
- Navigation: `src/navigation/{navmesh,astar,mod}.rs`; movement/interpolation:
  `src/movement/`.
- State machines: `src/demon/behavior.rs`, `src/human/behavior.rs`.
- Tests: `tests/navigation.rs` (synthetic navmesh + hand-built `MapData`),
  `tests/spatial.rs`, `tests/movement.rs` (fixed-step walk over several waypoints,
  step length independent of `time_scale`, `Transform` interpolation), unit tests inside
  `map/osm/*` and `map/meshing.rs` (projection, ring assembly, tree determinism, earcut).
