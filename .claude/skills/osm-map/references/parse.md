# Parsing — the seam, the readings, the finishing passes

Detail behind `map/osm/parse.rs`, `parse/tags.rs`, `parse/lots.rs` and the parse tests.
`SKILL.md` keeps the model (**MapData**) and the map of the pipeline; this file carries
how a response becomes that model: the seam between reading the elements and finishing
them, every tag reading and finishing pass in order, and how a tag rule is pinned.

## Parsing details

### The parse seam: reading the elements, then finishing

`parse()` is two halves with a line between them, and the line is what makes a single
pass reachable:

- **`read_elements(response, bounds) -> (MapData, Vec<Vec2>, ReadReport)`** — the element
  loop and nothing else. What comes out is *raw*: houses still standing in water, churches
  without a faith, skewed outlines, no doors, no trees. The `Vec<Vec2>` is the entrances
  that have nowhere to go yet — Overpass hands out nodes before ways, so at that moment the
  buildings do not exist.
- **`finish_parse(&mut MapData, &[Vec2]) -> PassReport`** — the **eight** finishing passes
  in their one correct order, closed by a ninth step, `compose_trees` for the default
  layout (the parser knows nothing about the panels, but it must not hand out a `MapData`
  whose `trees` is empty, or every reader has to remember a separate compose step; the
  player's own layout is reported by `map::trees::recompose_row_trees`, and only when it
  differs). **That order is their interface**. It used to live as notes in three doc
  comments out of eight and was written down whole nowhere; now it is one numbered list of
  nine steps on that function, each step with its "why here", and a pass's own doc comment
  only points at its step number.
- **The reports are values**, not the ten `eprintln!` that used to make up forty-five of
  `parse`'s hundred and twenty-five lines. `parse` prints both of them at the end of the
  load, as one contiguous block; a test compares the counters, which before meant reading
  stderr. A pass that still prints on its own prints *before* that block — the door
  generator's two warnings (`entrances/mod.rs`) used to interleave with the summary and
  now precede it; moving them into a report is work inside `entrances/`.

Two facts the order carries, both pinned by tests that can only exist now that a pass can
be called alone:

- **Attaching the mapped doors comes before squaring the skewed houses.** An entrance holds
  the *node's* coordinate; while it is attached to the house the same centimetre key
  carries it onto the straightened outline. `squaring_before_attaching_loses_the_door` runs
  the two passes in both orders and shows the second one drops the door.
- **`vertex_uses` is computed twice on purpose.** Squaring (step 4) and pulling houses off
  the sidewalks (step 5) both ask "is this vertex shared?", and the outlines **move**
  between them — a count taken before squaring answers about the old map. This was
  reported as duplicated work; it is not.

### Readings and passes, one by one

- **Building height** (`parse/tags.rs::building_height`) — metres, from two *independent*
  branches of OSM data that almost never co-occur: `height` verbatim (New York — 97%, a
  LiDAR import) or else `building:levels` + `roof:levels` × `settings::STOREY_HEIGHT` (3 m)
  (Paris 64%, Berlin 59%, London 50%, Tula 31%, **Tokyo 5%**). `parse_measure` handles
  the tag-value zoo — `12`, `12.5`, `12,5`, `12 m`, `3;4`, `40'6"`. Anything outside
  `BUILDING_HEIGHT_RANGE` (2–600 m) counts as *no tag*: OSM carries both `height=0` and
  order-of-magnitude typos. `None` is normal, not an error — and it is the majority
  everywhere but New York, so what fills it in matters: see **Inferred storeys** in
  `references/buildings.md`. Coverage is logged per city on load
  (`N buildings (M with height)`).
