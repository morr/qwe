# Trees — planting and rendering

Detail behind `map/osm/planting.rs` and `map/trees/`. The crown drawing algorithm has
its own write-up in `tree-algo.md` next to this file; this one covers where trees
stand, how density works, and which resources restyle them.

## Planting

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
    from the Roads section's on purpose: an avenue's polyline and a street's come from
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
  - **`TreeRowPlacement`, a toggle in the Tree rows section.** Road widths here are
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
  sources index different vectors). They live in `planting/index.rs`, the same idiom as
  `entrances/index.rs`; the precise tests behind the
  lookup are unchanged, so the planted set is identical.

## Rendering & styles

- **Tree crowns** (`map/trees/crown.rs`, algorithm write-up — `tree-algo.md` next to
  this file) — Watabou-style
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
  itself rather than the look. The section lives in `ui/trees.rs`, first in the **Map
  tab** — cycling rows for the palette fields plus three slider rows: `Density`,
  `Conifer share` and `Noise mix` (the last two hidden outside `TreeShape::Mixed`).
- **TreeRowStyle** (resource, BRP-writable, persisted; section `ui/tree_rows.rs`, Map tab,
  under Trees) — the avenue knobs, split from TreeStyle the way Buildings is: `enabled` (rows
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
  — tunable at runtime from the **Noise section** (`ui/noise.rs`, last in the Map tab;
  its knobs — not the section — are hidden while the `noise` overlay is off, and its own
  first row `Show` is that very toggle; ranges modeled on zxc's noise sliders). The seed
  stays fixed, so a city looks the same every run.
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
  - The **noise** debug toggle (`ui/debug/overlays.rs::sync_conifer_noise_overlay`) shows the
    field as one CPU-built 512² texture sprite over the whole map on
    `Z_CONIFER_NOISE_OVERLAY`: grey ramp = field value, green = at or above the current
    threshold, i.e. the stands the current share will produce. Green also covers built-up
    areas — the field is defined everywhere, trees only grow in Wood polygons. The
    overlay draws the **un-jittered** field: at mix > 0 single crowns deliberately sit
    on the «wrong» side of the green boundary.

## Seeing every crown at once

`cargo run --example tree_gallery` — the exhaustive gallery of crowns the game can
draw: `TreeShape::CONCRETE` (3 shapes with their own geometry; `Mixed` has none, it
resolves to Cotton/Conifer before the mesh is built) × `TREE_VARIANTS` (12), each cell
with its own shadow — long, offset or conifer fan, picked by the `h` rolled per variant,
which is why one row carries three different shadows. `H` toggles the shadow layer, `G`
cycles the ground under it (wood / park / pavement — the map's own three colors), `L`
the captions.

It is not a re-implementation: a cell is built by **`trees::crown_variant(shape,
variant, style, params)`**, the same call `spawn_trees` fills its variant pool with, and
the shadows go into one merged mesh through `push_template` exactly as the game's
`tree_shadows` layer does. `crown_variant` exists for this — when the crown pipeline
grows an axis (a new shape, a new shadow kind), the gallery shows it without a line of
its own, so the axis must be added there rather than in the example.

Its left panel drives **`CrownParams`** — every knob the crown geometry has, live:
base-polygon vertices, radius jitter, lobe (the "size" of the bump over an edge, and
because both `bloat` and `Spiker` grow their bump as the square root of edge/lobe, a
*smaller* lobe gives a *puffier* outline), band lift/scale/shade weight, the two stroke
widths, the spike floor, the five shadow numbers and the variant seed. `CrownParams`
lives in `crown.rs`; **its `Default` is field-for-field the constants the city is drawn
with**, which is what lets the whole `map::trees` test suite keep pinning the game
without a single changed expectation. The game itself passes `CrownParams::default()`
and grows no panel for them.

The panel carries one knob that is **not** `CrownParams` and **not** the game's value:
"Variance" in the `Цвет` group is `TreeStyle::variance`, and the gallery starts it at
**0** against the city's 0.35 — one flat green over every cell is what makes a shape
difference read, and set 5 was picked by eye in exactly that mode. That single departure
is why the reset button is labelled «Сброс» and not «Сброс к игре»: it restores the
gallery's own default — the game's geometry, the flat colour.

`CrownParams::seed` picks the **crown set**: same rules, a different `TREE_VARIANTS`
silhouettes. A single variant cannot be re-rolled — the set is one seed and the variant
its index within it — so a bad silhouette is dropped by changing the whole set. The city
runs on **seed 5**, chosen by eye in the gallery: set 0 had a conifer whose base vertices
landed too evenly, leaving low spikes (height grows as `len^1.5`) and one flattened
flank that read as a rendering fault among the other firs. The set stride is deliberately
not a multiple of the per-variant stride — with equal strides `BASE + seed·s + variant·s`
collapses to `BASE + (seed + variant)·s`, i.e. the seed slides the same row of crowns by
one cell instead of reseeding it, and a bad variant merely moves to its neighbour.

Knobs whose value differs per shape (12 base vertices vs 16 on a conifer, jitter 1/3 vs
1/4, band lift 0.15/0.12/0.1) are **multipliers**, not absolutes: one slider moves all
three shapes and keeps their proportions, where an absolute would erase the difference
and need three sliders per quantity. Absolute are only the quantities with no per-shape
value — stroke widths and the shadow geometry.

Two things the panel had to borrow from the game rather than invent, and both are the
kind of thing that silently looks wrong: the widget kit (`qwe::ui::slider`,
`spawn_panel_button`, `panel_background`, section-header blocks, `PANEL_WIDTH_PX`), and
the **font** — `apply_panel_font` lives in `UiPlugin`, which an example does not load, so
the panel inserts `InheritableFont` (feathers' Fira Sans + `PANEL_FONT`) on its own root.
Without it every label falls back to bevy's built-in font, which carries no Cyrillic and
renders the whole panel as tofu.
