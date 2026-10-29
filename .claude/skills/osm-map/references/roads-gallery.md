# Roads — the junction gallery, `examples/demos/roads`

The one gallery whose input is OSM data, not hand-written geometry: how its samples are
cut, checked against the game, and shot in batches. The hub is `roads.md`; the sample
toggles mirror the knobs described in `roads.md` (**RoadStyle and RoadShape**) and the
subjects named per sample below.

`cargo run --example roads` shows a city's typical road junctions in a column —
crossings of avenues (square and skew), of an avenue and a street, of a divided
avenue and a street, of private-sector streets and of yard drives, T's into an avenue and
into one half of a divided one, a fork round a triangular island, roundabouts of several
kinds, five arms, a drive into a street, a street that narrows, a sharp bend, a dead end —
each with its full address and **game coordinates** in a caption to the left of its
window. The numbered samples carry the mechanisms of the roads subjects (`roads-*.md`):
the tapers
(`16_lanes_taper` one-way, `22_lane_change` two-way, `28_narrowing_t` per-side at a T),
the tram bed and band (`18_lawn_median`, `24_tram_bed_end`, `27_tram_through_junction`),
the merges (`25_divided_merge`, `26_pure_merge` — and Belgorod's
`01_divided_merge_junction`, the repro of a merge at a signalized junction), the
turn pocket (`21_turn_pocket`), the rings (`04_roundabout_large`, `20_roundabout_arcs`
— the "egg"), the offset joins cluster (`19_offset_joins`), the smoothed axis
(`23_s_curve`), gores (`17_ring_gores`). Belgorod's `02_links_into_avenue` is the repro
for `Highway::family` folding `X_link` into `X` (**Merges** in `roads-merges.md`).
Ryazan, Kaluga, Oryol, Rostov and Belgorod have columns of their own (`ROADS_CITY=<slug>`,
most samples with a Yandex reference; the other cities carry the types Tula lacks or
draws differently — Rostov's one-way grid, crossings of two divided avenues, a T into a
six-lane two-way street, unpaved and gravel private-sector crossings, rings of every
size, six arms in one node).
**The column is a list of junction *types*, one sample per type, with no target count**;
the rules for adding one — one type once, readable in the window at a glance, flat road
junctions only (no level crossings, no multi-level interchanges, no arch through a house:
from above the roof covers the road and it reads as a house drawn on it), checked by eye
and against the game — are written out in `examples/demos/roads/samples.rs`.
Guardrail from adding them: nodes picked from the cache by their arm count routinely turn
out to be plain crossings in the frame — pick by looking, not by count.
It is the one gallery whose input is **OSM data, not hand-written geometry**, so it is the
place to look at a road-network defect end to end:

- **A sample is a window of the game's own Overpass cache, cut at start-up** — the
  gallery reads `assets/osm/<city>_…_vN.json` once (`download::city_extract`: the cache,
  or the game's loader when there is none — same path, same file), deserializes it and
  cuts every window out of the elements with **`map::osm::crop`** (`Cropper`, indexes
  built once; `GeoRect::around`), with no JSON in between. So **a sample always equals the
  game**: raise `QUERY_VERSION` and the new tags and nodes are in the samples from the
  next start. (Frozen pre-cut files were the first shape, and every query bump would have
  left them silently behind the game — exactly what the gallery exists to catch.) The
  price is that a sample is no longer immutable: a
  re-downloaded cache may bring a mapper's edit into the junction.
  Each sample goes through the game's `parse_response` with all nine finishing passes, then
  the game's `mesh_*` and `spawn_*` doors (surfaces with the parking layout, roads,
  buildings, fences, rails, tree rows, trees; near zoom buckets). **No parked cars, by the
  author's call**: the gallery is about the carriageway and the junction, and a kerb row
  covers exactly those — the edge, the kerb return, the markings by the crossing. The
  gallery owns no geometry at all. `F5` re-reads the cache and cuts again.
