---
name: ui-panels
description: Use when working on qwe's UI — the tabbed settings panel (Map / Nav / Sim / Debug) and its shell, the HUD (counters, telemetry, speed, city, hotkeys), the knob / slider / value-row kits, the feathers theme and its translucent plaques, the camera start view, prefs persistence. Deep detail behind CONTEXT.md's UI summary.
---

# UI & debug panels — deep detail

Detail behind `src/ui/` and `camera.rs`; the panel inventory lives in `CONTEXT.md`, and
the "UI input must not reach the game world" rule in `CLAUDE.md` is non-negotiable.
**Changing that mechanism is changing this file, in the same change** — a new kit, a
moved row, a retired widget. The term still goes to `CONTEXT.md`; the detail lives only
here.

## References

- [`references/panels.md`](references/panels.md) — the per-tab bodies: every section's
  rows, ranges and wiring for **Map**, **Nav** (the one UI for both pathfinding
  backends), **Sim** and **Debug**.

## Layout

Two layers over the map: a **HUD** that is always on screen, and **one settings
panel** with four tabs (`ui/shell.rs`).

- **`ui/shell.rs`** — the shell. `SettingsTab { Map, Nav, Sim, Debug }`: **declaration
  order is strip order**; the open tab and the collapsed flag are a persisted settings
  group (`UiShellState`). It collapses to the bare tab strip three ways:
  the `-`/`+` button, a click on the open tab (`on_tab_click`), and **`Tab`** — free
  because `TabNavigationPlugin` is deliberately not installed (see the theme
  note below). The left column is `top`/`bottom`-anchored (a ceiling for the body to
  scroll against) and carries **`Pickable::IGNORE`**: it is taller than its content, and
  an invisible 260 px strip eating the map's clicks is what the UI-input
  rule forbids. The component is per-entity and does not propagate, so a
  subtree that must be transparent carries it on **every** node (the hotkey help does:
  plaque, rows and both labels). Wheel over the panel scrolls the body
  (`Overflow::scroll_y` + a `Scroll` entity event ported from bevy's
  `scroll_and_overflow` example; `clamp_scroll` is the tested part).
- **Sections.** A panel asks for `spawn_section` (a column with a header band) or
  `spawn_block` (no header — the Nav tab, whose headers *are* rows). **`SectionSlot`'s
  declaration order is the order inside a tab** (`sort_sections`, one pass at the end
  of `Startup`): the sections are spawned by one system per section plugin, and system
  order inside `UiBuildSet::Sections` is unspecified. A section that arrives
  **without** a `SectionSlot` sorts **last** and is named in a `warn!`: the enum exists
  so a forgotten slot cannot pass silently — a slotless section sorted to the top
  would look like a mysteriously first panel body.
- **Bloom** (`post.rs::camera_post_process`) — the bundle `camera.rs` adds: `Hdr`,
  `Tonemapping::Linear`, `DebandDither::Disabled` and `Bloom::NATURAL` at
  `BLOOM_INTENSITY` 0.25 — 0.4 reads as haze now that every demon carries a halo and
  every death a spark — `Additive` composite. Only what is brighter than 1.0 draws itself
  so: the portal vortex and rim (`portal.rs`), the demon halo (~1.4 after alpha,
  `demon/look.rs`), the soul spark (`human/soul.rs`) — nothing on the map is. The
  prefilter is `BLOOM_THRESHOLD` 1.1 / `BLOOM_THRESHOLD_SOFTNESS` 0.1 — the knee runs
  `threshold × softness` both ways, so it starts at 0.99 and pure white lane markings
  (1.0) contribute nothing; a 1.0 threshold made everything above 0.6 glow and hazed the
  whole map. Tonemapping stays off on purpose: every built-in curve recolours the map
  palette, and only the halo is wanted. That is `Tonemapping::Linear`, not `None`: since
  Bevy 0.20 `None` is a full passthrough that also drops the negative-channel clamp an
  additive bloom wants — `Linear` is what `None` used to be. UI is drawn after
  post-processing, so panels never bloom.
