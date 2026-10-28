# Buildings — temples, the fortress, arches

Siblings: `buildings-heights.md` (height modes, extrusion, shadows),
`buildings-material.md` (roof/wall material and roof clutter), `buildings-garages.md`
(garage runs). `CONTEXT.md` carries the **Crown** and **Sacred** glossary terms.

## Temples and the fortress

- **Temples and the fortress** (`buildings/temples.rs`, `buildings/fortress.rs`) — the two
  kinds of building drawn by their meaning rather than by the house rules.
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
    seed** (`Sacred::complex`, `material::look_seed`) — with per-part seeds a cathedral's
    parts come out pink, white and teal side by side; a size rule for gold was
    dropped with it, since a part does not know its host's area — and the gold the tag now
    gives those drums is what the palette had rolled as silver. `ORTHODOX_DOMES` is half
    gold, green, blue and the black of the red-brick Успенский; silver is gone — a grey
    cupola read as a shed's galvanised roof. Western
    ≥ 20 m: a spire tower at the west front. Mosque: hemisphere (R 0.3 of the short side) and
    1 / 2 / 4 minarets by area (500 / 2000 m²). Synagogue ≥ 300 m²: a low dome. Eastern: roof
    only. A crown ignores clutter gating — it is the silhouette, not equipment — and roof
    clutter (vents, penthouses, chimneys) is **refused** on churches and fortresses.
  - **A bell tower is tiers** (`Crown::Tower { tiers, top }`, `push_tower`), and the photos
    are the reason: a bell tower is a stack of receding storeys with a white cornice between
    them and open arches on the top one, and a plain box with a tent read as a water tower.
    `tier_count` gives a tier per
    `TIER_ASPECT` 1.6 narrow sides of pillar height, 1–3 (`TIERS_MAX`): a ~50 m kremlin
    tower three, a ship's 6 m tower two, a chapel's turret one. Each
    tier is `TIER_SHRINK` 0.8 of the one below **and never wider than `SHAFT_SIDE_MAX`
    12 m** (`next_tier`; `tier_count` measures by that shaft too): a standalone tower's
    outline is its ground storey *with* the porches and side chambers — a 28 × 24 m cross
    or a 24 × 24 square — and the shaft above it is ten to thirteen
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
    `shadows.rs` — where `raised` alone used to decide). The OSM `height` is the height
    **with** the spire, so the pillar takes `TOWER_PILLAR_SHARE` 0.72 of it and the spire the rest; an
    inferred height (`BELL_TOWER_HEIGHTS`, to the belfry cornice) keeps the spire on top of it.
    Minarets keep their box under `Crown::Minaret`, Jewish and Eastern towers their box
    under a tent.
    **And the church it belongs to grows no ship tower of its own** (`Sanctuary::towered`,
    `Own::tower` into `crowns_with`, `Own::default()` being a church standing alone) —
    otherwise a cathedral whose plan is just long enough for a "ship" stands a seeded tent
    next to its real bell tower. The cupolas
    stay; this is the twin of `domed` / `Own::domes`.
  - **A bell tower sits on the church's own west projection first** (`Plan::west_piece`, tried
    by `Plan::tower_seat` before the search below) — the porch, the narthex, the mapped tower
    base. Then its walls **are** the church's walls carried upward and there is no junction
    with the roof to draw at all, which is the only version of that junction that reads: a
    tower seated anywhere else ends its wall in the middle of the roof, and a tower a few
    tens of centimetres inside a wall leaves a sliver of roof along it. The outline is cut
    by a chord from every reflex vertex
    along its own wall (`garages::reflex_cuts`, the cross-gable trick), the pieces reaching the
    plan's west end within `TOWER_PIECE_REACH` 1 m are kept, and of those the **smallest**
    that fills its `min_area_rect` to `TOWER_PIECE_FILL` 0.9, is no thinner than
    `TOWER_SIDE_MIN` and no wider than `TOWER_PIECE_SIDE_MAX` 16 m wins. The rect's **own**
    axis is used, not the plan's — a chapel's walls can stand a couple of degrees off the
    plan, and a pillar on the plan's axis pokes out of them. Hence `Crown::Tower` carries
    a `size: Vec2` rather than a side: a church's projection is never square.
  - **Otherwise a tower stands on its church all four corners** (`Plan::west_tower`) — the roof
    clutter's own rule, and it did not hold by itself: `min_area_rect` describes the porches
    and the apse along with the church, so at an end with a porch the rectangle is longer
    than the building, and a tower flush with its west end hung over the ground. So the
    square is slid east in
    `TOWER_SEAT_STEP` 0.5 m steps, narrowed at each step to `TOWER_SIDE_MIN` 4 m, and nudged
    **across** the axis at each width (`nudges`, from the middle of the end outward), until
    all four corners are on the footprint (`point_in_area`, holes included). The order of the
    search **is** the layout rule: the tower holds the west front, so the westernmost step
    wins, on it the widest tower, and on that the smallest nudge off the middle. Each of the
    three freedoms answers its own case: narrowing keeps the west face where it is, so it
    does not help against a straight facade and does help against a narrow porch; the nudge
    is there because a mapped porch is rarely on the plan's centreline. The test square
    is shrunk by `TOWER_SEAT_SLACK` 0.05 m — on a rectangular church the rect's corner lies
    exactly on the wall, and without the slack every tower would slide off its own end — and
    that slack is also the worst overhang left, under a pixel at any zoom. A seat that never
    fits leaves the church without a tower (its Orthodox "ship" falls back to the plain nave
    layout), which is the honest answer for a plan that has no room for one.
  - **And so do the cupolas** (`Plan::dome_seat`) — the same rule broken by the same
    rectangle from the other end: the Orthodox core is the *eastern* slice of
    `min_area_rect`, and on a cross plan that slice is the apse and the air beside it, so
    the cupolas grew out of the walls. The cluster at full spread is moved **west from the
    altar** in the same 0.5 m
    steps, no further than the bell tower's east face (`room` — past it the cluster would
    crown the tower, not the church); failing that the spread shrinks (never under
    `DOME_SPREAD_MIN` 1.1 central radii, where the minor cupolas merge into the main one),
    and last of all one cupola is left. The order is the layout rule: five cupolas outrank
    the place, the place outranks the east end. A disc is probed at `DOME_SEAT_PROBES` 8 rim
    points, at the **belly** radius rather than the drum's, so the whole cupola stays over
    the roof — **plus `roofs::landmark_inset`**, because a cupola stands on the roof's own
    platform and not on the outline: a hip's platform is pulled in from the eaves by the
    slope's overhang, and a drum at the outline's edge stood on the slope and hung off the
    roof. `landmark_inset` is `landmark_rise`'s twin —
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
    (`shadows::same_church`): the roof-shadow layer lies over the building layer.
  - **Parts of a church are assembled at draw time** (`temples::Sanctuary`, built once per
    layer build from the whole list). A **raised part** — a `Dome` part with
    `Sacred::floor_dm`, or a drum (≤ 14 m) on another church of its `complex` (floor = the
    host's height) — **draws no box**: its crown is one drum from the floor (not stretched —
    the height is real) and a cupola, seated at eave `ZERO`. A church whose cupolas are
    mapped as parts grows **none of its own** (its minor cupolas used to pair up with the
    real ones). The raised part is
    skipped as a roof-shadow target and casts no box sweep. **All crowns of the layer are
    pushed after all buildings** (`push_crowns` over `(Crown, eave)` pairs): an annex laid
    after its cathedral hid the cupolas' base — cupolas sticking out from behind walls. The
    price: a crown can overdraw a nearer taller neighbour's wall, rare since a cupola tops
    everything around it.
  - **Tower against wall is the ordinary pairwise order, nothing forced.** The
    `building=wall` sections end at the tower outlines, so at a joint the section on the
    camera side covers the tower's foot
    and the one behind goes under it — exactly what `order::upper` decides.
  - **Merlons** (`clutter::merlons`) along every edge of a fortress wall's outer ring, pitch
    2.6 m, 1.3 × 0.7 × 1.9 m, through `push_items` (clutter zoom bucket).
  - **`temple_gallery`** (`examples/demos/temple_gallery`) — faith × (ship, square, chapel,
    bell tower, drum) plus a kremlin row, built by the real `mesh_buildings`;
    `TEMPLE_GALLERY_SHOT` and `TEMPLE_GALLERY_FOCUS=row,column,m/px` for a close-up.

## Arches

- **Arch rendering** (`buildings/arches.rs::arch_openings` + `push_wall_with_openings`) —
  a building `passage` (арка) is also cut out of the *drawn* building. The opening is a
  rectangle **in the wall plane**, found from the passage's **endpoints**, not by segment
  intersection: an OSM arch is typically mapped outline-vertex to outline-vertex, so the
  road lies inside the building and only its ends touch walls. At
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
  wall — without it that wedge of the hole showed the grass beside the road. The wall is
  the passage offset by half its width, only its runs
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
