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
  All world coordinates are positive. `MAP_SIZE = 5600 × 3700` m.
- **Navtile** — navigation grid cell, **2 m by default, runtime-switchable to 1 m** via
  the `navtile:` cycler in the debug panel (`NavtileBase` in `settings.rs`, persisted in
  prefs; changing it reloads the world like a city switch, except the camera stays where
  it was — same city, same spot under inspection). The live value is a
  process-global atomic read by `settings::navtile_size()` — background threads (navmesh
  fill, entrance generation) have no ECS access; it is written only in
  `OnEnter(Loading)` before the load thread starts. Grid size is derived as
  `MAP_SIZE / navtile_size()` (2800 × 1850 tiles at 2 m); a filled `Navmesh` carries its
  own `grid_size`/`tile_size` snapshot, so stale snapshots (a cancelled northstar build)
  never index against the switched atomic. The northstar chunk scales to stay 50 world
  meters (25 tiles at 2 m, 50 at 1 m) — with the tile-25 chunk a 1 m build explodes from
  ~14 s to ~140 s. Cost of 1 m: northstar build ~14 s vs ~11 s, HPA* ×1.7 CPU,
  +1.6 GB RSS. `grid.rs`: `world_to_tile` / `tile_center`.
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
  left (or after `WARMUP_TIMEOUT` = 10 s, logged as a warning). It counts only what the
  dispatcher will actually serve — `wanderers_dispatched_at_zoom`, the same cutoff
  `dispatch_pathfinding_requests` uses: above `WANDER_DISPATCH_MAX_ZOOM` peaceful wanderers
  get no path at all, so waiting for them meant the loader sat out the whole timeout with a
  counter frozen on 4 000. `WARMUP_GRACE` = 0.5 s is the flip side: with nothing to wait
  for, "no requests yet" must stop meaning "they haven't been inserted yet".
  Reason for the hold at all: all 20 000 humans
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
  resets `DemonSpawner` + `Telemetry` + `SimTick` + `DeterministicRun`, respawns
  population. The navmesh persists — it is filled once per city. Under **Deterministic**
  (see "Determinism") this replays the previous run tick for tick.
- **RestartPending** (`restart.rs`, resource) — "a restart was ordered". The only way to
  ask for one from anywhere but the R key: changing the **world seed** or flipping
  **Deterministic**, whether from the panel or over BRP. `trigger_pending_restart`
  consumes it in `PreUpdate` after `InputSystems`, the same slot the R key uses and for
  the same reason — `on_restart` tears the scene down inside an observer, so triggering it
  from `Update` would kill entities that sibling systems have already queued commands for
  (see CLAUDE.md, "Where a mass despawn may happen"). It always fires
  `RestartEvent { to_portal: true }`: a changed world setting means a *different* world,
  and leaving the camera where it was makes the restart invisible.
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
    marks a piped section: it is **not drawn at all** and, alone among
    watercourses, **does not block the navmesh** — the water runs under the ground and a
    pawn walks over it, so there is nothing to see and nothing to cross. Everything else
    about waterways *does* block; see Navigation.
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
    unreachable. Every candidate point is probed `entrance_clearance()` (= one navtile,
    2 m by default) along the edge's outward normal, and a probe that lands inside another
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
  waterways, alleys, roads, building layers, walls): `MeshBuilder` triangulates polygons via
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
  rail bridges are out of scope. The curb is not just paint: the navmesh blocks the
  same bands (see **Bridge curbs are impassable** under Pathfinding).
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
- **Tree crowns** (`map/trees/crown.rs`, algorithm write-up — `TREE_ALGO.md`) — Watabou-style
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
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** (`bridge=yes` roads) → buildings block → walls block → **building
  passages carve back through them**. Without bridges the Упа river bisects the map and
  no cross-river path exists.
