# Entrances — real doors and the generator

Detail behind `map/osm/entrances/`. The summary lives in `CONTEXT.md`; the coverage
counts per city live in `osm-coverage.md` next to this file.

## Real OSM doors

**Entrances** (`parse.rs::parse_entrance` + `attach_entrances`) — `entrance=*` **nodes**
(the only nodes the Overpass query asks for), minus `NON_WALKABLE_ENTRANCES`
(`no` = not a door at all, `garage`, `emergency`). An entrance in OSM is normally a
*shared node of the building outline*, so attachment is an exact vertex lookup on a
1 cm grid, not a nearest-neighbour search — it lands 82% of Tula's, 79% of Berlin's,
65% of Paris's. The rest are orphans (porch nodes, buildings outside the bbox) and are
dropped with a count on stderr. Overpass emits nodes before ways, so entrances are
buffered through the element loop and attached after it. Coverage is thin everywhere
(Tula 431 doors / 6946 buildings; NY and Tokyo ~300 city-wide) — hence the generator
below. Real OSM doors always win: generation only runs on buildings that got none.

## Generated entrances

**Generated entrances** (`map/osm/entrances/`) — synthetic doors for the ~98% of
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
and least for the large cohorts.
