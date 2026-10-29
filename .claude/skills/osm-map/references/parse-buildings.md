# Parsing — the building readings and passes

The building half of the parse: the tag readings (`parse/tags.rs`) and the passes that
move or re-classify a building outline (`parse/faiths.rs`, the building passes of
`parse.rs`). The seam they hang on — `ParseKnobs`, `read_elements`/`finish_parse`, the
pass order, how a parse is tested — is `parse-seam.md`; the geometry passes are
`parse-geometry.md`.

## Readings

- **Building height** (`parse/tags.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim, or else
  `building:levels` + `roof:levels` × `settings::STOREY_HEIGHT` (3 m). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — and it is the majority in
  every city but the LiDAR-imported ones, so what fills it in matters: see
  **Inferred storeys** in `references/buildings-heights.md`. Coverage is logged per city
  on load (`N buildings (M with height)`).
- **Building use** (`parse/tags.rs::building_use`) — `BuildingUse: House | Apartments |
  Commercial | Retail | Industrial | Garage | GarageBlock | Church(Sacred) | Public | Other`, the class that
  picks two material tables — the **cladding** (`buildings/material.rs::wall_kind_of`) and
  the **roofing material** (`::kind_of`); neither colour comes from the class itself, both
  come from the chosen material's own palette (**Roof material** in
  `references/buildings-material.md`, bullet
  **The pick**). Three sources in order: `building=*` when the value names a *particular*
  building (`house`, `apartments`, `garages`, `church`,
  `school`, …), else a big-format **`shop=*`** (`is_big_format_shop`, **Retail box**
  below) — which also overrides the one generic value that does map to a class,
  `building=commercial`, so «Магнит» is a box and not an office — else `amenity=*` on the
  same outline (`school`, `hospital`, `police`,
  `place_of_worship`, …) — a school or a hospital in OSM is almost always `building=yes`
  + `amenity=…`. Anything outside the vocabulary is `Other`, and `Other` is read by shape
  rather than left flat: the wall takes the apartment-block table (height having spoken
  first — under `LOW_RISE_STOREYS` it is the low-rise one whatever the tag), the roof
  takes the private-sector table for a small footprint and the apartment-block one above
  it. The vocabulary covers what a city carries by the hundreds, not the OSM wiki.
  **`garages` and `garage` are two different
  classes**, and that is not pedantry: the plural is how OSM maps a *whole cooperative*
  as one outline, and drawing it as one shed is what made the ГСК look like a hangar —
  see **Garage rows** in `references/buildings-garages.md`. The
  singular, together with `carport`/`shed`/`barn`/`roof`, stays one box.
  `roof:shape` is **not** read; the roof shape is inferred instead — see **Gable roofs**
  in `references/buildings-heights.md` — with one exception below. The class is
  also one of the two inputs of **Inferred storeys** (the other is the footprint's shape),
  which is what fills in the height OSM does not carry.
- **Retail box** (`parse/tags.rs::is_big_format_shop`, `model::is_big_box`) —
  `BuildingUse::Retail`, the building that **is** a shop, and the one class where the
  **footprint decides as much as the tag**. Split off `Commercial` (which keeps the office,
  the kiosk and the pavilion) because an office block and a hypermarket share nothing from
  the air: the office has dwelling-height storeys and rows of windows, the box one tall
  trading hall, a blind facade under a brand band and a roof of skylights and plant.
  - **The tag.** `building=retail|supermarket|mall|department_store|shop`, and — under
    `building=yes`, which is where half of it lives — a **whitelist** of `shop=*`:
    `mall`, `supermarket`, `department_store`, `wholesale`, `doityourself`, `hardware`,
    `trade`, `garden_centre`, `furniture`, `car`. A whitelist for the reason every other
    one here is: `shop` has a hundred values and nearly all of them are a **point inside
    somebody else's house** — a bakery, a florist, «продукты» on the ground floor of a
    nine-storey block. **No `QUERY_VERSION` bump** — `out geom` returns every tag of the
    element, so `shop` has been in every cache since v1.
    **`building=shop` sits in the `Retail` arm, not the `Commercial` one**, because in OSM
    it is a plain synonym of `building=retail` — the building that is a shop — and the
    whitelist a line below already calls `shop=mall` and `shop=department_store` retail. The
    value is a *particular* one, so a small `shop=*` on the same outline no longer overrides
    it.
  - **The size, the storeys and the height** (`is_big_box`: `Retail` with a footprint ≥
    `BIG_BOX_AREA_MIN` 1200 m², **and** `PolyArea::storeys` at or under
    `BIG_BOX_MAX_LEVELS` 3, **and** a height at or under `BIG_BOX_MAX_HEIGHT` 17 m — each
    of the three can refuse on its own).
    OSM never marks crowd-format, so it is read off the pattern, exactly as
    `is_fortress_tower` reads a tower off its compactness. **The threshold is a chosen
    line, not a gap the data hands you**, and that is worth knowing before moving it.
    What 1200 m² says is that big format starts at a hypermarket of 1300–1400 m²
    («ДА!» 1363, «Верный» 1628), while a mall block of a thousand-odd still reads as an
    infill from the air; the price is that two buildings of nearly the same footprint are
    drawn differently, which is the cost of any single number here.
    **The storey count is what actually separates a mall from a box, and that is why
    `PolyArea` carries it** (`storeys`, `building:levels` as the tag says — see
    **MapData** in `SKILL.md`). No number in metres runs between a four-storey mall (measured
    in dwelling storeys, 12–15 m) and a real two-level box (12.5 m); `BIG_BOX_MAX_LEVELS` 3
    does, and the same constant is what `building_height` measures the shell by, so «measured as
    a box» and «drawn as a box» stay one statement rather than two that can part.
    **The height ceiling stays, as the answer for an unmapped storey count**
    (`BIG_BOX_MAX_HEIGHT` 17 m — the same three trading levels of `BIG_BOX_LEVEL_HEIGHT`
    4.5 m plus `BIG_BOX_SHELL_EXTRA` 3.5 m of shell, written in metres): a building with a
    verbatim `height` and no `building:levels` has nothing else to be judged by.
  - **What it decides.** Four things, each with its own bullet elsewhere: the **height**
    (below), the **roof material** (`BIG_BOX_ROOFS` — light membrane / bitumen / gravel —
    against `SHOP_ROOFS`), the **roof clutter** (the skylight grid and the roof plant, in
    `references/buildings-clutter.md`) and the **wall** (`WallKind::BigBox` with its brand band,
    `references/buildings-material.md`). A small shop gets none of it and keeps a shopfront over a house window.
  - **The height** (`building_height`, and it is why that function takes the outline as
    well as the tags). A trading level is `BIG_BOX_LEVEL_HEIGHT` 4.5 m, not the 3 m
    dwelling storey, and over the top one sit `BIG_BOX_SHELL_EXTRA` 3.5 m of technical
    floor and parapet — so `building:levels=1` on a hypermarket is 8 m, not 3.
    **Only up to `BIG_BOX_MAX_LEVELS` 3 trading levels** (and only while the shell they
    produce fits under `BIG_BOX_MAX_HEIGHT` — the same statement in metres, which here
    catches a `roof:levels` the storey threshold does not count) **and only from
    `BIG_BOX_MIN_LEVELS` 1 up**. Both ends are
    load-bearing. Above: a `shop=supermarket` mapped with `levels=9` onto the whole panel
    block it occupies the ground floor of would become a 45 m tower under a trading level —
    so the ordinary dwelling storey applies, which also happens to be right for the
    multi-floor malls (6 levels → 18 m, 5 → 15).
    Below: `building:levels=0` is abandoned tagging rather than a building, and without
    the lower bound `BIG_BOX_SHELL_EXTRA` would lift that zero to a plausible 3.5 m — a
    hypermarket drawn as a slab one storey tall, which is the very defect this bullet is
    about. Out of the range at either end the tag is read as the ordinary storey count, and
    a zero there stays implausible and falls through to the inferred height.
    **Both thresholds are the constants the predicate reads**, and that is the point: a
    house measured in dwelling storeys because it has a fourth one is, by the same number,
    not a box downstream either (`a_shop_on_a_tall_block_is_not_a_big_box_either`,
    `four_storeys_of_mall_are_not_a_two_level_box`).
  - **The untagged half is inferred to the same numbers** (`heights.rs`: `BIG_BOX_HEIGHTS`
    8–11 m, `SHOP_HEIGHTS` 4.5–6.5), deliberately — a box standing next to an identical box
    must not come out half as tall for want of a tag.
- **Places of worship** (`parse/tags.rs::faith`, `sacred_form`;
  `parse/faiths.rs::resolve_faiths`)
  — `BuildingUse::Church(Sacred { faith, form })`. `building=bell_tower|campanile|minaret`
  and `tower:type=bell_tower|minaret` are churches too (`form: Tower`); a part with
  `roof:shape=onion|dome` is `form: Dome` — the only reading of `roof:shape` in the parse.
  **Tagged colours** (`tags::area_colours` → `PolyArea::colours`, every building, not only
  churches): `building:colour` and `roof:colour` as sRGB bytes. `tags::colour` takes a hex
  value as is — the mapper picked it off a photo — and a **CSS name as the map's own paint**
  (`CSS_COLOURS`: `white` is whitewash, `blue` the roof palette's blue, `darkgray` darker
  than `gray` as a mapper means it, not lighter as CSS has it), because `#0000FF` on the map
  is a marker, not paint; an unknown name is `None`, never a guess. **Two things read
  them**: the temples (`temples::tagged_wall` / `tagged_roof` / `tagged_dome`,
  `references/buildings-landmarks.md`) and
  the retail box's **brand band** (`layers::brand_color`). Reading them on every house is a
  palette decision the private sector has not made.
  Faith: `religion=christian` + an Orthodox-family `denomination` → `Orthodox`, any other
  denomination → `Western`, none → `Unknown`; `muslim|jewish|buddhist|hindu|shinto|…` by
  religion, else by `building=mosque|synagogue|temple`. **`resolve_faiths`** runs right after
  the drowned-building pass and
  assembles churches from parts: a part's **host** is the largest larger church holding its
  centre, else the nearest larger one within `CHURCH_PART_REACH` 30 m (a bell tower stands
  beside, not inside). Hosts are followed **up the chain**: a bell tower whose nearest larger
  church is a part of a cathedral (a part sticking out past the outline, or tied with it on
  distance) belongs to the cathedral. The part takes the top host's first-vertex seed as
  `Sacred::complex` — what its colours are picked by — and, when untagged, the faith of the
  nearest host up the chain that has one; the rest take the
  city majority (Orthodox vs Western, ties → Western). `Sacred::floor_dm` is `min_height`,
  else `building:min_level` × 3 m (`tags.rs::part_floor`).
  **Annexes** (`parse/faiths.rs::absorb_annexes`, same pass, after hosting): a `BuildingUse::Other`
  building of `AreaKind::Building` that lies on a church **or an annex** — its centre in the
  host, the host's centre in it, or `ANNEX_OVERLAP_SHARE` 25 % of its own footprint
  shared (`overlap_area`, an `i_overlay` intersect) — at most `ANNEX_AREA_RATIO` 2.5× the
  host, becomes `SacredForm::Annex` with the largest such host's faith and complex, in up
  to `ANNEX_ROUNDS` 3 rounds.
  Only `Other`: a house, a school or a shop keeps its class however it lies.
- **Fortress** (`parse/tags.rs::is_fortification`) — `AreaKind::Kremlin` from
  — on an outline carrying `building` only (`area_kind` asks nothing else) —
  `historic=citywalls|castle|city_gate|fort`, `barrier=city_wall`,
  `man_made=tower` + `tower:type=defensive`, or `building=wall` at ≥ 6 m
  (`FORTRESS_WALL_MIN_HEIGHT`; lower is a garden wall).

## Finishing passes

- **Drowned buildings** (`parse.rs::drop_buildings_in_water`, step 1) — a building whose
  outline lies **entirely** inside a water polygon is dropped right after the element loop,
  before doors and trees. OSM tags floating restaurants and moored ships as buildings
  (`HMS Belfast`, `Café Barge`); the
  navmesh floods water impassable, so their doors are unreachable anyway and the box
  standing on the pond reads as a render bug. One vertex on land is enough to survive —
  piers and embankment houses stay. Logged on stderr when non-zero.
- **Squared houses** (`parse/squaring.rs::square_skewed_houses`, step 4) — a small house outlined as a
  **skewed quad** is replaced by a rectangle. The private sector is traced by eye off
  imagery, and a rectangular house comes out a rhombus: in 2.5D its ends stand askew to its
  front and no gable fits it. Taken: a
  4-vertex convex outline, `AreaKind::Building`, no holes, an `Other` of at most
  `SQUARE_AREA_MAX` 250 m² (the pitched-cohort threshold) or a `House` of at most
  `SQUARE_HOUSE_AREA_MAX` 400 m² (the tag already says private house, and a house is
  pitched at any size), with its worst corner
  `SQUARE_SKEW_MIN` 2° … `SQUARE_SKEW_MAX` 35° off square — under 2° the trace is
  already straight, over 35° it is a trapezoid by the plot. The ceiling went up from 20° on a
  measurement: **every** lone small quad above 20° was such a trace, none a real
  trapezoid — the shift cap is what guards the rest. The rectangle keeps the
  **centroid and the area**: the axis is the length-weighted mean of the edge directions
  with the angle ×4 (so both axes vote for one), the sides the mean lengths of opposite
  edges along it, scaled to the area; vertex `i` becomes corner `i` with the winding kept.
  **A skewed L is straightened too** — a six-vertex outline with exactly one reflex corner
  (a house with a wing), under
  the same area, skew and shift gates. `fit_ell` uses the same `outline_axis`; every edge
  goes to the nearer axis (they must alternate, or it is not an L) and gets a **level** —
  the mean of its two ends across that axis — and vertex `i` stands where its two edges'
  levels cross. So each wall lands halfway between its traced ends; the area is not
  rescaled, and a fit whose area drifts over `ELL_AREA_DRIFT` 15 % or that does not come
  out with exactly one reflex corner is left alone. `corner_skew` measures a reflex corner
  against 270°.
  Skipped when any vertex is **shared** with another outline or line (terraced houses, a
  fence along the wall, an arch — squared, they would open a gap) or when a vertex would
  move over `SQUARE_SHIFT_MAX` 3 m. **Which outlines count as sharing is
  an explicit list, not "every outline on the map"** (`vertex_uses`): the buildings, all
  the line layers, and **three of the eight area layers** — `parking`, `pitches`,
  `water`. Those three are drawn as a surface of their own with markings on it, and a lot
  carries parked cars as well, so a house that steps off that edge reads on the frame;
  `parks`, `grass`, `woods` and `sand` lie
  *under* the house and show no seam, and a `landuse` block's outline is not counted
  because private houses are routinely traced onto the block's boundary.
  `Obstacles` (the pull below) carries its own, narrower list and does not close this
  hole — it is about what a moving house bumps into, not about a vertex two outlines share.
  Runs after `attach_entrances` (the door order fact is the seam's,
  `parse-seam.md`) and before door
  generation and tree planting. The price: the render seed is the first vertex, so a squared house rolls
  its material and inferred storeys anew.
- **Houses pulled off the sidewalks** (`parse/house_pull.rs::pull_houses_off_sidewalks`, step 5) — the street's
  width is a class constant and the sidewalk is added by the renderer
  (`RoadLine::sidewalk()`, the band by class), so an old house standing at the kerb in OSM came out with its
  wall on the drawn sidewalk and, in 2.5D, its roof on the asphalt. The game does not
  need metre accuracy, and a house on the pavement reads as a bug, so the house moves.
  - **The band** is every non-bridge `RoadLine::is_carriageway` link out to its
    **verge edge** — `SidewalkProfile::verge_edge(width / 2, SIDEWALK_CLEARANCE)`: the
    band by class plus 2 m, **the tag not read** (a side `sidewalk=no|separate` took the
    sidewalk from keeps a verge instead; pinned by
    `the_parse_reads_the_mapped_edge_for_blocks_and_the_verge_for_houses`), in a `SIDEWALK_CELL` 32 m grid; raw OSM
    points, not the smoothed centreline — the difference is centimetres. The clearance is
    2 m because the 2.5D roof leans toward the street by another half metre to a metre.
  - **The row moves, not the house** (`Front`, union-find): each eligible building's deepest
    band intrusion names its street way and the side of it; fronts with `need >
    -ROW_SETBACK_TOLERANCE` 2 m on the same (way, side) whose bounding boxes are within
    `ROW_GAP` 20 m join a row. The row's shift is its deepest `need`, and every member whose
    own `need` is within `ROW_SETBACK_TOLERANCE` of it takes that same shift along its own
    away-direction — a neighbour standing on the line but not quite on the band moves too,
    one standing deeper in the block stays. A lone house pulled back breaks the facade
    line into a step; the row keeps it straight. Rows do not cross a way split (a junction
    usually splits the way anyway).
  - **The shift** translates the whole outline, holes and attached entrances by the row
    shift, then by the deepest remaining intrusion (closest points of wall edge and axis
    link, pushed out along their difference) for up to `SIDEWALK_SHIFT_ROUNDS` 4 so a
    corner house settles against the other street too.
  - **The cap is a cap, not a refusal**: `SIDEWALK_SHIFT_MAX` 6 m. A front needing more stays
    **out of the row union** (a row of itself) and moves by 6 m; the correction rounds clamp the
    total to the same 6 m, so a house that cannot fully clear the band stops part of the way.
    Guardrail: never a refusal cap here — the row's shift is its deepest member's, so one
    house over the limit leaves its whole row standing.
  - **Collisions** (`Obstacles`, built once per pass): other buildings (their current, possibly
    already moved, outlines; a grid of bboxes grown by the cap) and segments of every road and
    rail (reach = half width), wall, fence, pipe, open watercourse, water-area ring and
    industrial cylinder (a zero-length segment of its radius), each plus `SHIFT_CLEARANCE`
    0.5 m. The rule is **relative**: a shift is blocked only by an object the house comes
    **closer** to and within reach of — a footway along the wall or a fence on the plot line
    in OSM does not forbid moving away from it. A blocked shift is retried at
    `SHIFT_FRACTIONS` ¾ / ½ / ¼; blocked at ¼ too, the house stays. Not a sweep test: a house
    thinner than its shift could jump a line, which a ≤ 6 m move of a house does not.
  - **Left in place**: a street axis crossing the outline or ending inside it (`None` from
    `Front::of` / `sidewalk_push` — there is no "away"), every fraction blocked, any **shared**
    vertex (the `square_skewed_houses` rule, same `vertex_uses`: terraces, arches, fences on
    walls), `AreaKind::Kremlin`, `BuildingUse::Church` (parts stand on each other).
  - The log line reports moved (rows included), how many of the intruding ones stopped only
    part of the way (cap or obstacle), and how many were left.
  - Order: after squaring, before door generation and tree planting — the navmesh, doors
    and trees see the moved outline. Price: the first vertex moves, so the seed, material
    and inferred storeys roll anew.