- **Bridge curbs are impassable** — the same two bands the renderer draws
  (`bridge_curb_width`, offset off the centerline by `miter_offsets`, shared with
  `push_ribbon` so the blocked strip matches the drawn one by construction). Over water
  this changes nothing (water already blocks); on dry spans — approaches, overpasses —
  the curb is what stops a pawn from stepping off the deck sideways. Only the two
  longitudinal edges block, the deck ends stay open. All curbs block *before* any deck
  carves — the render layering (curbs under fills) repeated in the grid, so at a
  junction of two bridge ways one way's deck re-carves the other's curb and the bridge
  is never walled across by its own curb. The deck carve is **narrower than the deck by
  a tile diagonal**: a curb-chain tile's center wanders up to half a diagonal (√2 m)
  off the curb centerline — i.e. *into* the deck on a slanted bridge — and a full-width
  carve re-opened those tiles, turning the barrier into a dashed line. Deck
  connectivity survives the narrowing because `set_polyline` always walks the
  centerline chain, the same guarantee thin waterways rely on. A curb tile does
  **not** block unconditionally — OSM cuts one physical bridge into several ways
  (carriageway and its sidewalk are parallel ribbons), so each curb tile records
  its owners (`CurbTile`) and the decision is an **outward probe**: step one
  tile away from the owning way's centerline — if that point lands inside
  another bridge way's ribbon, the tile is an interior seam and stays open; if
  it lands on nothing, the tile is the outer boundary of the whole composite
  and blocks. This survives nominal class widths swallowing a parallel sidewalk
  whole (primary is 16 m by default — a "covered by a neighbour ribbon" rule
  would open *both* of the pair's outer curbs there). A **joining** non-bridge
  road opens the curb its panel covers — joining means sharing a node with a
  bridge way (`JOIN_EPSILON`); a riverbank path passing a few metres *under*
  the span shares no node and opens nothing. The rule only refrains from
  blocking, it never re-opens — an open-by-rule tile over water stays water. After the deck carves a **seal
  pass** restores the barrier where it degraded to corner contact: on a narrow
  (alley-width) bridge the deck centerline chain claims the same tiles as the
  way's own curb chain, the deck wins, and the curb continues one column over —
  touching diagonally. Our A* cannot cut that corner but northstar's
  `OrdinalGrid` (HPA*, Theta*) steps straight through it, the thin-waterway
  hazard again. The seal blocks, of the two open orthogonal neighbours of such
  a diagonal pair, the one **farther** from the owning bridge centerline — the
  deck stays walkable, the gap closes on the outside. Known cost of a
  single-level grid: a street passing *under* a dry overpass gets the curb
  bands stamped across it — passable where the street runs, blocked either side
  of it.
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
  which was already impassable. The one place the fill stops short of the capsule is the
  **culvert portal**: `set_polyline_capped` drops the tiles past the end plane there, so
  the mouth of the pipe — the only dry crossing a channel has — is not plugged by the
  half-disk. Same `water_line_caps` rule the ribbon is drawn with.
- **A rasterized polyline is a 4-connected chain, by construction.** `set_polyline` marks
  tiles whose *center* is within half the width — and that alone is not a barrier: below
  `tile_size · √2` (2.83 m at the default 2 m tile) a slanted band degenerates into tiles touching only at
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
  **Default is HPA\*** — 28× cheaper than flat A* per `examples/bench/pathfinding_bench.rs`
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
  **Only the selected backend is built** (`northstar_wanted`): the hierarchy serves
  HPA*/Theta* on the grid, so with `Algo: Polymesh`, or with a flat grid algorithm
  picked, those 12 s of every core are skipped entirely (measured on Tula: no
  `northstar grid built` line at all). Switching `Algo` back to `Navmesh` or cycling
  `Pathfind` to HPA* starts the build right then — the same lazy shape the polygonal mesh
  has, run from `Update` on a resource change.
- **PathfindingRequest → dispatcher → PathfindingTask** (`movement/`) —
  `Movable::to_pathfinding` only queues a `PathfindingRequest`;
  `dispatch_pathfinding_requests` turns requests into `AsyncComputeTaskPool` tasks
  (polled with `check_ready`). **Visibility gating**: peacefully wandering humans
  OUTSIDE the camera view (×1.2 margin) are never dispatched — their requests wait
  until the camera arrives; at zoom ≥ `WANDER_DISPATCH_MAX_ZOOM` (0.75 m/px) *no*
  wanderer counts as on screen — a pawn is a dot there, and "in view" would otherwise
  mean half the map, flooding the task pool and the per-frame sort with ~17k peaceful
  requests. Demons and fleeing humans are always dispatched at any zoom.
  **Priority** (`priority::` in `movement/pathfinding.rs`): demons and fleeing humans
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
- **pathfinding_bench** (`examples/bench/pathfinding_bench.rs`) — offline comparison of all six
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
- **Poly navmesh** (`navigation/polymesh/` — `build.rs` builds the mesh, `seams.rs` stitches
  the chunks, `path.rs` searches it; panel — `ui/polynav.rs`) — a *polygonal*
  polyanya mesh triangulated from the same vector sources the grid fill rasterizes,
  recovering the fidelity the 2 m grid loses (bridge curbs, narrow waterways). While the
  Polymesh panel is on and the mesh is built, **it is the pathfinding backend** — see
  **Polygonal routing** below. It is on **by default** (`PolymeshDebug::enabled`): the
  polygonal search is the world's navigation, and the grid is the fallback (while the
  mesh builds, and when the Navigation panel is switched back to `Navmesh` by hand).
  `show` defaults to *off* for the same reason — a default-on backend with a default-on
  overlay would bury a fresh install's city under polygon edges. The whole fill order collapses into one boolean
  (`i_overlay` difference):
  union(water ∪ non-culvert waterways ∪ bridge curb bands ∪ buildings ∪ walls) −
  union(bridge decks ∪ joining roads ∪ passages), clipped to the map rect **outset** by
  `MAP_EDGE_MARGIN` (an inset clip would leave a walkable sliver along the map edge for
  paths to sneak around a river; polyanya digests obstacles crossing its outer boundary —
  triangle walkability is a point-in-polygon test of the triangle center), then CDT via
  `polyanya::Triangulation` with agent-radius inflation.
  Deliberate deltas from the grid: obstacle **holes are dropped** (an unreachable pocket
  ≙ what `prune_unreachable` kills), the **diagonal seal pass has no analogue** (patches
  raster corner-contact only), and deck/joining widths are **verbatim** — the grid's
  `±tile·√2` corrections compensate wandering tile centers, which vectors don't have.
  The deck carve is therefore `road.width`, the carriageway the renderer fills, *not*
  the grid's `width + curb`: the curb bands live just outside the carriageway, and a
  carve that wide eats half of the barrier it is supposed to leave standing.
  The grid's **outward probe** — which curb tile is an interior seam of a composite
  bridge (carriageway + its sidewalk way, mapped separately) and which is the outer
  boundary — becomes the direct vector statement of the same intent: each way's curb
  bands **minus the full drawn bands of every other bridge way**. Covered by a
  neighbour ⇒ interior seam ⇒ open; uncovered ⇒ outer edge ⇒ blocks. N differences over
  a few dozen bridges cost less than the single building union.
  **Conditional and async**: nothing builds while the Polymesh panel is off
  (`PolymeshDebug`, persisted — on by default, so the usual path *is* a build on entering
  the world); the build runs on `AsyncComputeTaskPool`
  (`PolyNavmesh` resource: `PolymeshBuild` + generation counter + in-flight task,
  cleared by `city.rs::reload_world` alongside `NorthstarGrid`). `PolymeshBuild`
  carries the obstacle contours next to the mesh on purpose: polyanya stores only
  **walkable** polygons, so without them the overlay could not paint what is blocked.
  A radius-slider step supersedes the in-flight build, and superseding **cancels** it
  through an `Arc<AtomicBool>` — the same machinery `NorthstarGrid` uses and for the same
  reason: the task body is synchronous, so dropping the `Task` throws away the result but
  not the work. Measured on Tula: **~5 s at radius 0, ~20 s at any non-zero radius** (the
  obstacle inflation dominates), and one drag across the slider queues a build per step —
  without the flag five superseded builds ran all cores to completion. Checks sit before
  each long stage (boolean, clip, `as_navmesh`, `merge_polygons`); inside `i_overlay` and
  `spade` there is nowhere to look.
  **Chunked by default** (`CHUNK_TARGET_METERS` = 400 m, capped at `MAX_CHUNKS` = 240
  layers): the map is cut into a grid of polyanya layers stitched along seams computed
  once in world coordinates, and the search runs over the chunk graph. Now that the
  divergence is fixed (see below) the hierarchy is the default, and it wins on both
  numbers (Tula, 500 queries, radius 0.2, `examples/polymesh_bench`): **build 0.31 s vs
  5.72 s** flat — each chunk triangulates from its own small edge set — and **5.66 ms
  mean / 43 ms worst vs 6.18 / 104**, same misses. `QWE_POLYMESH_CHUNK_M` set larger than
  the map returns the flat single layer, which is how "hierarchy's fault" is told apart
  from "geometry's fault".
  **The corridor is the route plus a corner fill** (`PolymeshBuild::corridor`): the
  low-level polyanya query sees only the chunks the level-1 A* walked, every other layer
  blocked — *and* the fourth chunk of each 2×2 block the route turns in. The graph is
  four-connected (an edge is a shared seam **segment**, not a point), so a diagonal trip
  is always a staircase A→B→C; with only those three open the free region has a reflex
  corner, the shortest path must round its vertex, and that vertex is a chunk grid node.
  polyanya's funnel lands on it *exactly* — with a non-empty `blocked_layers` any vertex
  touching a blocked layer counts as a corner — and on screen dozens of paths converge on
  one point and radiate out of it (`Show` + movepath). `examples/polymesh_corner_audit`
  counts it: on Tula (400 queries, radius 0.4) a bend exactly on a grid node fell from
  **40.9 % of paths to 16.2 %** with the corner fill, 7.2 → 3.15 m per bend, path length
  against the straight line 1.090 → 1.061 (a flat mesh gives 1.037). It is paid for in
  open area — the corridor grows from 9.6 to 12.9 chunks and `polymesh_bench` goes
  5.61 → 6.23 ms mean, 45 → 75 ms worst, same misses — and only routes with a turn pay.
  The added chunk is the expensive kind: it sits *on* the straight line to the goal, so
  its polygons carry the smallest heuristic and get expanded first, all of them.
  Opening the whole ring of neighbours instead, or filtering turns by "the straight
  start→goal line touches that chunk", were both measured and rejected (the filter saves
  a few percent of the time and gives back most of the bends).
  **The polyline is then string-pulled** (`smoothed` / `segment_clear`): a waypoint is
  dropped when the straight cut past it lies wholly on the mesh, tested by walking
  polygons — not by sampling, which would step over a gap between buildings. The walk
  crosses seams the way polyanya's own `successors` does (the polygon shared by both
  endpoints of an edge), and it is deliberately conservative: an ambiguous crossing
  (exactly through a vertex, a start that never sat on the mesh) counts as blocked.
  Two subtleties cost a full debugging round each, both because **every waypoint is a
  mesh vertex**: the walk must start from a point a centimetre *along* the segment (a
  vertex belongs to several polygons, and the one localisation returns can be behind the
  cut — the walk then finds no exit and reports "clear" without moving; that let 10 % of
  smoothed segments run through blocks, one for 3.2 km), and a crossing at the very end
  of the segment is an arrival, not an exit. Corridor and smoothing fix *different*
  things and are both kept: the corridor shortens the route (1.090 → 1.061), smoothing
  removes the bend that remains (16.2 % → 5.1 % of paths, 20 bends over 396 paths) and
  costs 0.3 ms of the 6.5 ms mean.
  **Seam vertex sets are allowed to differ** between two neighbours, and the stitch is
  written for that. Only the chunk *outline* is global (`seam_points` — every crossing of
  an obstacle contour with a grid line, computed once for the whole map); the obstacles
  themselves are clipped, simplified and triangulated per chunk, so a half-metre slit
  between inflated contours can stay open in one chunk and close in its neighbour, and
  spade can split a boundary edge at an intersection only one side knows about. A vertex
  without a partner is therefore normal (Tula: 0–9 per map depending on radius, logged as
  `seam vertices face a wall on the other side`) — an earlier `debug_assert` demanding
  zero was measuring an invariant that never held, and it killed the build task whenever
  the radius slider landed on the wrong value.
  What is *not* allowed is a **one-way seam** (`verify_seams`), and `unstitchable` keeps
  it impossible by construction: an unpaired vertex is dangerous exactly when it sits
  strictly inside a segment the blind side keeps as a whole edge **and** the rich side
  holds both ends of that segment in one polygon — then stitching would hand the
  neighbour's edge a crossing its own two halves cannot answer. Such a segment is left
  unstitched (both ends dropped, since stitching addresses vertices, not edges), and in a
  debug build that is a **panic**: the mesh saved itself, but the geometry is broken and
  the dev build says so at once. The message is written to be read by an agent handed the
  log — what broke, what it costs, which functions to fix, what *not* to do, and the
  offline repro (`examples/polymesh_seam_audit -- <radius>`, which prints both counts per
  radius with coordinates). The sample it names is collected under `debug_assertions`
  only. On Tula the assert fires on none of the nine slider radii — usually the extra
  vertex splits the polygon too, so no shared polygon survives and nothing one-way can
  form.
  The build ends with **`mesh.bake()`**, strictly after `merge_polygons` (which starts by
  un-baking). Baking is what makes the mesh queryable at scale: without it point location
  is a linear scan over every polygon, twice per query, and an unreachable goal burns the
  full `polygons.len() * 10` budget instead of failing at once on the island check.

- **Polygonal routing** (`polymesh::find_path_polymesh`, dispatched in
  `movement/pathfinding.rs`) — with the Polymesh panel on, `dispatch_pathfinding_requests`
  routes through the polygonal mesh and the `PathfindingAlgorithm` cycler is bypassed.
  **While the mesh is still building** (5–20 s) `Pathfinder::polymesh_build()` is `None`
  and the grid serves the request — the same fallback shape HPA* uses while
  `NorthstarGrid` builds.
  A path is a **world-space polyline** (`Movable::path: VecDeque<Vec2>`,
  `PathfindingResult::path: Option<Vec<Vec2>>`), always including its start point: the
  consumer drops the first waypoint and reads a single-element path as "already there".
  polyanya's `Path::path` omits the start, so `find_path_polymesh` prepends it; the grid
  backends still return tiles and the dispatcher maps them through `tile_center`, so both
  look identical downstream.
  The **goal stays a tile** (`MovableState`, `PathfindingRequest`,
  `MovableReachedDestinationEvent`): it is the identity that discards a stale answer and
  the arrival test. Only waypoints became metric. The polygonal query therefore starts at
  the pawn's real `SimPosition` and ends at `tile_center(end_tile)`.
  A **missed goal is `PathfindingError`, not a fallback**: with a non-zero agent radius a
  target picked by tile passability can land inside an inflated obstacle, and polyanya
  only snaps endpoints within `search_delta * search_steps` (0.2 m). The cost of that
  choice is visible in the speed panel as `answers: N/frame, X % failed` — a pawn whose
  own position is off-mesh fails *every* repath and stands still, so the number is worth
  watching. It is computed from **two** diagnostics, `pathfinding/answered` and
  `pathfinding/failed`, both written every frame including zeros, and shown as the ratio
  of their averages: a percentage computed per frame and then averaged would count
  frames instead of answers (a lone late failure makes its frame read 100 %) and would
  freeze on its last value the moment answers stop. The denominator is on screen for the
  same reason: 100 % of 0.7 answers a frame is a trickle of hopeless repaths, not a dead
  navmesh.
  Coasting and the demon lunge's `line_of_sight` stay **grid** tests: they are cheap
  guards against walking into a wall, not path searches.
  Two knobs exist only because the default fails at city scale, both measured on Tula
  (40 199 polygons after merging, 20 000 pawns, 30×):
  - **Endpoint tolerance** (`SEARCH_DELTA * SEARCH_STEPS` = 1 m, half a navtile). polyanya
    defaults to 0.2 m, exactly the agent radius, and that is not enough: 80 % of wander
    targets are building outline vertices, and the grid calls a tile passable when its
    centre clears the polygon by a centimetre. **96 % of requests failed** with the
    default against 3.5 % on the grid; at 1 m it is **0.6 %**.
  - **`MAX_POLYMESH_PATHFINDING_IN_FLIGHT`** equals the grid's 1024 — an earlier low cap
    tried to contain runaway memory and instead stalled the whole dispatcher.
  The same arithmetic sets the **ceiling of the agent radius slider**
  (`POLYMESH_AGENT_RADIUS_MAX` = 0.6, range 0.2–0.6): the tolerance rescues a goal that
  sits inside the inflation by less than a metre, and the inflation grows with the radius,
  so the two meet. Misses over the ladder (`examples/polymesh_miss_audit`, 600 queries,
  the seeded set of `polymesh_bench`): 0.5 % at 0.2–0.3, 1.0 % at 0.4, 1.7 % at 0.6,
  2.7 % at 0.7, then the cliff — 6.7 %, 12.3 %, **20.7 % at 1.0**, where 47 % of goals no
  longer sit on the mesh at all and 88 % of the misses are the goal walled in, not a
  search that ran out. Physics agrees with the number: a human is 0.5 m across
  (`HUMAN_SIZE` is the doubled, readable size), so 0.6 is already twice the real body.
  The audit is the tool for that question — it splits each miss into an endpoint that
  never sat on the mesh and one that sat in another connected component.
  **Divergence, resolved twice over.** A single search used to allocate unbounded memory
  (flat ~3 GB, then past 17 GB in seconds, OS kill): polyanya's iteration budget caps
  only queue *pops* while `successors` pushes fans of nodes unchecked. Fixed at the root
  in the **vendored** `vendor/polyanya` (a `[patch.crates-io]` path dep, edits marked
  `QWE:`): exact node repeats — same polygon, root and interval — are deduplicated,
  killing the cycle where a corner vertex on a seam's collinear edge chain spins
  equal-cost nodes around its polygon ring forever (root_history only drops strictly
  worse nodes). Belt and braces on top: `bounded_path` polls `get_path` under an external
  work budget scaled to the open polygon count (~10 pops each, min 4096 polls), a
  `NotFound` returns immediately instead of idling out the limit, and in debug builds an
  exhausted budget or a one-way seam (`verify_seams`) is a panic, because either means
  broken mesh geometry. Measured after the fix: 2000 chunked queries, 0.7 % missed,
  5.3 ms mean, 42 ms worst, flat memory.

