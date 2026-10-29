# The layer seam — per-layer stories

The contract itself — `mesh_*` pure builds, `LayerMesh`, `MaterialSpec`, the adapter
shape — lives in SKILL.md, **The layer seam**. This file is what sits behind it: how the
converted modules each take the seam's shape, what the reports say, the checklist for
converting the next module, and the surface material (`map/surface.rs` holds the seam and
the material in one module).

## Flat materials and the system param

**`FlatMaterials`** (`Startup`, beside `SurfaceMaterials`) holds the two flat
`ColorMaterial`s the spec names. An **unconverted** module did
`materials.add(...)` on every rebuild — a per-rebuild allocation of a material that
never changes — and the conversion is what retired the last of them. Both resources
reach an adapter as one **`LayerMaterials`** system param (`#[derive(SystemParam)]`,
the `ui/debug/mod.rs::DebugValues` idiom), which is also where `resolve` lives. Two
separate `Res` were tried first and pushed `rebuild_tram` and `rebuild_industry` to
eight arguments, past clippy's limit; bundling them left every adapter shorter than
it had been before the seam.

## Cutoff and toggle belong in the build

**A zoom cutoff or a visibility toggle belongs in the build, not in the system.** A
far-bucket fence, an invisible tram, an invisible industry layer all return **nothing to
draw**, so the despawn in the adapter is unconditional — and that comes in two shapes,
both pinned by tests:

- an **empty list**, when the cutoff also saves work before the build — the fence's
  far bucket returns `Vec::new()` and never calls `fence_gaps` (8.7 ms on Tula), and
  `mesh_cars` off or past the last step does neither breaks nor districts
  (`fences/tests.rs::the_far_bucket_draws_nothing`);
- the module's **usual layers with empty builders**, when there is nothing to save and
  the layer order is worth seeing in the test — the tram, the industry layer
  (`tram/tests.rs` and `industry/tests.rs::the_toggle_off_draws_nothing`).

**A stepped layer takes the `ZoomBucket` itself, never a value already unrolled from
the LOD table.** `mesh_rails`, `mesh_tram`, `mesh_wagons`, `mesh_cars` and — last to
follow — `mesh_fences` all take the bucket and read their own table inside the door.
Handing the build a bare width instead (`FENCE_LODS[bucket.index].width`, as
`rebuild_fences` did) leaves half the cutoff in the system: the test then has to index
the table by hand to say which step it means, and the door can be called with a width
no step of the ladder ever produces. With the bucket a test says `for_zoom(MAX_ZOOM)`
and asserts on the report (`FenceReport::width == 0.0`), which is where the layer
carries `hidden`.

Either is safe, because `surface::spawn_layer` skips an empty builder anyway. That is
the very thing the builds' own doc comments now say from the other side —
«второй дороги, на которой можно забыть деспавн, нет» (`cars::mesh_cars`,
`industry::mesh_industry`) — a property of the shape rather than a thing to remember.
Do not confuse it with **«одно условие — одна регистрация»**, which each layer's
`rebuilds_on` states and `map/mod.rs` refers to: that one is about **double
registration spawning a layer twice**, which the seam neither removes nor touches. It
also makes the toggle testable: it used to live behind a `return` inside a Bevy
system, where no test could reach it.

Whichever shape a module takes, it says so in its **report** — see below; the shape
decides what is in the list, the report decides what the log line says.

## The report is a value, not a log line

`FenceReport`, `RailReport`: the counters `info!` used to be made of, returned so a
test can assert on them. `info!` is also the thing App Nap mismeasures on macOS, so a
returned `elapsed` is the only honest one. A module that logs nothing gets no report —
`spawn::mesh_tree_row_band` returns a bare `Vec<LayerMesh>`, deliberately; inventing a
report for symmetry would invent a number nobody reads. **The reports are not
symmetrical with each other, and are not meant to be.** `FenceReport` is `Clone, Copy,
PartialEq, Debug` — every field of it is a number, so the derives cost nothing;
`BuildingReport` derives nothing, and cannot: `Copy` is out (two `String` fields) and
no test compares it. Add a derive when something uses it, not for the symmetry; a
failure message wants `Display` anyway, which every report has and which prints the
log line itself.

