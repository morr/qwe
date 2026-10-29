# The parse seam — reading the elements, finishing the map

Detail behind `map/osm/parse.rs` and the parse tests. Siblings: `parse-buildings.md`
(the building readings and passes) and `parse-geometry.md` (the geometry passes —
sidewalks, blocks, lots, pockets, rings). `SKILL.md` keeps the model (**MapData**) and
the map of the pipeline; tag coverage lives in `references/osm-coverage.md`.

## The parse seam: reading the elements, then finishing

`parse(json, city, knobs)` is two halves with a line between them, and the line is what
makes a single pass reachable. **`ParseKnobs { lane_width, navtile, raw }`** (`parse.rs`) are
the parse's inputs that are not in the Overpass answer: the street lane width (the
sections — and by the width the houses, blocks and lots move), the navtile size (the
door generator's clearance, one tile in front of a door, carried by
`entrances::FootprintIndex`) and the **Raw OSM** level (below). **The parse reads no process global** —
`Default` is the knobs' defaults (3.3 m, 2 m) — the fixture, the tests, the bench and the replay parse with it; the game's
load thread gets the settled knobs from `loading.rs::start_job`, the roads gallery the
lane width off its own `RoadShapeOnMap`. `finish_parse` stores them in the map as
**`MapData::knobs`** — the snapshot of what the world was parsed with, which the
lane-width reload (`city.rs::lane_width_moved`), the paint's global
(`roads::shape::adopt_lane_width`) and the roundabout legs (`roads::leg_sections`) read;
a map built by hand in a test carries the defaults.

- **`read_elements(response, bounds) -> (MapData, Pending, ReadReport)`** — the element
  loop and nothing else. What comes out is *raw*: houses still standing in water, churches
  without a faith, skewed outlines, no doors, no trees. `Pending` is what has nowhere to go
  yet: the entrances (Overpass hands out nodes before ways, so at that moment the
  buildings do not exist). The roads with no `sidewalk*` tag at all, whose sidewalks the
  blocks around decide once the buildings are read, need no list there: they carry
  `SidewalkSide::Inferred` on the `RoadLine` itself.
  Its last act is the **OSM contours** snapshot (`contours.rs`, `MapData::osm_contours`):
  a copy of every road axis (streets and paths apart), every other line (rails,
  watercourses, fences, walls, tree rows, pipes) and every ring of every area vector and
  road area — the geometry before a single finishing pass moved it. Only points; the tags
  are on the map already. `ReadReport` logs its size (`osm parse: N contour points kept
  for the OSM overlay in …`). The overlay `mesh_osm_contours` draws it — street axes
  magenta with a square on each vertex (an OSM node), paths cyan, other lines blue, area
  rings yellow, flat material at `Z_OSM_CONTOURS_OVERLAY` 29.5 over the network overlay — in the Debug tab
  (`OSM contours`) and in the roads gallery (`Contours`, `ROADS_CONTOURS=1`). It is the
  cheap half of the **raw OSM** question («data or our parse?»): no reload, one frame.
- **`finish_parse(&mut MapData, &Pending, ParseKnobs) -> PassReport`** — the **nine** finishing passes
  (step 0 is the street sections, `map::roads::network::sections`, since the width they
  set is read by the passes after them — and right before them the roundabout tail Ys
  are straightened, `map::roads::rings::straighten_tails`, which moves nodes and cuts a
  way and so must run before the network is assembled; `references/roads.md`, **A tail
  Y is straightened in the data**) in their one correct order, closed by a tenth
  step, `compose_trees` for the default
  layout (the parser knows nothing about the panels, but it must not hand out a `MapData`
  whose `trees` is empty, or every reader has to remember a separate compose step; the
  player's own layout is reported by `map::trees::recompose_row_trees`, and only when it
  differs). **That order is their interface.** The numbered list of ten steps (0–9), each
  with its "why here", lives on `finish_parse`'s doc comment; a pass's own doc comment
  only points at its step number.