- **Antialias** (`post.rs::Antialias`, settings group `render`, default on) —
  `Msaa::Sample4` on the user's camera and on the offscreen-shot camera (`dev.rs`);
  `post::apply_antialias` (`retuned`) writes every
  `Camera2d` on a toggle. Default on because without it the map's ribbons — asphalt,
  kerbs, median noses, islands — stair-step on a close-up, while the paint beside them
  is smoothed by `paint.wesl` itself. HDR + bloom + the four `Material2d`s work with it
  unchanged. **Cost is memory, not frame time**: on a 4K window the multisampled HDR
  target and depth add ~400 MB; the frame was measured in the `roads` gallery
  offscreen at 7500×4750 (4× a 4K window) — medians within a millisecond.
  Hence the Debug tab's toggle (`4x` / `Off`); the galleries and
  `crowd_demo` run `Sample4` hard-wired.
- **Expensive-knob debounce** — the one place a panel is decoupled from what the map
  follows. **Sun** (`ui/sun.rs`) and the Roads shape sliders write their knob resource
  like any other kit row, but the map follows a **settled copy**: `map::sun::settle_sun`
  copies `SunStyle` into `SunOnMap`, `settle_road_shape` copies `RoadShape` into
  `RoadShapeOnMap`, each after **0.35 s of quiet** — a rebuild per drag step would cost
  hundreds of ms. The debounce sits between the setting and the map, not inside the
  panel, precisely so a section stays a section. Ranges and per-knob rules —
  `references/panels.md`.
- **Vignette** (`post.rs::spawn_vignette`) — a full-screen `Node` with a radial
  `BackgroundGradient` (transparent to 55% of the far-corner radius, black at
  `VIGNETTE_ALPHA` 0.22 in the corners). `GlobalZIndex(-1)` keeps it under every panel;
  `Pickable::IGNORE` keeps it out of `HoverMap` — without that it would be "a UI node
  under the cursor" everywhere and `pointer_over_ui` would hand the whole map to the UI.
  Not a `GameUiRoot`: it survives the city switch.
  The opaque loader root (z 0) covers it while `Loading` — the vignette frames the map,
  not the loader. Spawned once at `Startup`.

## HUD

- **Counters** (`ui/stats.rs`) — Pawns (`With<Human>`, i.e. alive: the component is
  stripped on death), Demons, Souls reaped (`Telemetry::killed`), first in the left
  column, **outside** the tabs: they are watched continuously — behind a tab choice
  they would be a keyhole on the simulation. The counters use
  `iter().len()`, not `count()`: with a purely archetypal filter the iterator is
  `ExactSizeIterator` — a sum over archetypes rather than a walk over
  20 000 entities every frame. In agent runs the red **BRP badge** (`ui/brp.rs`) owns
  that corner, and `offset_below_brp_badge` measures it and pushes every `BelowBrpBadge`
  node below — `ComputedNode::size` is in *physical* px, so multiply by
  `inverse_scale_factor` or the offset doubles on a retina screen. The game marks an
  agent run by `BRP_PORT` (`main.rs`); a demo has no port, so its `AgentBadgePlugin`
  also takes `CLAUDECODE`, which Claude Code sets on every process of its session.
- **Telemetry** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight / avg ms,
  entity count. Fixed width + right-padded digits (no jitter) only work in a
  monospace face, so this root brings its **own** `InheritableFont` (FiraMono);
  `apply_panel_font` therefore skips roots that already have one
  (`Without<InheritableFont>`). The panel's fixed columns are one half of the ruler
  `tools/shot_coords` works from — changing its geometry, font or any of these format
  strings means re-measuring the constants at the top of that script in the same change.