## Determinism

- **World seed** (`rng.rs::WorldSeed`, remembered by `prefs.rs`, panel row *Seed* in
  World) — the one number every simulation draw descends from. It governs the
  **simulation**, not the map: OSM is parsed from a cache file, and trees and entrances
  are seeded by their own coordinates (`map/trees/crown.rs::Lcg`,
  `entrances::lcg_seeded_by`), so those are already reproducible without it. Capped at
  `i64::MAX` (`MAX_SEED`) — `toml` cannot store more, and the seed has to survive a
  restart of the app. The *new* button rolls a 9-digit one so it can be read off the
  screen and typed back in.
- **Seed derivation** — `seed_for(world_seed, domain, key)`, two rounds of splitmix64.
  Nothing stores live RNG state, so **a restart has no RNG to reset**: every stream is
  re-derived. `RngDomain: Population | Human | Demon`.
- **Decision stream** (`rng.rs::WanderIndex::next`, on humans *and* demons) — a `SimRng`
  is built per *decision* and dies with it, seeded from `(PawnId, decision number)`. The
  seed is therefore the pawn's **observable identity plus which choice this is**, never
  the history of a stream. Draws do not depend on query iteration order, on how many
  neighbours drew before it this tick, or on how many draws the pawn's previous decision
  happened to consume. Each of those has bitten: a single shared generator collapses under
  any reordering (`panic` draws its repath period while walking a `HashSet<Entity>`, whose
  order differs between runs), and a live per-pawn stream shifts under one added
  `rng.random()` inside a decision.
  Consequence worth having: pawn K's k-th decision draws the same numbers **whenever it
  happens**, so it is the same with the toggle on and off. In normal mode that makes the
  *opening* reproducible — measured across two app launches, 99.8 % of the population
  picked an identical first target; the stragglers are the pawns near the camera that had
  already reached it and made a second decision. It does **not** make the run reproducible
  there: targets are chosen relative to the current position, and positions diverge with
  frame timing. Full replay is what the toggle is for.
  Position is deliberately *not* an input, tempting as it is: `move_moving_entities` sets
  `sim_position.0 = target` on arrival and waypoints are `tile_center(...)`, so a pawn
  standing on tile `T` is there bit-for-bit every time. `(pawn_id, tile) → target` would
  be a deterministic function, and every trajectory of one on a finite set eventually
  closes into a cycle — within minutes each human would pace a fixed loop forever. The
  decision number only ever grows.
- **PawnId** (`rng.rs`) — a pawn's spawn ordinal within its species and run (humans
  `0..HUMAN_COUNT`, demons `DemonSpawner::spawned`). Used wherever a stable "personal
  number" is needed — the RNG seed key, the flee-fan angle (`personal_spread`), the
  separation axis (`coincident_direction`), the dispatcher tiebreak. **Never `Entity`**:
  entity indices are recycled in a different order after a restart (the free list depends
  on who was eaten in the previous run), so an `entity.index()` hash would drift between a
  run and its replay under an identical seed.
- **SimTick** (`determinism.rs`) — the step counter, incremented at the head of the
  `FixedUpdate` chain. **The unit of replay**: world state is a function of
  `(seed, settings, SimTick)`. Not the same as `SimClock`, which counts virtual seconds
  and loses whatever `max_delta` discarded on a long frame. Compare states by tick, never
  by wall clock.
- **Deterministic** (`determinism.rs::Determinism`, panel toggle) — gates *scheduling*,
  not the dice; the RNG work above is unconditional. Off: today's behavior. On: wander
  target picking runs in `FixedUpdate`, pathfinding answers land on a fixed tick, the
  dispatcher stops looking at the camera, the navigation backend is frozen, and pawn
  separation is off — the Navigation panel's `Separation` row reads `off`, dimmed and
  unclickable, rather than a toggle that flips a resource nothing reads. A run is
  deterministic or not from tick 0, so flipping the toggle (like changing the seed) orders
  a restart via `RestartPending` — and that restart carries `RestartEvent { to_portal:
  true }`, i.e. the double-`R` camera reset. A changed seed or a flipped toggle is a
  *different world*, not the current one from the top; without the camera move nothing on
  screen changes and the setting reads as having done nothing.
- **Frame rate does not matter.** `Time<Fixed>`'s step is constant regardless of fps and
  of `SimSpeed`; the answer to a path query waits for its tick; and everything left in
  `Update` only draws. A slow machine therefore replays the same run more slowly — it does
  not replay a different one.
- **Retire tick** (`RetireAt`, `PATHFINDING_RETIRE_TICKS = 8`) — a request issued on tick
  `T` is applied on exactly `T + 8`, whether or not the search finished; if it did not,
  `apply_pathfinding_results` waits on it (`block_on`). That wait *is* the mechanism: it
  removes "when did the OS get around to it" from the simulation. Eight ticks ≈ 125 ms at
  1×, which is what today's `request → dispatch → task → collect` pipeline already costs,
  so pawn behavior is unchanged. It does **not** set throughput — the dispatch rate below
  does; K only buys a batch wall time before its join, and pays in path staleness. The
  constant must not scale with `SimSpeed` — that is user input and may not influence
  replayed content.
- **Dispatch rate** (`PATHFINDING_WANDER_UNITS_PER_TICK = 128`,
  `PATHFINDING_URGENT_UNITS_PER_TICK = 64`) — how much leaves the queue each tick,
  measured in *predicted search cost*, not in requests. A request costs
  `1 + chebyshev_tiles / PATHFINDING_UNIT_TILES` (integer: a float sum would depend on
  iteration order). Measured on Tula with polymesh, a 20–40 m stroll costs **0.26 ms** and
  a cross-city errand **5.3 ms** — 20× apart, so a budget in requests cannot fit both. The
  rate is derived from what the pool chews per tick: 5 `AsyncCompute` threads (`main.rs`
  overrides the policy to `percent: 0.5`) × 15.625 ms × ~45 % ≈ 35 ms of CPU, which is
  either ~128 strolls or ~6 errands.
  **Never reuse `MAX_*_IN_FLIGHT` here.** Those cap *concurrent* searches per frame in the
  normal dispatcher, and they are sized for a queue the visibility gate has already
  stripped by ~97 % (~9 searches/frame actually go out). With no gate, all 20 000 pawns
  file requests, and the same 1024 meant up to 65 000 searches per real second: the first
  errand wave (16 000 requests ≈ 85 s of CPU) collapsed the frame to **2.6 fps for ~19 s**
  around T+8…T+20. With the rate in place the same wave queues up to ~11 000 requests,
  drains over ~30 virtual seconds, and **holds 60 fps and `actual = 1.00` throughout**. A
  long queue is the *normal* state of this mode, not a jam.
  At 30× it still costs: tick rate scales with `SimSpeed`, so the mode settles around
  **2–5× against 13.4× with the toggle off** — by design, and the failure rate is visible
  for the same reason (~2–5 % against 0 %: the sample is the whole map, not the easy
  on-screen subset; a failed search just sends the pawn to pick another target next tick).
- **RequestedAt** — the tick a request was filed; the FIFO key of the deterministic
  dispatcher, whose key is `(requested_at, species, pawn_id)` — all integers, since ties
  between floats have no defined order. **Species precedes the number** because `PawnId` is
  only unique *within* a species, and the urgent queue mixes demons with fleeing humans.
  A small rate plus this FIFO *is* the deterministic replacement for the camera gate:
  distant pawns still wait longer, but reproducibly rather than because the player looked
  away. The camera does not appear in it at all.