- **A narrow pass takes a pass context, not the whole map** (`parse/lots.rs::PaveLotsCtx`,
  the pilot): `pave_lots(PaveLotsCtx)` — a struct of shared references to exactly what the
  pass reads (`roads`, `buildings`, `parks`, `woods`, `grass`, `sand`, `water`, `fences`)
  and the one `&mut` it writes (`parking`). The field list is the pass's contract with the
  map — what is not in it is not touched — and a test builds it by hand, empty slices
  standing for the inputs its scene has none of. `<Pass>Ctx::of(&mut MapData)` is the
  bridge `finish_parse` calls it through. The same shape carry `parse/pavements.rs`
  (`InferPavementsCtx`: roads in and out, three green layers in) and `parse/faiths.rs`
  (`ResolveFaithsCtx`: buildings, the one field it both reads and writes) — a converted
  pass lives in its own `parse/` module with its tests beside it. A pass that follows
  **ten fields or more** stays on `&mut MapData` — its footprint is the map itself
  (`sections::apply`, `compose_trees`, `square_skewed_houses` and
  `pull_houses_off_sidewalks` — ten and eleven fields through `vertex_uses`/`Obstacles` —
  `pull_areas_to_roads`, `pockets::fill_ground_pockets`).
- **The district measure is an explicit input.** `finish_parse` builds
  `cars::district::Districts` once, over the buildings left after the drowned pass, and
  hands it to `infer_sidewalks(&mut roads, &Districts)` — no lazy `OnceCell`, no reader
  rebuilding it; the parse is the one place outside the cars layer that asks it.
- **Raw OSM** — `ParseKnobs::raw: RawOsm` (`Off` / `Parse` / `Draw`, a persisted
  settings group `debug.raw_osm`, the Debug tab's `Raw OSM` row beside `Navtile`, the
  gallery's `Raw OSM` row / `ROADS_RAW=parse|draw`). With any level on, `finish_parse`
  skips: `rings::straighten_tails`; `sections::apply` — replaced by
  **`sections::apply_raw`**, which still glues the network (the drawing and the network
  overlay read it, and gluing changes no data) but infers no lanes: only a way with a
  `lanes` tag gets the section width, the rest keep the class width `road_class` read, and
  `lanes` stays `None` on them; `drop_buildings_in_water`; `infer_sidewalks`,
  `infer_pavements`, `verges::measure_footways_beside_streets` — replaced by
  `drop_untagged_sidewalks` (every `SidewalkSide::Inferred` becomes `None`: **a sidewalk
  only where `sidewalk*` says so**); `resolve_faiths`; `square_skewed_houses`,
  `pull_houses_off_sidewalks`, `pull_areas_to_roads` and, as its own line of the same
  step, `pave_lots` (with the `LotKind::Ground` it settles — every lot stays what
  `area_kind` read);
  `pockets::fill_ground_pockets`; `generate_entrances`. **`attach_entrances` stays** — a
  mapped door is data. Planting runs (the tree nodes are data), and on the `Draw` level its
  woods and rows are thrown away: no generated trees. The `Draw` level also stands no cars
  and no wagons (`cars::park_on`, `wagons::mesh_wagons`) and draws roads raw
  (`references/roads.md`). The skipped passes report zeros, and
  `PassReport` opens with an `osm parse: RAW OSM (…)` line so the zeros are not read as
  "nothing to do". The world comes up on it: navmesh and spawn read roads and outlines, and
  the gates only lose the plots whose doors were generated. Pinned by
  `raw_osm_skips_the_finishing_passes` and
  `raw_osm_draw_plants_no_generated_trees`.
- **The reports are values**, not prints. `parse` prints both of them at the end of the
  load, as one contiguous block; a test compares the counters, which before meant reading
  stderr. A pass that still prints on its own prints *before* that block — the door
  generator's two warnings (`entrances/mod.rs`) precede it; moving them into a report is
  work inside `entrances/`.

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

A case need not go through the whole pipeline. Three routes, and the choice is what is
under test:

- **The fixture through the real `parse`** — a tag rule, which is most cases. The route
  above. `Overpass::parse()` parses with `ParseKnobs::default()`; `parse_with(knobs)` names
  the lane width or the navtile itself — no global to lock, so such a test runs in
  parallel with the rest (`a_wider_lane_widens_the_section`). Expected road widths are
  functions of the knobs too (`residential_half(ParseKnobs::DEFAULT)` rather than a bare
  3.8), so a test states which lane width its number belongs to.
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
