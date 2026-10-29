# Panel bodies — the four tabs

The rows of **Map**, **Nav**, **Sim** and **Debug** — what each section holds, its
ranges, and the wiring and per-row laws the code does not say. The rows themselves the
code enumerates (`shape_knobs()`, `paint_knobs()`, the `spawn_knob` tables, the Nav tab's
builders); here is only the why. Shell, kits, camera and persistence live in `SKILL.md`.

## Map tab

Section order: Trees → Tree rows → Buildings → Roads → Road paint → Surfaces → Sun →
Noise.

- **Trees** (`ui/trees.rs`) — shape / foliage / crown details / color variance, one button
  per row cycling through a fixed palette (`bevy_ui` has no text input, so hex fields
  became cycles), plus **slider rows** — **density** over `TREE_DENSITY_MIN..MAX`,
  **conifer share** over `TREE_CONIFER_SHARE_MIN..MAX` and **noise mix** over
  `TREE_NOISE_MIX_MIN..MAX`. The kit's drag observer quantizes to the step and writes
  `TreeStyle` only when the step actually changes, so one drag rebuilds the crowns a
  handful of times, not once per pixel. The conifer-share and mix rows are
  `Display::None`d outside `TreeShape::Mixed` (`sync_mixed_row_visibility`) — they mean
  nothing for the other shapes. Also settable over BRP:
  `res set TreeStyle .shape '"Conifer"'`.
- **Tree rows** (`ui/tree_rows.rs`) — the alley-tree style (`natural=tree_row`),
  separated from Trees the way Buildings is: the tree-row toggle whole, the planting
  composition (placement policy, step source) and three green-underlay knobs, all cycle
  rows on `TreeRowStyle`; an edit rebuilds the tree set and the underlay.
- **Buildings** (`ui/buildings.rs`) — the `Height` cycler on `BuildingHeightMode`, the
  `Roof texture` knob on `RoofStyle` (a uniform, so the drag rebuilds nothing) and
  **Industry** — `IndustryStyle::visible`, **off** by default, the section's one row of a
  foreign resource, exactly the shape `Tram` has in Roads and for the same reason: a
  `BuildingHeightMode` case would remesh every building layer on a toggle whose only
  effect is one merged mesh (`map::industry::rebuild_industry`). It is read with the
  buildings because a cylinder stands on the ground and leans by the houses' own lift.
- **Roads** (`ui/roads.rs`) — the shape of the road first — five sliders on `RoadShape`
  (`map/roads/shape.rs`): **Lane width** (2.75–3.75 m, printed `3.30 m`), **Taper**
  (5–20, `10x`), **Curve tolerance** (0–5 m, `3.0 m`, `OSM` at 0), **Median gap**
  (1–6 m, `3.0 m`), **Corner radius** (0.5–2, `1.0x`). The table is `pub fn
  shape_knobs()`, re-exported as `qwe::ui::shape_knobs` and shared with the roads
  gallery, so the two panels' scales cannot drift. A drag writes `RoadShape` on every
  step but the map follows the settled `RoadShapeOnMap` (the SKILL.md debounce law — a
  rebuild of axes, pairs, nodes and the car row per step). Lane
  width, once settled, **reloads the world** (`city::reload_world` on `lane_width_moved` —
  the parse reads it). Then **Sidewalks** (a `RoadStyle` toggle). The old Joins,
  Smoothing and Casing rows went with the fields.
  Back in Roads, after Sidewalks: **Tram** — a cycle row of another resource
  (`TramStyle::visible`, off by default). The tram sits here because its track runs on
  the carriageway, and it is deliberately not a `RoadStyle` field: that would remesh
  every road layer on a toggle whose only effect is one merged mesh
  (`map::tram::rebuild_tram`). Under it, the second foreign resource in the section — so
  `add_knobs` is registered four times over (`RoadShape`, `RoadStyle`, `TramStyle`,
  `CarStyle`): **Cars** and **Occupancy** on `CarStyle` — a toggle (on by default) and a
  slider (0–100 %, step 5, printed as a percent), for the same reason and with the same
  shape, rebuilding only `map::cars::rebuild_cars`. Its drag needs no debounce, and that
  is measured: the knob kit quantizes to the step, so a full-scale drag is 20 rebuilds of
  4–7 ms each on Tula, not 400 — well under the ~30 ms at which
  `camera::track_camera_view`'s treatment would be called for. The **Occupancy** row is
  `Display::None`d while `Cars` reads `Off` (`sync_occupancy_row_visibility`, the Noise
  idiom — an unconditional `Update` system and a compare, no `resource_changed` window
  to miss on the first frame): with no row on the map there is nothing to tune, and a
  drag would still mark `CarStyle`, run `rebuild_cars` into its early return and rewrite
  `settings.toml` with nothing to show for it. The `Cars` row stays — it is what brings
  the layer back.