- **A frozen sample is the exception, not the rule** — `data/<city>/<name>.json` beside
  the manifest wins over the cache. That is how a bug repro is pinned and how "edit a tag
  in the file, press `F5`" still works. The file is made by `ROADS_DUMP=<dir>`, which
  writes every sample as the gallery cut it (one element per line, tags sorted —
  `crop::to_json`). The cache is not even read when every sample is frozen.
- **The manifest** `data/<city>.json` lists the samples: `name` (the stem of the Yandex
  shot and of a frozen file), title, what to look at, window centre as **lat/lon** (map
  metres move with `MAP_SIZE`) and the visible `half`. Adding one: find the node
  (`tools/osm_near`), put `"at": [x, y]` in the manifest instead of `geo`, start the
  gallery — it logs the `geo` to write in its place. Every city with a column has a
  manifest; any other city shows "no samples yet". Berlin is there for the sections:
  `lanes` is on about half of its streets (Tula: 97 %), so several of its samples show
  the lane count inferred by class next to the tagged one; `06_taper_on_a_bend` is
  Leipziger Straße's taper on a bend.
- **Cost**: reading and deserializing the city's cache and cutting the windows —
  see the `roads: extract … read in …` log line; the cut itself is milliseconds behind a
  per-element bounding-box prefilter and a node → roads index for `joining_roads`.
