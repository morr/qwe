# Look: the silhouette atlas, blood and the portal

The drawing half of a pawn: `silhouette/` and `portal.rs`. `CONTEXT.md`'s **Look** section
owns the terms and the invariants — the atlas shape, the `custom_size` ownership, the
no-renderer `None` that is not an error, the per-species tints (`Attire`, `demon_tint`,
`corpse_tint`, the halo, the soul); this file is the mechanism and the measurements behind
them.

## The atlas

**One image for all `Glyph::COUNT` glyphs** — `CELL_PX` (128) per cell, one row, so
27 × 128 today — built once per process by `SilhouettePlugin` (`main.rs`) in `Startup`
(`build_atlas`): glyphs depend on neither the city nor the run, so neither a restart nor
a city switch rebuilds it, and the `Handle`s outlive every world. It logs a
`silhouette atlas: …` line with its own size and timing.

**The number of glyphs costs nothing per frame** — which is why the blood families are
ten variants each and not four. A cell is rasterised once at startup, a pawn picks its
glyph once at spawn, and the batch is still one texture; what grows is the build time and
the texture (a cell is 128 × 128 × 4 B ≈ 64 KB on mip 0, ~87 KB with its mips).

One image and not one per species **because sprites batch by texture**: humans, corpses
and demons interleave in y-sorted z, so a second texture would cut the batch at every
demon.

- **Cell 128 px, not 64** — at the closest zoom (0.05 m/px) a corpse cell is ~75 logical
  px, 150 physical on a retina display; at 64 the figure's limbs blur.
- **Signed distance with a one-pixel edge** (`EDGE_PX`): `texel()` returns *(shade,
  alpha)* per texel, where shade is a multiplier on the sprite colour — so one glyph
  serves every tint, and a pawn's colour never touches the texture.
- **Transparent texels carry the edge colour, not black**, or linear filtering would mix
  a black rim into the outline.
- **The mip chain is computed here**, 2×2 averaging down to 1×1 (`downsample`), and the
  image is sampled **linearly** even though the app runs on
  `ImagePlugin::default_nearest()`: a silhouette is a smooth figure, not pixel art, and a
  2–4 px dot off a 128 px texture sparkles at every step of a pawn without mips.
- **`Glyph`** — three lone shapes: `Disc` (human), `Ember` (demon: seven spikes, a bright
  core), `Halo` (the demon halo, the soul spark, the portal stain — any glow); and three
  **families that carry a variant number** — `Pool(i)`, `Spatter(i)` (blood) and
  `Corpse(i)` (the four poses). A family's cells sit **consecutively in atlas order**, and
  the cell index is **`Glyph::cell()`**, not the discriminant — that is what lets a family
  grow from four variants to ten without twenty new names. Take a variant through
  `Glyph::pool` / `spatter` / `corpse`, which reduce mod the family size; `Glyph::all()`
  walks the atlas in cell order, `Glyph::COUNT` is its length.
  `Glyph::pool_anchor` gives the chest of a pose in cell coordinates (−1…1), the
  centre for every other glyph. `Silhouettes::sprite(glyph, color, size)` is the only way
  to make a pawn sprite; `set_glyph` swaps a live sprite onto another glyph of the same
  atlas (a human becoming a corpse).
- **`Capsule`** (`silhouette/mod.rs`) — the shared drawing primitive: two circles and
  their hull, signed distance plus a bounding circle for culling. Both the corpse
  skeleton and every blood lobe and drop are built from it.
- **The corpse figure** (`silhouette/figure.rs`) — head, torso, arms and legs as capsules
  on a skeleton given **in heights** (1 = standing height), joints as angles off the body
  axis: head towards +x, feet towards −x, the left side +y. Limbs are drawn 1.5–2× thicker
  than anatomy on purpose — an honest 0.05-height arm is a fraction of a pixel at crowd
  zoom, and the body has to read as a *pose*, not as wire. All four poses share one square
  `CELL_SPAN` (1.4 heights) cell, because the sprite rotates to any angle and a rectangular
  cell would win nothing.
- **Look at it with your eyes**: `SILHOUETTE_DUMP=/tmp/atlas.png cargo test dump_atlas --
  --ignored` writes mip 0 to a PNG (transparency reads better on a grey background).

## Corpses and tints