**"The layer is not drawn" is a state of the report, never a zero in a counter.** A
build that the toggle or the zoom step took away must not print what an empty city
prints: `industry: 0 structures, 0 pipes` was the line a map with a chimney on it
logged, and nothing in it said which of the two had happened. So the state is a
**field**, and the counters keep saying what came in:

- the field is `hidden: bool` (`IndustryReport`, `TramReport`, `WagonReport`) or an
  existing one that already carries the fact — `CarReport::detail: Option<CarDetail>`,
  `FenceReport::width` (zero *is* "not drawn at this step"). A second `hidden` beside
  `width` would be two sources of one fact;
- `Display` then prints `<layer>: hidden`, with the free input counters after it
  (`industry: hidden (1 structures, 0 pipes)`, `tram meshing: hidden (1 tracks)`,
  `fences: hidden (429 lines)`) — that parenthesis is what tells the hidden layer
  from the empty city at a glance;
- **free** means the count is a slice length or a filter over the input
  (`mesh_tram` walks `rails` and counts `RailKind::Tram` whatever the toggle says;
  `mesh_industry` counts its own `structures`/`pipe_lines`, and only the loops move
  to the substituted `drawn_*` slices). A counter that would need the build to run
  stays zero, and that is honest rather than a stub — `WagonReport::standing` and
  `CarReport::cars` are zero because the placement genuinely does not run, which is
  the whole point of taking the cutoff before the build (`fences::pieces` likewise:
  `fence_gaps` is 8.7 ms).

Pinned by `industry/tests.rs` and `tram/tests.rs::the_toggle_off_draws_nothing` (the
input counter stays 1 and `hidden` is true) and by
`fences/tests.rs::the_far_bucket_draws_nothing`, each asserting the log line itself.

## The converted modules

**Ten modules, eleven layer doors**: `fences`, `rail`, `tram`, `wagons`, `industry`,
`cars`, `roads`, all of `spawn.rs` (the 13 surface and paint layers as
`mesh_surfaces(map, parking_layout) -> (Vec<LayerMesh>, SurfaceReport)` — the parking
layout arrives ready, because the car rows are drawn off the same one — plus the
tree-row band, its **second** door), `buildings` and `trees`.
`surface::spawn_layer` (one layer, a ready `LayerMaterial`) survives only as the
primitive `spawn_layers` is built on.

Beside the doors stands `roads::build_roads`, the roads build whole: `mesh_roads`'
layers and report plus the rut lines and the tram tracks of the same pass, as one
named `RoadBuild` — the door the game takes (`rebuild_roads`, `spawn_map`).

**Count the modules when asking "is anything left".** The tree-row band lives in
`spawn.rs` and is not `trees` — that mistake is what once made the list read "all ten"
with `trees.rs` still spawning by hand.

## trees is a scatter, and the seam takes a different shape there

On the near zoom step a crown is an **entity per tree** — its own tint, its own
micro-step of z, its own scale — so it does not fit a `LayerMesh` at all, and
`mesh_trees(style, params, planted, field)` returns `TreeMeshes { pools, tints,
crowns, merged, shadows }` instead, **for every zoom step at once**: each piece carries
the `TreeLodMask` of the steps that draw it, and a step crossing only flips visibility
(`show_tree_lod`) — except the crown entities, which `spawn_tree_meshes` hands back
as a `CrownStream` to be spawned and despawned in batches (`stream_tree_crowns`). For
the far steps (`CrownDetail::Merged`) **`merged`** carries `TreeLayer`s — ordinary
`LayerMesh`es, one `tree_crowns` chunk per `CROWN_CHUNK` square and density band,
`MaterialSpec::Crown` — so there the trees are a converted layer like any other
(`references/trees.md`, **Crown detail by zoom**):