- **Building use** (`parse/tags.rs::building_use`) — `BuildingUse: House | Apartments |
  Commercial | Retail | Industrial | Garage | GarageBlock | Church(Sacred) | Public | Other`, the class that
  picks two material tables — the **cladding** (`buildings/material.rs::wall_kind_of`) and
  the **roofing material** (`::kind_of`); neither colour comes from the class itself, both
  come from the chosen material's own palette (**Roof material** in
  `references/buildings.md`, bullet
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
  Tula: `yes` 4004 of 7465, `house` 2249, `apartments` 744, commercial/retail/office 165,
  garage(s) 74, industrial 31, church 17. **`garages` and `garage` are two different
  classes**, and that is not pedantry: the plural is how OSM maps a *whole cooperative*
  as one outline (Tula 43 of them, the largest 255 × 51 m), and drawing it as one shed is
  what made the ГСК look like a hangar — see **the garage row** in
  `references/buildings.md`. The
  singular, together with `carport`/`shed`/`barn`/`roof`, stays one box.
  The Kremlin (`AreaKind::Kremlin`) keeps its
  red regardless of class. `roof:shape` is **not** read (283 of 7465 in Tula carry it);
  the roof shape is inferred instead — see **Gable roofs** in `references/buildings.md` — with one
  exception below. The class is
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
    nine-storey block. Tula v14: 69 building outlines carry `shop=*` (67 ways and 2
    relations), 35 pass the list (17 `mall`, 7 `supermarket`, 4 `doityourself`,
    3 `department_store`, 2 `furniture`, `hardware`, `car`). **No `QUERY_VERSION` bump** —
    `out geom` returns every tag of the element, so `shop` has been in every cache since v1.
    **`building=shop` sits in the `Retail` arm, not the `Commercial` one**, because in OSM
    it is a plain synonym of `building=retail` — the building that is a shop — and the
    whitelist a line below already calls `shop=mall` and `shop=department_store` retail. The
    value is a *particular* one, so a small `shop=*` on the same outline no longer overrides
    it. Latent on Tula, which carries **no** `building=shop` at all; it shows on another
    city, where such an outline gets the cassette, the brand band and the box roof instead
    of an office's shopfront.
  - **The size, the storeys and the height** (`is_big_box`: `Retail` with a footprint ≥
    `BIG_BOX_AREA_MIN` 1200 m², **and** `PolyArea::storeys` at or under
    `BIG_BOX_MAX_LEVELS` 3, **and** a height at or under `BIG_BOX_MAX_HEIGHT` 17 m — each
    of the three can refuse on its own).
    OSM never marks crowd-format, so it is read off the pattern, exactly as
    `is_fortress_tower` reads a tower off its compactness. **The threshold is a chosen
    line, not a gap the data hands you**, and that is worth knowing before moving it: on
    the Tula v14 cache the class has 90 outlines — one per outer ring, the way
    `parse_relation` assembles them, not one per way-member of a relation — of which 63 are
    `building=retail` and 47 of those carry no `shop=*` at all; 24 are at or above 1200 m²
    and 66 below, and the distribution around the line is continuous —
    1265 m² («Торговые ряды») above it, 1198 m² of an unnamed `building=retail` below, and
    ТЦ «Триумф», a `shop=mall`, at 1187. What 1200 m² says is that big format starts at a
    hypermarket of 1300–1400 m² («ДА!» 1363, «Верный» 1628), while a mall block of a
    thousand-odd still reads as an infill from the air; the price is that two buildings of
    nearly the same footprint are drawn differently, which is the cost of any single
    number here.
    **The storey count is what actually separates a mall from a box, and that is why
    `PolyArea` carries it** (`storeys`, `building:levels` as the tag says — see **MapData**
    above). The threshold used to live in the parse's height alone, and everything else on
    this list hung on the footprint by itself: «Пятёрочка», a `shop=supermarket` on the
    ground floor of a nine-storey panel block mapped onto the whole block, got honest 27 m
    *and* a blind cassette, a 5.5 m tier, a skylight grid and entrance groups every 55 m.
    Moving a **height** ceiling into the predicate was the first attempt and it could not
    do the job: a four- or five-storey mall is measured in dwelling storeys and lands at
    12–15 m while a real two-level box stands at 12.5 m, so the ceiling separated two of
    the nine tall malls and left seven — no number in metres runs between «Империя» (4
    storeys, 12 m) and ТРЦ «Макси» (2 trading levels, 12.5 m). `BIG_BOX_MAX_LEVELS` 3 does,
    and the same constant is what `building_height` measures the shell by, so «measured as
    a box» and «drawn as a box» stay one statement rather than two that can part.
    Of the 24 outlines above the area line **15 are big boxes**. The nine that go are
    exactly the `building:levels ≥ 4` ones: ТРЦ «Гостиный двор» (6), «Парадиз» (5),
    «Троицкий» (5), «Заречье» (5), «УтюгЪ» (4), «Империя» (4), «Талисман» (4), an unnamed
    `building=retail` of 1408 m² (4) and «Пятёрочка» (9).
    **The height ceiling stays, as the answer for an unmapped storey count**
    (`BIG_BOX_MAX_HEIGHT` 17 m — the same three trading levels of `BIG_BOX_LEVEL_HEIGHT`
    4.5 m plus `BIG_BOX_SHELL_EXTRA` 3.5 m of shell, written in metres): a building with a
    verbatim `height` and no `building:levels` has nothing else to be judged by, and on
    Tula the two questions never disagree — no `Retail` outline there carries a `height`
    tag at all.
  - **What it decides.** Four things, each with its own bullet elsewhere: the **height**
    (below), the **roof material** (`BIG_BOX_ROOFS` — light membrane / bitumen / gravel —
    against `SHOP_ROOFS`), the **roof clutter** (the skylight grid and the roof plant, in
    `references/buildings.md`) and the **wall** (`WallKind::BigBox` with its brand band,
    same place). A small shop gets none of it and keeps a shopfront over a house window.
  - **The height** (`building_height`, and it is why that function takes the outline as
    well as the tags). A trading level is `BIG_BOX_LEVEL_HEIGHT` 4.5 m, not the 3 m
    dwelling storey, and over the top one sit `BIG_BOX_SHELL_EXTRA` 3.5 m of technical
    floor and parapet — so `building:levels=1` on a hypermarket is 8 m, not 3. That single
    number is the loudest half of the original report: a 122 × 103 m «Магнит»
    (`building=commercial` + `shop=supermarket`, `levels=1`) was drawn three metres tall
    and read as a giant one-storey house. Tula: «Магнит» 8 m, ТРЦ «Макси» (`levels=2`)
    12.5, ТЦ «Сарафан» (3) 17.
    **Only up to `BIG_BOX_MAX_LEVELS` 3 trading levels** (and only while the shell they
    produce fits under `BIG_BOX_MAX_HEIGHT` — the same statement in metres, which here
    catches a `roof:levels` the storey threshold does not count) **and only from
    `BIG_BOX_MIN_LEVELS` 1 up**. Both ends are
    load-bearing. Above: «Пятёрочка» is `building=retail` + `levels=9` mapped onto the
    whole panel block it occupies the ground floor of, and a trading level there would
    have made a 45 m tower of it — so the ordinary dwelling storey applies, which also
    happens to be right for the multi-floor malls (Гостиный двор 6 → 18 m, Парадиз 5 → 15).
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
    8–11 m, `SHOP_HEIGHTS` 4.5–6.5), deliberately — «Верный» and ТЦ «Перспектива» carry no
    `building:levels` at all, and a box standing next to an identical box must not come out
    half as tall for want of a tag.