- **Point inspector** (`ui/inspector.rs`) — directly **below the telemetry**
  (`position_below_telemetry`, the `offset_below_brp_badge` idiom; in the roads
  gallery there is no telemetry, `Single` skips the system and the panel stands at the
  screen edge). Two FiraMono lines: the **camera line** `zoom/x/y cx/cy` and the **hits list** — what the map draws
  under the cursor, recomputed when the point moves over `INSPECTOR_QUERY_STEP` (0.5 m)
  or the semantic index is rebuilt. Each hit prints `z  label`; the label dictionary
  (**this file is its single owner**; `osm-map` points here): data features speak for
  themselves (`road residential 8.0 m, sidewalks`), a band the road drew itself carries
  the `(auto, …)` suffix — `sidewalk (auto, tagged)` (the sidewalk came from a
  `sidewalk=*` tag) / `sidewalk (auto, inferred)` (the parse inferred it) / `verge
  (auto)` (the verge to a separately mapped footway; no reason shown) — and a hit that
  came out of the extract ends with its OSM id, `#179102449` (the tags are one
  `tools/osm_near --id <id>` away). **A kerb parking pocket answers as `parking pocket
  (auto, tagged)`** (`parking:<side>` tag) or `parking pocket (auto)` (the sparse rule)
  — the pocket exists only as drawn geometry, so the index stores its polygon itself and
  rebuilds with the road shape. The two knobs the list follows: `TreeStyle::density`
  (the tree prefix) and `RoadStyle::sidewalks` (turning it off removes both auto bands,
  as it does from the drawing). **A cursor over any UI panel stops being a map point**
  (`update_inspector_hits` and `update_camera_text` gate on
  `camera.rs::pointer_over_ui`): the camera line reads `-/-`, the hits
  clear, the highlight despawns. The list is **one node per hit** (`sync_hits_rows`; ≤
  `INSPECTOR_MAX_LINES` rows + a `+N more` tail; empty — a bare `-`; no `ground` row —
  bare ground is not a feature). **`]`/`[` cycle the selection** — `]` next, `[`
  previous, both wrapping (any hit recompute resets it
  to the top) — the **selected** hit's row carries a translucent-yellow
  `BackgroundColor` — the same element whose shape the **highlight checkbox** draws on
  the map (`InspectorHighlight`, a resource the checkbox's `ValueChange` observer writes
  and `sync_highlight_checked` mirrors back onto `Checked`, so a BRP write moves the
  widget too; **on from launch** — `Default` is `true`, in the game as in the gallery,
  where the checkbox sits at the screen edge outside every sample viewport and cannot be
  clicked; the layer itself is `sync_inspector_highlight`, the `respawn_overlay` pattern
  at `Z_INSPECTOR_HIGHLIGHT`, rebuilt only when the selection changes or the gallery's
  `InspectOrigin`/`InspectViewport` move). **A highlight in a user screenshot is the
  user's pointer, not the subject of the question**: a shot with a translucent yellow
  shape on it means the feature was selected on purpose — identify it by its row in the
  hits list (`z … label #id`) and investigate **that feature** (why it is there, why it
  looks the way it does); the highlight overlay itself needs no explanation, and since
  it draws above every map layer it also shows outlines the map itself hides under
  roads and paint. `tools/shot_coords` OCRs the hits list too — the `z <depth> …` line
  shapes in `hit_line`/`format_hits` and the `HIGHLIGHT_ROW_BG` tint are what that
  reader keys on, so changing either means re-checking the tool in the same change.
  **The layer is placed at `map +
  InspectOrigin`** (`LayerMesh::offset`) — the inverse of the panel's own world→map
  subtraction — **and clipped to the viewport** (`InspectViewport::clip`): it never
  spills out of a sample window, even when the object is bigger. In the roads gallery
  the one-shot batch shift `place_new` therefore skips the layer by the public
  `InspectorHighlightMarker` (every other mesh it catches must be shifted; this one
  carries its shift already). **`InspectorHighlight` is the documented exception to
  "every UI resource is a `SettingsGroup`"**: a session-only debug aid, deliberately not
  persisted — do not "fix" it by adding a group or a `track_pref`. Two modes:
  `UiInspectorPlugin::game()` (a `GameUiRoot` — hidden until `PlayPhase::Live`, hidden
  on city switch) and `UiInspectorPlugin::standalone()` (the roads gallery: no states
  there, and the panel is visible only while the cursor is inside a sample
  viewport — each sample is its own viewport with its own map metres). The per-frame
  wiring is the gallery's (`examples/demos/roads`): `sync_hover_viewport` finds the
  viewport under the cursor and writes `InspectOrigin` (its shift, else the framed
  sample's — `brp probe` speaks the same metres) and `InspectViewport` (`Inside` the
  window / `Outside` between windows / `Whole` is the game's "no windows");
  `sync_current_map` re-parses the hovered sample's `MapData` (~40 ms per crossing) so
  the semantic index answers for what is under the cursor; `sync_inspector_visibility`
  hides the panel outside. `tools/shot_coords`
  reads the camera line; its geometry constants belong to **this** panel now.
- **Speed button** (`ui/speed.rs`) — left of the telemetry, a `Speed <value>` row-button.
  Left click walks the ladder up and wraps to 1x from its top step (`MAX_SIM_SPEED`),
  right click steps down; `Primary` while paused. It reads `Pointer<Click>` itself
  instead of `Activate` — `Activate` fires for *any* mouse button, so one right click
  would move both ways.
- **City select** (`ui/city.rs`) — one `FeathersMenu` (button + popup + item per
  city), not a button per city: exactly one is ever chosen — that
  is a select, and a row of buttons grew with each new city. The popup flips itself
  above the button when there is no room below. `sync_city_label` keeps the caption on
  the `City` resource, which `reset` and BRP also write. The select and the label sync
  are re-exported from `qwe::ui`: the roads gallery puts the same select on its own
  panel and registers the label sync itself.

## Shared kits & layout

- **Widgets and theme** (`ui/theme.rs`) — the controls are first-party `bevy_feathers`;
  the theme is `create_dark_theme()` with **colour overrides only**.
  **`PanelWidgetsPlugin`** installs `FeathersCorePlugin` plus `UiTheme` — a plugin of
  its own because the crowd demo calls the same kits and cannot bring up `UiPlugin`.
  Six decisions, not detail:
  - **`FeathersCorePlugin`, never the `FeathersPlugins` group.** The group also adds
    `TabNavigationPlugin`, and with it Tab focuses any panel widget (every control
    carries `TabIndex(0)`), after which Space both "presses" the focused button *and*
    pauses the simulation — the "UI input must not reach the game world" rule, and
    `typing_in_text_input` does not catch it because the focus is not on a text field.
    Without the plugin `TabIndex` / `FocusIndicator` simply do nothing.
  - **Everything that is not a widget is coloured by the same tokens**: backings
    `panel_background()` / `panel_block_background()`, labels `row_label` (`TEXT_DIM`)
    and `row_value` (`TEXT_MAIN`), titles `PANE_HEADER_TEXT`.
  - **The plaques are translucent; the text carries the legibility.** One near-black
    `UI_COLOR` at three alphas — `PANEL_ALPHA` .72 (pane body), .85 (header strip),
    .50 (nested block) — so the city stays visible under the panel; `TEXT_MAIN` is
    white and `TEXT_DIM` is oklch L .90 (`MUTED_TEXT`), both well above feathers' own,
    which assume an opaque plaque and over a bright map read as grey on
    grey. Raising the alpha would be the wrong knob: the panel lies *on* the map, not
    instead of it.
  - **A value row rests transparent, a button goes blue.** Rows are
    `ButtonVariant::Plain` (`BUTTON_PLAIN_BG` = `Color::NONE`): a panel is ten rows in
    a stack, and `Normal` gave each its own grey brick. A row says its state in
    **words** on the right (`On`/`Off`, `Round`, `Polymesh`); `Primary`
    (`button_variant(is_active)`) is for buttons that stand alone and say not their own
    value but "where am I" / "what state is the world in" — the open tab in the strip,
    pause on the speed button. Not the Debug tab's toggles (value rows printing
    `On`/`Off`), not the city (a `FeathersMenu` popup, no variant at all). Feathers
    does hover, press and disabled itself; panels never paint backgrounds.
  - **The font is feathers' 14 px** (`PANEL_FONT` = `size::MEDIUM_FONT`). One
    `apply_panel_font` system puts `InheritableFont` on every `GameUiRoot` except roots
    that brought their own (`Without<InheritableFont>` — the monospace telemetry).
    The kits patch the same size onto the widgets (which carry an `InheritableFont` of
    their own and would otherwise win).
  - **A container *between a font source and a label* comes from `ui_node` / `ui_row` /
    `ui_column`, never from a bare `Node`.** Propagation
    (`HierarchyPropagatePlugin::<TextFont, With<ThemedText>>`) *stops* at the first
    entity without `ThemedText` — a bare-`Node` wrapper between a root and its labels
    leaves everything below at the default 20 px. **The source itself is not such a
    container**: `InheritableFont` is
    `#[require(ThemedText, PropagateOver::<TextFont>)]`, so a root that brings its own
    font or is handed one by `apply_panel_font` carries the marker already and *starts*
    the chain instead of standing in it — which is why the roots of `ui/hotkeys.rs`,
    `ui/speed.rs` and `ui/city.rs` are bare `Node`s and are right to be. Exceptions: a
    `Node` patched onto a widget's own `bsn!` scene (`ui/rows.rs`, `ui/city.rs`,
    `ui/slider.rs` — the slider adds a bare `ThemedText` because `FeathersSlider` does
    not), and a `Node` bundled with the very text it lays out (the `flex_grow: 1.`
    spacers). `warn_broken_font_chain` (debug builds, after `apply_panel_font`) is the
    arbiter: it names the offending node in the log and stays silent in exactly the
    cases above.
  `spawn_panel_button` is the button kit; `spawn_panel_button_with` takes the caption
  as a bundle instead of a string, for the two-text cycler rows — the caption is a
  child rather than the scene's `@caption` because it comes from runtime strings.
