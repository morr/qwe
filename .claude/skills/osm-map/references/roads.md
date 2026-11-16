# Roads — the hub

Detail behind `map/roads.rs`, `map/roads/*` and the street half of `surface.wesl`.
`SKILL.md` keeps what every layer shares — the layer seam, the surface material and its
`Ribbon` attribute, the shadow rules, zoom buckets — and the `RoadLine` model; the big
lot's kerb, the boulevard's double line over the lot and the gores at a roundabout are in
`parking-big-lot.md` (**A big lot shows the road through it**); the parse passes that read a
road's width are in `parse-geometry.md`. This file is the map: the phases of the build
and which subject file carries which piece.

## The phases of the build

- **The phases of the build** — `build_roads` is a thin door now (raw OSM returns
  early; otherwise `road_skeleton` + the one step final, plus the merged composition
  over it). The phases live in
  three places, the section banners (`// ── name ──`) in their bodies the authoritative
  inventory. `road_skeleton`: the drawn network (`Drawn::new`, which carries no step of
  its own), junctions and kerb returns, merge edges, kerb pockets, turning circles,
  ribbon axes and the fill order, gores, ring islands and mouths, merge ramps, the
  medians of paired halves, road areas and safety islands. `RoadCommon::new`: each
  step's corner rounds, then the shared middle — kerbs, pockets, mouths, medians,
  islands, node paint, tram, crossings, walls, and the **ribbons** (asphalt, sidewalk
  bands, verges, paint, tapers, bridge decks), laid street-wise in threads
  (`roads/ribbon_lane.rs`). The one final — `finish_step_split` (the split door,
  plus the common layers out): what a step owns
  (verge bands, lawn scraps — paving, then green), the gores' asphalt, the lot layers,
  the rings' island grass, and the layer assembly, z-sorted, with the report.

## The subject files

The bold titles are the section names the other references point at
(`references/roads.md`, **Section** now resolves through this index).

- `roads-sidewalks.md` — **Sidewalks** (the band, the `sidewalk=*` sides, **Paved
  paths**, **Unpaved streets**, the tile overlaps), the **verges** and their lawns
  (**A lawn scrap is tiled**, the kerb pads, **One profile for the parse and the
  renderer**), **Kerb pockets**.
- `roads-pairs-medians.md` — **Paired halves**: **Finding**, **Queries**, **Alignment**,
  the **Paved median**, the **Lawn**, the pocket at a median end, the **Tram bed**, the
  **Laid tracks** and the **Tram band**, **Where it opens**.
- `roads-streets-network.md` — **Streets, sections, tapers**, the **network overlay**
  and the **OSM contours overlay**, **Raw OSM, drawing level**, the **one-off window**,
  the **Ribbon** primitive, and **The drawn network**: **`Drawn`** and its three
  **`Axis`**es, **The street axis** (smoothing, pinned nodes), **Stitches**, **Lane
  width is a world reload**, **Smoothing off the street axis**.
- `roads-markings.md` — **Markings — the paint layer**: the **One lane frame**, the
  taper paint, the dashes, the solid approaches, **Transverse paint**, the paint layers
  and their z, the **LOD**, **`RoadPaintStyle`**, **Lane count**, **Breaks**.
- `roads-junctions.md` — **Junctions** (the shared nodes, the **five sets of breaks**,
  the **fill order**), **Driveway crossings**, **Turning circles**, **Safety islands
  and carriageway areas**.
- `roads-junction-paint.md` — **Junction paint**: the clusters, who breaks and who
  leads, the zebras and stop lines, the ring entries and exits, the link arms.
- `roads-corners.md` — **Kerb returns**: the fillet and its radius table, the butt
  ends, the **outer corner** fan, the **nose**, the **bend**, the obtuse-corner wedge,
  the **fork gore**, the small paved islands, and **The sidewalk turns with the kerb**.