- **Places of worship** (`parse/tags.rs::faith`, `sacred_form`; `parse.rs::resolve_faiths`)
  — `BuildingUse::Church(Sacred { faith, form })`. `building=bell_tower|campanile|minaret`
  and `tower:type=bell_tower|minaret` are churches too (`form: Tower`); a part with
  `roof:shape=onion|dome` is `form: Dome` — the only reading of `roof:shape` in the parse.
  **Tagged colours** (`tags::area_colours` → `PolyArea::colours`, every building, not only
  churches): `building:colour` and `roof:colour` as sRGB bytes. `tags::colour` takes a hex
  value as is — the mapper picked it off a photo — and a **CSS name as the map's own paint**
  (`CSS_COLOURS`: `white` is whitewash, `blue` the roof palette's blue, `darkgray` darker
  than `gray` as a mapper means it, not lighter as CSS has it), because `#0000FF` on the map
  is a marker, not paint; an unknown name is `None`, never a guess. Tula: 30 `building:colour`
  and 91 `roof:colour` on 7.7 k buildings, 51 of them `blue`; on the churches — `#FFD700` on
  both kremlin-cathedral drums, `#5D948F` on the All Saints cathedral and its bell tower,
  `blue` on Свято-Никольский, `red` / `green` on the arms museum annex. **Two things read
  them**: the temples (`temples::tagged_wall` / `tagged_roof` / `tagged_dome`, below) and
  the retail box's **brand band** (`layers::brand_color` — ТРЦ «Макси» is tagged `orange`,
  which is its real colour). Reading them on every house is a palette decision the private
  sector has not made.
  Faith: `religion=christian` + an Orthodox-family `denomination` → `Orthodox`, any other
  denomination → `Western`, none → `Unknown`; `muslim|jewish|buddhist|hindu|shinto|…` by
  religion, else by `building=mosque|synagogue|temple`. Tula v14: 27 places of worship —
  17 `russian_orthodox`, 3 `orthodox`, 1 catholic, 1 evangelical, 1 `christian` bare, 4 with
  no religion (the kremlin cathedral's parts: 9×9 onion 35 m, 11×11 dome, 26×25 onion 30 m,
  the 70 m bell tower). **`resolve_faiths`** runs right after the drowned-building pass and
  assembles churches from parts: a part's **host** is the largest larger church holding its
  centre, else the nearest larger one within `CHURCH_PART_REACH` 30 m (a bell tower stands
  beside, not inside). Hosts are followed **up the chain**: a bell tower whose nearest larger
  church is a part of a cathedral (a part sticking out past the outline, or tied with it on
  distance) belongs to the cathedral. The part takes the top host's first-vertex seed as
  `Sacred::complex` — what its colours are picked by — and, when untagged, the faith of the
  nearest host up the chain that has one; the rest take the
  city majority (Orthodox vs Western, ties → Western). `Sacred::floor_dm` is `min_height`,
  else `building:min_level` × 3 m (`tags.rs::part_floor`).
  **Annexes** (`parse.rs::absorb_annexes`, same pass, after hosting): a `BuildingUse::Other`
  building of `AreaKind::Building` that lies on a church **or an annex** — its centre in the
  host, the host's centre in it, or `ANNEX_OVERLAP_SHARE` 25 % of its own footprint
  shared (`overlap_area`, an `i_overlay` intersect) — at most `ANNEX_AREA_RATIO` 2.5× the
  host, becomes `SacredForm::Annex` with the largest such host's faith and complex, in up
  to `ANNEX_ROUNDS` 3 rounds (the apse part below shares under a quarter with the
  cathedral itself and lies on the museum instead).
  Only `Other`: a house, a school or a shop keeps its class however it lies. Tula: the arms
  museum mapped over the Epiphany cathedral (45×39 vs 37×35, `building=yes`) and the
  cathedral's apse part (20×24, centre outside, half inside) were two apartment boxes with
  windows the cathedral's cupolas stuck out from behind.