- **Knob kit** (`ui/knob.rs`) — what panels actually call. A knob is a slider row bound
  to one field of one resource: `spawn_knob` with a `SliderBinding<R> { get, set,
  range, text }`, and `app.add_knobs::<R>()` registers the
  drag observer and the sync **once per resource** — not per knob, per panel or per
  field. Two binding shapes worth knowing: an integer knob casts in `get`/`set` and
  takes an integer step; a percent knob keeps the
  resource in 0..1 while `text` multiplies by 100. `add_knobs::<R>()` is
  **idempotent** (memoised `TypeId`s in `RegisteredKnobs`): one resource
  is knobbed from two panels at once — `HumanStyle` is `Speed spread` in Human *and*
  `Body radius` in Navigation — and each panel must register what it uses rather than
  assume its neighbour did. **The module is `pub`; the demo galleries call it**
  (`examples/demos/*`): each keeps a `ParamSpec { label, group, binding }` table of its
  own and registers `add_knobs::<Tuning>()` beside `init_resource`. A gallery still
  owns its **reset** button, which the kit has no notion of, and `tree_gallery`
  compares it knob by knob through `binding.get` rather than `!=` on the whole
  `Tuning` — `CrownParams` is not `PartialEq`.
- **Slider kit** (`ui/slider.rs`) — the layer under the knob kit, and the one
  `crowd_demo` calls directly, since its fifteen knobs interleave three resources in
  one `const` slice that also feeds the presets. Free functions only:
  `spawn_slider_row`, `quantize`, `apply_step`
  (quantize + thumb back on the stepped value), `retarget` (move the thumb
  when the value arrived past it — over BRP, from saved settings, from a lab
  preset); no systems, nothing registered by `UiPlugin`. What keeps a panel's thumb on
  its value is `sync_knob_values::<R>` in `ui/knob.rs`, registered by
  `add_knobs::<R>()`; a caller that drives demo-local state instead of a resource
  writes that loop itself.