- **`pools`** — the crown meshes, `TREE_VARIANTS` of them per concrete shape (`Mixed`
  has two pools, every other shape one), as plain `Mesh` **values**. A
  `Handle<Mesh>` would be the world, which is exactly what `MaterialSpec` keeps out
  of a build; the adapter uploads the pool to `Assets` and nothing else changes.
- **`crowns`** — `CrownPlacement { at, radius, z, shows, pool, variant, tint }`, one
  per tree the near step draws. This is what the conversion actually bought: the
  density prefix (`TreeSet::visible_count`), the species resolve off the conifer
  field, the tint slot and the z micro-step were all inside a Bevy system and
  unreachable from a test.
- **`shadows`** — the merged shadow meshes, one `TreeLayer` (a `LayerMesh` plus the
  `TreeLodMask` of the zoom steps that draw it) per density band, shadow template
  (full or thinned far) and `CROWN_CHUNK` square, built for every
  step at once and only shown or hidden by a step crossing (`show_tree_lod`;
  `references/trees.md`). Its colour moved **into the vertices** (`shadow_template`
  pushes `SHADOW_COLOR`) so the layer can be a plain `MaterialSpec::Blend`, the way
  every other shadow on the map already was; before that the layer allocated a
  coloured `ColorMaterial` on every rebuild. `tree_gallery` lays its own grid and
  therefore does not go through `spawn_tree_meshes`, but it had to follow the colour:
  its shadow material is now a blended white one.

So `spawn_tree_meshes` is the adapter, and it is the **one** place on the map that
still writes `DespawnOnExit` by hand — for the near-step crown entities. Every merged
layer, the far crown chunks included, gets it from `spawn_layer`.

## cars is the one whose build is a layer rather than a mesh

Every other `mesh_*` takes the data it draws; `mesh_cars(bucket, style, &Drawn, map,
layout)` (`Drawn::nodal(map, shape)` — the prepared roads the row stands on, built by
the adapter with the settled `RoadShape`, so the curve tolerance and the taper move
the row with the ribbon; `references/roads.md`, **The drawn network**) takes the whole
`MapData` (as `mesh_roads` does) and does the **assembly** as well — junction breaks,
`Districts`, `park_cars`, `fill_lots` — because that assembly is exactly what the
cutoff and the toggle gate. Off, or past the last zoom step, none of it runs and the
list is empty, so a hidden layer still costs what the old `return` inside the system
cost. The private `mesh_bodies(&[Car], CarDetail)` underneath is only the mesh; it
carried the name `mesh_cars` until the layer function took it. `CarReport::detail` is
an `Option<CarDetail>`, and `None` is what the `cars: hidden` log line prints — every
other counter is then zero, because the assembly those counters would count is exactly
what did not run. That is the one shape of the rule above (**"The layer is not drawn"
is a state of the report**); the four modules whose input counters are free print
theirs beside the word.

**The game splits the door at the zoom crossing**: the assembly does not depend on
the bucket, so `rebuild_cars` keeps it in `cars::CarPlacement` (`park_all` →
`ParkedCars`, keyed on the occupancy and the `RoadShape`, reset on world entry by
`forget_parked_cars`) and a crossing costs `mesh_parked` — the bodies only;
`CarReport::placed` says which of the two a log line was. `mesh_cars` is still the
whole thing in one call, for the tests.