- **Fortress** (`parse/tags.rs::is_fortification`) — `AreaKind::Kremlin` from
  — on an outline carrying `building` only (`area_kind` asks nothing else) —
  `historic=citywalls|castle|city_gate|fort`, `barrier=city_wall`,
  `man_made=tower` + `tower:type=defensive`, or `building=wall` at ≥ 6 m
  (`FORTRESS_WALL_MIN_HEIGHT`; lower is a garden wall). Tula carries **no** `historic` on
  its kremlin — before this every tower and the wall were plain buildings with windows.
- **Drowned buildings** (`parse.rs::drop_buildings_in_water`) — a building whose outline
  lies **entirely** inside a water polygon is dropped right after the element loop, before
  doors and trees. OSM tags floating restaurants and moored ships as buildings (`HMS
  Belfast`, `Café Barge`) and Tula carries a lone shed in the middle of Верхний пруд; the
  navmesh floods water impassable, so their doors are unreachable anyway and the box
  standing on the pond reads as a render bug. One vertex on land is enough to survive —
  piers and embankment houses stay. Counts: Tula 1, Berlin 6, NY 17, London 28, Paris 28,
  Tokyo 0; logged on stderr when non-zero.
- **Squared houses** (`parse.rs::square_skewed_houses`) — a small house outlined as a
  **skewed quad** is replaced by a rectangle. The private sector is traced by eye off
  imagery, and a rectangular house comes out a rhombus (Tula way 968419942, corners
  79°–100°): in 2.5D its ends stand askew to its front and no gable fits it. Taken: a
  4-vertex convex outline, `AreaKind::Building`, no holes, an `Other` of at most
  `SQUARE_AREA_MAX` 250 m² (the pitched-cohort threshold) or a `House` of at most
  `SQUARE_HOUSE_AREA_MAX` 400 m² (the tag already says private house, and a house is
  pitched at any size — Tula way 968378335, 348 m² at 21°), with its worst corner
  `SQUARE_SKEW_MIN` 2° … `SQUARE_SKEW_MAX` 35° off square — under 2° the trace is
  already straight, over 35° it is a trapezoid by the plot. The ceiling was 20° first and
  a screenshot of Tula's private sector (around `cam 1641 4539`) showed seven lone houses
  left crooked at 20.4°–32.3°; measured on the cache, **every** lone small quad above 20°
  was such a trace (10 of them, vertex shift ≤ 1.7 m), none a real trapezoid — the
  shift cap is what guards the rest. The rectangle keeps the
  **centroid and the area**: the axis is the length-weighted mean of the edge directions
  with the angle ×4 (so both axes vote for one), the sides the mean lengths of opposite
  edges along it, scaled to the area; vertex `i` becomes corner `i` with the winding kept.
  **A skewed L is straightened too** — a six-vertex outline with exactly one reflex corner
  (a house with a wing, Tula ways 968378349 / 968378329, corners up to 17°–20° off), under
  the same area, skew and shift gates. `fit_ell` uses the same `outline_axis`; every edge
  goes to the nearer axis (they must alternate, or it is not an L) and gets a **level** —
  the mean of its two ends across that axis — and vertex `i` stands where its two edges'
  levels cross. So each wall lands halfway between its traced ends; the area is not
  rescaled, and a fit whose area drifts over `ELL_AREA_DRIFT` 15 % or that does not come
  out with exactly one reflex corner is left alone. `corner_skew` measures a reflex corner
  against 270°. Tula: 27 of ~600 lone six-vertex small houses (561 are already square
  within 2°).
  Skipped when any vertex is **shared** with another outline or line (terraced houses, a
  fence along the wall, an arch — squared, they would open a gap) or when a vertex would
  move over `SQUARE_SHIFT_MAX` 3 m (2.5 m first; on Tula that left exactly one lone
  crooked house, way 968378327 at 27° and 2.84 m). **Which outlines count as sharing is
  an explicit list, not "every outline on the map"** (`vertex_uses`): the buildings, all
  the line layers, and **three of the eight area layers** — `parking`, `pitches`,
  `water`. Those three are drawn as a surface of their own with markings on it, and a lot
  carries parked cars as well, so a house that steps off that edge reads on the frame,
  which is the whole subject of issue #27; `parks`, `grass`, `woods` and `sand` lie
  *under* the house and show no seam, and a `landuse` block's outline is not counted
  because private houses are routinely traced onto the block's boundary. Tula, buildings
  with a vertex on an area layer (counted on the cache, so an upper bound — most of them
  fail the area, skew or shift gates anyway): `landuse` 35, `parks` 29, `grass` 15,
  `pitches` 13, `parking` 10, `woods` 5, `water`/`sand` 0. So the three counted layers put
  at most 23 buildings of 7975 out of reach and the other 43 stay eligible.
  `Obstacles` (the pull below) carries its own, narrower list and does not close this
  hole — it is about what a moving house bumps into, not about a vertex two outlines share.
  Runs after `attach_entrances` (they match by the same centimetre
  `vertex_key`, and an attached door is carried to the straightened outline by that
  same key — an entrance holds the *node's* coordinate, not the vertex's, so an exact
  `==` would silently leave the door behind while the house moved) and before door
  generation and tree planting. The price: the render seed is the first vertex, so a squared house rolls
  its material and inferred storeys anew. Tula: 207 (193 under the 20° / 250 m² / 2.5 m
  thresholds; python estimate from the cache, the exact count is the `osm parse:` line).