- **Cycle rows** — the button half of the knob kit, for the values `bevy_ui` has no
  input field for: `spawn_cycle_row(.., CycleBinding { cycle, text })` where `cycle`
  advances the field by itself. Deliberately not "next item of `ALL`": what cycles is
  enums, plain bools, colour palettes and step tables alike, and forcing them through
  one list would need a type per kind. Registered by the same `add_knobs::<R>()` as
  the sliders. `CycleBinding` is *not* a component — the click observer captures it,
  and the sync only needs it on the label. The rows that stay hand-written are the Nav
  tab's (`NavValueLabel`): their text is computed from **several** resources at once
  (`Backend` from `PolymeshDebug`, `Separation` from determinism + backend + style) —
  a binding to one resource cannot express that.
- **Value-row kit** (`ui/rows.rs`) — the layer under cycle rows, and what the
  Navigation panel calls directly (`spawn_value_row`, `row_color`, `next_in`,
  `on_off`). A row is a `FeathersButton` with its own scene rather than
  `spawn_panel_button_with`'s: it spans the panel, its left padding carries the
  nesting indent, its label stretches to push the value right. A row whose click
  currently does nothing gets first-party **`bevy::ui::InteractionDisabled`** from its
  panel, which both stops the highlight and swallows `Activate` — promising a reaction
  the click will not deliver is worse than not highlighting. There is no
  dimmed-disabled colour of our own: feathers simply stops repainting the row, so it
  keeps its *resting* colour — an inert row should look ordinary and merely not react.