- `roads-rings.md` — **Roundabouts**: the fit, the pinned nodes, **bend_approach**, the
  Y-legs and **A tail Y is straightened in the data**, the webs, one section and the
  closed fill, the island lawn and grass.
- `roads-merges.md` — **Merges**: the wedge, the ramp, the merge axis, the nose asphalt,
  and **split_mouths** at a crossing.
- `roads-bridges-wear.md` — **Turn paths** (the wear and the lane arrows), **Bridge
  layers** (shadows, decks, curbs, seam ends, slit aprons), **Asphalt wear** (the lane
  ruts, the rut lines overlay).
- `roads-gallery.md` — **The junction gallery** `examples/demos/roads`.

Cross-subject facts, one line each: the **fill order** (narrow first, leaders last, per
junction) is an invariant of the ribbon pass — mechanics in `roads-junctions.md`;
the layer z's (`Z_SIDEWALK` 1.6 … `Z_BRIDGE` 2.2) are named in their owning subjects,
the shared seam in `SKILL.md`; `RoadJoin`/`ROAD_JOIN` live in `roads/shape.rs` (beside
`RoadStyle`), `Smoothing` in `map/smooth.rs`.

- **Chasing a road defect** — the probe ladder for road layers lives in the live-app
  debugging skill (`.claude/live-app-project.md`, "Which layer draws a glitch"):
  `brp probe x y` → `brp layers` + `brp hide` + `shot-at` → `tools/osm_near --verts`,
  and only then an env-gated temporary probe beside the code it reads, removed with its
  test when the defect closes. A red-green test over the pure build (`mesh_roads` and
  friends) usually pins the mechanism better than the probe that found it.

## RoadStyle and RoadShape

The style/geometry resources themselves, their ranges and the rebuild/reload/settle
mechanics live in `CONTEXT.md` (**Style resources**); this is the roads-side residue:

- **RoadStyle** toggles (resource, BRP-writable, persisted; `ui/roads.rs` /
  `ui/road_paint.rs`): **sidewalks**, **markings** (both on), **crossings**
  (`CrossingMode`: `Off` / `Osm` / `Generated`, the default), **stop_lines** (on),
  **arrows** (**off**) — described in **Junction paint** (`roads-junction-paint.md`).
  Any change reruns `rebuild_roads` (despawn `RoadLayerTag` layers, respawn from the
  unchanged `MapData`). Old `join`/`smoothing`/`casing` keys in `settings.toml` are
  ignored silently; `RoadJoin` and `Smoothing` survive for the tree-row band's own rows
  and rails/tram/water.
  `mesh_roads` yields **25 layers** (`roads/fixtures.rs::LAYERS`): twelve ribbons (with
  `unpaved_roads` and `road_verges`), the wide verges' lawn as meadow and as yard grass
  (`road_verge_lawns`, `road_verge_yards`), the ring islands' lawn and their grass
  without the rim, eight paint layers, and the level-crossing decks (`rail_crossings`).
  `bridge_casings` stays — it is the bridge curb (**Bridge layers** in
  `roads-bridges-wear.md`);
  `footprint::casing_width` stays for the tree-row band and the planting index.
- **RoadShape plumbing** — the knobs' ranges sit beside the type and the accessors
  clamp on read; signatures carry the shape: `mesh_roads(map, style, shape)`,
  `mesh_cars(bucket, style, shape, map, layout)`, `axis::street_axes(.., &RoadShape)`.
  The map follows **RoadShapeOnMap** (`settle_road_shape` copies after 0.35 s of
  quiet, `seed_road_shape` at Startup, `register_pref::<RoadShapeOnMap>`);
  `roads::rebuilds_on` = `RoadStyle` | `RoadShapeOnMap` | `SunOnMap`,
  `cars::rebuilds_on` reads `RoadShapeOnMap` instead of `RoadStyle`.
- The **lane width** reload mechanics and the **smoothing** off the street axis are in
  `roads-streets-network.md` (**Sections**, **Smoothing off the street axis**).