- **Houses pulled off the sidewalks** (`parse.rs::pull_houses_off_sidewalks`) — the street's
  width is a class constant and the sidewalk is added by the renderer
  (`roads::sidewalk_width`), so an old house standing at the kerb in OSM came out with its
  wall on the drawn sidewalk and, in 2.5D, its roof on the asphalt (Tula way 179102449 on
  улица Бундурина: the wall 4.7 m from the axis against a 5.76 m band). The game does not
  need metre accuracy, and a house on the pavement reads as a bug, so the house moves.
  - **The band** is every non-bridge `roads::is_carriageway` link at `width / 2 +
    sidewalk_width + SIDEWALK_CLEARANCE` 2 m, in a `SIDEWALK_CELL` 32 m grid; raw OSM
    points, not the smoothed centreline — the difference is centimetres. The clearance was
    0.3 m first, and the author's look said the houses stood right on the pavement edge: the
    2.5D roof leans toward the street by another half metre to a metre.
  - **The row moves, not the house** (`Front`, union-find): each eligible building's deepest
    band intrusion names its street way and the side of it; fronts with `need >
    -ROW_SETBACK_TOLERANCE` 2 m on the same (way, side) whose bounding boxes are within
    `ROW_GAP` 20 m join a row. The row's shift is its deepest `need`, and every member whose
    own `need` is within `ROW_SETBACK_TOLERANCE` of it takes that same shift along its own
    away-direction — a neighbour standing on the line but not quite on the band moves too,
    one standing deeper in the block stays. The author's ask: a lone house pulled back broke
    the facade line into a step. Rows do not cross a way split (a junction usually splits the
    way anyway).
  - **The shift** translates the whole outline, holes and attached entrances by the row
    shift, then by the deepest remaining intrusion (closest points of wall edge and axis
    link, pushed out along their difference) for up to `SIDEWALK_SHIFT_ROUNDS` 4 so a
    corner house settles against the other street too.
  - **The cap is a cap, not a refusal**: `SIDEWALK_SHIFT_MAX` 6 m. A front needing more stays
    **out of the row union** (a row of itself) and moves by 6 m; the correction rounds clamp the
    total to the same 6 m, so a house that cannot fully clear the band stops part of the way.
    It was 4 m and a refusal first, and the row rule made that loud: the row's shift is its
    deepest member's, so one house over the limit left its whole row standing — Tula's улица
    Громова east side (13–35) stayed on the pavement because house 17 needed 4.01 m.
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
    and inferred storeys roll anew. Tula: **1121 moved (rows included), 56 of them only part
    of the way, 38 left**, 33 ms at load (`examples/bench/map_meshing`'s parse; it was 993
    moved / 160 left / 18 ms at the 4 m refusal with no collision test).