The corpse is one of the four corpse glyphs of the silhouette atlas (`silhouette/figure.rs`),
in one of `CORPSE_HEADINGS` (16) directions, mirrored or not, all chosen from the `Entity`
bits through a splitmix hash (`human/look.rs::corpse_pose`). The tint is the human's own
`Attire` drained (`corpse_tint`: half the saturation, half the lightness), written by
`lay_down`, an entity command queued *after* the tag removal so the calm-down observer
restoring the attire cannot overwrite it. The body's own `Silhouette` is rewritten to the
`CORPSE_SPAN` cell (`HUMAN_MIN_PX` floor), which is how the size lands on the sprite next
frame; under the chest hang the **two blood children** of the next section.

**Panic tint** (`human/look.rs`) — details beyond `CONTEXT.md`'s **Attire** entry: the
remove observer also fires on the corpse transition (harmless: `to_corpse` paints the body
afterwards) and on despawn. `Attire` itself is the three spawn draws of the decision
stream, now from a cool muted palette — the draw *count* is what must not change, since
`Pace` and the heading follow in the same stream.

The demon's look (`demon/look.rs`): y-sorting draws a neighbouring human *over* the halo;
`spawn_demon` gets the atlas through `DemonBirth`.

## Blood

`silhouette/blood.rs` draws it, `human/look.rs` spawns and grows it.

- **`BloodPool`** — the stain that *runs out from under the body*: a small core plus 5–8
  discs set on an **ellipse** of its own stretch and tilt (`POOL_STRETCH` up to 1.35 —
  blood runs downhill, and a round stain is a blot, not a pool; on the eye the stretch
  separates the variants more than anything else), all welded by a smooth minimum
  (`POOL_WELD` 0.14 — a hard union reads as a heap of pancakes), a crenellated rim (three
  harmonics, 0.028–0.010 of the half-cell) and 2–4 tapering **rivulets** with a sideways
  sway at the knee. A rivulet is given by **where it ends** (`POOL_RIVULET_REACH`
  0.58–0.78), not by its length: given a length it drowned inside the pool, whose lobes
  already reach 0.5–0.8, and only a two-pixel stub stuck out. Cell `POOL_RATIO` 0.75 of
  `CORPSE_SPAN`, z −0.02.
- **`BloodSpatter`** — the cast-off fan, thrown at one axis: 34–56 drops whose distance is
  biased towards the body (`u^1.6`), the fan widening and the drops shrinking with
  distance, one in seven fat (`SPATTER_FAT`), each drawn out into a **comma whose narrow
  tail points along the flight**, plus 3–7 thrown back and 2–5 fat satellites at the
  pool's edge. Cell `SPATTER_RATIO` 1.35 of `CORPSE_SPAN`, z −0.03 (under the pool: the
  drops fell first).

  **The tail is measured in the drop's own radii**, and that is the one number this fan
  lives or dies by: given in cell units it made a small far drop a needle ten calibres
  long, and the whole spatter read as rake marks. Sixteen big drops read as spilt grain,
  too — many small ones are what reads as blood.

**Why not one sprite.** A single glyph would have to either grow its drops with the pool
(drops do not grow) or give up the growth, and its cell would have to cover the fan's
reach, costing the pool half its atlas resolution. One extra entity per corpse buys both.

**Thickness is the whole colour model.** `Stain::texel` turns *how deep inside the
contour* a texel sits into `depth` — **by the square root**, full at `DEPTH` 0.032 of the
half-cell — and from it into **alpha** (`FILM` 0.70 at the rim → 1.0 deep) and **shade**
(`CORE_SHADE` 0.55 deep → 1.0 at the rim). The root and the small `DEPTH` are what keep
that from going too far — with a linear ramp over 0.075 a drop of two hundredths never
reached half depth and the whole fan came out pink.

Two traps, both paid for once:

- **`shade` is read as sRGB** (the atlas is `Rgba8UnormSrgb`), so 0.55 is ≈0.26 of the
  light — a value picked as a linear fraction comes out near black.