- **DeterministicRun** (`determinism.rs`) — the navigation backend frozen for the run
  (algorithm + northstar grid + polymesh), snapshotted on entering `Live` and on every
  `RestartEvent`. northstar and polymesh finish building at some moment of *real* time; a
  live `Pathfinder` would switch backends mid-run, and a replay would switch on a
  different tick. In this mode warmup waits for the wanted backend instead
  (`NavigationBuildPending`, `loading.rs::poll_warmup`), which costs ~11–14 s on first
  entry into a city on HPA — deliberately. Restarts do not pay it.
- **No pawn warmup** — once the backend is built, this mode enters `Live` immediately
  instead of waiting for on-screen pawns to be routed. There is nothing to wait *with*: the
  whole pipeline lives in `FixedUpdate`, which is paused for warmup, so the counter could
  not move and the screen burned the full `WARMUP_TIMEOUT` (`warmup timed out with 301
  pawns still routing`). And nothing to wait *for*: "pawns on screen" is a camera notion,
  and the number of ticks before entering the world may not depend on where the player
  looks. Unpausing instead is not the fix — the world would move behind the loader, pawns
  would reach their targets and file new requests, and the counter would oscillate near
  zero forever. The crowd does not stand still at the reveal either: the dispatch rate
  starts the population in a wave over a couple of seconds.
- **NeedsWanderTarget** (`movement/components.rs`) — marker held exactly on `Idle` and
  `PathfindingError`, maintained only by the `Movable` transitions. Target picking moves to
  `FixedUpdate` in this mode, i.e. ~30 runs per frame at 30×; without the marker each run
  would scan all 17 000 wanderers to find the few thousand standing ones.
- **The replay contract** — 1:1 holds only while `DemonStyle` / `HumanStyle` /
  `SeparationStyle` / the algorithm / the navtile size are left alone mid-run. Sliders are
  simulation input. Not enforced by code.
- **Not claimed**: float reproducibility across machines or compilers; replaying a run
  made with the toggle *off*. `bevy_northstar` builds its grid with rayon, so cross-process
  HPA replay is unaudited — within one session the grid outlives R, so restarts are safe.

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
- **Rescue** (`movement::rescue_from_impassable`) — a pawn standing on an impassable tile
  moves to the nearest passable one (`nearest_passable_tile`, ring search capped at
  `RESCUE_SEARCH_TILES` = 16 tiles), both ends of the interpolation are set to the new
  point and the stale path is dropped (`to_idle`). Ways in exist by construction: the
  spawn sifts tiles but stands the pawn on a tile centre whose own corner may already be
  inside a house (fill marks a tile by its centre), the polygonal mesh calls passable
  what the grid does not (contours inflated by the agent radius), coasting and the demon
  lunge move `SimPosition` past the path. Fixing each entrance separately is pointless —
  the end state is one and the same, and it is terminal: behaviour picks a target, the
  search finds nothing, behaviour picks another, forever.
  **The trigger is a failed search**, not a clock (`listen_for_pathfinding_tasks`, the
  `result.path == None` branch). That is the only signal a stuck pawn raises about
  itself, and it selects exactly the ones worth rescuing: flat A* does not test the start
  tile at all, so a pawn a tile or two inside a house walks out on its own successors;
  polyanya snaps the start onto the mesh; `None` comes back only when there is really no
  way out — all eight neighbours impassable, or a start belonging to no chunk of the
  northstar hierarchy. Cost is one index into the passability `Vec` per failed answer
  (~45 a frame on Tula at 31 % failures), and the ring search runs only for those
  actually walled in. A periodic scan over all 20 000 pawns would do the same work
  thousands of times over for nothing.
  **What counts as free is the active backend** (`Walkable`): the grid tile first (an
  index into a `Vec`), and — while the polygonal mesh is built and selected —
  `PolymeshBuild::contains`, a layer-hinted `point_in_mesh`. The mesh is the stricter of
  the two: its contours are inflated by the agent radius, so a tile that clears the grid
  can be inside an obstacle on the mesh.
  `rescue_trapped_entities` is the same check as a full pass, and it runs in exactly one
  place: **every completed mesh build** (`polymesh_rebuilt` watches
  `PolyNavmesh::generation` — `resource_changed` would also fire when a build merely
  starts). That is the only moment passability changes under pawns already standing. It
  logs `rescued N entities` with its own duration when it moves anyone.
  There is deliberately **no scan on entering the world**, though it looks called for: by
  then the grid is final and `spawn_population` picked its tiles with the very same
  `is_passable`, so the pass would re-test the predicate the spawn had just applied and
  could not find anyone (measured live on Tula: zero rescued). No mesh exists at that
  moment either — its build is async and starts in the same `OnEnter`, and the previous
  city's mesh is dropped by `city::reload`.
- **SpatialGrid<T>** — uniform grid per marker type (`Demon`, `Human`), 60 m cells
  (≥ the largest search radius, so a radius query is a 3×3 cell walk). Cells hold
  **entities only** — a candidate's position is read live from `SimPosition` through the
  `pos_of` closure every query takes. Storing `Vec2` in the cell would require a
  full rebuild every tick, or positions go stale by up to a cell size and chase/panic
  silently miss. `nearest_in_range_where` — nearest entity passing a filter;
  `for_each_in_cells_around` — raw candidate walk, caller does the exact distance;
  `for_each_in_rect` — the same raw walk over cells overlapping a rect (the viewport,
  for separation).
- **The human grid is incremental, the demon grid is rebuilt.** Humans (~20 000):
  `On<Add, Human>` / `On<Remove, Human>` observers cover spawn and death/despawn
  (`On<Remove>` fires on despawn too — escape, restart, city switch all funnel through
  it), and `move_moving_entities` moves an entity between cells when a step crosses a
  60 m boundary — an arithmetic compare per mover, hash work only on the rare crossing
  (a wanderer crosses a cell every ~21 virtual seconds), so the cost scales with
  crossings, not with population or how many pawns the camera lets move. Demons (~100):
  full rebuild per tick in `rebuild_demon_grid` is cheaper than bookkeeping, and the
  lunge moves demon `SimPosition` outside the mover system anyway.