- **Blocks pulled to the roads** (`parse.rs::pull_areas_to_roads`) — the same mismatch
  read from the other side. The road's width is a class constant and its sidewalk is the
  renderer's, while a `landuse` block is traced along the red line or the plot fences, so
  between the yard and the drawn sidewalk a strip of bare ground is left showing — reported
  from a screenshot of Tula's Воздухофлотская улица (block 185117817: the edge 6.1 m from
  the axis against a 5.76 m band, i.e. a 34 cm seam). On a photo a yard runs up to the
  kerb, and a seam of ground beside the pavement reads as an unpainted layer.
  - **The vertex moves, not the block**, and it moves **under** the asphalt: a vertex whose
    gap to the drawn edge (half the class width plus `roads::sidewalk_width` on a
    carriageway) is within `LANDUSE_GAP_MAX` 5 m is pulled to the road's axis until it
    stands `LANDUSE_OVERLAP` 0.5 m inside the band. The overlap is not decoration: the
    ribbon is drawn from the *smoothed* centreline while the gap is measured on the raw OSM
    points, and without it a bend keeps a centimetre of seam. Beyond the limit nothing is
    done — that is a real gap (a front garden, a verge, a right of way), not a seam.
    Measured on the Tula cache: of 2756 block vertices, 1135 already lie under the asphalt
    and 752 are within five metres (261 / 174 / 130 / 88 / 99 by the metre), with 200-odd
    more beyond that. The limit was 3 m first and went up on the author's look at the
    frame: at four and five metres the strip of ground along a street still reads as a
    seam rather than as a verge.
  - **Green only grows**, and that single rule is what keeps the pass safe: a vertex moves
    only when the move leads **outward from the fill**, which is read **locally off the
    ring** — the ring's own signed area gives the side, so the outer contour grows away
    from the block and a hole shrinks into itself (the green lies outside a hole's ring).
    A street running inside a block would otherwise drag its boundary inward, while a
    street through a courtyard correctly pulls the **hole's** edge to the asphalt — there is
    no green in the hole, and it is its rim that has to reach the road.
    Locally, rather than by asking whether the road lies outside the block: for a strip of
    lawn between two streets the nearest street is the one **beyond the far edge**, and that
    answer would squeeze the strip instead of stretching it. `point_in_area` also has
    nothing to say where the projection lands exactly on the outline — which is every road
    that ends against a block, and was two of the four corners of the courtyard test scene.
    Everything drawn on a block lies above it (`Z_LANDUSE` 0.25 against `Z_SIDEWALK` 1.6,
    parks and grass at 0.5–0.7), so the part that ends up under the road, under a park or
    on a neighbouring block is never seen; only the closed seam is.
    **One exception moves a vertex inward**: the nearest road is a **walkway**
    (`RoadClass::Alley` — a sidewalk mapped as its own footway) lying inside the fill
    within `SIDEWALK_TUCK_MAX` 3 m, and a carriageway lies outward within
    `LANDUSE_GAP_MAX`. The vertex then goes `LANDUSE_OVERLAP` under the footway. Berlin
    draws its blocks to the kerb and maps the sidewalk as a footway inside them, while
    our carriageway is narrower than the real one (Berlin, gallery 03: the block edge
    6–7 m from the axis against a 3.8 m half width), so the yard stuck out from under the
    sidewalk as a dark crescent at every rounded corner. The strip between a sidewalk
    and the kerb is paving, not yard: it is left as ground.
  - **The band is what is drawn**: a street's sidewalk counts in its reach only when it
    has one (`RoadLine::sidewalks`). `sidewalk=separate|no` used to count anyway, and the
    block was pulled under a sidewalk that is never drawn — its edge stood past the kerb.
  - **A long edge beside a road is split first** (`LANDUSE_STEP` 8 m, and only where the
    grid has a road near the edge): between its own two vertices an edge is straight while
    the road bends, and on the outside of a turn the seam would stay in the middle of the
    edge, where there is nothing to move. An inserted point that found nowhere to go is
    dropped again, so a ring does not collect vertices for nothing.
  - Bridges and passages give no segments: a block is drawn under a bridge anyway, and an
    arch through a house is not the edge of a yard. Everything else that is drawn does,
    alleys included — a footpath with a seam of ground beside it reads the same way.
  - **The parking lots reach their roads in the same step, by a different construction**
    — `parse/lots.rs::pave_lots`, a **polygon closing**, called at the end of
    `pull_areas_to_roads`.
    - **The vertex pull was here first and was removed** (`Stretch::Lot`, `Untouched`,
      `untangled`, `PARKING_GAP_MAX` 12 m, `PARKING_STEP` 3 m). Each point of the outline
      looked for its own road — the farthest one outside within the limit — and
      neighbours moved by different amounts. On Tula's ТРЦ «Макси» lot (way 397005593,
      an aisle across the outline every 17 m, a perimeter drive 10–20 m off) that left,
      all on one screenshot from the author: a **tooth of ground beside every aisle stub**
      where the drive was past the limit, **needles of asphalt** toward a far road,
      unfilled wedges where the ring crossed itself (`untangled` kept only the largest
      contour), and holes in the blob two thin lots and their drive merged into. None of
      it is a threshold to tune: a ring whose vertices move independently has no way to
      stay smooth. Do not bring the pull back for lots.
    - **The construction.** `base` = the lot ∪ the bands of the streets beside it
      (`road_pieces`: a road is cut every `ROAD_PIECE` 3 m and a piece is taken while its
      band is within two closing radii of the lot, so a long street does not drag asphalt
      along its whole way; pieces of one road are glued across links into one stroke;
      `RoadClass::Street` only — a footpath is not what a lot is entered from; the band
      of a carriageway includes its sidewalks). `closed` = `base` offset **out by the
      radius and back in** (`i_overlay` `outline`, round joins, `ARC` 0.3). `closed − base`
      is what the closing added, and a piece of it is kept only if it lies **between the
      lot and a road** (`between`: a vertex within `TOUCH` 0.25 m of the lot's rings — or
      of a **drive**, a street with no sidewalk, whose asphalt is the lot's own — and a
      vertex on some road band). A notch in the outline (the lawn in the corner of an
      L-shaped lot) touches no road; a wedge between two streets touches no lot. Kept
      pieces grow by `LANDUSE_OVERLAP` 0.5 m (bevel join) so the edge goes under the
      ribbon drawn from the smoothed centreline — **and only there**: the grown ring is
      intersected with the road bands. Grown on every side, a piece also stepped out
      half a metre along its *free* edge (the closing's arc, a house wall, a lawn), and
      wherever that edge met the lot's own there was a half-metre jog — the «jerks» on
      the mall lot's border, the author's report. The largest shape of lot ∪ pieces is
      the new outline, **holes included** — the island of a roundabout at the lot's edge
      stays ground.
    - **The apron** (`Around::walls`, `BUILDING_APRON` 1 m) — last, and on every lot
      whether it grew or not: **every** house near it, of any size, swollen by the apron
      and subtracted. OSM draws a lot overlapping a house or flush with its wall, and the
      closing pulls asphalt into the slit between them, so stalls stood in the wall (the
      green block and the ticket booth on the mall lot). `KEEP_BUILDING_AREA` above is a
      different question — what the *added* asphalt may not crawl over — and stays. A
      booth in the lot gets its island; a house across a lot cuts it, and then the parts
      of `MIN_LOT_PART` 30 m² and up all stay lots (the first in place, the rest appended
      to `MapData::parking` — Tula 349 → 355). `comes_near` keeps the boolean off lots no
      house reaches. Pinned by `a_lot_steps_back_from_the_houses_on_it`.
      **And it is counted apart from the growth, because it happens apart from it**
      (`PavedLots { grown, trimmed }`, printed as two numbers in the `osm parse:` line —
      «N parking lots paved up to their roads, K only stepped back from the houses on
      them»). A lot with no road within reach still meets the apron, so one counter for
      both events printed a number the sentence beside it did not describe; the pinning
      test is exactly such a scene, with no road in it at all.
    - **`CLOSING_RADIUS` 7 m** — a gap under 14 m closes: a stall row with its aisle
      (`STALL_DEPTH` 5.2 + `AISLE` 6) and a little, the same reading the 12 m limit had;
      the pockets between the mall's aisle stubs are 12 m between bands.
      **`GROUND_CLOSING_RADIUS` 12 m** on a big lot (`parking::is_ground`): its perimeter
      drive stands 15–20 m off, and all of that strip is the lot's asphalt on a photo —
      exactly the pockets the vertex pull was written down as not reaching.
    - **What the asphalt does not crawl over** is subtracted from the kept pieces, and
      `between` is asked again: a building of `KEEP_BUILDING_AREA` 100 m² or more (the
      hardware shop, way 764017758, has a yard behind it; the ticket booth *in* the lot,
      way 1435094568, does not), greenery, water — and **fences**, as a band of
      `FENCE_HALF` 0.75 m either side of the line. A fenced lot's outline lies along its
      `barrier`, so the band separates the added piece from the lot, the piece no longer
      touches it and is dropped; a fence standing a few metres off cuts the piece in two
      and each half fails one side of `between`. Both cases of the hospital lot (way
      344589378) are pinned: `a_fenced_lot_stays_behind_its_fence`,
      `a_lot_does_not_step_over_a_fence_it_was_not_standing_on`.
    - **The road band itself is not part of the lot** — a lot is entered *from* that
      road, and the layout stripes whatever the outline holds. **Except a drive with the
      lot's asphalt on both sides of it** (`sandwiched`, probed every 3 m at `BESIDE`
      0.4 m past either edge of the band, butt ends): without that a lot falls apart into
      strips along its own drives and no row fits any of them. A second, small closing
      (3 m) did the same job and cost four more `i_overlay` calls per lot.
    - **What it changed besides the edge, and it is a correction**: the vertex pull went
      to the *farthest* road, so a pocket or a strip lying against a street was stretched
      **over the street** and striped there — 96 stalls on the carriageway beside way
      702257069 alone. Lots shrank to their honest area (that one 5661 → 2036 m²); what
      puts stalls back on such a strip is the layout's pocket rule (**Parking** in `parking.md`).
      City total 20 245 → 19 440 stalls.
    - **Cost, and why it is threaded.** A lot costs half a dozen `i_overlay` calls, and
      the price of a call is the call, not the geometry — ≈ 0.3 ms even on a four-vertex
      lot — so 349 lots were 0.5–1 s single-threaded against 74 ms for the whole old step.
      Lots are independent, so `pave_lots` splits them across `available_parallelism`
      threads (`std::thread::scope`, results applied in order — the output does not
      depend on the split): the step is **112 ms** on Tula (`map_meshing`'s parse, `dev`
      profile), +38 ms per world load — and **137 ms** with the road-only growth and the
      apron (one more boolean each, same run of the bench). **Release buys nothing here**
      — 119–133 ms then, 98–103 once the grids went off SipHash (**The uniform grid**),
      the same number as `dev` within the bench's swing either way — because the time is
      `i_overlay`'s and a dependency is optimised in `dev` too; see the same point under
      **A big lot shows the road through it → Cost**.
      Two things that were measured on the way: subtract
      obstacles from the *kept* pieces, not from everything added, and filter them by the
      pieces' bounds (the river's outline otherwise goes into a boolean for every lot on
      the embankment); glue road pieces into one stroke per road, not per link.
  - Order: after the houses are pulled off the sidewalks, before door generation. It could
    stand anywhere in the tail — neither `landuse` nor `parking` reaches the navmesh, the
    doors, tree planting or the parked cars' districts — and it is the only pass here whose
    effect is purely what is drawn. **After the squaring** (step 4) it must stay, though:
    `vertex_uses` counts a parking outline among the layers a house may share a vertex
    with, so a pulled edge would change which houses get squared.