- **transparency belongs to the glyph, not to the sprite colour.** `BLOOD_ALPHA` is 1.0
  and the `BLOOD_HUE` / `BLOOD_SATURATION` / `BLOOD_LIGHTNESS` triple's job is only to say
  *what colour thin blood is*. A translucent colour
  was tried first: on the pavement (linear 0.74 against blood's 0.3) even an eighth of the
  ground bleaching through turned the pool brown.

**Per-corpse variety** (`human/look.rs::blood_look`) — the pool glyph, the spatter glyph,
a spin for each (`BLOOD_SPINS` 64 steps, on top of the body's own rotation), a size
(0.80–1.20) and a tint (hue +0–12°, lightness ×0.78–1.14), all sliced out of
`splitmix64(entity ^ BLOOD_SALT)` at non-overlapping bit offsets. The salt is the point:
hashed with the pose's own bits, pose and pool would walk in step and four pairs would
become visible long before forty.

**Growth** (`spread_blood`, `Update`, `SimPipeline::BothModes`) — the pool opens ease-out
by writing `Silhouette::body` (never `custom_size` — that belongs to `silhouette`).
Ordered `.before(size_fresh_silhouettes)` so the new size reaches the sprite in the same
frame.

**Look at it with your eyes**: `cargo run --example blood_gallery` — every pool and every
spatter side by side, then a field of bodies built by the real `to_corpse`, on three
grounds. `BLOOD_GALLERY_SHOT=<path>` makes it screenshot itself and quit.

## The screen-size floor and the two LOD systems

`Silhouette { body, min_px }` carries the body in metres plus the floor in logical px on
the body's *smaller* side. `drawn_size(zoom)` (zoom = metres per logical px, i.e. the
camera `Transform`'s scale) stretches the body proportionally until its smaller side
reaches `zoom * min_px`, and never shrinks it below the body. Without the floor a 1 m human
is 0.2 px at city zoom and disappears; `SOUL_MIN_PX` 3 keeps a 1.4 s spark visible.

**`Sprite::custom_size` belongs to this module**, and to three writers inside it:
`Silhouettes::sprite` sets the body at spawn, and from then on the size is owned by two
chained `Update` systems. Nothing outside `silhouette/` writes the `custom_size` of an
entity that carries a `Silhouette` — a look system that wants a different size rewrites
the component, not the sprite. The two:

- **`size_fresh_silhouettes`** — every frame, but `Changed<Silhouette>` only. A spawn or a
  corpse rewrite gets its size in the frame it appears, instead of waiting for the first
  wheel click.
- **`resize_silhouettes_on_zoom`** — only when the zoom actually changed (`Local<f32>`
  holds the last one), and even then **a window, not the whole set**: `RESIZE_BATCH`
  (2048) silhouettes per zoom-change frame, cursor held in a `Local`, and inside the
  window a write happens only when the size moved more than `RESIZE_TOLERANCE_PX`
  (half a logical pixel) — a write of the same value still marks the sprite changed and
  drags it through bounds, extract and the render phases, and at the near zoom (floor
  inactive, size = body) every wheel click used to pay 20 000 such no-op writes
  (measured 10.2026: they were not the zoom-jerk driver — the frame jitter of the same
  magnitude happens without them — but they were pure waste, and the full 20 k rewrite
  did land on one frame). The whole set catches up within `total / RESIZE_BATCH`
  zoom-change frames; the tolerance holds a silhouette within half a pixel of the exact
  size at the moment of its check — one whose turn has not come in this lap stands at
  its last write and can sit past the tolerance until it.

Both ask for the **user's** camera — `With<Camera2d>` *and* `With<PanCamera>`, because a
`dev::OffscreenShotEvent` puts a second `Camera2d` in the world for three frames and a
`Single` matching two entities is skipped without a word. Both return early when there is
no such camera, which is the same contract as the atlas:
without a renderer (replay, tests) `Silhouettes` stays `None`, `sprite()` hands back a
plain square `Sprite`, and **that is not an error**. `HumanPlugin` / `DemonPlugin` /
`PortalPlugin` only `init_resource::<Silhouettes>()`; `SilhouettePlugin` is what fills it,
so a test yard that spawns pawns or runs the kill observer must `init_resource` it too.

## Portal

`portal.rs` + `assets/shaders/portal.wgsl` — a `Mesh2d` quad with `PortalMaterial`
(`Material2d`): a three-armed log-spiral vortex with fbm turbulence computed per pixel
from `globals.time`. No spritesheet, no frames, no `Update` system — the nine-frame 160 px
sheet it replaced read as pixel mush stretched over 36 m, and nobody is going to repaint
it. The Rust `PortalUniform`'s layout must stay in step with `PortalUniform` in the WGSL.
The rim is **HDR** (`PORTAL_RIM`, > 1.0) so the camera's bloom catches it; `PortalPos`
itself — the hint, the snap to a passable tile — is `navigation-deep`.