- **Separation** (`movement/separation/`, toggle in the Navigation panel, persisted)
  — soft pairwise anti-overlap: pawns on screen keep their body radii (`HUMAN_BODY_RADIUS`
  0.585 m / `DEMON_BODY_RADIUS` 1.17 m) apart — deliberately **larger** than half the
  sprite, so a resting pair leaves a visible gap (1.17 m against a 1.0 m `HUMAN_SIZE`).
  At the earlier 0.45 m the rest distance was *narrower* than the sprite and a correctly
  separated crowd still drew as a solid mosaic. Deliberately local and cosmetic, four
  gates in order: **the mode** (`separation_runs`) — no separation under determinism, and
  none on the grid backend either (`PolymeshDebug::enabled` off): grid waypoints sit in
  navtile centers and the walk puts a pawn back on them every step, so a separated pair
  is re-collapsed by the next tick and all the mechanism adds is jitter and holds.
  Personal space presupposes metric waypoints, i.e. the polygonal mesh. It is the
  *toggle*, not mesh readiness: while the mesh builds the grid serves the requests, but
  blinking separation over that transition is worse than half a second of the old
  behavior. The panel treats this exactly like determinism — the `Enabled` row
  reads `Off`, dimmed and unclickable (`separation_allowed_by_mode`, the one rule shared
  by the schedule and the panel), and `SeparationHolds` is cleared as soon as the mode
  turns the run off, or pawns held by the last run would stay slowed forever.
  Then: the toggle; **once per rendered frame** (it lives in `FixedUpdate`
  right after `move_moving_entities` — the only point where the tick's positions are
  final and the snapshot is already taken, so the push reaches the screen through
  interpolation — but at 30x that schedule runs ~1920 ticks/s and even 0.03 ms per tick
  would eat ~6% of a real second; ticks between runs only accumulate virtual dt); **zoom
  below `SEPARATION_MAX_ZOOM`** (same 0.75 as the wander-dispatch cutoff — farther out a
  pawn is 1–2 px and overlap does not read). Candidates come from both coarse grids via
  `for_each_in_rect` over the viewport (`VIEW_MARGIN` slack), then a throwaway fine grid
  (`SEPARATION_CELL` 2.4 m — tied to the radii, it must exceed the largest sum
  demon+demon 2.34 m or a pair can fall outside the 3 × 3 scan; head+next linked lists in
  `Local` buffers — no steady-state
  allocations) resolves pairs: each pair sheds `SEPARATION_RATE` (8/s) of its overlap per
  virtual second, clamped to `SEPARATION_MAX_STEP` (0.3 m) per run, split by mobility
  (human 1.0, demon 0.25, devouring demon 0 — it pushes but never moves off its corpse).
  Three rules keep the push from fighting the walk it is correcting, each fixing a
  symptom that was visible on screen:
  **only across the heading** (`across_heading`) — a moving pawn is never displaced along
  its own path, because the longitudinal part read as a follower reversing for a step,
  and summed into a whole jam rotating like a carousel;
  **the pawn behind gives way** (`shares`) — for a pair on roughly the same course the
  follower takes the entire correction and the leader none, instead of the leader being
  shoved in the back by someone who caught up;
  **a walker squeezes past a stander** (`pass_squeeze`, `SEPARATION_PASS_SQUEEZE` 0.6) —
  in a pair where exactly one is walking the rest distance shrinks to 60 % for as long as
  the pass lasts; two standers and two walkers keep the full distance. This is what makes
  the destination-slot lattice passable at all: its step is 2.0 m while a walker needs
  `2 × rest` = 3.6 m of clearance between two settled pawns, so inner slots were
  unreachable except *through* bodies. Squeezing the **pass**, not the crowd, is the whole
  point — the earlier `compress` knob shrank by neighbour count, i.e. hardest exactly
  where the crowd stands still, and a settled crowd ended up permanently overlapped (93 %
  of a pawn's time inside another body on the lab's funnel);
  **a fifth of pawns dodge left** (`left_share`, `SEPARATION_LEFT_SHARE` 0.2) — the side
  is personal and stable (a `PawnId` hash, like the coincidence axis). One side for
  everyone produces two pictures that do not occur in life: a same-way flow congealing
  into a single column, and pawns that failed to reach a dense crowd orbiting it as one
  carousel. Measured best on the lab's street: the lowest time-in-separation of anything
  tried, and the only knob that actually widens the flow;
  **head-on pairs step right** (`sidestep`, strength `SeparationStyle::sidestep`) — two
  pawns walking straight at each other have their pair axis collinear with both
  velocities, so the plain correction has no lateral component at all and they lock
  together until an outside asymmetry frees them. With the across-heading rule this is
  the *only* thing that resolves a head-on pair, so it must stay above zero;
  **a blocked pawn steers aside** (`SeparationSteer`, strength `SEPARATION_STEER` 1.0) —
  the push moves a **position** while the walk immediately carries the pawn back toward
  its goal, so the two forces cancel and the whole result goes into distance walked.
  Steering turns the **heading** of that same walk instead: full speed kept, the pawn
  arcs around and never "returns". A symmetric head-on flow does not disperse without it
  at all — the pair axis is collinear with both courses, so no pair has any lateral
  component (the lab measured a spread of exactly 0.00 m at every `rate`, `hold` and
  `sidestep`). Its side is the same personal one as `left_share`, and it is released
  within `steer_release` of a waypoint, or a pawn circles the point instead of passing it;
  **a blocked pawn eases off** (`SeparationHolds`, share `SeparationStyle::hold`,
  default `SEPARATION_HOLD` **1.0 — i.e. no easing off at all**, since steering solves
  the same problem better; every fraction below 1 measurably ruins convergence) — a
  *human* whose heading points into an overlapped
  neighbour that is **standing or oncoming** (only there is pressing futile — holding on
  any touch made whole same-way flows crawl at the hold fraction, companies of
  co-travellers strangling themselves) walks at a fraction of its speed until the next
  separation run, which
  collapses the walk-vs-separation equilibrium overlap from `speed / SEPARATION_RATE`
  (~0.35 m, a frozen jittering clump) to `hold ×` that (~0.07 m, invisible); 0 would be
  a full stop and makes dense crossings move in stop-motion jerks. Demons are never
  held (a chase must close in; the crowd flowing around a demon is already expressed
  by mobility). The hold set is the one deliberate breach of "cosmetic": it feeds back
  into `move_moving_entities`, which also grants a held pawn **arrival** when it is
  within the rest distance of its goal — the blocking body will not let it any closer,
  and without the grant it would shove at that body forever. (Arrival on an exhausted
  path is likewise forgiven within the rest distance — separation may push a pawn off
  its final tile at the last moment.) The set is rebuilt from scratch each run and
  cleared by the toggle, the zoom gate and world entry, so under determinism it is
  empty from tick 0 and movement never depends on it.
  Coincident positions split along a deterministic per-entity hash axis (the
  `personal_spread` trick). A push into an impassable tile is dropped (`rescue_*` only
  catches failed path searches, it would never find a pawn squeezed into a wall); a push
  that crosses a 60 m cell boundary re-inserts the human into its grid. **Lunging demons
  (`DemonLungeTag`) are exempt entirely** — the lunge writes `SimPosition` itself and
  must close to `KILL_DISTANCE`, which is *smaller* than the demon+human radius sum;
  separation fighting it would starve kills. Corpses are outside by construction (no
  `SimPosition`). The sim is knowingly camera-dependent — the user's viewport-only
  optimization accepts that; off-screen crowds still stack and pay nothing.
  `sim/separation_ms` measures **per run** (~60/s), not per tick like its `sim/*_ms`
  neighbours. Reproduced and measured on demand by `examples/demos/crowd_demo.rs` — a
  windowed scene running the real `separate_pawns` on an empty navmesh, with the crowd
  arranged into the cases that are otherwise waited for (a pile, a funnel, counter-flowing
  columns, a walled corridor, real wander AI), a body-radius gizmo per pawn and a live
  count of overlapping pairs. It navigates the way the game does — `find_path_polymesh`
  over an empty `MapData`, on the default-on polymesh backend, because separation only
  exists there; the scenario's corridor walls are therefore written into `MapData::walls`
  as well as into the grid, and switching scenarios rebuilds the mesh. A query that finds
  no path leaves the pawn standing and ticks a `path misses` counter — falling back to a
  straight line would walk it through the wall and read as a working scenario. There is
  deliberately no key to switch the scene to the grid backend: separation does not run
  there, and that is what the scene is for. Two traps that scene made visible and any
  measurement of separation has to respect: **count only pawns inside the camera rect** (off-screen ones
  are never separated by design, and including them makes on/off indistinguishable), and
  **allow a millimetre tail** — the solver is soft, so a converged crowd still reports
  pairs a few mm inside the radius sum. Two more the scene added later, both of which
  had silently voided every earlier comparison: **a scenario's spawn spacing has to be
  re-checked against `HUMAN_BODY_RADIUS`** (the columns kept a 1.2 m step through the
  0.45 → 0.9 m radius change and spawned already overlapping, so the run measured
  recovery from the spawn, not flow), and **lateral spread must be measured against the
  crowd's own mean, not the map centre** (goals sit in navtile centres, so a
  centre-relative figure is a constant and reads the same with separation off).
- **The separation lab** (`SeparationLab`, `SeparationStats`, `SeparationSteer`;
  `tools/separation_lab/`, findings in its `REPORT.md`) — runtime knobs for the parts of
  separation the game fixes in constants, so the crowd demo can sweep them.
  Deliberately **not** a `SettingsGroup`: it is a measuring rig, not a user choice, and
  its default reproduces the shipped behaviour exactly (`rate` / `max_step` equal to
  their constants, everything else zero, i.e. the added branches do not execute). What
  it made visible: in a symmetric head-on flow the pair correction is collinear with
  both headings, `sidestep` is gated off by `alone`, and so **no lateral force exists at
  all** — the crowd stays a strictly one-dimensional chain and no value of `rate`,
  `hold` or `sidestep` changes anything. `SeparationSteer` is the answer that measured
  best: instead of displacing a blocked pawn, the run hands it a lateral direction and
  `move_moving_entities` bends the *walk* by it, so the pawn keeps full speed and rounds
  the obstruction instead of fighting its own path. Its one trap is worth remembering —
  `Movable::last_direction` must stay the **desired** heading, because the steer side is
  the right normal *of that heading*, and writing the bent course back turns the pawn
  further right every frame until it circles in place.