- **Ring assembly** (`parse.rs::assemble_rings`) — multipolygon relation members joined
  end-to-end (ε = 0.01 m) into closed rings; chains broken by the bbox edge are
  force-closed if ≥ 3 points. Inner rings become holes of the outer containing them.

## Testing the parse

`parse/tests.rs` is where a tag rule is pinned, and the fixture it uses is
`fixture.rs::Overpass` — a scene stated in **map metres**, turned into an Overpass
response, fed through the real `parse`:

```rust
let map = Overpass::new(CITY)
    .way(&[("railway", "rail")], vec![sw, ne])
    .way(&[("railway", "platform")], vec![sw, ne])       // dropped: whitelist
    .area(&[("natural", "wood")], square(CENTER, 110.0)) // area = closed way
    .node(&[("entrance", "main")], sw)
    .relation(&[("natural", "water")], &[("outer", …), ("inner", …)])
    .parse();
```

Four things about it worth knowing before writing a case:

- **Metres, not degrees.** Points are unprojected with `GeoBounds::unproject`, so a test
  builds its scene in the same numbers it asserts on (`CENTER`, `corners(half)`); the
  round trip is pinned by `unproject_returns_the_point_project_started_from`.
- **It goes through the JSON text**, not around it: deserialization of the Overpass DTO
  is part of what these tests cover. That is why the builder emits a string rather than
  handing `Element`s to an inner function.
