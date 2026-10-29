# Buildings — height modes and the extrusion pipeline

Siblings: `buildings-material.md` (roof/wall material and roof clutter),
`buildings-garages.md` (garage runs), `buildings-landmarks.md` (temples, fortress,
arches). The `map/buildings/` layer set and file map live in `SKILL.md`; the glossary
terms in `CONTEXT.md`.

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
      Everything reads it through the process global — `shadow_dir()`, `sun_light()`,
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
      gated on `retuned::<SunOnMap>`, never on `SunStyle`. The road layers are in that
      list because of the **bridge shadow** — the only thing in them the sun moves: its
      offset is baked into the merged mesh, so `rebuild_roads` is gated on
      `retuned::<RoadStyle>.or_else(retuned::<SunOnMap>)`.
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
        off the slopes as a dark rectangle across them. The price: a tall block no longer
        darkens the private houses' roofs next to it (its ground shadow still lies under
        them);
      - for each building, the union of its **taller** neighbours' sweeps
        **intersected with its own footprint** — one `overlay(Intersect, NonZero)` call,
        so overlapping shadows on one roof merge instead of stacking into double darkness;
      - filled with a plain `push_polygon`, **no `PENUMBRA_WIDTH` band** like the ground
        sweeps get (see **Soft edge** above) — deliberately: part of the intersection
        contour is not the shadow's edge but the cut along the roof outline, and a
        feathered band there would draw a light rim around every roof;
      - a neighbour counts only if it is `SHADOW_MIN_DROP` (3 m) taller. Below that the
        shadow reaches the eaves at most, and the pair test would run for nearly every
        pair in a city whose median height is one storey;
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
      and as its own `roof shadows`
      row in `examples/bench/map_meshing`, where the two shared steps in front of it —
      `order` and `sweeps` — have rows of their own, the way the car layer's `breaks` and
      `parking` do. Load-time only — nothing here runs per frame.
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
    five-storey wall came to lie over a nine-storey roof (pinned by
    `the_order_puts_the_leaning_neighbour_over_the_wing_it_covers`). What the
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
      the same list. Pinned by
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
    order of magnitude more to produce. Now `roofing` hands a
    hip only to `HIP_SHARE_OF_100` (3) of the houses of at least `HIP_HOUSE_AREA_MIN`
    120 m² and to outlines no **gable form** fits.
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
      overhanging it. Wing slopes are
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
    wear a rectangle sticking out of it, so it takes a **cross gable** instead. **Fill alone is
    not enough**, so the rectangle's corners
    must also lie on the walls — no corner farther than `GABLE_OVERHANG_MAX` 0.6 m from
    the outline (`gable_rect`, reported as `ShapeFacts::gable_overhang`): a skewed quad
    fills its rectangle well and still leaves a corner of the roof over nothing. Slope
    tone: base roof colour mixed toward
    white/black by the slope's plan normal against `map::sun_light()` (`SLOPE_LIT_MIX` 0.14 /
    `SLOPE_SHADED_MIX` 0.11, through the same `shade_by_light` helper as the walls, in
    sRGB), softer than walls. In 2.5D the ridge is lifted a further
    `ridge_rise(width) = min(width/2 × ROOF_PITCH 0.8, ROOF_RISE_MAX 5 m)` real metres
    through `ridge_lift` (same `EXTRUDE_SCALE`, no `EXTRUDE_RANGE` clamp), and the two
    gable triangles are drawn on the visible end walls with the wall's top colour before
    the slopes. In flat modes the ridge lift is zero and the two shades are all that
    remains.

## Inferred storeys

`CONTEXT.md` carries the branch tables (section, tower, low, private house, public,
retail); this file carries what it does not.

- **Inferred storeys** (`buildings/heights.rs`) — the height of the ~70 % of a city that
  OSM leaves untagged.
  - **The footprint decides**, the way it does for an eye reading an aerial photo, and
    only then the use: a **section** is a long thin box (≥ `SLAB_MIN_LENGTH` 35 m by
    ≤ `SLAB_MAX_WIDTH` 18 m) at 5 / 9 / 12 storeys with the weights a Russian city has
    (half of them five); a **tower** is compact and large (≥ `TOWER_FOOTPRINT_MIN` 500 m²,
    sides within `TOWER_MAX_RATIO` 1.7) and is mostly nine; a **low** building is
    ≤ `LOW_FOOTPRINT_MAX` 300 m² at 2–4 (over 300 m²: the area test comes first, so a long
    thin shed stays low); everything else large is 2–5 — that last branch
    is the most populated one and being generous with it is what made the first version
    come out skyscraping.
  - **A private house is one storey with an attic in the roof**: `HOUSE_HEIGHTS` is
    3–3.4 m in 8 slots of 10 and 6–6.4 m in 2, for `building=house` and for an untagged box
    up to `roofs::SMALL_FOOTPRINT_MAX` 250 m²; an untagged box up to `SHED_FOOTPRINT_MAX`
    40 m² is a yard shed at 2.4–3 m. An untagged box
    also takes the house wall table (`material::wall_kind_of`), not the low-rise one with
    its shopfront — at the same footprint ≤ `SMALL_FOOTPRINT_MAX` and under
    `LOW_RISE_STOREYS` (= `BALCONY_STOREYS_MIN` 4). **One constant for roof, walls and
    height**, owned by `roofs.rs` and imported by `material.rs` and `heights.rs` — the
    height rule must not grow its own footprint border, or an untagged box near the border
    comes out with a gable roof and house walls over 2–4 storeys
    (`an_untagged_box_under_the_pitched_cohort_border_is_a_private_house`).
  - **Some uses are measured in metres, not storeys**: an industrial hall or a store has
    one tall span, a church has one storey to the cornice, a garage is one box and gets no
    spread at all (a row of garage boxes on a photo is all one height).
  - **Retail is measured by its own size** — `BIG_BOX_HEIGHTS` 8–11 m for a big box
    (`model::is_big_box`), `SHOP_HEIGHTS` 4.5–6.5 for a corner shop, and the branch stands
    beside the industrial hall's rather than in the shape test, for the hall's reason: a
    hypermarket is one tall trading floor whatever its plan looks like. The big-box numbers
    are deliberately the same ones `parse::building_height` assembles out of
    `building:levels` — a box next to an identical box must not come out half as tall
    for want of a tag. The class and the threshold are in `parse-buildings.md`, **Retail box**.
  - **The slot inside a group is the building's own seed** — the same
    `material::building_seed` that picks the roofing material, so heights survive a mode
    switch, a sun rebuild and a restart, and two identical footprints in different places
    still come out different. **A tag always wins**; the inference runs only where
    `PolyArea::height` is `None`.
  - **`height_mix`** puts the result in the `building meshing:` log line — a height
    distribution is exactly the thing a screenshot cannot show. **Re-read it after every
    change to a table here.**