- **What the cut does — and a way is never clipped.** Every way touching the window is
  kept **whole**, because the pipeline keys on a way's own points: a street's first point
  seeds its row of cars, a lot's outline decides its stall layout, a shared vertex decides
  whether a house is squared. The first version clipped ways to the window and the sample
  visibly stopped matching the game (other stalls in the lot, other cars in it). Roads
  that share a node with a kept road are added too (`joining_roads`, one level) so the
  kept streets have their junctions, and so are the **footways along a kept street**
  anywhere on its length (`footways_along`: a path way — the `road_class` Alley values —
  with a vertex within `FOOTWAY_REACH` 32 m of a kept carriageway's link). The parse
  decides a street's sidewalk band and its verge by probes along the **whole** street
  (60 % of them must find a footway), and an avenue kept whole with only its
  window's footways lost its verge altogether — the game, parsing the whole city,
  never had that. Only non-building multipolygons (a river runs for
  kilometres) are polygon-clipped; the tag-only `is_in` boundary (the `driving_side`
  carrier) is kept with its `name:*` tags dropped; nodes inside the window are kept (doors,
  trees today; crossings and signals once the query asks for them). `CROP_MARGIN` is 120 m
  beyond the visible half — the reach the parked cars read their quarter by
  (`cars::district`).
- **The layers are clipped to the window as meshes** — `MeshBuilder::clip_to_rect`, per
  triangle, colour and the `Ribbon`/`Roof` attributes interpolated linearly, i.e. exactly as
  the GPU would, so nothing inside the window moves. A ground-coloured mask over the rest
  was the first attempt and **cannot work**: the windows stand 30 m apart, a sample's rim
  lands *inside the neighbour's window*, and a mask only covers what is outside all of
  them. Crowns are entities,
  not a layer: they are kept by their centre.
- **Checked against the game** (offscreen shots of the same spots): roads,
  markings, kerb returns, sidewalks, buildings, shadows and lots with their stalls come out
  identical; a difference means the gallery has drifted from the game's pipeline. Worth
  knowing if the cars ever come back: a lot's cars
  match the game to the car, **the kerb rows do not and cannot** — an accepted car spends
  several RNG rolls, and how many are accepted upstream depends on the buildings along the
  *whole* street, which a window does not hold. Same rule, another draw.
- **Game coordinates are real**: a sample is projected with its city's `GeoBounds`, and is
  moved into the column by shifting the spawned entities (`place_new`, by `Added<Mesh2d>`),
  which is why samples are built one per frame and why the game's spawn doors are called
  as they are. The address is read from the sample itself (`name` of the highways through
  the centre, the country off the boundary relation).
- **The reference beside each window is a Yandex Maps screenshot of the same extent** —
  `data/<city>/<name>.yandex.png`, drawn to the right of the render at the same size
  (a missing file just leaves the place empty). The window is cut by arithmetic
  (`z=19` is 0.1747 m per CSS pixel at Tula's latitude) and the frame centre is
  **measured with a `&pt=lon,lat` marker** — Yandex centres `ll` on the visible part of
  the map beside its side panel, not on the viewport. The rest of the recipe (capture
  twice, the map paints its tiles lazily; a live browser through Claude in Chrome,
  headless Chrome gets a `limited` stub) is in the docs of
  `examples/demos/roads/samples.rs`.
- The panel mirrors the game's (`examples/demos/roads/panel.rs`): the city select (the
  game's own `qwe::ui::spawn_city_select`), a **Roads** header (the five `RoadShape`
  sliders + Sidewalks), a **Road paint** header (Markings, Paint, Crossings, Stop lines,
  Arrows, Wear, Turn wear) and four data rows — `Network`, `Ruts`, `Contours`, `Raw OSM`;
  a change rebuilds every sample. The gallery settles `RoadShape`
  like the game, hands the settled lane width to `parse_response` in `ParseKnobs` and sets
  the paint's lane-width global (`apply_lane_width`) before re-parsing its samples.
  `ROADS_SHOT=path.png` takes a frame and exits — counting the shared
  `gallery_shot.rs` frames only from the frame every sample is built and placed
  (`gallery_ready`), since the samples build one per frame and a fixed frame number left
  the late ones empty; `ROADS_SHOT_SCALE=2` takes it at twice the window's
  logical size; `ROADS_SAMPLE=N` frames sample N,
  `ROADS_CITY=<slug>` opens the gallery on that city.
- **A list of frames in one run — `ROADS_SHOTS=manifest.json`**: an array of
  `{"path": "….png", "at": [x, y], "half": 30, "scale": 2}` (`half` defaults to
  40, `scale` to 1; `at` is map metres — the card's or report's coordinates). Each
  frame becomes a one-off window (`samples::one_off`), built and shot close-up in
  turn: `read_shot_list` plants the list, `reload` rebuilds the window on every
  `index` change, `auto_shot` shoots it, and `on_shot_frame_done` advances to the
  next entry — the shared `gallery_shot.rs` gained `ShotHold` (don't exit after the
  frame) and `ShotFrameDone` (tell the gallery the frame is written) for exactly
  this. A run costs ~100 s (cargo + window + cache + crop), every extra frame a
  couple of seconds — that is the point: a before/after tour of five spots used to
  be five relaunches. The live-app alternative for a single frame is
  `brp shot-at` on the running game (`.claude/live-app-project.md`, "Live shot or
  the batch gallery").
- **Frames across revisions — `tools/gallery_batch <tasks-file>`**: lines of
  `rev|city|env|manifest` (env like `ROADS_CARS=full` or `-`), one gallery run per
  line — the review rounds' before/after collector ("было" on each fix commit's
  parent, one revision's entries grouped into one manifest). It keeps its own
  seeded worktree (`.claude/worktrees/gallery-batch`, target cloned from the main
  checkout — never a shared `CARGO_TARGET_DIR`), recovers from build dirt on
  checkout, and crops every frame with `tools/gallery_crop`: the
  1820×1820+866+36 sample-window crop of a scale-2 frame, measured against the
  review pages' close-ups so new pairs line up with old ones (other frame sizes
  rescale proportionally). Progress lands in `<tasks-dir>/progress.log`; run it
  in the background — it is minutes per revision. **Wait on the file, not on a
  clock** — poll `progress.log` (`until [ "$(grep -c 'crop: ok' <dir>/progress.log)" -ge N ];
  do sleep 10; done`, or `until grep -q 'ALL DONE' …`), never a fixed sleep: frames
  arrive every ~90 s and a clock wait idles past them.
- **Captions are always drawn**, even where they run under the panel or off the screen
  (a hide-under-panel system was removed at the user's request: zooming in made the whole
  caption vanish while most of it still lay on empty ground).