- **`area` closes the ring, `way` does not, `relation` members are left as given** —
  OSM cuts an outer ring into several open ways, and assembling one from the pieces is
  itself under test.
- **Scenes are shared** where they repeat: `wood_scene()` (planting rules are checked as
  subtractions from a full wood), `tree_row(tags)`, `fixture()`.

A new tag reaching the map means a case here — a builder line and an assertion, not a
new JSON literal. Coverage of tags overall is the audit in `references/osm-coverage.md`.

**Since the parse seam, a case need not go through the whole pipeline.** Three routes,
and the choice is what is under test:

- **The fixture through the real `parse`** — a tag rule, which is most cases. The route
  above.
- **`read(scene)`** (the helper in `tests.rs`) — the fixture's JSON through `read_elements`
  alone, so the *raw* map can be asserted on before any pass touches it
  (`reading_the_elements_leaves_the_passes_undone`), or `finish_parse` called on it as one
  value-returning step (`finishing_the_parse_reports_what_each_pass_did`).
- **A pass called by name on a `MapData` built by hand** — no JSON, no `GeoBounds`, none of
  the other passes (`a_pass_runs_on_a_hand_built_map`, `squaring_runs_on_its_own`,
  `pulling_houses_off_the_sidewalks_runs_on_its_own`,
  `pulling_the_blocks_to_the_roads_runs_on_its_own`, `resolving_the_faiths_runs_on_its_own` —
  every one of them on a scene that makes the pass's counter **non-zero**, since a pass
  that did nothing proves nothing about being called). This is
  also the only way to test the *order*: `squaring_before_attaching_loses_the_door` runs the
  same two passes both ways round.
  **The geometry under the passes has its own unit tests**, one step below the pass —
  `fit_rectangle`, `fit_ell` and `closest_between_segments` take rings and segments, not a
  `MapData`, so they are asserted on directly
  (`a_fitted_rectangle_keeps_the_area_the_centroid_and_the_winding`,
  `a_fitted_ell_squares_its_corners_and_refuses_what_is_not_an_ell`,
  `the_closest_pair_of_two_segments_is_none_only_when_they_cross`).
