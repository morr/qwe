# Buildings — roof clutter

Siblings: `buildings-material.md` (the roof material the clutter rides inside, the
cell-grid rule it is an exception to), `buildings-heights.md` (the zoom buckets and the
shadow sweeps it shares). `CONTEXT.md` carries the **Roof clutter** and **Skylight grid**
glossary entries.

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
  - **A big box's roof is a different roof** (`model::is_big_box` — the footprint, the
    storey count **and** the height ceiling, **Retail box** in `parse-buildings.md`), and this is the one place the size distinction is worth
    the most: from directly above, the roof *is* the hypermarket. Three additions, none of
    which a corner shop gets:
    - **The skylight grid** (`push_skylight_grid`) — `GRID_SKYLIGHT_SIDE` 2.8 m squares on
      a `GRID_SKYLIGHT_PITCH` 13 m lattice laid from the middle of the frame outward (so
      both edges of the roof keep an equal margin rather than one full step and one
      offcut), each tested by the usual `fit`, capped at `GRID_SKYLIGHT_MAX` 300. A nine
      thousand square metre trading hall is lit from above, and on every reference photo
      that regular field of pale squares is what the roof is made of.
      The **pitch is a target, not a constant** (`grid_pitch`, the same way `PANEL_WIDTH`
      is a target for a wall): a roof on which the 13 m lattice would not fit under the cap
      gets the pitch stretched instead — by the square root of the excess, at most
      `GRID_PITCH_TRIES` 6 times — so a giant roof comes out with a thinner grid over the
      whole of it rather than a full grid over its first
      rows and a bare half beyond them. `a_giant_roof_thins_the_grid_instead_of_cutting_it_off`
      pins that.
      **This is the deliberate exception to "a cell grid places a feature, it never *is*
      the feature"** (the rule under `repair_patch` in `buildings-material.md`). Real
      skylights sit on the frame's columns, by the ruler; jitter here would be an error,
      not life — the same argument that makes a garage run's bay seams a grid.
      `the_skylight_grid_keeps_its_pitch` pins it.
    - **The roof plant** (`push_plant`) — 2–5 air-handling units (`PLANT_SIZE` 3.6 × 2.2 m,
      `PLANT_HEIGHT` 1.8) **in one row**, not scattered: on the photos they stand as a
      single block, because they hang off one duct run, and spread over the roof they would
      be indistinguishable from vents. The whole run's envelope is `fit`-tested first, so
      the last unit of the row cannot hang off the outline.
    - **The vent cap goes 10 → `VENT_MAX_BIG_BOX` 26.** `VENT_MAX` was sized for a house,
      and on a hypermarket it put ten specks on a hectare of roof.
  - **Placement** is the shared Park–Miller LCG — `map/seed.rs::Lcg`, one copy for the
    whole of `map/*` (crowns, this clutter, the parked cars), the crown generator's
    original lifted out of `map/trees` — seeded from the roof material's building seed
    (`seed::seed_from_point`, the same door), so the
    equipment survives a mode switch, a sun rebuild and a restart in the same
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
    rolled independently. It is a cheap check because **every item of a flat
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
    missing part of the union, and no convex hull has to be built.
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
    it lives in the same merged mesh as the houses (painter's order is per building:
    walls, roof, then its own clutter — a nearer neighbour's wall covers a farther roof's
    shaft). Buildings have a zoom bucket of their own — `BuildingLods` /
    `BuildingZoomBucket`, two steps at `ROOF_CLUTTER_MAX_ZOOM` (0.5 m/px) — and **the
    crossing rebuilds nothing**: the game always builds the layer with clutter
    (`BuildingPlan::game`), `push_items` marks every clutter vertex by a **negative**
    material slot (`MeshBuilder::set_clutter`, `[0, 0, -1, 0]`; the fragment reads the
    slot through `max(…, 0)`, so it is still "no texture"), and on the far step
    `material::show_roof_clutter` sets `RoofParams::clutter` to 0, on which `roof.wesl`'s
    vertex stage sends those vertices to one point past the far plane — degenerate,
    clipped, no fragments. It runs every frame (one asset read and a compare; it writes
    only on a crossing), so the unflagged `seed_zoom_bucket` needs no second call.
    **Rejected alternatives**: a separate clutter layer on top — it would break the
    painter's order and show a low roof's shafts through a taller nearer wall; two
    prebuilt variants of the extruded mesh with a `Visibility` switch — twice the build on
    every mode/sun change and +1 M resident vertices for nothing the shader cannot do.
    The cost that stays: on the far step the clutter vertices still go through the vertex
    stage every frame, and the far-zoom mesh stays at its near-zoom size in memory.
