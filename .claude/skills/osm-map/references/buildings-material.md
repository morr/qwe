# Buildings — roof and wall material

Siblings: `buildings-heights.md` (height modes, extrusion, shadows),
`buildings-clutter.md` (what stands on the roof), `buildings-garages.md` (garage runs —
codes `7…8`, the `WallFrame::run` grid), `buildings-landmarks.md` (the `Sacred`
exceptions). `CONTEXT.md` carries the **Roof material** glossary entry and the code
dictionary.

- **Roof material** (`buildings/material.rs`, shader `assets/shaders/roof.wgsl`) — what
  the roof is *covered with*. The goal is the aerial photo: from above, a roof is a
  **material** first (rolled bitumen with its seams and repair patches, gravel ballast,
  standing-seam metal, corrugated sheet, tile, PVC membrane) and a colour second, and the
  old per-`BuildingUse` roof colour plus a ±3 % tint by list index made a district read as
  a dress pattern.
  - **The pick** (`roof_look`) — a **seed hashed from the building's first vertex**
    (three xorshift-multiply rounds over the centimetre coordinates; the door generator
    is seeded from the same point, so both survive a rebuild and neither depends on the
    order of `MapData::buildings`) chooses a slot in a **ten-slot table per
    `BuildingUse`** — ten slots so a table reads as percentages: `House` is 5 tile / 2
    seam / 2 corrugated / 1 bitumen, `Apartments` 7 bitumen, `Industrial` 5 corrugated,
    and so on. `Church` takes its material and palette from its faith
    (`temples::roof_kind` / `roof_palette` — Orthodox seam metal, Western and Eastern tile,
    mosque gravel), the Kremlin from `fortress` (seam on a tower, gravel on the wall's
    walkway), and `Other` — half the city — splits by
    footprint at the same `SMALL_FOOTPRINT_MAX` 250 m² the gable rule uses: a small box is
    a private house, a big one a block.
  - **The colour** comes from that material's own palette (3–7 plausible shades, picked
    by another slice of the same seed, then ±3 % of value). **Value is calibrated against
    the aerial photo at the city zoom**, where the texture has faded and the base colour
    is all that is left: a panel block's bitumen is a mid grey there (~0.55 sRGB, palette
    0.50–0.63), not the 0.38–0.50 of the first version, which on the map's ~0.9 ground
    read as dirty-dark boxes — fine close up, unreal zoomed out. The spread differs by
    what the roof covers: the **flat roofs of blocks** (bitumen, gravel, membrane) are
    **tight in value and wide in hue** — neighbouring panel blocks on a photo differ in
    shade, not in brightness, and a wide value spread there reads as confetti — while the
    **private sector** (tile, and the seam / corrugated it also draws) is **bright and
    many-hued**: red and brown metal tile, green, blue, silver and dark slate stand fence to
    fence, and a palette of greys made the whole quarter one colour.
  - **The texture** is a `Material2d` in the shape of `SurfaceMaterial`: one material for
    the whole app (`RoofMaterialHandle`, built at `Startup`), a `RoofParams` uniform
    (`light` = `map::sun_light()`, `intensity` = `RoofStyle::texture`) and a per-vertex
    **`Roof` attribute** (`meshing::ATTRIBUTE_ROOF`, `[axis x, y, material code,
    seed]`). All four numbers are constant over a **face** — the roof's axis is the
    building's long axis, a wall's is its own direction — so the attribute is a
    *builder state* (`MeshBuilder::set_roof`), like the lane frame of a street, not an argument of
    every `push_*`; the fragment reads it `@interpolate(flat)`. Code `0` means **no
    texture** — roof clutter rides in the same mesh (2.5D is one
    painter's-order layer) and comes out with its vertex colour untouched. Walls rode
    at `0` too until they got codes of their own (`WallKind`, below), and so did the
    **gable**: it is the top of an end wall, takes the same `wall_frame` as the wall
    under it, and would otherwise break the pattern at the eaves.
  - **What the shader draws**, by world position rotated into the building's long axis
    (`min_area_rect`'s first edge), phase-shifted by the seed so neighbours' seams do not
    line up: bitumen — 0.95 m roll seams, scattered repair patches (as many as the roof's
    **age** below), ponding stains; gravel —
    strong fine grain and bright specks; seam metal — a lit rib and its shadow every
    0.62 m; corrugated — a 0.30 m wave plus 1.05 m sheet laps; tile — 0.32 m rows with a
    shadow line and per-tile jitter; membrane — 2 m sheet seams. **Rolls and tile rows run
    along the ridge (constant `v`), seams and corrugation run down the slope (constant
    `u`)** — the first version had the corrugation along the ridge, and water would have
    run along the rib rather than down it. Rib lighting is scaled by
    `|axis · light|`, so ribs pointing at the sun neither highlight nor shade. Every
    octave and every stripe grid fades by `visible(wavelength, px)`, the `noise.wgsl`
    rule, so nothing moirés when zoomed out; at the city zoom the texture is simply gone
    and only the material's colour is left. The noise helpers come from
    **`assets/shaders/noise.wgsl`** — its full set is named under **Surface material**
    in `SKILL.md` — and this shader imports by path the subset it calls
    (`#import "shaders/noise.wgsl"::{hash21, value_noise, visible, fbm3, stripes}`);
    the `visible` rule in particular must not drift between the shaders that import it.
  - **The wall is the same mechanism on its own coordinates and its own codes**
    (`layers.rs::wall_frame`, `meshing::WallFrame` — the same frame the garage ribbon
    in `buildings-garages.md` is measured in). A 2.5D wall is a **parallelogram** —
    base edge `a→b`, side edges along the lean — and what the builder writes into
    `ATTRIBUTE_ROOF` at each of its vertices is where that vertex stands *in it*: **the panel
    number along the base and the storey up the lean**. Both linear in the point, so each is
    one dot product (the frame holds the rows of the inverted `[base, lean]` basis, already
    scaled by the counts), and the attribute is therefore **interpolated, not flat** — a
    roof's own numbers are equal on all three vertices of any triangle, so nothing changes
    for it.
    - **The counts are whole**: `round(wall length / PANEL_WIDTH 3.2 m)` panels and
      `round(building height / STOREY_HEIGHT 3 m)` storeys, at least one of each. That is
      the whole construction, and it is what the first version got wrong: it laid a **global
      metre grid** over every wall, and a grid that does not know where a wall ends cuts the
      edge panel and slices balconies in half at the corners. With whole counts the edge
      panel is whole, the top storey ends exactly at the eaves, a balcony inset inside its
      cell cannot reach the wall's edge, and at a corner both walls end on a whole panel and
      a whole storey — so their seams meet, which the global grid could not do either
      (its phase was arbitrary, and the two walls of one corner disagreed).
    - **The panel width is a target, not a divisor**: a 40 m wall takes 13 panels of 3.08 m,
      a 10 m one 3 panels of 3.33 m. A real panel block divides its facade evenly too.
    - **The shader needs no metres at all** — no panel width, no storey height, no lean:
      `fract` of what the vertex carries is the position inside the cell, and `fwidth` of the
      same number is cells per pixel, which is how each grid fades (`visible(1.0, px_cell)`).
      Foreshortening comes out of that derivative for free: a west wall, squeezed by the
      lean, loses its storeys earlier than a south one without a line of code about it.
    - **`WallKind` is to the wall what `RoofKind` is to the roof**, and the codes are one
      dictionary in one attribute slot: `0` no texture, `1…6` roofing, `7…8` the two garage
      runs, `9…15` cladding, `16` the door leaf (`WallKind::code` derives itself from
      `RoofKind::CODES`, the
      **last** roofing code rather than the length of `ALL` — the garage runs are outside
      `ALL` and would otherwise have been overwritten by the claddings — and `DOOR_CODE`
      derives from `WallKind::CODES` the same way, so a new roofing shifts the wall codes,
      a new cladding shifts the door, and the shader's mirror is edited whole).
      Eight claddings —
      `Panel | Brick | Plaster | Shopfront | Shed | BigBox | GarageDoors | Sacred` — because panel seams with
      balconies are
      exactly **one** kind of building, and while the wall was one, a garage and a church
      wore them too. Six of them are chosen by the tables below; `Sacred` by the class
      alone (every `Church`, before any table); **`GarageDoors` is chosen
      by geometry**, like the garage runs on the roof — `layers::wall_of_run` puts it on
      every box of a run and nothing else can reach it, which is why
      `every_cladding_reaches_the_city` skips it.
    - **The pick** (`material::wall_look`) is `roof_look`'s twin — a ten-slot table per
      `BuildingUse`, the slot from the building's seed, the colour from the material's own
      palette plus ±3 % — with one difference: **height is consulted before the tag.**
      Anything under `LOW_RISE_STOREYS` (4) that is not already `House`, `Garage`, `Church`,
      `Industrial` or `Retail` (which is settled by its size) drops into `LOW_RISE_WALLS`,
      because a low building is neither a panel block nor a curtain wall whatever OSM calls
      it — and the retail exception is only about the curtain wall: neither trade table
      carries a panel. **A hull deeper than housing is not housing**: a `BuildingUse::Other`
      (`building=yes` and the like — no apartments/residential tag) whose plan is deeper than
      `DEEP_HULL_MIN` 30 m (`material::is_deep_hull`) takes `DEEP_HULL_WALLS` — shed 7,
      plaster 3, no panel and no brick, so no balcony either (`balcony_house` asks the
      cladding) — before the low-rise arm, since a one-storey 100 m hull is a warehouse, not
      a cottage. A residential section is 12–18 m deep, a point tower up to ~25. Depth is
      read **twice and both must pass**: the short side of `min_area_rect`, and the short
      side of the *equivalent rectangle* (same area and perimeter as the plan, courtyards
      subtracted and their rims added) — equal to the span on a solid box, the wing width on
      a courtyard block, an L or a U. An area rule (> 3000 m²) was rejected because it
      catches long 20–29 m slabs. The roof is
      not moved: `kind_of` still covers a large `Other` with `APARTMENTS_ROOFS`.
      The seed is read from **other bytes**
      than the roof's (`>> 4`, `>> 12`, `>> 20` against the roof's raw, `>> 8`, `>> 16`):
      the two materials must be independent, or every panel block would also be under one
      bitumen. Kremlin is brick (every wall `WallMark::Solid` — no window, and `push_doors`
      skips it), `Church` is `WallKind::Sacred` coloured by faith — the same two exceptions
      the roof has, in the same order.
    - **Wall colours are calibrated the other way from roofs**: a wall is **lighter than its
      roof**, which is what holds the 2.5D box together (dark bitumen over light panel), so
      panel and plaster live in 0.66–0.86 and only `Shopfront` is deliberately darker — and
      it goes to shopping centres, of which a district has a couple. Spread follows the roof
      rule: tight in value and wide in hue for mass housing, bright and many-hued for the
      private sector, where ochre, whitewashed brick and blue plaster stand fence to fence.
      The **per-use facade colours are gone**; they painted half the city (`building=yes`)
      in one tone.
    - **What each cladding draws between the openings**: panel — floor seams, panel joints
      and a ±2 % tone jitter per *cell* (a chequerboard is the right answer here, the wall
      really is assembled from separately cast slabs); brick — courses at a twelfth of a
      storey, so they fade first and leave an even tone; plaster — `fbm3` streaks and no
      seam at all; shopfront — a spandrel band between the glazing strips; shed —
      corrugation ribs at a sixteenth of a panel plus a faint eaves line.
    - **A window is the only thing here that replaces the surface colour instead of
      correcting it.** Glass is not plaster some per cent darker, so `wall_shade` returns
      three numbers (`Wall { shade, glass, sky }`) and the fragment `mix`es toward
      `mix(GLASS_ROOM, GLASS_SKY, sky)` — both **linear** constants, because the vertex
      colour here is linear (`wall_colors` returns `LinearRgba`). `sky` rises up the pane
      (dark room below, reflected sky above), dips under the lintel for the reveal shadow,
      and is scaled by a per-window `tone`: a curtain, an open sash, dirty glass. Without
      that per-window draw a row of windows reads as a stencil.
    - **The opening is what a cladding is really about**: a two-sash window per panel (0.42
      of the panel, 0.30…0.72 of the storey), a narrower brick one, a small house window on
      plaster, a full-panel glazing strip on a shopfront, a high narrow ribbon on 55 % of a
      shed's panels. Mullions are placed **from the left edge of the opening**
      (`stripes(inside.x - 0.5 + half, wide / panes, …)`), so any pane count comes out
      right; centring them on the middle only works for even counts.
    - **The ground floor is its own case on every cladding**: never a balcony, a shopfront
      lower and taller there than the strip above it, and over it all a dark **plinth** band,
      the line that says where the building stops and the ground begins. The entrance is
      *not* drawn here — it comes as geometry from the data (the door bullet below); the
      shader rolls no doorway of its own on any cladding.
    - **The top of a wall needs room, not a stripe.** A wall is drawn exactly
      `storeys × 3 m` and a real building is not: above the last storey sit the ceiling, the
      roof slab and the parapet, and without them the top window butts straight into the roof.
      The first fix drew a **cornice** — a light coping with a dark seam — and changed
      nothing, because it painted inside the same `1 - WINDOW_HIGH` that was already there:
      the gap stayed identical to the pixel and one more line appeared. It was reported as
      exactly that and taken back out.
      What works is `meshing::PARAPET_CELLS` (0.15): the frame runs the storey coordinate to
      `storeys + PARAPET_CELLS`, and everything above the last whole storey is **plain wall
      with no openings** — nothing is drawn there at all. Whole cells survive, because the
      invariant is that *storey boundaries* are whole, not that the wall ends on one; the
      cost is the drawn storey shrinking by `1/(storeys + 0.15)`, 1.5 % on a nine-storey
      block.
      The check that openings stop at `storeys` is not belt-and-braces: a window starts at
      0.30 of a storey and misses 0.15 on its own, but a **balcony** starts at 0.01, and its
      slab shadow and slab edge would climb into the cornice.
    - **Knowing where the top is takes the storey count**, and that rides **in the material
      slot** beside the code: `meshing::STOREY_STRIDE` (32 — raised from 16 when
      `WallKind::Sacred` pushed the door to code 16) puts the code in the remainder and
      the storeys in the quotient, zero on a roof. That slot is the one field with a spare
      digit; a fifth float in the attribute would cost four bytes on every vertex of the
      building layer. `meshing::unpack_material` is the Rust mirror and exists only for the
      test that pins it — in the game the slot is written, and read by the shader alone.
    - **A *line* goes through `stripes`, never `cell_band`** — learned from the cornice seam
      before it was removed, and it still holds for every line here: `stripes` floors its
      width at one pixel (`max(width, px)`), `cell_band` does not, so a 0.05-cell seam (five
      drawn centimetres) vanishes at every zoom where the wall is visible at all. It took a
      probe run with the amplitude at 0.9 to tell "the branch never runs" from "the branch is
      too faint", and it was the second — worth remembering as the way to split those two.
      `cell_band` stays right for a *band* — plinth, balcony rail — which is thick enough to
      survive on its own.
    - **One opening per cell, on every cladding.** The shed briefly had two — a gate and the
      ribbon window, «because they sit at different heights» — and that stopped being true
      the moment the ribbon dropped from 0.60 to 0.45 to survive the fade: the window's
      sashes climbed onto the gate leaf and its bottom row came out as a stump over the dark
      rectangle. Different heights are not enough, because an opening owns its reveal and
      sill below it too, so the gap between two would have to allow for the frame; picking
      one of the two is the version that cannot drift.
    - **A door comes from the data, as geometry** (`layers::push_doors`, code `DOOR_CODE`
      12), and this is the one opening the shader does not place. It used to roll one on
      `DOOR_SHARE` (0.24) of the ground-floor columns and a gate on 0.30 of a shed's, which
      put drawn doors where `osm::entrances` has none and left the real entrance — the point
      the door gizmo marks and the pawn walks to — on blank wall.
      Two quads per door, both pushed **after** the wall they sit on:
      - a **patch** over the whole cells the leaf touches, carrying the wall's own frame and
        code so seams and courses run through it, marked `WallMark::Solid` (no openings).
        It is what stops the cell's window from peeking out beside the leaf: a window is
        centred in its cell, a door stands where the data put it, and they overlap. Whole
        cells, not the leaf's span — clipping a window in half looks worse than losing it;
      - the **leaf**, exactly on the entrance point, with `WallFrame::opening` — a frame
        with neither storeys nor parapet that maps the quad to `[0, 1]²`, so `doorway_of`
        draws the leaf at its centre and `DOOR_LEAF_WIDE/HIGH` are fractions of the opening
        rather than of a cell.
      **The metres are chosen on the CPU** (`layers::door_size`, by cladding: подъезд
      1.9 × 2.8 m, house door 1.3 × 2.4, shop leaves 2.4 × 3.0, shed gate 3.2 × 2.9) — a door
      is a scale ruler, and a gate that is wider than it is tall reads as a letterbox slot
      (the old `GATE_WIDE` 0.62 × `GATE_HIGH` 0.50 was exactly that, 2.0 by 1.5 m).
      A door at a ring **vertex** — which is where every real OSM `entrance` sits — is pushed
      inside the wall by half a leaf, and claimed by the edge it *starts*, so the two walls
      of a corner do not draw it twice. **The claim is by the nearest edge, not by every
      edge in tolerance** (`layers::door_edge`, ties to the earlier edge in the ring):
      `DOOR_ON_WALL` is half a metre and an OSM step
      can be twenty centimetres, so both edges took the door, each shifted its leaf inward
      to make it fit, and the wall came out with two doors side by side under a single
      gizmo. Cost: eight vertices per door, on drawn walls only.
      **Snapping the leaf to its panel instead was rejected**: it saves the geometry but
      leaves up to ±1.6 m between the drawn door and the entrance — the very gap this
      change exists to close — and packing a column index into the material slot caps out at
      twelve columns (≈38 m of wall), which is exactly where a second подъезд appears.
    - **A balcony is a stack of bands**, not a box: the slab's shadow on the wall, the bright
      slab edge, the parapet (its tone by its own draw, from light panel to dark sheet), and
      above it either glazing or an open recess in shade. 72 % of a panel, and along the wall
      **by the period of a section**: `(column + phase) % period < BALCONY_FILLED`, the
      period 4–6 panels (`BALCONY_PERIOD_MIN` + `BALCONY_PERIOD_SPAN`) and the phase both
      rolled from the **wall's** seed, two filled columns in it — a column the whole height of
      the wall, as on a real block, and a repeating step between columns.
      **A share is not a row**, and that is what the period fixes: the first version drew
      each column independently at 58 %, which on a ten-panel wall routinely gives three
      balconies in a row and then two blanks — a scattering with no step, exactly what the
      spec's «регулярным рядом» is not. The share is still a majority-ish (2 of 4–6), so
      nothing about the *density* changed; the dice survive only as the phase, so that two
      walls meeting at a corner do not start alike.
      Hashing the *cell* for **existence** is the
      thing that must not be done either: that is an independent draw per cell, i.e. a
      chequerboard; hashing it for *glazed or open* is fine and is what varies a column.
      A brick building's balconies are **recessed loggias** — no slab overhang, no bright
      edge, a deeper shade — and rarer: the same period with `BALCONY_FILLED_RECESSED` (1)
      column of it filled.
      **Two sashes, not four.** The lean squeezes the wall threefold vertically, so a
      balcony on screen is a ribbon four times wider than it is tall; cut into four it read
      as a scatter of dots. A balcony must read as *bands*, and anything chopping the ribbon
      crossways eats them.
    - **Who gets balconies is a CPU decision** (`layers::balconies_fit`): only `Panel` or
      `Brick`, never a `building=house` (the storey floor catches almost all of them, but a
      five-storey `house` does occur in the extract, and balconies on it would read as a
      parse error, which is what they would be), never under `BALCONY_STOREYS_MIN` 4
      storeys, never on a wall under `BALCONY_COLUMNS_MIN` 3 panels wide, and never on a
      **gable end**. The shader cannot
      decide any of it: it knows neither the use of the building nor how many storeys the
      wall has in total. Note what moved: the *use* filter now mostly lives in the cladding
      pick — plaster, shopfront and shed have no balconies by the meaning of the material,
      and they are exactly what the private sector, the mall and the warehouse get.
    - **A gable end is a direction, not a width** (`layers::plan_long_axis`,
      `WallSpan::gable_end`). «Глухие торцы» stood for a while as `BALCONY_COLUMNS_MIN`, and
      that threshold cannot say it: a real panel section is 12–14 m deep (the measurement is
      in `entrances.md` next to this file), which is four panels — over the threshold, so every
      gable end wore balconies like a facade, and the words «long facade» and «gable end»
      existed nowhere in the code. What the width threshold actually cuts off is a **step in
      the outline** — a three-metre sliver of wall where a row of projections could only read
      as a pattern — and it stays for that.
      The end is told from the facade by the **long axis of the plan**: `roofs::min_area_rect`
      (already computed for hipped roofs and the roof texture, so no second notion of "which
      way this building faces"), its first edge being the long one, and a wall counts as an
      end when its own direction is within 60° of the cosine of that axis
      (`GABLE_END_COS_MAX` 0.5, i.e. no more than 30° off the perpendicular).
      **The aspect-ratio guard is the load-bearing half**: on a plan that is not elongated a
      "short side" means nothing, so `plan_long_axis` returns `None` under
      `GABLE_PLAN_RATIO_MIN` (1.5) and *every* wall keeps its balconies — a tower
      (`heights::TOWER_MAX_RATIO` 1.7 the other way) would otherwise lose half its walls on a
      rounding, while a section (12–14 m by 35+, ratio 2.5 and up) is well clear of it. An
      oblique wall of a non-rectangular plan is likewise left a facade: when in doubt,
      nothing changes.
      The axis is computed **once per building and only when the building could carry
      balconies at all** (`balcony_house` — material, use, storeys), because `min_area_rect`
      is quadratic in the ring's vertices; it rides to each wall in `WallSpan::long_axis`.
      The end is marked `WallMark::Blank`, not `Solid` — it keeps its windows, since a blank
      wall and a windowless one are not the same thing (the bullet below).
    - **The verdict travels as the seed's own value**, not its sign: `WallMark`
      (`WallFrame::marked`) encodes `[0, 1)` balconies, `(-2, -1]` blank, `(-4, -3]` solid.
      Three states are needed because *blank* and *solid* are not the same thing — a blank
      wall (narrow, low, wrong material) still has **windows**, a solid one has none. Three
      surfaces are solid: the **gable**, where a window would be cut by the slope, and the
      two patches of one construction — **under a door** and **around an arch**
      (`WallCells`), where a cell is handed whole to an opening the shader knows nothing
      about. Gable and patch are unrelated: the old sign flip could say only one
      of the two, and needed an idempotency guard on top (the gable marks itself over an
      already-blank wall, and a second negation gave the balconies back); replacing the
      state is the guard.
    - **The seed is per wall**, `seed_from_point(a)` — the generator the roofs, doors and
      parked cars already share — not the building's: the balcony columns of two adjacent
      walls must not start alike. The **cladding**, on the contrary, is per building: one
      house does not have a panel end and a brick front.
    **The wall leaves the fragment before the common roof pass** — `wall_shade` is a branch
    of its own in `fragment`, not a case inside `roof_shade` — and that early exit is the
    whole of what the wall costs: two `stripes` and one hash, against the cheap path code `0`
    used to give it for free. Falling through into the common pass instead would hand every
    wall pixel `roof_age` and two more `fbm3` per frame, and with them a roof's
    fade-and-dirt: that octave is 8 m long and `visible` only kills it past ~5 m/px, so the
    *base colour* of every wall in the city would drift at the city zoom, where the texture
    is supposed to be gone. A wall therefore has **no age** — `roof_age` is the roof's — and
    no grime layer of its own either.
  - **The big box is the cladding whose cell is neither a panel nor a storey**
    (`WallKind::BigBox`, code `14u`). A hypermarket facade is a curtain of composite
    cassettes hung on the frame's columns, so its cell along the wall is
    `layers::BIG_BOX_BAY` **7 m** (against `PANEL_WIDTH` 3.2) and its cell up the wall is
    `BIG_BOX_TIER` **5.5 m** — one trading hall with its technical floor, the same trick
    `SACRED_TIER` plays for a church and for the same reason. With the panel grid a 122 m
    wall of «Магнит» came out as forty columns and a ribbon window in each read as forty
    rows of flats; with the 3 m storey its 8 m shell came out as three floors of windows.
    - What the shader draws between the openings is **two lines and nothing else**
      (`CASSETTE_SEAM` on the bay edge, a horizontal joint every `CASSETTE_COURSE` quarter
      of a tier): a cassette is a large flat panel, and everything else on that facade is
      said by the band above it and the glazing below it.
    - The opening is a **ribbon of curtain glazing high under the parapet**
      (`BIG_BOX_WINDOW_LOW` 0.52 … `HIGH` 0.70, five panes), on
      `BIG_BOX_WINDOW_SHARE` **34 %** of the bays by the same column roll the shed's ribbon
      uses (its own share is higher, `SHED_WINDOW_SHARE` 0.55), and low on purpose: a
      ribbon on every bay is a
      `Shopfront`, i.e. a downtown mall, not a box by the highway. There is no dwelling
      window on this cladding at all.
    - The **plinth is taller** (`BIG_BOX_PLINTH` 0.22 of a tier against `PLINTH_HIGH`
      0.14): under the glazing runs the dark band that hides the loading bays.
    - The **door is the widest opening on the map** (`door_size` 4.2 × 3.4 m): a group of
      glass leaves with a lobby, and you walk in with a trolley.
  - **The brand band is geometry, not shader** (`layers::push_brand_band`,
    `BRAND_BAND_SHARE` 0.30 of a **tier**, its top `BRAND_BAND_GAP` 0.05 of a tier below
    the wall's own top) — one quad
    across every drawn wall of a `BigBox`, with `set_roof(None)` so no texture touches it,
    shaded by the wall's own outward normal through `shade_by_light` so the corner of the
    box does not vanish. **It is what the eye reads a hypermarket by** on every one of the
    reference photos — the coloured frieze running the whole perimeter — and it is the one
    thing that separates a retail box from a warehouse of the same size and shape.
    It cannot be a shader branch: a wall hands the fragment a **brightness and a glass
    amount**, never a colour of its own, and a frieze is a colour. So it arrives the way
    the **door** arrives, and for the same reason — a feature whose place or paint the data
    knows cannot be rolled in the shader. The colour is `building:colour` when the mapper
    set one (ТРЦ «Макси» is tagged `orange`, which is its real colour) and otherwise a
    five-slot chain palette by the building's seed (`BRAND_COLORS`) — red, yellow, green,
    blue, orange, i.e. Магнит / Лента / Леруа / Глобус / Макси; a sixth, magenta, was
    dropped on the first look at the frame, since no chain here is that colour. **This is
    the first place outside the temples that reads `building:colour`**, which is why the
    note under **Tagged colours** in `parse-buildings.md` now names two readers and not one; the
    private sector still does not read them.
    Pushed **after** its own wall and **before** the door, so the frieze lies on the facade
    and the entrance lies on the frieze.
    **Two things about it are measured in somebody else's system, and both were got wrong
    once**, so they are stated here. Its **height is a tier, not a share of the wall**
    (`WallCells::storey`): the glass ribbon it must sit above lives inside a *cell* of the
    shader's grid (`roof.wgsl::BIG_BOX_WINDOW_HIGH` 0.70 of a tier), and a share of the
    whole wall equals a share of a tier only on a one-tier box — on a two-tier one such a
    band lay over the top tier's ribbon completely. And its **normal is the wall's own**
    (`layers::wall_normal`, the helper `wall_colors` takes too): OSM does not normalise a
    ring's winding, so the raw right-hand perpendicular points into the building on half of
    them — written out a second time inside the band, it painted the lit face dark and the
    shaded one light.
    Its wall palette is deliberately quiet (`BIG_BOX_WALL_COLORS`: three near-whites, a
    cool grey and one anthracite) — the band is the colour of the building, and a bright
    cassette under it would fight it.
  - **A cell grid places a feature, it never *is* the feature** (`repair_patch`). The
    bitumen patch started as `hash21(floor(uv / 6))` — a shade of its own for every 6 m
    cell — and that is not repair patches but a **chequerboard across the whole roof**:
    the edge is hard, the grid is aligned to the walls (`uv` is the building frame), and
    ±4 % of brightness on a big dark roof is plainly visible at the working zoom. The
    rule the fix follows — and the one the **asphalt patches** broke a second time, which
    is why they are gone (**Asphalt wear** in `roads.md`): only a minority
    of cells carry the feature (the `share` argument), and
    inside its cell the feature is smaller than the cell and jittered, so two neighbours
    never meet at a cell boundary. (The **wall balconies** above keep only the second half of
    that rule — a balcony is smaller than its cell — and deliberately break the first twice
    over: they are on about half the columns, because a panel block's balconies are a
    majority and a *column* of them is the pattern, not a scattering; and which columns is
    not a draw at all but the **period of a section**, because the pattern there is a step.
    A minority-by-dice is right for a feature that is an accident — a repair patch — and
    wrong for one that is construction.) Placement stays a grid (cheap, no extra octaves); the
    pattern does not.
    The garage bay in `buildings-garages.md` is the deliberate exception — there the seam
    grid *is* the feature, because a row of doors really is one.
  - **Roof age** (`roof.wgsl::roof_age`) — one number per building in [0, 1), **hashed from
    the same seed** the texture phase rides on, and with a fixed patch share that was the
    missing half of the patch fix: a minority of cells carried a patch, but *the same*
    minority on every bitumen roof, so the whole district read as re-roofed and repaired in
    one year. Age drives the patch share (`PATCH_SHARE_NEW` 0.04 → `PATCH_SHARE_OLD` 0.28,
    mixed by `age²` — age is uniform, repairs are not; with ⟨age²⟩ = 1/3 the mean share
    lands at 0.12, half the old fixed 0.22, so a patched roof is an event against clean
    neighbours instead of the district's baseline), the ponding amount (0.07 → 0.13) and,
    on **every roofing** material, the common fade-and-dirt amplitude (×0.75 → ×1.35) — the
    last one is what makes the age read as age rather than as a patch counter. **The wall is
    not one of them**: it returns from `roof_shade` before the age is ever hashed (see the
    wall bullet above), so a wall carries neither an age nor the common pass.
    It gets **no vertex attribute of its own**:
    the seed is already a per-building random number the shader hashes several ways
    (`seed·17`, `seed·11`, `seed·37`), the correlation between two patterns of one building
    is not visible, and a fifth float would cost four bytes on every vertex of the building
    layer. The consequence for the gallery: its `Seed` knob rolls the age too — that is how
    a new roof is compared against an old one there.
  - **There is no parapet, and putting one back needs a different construction.** A soft
    flat roof (bitumen / gravel / membrane, the old `RoofKind::has_parapet`) used to get a
    0.7 m inset band along its ring and every courtyard ring, lit by `shade_by_light` like
    a wall (0.24 / 0.20) — bright on the sunny edges, dark on the shaded ones. That is
    **the same construction as a hip roof's slopes** (an inset band on miter offsets,
    shaded by the edge's own outward normal) at a third of the width, so from the air every
    panel block wore a small hip, and the hip of a large private house — inset capped at
    `HIP_INSET` 2.2 m — was the same picture only wider. Two things a roof shape must never
    do, and it did both. The band's other cost is why the retreat is not a one-liner back:
    it wants to read as a vertical coping standing *above* the roof, and an inset band
    shaded by a plan normal cannot say that. `MeshBuilder::push_inset_band_with` (the
    per-edge-colour sibling of `push_inset_band`) survives as a primitive; nothing in the
    building layers uses it any more.
  - **One call lays every flat roof** — `layers.rs::push_flat_roof(builder, look, outer,
    holes, color)`: set the roof frame, fill the contour with its courtyards. Both callers
    use it — the flat modes with the real contour, 2.5D with the contour already lifted
    onto the walls. It is `pub(super)` now: the gallery reaches the same geometry through
    `push_house`, one level up. Slopes stay outside it: a gable
    roof is computed by the caller, which needs the same `GableRoof` for the gables it
    draws *with the walls*, before the roof.
  - **A roof is now darker than the walls under it.** That inverts the old "roof lighter
    than wall, so the wall reads as a band under it" rule, which is retired: on a photo a
    dark bitumen roof over light panel walls is the normal relation, and the 2.5D box is
    held together by the two visible walls' own tones. The relation is "as a rule": the
    light materials (membrane, gravel, silver tile) sit above their walls, as they do on
    the photo. The height ramp (`ShadowsTint`) keeps its direction only while
    `ROOF_TALL_COLOR` is darker than every base — a mid-value neutral would *lighten* a
    saturated red or green tile.
  - **`RoofStyle::texture`** (section Buildings, row `Roof texture`, persisted, BRP) is
    the amplitude of all of it; 0 leaves flat material colours. It rewrites the uniform
    only, so dragging the slider rebuilds nothing.
  - **The gallery** — `cargo run --example roof_gallery` (`examples/demos/roof_gallery/`),
    materials and palettes in one grid, every roof *shape* over a fixed set of outlines in
    the other. The rules it lives by:
    - every house goes through **`push_house`**, the per-building body of the extrusion
      layer, and every shape number comes from `roofs::shape_facts` — the gallery may not
      roll its own geometry or restate a threshold;
    - the tuning constants it prints are **parsed out of `roof.wgsl` and `roofs.rs` by
      `constants.rs`** (`include_str!`, lines of the form `const NAME: f32 = …;`) rather
      than mirrored as Rust numbers: a mirror would drift on the first edit and the gallery
      would then lie about exactly what it is opened for. They are text, not knobs: the
      numbers live in the code.
    - **`roof.wgsl` holds two textures, and the two galleries split it by the section
      banner** (`─── стена`): the roof gallery parses everything **before** it, the wall
      gallery everything **after**. A name list would have to be extended on every new
      constant; the banner sits exactly where the meaning changes. Both halves are pinned by
      a test that the other half's constants did *not* come through — an empty or
      over-full group is a parser drift, not a fact about the code.
    - `ROOF_GALLERY_SHOT=path.png` takes one frame and exits. The example has no BRP, and a
      screen grab over another window comes out black, so this is the only way a session
      without the window in front of it can look at its own work.
  - **The wall gallery** — `cargo run --example wall_gallery`
    (`examples/demos/wall_gallery/`): claddings down one side, `BuildingUse`s down the
    other, the cladding chosen by the game's own `wall_of`. Its readout prints the **storey
    in pixels**, not only metres per pixel: a wall is measured in cells and every fade is
    keyed to them, and a storey is `STOREY_HEIGHT` metres already squeezed by the lean — so
    metres per pixel alone does not say when a window is due to vanish. It comes from
    `extrusion_lift` on a probe building rather than from a copy of `EXTRUDE_SCALE`, which
    is private and would drift. `WALL_GALLERY_SHOT=path.png` takes one frame and exits,
    same as the roof gallery's.
