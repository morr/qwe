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
  All world coordinates are positive. `MAP_SIZE = 7600 × 5700` m — it was 5600 × 3700 until
  the map was pushed out by a kilometre on every side, and since the bbox keeps its geo
  centre, **that move shifted every absolute map coordinate by +1000 on both axes**: the
  portal hints moved with it, and a `cam x y` written down before it points a kilometre
  south-west of what it meant.
  **The same move invalidated every per-city count in these docs and in the skills**: the
  bbox grew by 2.1×, Tula from 7751 to 14597 buildings and 4649 to 7706 roads. A `Tula: N`
  below was measured on the 5600 × 3700 bbox unless the line says which cache it comes
  from — re-measure before quoting one, don't carry it forward.
- **Navtile** — navigation grid cell, **2 m by default, runtime-switchable to 1 m** via the
  `navtile:` cycler in the Debug tab. Grid size is derived as `MAP_SIZE / navtile_size()`
  (3800 × 2850 tiles at 2 m); the live value is a process-global atomic
  (`grid::navtile_size()`), written only in `OnEnter(Loading)`, and **a filled `Navmesh`
  carries its own `grid_size`/`tile_size` snapshot** so stale snapshots never index against
  the switched atomic — the fill and the navmesh-side queries convert through
  `Navmesh::to_tile` / `Navmesh::tile_center`, never the global pair. Switching reloads the
  world like a city switch, except the camera stays put. `grid.rs`: `world_to_tile` /
  `tile_center`, for callers with no `Navmesh` at hand. Costs and the chunk scaling —
  **navigation-deep skill**.