- **Printing text**: a value that prints with a sign keeps the ASCII `+/-`, not `±` —
  the built-in font is a narrow subset and draws anything outside ASCII as an empty box.
- **Absolute nodes are the exception.** Inside a tab ordinary flex stacks the sections,
  and a section that hides itself (`Display::None`) is collapsed by the same flex; the
  only absolutely anchored nodes left are HUD ones (the left column, the panel, the
  hotkey help, the city select).
- **A demo scene that calls the kits brings the theme *and* the font itself.**
  `PanelWidgetsPlugin` installs only feathers and the theme: `apply_panel_font` lives
  in `UiPlugin`, which no example can raise. Without an `InheritableFont` on the
  scene's own plaque every label falls back to bevy's default 20 px face, the slider
  rows grow out of the panel and the value text wraps. So a demo plaque is
  `ui_node(…)` + `panel_background()` + its own `InheritableFont { fonts::REGULAR,
  PANEL_FONT }` (`crowd_demo/panel.rs::panel_font`, `tree_gallery/panel.rs::spawn_panel`),
  group headers `panel_block_background()` + `panel_title`, labels `row_label` /
  `row_value`, and the plaques hang in a `Pickable::IGNORE` column anchored to both
  screen edges — the shell's own shape. No colour of its own anywhere: a scene painted
  past the theme stops being a preview of the game's panel, which is the only reason
  to look at it. And every scene adds **`qwe::ui::QuitOnEscPlugin`** beside
  `PanelWidgetsPlugin` (the game's own Esc-quit, `ui/quit.rs`: focused window only,
  not while typing — a demo does not write its own copy) and
  **`qwe::ui::AgentBadgePlugin`** (the red BRP badge), with `qwe::ui::BelowBrpBadge`
  on the top-left plaque so the badge does not cover it.

## Camera start view