- **Road paint** (`ui/road_paint.rs`, `SectionSlot::RoadPaint` right after `Roads`, a
  `panel_title` header with no count) — what lies as paint on the asphalt. Four
  `RoadStyle` toggles — **Markings**, **Crossings** (`CrossingMode`: `Off` / `OSM` /
  `OSM + gen`), **Stop lines**, **Arrows** (the lane arrows, their own field now, off by
  default) — each rebuilding the road layers; interleaved with three `RoadPaintStyle`
  sliders: **Paint** (the paint layer's opacity, 0–100 %, step 5), **Wear** (the asphalt
  rut amplitude, 0–15 %, step 0.5, printed with one decimal) and **Turn wear** (the
  junction turn paths' rut amplitude, `map/roads/turns.rs`, 0–8 %, step 0.5, same
  print). The three are one table, `pub fn paint_knobs()`, re-exported as
  `qwe::ui::paint_knobs` and shared with the roads gallery like `shape_knobs` — an array
  the caller destructures, not loops over, because the `RoadStyle` toggles stand between
  its rows. Their own resource because all three are material uniforms
  (`surface::retune_surface_materials`) and a drag rebuilds nothing. A section of its
  own rather than the tail of Roads: with the shape sliders Roads would have grown to
  fifteen rows, and paint and shape are tuned apart — one by the junction's picture, the
  other by the street's. The roads gallery's panel mirrors both (a "Roads" and a "Road
  paint" header).
- **Surfaces** (`ui/surfaces.rs`) — one knob, **Texture** (`SurfaceStyle::texture`,
  0–150 %), the strength of the procedural surface shader; it rewrites material
  uniforms, so a drag costs nothing and rebuilds no mesh. A header without a count
  (`panel_title`), like Noise: the texture lies on the whole map.
- **Sun** (`ui/sun.rs`) — two knobs on `map::SunStyle`, azimuth (0–355°, step 5) and
  elevation (15–80°, step 1). They are the most expensive knobs on the panel: a change
  rebuilds the building layers **including the shadow union**, the tree crowns, the car
  layer and the roof material's uniform, because every one of those bakes the light
  into vertex colours or geometry — 77–86 ms on Tula for the buildings alone. So the
  steps are coarse, and the map follows the settled `SunOnMap` (the SKILL.md debounce
  law). The azimuth top is 355 and not 360 because 360° is the same sun as 0° — a scale
  with equal ends gives 73 positions for 72 directions and sticks instead of turning
  through zero. The elevation floor is 15°, not 0: `cot 5°` is 11 metres of shadow per
  metre of height, and every building would blanket its block; a value from outside the
  scale (an older `settings.toml`, a BRP write) is clamped on the read, in
  `SunStyle::elevation()`. The same two knobs stand in the `roof_gallery` example, over
  the same globals.
- **Noise** (`ui/noise.rs`) — the conifer-field fbm knobs (`ConiferNoiseStyle`:
  wavelength / octaves / lacunarity / persistence). In the Map tab, not Debug: the
  field decides where conifer stands land, so it is the look of the map, tuned next to
  Trees whose conifer share it distributes. Its first row is `Show` — the **same**
  `DebugConiferNoise` the Debug tab's overlay row carries, because tuning the field
  without the overlay is blind and sending the user to another tab for the switch would
  leave the section without one. The knobs (not the section) are `Display::None`d while
  the overlay is off. Noise *mix* is deliberately not here: it is a gameplay look knob,
  so it sits in Trees.

## Sim tab (`ui/stats.rs`)

Three sections. **World**: `Deterministic` and the `Seed` field + `new`. **Demon**: the
four `DemonStyle` knobs — **Max demons** (0…500, step 5), **Spawn every** (0.1…10 s,
step 0.1), **Speed** (100…200%, step 5) and **Lunge boost** (+0…+100%, step 5); both
percent rows print as percent, a bare `1.3` on the panel says nothing. **Human**:
**Speed spread** (0…35%, step 5) — printed with a sign because it is a half-width, and
a bare `15%` would read as "everyone 15% faster"; the sign is the ASCII `+/-`, not `±`
(the SKILL.md printing law). Body radius and the crowd knobs stood in this tab until
all six moved into the Nav tab's crowd groups — they are about movement.

## Nav tab

(`ui/navigation/` — `mod.rs` the rows, `knobs.rs` the crowd knobs, `overlay.rs` the
polymesh overlay.) One block, **one UI for both pathfinding backends**.

- **`Algo`** — the top row, cycles `Navmesh` ⇄ `Polymesh`: pawns always walk one of the
  two, so it is a choice, not two toggles that could both read `Off` while the grid
  quietly served every request. Its single source of truth is `PolymeshDebug::enabled`,
  which defaults to `Polymesh` — and which the `Separation` row below follows, since
  separation does not run on the grid backend (see the navigation-deep skill): picking
  `Navmesh` here greys that row out the way determinism does.
- **Backend-conditional rows.** Under `Algo` stand the settings **of the selected
  backend only** — the other set is `Display::None`d out of the layout
  (`sync_section_visibility`), because an agent radius means nothing while pawns walk
  tiles, and a grid search algorithm means nothing while they walk the mesh:
  - `Navmesh` → **`Pathfind`** (`PathfindingAlgorithm`, cycles A*/Dijkstra/Fringe/BFS/
    HPA*/Theta*), **`Show`** (the grid fill overlay, `DebugNavmesh`);
  - `Polymesh` → **`Show`** (mesh overlay, draws nothing else), **`Chunks`** (default
    on), and an **agent radius** slider (`POLYMESH_AGENT_RADIUS_MIN..MAX`, step 0.1 m)
    inflating obstacles at triangulation time.
- **`Chunks`** — the chunk hierarchy: it switches the *build* between layered and one
  flat layer (`POLYMESH_FLAT_CHUNK_METERS`) and therefore triggers a rebuild, and it is
  what puts the grid on the overlay; one toggle for both halves on purpose, since a
  grid drawn over a search that does not use it is a picture of something untrue.
- **Agent radius** — the minimum is deliberately non-zero (0.2 m) now that pawns walk
  the mesh, and it is read through `PolymeshDebug::radius()`, which clamps — the
  minimum was raised after the setting was already being persisted, so an older prefs
  file holds 0.0.
- **The polymesh overlay** — one merged mesh at z 5.3 (above the grid navmesh fill at
  5.2): **blocked contours filled** in the *same* red as `sync_navmesh_overlay`, then
  **all polygon edges** of the built mesh stroked over it (shared edges deduped, so a
  translucent seam is never double-painted). Same colour is the point — the two layers
  paint the same claim, and only an identical fill makes their accuracy comparable by
  eye; with a non-zero agent radius the gap between fill and edges *is* the inflation.
  The chunk-grid boundaries go on top of the mesh edges — dark, half-transparent, and
  the same 0.4 m stroke width as an edge, since the grid is a partition drawn over the
  geometry, not another layer of world. They are drawn unconditionally from the built
  grid, which is 1×1 (no lines) for a flat mesh — the overlay states what the search
  actually walks, not what the toggle asks for. Cache key (build generation + radius
  bits) lives on the overlay marker, the conifer-overlay idiom; chunks are absent from
  it because flipping them moves the generation. The two overlays can no longer
  collide — two red fills over one map read as a single layer at double alpha, and each
  is now drawn **only while its backend is the selected one** (`sync_navmesh_overlay`
  returns early when `polymesh.enabled`), so the mutual-exclusion system that used to
  push the toggles apart is gone.
- **The crowd groups** — `Separation` and `Slots` (`KnobGroup` — a plain enum, no
  resource and no component: it only sorts the knobs under their headers), holding
  every knob about how pawns get past each other and how they divide up end points.
  They live here rather than in World because both are about movement; World is the
  run (seed, determinism, counters), and the crowd knobs only ever sat there because
  the mechanism is species-independent. **Both groups are always expanded** — crowd
  knobs are tuned together, and hiding half of them would mean clicking back and forth
  mid-tuning; hiding is for what the current settings make irrelevant (the unselected
  backend's rows above), and these two always run. The `Separation` header *is* the
  toggle, exactly like `Algo`: `on`/`off` on the right, dimmed and inert (no hover
  highlight either) under determinism and on the grid backend
  (`separation_allowed_by_mode`), and its **knobs disappear** whenever separation is
  not running — determinism, the grid backend, or its own `off` — since there is
  nothing to tune while the mechanism never starts, the same rule that removes the
  unselected backend's rows. The header row stays: it *is* the toggle that brings
  separation back, and hiding it would lock you out. Their initial `Display` is set at
  spawn rather than left to `sync_separation_knob_visibility`, which runs under
  `resource_changed` and so does nothing on the first frame — with separation off at
  startup the sliders would have hung there until something else changed.
  Slots have no toggle — they run in both modes always — so `Slots` is a plain label,
  spawned without `Button` or an observer. Under the headers: **`Pass squeeze`**,
  **`Left share`** / **`Body radius`**, **`Slot search`**. Body radius is here despite
  living on `HumanStyle`: it sets both the rest distance and the slot side, so tuning
  wants it beside the other crowd knobs, not half a screen away in Human. All four are
  knob-kit rows, and each binds its **own** resource (`SeparationLab`, `HumanStyle`,
  `SlotSearch`) — the `Knob` enum that used to cover the group had to carry them all
  in one `SystemParam` for every drag. (`SlotLab` stays a demo-stand resource: the
  `Regroup` row that bound it was dropped together with the game-side
  `regroup_onto_slots`, which the wander invariant — every settled pawn carries
  `NeedsWanderTarget` — made unreachable in the game.)
- **Nesting** — nested *slider* rows are indented by `indent_slider_row`, which patches
  the padding the shared `ui/slider.rs` kit knows nothing about; without it a section's
  slider sat left of that same section's button rows.

## Debug tab (`ui/debug/`)

The overlay rows (grid / doors / move paths / noise field / **Road network** / **Rut
lines** / **OSM contours**), the `Camera start`, `Antialias`, `Navtile` and **`Raw
OSM`** cyclers, and `reset`. All rows are knob-kit rows (`spawn_cycle_row` +
`add_knobs::<R>()`) — a row that prints `Off` says its state outright, where the old
row of bare buttons (`grid`, `doors`, `camera:`) had no value text at all and needed a
green fill to say "on" and "at its default": the colour became a second, weaker
channel for the same claim.

- **`Raw OSM`** cycles `map::osm::parse::RawOsm` (`Off` → `parse` → `parse+draw`,
  persisted `debug.raw_osm`, registered by `MapPlugin`); it is a parse input, so it
  sits beside Navtile and reloads the world the same way (`city::raw_osm_moved`
  compares it with `MapData::knobs.raw`).
- **Road network** — `DebugRoadNetwork` (settings group `debug`, key `road_network`):
  `ui/debug/overlays.rs::sync_road_network_overlay` despawns the
  `RoadNetworkOverlayMarker` layer and, when on, spawns `map::mesh_network_overlay`
  through `surface::spawn_layers` (which is where its `DespawnOnExit` comes from) — on
  the toggle and on `OnEnter(Playing)`, never on a shape knob: the network is the
  parse's and the knobs do not move it. Each street its own colour, line width by
  lanes, a white dot on each seam.
- **Rut lines** (`DebugRutLines`, key `rut_lines`, right under Road network) — the
  same shape again, with one difference: its source is the `RutLines` resource the
  road build writes on every rebuild, so `sync_rut_overlay` runs on the toggle **or**
  on `resource_changed::<RutLines>` (no `OnEnter` entry — world entry writes the
  resource too), and a shape knob moves it (osm-map `references/roads.md`, **Asphalt
  wear**).
- **OSM contours** (`DebugOsmContours`, key `osm_contours`, under Rut lines) — the
  same shape over another layer: `sync_osm_contours_overlay` spawns
  `map::mesh_osm_contours(&map.osm_contours)` — the parse's snapshot of the geometry
  before its finishing passes (osm-map `references/parse.md`) — on the toggle and on
  world entry only, and logs its vertex count and build time.
- **`reset`** (`prefs::ResetSettings`) stays an action **button**, not a row: it has
  no state of its own to print — every settings group goes back to its `Default`,
  world settings included, so an off-baseline city / seed / navtile / determinism
  reloads the map on the click.
- **Navtile is in this tab**, not under `Navmesh`, because the world is *always* built
  in tiles of that size — the passability fill, the unreachable prune, the portal
  snap, the entrance generation — whichever backend the pawns then walk; hiding it
  with the grid settings would call a global setting a local one.