- **Post-processing** (`post.rs`) — the camera renders to an HDR target with **bloom**
  thresholded at 1.1, so only what draws itself above 1.0 glows — the portal vortex, demon
  halos, soul sparks — and the map's white markings and light roofs do not; tonemapping is
  off so the map palette is untouched. The map's edges are smoothed by **`Antialias`**
  (`post.rs`, persisted, the Debug tab's `Antialias` row, on by default): `Msaa::Sample4`
  on the user's camera and on the offscreen-shot one — ribbon edges are bare geometry, and
  without it they stair-step on a close-up while the paint beside them is smoothed by its
  own shader. The galleries run `Sample4` too. A full-screen **vignette** is
  a UI node under the panels, `Pickable::IGNORE` (detail in the `ui-panels` skill).
- **Viewport** (`camera.rs`) — the piece of the world in frame, as a value **and a
  resource**: `centre`, `half_extent`, `zoom` (world m per logical pixel). `contains`
  — **the edge counts as inside**. `ViewportPlugin` recomputes it once per frame before the
  fixed loop; the resource holds exactly the frame, and each of the five visibility gates
  adds **its own margin** with `with_margin` (warmup 1.0, dispatcher/separation
  `VIEW_MARGIN` 1.2, movepath gizmos 3.0, door gizmos 1.5), because each asks a different
  question — the table is in the **navigation-deep skill**. No `Default`, like `Backend`:
  a stand inserts it. Not Bevy's `Camera::viewport`, which is in pixels.
- **Geo anchor** — a `*_GEO_CENTER` per city (Tula's puts the kremlin near frame center),
  beside `City` in `city.rs`, which is the only reader; the portal hints
  (`TULA_PORTAL_POS`, `NY_PORTAL_POS`, `MAP_CENTER_PORTAL_POS`) live there with them.
  Projection is local equirectangular (`GeoBounds` in `map/osm/overpass.rs`): bbox SW
  corner → (0,0), f64 math, `MAP_SIZE`-sized bbox derived from the center, with
  `METERS_PER_DEG_LAT` in `settings.rs` — that one belongs to the projection, not to a
  city.
- **Z-layers** — constants in `settings.rs`, bottom to top: ground → ring islands
  (0.05) → verge lawns (0.08) → verge yards (0.09) → verges (0.1) → landuse works →
  landuse yards → parks → woods → tree-row band casing → tree-row band → grass → ring
  island grass (0.65) → sand → alleys (1.5) → sidewalks (1.6) → road medians (1.7) →
  unpaved roads (1.8) → roads (2.0) → road wear mask
  (2.0002) → road wear (2.0003) → road paint (2.0005) →
  parking (2.001) → lot
  sidewalks (2.002) → lot lines (2.003) → road islands (2.0035) → parking markings
  (2.004) → pitches (2.005) →
  pitch markings → water (2.01) → waterways (2.02) → rail ballast (2.03) → rail ties →
  level-crossing decks → rail steel → wagons (2.045) → bridge shadows (2.05) → bridge
  casings → bridges → bridge paint (2.25) → track-bridge ballast, ties, steel (2.3) →
  tram (2.6) → cars → fences (2.75) → pipe shadows (2.76) →
  pipes (2.77) → portal stain → corpses → portal → industry shadows (4.55) → buildings (5) →
  roof shadows (5.05) → industry walls (5.06) → industry tops (5.07) → units → souls (18)
  → tree shadows → trees (20). Four live in their
  own modules: `Z_BUILDING_SHADOW` 4.5, `Z_FACADE` 4.9, `Z_ROOF_SHADOW` 5.05
  (`map/buildings/mod.rs`), `Z_WALL` 5.1 (`map/roads.rs`). Units are y-sorted:
  `unit_z(y) = Z_UNIT_BASE − y · Y_SORT_FACTOR` (10 − y·0.002). **Invariant: the unit z
  range must stay above buildings (5) for any y ≤ MAP_SIZE.y** — a bigger map once sank
  northern units under roads. **Invariant: a road lies over water only as a bridge** —
  area water and open waterways both sit above every road ribbon and below the bridge
  shadow, because the navmesh blocks both and only a bridge carves them (a narrow river
  arm is mapped as a ribbon or as a polygon, so one rule covers both); the price is
  water cutting an embankment sidewalk or footway mapped against the bank, and a street
  over an unmapped culvert.

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
- **City** (`city.rs`, resource, persisted) — `Tula | Ryazan | Kaluga | Oryol | Rostov |
  Belgorod | Moscow{Center, North, NorthEast, East, SouthEast, South, SouthWest, West,
  NorthWest} | NewYork | Paris | Berlin | London | Tokyo | DevilsLake`, each with its geo center, portal hint and cache slug. `MAP_SIZE` and
  therefore the derived `grid_size()` are shared, so switching city never resizes the
  navmesh. UI — a select at
  bottom centre (`ui/city.rs`).
- **City switch = full world reload** — writing `City` sends the app back to
  `AppState::Loading`; the scene is torn down, the new extract downloaded and parsed, the
  same navmesh refilled, the camera reset. Gated on `in_state(Playing)`: restarting a load
  on top of a running one would put two threads into one navmesh. A settled new navtile
  size or **lane width** (`RoadShape`) reloads the same way, same city, camera kept.
- **`DespawnOnExit(AppState::Playing)`** — the *only* thing that clears the old city. Every
  world entity must carry it; the rule and the list of spawn sites live in **CLAUDE.md**
  ("World entities"), and `loading.rs::warn_leftover_world_entities` warns when something
  survived.

## OSM map pipeline

Summary; the mechanism — **osm-map skill** (entrance statistics in
`references/entrances.md`, planting and crowns in `references/trees.md`, the tag coverage
audit in `references/osm-coverage.md`, the crown algorithm in `references/tree-algo.md`).

- **Overpass** — the Overpass API, queried once per city with `[out:json]` + `out geom`;
  bbox is `MAP_SIZE` around the `City` geo center; mirrors in `OVERPASS_URLS` tried in
  order. **Bump `QUERY_VERSION` in `overpass.rs` whenever the query gains tags**
  (currently 15), or existing caches keep serving extracts that lack them.
- **Driving side** (`TrafficSide: Right | Left`, `MapData::traffic_side`) — the
  `driving_side` tag of the country boundary the map's centre lies in (`is_in`, a second
  `out tags` output). No answer means `Right`.
- **Cache** — `assets/osm/{slug}_{lat}_{lon}_{w}x{h}_v{QUERY_VERSION}.json` (gitignored);
  written only after a successful parse, and **exactly one current file per city**
  (`prune_stale_caches()`). The junction gallery reads the same file, so a query bump
  reaches it by itself.
- **Overpass fixture** (`map/osm/fixture.rs::Overpass`) — a test scene stated in **map
  metres** and fed through the real `parse`. **Add a tag case here, not another literal.**
- **MapData** (`map/osm/model.rs`) — the parsed map resource, resident after spawn. The
  per-field catalog — provenance, consumers, invariants — is the **osm-map skill**,
  `references/model.md`.
  - **The nine `Vec<PolyArea>` and `AreaKind` are not the same fact twice** — the vector
    says which *layer* an area is drawn in, the kind says which *member of that layer* it
    is, and three vectors hold more than one member.
  - **PolyArea** — polygon with holes, rings open. `AreaKind: Building | Kremlin | Water |
    Park | Wood | Grass | Sand | Residential | Industrial | Parking | Pitch(PitchKind)`;
    **only Wood carries trees**; `colours` are the **tagged colours** (`building:colour` /
    `roof:colour`), read so far only by churches (**Crown** below). Residential/Industrial
    are the `landuse` **blocks** — the fill under everything else, no effect on navigation
    or planting; **a residential block is drawn as a yard**, industrial keeps the cold
    grey. **Parking** is asphalt with marked stalls (**Parking lots** below); its **lot
    kind** is the parse's answer, read and never re-derived (`Kerbside` | `Ground` |
    `Yard`; a lot whose asphalt is not on the ground is no area at all). **Pitch** — a
    sports or children's ground (**Pitches** below). Buildings carry `height`, `storeys`,
    `entrances: Vec<Vec2>` and `building_use`.
  - **RoadNode** / **RoadArea** — the v15 junction vocabulary (`MapData::road_nodes`,
    `road_areas`): crossings, signals, stop / give-way, mini-roundabouts, turning
    circles, island points; closed carriageway / walkway / island outlines from
    `area:highway`, `highway` + `area=yes`, `traffic_calming=island`. A crossing or an
    island point on a two-way street is a **safety island** — a kerbed lens over the
    asphalt and its paint — and the closed line an area also arrives as is **not laid as
    a ribbon** (`roads/islands.rs`).
  - **RoadLine** — centerline + width **from its section** (`tags.rs::path_width` for a
    path; footbridges and arches keep the class 3.5, their band is a navmesh corridor);
    `RoadClass: Street | Alley`; `highway: Highway` — **`Highway::is_street` is what makes
    a carriageway, not the width**; `bridge` / `passage` flags. **`carves_navmesh` is the
    one predicate the drawers ask**: an arch keeps its OSM centerline (neither smoothed,
    paired, ringed, tapered nor stitched); a bridge is smoothed and paired and takes a
    **taper** at a ground/bridge seam like any seam — **and at that seam every ribbon ends
    square** (deck, approach sidewalk band and verge, apron; `bridges::seam_ends`); a
    **footbridge** mapped inside a street deck is moved out past its edge, one mapped
    beside it within 3 m gets a **slit apron** (`roads/aprons.rs`, render only — the curb
    stays the navmesh's edge); a path onto a footbridge widens into its curbs; a paired
    deck's median lies on the deck.
    `oneway`, `roundabout` — the notion is **`RoadLine::is_roundabout`, tag or shape**: a
    closed one-way way is a ring too. A **Ring** (`roads/rings.rs`) is such ways chained
    into a loop and drawn as one smooth ellipse through its nodes, **one section for all
    arcs, its asphalt one closed fill** along the whole loop, a kerb round its island and
    a lawn over it; approaches enter by a tangent arc; a **Y-tail is straightened by the
    parse** (`rings::straighten_tails`, before the sections); a **web** (`Rings::webs`) is
    asphalt filling the slit between the ring and a street running along it outside.
    `lanes: Option<u8>` — the section's count, below; `parking_aisle` is read by the stall
    layout only (see **Parking lots** below); `pavement: Option<Pavement>` — a **paved
    path** is drawn in the sidewalk layer, an unpaved one the sand trail of the alley
    layer. On a street only the `surface` tag sets it: an **unpaved street** has its own
    layer `unpaved_roads` under the asphalt, **no paint at all**, and two dirt roads meet
    with a small corner; a street ending at a dirt road stops at its kerb
    (`KerbReturns::setback`). A surface change along a way **pins the seam**
    (`axis.rs::Run::stitch`, `resurfaced`) — it holds like a seam another street joins.
  - **Street** (**RoadNetwork**, `MapData::network`) — ways glued end to end through their
    seams: the **most collinear pair of ends of one `Highway` class** continues one
    street. Rings and closed ways are streets of one way; paths are in none.
  - **Section** (`map/roads/network/sections.rs`, first pass of `finish_parse`) — the lane
    count (from `lanes`, else the nearest tagged way of its street, else a default by
    class) and the width that follows: lanes × the **lane width** (a `RoadShape` knob,
    3.3 m) + a 0.5 m edge each side; a lone short jump is cut to its neighbours; a
    two-way way carries its **split** between the flows. **The only roads stage that
    moves the model** — the parse passes after it read the width.
  - **Taper** (`map/roads/tapers.rs`) — where the way goes on with a different width, the
    wider one starts at the narrower's width and widens over `RoadShape::taper` metres
    per metre of difference; per side only the kerb with no other carriageway arm
    narrows; **any node under the wedge reads the wedge's width there**; an obtuse
    continuer **grows the mouth** instead (`tapers::widen_mouth`). Drawing only.
  - **Street axis** (`map/roads/axis.rs`) — the drawn centerline of a whole street, not of
    a way: within the **curve tolerance** the street is simplified and each bend becomes
    an arc; a node shared with a third road stays exactly in place. Bridges, arches and
    paths keep the per-way Chaikin `centerline`. **The ribbon, the sidewalk band and the
    parked cars all stand on it.** Drawing only.
  - **Lane frame** (`meshing::LaneFrame`) — where a carriageway's lanes lie across its
    axis: a grid node `origin` and the bounds `low..high`, boundaries at
    `origin + k · lane width` (the one lane width of the city). **One frame for the ruts
    and the lines.** On a taper it drifts from the narrow section's to the wide one's,
    and when the parity changes the grid slides by half a lane over the taper; a
    **one-way** wedge adds its lanes at one kerb (`paint::wedge_drift`).
  - **Paint layer** (`map/roads/paint.rs`, shader `paint.wesl`) — the lane lines as
    **geometry** off the street axis, not a pattern of the asphalt shader: dashes **by
    the street's arclength** (the phase does not restart at a seam), a lane line solid
    for the last 25 m of its **approach** to a junction break, the axis of a two-way
    street of 4+ lanes as a double solid — **the axis is the border between the flows**;
    across a taper it starts on the seam way's frame and drifts over the wedge. **A line
    never ends in a hook.** `PaintLods` hides the paint past its zoom maxima without a
    rebuild; `RoadPaintStyle` is uniforms only. Detail — `roads-markings.md`.
  - **Junction paint** (`map/roads/node_paint.rs`) — where the paint layer breaks at a
    junction and what it draws there, **on its own breaks rather than the asphalt ones**.
    Overlapping junction zones form one **cluster**; the **arm edge** — where the arm's
    cross-section leaves the other roads' asphalt — is what zebras, stop lines and turn
    paths measure from; a **link** (an arm that never leaves the junction's asphalt) gets
    no rule paint; **main through** decides who keeps its lines through a cluster (ends
    there, a crossroads, a higher rank, or signals break it); on every breaking arm a
    **zebra** and a **stop line** by rule or OSM; **the plank cuts every carriageway whose
    section passes through it**; a zebra is drawn as **whole bars** only. Every output
    leaves as a typed view of its own; the fields are closed inside `node_paint`. Detail —
    `roads-junction-paint.md`.
  - **Turn paths** (`map/roads/turns.rs`) — a lane-to-lane Bézier for every allowed
    maneuver (`turn:lanes`, else the rule), with 5 m tails; drawn as **turn wear**, two
    faint ruts **like a shadow** — where ruts overlap the pixel takes the strongest, not
    their sum. The same maneuvers give the **lane arrows** (toggle `RoadStyle::arrows`).
    Detail — `roads-bridges-wear.md` (**Turn paths**).
  - **Kerb pocket** (`map/roads/pockets.rs`) — a parking bay cut into the sidewalk beside
    the carriageway, where the parked cars stand: by `parking:<side>=street_side`, or by
    rule on trunk/primary/secondary; **never along a lot's frontage**. One answer
    (`pockets::all_kerbsides`) for the ribbon and for `map::cars`. Detail —
    `roads-sidewalks.md` (**Kerb pockets**).
  - **Sidewalk sides** — `sidewalk=*` decides which sides carry a sidewalk band; an
    **untagged** street has none on an unpaved surface, and a residential-class street
    has them only among blocks of 3+ storeys on average (`parse::infer_sidewalks`); any
    carriageway loses an untagged side to a **separately mapped footway** beside it.
    A side with such a footway alongside carries a **verge** (`RoadLine::verges`,
    **by place** — `verge_profile` follows a footway that drifts or turns): sidewalk tile
    under every green; a verge wider than 4 m is a **lawn** under a narrow tile strip at
    the kerb — yard grass by default, a meadow only beside a mapped lawn or park — and
    from 2 m/px (`VergeLods`) a verge wider than 5 m is not drawn at all. A corner behind
    the lawn is a **kerb pad** of tile along the kerb arc. What the pad, a crossing path
    and a footway pressed to the kerb leave of the lawn is a **lawn scrap**
    (`roads/scraps.rs`): a small piece of verge lawn enclosed by paving, tiled over in
    `road_verges`; the same question asked of **ground green** lays its scraps in
    `sidewalks`. The per-side decision is a **`SidewalkSide`** tristate on the `RoadLine`:
    `Tagged`, `Inferred`, `None`. Detail — `roads-sidewalks.md`.
  - **Paired halves** (`map/roads/network/pairs.rs`, `Pairs`) — a divided street as OSM
    draws it: two opposite one-way ways of one class side by side, up to `PAIR_MAX_GAP`
    between the kerbs. Found once, by `axis::street_axes`, and **aligned** there — the
    halves are set at the run's median gap from the midpoint, fading out at a run's end
    and at a foreign node — so the ribbons and the parked cars stand on the aligned axis.
    **A half has no sidewalk on its paired side.** Drawing only: `RoadLine::points` do
    not move; a half's answers are asked of `Pairs` (`beside`, `partners`, `is_paired`,
    `across_median`, `band_pieces`), the fields are closed to `network/*`.
  - **Median** (`roads/medians.rs`, one door `medians::draw`) — what lies between the
    halves: up to the **median gap** (`RoadShape::median_gap`) asphalt under both ribbons
    with a double solid, wider a lawn with a kerb and a rounded nose; a zebra across both
    halves cuts a **passage** through the lawn, not an end to it. A median a tram runs
    in is a **tram bed** (`Median::carries_tram`): each half widened by an inner lane
    without marking, the double solid between the tracks. A **tram band** is the lighter
    asphalt along a tram track under a street (`roads/tram_band.rs`); both lie along the
    **laid tracks** (`roads/tram_lay.rs`, `TramTracks`), not the OSM ones. The median
    opens only at a break of **both** halves facing each other. Detail —
    `roads-pairs-medians.md`.
  - **RailLine** — `railway=*` centerline; `RailKind: Active | Tram | Disused` *is* the
    drawing style; underground track is dropped. The rail branch of `parse_way` runs
    before `highway` and **falls through** — a way can be both street and track. A
    **rail gap** (a track broken at a street's kerbs) is stitched at parse. Non-tram
    track is `map/rail.rs` (ballast, ties, steel, a zoom LOD); a **track bridge** rides
    the road layer's bridge layers; a **level crossing** is found **by geometry**, not by
    tag, and laid along the street **as drawn**. Tram is `map/tram.rs`, drawn only while
    `TramStyle::visible`, along the laid tracks.
  - **WallLine** — `barrier=city_wall` (the kremlin), 3 m, impassable. **Drawn only where
    no fortress building stands on it** (`roads.rs::Fortresses`); the navmesh still
    blocks the whole line.
  - **FenceLine** — a plot boundary: `FenceKind: Fence | Railing | Wall | Hedge`, plus
    the **default gates** the load adds. **Impassable in the navmesh with gaps**
    (`FENCE_BAND_WIDTH` is the physical thickness, not the drawn width — see **Fence
    gap** under Navigation). The parse branch falls through.
  - **WaterLine** — a *linear* watercourse; `tunnel: bool` marks a **culvert**: not
    drawn, and the only watercourse kind that does **not** block the navmesh. Drawn only
    **outside area water**; a stretch cut at **both** ends is a **water gap** — its band
    goes into the area water's union, and the ribbon layer does not draw it at all.
    Render-only; the navmesh blocks the whole channel band.
  - **TreeRow / TreeNode** — `natural=tree_row` avenues and single surveyed trees, with
    optional `spacing`/`radius` from tags.
  - **trees** (`TreeSet`) — what the renderer reads; a position and its appearance
    threshold are **one value** (they enter through one `push` and cannot drift apart).
    **The prefix rule**: the density slider shows the beginning of the set, not a filter —
    a step up only ever adds trees and never moves the standing ones.
- **The parse seam** (`map/osm/parse.rs`) — reading the elements and finishing the map are
  two things, and either half is callable alone. **`read_elements`** runs the Overpass
  element loop and stops; **`finish_parse`** runs the finishing passes (the street
  sections first), **and that order is their interface** — all steps written down whole
  on that function. Both halves report as values; `parse` prints them. Order invariants:
  mapped doors attach **before** skewed houses are squared, and `vertex_uses` is
  deliberately computed **twice** (the outlines move between its two readers). A narrow
  pass takes a **pass context** instead of `&mut MapData`; a pass that follows ten fields
  or more stays on the whole map. Detail — `parse-seam.md`.
- **OSM contours** (`map/osm/contours.rs`) — the map's geometry as the element loop read
  it, **before any finishing pass**, drawn as an overlay. **The contour sits on the
  defect — the data; it does not — our processing.**
- **Raw OSM** (`parse.rs::RawOsm`, `ParseKnobs::raw`) — the map with our own additions
  switched off, to tell a data defect from a processing one: **`Parse`** skips every
  finishing pass but the mapped doors; **`Draw`** adds no drawing additions on top. A
  parse input, so switching it **reloads the world**.
- **ParseKnobs** (`map/osm/parse.rs`) — the parse's inputs that are not in the Overpass
  answer: `lane_width`, `navtile`, `raw`. `parse` **reads no process global**; the map
  keeps a snapshot (`MapData::knobs`) — what the world was parsed with, what the
  lane-width reload compares against.
- **Building height** (`parse/tags.rs::building_height`) — metres from `height` or
  `building:levels` × 3 m; outside 2–600 m counts as no tag. `None` is normal and
  common (coverage varies wildly by city, logged on load).
- **Squared house** (`parse.rs::square_skewed_houses`) — a small skewed-quad or skewed-L
  house is replaced at parse by a rectangle of the same centroid and area, or a
  right-angled L; not when a vertex is shared with another building, a line, or one of
  the three area layers whose **edge is drawn**. Everything downstream sees the
  straightened outline.
- **Pulled-back house** (`parse.rs::pull_houses_off_sidewalks`) — a building whose
  outline reaches into a street's **verge edge** is moved at parse, whole and with its
  doors, straight away from the street, by at most 6 m — **its row with it**, so the
  facade line stays straight; a shift that would crowd something is shortened or
  dropped. Not for churches, fortresses, shared vertices, or a street through the house.
- **Block pulled to the road** (`parse.rs::pull_areas_to_roads`) — a `landuse` block's
  edge traced along the plot line leaves bare ground against the drawn sidewalk; a
  vertex near the road's edge is pulled **under** the asphalt, **green only grows**
  (the move must lead outward from the fill), and slivers left on the verge tiles are
  then **cut out by area** (`parse/verges.rs`). Render-only in effect.
- **Lot paved to its roads** (`parse/lots.rs::pave_lots`) — a parking lot reaches its
  roads **as a polygon, not vertex by vertex**: lot and road bands are **closed** and
  only the pieces lying **between the lot and a road** are kept; buildings, greenery,
  water and fences are subtracted; **a fill thinner than `THIN_FILL_HALF_WIDTH` is
  trimmed**; last, the lot steps back an **apron** from every house. **The pass reports
  the two events apart** (`PavedLots { grown, trimmed }`). Lots are paved **across
  threads**. Detail — `parse-geometry.md`.
- **Ground pocket** (`parse/pockets.rs::fill_ground_pockets`, `MapData::pockets`) — a
  scrap of bare ground **enclosed** by what is drawn that no tag describes: a **hole of
  the union** of the covers, found per tile; a hole touching a paved road, touching no
  dirt path and small enough is sown with grass — beside a block that block's grass, else
  a mapped meadow **merged into that very green**, else the yard grass. **A pocket is not
  a block** — a verge next to it does not turn to yard grass. A **green apron**
  (`Scene::green_aprons`) asks the same question of strips that are never holes. Detail —
  `parse-geometry.md`.
- **Inferred storeys** (`map/buildings/heights.rs`) — what a building without a `height`
  tag is drawn as: **the shape of the footprint decides**, the way an eye reads an
  aerial photo (a long thin box a panel section, a compact large one a tower, a small
  one an old low building, the rest a generic block; industrial/commercial/church
  footprints are measured in **metres of span**). The slot inside each group comes from
  the building's own **seed** — stable across rebuilds and modes. **The tag always
  wins.** Detail — `buildings-heights.md`.
- **Building use** (`parse/tags.rs::building_use`) — the **drawing class** of a building,
  `BuildingUse: House | Apartments | Commercial | Retail | Industrial | Garage |
  GarageBlock | Church(Sacred) | Public | Other`, from `building=*`, then `shop=*`, then
  `amenity=*`. **`Sacred { faith, form }`** rides inside `Church`: **`Faith`** from
  `religion` + `denomination` (`Unknown` does not survive the parse —
  `parse::faiths::resolve_faiths` gives a part the faith of its church, anything else the
  city's majority), **`SacredForm: Nave | Tower | Dome | Annex`**. **Fortress** —
  `AreaKind::Kremlin` is told also by `building=wall` ≥ 6 m and `man_made=tower` +
  `tower:type=defensive` (Tula carries no `historic` at all). **`garages` (plural) is its
  own class** — drawn as rows of boxes (**Garage rows** below). Each class owns a
  (roof, wall) colour pair in `map/buildings/`. Detail — `parse-buildings.md`.
- **Retail box** (`BuildingUse::Retail`, `model::is_big_box`) — a building that **is** a
  shop, split from `Commercial`. **Size, storeys and height decide as much as the
  class**: `is_big_box` (footprint ≥ 1200 m², ≤ 3 storeys, height ≤ 17 m) is the one
  predicate — **the storey count is what tells a mall from a box**. What it decides: the
  **height** (a trading level is 4.5 m), the **roof** (membrane, the skylight grid, the
  roof plant) and the **wall** (`WallKind::BigBox`, a cassette facade with a **brand
  band** in the building's tagged colour). A corner shop keeps a shopfront and an
  ordinary flat roof. Detail — `parse-buildings.md`, `buildings-material.md`.
- **Skylight grid** and **roof plant** (`map/buildings/clutter.rs`) — what a big box's
  roof carries and nothing else does: a **regular lattice** of skylights plus a **row**
  of air-handling units — **the deliberate exception to "a cell grid places a feature,
  it never *is* the feature"**: real skylights stand on the frame's columns, by the
  ruler.
- **Roofing** (`map/buildings/roofs.rs::roofing`) — the *shape* of a roof,
  **inferred**, not read. The private sector is **gabled** — five **`GableForm`s**
  (gable, half-hip, gambrel, lean-to, cross gable), a dormer, the **hip** now the rare
  case; everything else is **flat**. A **tent** is assigned, never chosen: a church and
  a fortress take their **`LandmarkRoof`** by faith and kind. **`RoofShape`** is the same
  kinds as an *input* — only `roof_gallery` asks for one, and a refusal there stays a
  refusal. **`shape_facts`** hands out the numbers the choice is made from. Detail —
  `buildings-heights.md`, `buildings-landmarks.md`.
- **Roof material** (`map/buildings/material.rs`) — what a roof is *covered with*:
  `RoofKind`, picked deterministically from `BuildingUse` (+ footprint size) and a
  **seed hashed from the building's first vertex**. The colour comes from the material's
  own palette — **the per-use roof and wall colours are gone**: a wall is picked the
  same way (**`WallKind`**, its own ten-slot table per use, plus `GarageDoors` by
  geometry and `Sacred` for a church). Texture from **`RoofMaterial`** (shader
  `roof.wesl`) reading the **`Roof` attribute** — **one code dictionary for all of it**
  (`0` no texture, roofings, garage runs, wall claddings, a door leaf). **Roof age** is
  the second thing the seed carries (patch count, water, fading) — **a wall has no age**
  (it returns before the common roof pass). **A wall's coordinates are the wall's own**
  (`meshing::WallFrame`): the vertex carries its **panel and storey in whole cells**, so
  edges never cut a panel and corner seams meet — the shader takes `fract` and needs no
  metres. The cladding decides the seams, the **openings** and whether there are
  **balconies**; **who gets balconies is decided on the CPU** and rides the seed's value
  (`WallFrame::marked`). **A door is not drawn by the shader's dice: it comes as
  geometry, from the data** — a patch plus the leaf (`layers::push_doors`); **an arch
  gets the same patch**. Above the last storey the wall keeps a **cornice** of plain
  cells. **A roof is darker than the walls under it, deliberately.** Detail —
  `buildings-material.md`.
- **Crown** (`map/buildings/temples.rs`) — what stands **above a church's roof** (cupolas
  on drums, a bell tower — **tiers** — a spire, a dome, minarets). **A standalone bell
  tower is boxless** — the whole building is that crown from the ground. Colours come
  from the tags first, the church's seed otherwise. **A crown stands on its church,
  whole** — the tower takes the church's own west projection, a cupola sits on the roof's
  platform, not the outline. **`Sanctuary`** is the city's churches assembled; a raised
  part is **boxless** too. **Crowns are laid after every building of the layer**, not per
  house. A fortress wall carries **merlons**. A **balcony** is a stack of bands laid out
  by the **period of a section**, not per column; **a window is the only thing here that
  replaces the surface colour**. Detail — `buildings-landmarks.md`, `buildings-material.md`.
- **Garage rows** (`map/buildings/garages.rs`) — a cooperative drawn as **rows of
  boxes**. Outlines within 2 m are stitched into a **run**; **every member takes one
  shared seed** — but **the geometry belongs to the piece, not the run** (`GarageRect`):
  each outline is **cut into near-rectangular pieces** at a reflex vertex, and the axis,
  the cell grid and the roof are each piece's own. A piece that reads as a garage is a
  **ribbon** (`RoofKind::GarageRow`, a seam on every bay boundary) or a **cooperative**
  (`GarageBlock`, bay seams plus a darkened aisle per row boundary — shading, not a
  hole). **The wall of a run says the same from the side** (`WallKind::GarageDoors`): a
  **gate in every cell**, only on the walls running along the piece's axis. **The grid
  is the piece's own, in whole cells, rounded down** — the remainder spread evenly over
  all the bays. Detail — `buildings-garages.md`.
- **Roof shadows** (`map/buildings/shadows.rs::roof_shadow_builder`) — the layer that
  carries each building its **taller neighbours' shadow sweeps clipped to its own
  footprint** (merged, not stacked), sitting **over** the building layers the ground
  shadow sits under. **A pitched roof takes none**; a neighbour casts only if it is 3 m
  taller; the drawn bodies of the neighbours are subtracted. Detail — `buildings-heights.md`.
- **Roof clutter** (`map/buildings/clutter.rs`) — what stands *on* the roof, each item a
  small oblique box with its own **opaque** shadow, placed by the building's seed, every
  candidate rejected unless all four corners fall inside the footprint **and the place is
  free**. Which items follow the **material**, not the use. **The clutter is the only
  thing zoom changes about buildings** — a negative material slot collapsed in the
  vertex stage, no rebuild.
- **Lean** (`map/buildings/mod.rs`, `Lean`) — which way the *top* of a building is
  displaced: **one oblique skew for every building**, which is what a **satellite**
  frame looks like. It carries the **base key of the draw order** (the far end written
  first); who covers whom is decided **pair by pair** (`order.rs::draw_order`), and once
  more **inside** a house (`wall_order`). **The sun is independent of it** — the lean is
  the camera, the shadow is the light. Detail — `buildings-heights.md`.
- **Sun** (`map/sun.rs`, `SunStyle`) — one light for the whole map, as **two knobs**:
  **azimuth** (default 300°) and **elevation** (default 59°, the summer noon of Tula's
  latitude), from which **`shadow_length_scale()` = cot(elevation)** and **`sun_stretch()`**
  — what every length calibrated at 59° is multiplied by. **It is read through a process
  global**, not a `Res` (the readers are pure functions deep inside mesh building); what
  the global holds is the ready shadow vector and cotangent, not the angles. **In the app
  only `apply_sun` writes it** — once in `Startup` after `seed_sun`, then every frame in
  `PreUpdate`; everywhere else a reader takes **`SunOnMap`** as a resource.
- **`SunOnMap`** — the sun the map is *built* with, as against `SunStyle`, the sun on the
  slider. `settle_sun` moves one into the other after 0.35 s of quiet, and it is
  `SunOnMap` that both the global and every rebuild follow — one division of the slider
  would otherwise cost a full building rebuild with its shadow union per step.
- **Soft shadow** (`map/buildings/shadows.rs::shadow_builder`) — every contour of the
  shadow union carries a **penumbra band fading to zero alpha**, **tapered per vertex by
  `penumbra()`**: a shadow meets its own building hard and blurs with distance — the
  contact edge gets no band at all. What goes into the union is still the silhouette
  sweeps, **built once per layer build** (`ShadowSweeps`) and handed to both shadow
  layers. Both carry **`BuildingShadowTag`**, not `BuildingLayerTag` — they rebuild only
  when the height mode or the sun changes.
- **Map seed** (`map/seed.rs`) — one Park–Miller LCG (`Lcg`) and one point hash
  (`seed_from_point`) shared by everything the map *layers* scatter. **The seed is the
  object's own reference point** — its first vertex — never its index in the extract, so
  a zoom rebuild, a height-mode switch and a restart move nothing. The parse stage keeps
  its own point-seeded `rng::lcg_seeded_by`.
- **Arclength walk** (`map/along.rs`) — `arclengths` + `place_on_path`, the one walk along
  a polyline's **whole** length, shared by every layer that places objects along linear
  geometry; the defect it exists against is a per-link walk that drops short links and
  resets at every vertex. **Both edges of `[0, total]` are inside.** `densify` lives
  beside it — original vertices kept exactly; the pair alignment keeps its own copy whose
  arclengths come off the OSM links.
- **Shape vocabulary** (`map/shapes.rs`) — the one dialect of `i_overlay` on the map:
  `Contour` / `Shape`, `ARC`, `RING_EPSILON` with `is_ring`, `oriented` / `area_contours`
  (the CCW-outer, CW-hole winding NonZero needs), the area helpers, `stroke` and
  `push_shape`. **`stroke` takes «closed ring» as an argument**: a road ring is stroked
  closed, the parse strokes its road pieces open whatever shape they come out.
- **Entrances** — real `entrance=*` nodes attach by exact vertex lookup; coverage is
  thin, so `map/osm/entrances/` **generates** doors for the rest. **How a building is
  doored is a property of the building** (`DoorPitch`): a dwelling by its **length** at a
  measured pitch, a **retail box** by its **perimeter** with wide floors — entrance
  groups; walls a neighbour stands against get none, **a door never stands in an arch**,
  and the result is deterministic per building. **Real doors are never moved** (an arch
  door is the one dropped) **and a half-mapped building is topped up**. A **section's
  подъезд is through** — the courtyard half added as its own `entrances` element, the
  cohort count unchanged. Detail — `entrances.md`.
- **Trees** (`map/osm/planting.rs`) — planted **only inside Wood polygons** plus
  standalone trees and avenues; deterministic LCGs seeded by geometry. **Planting runs
  once at the density ceiling**; the slider shows a monotone **prefix**. **Crown
  geometry** is all in `CrownParams` (`map/trees/crown.rs`); **the city is drawn with
  `CrownParams::default()`**, whose seed picks the **crown set**. **The crown is coloured
  by a material of its own** (`CrownMaterial`, shader `crown.wesl`). **Crown detail
  follows the zoom step**: near, an entity per tree; far, light fills **merged into
  chunks** — and **the trees are built once for every zoom step**, split by density band,
  a crossing flipping `Visibility` only. The near step's crown entities are **streamed**
  (`CrownStream`). Detail — `trees.md`, `tree-algo.md`.
- **Pitches** (`map/pitch.rs`) — sports and children's grounds, **over every road ribbon**
  and parking, under water: OSM runs yard footways across a field, and on a photo the
  field is whole. `PitchKind` decides the surface colour; **only `Soccer` and `Hard` get
  markings**, and only when the outline fills its `min_area_rect` frame.
- **Parking lots** (`map/parking.rs`) — an `AreaKind::Parking` area is asphalt (**over
  every road ribbon and sidewalk**, under pitches and water) with its **stalls drawn on
  it** in flat paint. `stalls_beside` lays them out under one law: **a car has to be able
  to drive to every stall.** Where the lot is driven OSM draws — **`service=parking_aisle`**,
  the only thing the layout reads out of the road network; stalls go **by the pocket
  between neighbouring aisles**, the stall's **depth is a field, not a constant**. A lot
  with no aisle gets an **invented** layout: rows along the **longest side of the
  outline**, `row — aisle — pair` across and `ROW_BLOCK` cross drives along. Three rules
  decide **which stalls exist at all**: a run of at least two, asphalt in front of the
  nose (**reachable**), and — on an aisle-driven row — clearance to the outline. **The
  markings and the cars read the same `ParkingLayout`** — computed once per world load,
  not per rebuild. **The lot's asphalt is the road's** (`PARKING_COLOR` *is*
  `roads::ROAD_COLOR`, same `SurfaceKind::Street`), so the lot has **no rim**; its
  outline clips every ribbon exactly. A lot under `MIN_AREA` gets no markings.
  Detail — `parking-layout.md`.
  **Big lot** (`LotKind::Ground`, ≥ 8000 m² paved) — the one kind that does **not** hide
  every road on it: a **through road** is shown by its **kerb** alone — **a polygon, not
  ribbons** (`roads/lots.rs`) — and a paved **median** gets its double solid again in
  `lot_lines`. **No stall stands under a through road or its kerb.** Detail —
  `parking-big-lot.md`.
  **Gore** (`roads/gores.rs`) — the splitter island at a roundabout: the wedge between
  entry, exit and ring is **asphalt with diagonal hatching**, not a triangle of sidewalk.
  A property of the network at every ring: a wedge counts only if it touches **two
  arms**, an arm closing only over its **fan**; the whole wedge is asphalt, the wedge
  without its thin tips is what gets hatched — the outline is an **area**, never a strip
  along the contour, so it cannot overlap itself. A **mouth** is the same island where no
  ring is; a **splitter** — the same island **by rule** on a two-way approach that leaves
  the ring along its ray; a **fork gore** — the same hatching ahead of the nose of a
  sharp fork away from a ring. **Closedness of a way is read off the raw OSM points,
  never off the drawn path** — a style knob must not decide whether a way is a ring.
  Detail — `parking-big-lot.md`, `roads-merges.md`.
- **Asphalt wear** (`surface.wesl`, `SurfaceParams::wear`) — an asphalt road on a photo
  is never one tone. **Wheel ruts in the lane frame**, so they follow the lane and not
  the compass, **zero-mean**; **kerb dirt was removed**; **repair patches were tried here
  and removed** — a per-cell fill is a chequerboard aligned to the compass; **don't
  reintroduce one**. The ruts **fade out in a junction gap**. Wear rides the **lane
  frame**: every carriageway gets one, whatever `RoadStyle.markings` says; the amplitude
  is its own knob. Detail — `roads-bridges-wear.md` (**Asphalt wear**).
- **Industry** (`map/industry.rs`) — what gives an industrial belt away from the air:
  **round spots and their shadows** (`MapData::structures`, cylinders — shadow as a
  **sweep**, the visible wall the **near** half, the top with a darker rim) and the
  overhead **pipelines** (`MapData::pipes`) — **only an explicitly above-ground pipeline
  is kept** (the inverse of the rail and waterway rule: an untagged pipeline is buried).
  Neither touches the navmesh. **`IndustryStyle::visible` is the whole style surface, off
  by default** — a toggle rebuilds this layer and nothing else.
- **Standing wagons** (`map/wagons.rs`) — wagons go **only on service track** (from
  `service=siding|yard|spur`), never on the running line; density follows the place — a
  station is a **fan** of neighbouring tracks and each rake takes the share that width
  gives. They stand in **rakes** with empty stretches between, stepped along the **whole
  track's arclength**. Decoration only, **no style knobs** — the layer comes off by zoom
  alone.
- **Fences** (`map/fences.rs`) — `barrier=fence|wall|retaining_wall|hedge` as a line and,
  more to the point, **its shadow** — in a private-house district the grid of plot
  boundaries is the texture of the whole district. The shadow is a **sweep**, and the
  sweeps of all fences are **unioned** (translucent shadows must not stack). **A gap is
  drawn as a gap** — the line and the shadow are laid from `footprint::fence_pieces`, the
  fence minus the same gap discs the navmesh opens, **and minus what a bridge deck
  covers** (`footprint::BridgeDecks`) — the one place the drawn fence and the blocking
  one part. The drawn width **grows as you zoom out** (`FENCE_LODS`); the layer
  disappears past 0.9 m/px. Detail — `layers.md`.
- **Bridge shadow** (`map/roads/bridges.rs`, `Z_BRIDGE_SHADOW` 2.05) — a bridge deck
  throws the same shadow every other object does, drawn under the bridge and over
  whatever it crosses — except what the z ladder draws above bridges. The invariants,
  all in `map/roads/bridges.rs`: **a bridge is a chain of ways, not one way** (what is
  glued is the *arithmetic* — the span and the distance to a free end — never the
  geometry); **height follows the span**; **a span under 35 m casts only over a gap**
  (water or rail, never a road); **the offset tapers to zero at the free ends** and is
  clamped by the span left ahead; **the band's cross-section stands across the deck, not
  across the offset path**; the band is 1 m wider than the deck; **the edge is soft**;
  **the cores of all bridges are unioned**, the penumbra laid per bridge before the
  union. Detail — `roads-bridges-wear.md` (**Bridge layers**).
- **Parked cars** (`map/cars/`) — a row of cars along every **carriageway** (the same
  `is_carriageway` that decides sidewalks and markings), minus bridges and roundabouts.
  The pitch is walked along the **whole street's arclength**; the row **breaks where the
  kerb pockets do** (`pockets::row_breaks`) and under a bridge deck. A **one-way**
  carriageway gets a single row on the driving side (`oneway=-1` is normalized at parse
  by reversing the way); **every car faces the traffic of its own kerb**. Cars also
  **fill the lots** — occupancy **falls with the lot's size**. **Yard rows** are the one
  place cars stand along a `service` drive, only among multi-storey housing, every body
  **probed**. **How densely either parks is decided by the district** (`Districts`): the
  area-weighted mean height within 120 m multiplies both shares. **A car is not a
  rectangle** (`cars/body.rs`): a rounded silhouette with a cabin, by **body type**; each
  casts its own swept shadow, **not unioned** with neighbours'. **Decoration only** — in
  no navmesh and no simulation, deliberately. One merged mesh, a zoom bucket dropping
  **detail** before the layer. Detail — `parking-parked-cars.md`.
- **The shadow rules** (`map/shadow.rs`) — seven layers cast a shadow (buildings, fences,
  cars, wagons, industry, bridges, roof clutter) and three things are common to all of
  them. **`length(height)`** is where `shadow_length_scale()` lives; **`offset(height)`**
  is that length as a vector — **the shadow starts under the object** and flows out from
  under it; **`penumbra(direction)`** is the soft-edge share — zero where the shadow meets
  what casts it, full at the far end. **`push_union`** unions the sweeps (NonZero, so a
  translucent layer never doubles on itself) and lays the tapered band. **The crowns are
  the eighth caster and stay outside it** — a crown's shadow length is a drawn length of
  its own. **The light stays a process global** on purpose: it is what lets one build run
  on the load thread, in a test and in the offline bench.
- **The uniform grid** (`map/grid.rs::Grid<T>`) — the one spatial index of the map.
  **The cell size belongs to the grid** (`Grid::new(size)`); `insert` puts the value in
  **every cell its box touches**, which is what makes a one-cell `at(point)` complete
  rather than approximate. `near` is **sorted and deduped** — an invariant, not a
  convenience: `HashMap` iteration order must not reach the geometry. `near_each` is the
  raw form; `pairs()` the candidate enumeration; `cell_of` the escape hatch for the one
  ring-walking caller. **`spatial.rs` is a different animal** and stays outside; so does
  `planting::Occupied`, on the opposite convention.
- **Footprint bands** (`map/footprint.rs`) — the strips linear geometry occupies on the
  ground, as **(centerline, width, role)** values plus the width policy. One
  construction, three consumers: the grid fill rasterizes, the mesh build outlines, the
  renderer draws its own smoothed copy. **A drawn band and a blocking band match by
  construction, not by discipline.**
- **Merged meshes** — one merged `Mesh2d` per layer: earcut triangulation, per-vertex
  colors over the flat `ColorMaterial` or the **surface** / **roof** materials;
  ~7000 buildings cost a handful of entities. Trees are individual entities on the near
  zoom step only; their far crowns and all their shadows are merged meshes cut by density
  band and 1 km chunk. Building shadows are one merged mesh.
- **The layer seam** — building a layer and putting it in the world are two things, and
  the line between them is `map/surface.rs`. A converted module offers **one pure
  function**, `mesh_<layer>(data, style) -> (Vec<LayerMesh>, Report)`: no `Commands`, no
  `Assets`, so the game, a test and the offline bench all call one and the same function.
  **`LayerMesh`** is one type for every layer of the map; **`MaterialSpec`** **names** the
  material instead of carrying a `Handle` — the only thing that would have dragged Bevy
  into the build. Every spawned layer entity carries the **`MapLayer`** marker — what
  `brp layers` / `hide` / `probe` select by. Two modules return more than a
  `Vec<LayerMesh>`: **`buildings`** (shadows carry their own tag) and **`trees`** (a
  scatter, not a merged mesh, on the near step). **A zoom cutoff and a visibility toggle
  belong in the build, not in the system**: **"not drawn" is a state of the report, never
  a zero in a counter.** **When a layer rebuilds is the layer's own business** —
  `rebuilds_on()` beside its `rebuild_*`; **one condition, one registration** (two copies
  in one schedule can both fire and spawn the layer twice — conditions are summed with
  `or_else`). Detail — `layer-seam.md`.
  **Ribbon** (`push_ribbon`) — constant-width band along a polyline with join/cap knobs;
  **a closed way is drawn as a closed ribbon** (`RibbonShape::closed`, taken from the
  path itself): a ring has no ends, so no caps. **Junction geometry is not computed as a
  union** — ribbons overlap in one opaque layer, an arm ending in a junction ends
  **square** on the node, and the kerb returns and outer corners fill the rest; **keep
  the road layer opaque, and its colour a function of world position only**. What *is*
  computed are **junction nodes** — a node shared by two or more carriageways, found by
  coordinate match on a 5 cm grid; they feed the markings.
  - **Drawn** (`map/roads/drawn.rs`) — the **prepared roads**: the map's roads as they
    are drawn, their shared nodes and street axes, stitches, tapers and merges, built
    once per `mesh_roads`; the fields are closed, consumers ask. The OSM roads stay
    beside it in `MapData`.
  - **Axis** (`Drawn::axis(i, Axis)`) — **which of a road's axes a consumer takes**,
    named in the type: `Nodal` (after smoothing, before stitches; ends are OSM points) or
    `Ribbon` (with the stitches). The OSM points are the third axis and stay on
    `RoadLine::points`.
  - **Pinned nodes** — Chaikin smoothing never cuts a shared node; the street axis pins
    only nodes shared with another **carriageway**.
  - **Kerb return** (`map/roads/corners.rs`) — the rounded corner between two arms of a
    shared node of **one class**, pushed into that class's fill layer **before** any
    ribbon. The radius goes by the **minor class of the pair**, scaled by
    `RoadShape::corner_radius`. **The sidewalk turns with it** — the same wedge on the
    band edges, on an arc of the **same centre**, the radius smaller by exactly the
    sidewalk width. A corner on curved edges is a **bend**; two arms 155°–178° apart at
    different widths get the **wide kerb's wedge** instead. All of it comes out as one
    `KerbReturns` value **laid by the module itself** (`KerbReturns::lay`) — **the layer
    order is `corners`' invariant**, and the fields are closed inside the module. Detail —
    `roads-corners.md`.
  - **Nose** (`corners.rs::nose`) — the rounded tip of the island between two arms of a
    **sharp fork**: an arc of a small radius where the two edges have parted by two
    radii, found by walking the arms' **axes**.
  - **Junction** (for the drawn asphalt) — a node's class group of three arms or more,
    or two meeting at an angle: an arm that **ends** there ends **square** on the node,
    unless it is a path arm whose cut no corner piece would cover; between two neighbour
    arms more than 180° apart the **outer corner** is a fan from the node.
  - **Merge** (`map/roads/merges.rs`) — a node where both halves of one pair end on a
    two-way way of their `Highway`. It is **not a junction** — no square ends, no
    corners — and each half's outer kerb runs into the continuation's kerb over a **merge
    wedge**. The paint runs through a **pure** merge; a merge **at a crossing** is drawn
    as the **pair up to the crossing** instead (`merges/mouths.rs::split_mouths`). A
    **Y-fork** is a merge too when its node is pure. Detail — `roads-merges.md`.
  - **Stitch** — a straight render-only segment appended to a **loose end** up to the
    centreline of the nearest road ahead, when that road's drawn edge is at most
    `STITCH_MAX_GAP` away and the segment crosses no building or water. A street is
    carried only by a street; a footway is carried by anything.
  - **Driveway crossing** — a footway way under 20 m both of whose ends are **ends of
    streets**: drawn as asphalt at the narrower drive's width. A crosswalk is not one.
- **Surface material** (`map/surface.rs`, shader `surface.wesl`) — the ground, the area
  layers, water and the road fills are drawn by **`SurfaceMaterial`**: the vertex colour
  stays the base, the shader multiplies in procedural noise **by world position** —
  identical in any two overlapping ribbons. **Every octave fades by pixel size**, so
  nothing shimmers — that rule lives in **`assets/shaders/noise.wesl`** and is imported
  by all three map shaders. One material per **`SurfaceKind`**; **`SurfaceStyle::texture`**
  scales amplitudes and retunes uniforms without rebuilding. A mesh for it is built with
  **`MeshBuilder::with_surface_coords`** — the **`Ribbon` attribute** in metres, zeros on
  polygons.
- **Rims** (`map/spawn.rs::push_area`) — every area polygon carries a gradient band along
  its contour, holes included. **Width is clamped to 0.6 × area / perimeter**, so a thin
  median strip never bleeds its rim onto the road.
- **Shoal** (`map/water.rs::mesh_water_areas`) — water has **no rim**: its lighter shore
  is a **distance field to the nearest bank**, over all water polygons **unioned** first.
  A **waterway ribbon** carries the same field on its edges from the shader.
- **Sidewalk profile** (`osm/model.rs`, `RoadLine::sidewalk()`) — one answer to "what
  sidewalk does this road have" for the parse and the renderer: the sides (the
  **`SidewalkSide`** tristate) and the **band by class**. **Derived from the current
  `width`, never stored** (the renderer's clones change the width). Three edges come off
  it, three on purpose: the **mapped edge** (blocks and lots pulled to it), the **verge
  edge** (houses pushed off it) and the **kerb edge** (a lot's stalls clear of it).
- **Sidewalks & markings** (`map/roads.rs`) — a **carriageway** (by class, never by
  width) is asphalt grey and gets a light **sidewalk band** at `Z_SIDEWALK` — under the
  street ribbon, **over the alley one**; **never a bridge deck**, which has its curb
  instead. Lane lines are the **Paint layer** above; where they stop is **Junction
  paint**. **Junctions** (`junctions::Junctions`) owns the marking breaks, the junction
  paint and the kerb row's breaks in one value — **the five sets of breaks a road
  carries, none merged into another; each consumer takes its own set as a type of its
  own**, so a swap is a compile error: the **base**, the **asphalt**, the paint **cut**
  and **solid**, and the **row** (`pockets::RowBreaks`). Wider fills are pushed after
  narrower ones, and a junction's **leading road** after all of its arms.
- **Style resources** — each is BRP-writable, persisted, and a change rebuilds only its
  own layers from the unchanged `MapData`: **RoadStyle**, **RoadShape** (`map/roads/shape.rs`,
  ranges beside it, clamped on read; the map follows **RoadShapeOnMap**, the copy that
  settles 0.35 s after the last slider step; **a settled new lane width is a world
  reload** — the parse reads it). **Any smoothing works on a *copy*** — `RoadLine::points`
  are load-bearing for navmesh, arches, planting and entrances. Then
  **BuildingHeightMode**, **TreeStyle**, **TreeRowStyle**, **ConiferNoiseStyle**,
  **SurfaceStyle** and **RoofStyle** (the last two: uniforms only, no rebuild).
  **`CrownParams` is deliberately not one of them.** Rail and tram answer to **no style
  resource** for their *geometry* — it is a function of the camera zoom (a **zoom
  bucket** each), so a smoothing knob that moved the centerline would slide the track
  against its own ballast. **TramStyle**, **CarStyle**, **IndustryStyle** — `visible`
  (± occupancy for cars) alone, each toggling its own rebuild and nothing else.
- **map_meshing** (`examples/bench/map_meshing.rs`) — offline measurement of the layer
  build, **from the Overpass cache with no window and no GPU** — that is the point: macOS
  App Nap slows an invisible window, and absolute numbers follow the machine's power
  state, so **a run is compared with a run, never with the log**. Run it after touching a
  building or car layer builder.

## Navigation

Summary; the mechanism and the measurements — **navigation-deep skill** (polymesh in
`references/polymesh.md`, grid fill in `references/grid-fill.md`, dispatcher & backends in
`references/pipeline.md`, separation & slots in `references/crowd.md`).

- **Navmesh** (`navigation/navmesh.rs` — the type, its storage and the world↔tile
  conversions; the work on it is split by role into `navmesh/{raster,fill,gates,reach}.rs`)
  — `Vec<bool>` passability grid, index
  `x * grid_size.y + y`, out-of-bounds reads impassable. `successors` — 8-way, diagonals
  only when both adjacent orthogonal tiles are passable (**no corner cutting**).
- **Fill order matters** (`fill_from_mapdata`): water areas block → **linear waterways
  block** (all but culverts) → **bridge curbs block** → **bridge decks carve passable
  strips back** → buildings block → walls block → **fences block, minus their gaps** →
  **building passages carve back through them**. Without bridges the Упа river bisects the
  map and no cross-river path exists.
- **Fence gap** (`footprint::fence_gaps`) — where a fence stays open: a non-bridge road
  whose **centerline crosses** the fence, **ends on it**, or is a **loose end dropped short
  of it** and aimed at it (the stitch's `STITCH_MAX_GAP`, since it is the same sloppy
  mapping), and a **default gate**. Never mere band overlap — a street running along a
  fence covers it end to end with its nominal width. A gap is cut **out of the fence
  mask**, never carved into the grid, so it cannot open the house or water it touches.
  A bridge opens nothing — a span runs over a fence, not through it — and the stretch it
  hides on the picture (`footprint::BridgeDecks`, see **Fences** under the OSM map) is a
  drawing cut only.
- **Default gate** (`FenceLine::gates`, `Navmesh::open_sealed_fences`) — a 3.5 m gate the
  load thread opens between fill and prune where a fence cut off a pocket that holds a door
  or is at least 400 m², on the fence side **nearest a carriageway**: OSM rarely maps
  gates, and schools, churches and works are fenced whole — and entered from the street. Stored in `MapData`, so the polygonal mesh, built later from it, gets the same
  gates.
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
  Without it, arch-only courtyards get sealed by the prune. **The carve reaches half its
  width past both ends in both fills** — the grid's capsule by construction, the polygonal
  mesh by extending the ribbon (`polymesh/build.rs::extended_ends`): a butt-cut ribbon left a
  hair of `city_wall` band at the gate towers' mouths, and the whole Tula kremlin became a
  dropped hole of the obstacle union.
- **prune_unreachable** — BFS flood from the portal; unreachable pockets become impassable,
  because an A* to an unreachable target floods the whole region (a 12 000 request backlog
  once "froze" the crowd).
- **ArcNavmesh** — `Arc<RwLock<Navmesh>>`; async tasks read it off-thread. Filled and pruned
  by the map-load thread while the loader is up.
- **PortalPos** (resource) — the actual portal position; the city's `*_PORTAL_POS`
  (`city.rs`) is only a hint,
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
  (urgent first, nearest-to-camera, cap `MAX_PATHFINDING_IN_FLIGHT` 1024). **The live
  dispatcher stands while the player pauses** (`live_dispatch_runs`) — urgent requests
  included; they pile up and leave on the first unpaused frame. The warmup pause is the
  exception: the warmup waits for exactly these answers.
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
- **Deterministic** (`determinism/mod.rs::Determinism`, panel toggle) — gates *scheduling*, not
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
- **Portal** (`portal.rs`, `assets/shaders/portal.wesl`) — a `Mesh2d` quad with a
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
  **Speed button** (`ui/speed.rs`), the **point inspector** (`ui/inspector.rs`, below the
  telemetry), the **City** select bottom centre (`ui/city.rs`), hotkey help
  (`ui/hotkeys.rs`), the agent **BRP** badge (`ui/brp.rs`). And **one settings panel**
  with four tabs (`ui/shell.rs`): **Map**, **Nav**, **Sim**, **Debug**. `Tab` (or the `-`/`+`
  button, or a click on the open tab) collapses it to the tab strip; the open tab and the
  collapsed flag are a persisted settings group (`UiShellState`). **Section order inside a
  tab is `SectionSlot`'s declaration order** (`sort_sections`), because the sections are
  spawned by one system per section plugin.
- **Point inspector** (`ui/inspector.rs`) — the HUD panel under the telemetry (**cursor
  ui** in the author's requests is this panel plus its highlight layer): the camera
  line `zoom/x/y cx/cy` (moved here from the telemetry, read from screenshots by
  `tools/shot_coords`) and the live list of what the map draws under the cursor, from the
  **semantic index** (`map/inspect.rs::InspectIndex` — a `Grid` over the `MapData`
  features, built once per load, rebuilt when `MapData` **or the road shape** changes; a
  hit carries the feature label and the z of the layer it draws in, top layer first; bare
  ground is **not** a hit —
  an empty list reads `-`). **Kerb parking pockets answer too** — `parking pocket
  (auto, tagged)` by the `parking:<side>` tag, `(auto, inferred)` by the sparse rule: they exist
  only as drawn geometry, so the index rebuilds their polygons on the nodal axis
  (`roads::pockets::kerb_pocket_outlines`, the same lift as the parked cars) and stores
  them itself — the one model answer computed on the drawn axis. **Cmd cycles the
  selection** through the hits under the cursor
  (1 → 2 → 3 → 1; a recompute resets it to the top) — the selected hit's row and, with the
  highlight checkbox on (it is **on from launch**, game and gallery alike — session-only,
  `InspectorHighlight`), the map layer both follow it. `InspectOrigin` — the
  world→map shift: zero in the game, in the roads gallery the shift of the **current**
  sample — the one whose viewport the cursor is in, else the framed one — so the panel,
  the highlight and `brp probe` all speak the current sample's map metres (`tools/osm_near`
  reads them directly). **`InspectViewport`** (`map/inspect.rs`, default `Whole`) is the cursor's viewport,
  written every frame beside the origin: `Inside` (the window in map metres — the panel is
  visible only while the cursor is inside it), `Outside` (between windows), `Whole` (the
  game: the whole map is one window). The **highlight layer** goes the opposite way of the
  panel — drawn at `map + InspectOrigin` (`LayerMesh::offset`) — and is clipped to the
  viewport (`InspectViewport::clip`): it never spills out of a sample window, even when
  the object is bigger. **A cursor over any UI panel stops being a map point**: the
  camera line reads `-/-`, the hits clear and the highlight despawns — the same
  `pointer_over_ui` gate the camera drag uses, because the world under a panel is not
  what the user is aiming at.
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
- **Debug tab** (`ui/debug/`) — the grid / doors / movepath / noise / **road network**
  (`DebugRoadNetwork`: every street its own colour, a dot on each seam) / **rut lines**
  (`DebugRutLines`: the lane axes with their two wheel lines at `RUT_OFFSET` in orange,
  the junction turn curves and tails in green — the two sources of asphalt wear, taken
  from the same road build as `RutLines`, never recomputed) / **OSM contours**
  (`DebugOsmContours`: the data before our passes, over the map) overlay rows, the
  `Camera start` and `Navtile` cyclers (global settings, deliberately not under a backend
  section), the **`Raw OSM`** cycler beside `Navtile` (**Raw OSM** above — a parse input,
  so it reloads the world) and **`reset`** (`prefs::ResetSettings`). The navmesh overlay is **one merged
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
  **`OffscreenShotEvent`** → the same frame rendered into an **offscreen texture** by a
  camera of its own, with its own centre, zoom and size, and written without the window
  server: a locked screen or a covering window turns the ordinary screenshot solid black
  and this one still works. It carries the real camera's **bloom**, but nothing drawn as UI:
  UI renders to the default UI camera, and that one targets the *window*, so the panels
  **and the vignette** (`post.rs` draws it as a UI node) are out of the frame and its
  corners are lighter than the window's. `IsDefaultUiCamera` on the main camera names that
  camera outright instead of leaving it to bevy's fallback; it is not what keeps the UI out
  of the shot. A **differing zoom is applied to the user camera for the duration of the
  shot**, because the zoom-LOD layers keep one mesh for all views and read their step from
  that camera; it is put back when the shot's camera despawns. For the frames of a shot the
  world holds a **second `Camera2d`**, so every query for the user's camera filters by
  **`With<PanCamera>`** — a bare `Single<…, With<Camera2d>>` then matches two entities and
  is skipped silently.
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
  **`ProbeLayersEvent` → `LayerProbe`** (`brp probe x y [r]`): what the map draws at a
  point — layers over it top-first with z, covering triangles and the nearest vertices —
  as one query, replacing hide-and-reshoot elimination and in-session temporary probes.
  The report ends with the **semantic index** section (the same hits the point inspector
  shows live): features of `MapData` at the point with their layer z — the answer to
  "what *is* this" beside the layers' "which mesh draws it". Works in the roads gallery
  too (it inserts `MapData` and the probe's input resources itself).
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

- World-wide tuning constants: `src/settings.rs` (sizes, speeds, radii, spawn rates,
  z-layers, the projection scale `METERS_PER_DEG_LAT`) — what belongs to the world as a
  whole, i.e. what more than one
  owner reads. **A number with exactly one owner lives beside that owner**, and "it is a
  constant" is not a reason to move it here. Not there: a number that *is* a rule of a
  decision ladder rather than a knob
  over it stays beside its `decide.rs` — `MAX_CHASERS_PER_TARGET` and the ×1.5/×0.7 switch
  factors in `demon/decide.rs`, `FLEE_STEP`/`FLEE_SPREAD`/`ESCAPE_MARGIN` in
  `human/decide.rs`. A constant both species declare moves to `settings.rs`
  (`WANDER_MAP_MARGIN`). A slider's `_MIN`/`_MAX`/`_STEP` range belongs to the resource
  whose field it bounds: the range is part of the model — it says which values that field
  may take, and the panel only renders it — the agent-radius range is exactly that and lives
  in `navigation/polymesh/`, where `PolymeshDebug::radius()` also clamps on read. Same split in the polymesh for the rest: the world-scale metres
  are in `settings.rs` under the `POLYMESH_` prefix (endpoint tolerance, map-edge
  margin, chunk sides), while the rules of the algorithm stay in `navigation/polymesh/` —
  `MAX_CHUNKS` (polyanya's layer-index width), the f32 tolerances (`SEAM_EPSILON`,
  `SEAM_QUANTUM`, `SIMPLIFY_EPSILON`, `WALK_*`), the search budget and `COST_SCALE`.
  Also not there: what a gizmo *looks like* and each visibility gate's own view margin —
  `MOVEPATH_COLOR`/`MOVEPATH_ARROW_TIP`/`MOVEPATH_VIEW_SCREENS` (`movement/systems.rs`),
  `DOOR_*` (`ui/debug/overlays.rs`), `VIEW_MARGIN` (`movement/mod.rs`) — they live beside
  the draw call or the gate, see `camera::Viewport::with_margin`. And not there either: the
  **geo anchor** — the `*_GEO_CENTER` per city and the `*_PORTAL_POS` hints live beside
  `City` in `city.rs`, its only reader; only `METERS_PER_DEG_LAT` stays, because it belongs
  to the projection rather than to a city.
  Detail — **species-behavior** and **navigation-deep** skills.
- OSM pipeline: `src/map/osm/{overpass,download,parse,model}.rs`; rendering:
  `src/map/{meshing,spawn}.rs`. Detail — **osm-map skill** (`SKILL.md` is the map; its
  `references/` carry the parse passes, roads, parking and cars, the other layers,
  buildings, trees, doors, the tag coverage audit and the crown-algorithm write-up).
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
- Tests: the integration suites `tests/navigation.rs` (synthetic navmesh + hand-built
  `MapData`), `tests/spatial.rs`, `tests/movement.rs`, `tests/determinism.rs`,
  `tests/map.rs`; everything else is a **unit-test module beside the module it tests** —
  a `tests.rs` in the module's own directory —
  `navigation/navmesh/{raster,fill,gates,reach}/tests.rs`, `map/seed/tests.rs`,
  `map/smooth/tests.rs`, `sim_time/tests.rs`, … — three dozen of them today.
