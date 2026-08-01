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
- **Z-layers** — constants in `settings.rs`: ground 0 → parks 0.5 → woods 0.55 → grass
  0.6 → sand 0.7 → water 1 → waterways 1.05 → alley casings 1.4 → alleys 1.5
  → road casings 1.9 → roads 2
  → bridge casings 2.1 → bridges 2.2
  → rails 2.4 → rail dashes 2.5 → tram 2.6 → corpses 3 → portal 4 → buildings 5 → units → tree
  shadows 19 → trees 20. Three more
  live in their own modules: `Z_BUILDING_SHADOW` 4.5 and `Z_FACADE` 4.9
  (`map/buildings/mod.rs`), `Z_WALL` 5.1 (`map/roads.rs`). Units are y-sorted:
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
  resets the camera (`camera.rs::place_camera_on_world_ready` — onto the new portal, back
  to `START_ZOOM`, whatever `CameraPositionMode` says: the saved view belongs to the map
  that was just thrown away). `DemonSpawner`, `Telemetry`, `NorthstarGrid` and `WarmupProgress` are
  reset on the way. The switch is gated on `in_state(Playing)` — restarting a load on top
  of a running one would put two threads into one navmesh.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city.
  Every world entity must carry it; the list of spawn sites and the rule live in
  `CLAUDE.md` ("World entities"), and `loading.rs::warn_leftover_world_entities` warns on
  every entry into `Loading` if something survived.

## OSM map pipeline

- **Overpass** — the Overpass API (`overpass-api.de`), queried once with `[out:json]` +
  `out geom` (inline geometry, no node lookup). Query covers: `building` (way+rel),
  `highway` (way), `natural=water` / `waterway=riverbank` (way+rel),
  `waterway=river|stream|brook|canal|ditch|drain|weir` (way — the *linear* watercourses),
  `leisure=park|garden`,
  `landuse=recreation_ground|forest` + `natural=wood`, `natural=tree_row` (way),
  `natural=tree` (node), `landuse=grass|meadow` / `natural=grassland|meadow`,
  `natural=sand|beach`, `barrier=city_wall`. The bbox is `MAP_SIZE` around the selected
  `City`'s geo center. `QUERY_VERSION` is **7** (v3 added `entrance` nodes, v4 `railway`,
  v5 `natural=tree_row`, v6 `natural=tree` nodes, v7 linear `waterway`).
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
    z). `bridge` and `passage` flags — the navmesh carves (see navmesh); `bridge` also
    moves the road into the bridge deck layers (see **Bridge layers** below).
  - **RailLine** — `railway=*` centerline + width by value (`rail` 5 → `light_rail` /
    `narrow_gauge` / `subway` 4 → `tram` 1.2). `RailKind: Active | Tram | Disused` — the
    kind *is* the drawing style, not a label: **Tram** is a thin line with cross ties
    (see **Tram** below — its width from parse is ignored, the zoom LOD picks it),
    **Disused** (`abandoned` / `disused` / `razed` / `dismantled`)
    is the `Active` ribbon washed out. A tram runs *on* the carriageway, so a
    gauge-wide ribbon would cover its own street.
    `parse::rail_class` is a
    **whitelist**, so the station vocabulary (`platform`, `station`, `switch`, `signal`,
    `construction`, …) never becomes a line. The rail branch in `parse_way` runs *before*
    the highway branch and deliberately **falls through**: an OSM way is routinely tagged
    both `railway=tram` and `highway=*`, and such a way is both a street and a track.
    **Underground track is dropped** (`parse::is_underground`) — a metro tunnel or a
    sunken through-line is invisible from above. Both markers are needed, neither alone
    suffices: of Tula's three underground ways two carry `tunnel=yes` *and* `layer=-1`,
    the third only `layer=-1`. `tunnel=no` is an explicit no, and elevated track
    (`layer` ≥ 0, no tunnel) still draws — that is what keeps an elevated subway on the
    map. **Rails never touch the navmesh** — see below.
  - **WallLine** — `barrier=city_wall` (the Tula kremlin), 3 m wide, kremlin red,
    impassable.
  - **WaterLine** — a *linear* watercourse: `waterway=river` 8 m → `canal` (and `weir`)
    6/4 m → `stream|brook` 2.5 m → `ditch|drain` 1.5 m, water blue, one merged ribbon at
    `Z_WATERWAY`. Widths are drawing widths, not hydrology: OSM draws as a line what is
    too narrow for a polygon, so a `river` line is narrower than the Упа (which is an
    area). A plausible `width` tag (`WATER_WIDTH_RANGE`, 0.5..50 m) overrides the class
    default. `parse::water_class` is a **whitelist** for the same reason `rail_class` is:
    `waterway=*` also carries `riverbank` (that one is an area, and `area_kind` claims
    it), `dam`, `dock`, `lock_gate`, `waterfall`. Like the rail and tree-row branches,
    the waterway branch in `parse_way` runs before `highway` and **falls through** — a
    culverted stream under a street shares its way with `highway=*`.
    **`tunnel: bool`** (`parse::is_underground`, the same test that drops subway track)
    marks a piped section: it is drawn as a **dashed** ribbon and, alone among
    watercourses, **does not block the navmesh** — the water runs under the ground and a
    pawn walks over it. Everything else about waterways *does* block; see Navigation.
  - **TreeRow** — `natural=tree_row`: an avenue's centerline polyline plus what the data
    itself knows about the planting — `spacing: Option<f32>` (from `spacing`, or the row
    length spread over `count` / `tree:count`) and `radius: Option<f32>` (half
    `diameter_crown`). Both are rare, semi-standard tags, so almost every row is
    `None`/`None` and falls back to the density slider. Like the rail branch, the
    `tree_row` branch in `parse_way` runs before `highway` and **falls through**.
  - **TreeNode** — `natural=tree` node: a single surveyed tree, position plus
    `radius: Option<f32>` (half `diameter_crown`, same parse as on rows). Raw input for
    `planting::plant_standalone`; see **Standalone trees** below.
  - **wood_trees / row_trees_kept / row_trees_slid** — `(pos, radius, appears_at)`,
    each sorted by threshold: the forest (with standalone surveyed trees at threshold 0
    in front), and the avenues under each placement policy.
    Raw material, not what the renderer reads.
  - **trees** / **tree_appears_at** — what the renderer reads: `MapData::compose_trees`
    merges the forest with the avenues of the selected policy (a merge, not a sort — both
    inputs are already ordered). `composed_for` records which policy it was built for;
    it lives on `MapData` rather than in a system `Local` precisely because a city switch
    replaces the whole resource, and a `Local` would survive it and skip the rebuild.