**The bench follows that split, the gallery still assembles on its own**:
`measure_cars` is `Drawn::nodal` on its own row (`drawn`), the placement once
(`placement`, with `breaks` as its share) and then `mesh_parked` once per detail step —
the `cars *` rows are exactly a zoom crossing (`ParkingLayout::new` is built for it,
outside the timer: it is the layer's input, not its cost); `cars_mesh` is
the gallery's one door and builds with neither lots nor districts on purpose. The
steps the gallery repeats are the game's calls, not look-alikes: it breaks the row with
`pockets::row_breaks` over `Tapers::of_map` (the bench takes the tapers off `Drawn`, the
gallery over an empty network and no nodes, as its axes are), never the bare
`marking_breaks(is_carriageway)`, which let a service drive leave a gallery row
unbroken and made the bench time a cheaper function than the game runs.

## Buildings was last, and not for being big

Two things are peculiar to it and worth knowing before touching it:

- it is the only module needing **`MaterialSpec::Roof`**, and the variant was added
  exactly when it arrived — before that nothing could construct it;
- it spawns under **two tags** (`BuildingLayerTag`, and `BuildingShadowTag` on its
  own rebuild schedule — the zoom bucket does not touch the shadows), while
  `spawn_layers` takes one tag per call. Hence `BuildingMeshes { layers, shadows }`
  and two calls, wrapped in `spawn_building_meshes` so `rebuild_buildings` and
  `spawn_map` share one door. Carrying the tag *inside* `LayerMesh` was rejected: a
  tag is what the **adapter** despawns by, not a property of what was drawn.

The local closure in `spawn_buildings` that shadowed the name `spawn_layer` is gone
with the conversion; it now just pushes into a `Vec<LayerMesh>`.

The 13 surface layers still spawn with tag `()`, and after the conversion that
is visible as one line in the list rather than a silent argument. They are not
rebuilt by anything, so they have nothing to be found by; giving them a tag is worth
doing together with a reason to rebuild them, not before.

## When a layer rebuilds

**When a layer rebuilds is the layer's own business** — `rebuilds_on()`, a run
condition next to its `rebuild_*`, and `map/mod.rs` only wires it
(`roads::rebuild_roads.run_if(roads::rebuilds_on())`). The reason a gate lists what it
lists is a fact about the layer: `roads` carries `SunOnMap` **because the bridge
shadow is baked into its mesh**, and that is something you need to know while editing
`roads.rs`, not while reading the plugin. Before this it was a ten-line comment in
`map/mod.rs`, a file the layer's author has no reason to open.

**One layer takes its condition from another module**: `trees::rebuilds_on()` gates
the whole four-system chain — `recompose_row_trees`, `retune_conifer_field`,
`spawn::rebuild_tree_row_band` and `rebuild_trees` — so the tree-row band, which
lives in `spawn.rs` and is not `trees` (see **The converted modules** above), is gated
from `trees.rs`. What rebuilds is the chain, so the chain is what the condition
belongs to.

- **One condition, one registration**, and it is written on every one of them. Two
  copies of one system in one schedule can both fire in a frame: the second one's
  despawn runs against data taken before the first one's commands were applied, and
  the layer spawns twice. That is not theory — the industry layer arrived with
  `rebuild_industry` listed twice. So conditions are summed with `or_else` rather
  than split across registrations, and the rule now has a single place to live
  instead of the three comments that used to repeat it.
- `trees::switches_on` goes through `IntoSystem::into_system` because a bare
  function-condition carries its own type marker; the other layers' `or_else` erases
  it. That is the only oddity in the shape.
- **A full layer registry was considered and rejected.** Declaring a layer as data —
  tag, triggers, build, z, material — and letting one generic system register it
  would close the double-spawn trap by construction, and the trap is real. It would
  also cost an associated-type-per-layer trait and one indirection between "what is
  drawn" and "when", to replace eleven adapters of about ten lines each. Most of what
  that card was written against is already gone: the per-rebuild material
  allocations, the redundant `is_empty` guards and the two modules writing
  `DespawnOnExit` by hand all went with the seam itself. What was left was the gates'
  prose living away from its layer, and that is what `rebuilds_on` fixes.

## Converting a module

Converting a module means: lift the build to `mesh_*` returning
`Vec<LayerMesh>`, derive `Clone, Copy` on its `*LayerTag` (`spawn_layers` hands the
tag to every layer), move any cutoff or toggle into the build — a zoom ladder as the
`ZoomBucket` itself, not as a width read out of the table by the adapter — drop its
`materials.add(...)` and its now-redundant `is_empty` guard, write its `rebuilds_on()`
beside the `rebuild_*` (**When a layer rebuilds** above), and write the tests the seam
has just made possible. Do not add a `MaterialSpec` variant before a module
needs it — the `Surface` one sat unconstructed until the tree-row band arrived, and
the compiler said so. A module whose entities are **not** one merged mesh per layer
(so far only `trees`) returns its own struct instead of a bare `Vec<LayerMesh>`, and
the rule that survives is the division, not the return type: the build says what is
drawn, the adapter says where it goes.

## Surface material

`map/surface.rs`, shader `assets/shaders/surface.wgsl`, a `Material2d` with its own
vertex + fragment stage — procedural texture without a single asset: the vertex colour
is the base, and the fragment multiplies in noise sampled by **world position**, so two
overlapping ribbons of one layer get the same pixel (the junction trick survives). The
zoom rule (`visible(wavelength, px)`) and the shared `assets/shaders/noise.wgsl`
library are in SKILL.md, **The noise rule**.

Per `SurfaceKind` (`Ground | Yard | Park | Wood | Grass | Sand | Water | Street |
Alley | Sidewalk | Unpaved`) a `SurfaceParams` uniform:

- **mottle** — four octaves of value noise from `mottle_scale` down to an eighth of it,
  with a per-channel `tint` shift so a lawn goes yellow-green ↔ blue-green, not just
  light ↔ dark;
- **grain** — three octaves from `grain_scale` down to a quarter;
- **speckle** — a thresholded noise field → sparse dark dots, grass tufts and
  undergrowth on Park/Grass/Wood;
- **drift** — the mottle slides with `globals.time` — only Water;
- **shore** — `shore_color` / `shore_width` — only Water: the channel ribbon's shoal by
  its `across` (see `references/layers.md`, **Waterways**);
- the **wear** block (Street — wheel ruts on the lane frame; a bridge deck is the same
  kind and carries its street's ruts, a footbridge in the same mesh has no lane frame
  and stays bare). The lane **lines** are not this shader's any more: they are the
  paint layer (`roads/paint.rs`, `paint.wgsl`, `references/roads.md`).

Materials are built once (`SurfaceMaterials`, `Startup`) and
shared by every city; `SurfaceStyle::texture` (section **Surfaces**, `ui/surfaces.rs`,
persisted) rewrites the `intensity` uniform of each and rebuilds nothing.

**This is the map's only grain.** A second one — `map/grain.rs`, a map-sized sprite
tiled with a 256 px seamless simplex texture — was written in the building-look
worktree and dropped when that branch was merged on top of this material; the commits
describing it are history, not a missing file. Don't reintroduce a sprite grain: two
noise fields over the same fills read as dirt, and the shader one already fades by
pixel size.

The material demands the **`Ribbon` vertex attribute** (`meshing::ATTRIBUTE_RIBBON`,
in metres: `[across, to-break, half width, 0]` on a ribbon without lanes, `[across,
to-break, low, high]` from the lane grid node on a carriageway with a **lane frame**
(`MeshBuilder::set_lanes` / `set_lane_taper`, `meshing::LaneFrame`); *to-break* is the
signed distance to the nearest break — a marking break on a street, a mouth on a
channel) and a mesh gets it only from `MeshBuilder::with_surface_coords()`;
`push_ribbon` / `push_ribbon_shaped` fill it from the ribbon frame (quads: ±half width;
join fans: the outer side; round caps: the projection onto the normal, with *to-break*
extrapolated past the node along the last quad's slope), polygons get zeros. It costs
16 bytes per vertex, which is why building, crown and overlay meshes are built without
it. Where the breaks come from and why the mesher inserts a vertex at every kink of
*to-break* is `references/roads.md`, **Markings → Breaks**.