- **Camera start view** (`camera.rs`) — **`CameraPositionMode`** (`reset | save`,
  default `save`, the `Camera start` row of the Debug tab, persisted) decides where the
  camera stands at world start: `reset` — the snapped portal at `START_ZOOM`;
  `save` — the x/y/zoom written into **`SavedCameraView`** (persisted, same `camera`
  settings group) by `save_camera_view_on_exit`, a `Last` system that fires on
  `AppExit` and saves synchronously. It must run **after `bevy::window::ExitSystems`**:
  window close writes `AppExit` from `exit_on_all_closed`, itself in `Last` —
  without the ordering the save silently ran too early and nothing was ever written. `track_camera_view` (Update, only in `save` mode) covers the exits
  no schedule can see — macOS Cmd-Q, `brp quit` (its `AppExit` comes from
  `RemoteLast`, after `Last`), a crash — by writing during play, **debounced 1 s after
  the camera stops and throttled to one write per 10 s while it keeps moving** — a
  per-frame write would rewrite `settings.toml` a hundred times per drag. The debounce
  runs on `Time<Real>` on purpose — first-party
  `SaveSettingsDeferred` ticks on virtual time, so it would never fire while paused
  and fire 30× early at 30x speed. The view is applied in three places, all through
  `start_view` + `apply_view`: camera spawn (`Startup`, portal *hint* — the snapped
  position isn't known yet), `place_camera_on_world_ready` (`OnEnter(Playing)`) and
  the `RestartEvent` observer, so R puts the camera where an app start would.
  A world entry that is **not** the first one is a city switch and always resets to
  the new portal. **RR** — a second `RestartEvent` within `RESTART_DOUBLE_PRESS`
  (0.5 s real) — goes to the portal at `START_ZOOM` whatever the mode says: in
  `save` mode a single R is a no-op for the camera (the saved view follows it live),
  so the way back to the portal is the double press. `RestartEvent { to_portal: true }`
  asks for the same thing without the double press; world-setting restarts use it
  (`RestartPending`).

## Persistence & dev

- **Remembered UI options** (`prefs.rs`) — every UI-settable resource (`DebugGrid`,
  `PathfindingAlgorithm`, `TreeStyle`, `CameraPositionMode`, …) is a
  `bevy::settings::SettingsGroup`, so a click survives a
  restart. `SettingsPlugin` reads `settings.toml` from the OS settings dir (macOS:
  `~/Library/Preferences/com.github.morr.qwe/`) while the `App` is still being built,
  before any schedule; `PrefsPlugin` is registered **last** because that scan needs the
  other plugins' `register_type` calls to have run. Delete the file to reset — or
  press the Debug tab's **`reset`** button, the same thing.

  **`prefs::ResetSettings`** is a `Command`, and it too is a registration rather than a
  list: it walks the type registry, keeps whatever carries `ReflectSettingsGroup`, and
  writes back that type's `ReflectDefault` (both from
  `#[reflect(Resource, SettingsGroup, Default)]`). Two details that are correctness,
  not polish: a group **already at its default is skipped**, because merely taking a
  `Mut` marks the resource changed and `retuned` would then order a crown / road /
  building / polymesh rebuild for nothing; and the command queues
  `SaveSettingsSync::IfChanged` **itself** instead of trusting that something among the
  reset groups happened to be `track_pref`ed (`SavedCameraView` is not).
  `SavedCameraView` is reset like everything else and then immediately re-recorded by
  `track_camera_view` in `save` mode — right, since the setting is the *mode*, not
  where you happen to be looking.

  **Persisting is a registration, not a list**: `app.track_pref::<T>()` next to the
  resource's own `init_resource` adds a one-line system that queues
  `SaveSettingsSync::IfChanged` when `T` changes. A hand-maintained mirror of the
  tracked list had already lost `RoadStyle`, whose edits only reached the disk if some
  *other* tracked resource happened to change in the same frame. Deliberately
  untracked: `SavedCameraView` (changes every frame of a camera drag;
  `camera::track_camera_view` debounces it and saves by hand). `IfChanged` re-checks
  the whole file's change ticks itself, so two resources changing in one frame still
  cost one write.

  **`retuned::<T>`** (same module) is the matching run condition — `changed && !added`.
  Settings are applied to resources while the `App` is built, so on the world's first
  frame every tunable reads as *changed*; a rebuild fired by that is at best redundant
  (the world was just built from those values) and at worst a panic — there is nothing
  spawned to despawn yet. Every `rebuild_*`, the city reload and the
  determinism restart request use it; do not hand-roll
  `resource_changed::<T>.and_then(not(resource_added::<T>))` again.
- **dev.rs** — `TakeScreenshotEvent` (BRP-triggerable) → `screenshot.png`
  (gitignored); **`OffscreenShotEvent`** → a second camera rendering into an offscreen
  texture (what that means for shots is in `CONTEXT.md`; details in
  `.claude/live-app-project.md`). One law this layer adds: bevy's fallback UI camera is
  the highest-order camera **targeting the primary window**
  (`DefaultUiCamera::get` in bevy_ui) — that is why the main camera's
  `IsDefaultUiCamera` is a statement of intent against a future second *window* camera,
  and why the image-targeting offscreen camera was never a candidate for the fallback.
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