- **Destination slot** (`movement/destination.rs`) — the reservation that stops two pawns
  from being aimed at the same point. A **slot** is a `k × k` block of navtiles,
  `k = ceil(rest distance / navtile_size())`, claimed by one pawn
  (`DestinationClaim`, reverse-indexed by the `DestinationClaims` resource); its goal is
  strictly the block's **centre** tile, so the goals of neighbouring slots sit exactly
  `k · navtile` apart — never less than the rest distance, for any combination of
  `NavtileBase` and the `HumanStyle::body_radius` slider (the Navigation panel's `Slots`
  group, and the crowd demo).
  The radius lives with the **human**, not with separation, precisely because slots read it
  too and they run even when separation is toggled off — while it sat in `SeparationStyle`
  the panel printed `off` under determinism and the knob went on reshaping the slot
  lattice. Without slots, separation has no
  way out at all: `move_moving_entities` only pops a waypoint when the tick's travel
  budget covers the remaining distance, and a pawn pressing into a taken point is pushed
  back exactly as far as it steps, so overlap parks on the equilibrium
  `HUMAN_WALK_SPEED / (SEPARATION_RATE × share)` = 0.70 m and stays there — the pair
  either orbits (with the sidestep) or stands and jitters (without it), and a crowd that
  reaches a shared point never settles. Why a block and not a tile: one-per-tile only
  guarantees a non-overlapping resting crowd while `2 × radius ≤ navtile_size()`, which
  `NavtileBase::M1` (1 m tiles against a 1.8 m rest distance) and any radius above 1.0 m
  break. Why not the user's fractional lattice: every goal here is navtile-keyed
  (`MovableState`, `PathfindingRequest::end_tile`, the stale-answer filter, the arrival
  test, `tile_center(end_tile)` in the polymesh), so a point that is not a tile centre is
  a point no pawn can be said to have reached. The centre tile is fixed on purpose — let a
  block pick any passable tile in itself and two neighbouring blocks pick adjacent
  corners, which is the very thing the block exists to prevent; the price is that a block
  with an impassable centre goes unused and the pitch rounds up to a whole tile (up to
  ~2× the rest distance), so a crowd parks a little sparser than strictly needed. A taken
  slot moves the goal outward by ring search (`nearest_tile_where`, bounded by the
  `SlotSearch` resource, `CLAIM_SEARCH_METERS` 16 m by default and a slider in the crowd
  demo — deliberately not a `SettingsGroup`, it is a tuning bound and the demo must not
  write the game's config). Nothing free inside the bound is the one branch where the old
  pathology returns in full: the goal stays the shared unclaimed tile and the pawn presses
  into it forever, exactly as before slots. That is still better than refusing a target,
  which would park the pawn for good — and with the bound on a slider the branch is
  visible on demand (drop `Slot search` to 2 m on the funnel and the tail of the crowd
  collapses onto one point). A pawn that finds nothing also loses its previous claim, so a
  saturated crowd churns reservations.
  The claim is **not** released on arrival: a pawn standing on its slot is exactly the
  occupancy being modelled. It moves on the next target selection, and is released by an
  `On<Remove, DestinationClaim>` observer on despawn (escape, restart, city switch) and by
  the corpse strip in `demon/behavior.rs`. Hook point is a single system,
  `assign_destination_slots` over `Added<PathfindingRequest>`, registered in both
  dispatcher chains — human wander, demon wander and the test walker are all covered
  without touching their behaviour systems; the `Update` registration needs an explicit
  `.after(human::pick_wander_targets)`, or the request reaches the dispatcher unslotted
  every so often. **Chase is excluded** (a shared goal is the pincer, by design) and so is
  **flee** (targets churn every 0.7–1.2 s and point off-map — so a panicking crowd is not
  covered by slots). Unlike separation this runs in **both** modes: it is simulation, not
  cosmetics, which is also why there is no camera gate — unslotted clumps would pile up
  and freeze off screen, and the camera would arrive into exactly the pathology. No
  `HashMap` iteration reaches the output (keyed lookups only) and the assignment batch is
  sorted by `(species, PawnId)`, the same key discipline as `apply_pathfinding_results`.
  Changing the radius slider or the navtile size re-keys the lattice, so the index is
  dropped and rebuilt from the next selections.
- **Human** states (`human/behavior.rs`): **Wander** (`WanderPause` 2–10 s *between*
  walks, zero at spawn so nobody stands around after launch; then 80%
  head to a random building anywhere in the city — long routes, the real pathfinding
  load — and 20% stroll 20–40 m nearby; the one exception is the first target after
  calming down from panic, which is *always* an errand — see **PanicRecoil**) ⇄
  **Flee** (demon within `HUMAN_PANIC_RADIUS`
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
  uniformly random and pawns wobbled in place instead of walking somewhere. `flee`
  rewrites it to the away-vector on every repath, so a calmed human resumes facing away
  from the demon rather than on its stale pre-panic course — which pointed at the demon,
  since that is where it was walking when it got scared.
- **PanicRecoil** — inserted on the Flee → Wander calm-down, a unit vector *toward* the
  demon (the negated `WanderHeading`, i.e. the last flee away-vector). It is remembered
  during flee, never queried live: `pick_wander_targets` iterates all 20 000 wanderers
  and must stay off the demon grid, which is the whole point of the inverted `panic`. At
  calm-down the demon is already >90 m away and unavailable anyway — the branch fires
  *because* `nearest_in_range` returned `None`. Staleness is ≲13° (≤9.6 m of travel at
  the 0.7–1.2 s repath period against a ≥90 m separation). While the component is on,
  the next target must be an errand and must clear two filters inside
  `pick_building_ahead`: not within `RECOIL_CONE` (±45°, a 90° cone) of the recoil
  vector, and not closer than `RECOIL_MIN_ERRAND` (90 m, the panic hysteresis radius) —
  the second one matters because a building just outside the cone but 15 m away
  reproduces the short walk being ruled out. Rejected candidates are dropped *before*
  the "best-aligned of the 8" fallback, which is what used to hand back a building
  nearly 180° from the heading, i.e. straight at the demon. A sample with nothing
  acceptable re-rolls next frame (never a stroll); only a city with no buildings at all
  falls back to a stroll, cone-checked after the `MAP_MARGIN` clamp, since at the map
  edge the clamp is what turns the direction around. Dropped at the first successful
  pick. Note the cone is centred on the human's own *spread-tilted* flee vector, so it
  reads as "don't go back the way you came" — the exact demon bearing is inside it
  regardless, as `FLEE_SPREAD` (±0.6 rad ≈ 34°) is smaller than the 45° half-angle.
- **HumanFirstWanderTag** — the very first target after spawn is always the *near*
  stroll, never a building errand; the tag is dropped when that target is picked. All
  20 000 humans queue their first path in the same frame, and cross-city A* costs
  hundreds of ms per request: with errands first the on-screen pawns took 3.9 s to route
  (the whole `PlayPhase::Warmup`), with strolls first — 0.15 s. `PanicRecoil` overrides
  it: a panic at spawn reaches only the crowd within 60 m of the portal, and their
  calm-downs spread over seconds, so there is no burst of that shape to guard against.
- **Pace** — a human's personal speed multiplier, rolled once at spawn and stored
  **normalized**, −1…+1. The effective speed is `base × (1 + Pace × HumanStyle::spread)`:
  a negative roll is slower than the base, a positive one faster, zero exactly the base.
  The same multiplier applies to *both* bases, `HUMAN_WALK_SPEED` and `HUMAN_FLEE_SPEED` —
  a fast human is fast walking and fleeing alike — so the three places that write
  `Movable::speed` for a human (spawn, the Wander → Flee switch in `panic`, the calm-down
  branch of `flee`) all go through `Pace::speed(base, spread)`. Storing the *normalized*
  deviation rather than the finished multiplier is what makes the slider sane: it widens
  and narrows the ordering the crowd already rolled (at 0% everyone walks the base speed)
  instead of dealing every pawn a fresh lot on every frame of a drag. It is a component
  and not something derived from `Movable::speed`, because that field is overwritten on
  every Wander ⇄ Flee transition — the first panic would erase the spread.
  **HumanStyle { spread }** carries it, a `SettingsGroup` persisted like `DemonStyle`,
  driven by the **Speed spread** slider (0…35%, step 5, default `HUMAN_SPEED_SPREAD`
  = 15%). The ceiling is derived, not round: a demon at `DEMON_SPEED_FACTOR_MIN` moves at
  exactly `HUMAN_FLEE_SPEED × 1.35`, so a spread above 0.35 would make the fastest humans
  literally uncatchable. Moving the slider reaches the humans already walking through
  `sync_human_pace` (`resource_changed`, not per-frame), which picks the base off
  `Has<HumanFleeTag>` — recomputing a fleeing human from the walk base would leave it
  strolling for the rest of its panic, since `flee` only rewrites the speed on the way
  *out* of the state.
- **CorpseTag** — a killed human: behavior/movement components removed, dark lying
  sprite at `Z_CORPSE`. Not in the human spatial grid (grid filters on `Human`).
- **DemonSpawnPause** — a demon that just stepped out of the portal stands still for a
  random **0.5–3 s** (`DEMON_SPAWN_PAUSE`), the initial burst included. The component
  filters *both* `pick_wander_targets` and `acquire_targets` out with a plain
  `Without<DemonSpawnPause>`, so the pause blocks aggro as well: humans walk past the
  portal constantly, and a pause the first victim cancels is not a pause. `tick_spawn_pause`
  (`Update`, so `Res<Time>` is `Time<Virtual>` — the pause scales with sim speed, like the
  human `WanderPause`) removes the component when the timer finishes; nothing else reads
  the timer. Without it the whole burst scattered inside one frame and a demon's arrival
  on the map did not read at all.
- **Demon** states (`demon/behavior.rs`): **Wander** (target biased away from portal) →
  **Chase** → **Devour** → Wander. Chase claims: **max 2 chasers per target**
  (`ChaserCounts`). Repath throttle 0.4 s, and on that same tick the demon may
  **switch** target, two cases: sharing its target, it takes any *unclaimed* human
  no farther than ×1.5 its current distance (the pincer breaks up); otherwise it
  takes whoever is nearer than **×0.7** of the current target — without that a demon
  runs through a crowd past easy prey, holding the target it locked on to until the
  victim dies or the aggro hysteresis drops it. The ×0.7 is the anti-flip-flop
  margin: two near-equidistant victims would otherwise trade the demon back and
  forth every repath tick, and each switch costs a fresh path request. Both cases
  require **`line_of_sight`** to the candidate, checked on the search winner only
  (in the grid filter it would run for every candidate in the 3×3 cells) — a human
  close by euclid but cut off by a building is unreachable, and chasing it just
  makes the demon dither. The search radius is proportional to the current distance,
  which self-limits at both ends: 1.4 m when the demon is 2 m from its victim (it no
  longer turns aside), 47 m at the far end of the hysteresis (still a 3×3 cell walk).
  **Lunge** — inside `DEMON_LUNGE_RANGE` (6 m) *and* with `line_of_sight` to the victim,
  the demon drops its path and steps `SimPosition` straight at the target, at its speed
  plus `DemonStyle::lunge`. Without it a chase never converts: a tile path aims at the
  *center* of the victim's tile while the
  victim keeps moving inside it, so the last ~1.4 m — more than `KILL_DISTANCE` — is
  never closed and the demon "almost catches" forever. The line-of-sight check is what
  keeps the lunge from cutting through a building when the victim rounds a corner.
  A lunging demon carries **`DemonLungeTag`** (set/cleared in `chase`) — it has no tile
  path left, so the movepath gizmo would show nothing; `draw_lunge_paths` draws its arrow
  straight at the victim's live `SimPosition` instead.
  Kill at `KILL_DISTANCE` triggers `DemonCaughtHumanEvent` (observer); `killed_this_tick`
  HashSet dedupes double kills within one command flush. **Devour** — pause 1.5–2 s with
  a sine **pulse** ×1 → ×1.5 (0.5 s period), scale reset on exit.
- **DEMON_SPEED** — one base for every state, `HUMAN_FLEE_SPEED × 1.35`, wandering and
  chasing alike. Since humans got a per-pawn `Pace`, that "+35% over a fleeing human" is
  true of the *average* human; it is also where the `Speed spread` slider's ceiling comes
  from (see **Pace**). Do not reintroduce per-state demon speeds: the only multipliers are the
  two user ones, `DemonStyle::speed` (whole demon, ×1.0…×2.0) and `DemonStyle::lunge`
  (the lunge phase only, +0…+100%). `Movable::speed` is written **once**, at spawn, as
  `DEMON_SPEED × speed`; moving the slider reaches the demons already out through
  `sync_demon_speed` (`resource_changed`, not per-frame). The lunge boost never touches
  `Movable::speed` — that phase moves `SimPosition` itself, past
  `move_moving_entities`, so the multiplier belongs at the one line in `chase` that
  steps it, and there is nothing to unwind when the lunge ends.
- **DemonSpawner** — initial burst at the portal rim, then one demon per interval up to
  the cap. Runs in `FixedUpdate` so restart re-fires the burst for free. Cap and interval
  are **not** constants: they live in **`DemonStyle { cap, interval, speed, lunge }`**,
  driven by the sliders of the Demon panel and persisted through `prefs` —
  `DEMON_CAP = 100` and `DEMON_SPAWN_INTERVAL = 1.0` are only its `Default`. Three
  consequences worth knowing: the burst is capped too (`DEMON_INITIAL_BURST.min(cap)`, and it fans over the *reduced*
  count, else a cap below 8 would still let a full burst out); lowering the cap never
  despawns demons already out — the spawner just goes quiet, so it reads on screen only
  after `R`; and the timer's period is re-synced inside `tick_spawner` rather than by a
  `resource_changed` system, because restart and city switch rebuild `DemonSpawner` whole
  (`restart.rs`, `city.rs`) — the timer would fall back to the constant with no further
  resource change to fix it.
- **Telemetry** — `{killed, escaped}`, BRP-readable, and `killed` is what the World panel
  shows as **Souls**. Invariant (check paused):
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
- **World, Demon and Human panels** (`ui/stats.rs`) — top-left, the only corner the other panels
  leave free, one under the other in a plain flex column (it grows *downward* from the
  screen edge, so unlike `stack_bottom_columns` nothing has to measure heights).
  **World** holds three live counters — **Pawns** (`With<Human>`, i.e. alive: the
  component is stripped on death), **Demons**, **Souls reaped** (`Telemetry::killed`), on
  their own `Heavy` backing the way slider rows have one. **Demon** holds the four
  `DemonStyle` slider rows from the same `ui/slider.rs` kit as Trees and Noise —
  **Max demons** (0…500, step 5), **Spawn every** (0.1…10 s, step 0.1), **Speed**
  (100…200%, step 5) and **Lunge boost** (+0…+100%, step 5); both percent rows print as
  percent, a bare `1.3` on the panel says nothing. **Human** holds the single
  `HumanStyle` row, **Speed spread** (0…35%, step 5) — printed with a sign because it is
  a half-width, and a bare `15%` would read as "everyone 15% faster". The sign is the
  ASCII `+/-`, not `±`: the built-in font (the `default_font` feature) is a narrow subset
  and draws anything outside ASCII as an empty box. One row means no
  row enum: a pair of marker components (`SpreadValueLabel` / `SpreadSlider`) addresses
  it, and `HumanRow` gets written when a second row appears. **Body radius** stood here
  and the `Separation` toggle, `Slot search` and the three crowd knobs stood in World
  until all six moved into the Navigation panel's crowd groups — they are about
  movement, and World had stopped reading as a summary of the run.
  The counters use `iter().len()`, not
  `count()`: with a purely archetypal filter `QueryIter` is an `ExactSizeIterator`, so the
  length is a sum over archetypes rather than a walk over 20 000 entities every frame.
  In agent runs the red **BRP badge** owns that same corner, and `offset_below_brp_badge`
  measures it and pushes the column below — the `ComputedNode` physical-vs-logical px trap
  is the same one `stack_bottom_columns` documents.
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
- **Navigation panel** (`ui/navigation.rs`) — slot 2 of the left column, always visible,
  **one UI for both pathfinding backends** (the Roads/Trees row-button idiom — label
  left, value right). The top row **`Algo`** cycles `Navmesh` ⇄ `Polymesh`: pawns always
  walk one of the two, so it is a choice, not two toggles that could both read `Off`
  while the grid quietly served every request. Its single source of truth is
  `PolymeshDebug::enabled`, which defaults to `Polymesh` — and which the `Separation`
  row below follows, since separation does not run on the grid backend (see
  **Separation**): picking `Navmesh` here greys that row out the way determinism does.
  Under it stand the settings **of the selected backend only** — the other set is
  `Display::None`d out of the layout (`sync_section_visibility`), because an agent radius
  means nothing while pawns walk tiles, and a grid search algorithm means nothing while
  they walk the mesh:
  - `Navmesh` → **`Pathfind`** (`PathfindingAlgorithm`, cycles A*/Dijkstra/Fringe/BFS/
    HPA*/Theta*), **`Show`** (the grid fill overlay, `DebugNavmesh`);
  - `Polymesh` → **`Show`** (mesh overlay, draws nothing else), **`Chunks`** (default on)
    — the chunk hierarchy: it switches the *build* between layered and one flat layer
    (`FLAT_CHUNK_METERS`) and therefore triggers a rebuild, and it is what puts the grid
    on the overlay; one toggle for both halves on purpose, since a grid drawn over a
    search that does not use it is a picture of something untrue — and an **agent
    radius** slider (`POLYMESH_AGENT_RADIUS_MIN..MAX`, step 0.1 m) inflating obstacles at
    triangulation time.
  The radius minimum is deliberately non-zero (0.2 m) now that pawns
  walk the mesh, and it is read through `PolymeshDebug::radius()`, which clamps — the
  minimum was raised after the setting was already being persisted, so an older prefs
  file holds 0.0. The overlay is one merged mesh at z 5.3 (above the grid navmesh fill
  at 5.2):
  **blocked contours filled** in the *same* red as `sync_navmesh_overlay`, then **all
  polygon edges** of the built mesh stroked over it (shared edges deduped, so a
  translucent seam is never double-painted). Same colour is the point — the two layers
  paint the same claim, and only an identical fill makes their accuracy comparable by
  eye; with a non-zero agent radius the gap between fill and edges *is* the inflation.
  The chunk-grid boundaries go on top of the mesh edges — dark, half-transparent, and the
  same 0.4 m stroke width as an edge, since the grid is a partition drawn over the
  geometry, not another layer of world. They are drawn unconditionally from the built
  grid, which is 1×1 (no lines) for a flat mesh — the overlay states what the search
  actually walks, not what the toggle asks for.
  Below the backend settings sit two **groups**, `Separation` and `Slots` (`KnobGroup` —
  a plain enum, no resource and no component: it only sorts the knobs under their
  headers), holding every knob about how pawns get past each other and how they divide up
  end points. They live here rather than in World because both are about movement; World
  is the run (seed, determinism, counters), and the crowd knobs only ever sat there
  because the mechanism is species-independent. **Both groups are always expanded** —
  crowd knobs are tuned together, and hiding half of them would mean clicking back and
  forth mid-tuning; hiding is for what the current settings make irrelevant (the
  unselected backend's rows above), and these two always run. The `Separation` header
  *is* the toggle, exactly like `Algo`: `on`/`off` on the right, dimmed and inert (no
  hover highlight either) under determinism and on the grid backend
  (`separation_allowed_by_mode`), and its **knobs disappear** whenever separation is not
  running — determinism, the grid backend, or its own `off` — since there is nothing to
  tune while the mechanism never starts, the same rule that removes the unselected
  backend's rows. The header row stays: it *is* the toggle that brings separation back,
  and hiding it would lock you out. Their initial `Display` is set at spawn rather than
  left to `sync_separation_knob_visibility`, which runs under `resource_changed` and so
  does nothing on the first frame — with separation off at startup the sliders would have
  hung there until something else changed.
  Slots have no toggle — they run in both modes always —
  so `Slots` is a plain label, spawned without `Button` or an observer. Under the headers:
  **`Pass squeeze`**, **`Left share`** / **`Body radius`**, **`Slot search`**,
  **`Regroup`**. Body radius is here despite living on `HumanStyle`: it sets both the rest
  distance and the slot side, so tuning wants it beside the other crowd knobs, not half a
  screen away in Human. All five are one `Knob` enum (spawn, label sync and thumb sync
  each written once). Nested *slider* rows are indented by `indent_slider_row`, which
  patches the padding the shared `ui/slider.rs` kit knows nothing about; without it a
  section's slider sat left of that same section's button rows — `Agent radius` had been
  sitting unindented since it was added.
  Cache key (build generation + radius bits) lives on the overlay marker, the
  conifer-overlay idiom; chunks are absent from it because flipping them moves the
  generation. The two overlays can no longer collide — two red fills over one map read as
  a single layer at double alpha, and each is now drawn **only while its backend is the
  selected one** (`sync_navmesh_overlay` returns early when `polymesh.enabled`), so the
  mutual-exclusion system that used to push the toggles apart is gone. See **Poly
  navmesh** and **Polygonal routing** under Navigation for what the mesh is and how pawns
  walk it.
- **Slider kit** (`ui/slider.rs`) — `spawn_slider_row` (label + value text + discrete
  `bevy_ui_widgets::Slider`), `quantize`, and one `sync_slider_thumbs` for all panels
  (sliders carry the shared `UiSlider` marker; registered once in `UiPlugin`). Callers
  pass their own marker bundles for the value label and the slider to address them in
  their sync systems.
- **Value-row kit** (`ui/rows.rs`) — the sibling of the slider kit for the rows that are
  *buttons*: `spawn_value_row` (grey label left, white value right, click on an observer),
  `row_color`, `next_in`, `on_off`, and one `highlight_value_rows` for every panel (rows
  carry the shared `ValueRow` marker; registered once in `UiPlugin`). A row whose click
  currently does nothing gets `RowInert` from its panel and stops highlighting — promising
  a reaction the click will not deliver is worse than not highlighting. The only carrier
  today is the Separation toggle under **Deterministic** or grid navigation.
  Highlighting writes through `set_if_neq`: the system runs every frame over every row of
  every panel, and at most one of them — the one under the cursor — actually changes.
- **Bottom UI columns** (`ui/mod.rs::stack_bottom_columns`, `UiRightColumnSlot` /
  `UiLeftColumnSlot`) — right: Tree rows → Trees → Buildings → Roads → hotkey help;
  left: debug toggles → Noise → Navigation; both bottom-up. The panels are absolute (`bevy_ui` does
  not stack them), and the columns change height at runtime (Trees grows two rows on
  `Mixed`, Noise exists only with the `noise` toggle), so each panel's `bottom` is the
  summed **measured** height of those below it instead of a hardcoded constant;
  `Display::None` panels are skipped by their `Node.display`, not their last-frame
  `ComputedNode`. `ComputedNode::size` is in *physical* pixels — multiply by
  `inverse_scale_factor` or every offset doubles on a retina screen.
  Panels sit flush by default — a column of map-style panels reads as one block. A
  **`UiPanelGapBelow`** marker inserts one gap under a panel, and the gap is
  `UI_SCREEN_EDGE_PX_OFFSET`, the same distance the UI keeps from the screen edge, so
  every space in the layout is the same width. Two panels carry it, both where the
  *kind* of UI changes: Navigation (the button row below it is not a panel) and the
  hotkey help (the panels below it are map settings).
- **Debug toggles** (`ui/debug.rs`) — grid / doors / movepath / noise buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay it still owns is **one merged mesh** — per-tile entities once cost 330 k
  entities; the noise overlay is one sprite with a CPU-built texture (see Conifer stands).
  The *backend* settings — the grid overlay toggle, `pathfind:`, the agent radius — moved
  into the **Navigation panel** above, next to the other backend's settings; the row keeps
  the cycling buttons that are not about one backend's layer: **`camera:`** (start view)
  and **`navtile:`** (`NavtileBase`, 2 m ⇄ 1 m, reloads the world). Navtile is here and
  not under `Navmesh` because the world is *always* built in tiles of that size — the
  passability fill, the unreachable prune, the portal snap, the entrance generation —
  whichever backend the pawns then walk; hiding it with the grid settings would call a
  global setting a local one. A cycler goes green
  (`TOGGLE_ACTIVE_COLOR`, the same "on" colour as a toggle) while its resource equals
  `Default::default()` — `save`, `2m` — so a setting steered away from the baseline is
  visible at a glance; the check is against the `Default` impl, not a hardcoded variant, so
  moving `#[default]` moves the highlight with it. Label text and green-ness come from one
  `cycler_state` used by both the spawn and the sync system, so they cannot drift apart.
- **Camera start view** (`camera.rs`) — **`CameraPositionMode`** (`reset | save`, default
  `save`, the `camera:` button, persisted) decides where the camera stands when the world comes up:
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
  **RR** — a second `RestartEvent` within `RESTART_DOUBLE_PRESS` (0.5 s of real time) —
  goes to the portal at `START_ZOOM` whatever the mode says: in `save` mode a single R is
  a no-op for the camera (the saved view follows it live), so the way back to the portal
  is the double press. `RestartEvent { to_portal: true }` asks for the same thing without
  the double press, and every restart ordered by a changed world setting uses it
  (`RestartPending`).
- **sim_time.rs** — Space pauses, `=`/`-` walk the speed ladder (`SPEED_LADDER`:
  1 → 2 → 5 → 10 → 20 → 30; the button's `cycle_time_scale` wraps to 1x from the top
  step; an arbitrary BRP-written speed snaps to the nearest step on the next press).
  - **SimSpeed** — `{requested, pipeline, affordable, effective, actual}`. `requested` is
    what the ladder says; `pipeline` is the pathfinding-pipeline ceiling, the one
    regulator value with memory (see below); `affordable` is what the regulator computed
    the machine can carry (already `min`-ed with `pipeline`);
    `effective` is its command, what reaches `Time<Virtual>`; `actual` is measured —
    virtual seconds per real second, averaged over `ACTUAL_SPEED_WINDOW` (0.5 s of *real*
    time, so long frames weigh what they cost). `actual` is the only honest one: Bevy
    clips a frame's virtual delta at `max_delta`, so a stall eats simulated time behind
    the regulator's back. The panel and `is_throttled` read `actual`.
  - **SimLoad — what one tick costs, split in two.** `begin_sim_load` / `end_sim_load`
    bracket the fixed loop (`RunFixedMainLoopSystems::BeforeFixedMainLoop` /
    `AfterFixedMainLoop`) and divide the wall time of the frame's `FixedUpdate` run by the
    `SimTick` delta over the same bracket, smoothed with `SIM_LOAD_SMOOTHING` (0.5 s of
    real time, so the filter does not change with the frame rate). `SimTick` zeroes on
    restart and city switch, so the delta is a `saturating_sub` and a frame whose counter
    went backwards is skipped.
    **The split is the point.** `tick_ms` is CPU work — a property of the world, not of
    the speed, since speed changes how many steps a frame runs and not what a step costs.
    `wait_ms` is per-frame time inside the fixed loop that is not per-tick work — chiefly
    the main thread standing in `block_on` waiting for the pathfinding pool
    (`apply_pathfinding_results` reports it through `SimLoad::add_frame_cost`) — and that
    one depends on the speed directly: the answer's deadline is measured in **ticks**
    (`PATHFINDING_RETIRE_TICKS`), so faster ticks give the pool less real time for the
    same work. Blending the two into one number closes the regulator on a quantity it
    controls itself — measured live as tick cost swinging 2.9…7.6 ms with a 2–4 s period
    and the speed following it 1.3…3.4×. `wait_peak_ms` is the **peak-hold** companion of
    `wait_ms`: it jumps to any raw sample at once and decays toward the mean over
    `SIM_LOAD_PEAK_DECAY` (3 s) — bursts arrive in packs seconds apart, and the mean
    dilutes them before the pipeline ceiling can answer. Published as `sim/tick_ms`,
    `sim/wait_ms` and `sim/wait_peak_ms`, all on the panel's third line
    (`tick 1.20 + 3.50 ms wait (pk 8.10)`).
  - **The regulator solves where it can, integrates where it cannot.** Two independent
    bounds, the smaller wins.
    *By CPU it solves*: a frame of length `d` carries `d × S × 64` ticks, so allowing the
    simulation `SIM_FRAME_SHARE` of any frame gives `S = 1000 × share / (64 × tick_ms)` —
    `d` cancels, so the answer does not depend on which frame just happened, on vsync
    quantisation, or on history.
    *By pipeline it integrates* (`pipeline_limit`, state in `SimSpeed::pipeline`): the
    pathfinding pool is a queue, and near saturation the wait grows as `1/(1 − load)` —
    unbounded gain, so any one-step formula overshoots (measured live: wait swinging
    0.9…8.0 ms/tick with a ~1 s period over a flat CPU cost). Instead the ceiling steps
    against the busy-per-tick ratio (`tick_ms + wait_peak_ms` vs the share's per-tick
    allowance), **proportionally to the overrun** — 10 % over cuts 10 % per
    `SPEED_BACKOFF_TIME` (1 s), double cuts half; a constant step keeps cutting while the
    queue drains and makes its own sawtooth (measured: 1.1…1.8× with a ~2 s period).
    Probing back up runs on `SPEED_PROBE_TIME` (6 s). Sizing to the **peak** wait, not
    the mean, is a deliberate speed-for-smoothness trade.
    `SIM_FRAME_SHARE` is derived, not tuned: `1 − SIM_RENDER_BUDGET × MIN_SIM_FPS`
    (13 ms per frame reserved for everything that is not simulation → 0.61). Frames then
    settle at `rest / (1 − share)` — a contraction with gain `share < 1`, stable by
    construction. Applied asymmetrically: **down at once, up by doubling every
    `SPEED_CLIMB_DOUBLE_TIME` (0.75 s)**, with a symmetric `SPEED_DEADBAND` (2 %). The
    climb limit is the one thing the solver cannot compute — tick cost lags the speed,
    because a speed-up spawns path requests whose cost lands a second or two later.
    On top of the solved target, `frame_overrun` divides by how late the real frame ran
    versus `1/MIN_SIM_FPS` — unity in normal operation, it breaks the "long frame carries
    more ticks carries a longer frame" self-amplification during a dip.
    Floored at `MIN_SIM_SPEED` (0.1). The button shows `15x → 8.6x` when limited.
  - **The frame-budget guard** (`guard_frame_budget`, first system of `FixedUpdate`) is
    the hard backstop behind all of the above: the regulator aims at `SIM_FRAME_BUDGET_MS`
    (share × target frame ≈ 20 ms) from **smoothed** measurements, and a burst lands
    before any filter learns of it. Once a frame's fixed-loop run has eaten the budget,
    the guard strips the remaining `Time<Fixed>` overstep — the loop stops after the
    current tick — and books it into **TickDebt**, which `begin_sim_load` returns to the
    accumulator next frame (capped at `SIM_TICK_DEBT_CAP`, beyond which time is honestly
    dropped, same philosophy as `max_delta`; held while paused; zeroed on restart and
    city switch). Deferred ticks do not change game logic — how many ticks share a render
    frame floats anyway — a burst shows as a brief `actual` dip instead of a visible
    hitch. `TickDebt.deferred` (BRP-readable) counts everything ever deferred, so a live
    check can see the guard actually firing.
  - **Why fps is not the feedback signal**, though the goal is stated in frames. The
    window is `PresentMode::AutoVsync`, so measured fps only takes values `refresh/n`:
    while a frame fits in 16.7 ms the reading is flat 60 and says nothing about the
    remaining headroom — the regulator learns about an overload only after it has already
    overshot. Deriving the render cost as `frame − sim` does not rescue it either: under
    vsync that difference contains the sleep, and it grows exactly as the speed is cut,
    which drives a limit cycle between the 60 and 30 steps. Both numbers are fine to show
    and unfit to close a loop on. Two earlier attempts tuned this loop's coefficients
    (smoothing, then a hysteresis band) without touching that; the sawtooth survived both.
  - **`MAX_FRAME_DELTA` is not a speed ceiling**, and reading it as one is the mistake
    this loop was built on for a while. `Time<Virtual>::max_delta` clamps the **raw**
    frame delta, *before* the speed multiplies it
    (`bevy_time/src/virt.rs::advance_with_raw_delta`) — it is "the longest real frame we
    still count in full", and its only job is to stop a freeze from becoming an avalanche
    of ticks. `Time<Fixed>` has no per-frame step limit of its own
    (`bevy_time/src/fixed.rs::expend` runs while `overstep` allows), so this constant is
    the only thing between a long frame and `max_delta × S × 64` ticks inside it — at 0.5 s
    and 10× that was 320. A long frame carries more virtual time, hence more ticks, hence
    a longer frame still: the pit was self-sustaining and exactly `max_delta` deep. Hence
    **0.25 s** (Bevy's own default). The cost of the trade is that a frame longer than
    that silently hands the simulation less time than really passed, visible only in
    `actual`.
  - **Requested cap** — `MAX_SIM_SPEED` (30x, the top of `SPEED_LADDER`) is a hard
    ceiling on `requested`: a deliberate product limit, not a hardware one. The ladder
    never steps past it, and `throttle_speed_to_frame_budget` clamps `requested` itself so
    a BRP write cannot exceed it either.
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