- **Building height** (`parse.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim (New York — 97%, a
  LiDAR import) or else `building:levels` + `roof:levels` × `METERS_PER_LEVEL` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — every consumer owns a
  default. Coverage is logged per city on load (`N buildings (M with height)`).
- **Drowned buildings** (`parse.rs::drop_buildings_in_water`) — a building whose outline
  lies **entirely** inside a water polygon is dropped right after the element loop, before
  doors and trees. OSM tags floating restaurants and moored ships as buildings (`HMS
  Belfast`, `Café Barge`) and Tula carries a lone shed in the middle of Верхний пруд; the
  navmesh floods water impassable, so their doors are unreachable anyway and the box
  standing on the pond reads as a render bug. One vertex on land is enough to survive —
  piers and embankment houses stay. Counts: Tula 1, Berlin 6, NY 17, London 28, Paris 28,
  Tokyo 0; logged on stderr when non-zero.
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
- **Generated entrances** (`map/osm/entrances/`) — synthetic doors for the ~98% of
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
  - **Entrance cohorts** (`entrances/cohorts.rs`) — length × height, with area as a
    demotion guard. Height only
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
- **Trees** (`map/osm/planting.rs::plant_woods`) — planted **only inside Wood polygons**,
  never across a whole park (avenues are separate, see **Tree rows** below):
  deterministic LCG seeded per wood polygon, rejection sampling
  inside the polygon, never on buildings or within `TREE_KERB_CLEARANCE` (0.5 m) of a road
  edge (park alleys count as roads) or `TREE_WALL_CLEARANCE` (1.5 m) of a building wall,
  the latter measured from the **crown** edge. Also rejected inside water or within
  `TREE_SHORE_CLEARANCE` (3 m) of a shoreline — a pond is drawn *over* the park fill, so
  an unfiltered tree grew out of the water — and anywhere inside a Grass or Sand polygon
  (a lawn is a lawn; overhang from a neighbouring tree is fine). The same shore clearance
  applies to a **linear** watercourse, measured from the ribbon edge (`NearbySegments`,
  the per-segment index shared with roads); culverts are excluded, since above a pipe
  there is ground and a tree on it is legitimate.
- **Standalone trees** (`planting.rs::plant_standalone`) — single surveyed trees from
  `natural=tree` nodes, planted **first**, before the forest and the rows, so both keep
  `TREE_MIN_SPACING` from them via the shared `Occupied` grid. A node is dropped when the
  procedural planting already covers it: inside any Wood polygon, or within
  `TREE_MIN_SPACING` (6 m) of a `tree_row` centerline. Also dropped when
  `Obstacles::solid` (in a building or water), when the trunk stands on a road bed or
  its casing (`Obstacles::on_road` — our synthesized roads are wider than real ones, so
  a pavement tree routinely lands "in the asphalt"), or on `Occupied::crowded` (a
  duplicate node). Kerb-side and lawn trees survive: unlike the road clause of
  `blocked`, `on_road` has no crown gap — the same reasoning as
  `TreeRowPlacement::Keep`. Every standalone tree gets
  `appears_at = 0` (a surveyed tree is visible at any density), radius from
  `diameter_crown` or rolled in the forest range from an LCG seeded by the node's own
  coordinates. They ride in front of `wood_trees`, so everything downstream — crowns,
  shadows, conifer field, density prefix — needs no new code.
- **Tree rows** (`planting.rs::plant_rows`) — avenues from `natural=tree_row`, walked
  along the polyline instead of sampled inside a polygon. Everything downstream is
  untouched: row trees land in the same `MapData::trees`, so crowns, the merged shadow
  layer, the conifer field and the density prefix apply to them without a line of new
  rendering code.
  - **A row carries a green band under it** (`spawn.rs::spawn_tree_row_band`) — a ribbon
    of `TREE_ROW_BAND_WIDTH` (10 m) in `WOOD_COLOR` at `Z_TREE_ROW_BAND`. On the map an
    avenue is a wood one crown wide, and without the band its crowns hang over bare
    asphalt while every park tree stands on green. The band is deliberately narrower than
    the full crown reach (12 m) so crowns overhang its edge — otherwise the eye reads the
    stripe instead of the trees. It sits just above `Z_WOOD` and therefore *under* alleys
    and roads, exactly as the wood fill does inside a park.
    It is its **own entity**, not part of the merged `woods` mesh, because it carries the
    same three knobs the road ribbons do — **join, smoothing (Chaikin), casing** — reusing
    `roads::push_ribbon` / `smooth_path` / `casing_width` verbatim. The knobs are separate
    from the Roads panel's on purpose: an avenue's polyline and a street's come from
    different data, and the band must read as *wood* even where the roads are left raw.
    Defaults differ from roads accordingly — smoothing `Light` (a street may turn a
    corner, a wood never does) and casing off (a second green outline reads as one more
    path alongside the avenue).
  - **Spacing comes from the data when the data has it** — and the *Row spacing* toggle
    decides whether we listen. On `OSM` (default) `TreeRow::spacing` (from `spacing`, or
    the row length spread over `count`) fixes the planting, and every tree of such a row
    gets `appears_at = 0`: the slider does not thin what the map already decided. On
    `slider`, or when the tags are absent, the row is planted at `TREE_MIN_SPACING` (the
    same physical floor as the forest) and thresholds come from the slider, below.
  - **Spacing is derived from the forest, not chosen.** A wood at density `d` holds one
    tree per `TREE_AREA_PER_TREE / d` m², so its neighbours sit roughly `√(410/d)` apart —
    `row_spacing_at(d)`. A row targets exactly that, so the threshold of slot `n` is
    `density_for_row_spacing(length / n)` = `n² · 410 / length²`. The **square** is the
    whole point: the wood is two-dimensional and a row is one-dimensional, and the earlier
    linear formula (a flat 9 m at `d == 1`) made every avenue about twice as dense as the
    park beside it — a solid green sausage next to a sparse wood, and it pinned the row to
    `TREE_MIN_SPACING` at slider settings where the forest never comes near that floor.
  - **Ranks are bit-reversed** (van der Corput, `scattered_ranks`). On a line the "first
    n in order" is its *beginning*, so a natural order would show half the avenue and
    half bare ground. Reversing the index bits makes every prefix spread over the whole
    row while keeping thinning monotone.
  - **`TreeRowPlacement`, a toggle in the Tree rows panel.** Road widths here are
    *synthesised* from the highway class (8–16 m) and know nothing about real kerbs, so a
    mapped avenue routinely lies inside our road polygon. **`Keep`** (default) trusts the
    OSM position and rejects only what can never be right — inside a building or in water
    (`Obstacles::solid`); the full check would erase exactly the boulevard rows the
    feature exists for. **`Slide`** runs the full `Obstacles::blocked` and walks a
    blocked tree forward along the row in 1 m steps, at most one planting step, then
    gives up on it. Both respect the forest's `Occupied` grid, so a row crossing a wood
    never stacks on a forest tree.
  - **Every layout is planted at load.** `TreeRowLayout` = placement × `osm_spacing`, and
    both axes move *positions*, so all four combinations are planted up front into
    `MapData::row_trees` (`RowTrees`); flipping either toggle only re-runs
    `MapData::compose_trees` + a conifer resample (`trees::recompose_row_trees`).
    Planting on click would mean rebuilding `Obstacles` — index grids over ~7 000
    buildings and every road — which is far too expensive for a UI toggle; four passes
    over a few hundred rows are not. The load log prints all four counts.
- **Tree density** — base density is 1 / `TREE_AREA_PER_TREE` (410 m² of wood outline) at
  `TreeStyle::density == 1`; the slider multiplies it, `TREE_DENSITY_MIN` (0.25, in
  `settings.rs`) … `TREE_DENSITY_MAX`, step 0.25. Planting runs once at the ceiling, so
  `MapData::trees` holds the densest forest and the slider only **shows a prefix** of it —
  never a replant, which would reshuffle every position and make the whole forest jump on
  each step.
- **The density ceiling is derived, not chosen** (`planting.rs`) — `TREE_MIN_SPACING` (6 m)
  caps how dense a forest can *physically* get: random placement with a hard-core exclusion
  saturates near `RSA_JAMMING_FRACTION / (π·(d/2)²)` trees per m² — one per ~52 m² at
  d = 6, i.e. ~7.9× the base. `TREE_PLANTING_DENSITY` is that number times
  `TREE_DENSITY_HEADROOM` (0.8, because the approach to saturation is asymptotic), and
  `TREE_DENSITY_MAX` is it rounded up to a slider step — **6.5×** today. It is computed so
  that editing the spacing can't silently strand the top of the slider. Raising the ceiling
  beyond this does nothing; the lever for a denser forest is `TREE_MIN_SPACING`.
- **`MapData::tree_appears_at`** — the density at which each tree appears, same length and
  order as `MapData::trees` (sorted ascending). Threshold is
  `(rank within its wood + 1) · TREE_AREA_PER_TREE / wood area`, so every wood contributes
  exactly its own share at any density, *including* woods that hit saturation and never
  filled their ask. Row trees carry their own threshold (see **Tree rows**), and a row
  whose spacing came from OSM carries `0` — it stands whole at every step of the slider.
  `map::trees::visible_count` is then a `partition_point` — thinning is
  monotone (a step up only adds trees) and exact, where the earlier hash-share thinning
  drifted ~20% sparse at 1× because it divided by the nominal ceiling the map never reached.
- **Asked vs planted** — the log line
  `osm parse: N trees planted of M asked, a/b/c/d in R tree rows (keep/slide x osm/slider)
  in T` is the health check: Tula plants 15 356 of 21 155 (73%). The shortfall is real and expected —
  a wood outline contains alleys, lawns and ponds where nothing can stand, and the last
  few percent of saturation costs unbounded attempts. `ATTEMPTS_PER_TREE` (60) is the knob:
  doubling it from 30 bought +740 trees for +67 ms of load.
- **Planting is indexed, not scanned** — `blocked()` (is this spot taken by a building,
  road, pond, lawn?) runs once per rejection-sampling attempt, tens of thousands of times,
  and a linear pass over 7 475 buildings and every road was almost the whole planting cost
  (615 ms on Tula). Candidates now come from uniform cell grids over the same padded AABBs
  (`NearbyAreas`, and `NearbySegments` — roads and watercourses indexed **per segment**,
  because a river's AABB spans the map; the cell carries the ribbon width, since the two
  sources index different vectors). Same idiom as `entrances/index.rs`; the precise tests behind the
  lookup are unchanged, so the planted set is identical.
- **Rendering** (`map/meshing.rs` + `map/spawn.rs`, road layers in `map/roads.rs`,
  building layers in `map/buildings/`) — **one merged `Mesh2d` per layer** (parks, water,
  waterways + culverts, alleys, roads, building layers, walls): `MeshBuilder` triangulates polygons via
  `earcutr` (holes supported, degenerate contours skipped + counted) and emits per-vertex
  colors over a single white `ColorMaterial`. ~7000 buildings cost a handful of entities.
  Trees stay individual entities (see tree crowns below).
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
- **Junctions are not computed.** Overpass returns `out geom`, so shared node identity
  between ways is never available; roads are independent polylines drawn overlapping in
  one opaque single-colored layer. `Round` caps are what makes a junction *look* joined —
  the caps of the ways meeting at a node overlap into a rounded blob, exactly how
  osm-carto gets its smooth junctions (`stroke-linejoin: round` + `stroke-linecap:
  round`). This is why the road layer must stay opaque and flat-colored: transparency or
  a per-way tint would expose every crossing.
- **RoadStyle** (resource, BRP-writable, persisted; panel `ui/roads.rs` above Buildings)
  — how road ribbons are drawn; any change reruns `rebuild_roads` (despawn
  `RoadLayerTag` layers, respawn from the unchanged `MapData`). Three independent knobs:
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
  (`Z_BRIDGE` 2.2): a gray **curb** (`BRIDGE_CURB_COLOR` 0.60, 12% of the width clamped
  0.8–2 m) under the fill in the class color. The 2GIS look — the curb bands along both
  deck edges are what makes a bridge read as a bridge, so the curb draws **always**,
  independent of `RoadStyle::casing`, and is both darker and thicker than a casing so
  the two never blend. Curb caps are always `Butt` (`push_bridge_curb`) — the deck ends
  in a square cut; a `Round` half-disc or the `Square` end-extension would poke a curb
  tongue past the bridge end. The deck sits above `Z_ROAD` so an overpass covers the
  street it crosses, and below `Z_RAIL` so a track on the bridge stays visible; curbs
  below fills for the casing reason (a junction of two bridge ways is never cut by a
  curb band). Street and footbridge fills share one mesh — bridge-over-bridge overlap
  is push order, rare enough not to warrant four layers. Rails carry no bridge flag —
  rail bridges are out of scope.
- **Rail layers** (`map/roads.rs`, same file and the same `RoadLayerTag`, so a style
  change rebuilds them with the roads) — osm-carto's dashed railway, two merged meshes:
  a dark bed at `Z_RAIL` (2.4) and a white dash pattern at `Z_RAIL_DASH` (2.5), 6 m on /
  6 m off, dash width 60% of the bed. Two layers rather than one mesh, for the casing
  reason inverted: coplanar geometry z-fights, and the dashes must sit above *every*
  bed. Both above `Z_ROAD` (2) so a track lies on its street, not under it.
  `MeshBuilder::push_dashes` is the primitive — a single arclength pass emitting
  `Butt`-capped ribbon chunks, keeping the OSM vertices inside a dash so the pattern
  turns with the track. A way shorter than one dash still gets one, since most ways in a
  junction are short and a bare bed reads as a road. Tram ways are skipped here — they
  have their own module.
- **Tram** (`map/tram.rs`, its own module so a zoom-LOD step never rebuilds the
  road/rail meshes) — a thin blue line with perpendicular cross ties, the
  Yandex/2GIS convention; `TRAM_COLOR` is the only thing separating the two (Yandex dark
  red, 2GIS blue) and we take 2GIS's blue, since red on this map already means kremlin
  wall. Line and ties share one colour, so both go in one mesh (`TramLayerTag`, `Z_TRAM`
  2.6 — above the rail dashes at crossings, name `tram`) — self-overlap costs nothing,
  and there is no white dash layer for a tram. The tie primitive is
  `MeshBuilder::push_ticks`: the same arclength walk as `push_dashes`, but each mark is
  a perpendicular bar rather than a piece of the path, and the first one is offset half
  a step so a bar never lands exactly on a way endpoint and pairs into a cross at joins.
  The style is fixed, no panel and no resource: on a line 1.5–2 px wide a join style is
  invisible and Strong smoothing is indistinguishable from Light, so it is hardwired to
  `Round` + `Light` (`TRAM_JOIN` / `TRAM_SMOOTHING`), and the sparse tie spacing is
  baked into the LOD table.

  **Tram zoom LOD** (`TRAM_LODS`) — the mesh is rebuilt at discrete zoom thresholds,
  pseudo-gizmo style: five buckets over the camera zoom range, each with its own line
  width (targeting ~1.8 screen px, so the line neither fattens close up nor vanishes far
  out) and tie length/thickness/spacing (on-screen tie spacing never drops below ~10 px);
  the farthest bucket drops ties entirely, as 2GIS does at city scale. `TramZoomBucket`
  (resource, **not** persisted — zoom comes back from the camera's start view on every
  world entry: `START_ZOOM`, or the saved zoom under `position: save`) holds
  the current bucket index; `update_tram_zoom_bucket` recomputes it each Update frame
  from `PanCamera::zoom_factor` via `set_if_neq`, so `rebuild_tram` fires only on an
  actual threshold crossing, never per frame. The tram centerline is smoothed with a
  fixed `TRAM_SMOOTH_WIDTH` (1.2 m) clamp rather than the bucket's line width, so the
  path itself is identical across buckets and LOD switches don't wiggle the track.
  `RailLine::width` from parse is ignored for trams.
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
    footprint (edges whose outward normal faces the 30° light — `map/mod.rs::SHADOW_DIR`,
    one source for building and tree shadows alike) one swept
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
  shadow axis on `Z_TREE_SHADOW`. Each shape shades its bands its own way, as watabou
  does: cotton by single edges through the RNG (`drawShaded1`), **conifer by chevrons
  with no RNG at all** (`drawShaded2` — base→spike→base in one deterministic stroke, so
  a tier is one unbroken zigzag on the shadow side and the lit side stays clean; the
  innermost band, lifted to the top, reads as the fir's tip), palm by whole leaves
  (`drawShaded4`). The conifer outline is built by `cone_outline`, which keeps every
  notch readable under the 12%-of-radius ink: a notch narrower than two strokes reads as
  a black needle inside the crown, one shallower than one stroke as a flat-topped bump —
  the first is opened by nudging the offending base vertex across its neighbours' chord,
  the second by a floor on spike height (watabou's `len^1.5` leaves short edges stubby).
  Cloud and palm skip the pass — their sub-stroke ripple is meant to melt into the ink.
  A tree's «height» `h` (`0.4 + 0.8·gauss3`, per crown variant) picks
  its shadow: long, or plain offset, or — for a conifer — the **cone fan** of shrinking
  silhouettes along the shadow (`drawConiferShadow`), unioned with `i_overlay` so the
  translucent copies never stack into double darkness. `TREE_VARIANTS` unit-radius crown meshes are reused
  across all trees; per tree — variant, quantized brightness tint (material multiplies
  vertex colors, so ink stays ink) and radius as `Transform::scale`.
  Geometry RNG is a deterministic Lehmer LCG (same family as tree planting).
  **Shadows are one merged mesh** (`tree_shadows`, like `building_shadows`), not an
  entity per tree: the silhouette template of each variant is baked into it with the
  tree's offset and radius. A blended `Mesh2d` lands in the sorted `Transparent2d`
  phase, and a thousand of them sharing one z alongside the pawn sprites lose a
  random one or two per frame — the tree shadow visibly blinks. One mesh, one phase
  item, no blinking (and one draw call instead of hundreds).
- **TreeStyle** (resource, BRP-writable) — the watabou «Style settings → Trees» tab:
  `foliage`, `details` (ink), `variance` (brightness spread), `shape`, `conifer_share`
  and `noise_mix` (see Conifer stands below), `density` (planting multiplier, see Tree
  density above), plus the two source toggles — `woods` (forest polygons) and
  `standalone` (individual `natural=tree` trees). **TreeShape** is `Cotton | Conifer | Palm | Mixed` — cloud
  outline (`bloat`), spiky cone (`Spiker::simple`), bent fronds (`Spiker::bent`), and
  conifer stands among cloud crowns. Any change reruns `rebuild_trees` (despawn
  `TreeTag`, respawn from the `MapData::trees` positions); the source toggles
  additionally rerun `recompose_row_trees` first, because they change the position set
  itself rather than the look. The panel lives in `ui/trees.rs`, bottom-right, one
  cycling button per field.
- **TreeRowStyle** (resource, BRP-writable, persisted; panel `ui/tree_rows.rs` above
  Trees) — the avenue knobs, split from TreeStyle the way Buildings is: `enabled` (rows
  on/off — removes both the row trees and the green band), `placement`
  (`TreeRowPlacement`), `osm_spacing`, and the band's `join` / `smoothing` / `casing`
  (see Tree rows above). Which sources end up in `MapData::trees` is captured by
  **`TreeCompose`** (layout + woods/rows/standalone flags, `model.rs`) — the value stored
  in `composed_for`, so `recompose_row_trees` re-merges only when the composition
  actually changed. Crown look is inherited from `TreeStyle`.
- **Conifer stands / conifer field** (`map/trees/conifer.rs`, `ConiferField` resource) —
  which trees of a `Mixed` forest are spruce. Conifers grow in **stands**: a patch of
  forest is conifer almost entirely, and between patches there is almost none. So the
  species is **not** a function of the tree's index in `MapData::trees` (that carries no
  geography and would scatter single conifers among the cloud crowns) but of an
  **fbm-simplex field of the trunk's world position** — neighbouring trees read nearly
  the same value and turn conifer together. The fbm parameters live in
  **`ConiferNoiseStyle`** (resource, persisted, BRP-writable): `wavelength` (default
  400 m sets the stand size, ~120–250 m across), `octaves`, `lacunarity`, `persistence`
  — tunable at runtime from the **Noise panel** (`ui/noise.rs`, bottom-left above the
  debug toggles, visible only while the `noise` debug toggle is on; ranges modeled on
  zxc's noise sliders). The seed stays fixed, so a city looks the same every run.
  - The cut is an **empirical quantile** of the field's values at the trees, not a fixed
    noise level: fbm is bell-distributed, so «everything above 0.9» would give a share
    unrelated to the one asked for. The quantile makes `TreeStyle::conifer_share` an
    exact share at any noise parameters, and clustering is unaffected — the trees kept
    are still the ones on the peaks. 0 % / 100 % are special-cased to «nobody» / «all».
  - **Mix jitter** (`TreeStyle::noise_mix`, «Noise mix» slider next to Conifer share) —
    stands need not be solid: each tree's value gets `mix · jitter` added at resample
    time, where jitter ∈ ±0.5 is a position-hashed (murmur3 finalizer), deterministic
    per-trunk offset. It pushes trees across the threshold both ways — deciduous
    inclusions inside stands and lone spruces deep in deciduous masses, deeper the
    higher the mix. Baked **before** the quantile, so the share stays exact at any mix;
    hashed by **position**, not index, so composition toggles and density thinning never
    flip a standing tree's species. Mix 0 restores solid stands (test-pinned).
  - Values are sampled once per city in `build_conifer_field` (`WorldInitSet::Spawn`,
    before `spawn_map`); only the threshold moves when the share slider does. Noise or
    mix edits resample via `retune_conifer_field` in the rebuild chain (it no-ops when
    the field is already sampled for the current params — `ConiferField` remembers what
    it was sampled with, plus a `generation` counter the overlay uses as cache key).
  - Species is **orthogonal to density thinning**: the quantile runs over all planted
    trees while the density slider spawns a prefix of them (`visible_count`), and that
    prefix is a spatially uniform subsample, so the share among spawned trees holds and a
    tree does not change species as the slider is dragged.
  - The **noise** debug toggle (`ui/debug.rs::sync_conifer_noise_overlay`) shows the
    field as one CPU-built 512² texture sprite over the whole map on
    `Z_CONIFER_NOISE_OVERLAY`: grey ramp = field value, green = at or above the current
    threshold, i.e. the stands the current share will produce. Green also covers built-up
    areas — the field is defined everywhere, trees only grow in Wood polygons. The
    overlay draws the **un-jittered** field: at mix > 0 single crowns deliberately sit
    on the «wrong» side of the green boundary.

## Navigation

- **Navmesh** (`navigation/navmesh.rs`) — `Vec<bool>` passability grid, index
  `x * GRID_SIZE.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge corridors carve passable strips back**
  (`bridge=yes` roads) → buildings block → walls block → **building passages carve back
  through them**. Without bridges the Упа river bisects the map and no cross-river path
  exists.
- **Linear waterways block, unlike rails** — a `WaterLine` is water, and water is crossed
  by bridge, not waded. They carry the rail hazard below (an unbroken thread across the
  city that `prune_unreachable` would amputate a bank of), so two things keep the map
  connected and both are load-bearing: the bridge carve runs *after* this fill, and
  **culverts do not block at all** (`WaterLine::tunnel`) — where a stream crosses under a
  street, OSM far more often pipes it (`tunnel=culvert`) than bridges the street over it.
  The number that proves it held is the **pruned-tile count** in the log — a jump of
  thousands means a watercourse cut a district off, and that is the thing to check after
  any change here or in `water_class`. On Tula (25 waterways, 7 of them culverts) the
  channels took 6 461 tiles out of the passable set and pruning did **not** move at all
  (9 781 before and after), i.e. no bank was severed. Much of that is free: the Упа's
  *centerline* is also tagged `waterway=river`, and it runs inside the Упа water polygon,
  which was already impassable.
- **A rasterized polyline is a 4-connected chain, by construction.** `set_polyline` marks
  tiles whose *center* is within half the width — and that alone is not a barrier: below
  `NAVTILE_SIZE · √2` (2.83 m) a slanted band degenerates into tiles touching only at
  their **corners** (the navmesh overlay draws it as a chequerboard along the line). Our
  own A* cannot step through that — it does not cut corners — but every other consumer
  can: `bevy_northstar`'s `OrdinalGrid` (HPA*, Theta*) is built with no corner-cutting
  filter and steps diagonally between two blocked tiles, and `line_of_sight` samples
  points along a ray and slips through the contact point. A 2.5 m stream was crossed by
  pawns on HPA* for exactly this reason. So `set_polyline` also walks the centerline
  tile by tile (Amanatides–Woo; each step crosses one grid line, so consecutive tiles
  share an *edge*). Raising narrow widths to a minimum instead was tried and rejected:
  the threshold depends on the line's angle and on its offset against the grid, and even
  3 m still left a gap. Pinned by `tests/navigation.rs`.
- **Ordinary roads do not touch the navmesh.** The grid starts all-passable and
  `fill_from_mapdata` only ever *subtracts* (water, buildings, walls); roads enter it
  solely through the `bridge` and `passage` carves above. Pawns walk on grass and asphalt
  alike. Consequently road **rendering** (`map/roads.rs`, `MeshBuilder::push_ribbon`) and
  road **rasterization** (`Navmesh::set_polyline`, a capsule sweep by
  `distance_to_segment` — round joints by construction) are two independent code paths
  over the same `RoadLine`, and changing how a road is drawn cannot change where anything
  walks. Changing `RoadLine::points` or `width` would change both at once.
- **Rails do not touch the navmesh either, and deliberately so.** `MapData::rails` is
  absent from `fill_from_mapdata` by design, not by omission (`tests/navigation.rs`
  pins it): a rail line runs unbroken across the whole city, so blocking it would slice
  the map in two and `prune_unreachable` would amputate whichever half does not hold the
  portal. Pawns cross the tracks as if they were ground.
- **Building passage** (арка) — a road that runs *through* a building: OSM
  `tunnel=building_passage`, or `covered=building_passage|yes` (both tag styles occur;
  `tunnel=yes` is an underground tunnel and is **not** one). `parse::is_building_passage`
  sets `RoadLine::passage`; the navmesh carves those centerlines passable **last**, after
  buildings and walls, since the whole point is to punch through a block that was just
  filled. Carve width is `min(road width, PASSAGE_MAX_WIDTH)` — the way is usually tagged
  `service` (5 m) but the arch itself is narrower, and an uncapped corridor would eat a
  tile of facade on each side. Tula has ~70 of them, London ~1700; without the carve,
  courtyards reachable only through an arch get sealed off by `prune_unreachable`.
- **Arch rendering** (`buildings/arches.rs::arch_openings` + `push_wall_with_openings`) — the
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
  until the camera arrives; at zoom ≥ `WANDER_DISPATCH_MAX_ZOOM` (0.75 m/px) *no*
  wanderer counts as on screen — a pawn is a dot there, and "in view" would otherwise
  mean half the map, flooding the task pool and the per-frame sort with ~17k peaceful
  requests. Demons and fleeing humans are always dispatched at any zoom.
  **Priority** (`priority::` in `movement/systems.rs`): demons and fleeing humans
  (`URGENT`) go before wandering humans in frame (`WANDER_ON_SCREEN`), within a
  priority nearest-to-camera-center first, capped at `MAX_PATHFINDING_IN_FLIGHT`
  (512). The order only bites when the cap binds — in normal play in-flight sits
  around 100 of 512. The speed panel shows in-flight / queued / avg ms.
- **Repath on the move** — `to_pathfinding` keeps the current path and the
  `MovableStateMovingTag`, so an entity walks its old path while the new one is
  computed; `MovableStateMovingTag` therefore means "has a path **or is coasting**",
  *not* "state is `Moving`". Dispatch and pickup both live in `Update`, so a reply
  costs 2–3 frames — and a frame carries `speed × 1/fps` virtual seconds, so at 30×
  the reply lags by 1–1.5 virtual seconds while a fleeing human repaths every ~1 s:
  the old path routinely runs out before the reply lands.
- **Coasting** (`move_moving_entities`) — an entity whose path is exhausted while the
  state is still `Pathfinding` keeps moving along `Movable::last_direction` (the
  direction of its last step) as long as the tile ahead is passable; a zero vector,
  a wall or the map edge ends the coast (tag removed, as before). Arrival is not
  coasting: state `Moving` + empty path still means `to_idle` — a wanderer must stop
  at its destination. Before coasting, 26–42% of fleeing humans stood at any instant
  at 30× (measured); the reply (`to_moving`) or `PathfindingError` ends the coast.
  When the reply lands, up to `REPATH_TRIM_LIMIT` (4 — coasting drifts 4–6 tiles off
  the request's start tile at flee speed) leading waypoints are dropped while the
  next one is no further than the first — without the trim the first step would be
  backwards; each drop is geometry-gated, the limit only guards corner-straightening.
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
- **SpatialGrid<T>** — uniform grid per marker type (`Demon`, `Human`), 60 m cells
  (≥ the largest search radius, so a radius query is a 3×3 cell walk). Cells hold
  **entities only** — a candidate's position is read live from `SimPosition` through the
  `pos_of` closure every query takes. Storing `Vec2` in the cell would require a
  full rebuild every tick, or positions go stale by up to a cell size and chase/panic
  silently miss. `nearest_in_range_where` — nearest entity passing a filter;
  `for_each_in_cells_around` — raw candidate walk, caller does the exact distance.
- **The human grid is incremental, the demon grid is rebuilt.** Humans (~20 000):
  `On<Add, Human>` / `On<Remove, Human>` observers cover spawn and death/despawn
  (`On<Remove>` fires on despawn too — escape, restart, city switch all funnel through
  it), and `move_moving_entities` moves an entity between cells when a step crosses a
  60 m boundary — an arithmetic compare per mover, hash work only on the rare crossing
  (a wanderer crosses a cell every ~21 virtual seconds), so the cost scales with
  crossings, not with population or how many pawns the camera lets move. Demons (~100):
  full rebuild per tick in `rebuild_demon_grid` is cheaper than bookkeeping, and the
  lunge moves demon `SimPosition` outside the mover system anyway.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s *between*
  walks, zero at spawn so nobody stands around after launch; then 80%
  head to a random building anywhere in the city — long routes, the real pathfinding
  load — and 20% stroll 20–40 m nearby) ⇄ **Flee** (demon within `HUMAN_PANIC_RADIUS`
  60 m; repath every 0.7–1.2 s, step 40–60 m away from the nearest demon). The
  Wander → Flee check (`panic`) is **inverted**: each demon collects neighbors from the
  human grid instead of every wanderer polling the demon grid, so its cost tracks the
  crowd near demons, not the city population. **Flee fan** — a
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

- **UI input never reaches the world** — the panels sit over the map, so a click, drag or
  scroll that lands on one must not also drive the camera or anything in the world.
  `camera.rs::drag_pan` decides *in the press frame* whether the gesture belongs to the UI
  (`pointer_over_ui` over `HoverMap`, the idiom from `zxc/src/input.rs`) and holds that
  verdict until the button is released — a per-frame test would hand the camera the tail
  of every slider drag that runs off the panel. See CLAUDE.md for the rule.
- **Telemetry panel** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight /
  avg ms, entity count, camera. Fixed width + right-padded digits (no jitter).
- **Speed button** (`ui/speed.rs`) — left of that panel, a `Speed <value>` row-button in
  the Buildings-panel style. Left click walks the ladder up and wraps to 1x from its
  top step (`MAX_SIM_SPEED`), right click steps down; green while
  paused. It reads `Pointer<Click>` itself instead of `Activate`, which fires for *any*
  mouse button and would make one right click move both ways.
- **Tree style panel** (`ui/trees.rs`) — bottom-right: shape / foliage / crown details /
  color variance, one button per row cycling through a fixed palette (`bevy_ui` has no
  text input, so hex fields became cycles), plus **slider rows** built by the shared
  `ui/slider.rs::spawn_slider_row` kit — **density** over `TREE_DENSITY_MIN..MAX`,
  **conifer share** over `TREE_CONIFER_SHARE_MIN..MAX` and **noise mix** over
  `TREE_NOISE_MIX_MIN..MAX`. Each `ValueChange` observer quantizes to its step and writes
  `TreeStyle` only when the step actually changes, so one drag rebuilds the crowns
  a handful of times, not once per pixel. The conifer-share and mix rows are
  `Display::None`ed outside `TreeShape::Mixed` (`sync_mixed_row_visibility`) — they mean
  nothing for the other shapes. Writes `TreeStyle`; `map::trees::rebuild_trees` picks the
  change up. Also settable over BRP: `res set TreeStyle .shape '"Conifer"'`.
- **Noise panel** (`ui/noise.rs`) — the conifer-field fbm knobs (`ConiferNoiseStyle`:
  wavelength / octaves / lacunarity / persistence), same slider kit; sits bottom-left
  **above the debug-toggles row** (the right column is already packed with style
  panels) and is `Display::None`ed while the `noise` debug toggle is off — tuning the
  field without the overlay showing it is pointless. Noise mix is deliberately *not*
  here: it is a gameplay look knob, so it sits in the Trees panel.
- **Slider kit** (`ui/slider.rs`) — `spawn_slider_row` (label + value text + discrete
  `bevy_ui_widgets::Slider`), `quantize`, and one `sync_slider_thumbs` for all panels
  (sliders carry the shared `UiSlider` marker; registered once in `UiPlugin`). Callers
  pass their own marker bundles for the value label and the slider to address them in
  their sync systems.
- **Bottom UI columns** (`ui/mod.rs::stack_bottom_columns`, `UiRightColumnSlot` /
  `UiLeftColumnSlot`) — right: Tree rows → Trees → Buildings → Roads → hotkey help;
  left: debug toggles → Noise; both bottom-up. The panels are absolute (`bevy_ui` does
  not stack them), and the columns change height at runtime (Trees grows two rows on
  `Mixed`, Noise exists only with the `noise` toggle), so each panel's `bottom` is the
  summed **measured** height of those below it instead of a hardcoded constant;
  `Display::None` panels are skipped by their `Node.display`, not their last-frame
  `ComputedNode`. `ComputedNode::size` is in *physical* pixels — multiply by
  `inverse_scale_factor` or every offset doubles on a retina screen.
- **Debug toggles** (`ui/debug.rs`) — grid / navmesh / doors / movepath / noise buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay is **one merged mesh** — per-tile entities once cost 330 k entities; the
  noise overlay is one sprite with a CPU-built texture (see Conifer stands). The row also
  carries the two cycling buttons that are not layer toggles — `pathfind:` and
  `position:` — since there is no other row of buttons in the UI.
- **Camera start view** (`camera.rs`) — **`CameraPositionMode`** (`reset | save`, the
  `position:` button, persisted) decides where the camera stands when the world comes up:
  `reset` — the snapped portal at `START_ZOOM`; `save` — the x/y/zoom written into
  **`SavedCameraView`** (persisted, same `camera` settings group) by
  `save_camera_view_on_exit`, a `Last` system that fires on `AppExit` and saves
  synchronously. It must run **after `bevy::window::ExitSystems`**: closing the window
  writes `AppExit` from `exit_on_all_closed`, which is itself in `Last`, so without the
  ordering the save silently ran a system too early and nothing was ever written.
  `track_camera_view` (Update, only in `save` mode) covers the exits no schedule can see —
  macOS Cmd-Q, `brp quit` (its `AppExit` comes from `RemoteLast`, after `Last`), a crash —
  by writing during play, **debounced 1 s after the camera stops and throttled to one
  write per 10 s while it keeps moving**: a drag is dozens of frames, and a per-frame write
  would rewrite `settings.toml` a hundred times per gesture. The debounce runs on
  `Time<Real>` on purpose — first-party `SaveSettingsDeferred` ticks on virtual time, so it
  would never fire while paused and fire 30× early at 30x speed. The view is applied in three places, all through `start_view` + `apply_view`:
  camera spawn (`Startup`, portal *hint* — the snapped position isn't known yet),
  `place_camera_on_world_ready` (`OnEnter(Playing)`) and the `RestartEvent` observer, so R
  puts the camera exactly where an app start would. A world entry that is **not** the
  first one is a city switch and always resets to the new portal.
- **sim_time.rs** — Space pauses, `=`/`-` walk the speed ladder (`SPEED_LADDER`:
  1 → 2 → 5 → 10 → 20 → 30; the button's `cycle_time_scale` wraps to 1x from the top
  step; an arbitrary BRP-written speed snaps to the nearest step on the next press).
  - **SimSpeed** — `{requested, effective, actual}`. `requested` is what the ladder says;
    `effective` is the regulator's command, what reaches `Time<Virtual>` after **fps
    throttling**; `actual` is measured — virtual seconds per real second, averaged over
    `ACTUAL_SPEED_WINDOW` (0.5 s of *real* time, so long frames weigh what they cost).
    `actual` is the only honest one: Bevy clips a frame's virtual delta at `max_delta`, so
    a stall eats simulated time behind the regulator's back. The panel and `is_throttled`
    read `actual`.
  - **Speed ceiling** — Bevy hands `FixedUpdate` at most `Time<Virtual>::max_delta`
    (`MAX_FRAME_DELTA` = 0.5 s, pinned explicitly at startup; was Bevy's default 0.25,
    which put the ceiling at exactly 15x under 60 Hz vsync — zero margin, any fps jitter
    knocked a requested 15x down) of virtual time per frame, so a speed of S is only
    real if `S ≤ fps × MAX_FRAME_DELTA` — 30 at 60 fps, 20 at 40 fps. Above the ceiling the ticks pile into frames, `Update` (path dispatcher,
    input, UI) starves, and humans that finish a route just stand there.
    `throttle_speed_to_fps` closes the loop on measured fps and eases `effective` toward
    the ceiling (`SPEED_SETTLE_RATE` up, the faster `SPEED_DROP_RATE` down). It throttles
    **below 1× too** — under 4 fps even real time is unaffordable — down to
    `MIN_SIM_SPEED` (0.1). The button shows `15x → 8.6x` when limited, and
    `1x → 0.42x` while something (the async northstar build, say) is starving the
    frame.
  - **Requested cap** — `MAX_SIM_SPEED` (30x, the top of `SPEED_LADDER`) is a hard
    ceiling on `requested`, equal to the fps ceiling at a steady 60 fps: the ladder never
    steps past it, and `throttle_speed_to_fps` clamps `requested` itself so a BRP write
    cannot exceed it either. Asking for more than the hardware can hand `FixedUpdate`
    only makes the panel display a number that never happens.
  - Set the requested speed over BRP with `res set SimSpeed .requested N` (clamped to
    `MAX_SIM_SPEED`) — `brp speed` writes `Time<Virtual>` directly and the throttle
    overwrites it on the next frame.
  - **SimClock** — `elapsed`, virtual seconds the *current world* has lived, zeroed on
    entering `PlayPhase::Live` (so map load and warmup don't count, and a city switch
    restarts it). Not wall-clock: it stops on pause and runs `actual`× faster on speedup.
    The panel's first line shows it as plain seconds (`T+8130`), and it is readable
    over BRP as `SimClock`.
  - **Per-tick cost** (`sim/*_ms` diagnostics, 20 000 humans / 100 demons): with the
    entity-only incremental grid and the inverted `panic` the tick sums to ~0.1 ms —
    `flee` ~0.06 > `panic` ~0.02 > `move` ~0.01 > `spatial` (demon rebuild) ~0.004.
    History: `panic` once scanned the demon grid per wandering human (~0.8 ms/tick, the
    speed ceiling); a `DemonDangerMap` boolean prefilter cut it to ~0.15, and the
    inversion replaced the map entirely. At full zoom-out (every pawn moving) the sim
    stays ~0.2 ms/tick — the limiter there is rendering 20k sprites, not the sim.
- **Remembered UI options** (`prefs.rs`) — every UI-settable resource (`DebugGrid`,
  `DebugNavmesh`, `DrawMovePaths`, `PathfindingAlgorithm`, `TreeStyle`,
  `CameraPositionMode`) is a `bevy::settings::SettingsGroup`, so a click survives a
  restart. `SettingsPlugin` reads
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
