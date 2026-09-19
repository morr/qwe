# Buildings — heights, landmarks, roofs

Detail behind `map/buildings/`. `SKILL.md`'s **Buildings** bullet under Rendering says
what the layer set is and which file builds which part of it; this one covers the height
modes and their shadow sweeps, the temples and the fortress, the storeys inferred where
OSM gives no height, and the roof and wall material with its shader, its clutter and its
arches.

## Height modes

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
  - **Shadows** — facade band plus a long shadow. **Both shadow layers live in
    `map/buildings/shadows.rs`**, not in `layers.rs`: `ShadowSweeps`, `shadow_builder`,
    `roof_shadow_builder`, `DrawnBodies` and the silhouette **chains** they are swept
    from, plus `SHADOW_LENGTH_RANGE`, `PENUMBRA_WIDTH`, `SHADOW_MIN_DROP` and
    `SHADOW_CELL`. `layers.rs` keeps the facades, the roofs and the extrusion — and with
    them `silhouette_edges`, the per-edge silhouette, which is a wall question and not a
    shadow one (`clutter` and `arches` call it). The layer itself is one translucent
    merged mesh at `Z_BUILDING_SHADOW` (4.5 — *below* every building layer, so a neighbour's roof or
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
      across it on a lateral one. Hence `shadow.rs::penumbra(direction) =
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

## Temples and the fortress

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
    neighbours' roof shadows on it (the four `boxless` sites — two in `layers.rs`, two in
    `shadows.rs` — where `raised` alone used to decide). Before this the box went up the whole height with church windows
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
    (`shadows::same_church`): the roof-shadow layer lies over the building layer, and the
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

## Inferred storeys

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
  - **Retail is measured by its own size** — `BIG_BOX_HEIGHTS` 8–11 m for a big box
    (`model::is_big_box`), `SHOP_HEIGHTS` 4.5–6.5 for a corner shop, and the branch stands
    beside the industrial hall's rather than in the shape test, for the hall's reason: a
    hypermarket is one tall trading floor whatever its plan looks like. The big-box numbers
    are deliberately the same ones `parse::building_height` assembles out of
    `building:levels` — half of Tula's crowd-format carries no level tag at all («Верный»,
    ТЦ «Перспектива»), and a box next to an identical box must not come out half as tall
    for want of a tag. The class and the threshold are in `SKILL.md`, **Retail box**.
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

## Roof and wall material

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
    in `SKILL.md` — and this shader imports by path the subset it calls
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
      Eight claddings —
      `Panel | Brick | Plaster | Shopfront | Shed | BigBox | GarageDoors | Sacred` — because panel seams with
      balconies are
      exactly **one** kind of building, and while the wall was one, a garage and a church
      wore them too. Six of them are chosen by the tables below; `Sacred` by the class
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
      in `entrances.md` next to this file), which is four panels — over the threshold, so every
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
  - **The big box is the cladding whose cell is neither a panel nor a storey**
    (`WallKind::BigBox`, code `14u`). A hypermarket facade is a curtain of composite
    cassettes hung on the frame's columns, so its cell along the wall is
    `layers::BIG_BOX_BAY` **7 m** (against `PANEL_WIDTH` 3.2) and its cell up the wall is
    `BIG_BOX_TIER` **5.5 m** — one trading hall with its technical floor, the same trick
    `SACRED_TIER` plays for a church and for the same reason. With the panel grid a 122 m
    wall of «Магнит» came out as forty columns and a ribbon window in each read as forty
    rows of flats; with the 3 m storey its 8 m shell came out as three floors of windows.
    - What the shader draws between the openings is **two lines and nothing else**
      (`CASSETTE_SEAM` on the bay edge, a horizontal joint every `CASSETTE_COURSE` quarter
      of a tier): a cassette is a large flat panel, and everything else on that facade is
      said by the band above it and the glazing below it.
    - The opening is a **ribbon of curtain glazing high under the parapet**
      (`BIG_BOX_WINDOW_LOW` 0.52 … `HIGH` 0.70, five panes), on
      `BIG_BOX_WINDOW_SHARE` **34 %** of the bays by the column roll — the share the
      shed's ribbon already uses, and low on purpose: a ribbon on every bay is a
      `Shopfront`, i.e. a downtown mall, not a box by the highway. There is no dwelling
      window on this cladding at all.
    - The **plinth is taller** (`BIG_BOX_PLINTH` 0.22 of a tier against `PLINTH_HIGH`
      0.14): under the glazing runs the dark band that hides the loading bays.
    - The **door is the widest opening on the map** (`door_size` 4.2 × 3.4 m): a group of
      glass leaves with a lobby, and you walk in with a trolley.
  - **The brand band is geometry, not shader** (`layers::push_brand_band`,
    `BRAND_BAND_SHARE` 0.26 of the drawn wall, top at `BRAND_BAND_TOP` 0.96) — one quad
    across every drawn wall of a `BigBox`, with `set_roof(None)` so no texture touches it,
    shaded by the wall's own outward normal through `shade_by_light` so the corner of the
    box does not vanish. **It is what the eye reads a hypermarket by** on every one of the
    reference photos — the coloured frieze running the whole perimeter — and it is the one
    thing that separates a retail box from a warehouse of the same size and shape.
    It cannot be a shader branch: a wall hands the fragment a **brightness and a glass
    amount**, never a colour of its own, and a frieze is a colour. So it arrives the way
    the **door** arrives, and for the same reason — a feature whose place or paint the data
    knows cannot be rolled in the shader. The colour is `building:colour` when the mapper
    set one (ТРЦ «Макси» is tagged `orange`, which is its real colour) and otherwise a
    five-slot chain palette by the building's seed — red, yellow, green, blue, orange, i.e.
    Магнит / Лента / Леруа / Метро / ОБИ; a sixth, magenta, was dropped on the first look
    at the frame, since no chain here is that colour. **This is the first place
    outside the temples that reads `building:colour`**; the note under **Tagged colours**
    in `SKILL.md` that only temples read them is now half true, and the private sector
    still does not.
    Pushed **after** its own wall and **before** the door, so the frieze lies on the facade
    and the entrance lies on the frieze.
    Its wall palette is deliberately quiet (`BIG_BOX_WALL_COLORS`: three near-whites, a
    cool grey and one anthracite) — the band is the colour of the building, and a bright
    cassette under it would fight it.
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
    is why they are gone (**Asphalt wear** in `SKILL.md`): only a minority
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

## Roof clutter

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
  - **A big box's roof is a different roof** (`model::is_big_box`, the size half of
    **Retail box** in `SKILL.md`), and this is the one place the size distinction is worth
    the most: from directly above, the roof *is* the hypermarket. Three additions, none of
    which a corner shop gets:
    - **The skylight grid** (`push_skylight_grid`) — `GRID_SKYLIGHT_SIDE` 2.8 m squares on
      a `GRID_SKYLIGHT_PITCH` 13 m lattice laid from the middle of the frame outward (so
      both edges of the roof keep an equal margin rather than one full step and one
      offcut), each tested by the usual `fit`, capped at `GRID_SKYLIGHT_MAX` 90. A nine
      thousand square metre trading hall is lit from above, and on every reference photo
      that regular field of pale squares is what the roof is made of.
      **This is the deliberate exception to "a cell grid places a feature, it never *is*
      the feature"** (the rule under `repair_patch` above). Real skylights sit on the
      frame's columns, by the ruler; jitter here would be an error, not life — the same
      argument that makes a garage run's bay seams a grid. `the_skylight_grid_keeps_its_pitch`
      pins it.
    - **The roof plant** (`push_plant`) — 2–5 air-handling units (`PLANT_SIZE` 3.6 × 2.2 m,
      `PLANT_HEIGHT` 1.8) **in one row**, not scattered: on the photos they stand as a
      single block, because they hang off one duct run, and spread over the roof they would
      be indistinguishable from vents. The whole run's envelope is `fit`-tested first, so
      the last unit of the row cannot hang off the outline.
    - **The vent cap goes 10 → `VENT_MAX_BIG_BOX` 26.** `VENT_MAX` was sized for a house,
      and on «Магнит» it put ten specks on a hectare of roof.
    **What the three cost**, measured before and after on one machine, `dev` profile,
    `examples/bench/map_meshing`, 2.5D+shadows+tint: **865 732 verts / 114.6 ms →
    883 917 / 115.2**, i.e. **+18 k vertices (+2.1 %) and +0.6 ms** for all eighteen of
    Tula's big boxes together. Load-time only, like the rest of the layer. The
    clutter-off row moved by **185 vertices** in the same runs — that is the brand band,
    a quad per drawn wall of a big box, and nothing else about the walls grew.
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
    `buildings::rebuilds_on()` the height mode uses — one registration with `or_else`, by
    the rule under **When a layer rebuilds** in `SKILL.md`.
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

## Arches

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
