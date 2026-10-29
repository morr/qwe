---
name: ui-panels
description: Use when working on qwe's UI — the tabbed settings panel (Map / Nav / Sim / Debug) and its shell, the HUD (counters, telemetry, speed, city, hotkeys), the knob / slider / value-row kits, the feathers theme and its translucent plaques, the camera start view, prefs persistence. Deep detail behind CONTEXT.md's UI summary.
---

# UI & debug panels — deep detail

Detail behind `src/ui/` and `camera.rs`. The panel inventory and the UI-input rule live
in `CONTEXT.md`; the "UI input must not reach the game world" rule itself is in
`CLAUDE.md` and is non-negotiable.

**Changing that mechanism is changing this file, in the same change** — a new kit, a moved
row, a retired widget. The term still goes to `CONTEXT.md`; the detail lives only here.

## References

- [`references/panels.md`](references/panels.md) — the per-tab bodies: every section's
  rows, ranges and wiring for **Map** (Trees / Tree rows / Buildings / Roads / Road
  paint / Surfaces / Sun / Noise), **Nav** (the one UI for both pathfinding backends,
  the crowd groups, the polymesh overlay's drawing laws), **Sim** (World / Demon /
  Human) and **Debug** (the overlay rows, `Raw OSM`, `Navtile`, `reset`).

## Layout

The UI is two layers over the map: a **HUD** that is always on screen, and **one settings
panel** with four tabs (`ui/shell.rs`).

- **`ui/shell.rs`** — the shell. `SettingsTab { Map, Nav, Sim, Debug }` — **declaration
  order is strip order**; the open tab and the collapsed flag are a persisted settings
  group (`UiShellState`, `group = "ui"`). `SettingsPanes` hands every panel the entity of
  its tab (`pane`) and the left column (`column`, for HUD blocks above the panel).
  It collapses to the bare tab strip three ways: the `-`/`+` button, a click on the open
  tab (`on_tab_click`, a pure function with tests), and **`Tab`** — free precisely because
  `TabNavigationPlugin` is deliberately not installed (see the theme note below), gated
  on `typing_in_text_input` like every other hotkey.
  The left column is `top`/`bottom`-anchored so the body has a ceiling to scroll against,
  and carries **`Pickable::IGNORE`**: it is taller than its content, and an invisible
  260 px strip eating the map's clicks would be exactly what "UI input never reaches the
  world" forbids. Its children stay pickable — the component is per-entity and does not
  propagate, so a subtree that must be transparent carries it on **every** node (the hotkey
  help does: plaque, rows and both labels, `ui/hotkeys.rs`).
  Wheel over the panel scrolls the body (`Overflow::scroll_y` + a `Scroll` entity event
  ported from bevy's `scroll_and_overflow` example; `clamp_scroll` is the tested part).
- **Sections.** A panel asks for `spawn_section(commands, pane, slot, header, name)` — a
  column with a header band — or `spawn_block` for one without a header (the Nav tab, whose
  headers *are* rows: `Algo`, the `Separation` toggle, the `Slots` label). The returned
  entity is the parent it hands to the kits. **`SectionSlot`'s declaration order is the
  order inside a tab**
  (`sort_sections`, one pass at the end of `Startup`): the sections are spawned by one
  system per section plugin, and system order inside `UiBuildSet::Sections` is
  unspecified. A section that arrives **without** a `SectionSlot` sorts **last**
  (`slot_order`) and is named in a `warn!` — the enum exists so a section's place cannot
  be forgotten, and sorting a slotless one to the top would show that mistake as a
  mysteriously first panel body. `UiBuildSet` chains `Shell → Sections → Sort`.
- **Bloom** (`post.rs::camera_post_process`) — the bundle `camera.rs` adds to the camera:
  `Hdr`, `Tonemapping::Linear`, `DebandDither::Disabled` and `Bloom::NATURAL` at
  `BLOOM_INTENSITY` 0.25 — 0.4 reads as haze now that every demon carries a halo and
  every death a spark — `Additive` composite. What is brighter than 1.0 draws itself so:
  the portal vortex and its rim (`portal.rs`), the demon halo (~1.4 after alpha,
  `demon/look.rs`) and the soul spark (`human/soul.rs`) — nothing on the map is. The
  prefilter is `BLOOM_THRESHOLD` 1.1 / `BLOOM_THRESHOLD_SOFTNESS` 0.1 — the knee runs
  `threshold × softness` both ways, so it starts at 0.99 and pure white lane markings
  (1.0) contribute nothing; a 1.0 threshold made everything above 0.6 glow and hazed the
  whole map. Tonemapping stays off on purpose: every built-in curve recolours the map
  palette, and only the halo is wanted. That is `Tonemapping::Linear`, not `None`: since
  Bevy 0.20 `None` is a full passthrough that also drops the negative-channel clamp, and
  an additive bloom wants the clamp. `Linear` is what `None` used to be. UI is drawn
  after post-processing, so panels never bloom.
- **Antialias** (`post.rs::Antialias`, settings group `render`, key `antialias`, on by
  default) — `Msaa::Sample4` on the user's camera (`camera.rs::spawn_camera`) and on the
  offscreen-shot camera (`dev.rs`), both read the resource at spawn and
  `post::apply_antialias` (`retuned`) writes every `Camera2d` on a toggle. Default on
  because without it the map's ribbons — asphalt, kerbs, median noses, islands —
  stair-step on a close-up, while the paint beside them is smoothed by `paint.wesl`
  itself. HDR + bloom + the four `Material2d`s work with it unchanged (they specialise
  on the sample count themselves). **Cost is memory, not frame time**: on a 4K window
  the multisampled HDR target and depth add about 400 MB; the frame was measured in the
  `roads` gallery rendering offscreen at 7500×4750 (4× a 4K window) — medians within a
  millisecond of each other, both at the 60 Hz pace. Hence the toggle: the Debug tab's
  `Antialias` row (`4x` / `Off`, in World build under `Camera start`). The galleries and
  `crowd_demo` run `Sample4` hard-wired.
- **Expensive-knob debounce** — the one place a panel is decoupled from what the map
  follows. **Sun** (`ui/sun.rs`) and the Roads shape sliders write their knob resource
  like any other kit row, but the map follows a **settled copy**: `map::sun::settle_sun`
  copies `SunStyle` into `SunOnMap`, `settle_road_shape` copies `RoadShape` into
  `RoadShapeOnMap`, each after **0.35 s of quiet** — a rebuild per drag step would cost
  hundreds of ms. Nothing in `ui/` knows about that: the debounce sits between the
  setting and the map, not inside the panel, precisely so a section stays a section.
  Ranges and per-knob rules — `references/panels.md`.
- **Vignette** (`post.rs::spawn_vignette`) — a full-screen `Node` with a radial
  `BackgroundGradient` (transparent to 55% of the far-corner radius, black at
  `VIGNETTE_ALPHA` 0.22 in the corners). `GlobalZIndex(-1)` keeps it under every panel,
  `Pickable::IGNORE` keeps it out of `HoverMap` — without that it would be "a UI node
  under the cursor" everywhere and `pointer_over_ui` would hand the whole map to the UI.
  Not a `GameUiRoot`: it is not hidden with the game UI and survives the city switch. The
  opaque loader root (z 0) covers it while `Loading` — the vignette frames the map, not
  the loader. Spawned once at `Startup`.

## HUD

- **Counters** (`ui/stats.rs`) — Pawns (`With<Human>`, i.e. alive: the component is
  stripped on death), Demons, Souls reaped (`Telemetry::killed`), first in the left column,
  **outside** the tabs: they are watched continuously, and putting them behind a tab choice
  would mean watching the simulation through a keyhole. The counters use `iter().len()`,
  not `count()`: with a purely archetypal filter `QueryIter` is an `ExactSizeIterator`, so
  the length is a sum over archetypes rather than a walk over 20 000 entities every frame.
  In agent runs the red **BRP badge** (`ui/brp.rs`) owns that corner, and
  `offset_below_brp_badge` measures it and pushes every `BelowBrpBadge` node below —
  `ComputedNode::size` is in *physical* px, so multiply by `inverse_scale_factor` or the
  offset doubles on a retina screen. The game marks an agent run by `BRP_PORT` (`main.rs`);
  a demo has no port, so its `AgentBadgePlugin` also takes `CLAUDECODE`, which Claude Code
  sets on every process of its session.
- **Telemetry** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight / avg ms,
  entity count. Fixed width + right-padded digits (no jitter) — which only works in a
  monospace face, so this root brings its **own** `InheritableFont` (FiraMono);
  `apply_panel_font` therefore skips roots that already have one
  (`Without<InheritableFont>`). Its fixed columns of the pathfinding lines are one half
  of the ruler `tools/shot_coords` works from — changing the panel's geometry, font or
  any of these format strings means re-measuring the constants at the top of that script
  in the same change.
- **Point inspector** (`ui/inspector.rs`) — directly **below the telemetry**
  (`position_below_telemetry` measures the telemetry's `ComputedNode` and sets `top`, the
  `offset_below_brp_badge` idiom; in the roads gallery there is no telemetry, `Single`
  skips the system and the panel stands at the screen edge). Two FiraMono lines: the
  **camera line** `zoom/x/y cx/cy` and the **hits list** — what the map draws under the
  cursor, recomputed when the point moves over `INSPECTOR_QUERY_STEP` (0.5 m) or the
  semantic index is rebuilt.
  Two modes: `UiInspectorPlugin::game()` (a `GameUiRoot` — hidden until
  `PlayPhase::Live`, hidden on city switch) and `UiInspectorPlugin::standalone()` (the
  roads gallery: visible from the first frame, states don't exist there). `tools/shot_coords`
  reads the camera line; its geometry constants belong to **this** panel now.
- **Speed button** (`ui/speed.rs`) — left of the telemetry, a `Speed <value>` row-button.
  Left click walks the ladder up and wraps to 1x from its top step (`MAX_SIM_SPEED`), right
  click steps down; `Primary` while paused. It reads `Pointer<Click>` itself instead of
  `Activate`, which fires for *any* mouse button and would make one right click move both
  ways.
- **City select** (`ui/city.rs`) — one `FeathersMenu` (button + `FeathersMenuPopup` +
  `FeathersMenuItem` per city), not a button per city: exactly one is ever chosen — that
  is a select, and a row of buttons grew with each new city. The popup flips itself above
  the button when there is no room below. `sync_city_label` keeps the button caption on
  the `City` resource, which `reset` and BRP also write. The select itself is
  `pub fn spawn_city_select(commands, parent, city, width)`, re-exported with
  `sync_city_label` from `qwe::ui`: the roads gallery puts the same select on its own
  panel and registers the label sync itself.

## Shared kits & layout

- **Widgets and theme** (`ui/theme.rs`) — the controls are first-party `bevy_feathers`;
  the theme is
  `create_dark_theme()` with **colour overrides only**. **`PanelWidgetsPlugin`** installs
  `FeathersCorePlugin` plus `UiTheme`; it is a plugin of its own rather than two lines in
  `UiPlugin` because the crowd demo calls the same kits and cannot bring up `UiPlugin`.
  Six things about it are decisions, not detail:
  - **`FeathersCorePlugin`, never the `FeathersPlugins` group.** The group also adds
    `TabNavigationPlugin`, and with it Tab focuses any panel widget (every control carries
    `TabIndex(0)`), after which Space both "presses" the focused button *and* pauses the
    simulation — the "UI input must not reach the game world" rule, and
    `typing_in_text_input` does not catch it because the focus is not on a text field.
    Without the plugin `TabIndex` / `FocusIndicator` simply do nothing.
  - **Everything that is not a widget is coloured by the same tokens.** Panel backings are
    `panel_background()` (`tokens::PANE_BODY_BG`) and `panel_block_background()`
    (`GROUP_BODY_BG`), labels are `row_label` (`TEXT_DIM`) and `row_value` (`TEXT_MAIN`),
    titles `PANE_HEADER_TEXT`.
  - **The plaques are translucent, and the text carries the legibility.** One near-black
    `UI_COLOR` at three alphas — `PANEL_ALPHA` .72 (pane body), .85 (header strip), .50
    (nested block) — so the city stays visible under the panel; `TEXT_MAIN` is white and
    `TEXT_DIM` is oklch L .90 (`MUTED_TEXT`), both well above feathers' own. The library's
    values assume an opaque inspector plaque; over a bright map they read as grey on grey.
    Raising the alpha instead would have been the wrong knob: the panel is supposed to lie
    *on* the map, not replace it.
  - **A value row rests transparent, a button goes blue.** Rows are
    `ButtonVariant::Plain` (`rows::VALUE_ROW_VARIANT`, `BUTTON_PLAIN_BG` = `Color::NONE`):
    a panel is ten rows in a stack, and `Normal` gave each its own grey brick. A row says
    its state in **words** on the right (`On`/`Off`, `Round`, `Polymesh`); `Primary`
    (`button_variant(is_active)`) is for the buttons that stand alone instead of in a column
    of rows, and say not their own value but "where am I" / "what state is the world in" —
    the open tab in the strip (`ui/shell.rs::sync_shell`) and pause on the speed button
    (`ui/speed.rs::update_speed_button`). Not the Debug tab's toggles (`spawn_cycle_row`
    value rows printing `On`/`Off`), and not the city, which is a `FeathersMenu` popup with
    no variant at all. Panels never paint backgrounds; feathers does hover, press and
    disabled itself, in a `Changed`-filtered `PreUpdate` system.
  - **The font is feathers' 14 px** (`PANEL_FONT` = `size::MEDIUM_FONT`). One
    `apply_panel_font` system puts `InheritableFont` on every `GameUiRoot` — except
    roots that brought their own (`Without<InheritableFont>`, i.e. the monospace
    telemetry). The kits patch the same size onto the widgets (which carry an
    `InheritableFont` of their own and would otherwise win).
  - **A container *between a font source and a label* comes from `ui_node` / `ui_row` /
    `ui_column`, never from a bare `Node`.** Font propagation runs
    `HierarchyPropagatePlugin::<TextFont, With<ThemedText>>`, and the traversal *stops* at
    the first entity without `ThemedText` — a wrapper between a root and its labels that
    lacks the marker leaves everything below at the default 20 px. **The source itself is
    not such a container**: `InheritableFont` is
    `#[require(ThemedText, PropagateOver::<TextFont>)]`, so a root that brings its own font
    (the monospace telemetry) or is handed one by `apply_panel_font` (every `GameUiRoot`)
    carries the marker already and *starts* the chain instead of standing in it — which is
    why the roots of `ui/hotkeys.rs`, `ui/speed.rs` and `ui/city.rs` are bare `Node`s and
    are right to be. Nor is a `Node` patched onto a widget's own `bsn!` scene
    (`ui/rows.rs`, `ui/city.rs`, `ui/slider.rs` — the slider adds a bare `ThemedText`
    because `FeathersSlider` does not) or a `Node` bundled with the very text it lays out
    (the `flex_grow: 1.` spacers). `warn_broken_font_chain`
    (debug builds, `Added<Text>`, after `apply_panel_font`) is the arbiter: it walks the
    chain, names the offending node in the log, and stays silent in exactly the cases
    above.
  `spawn_panel_button(commands, parent, marker, label, is_active, on_activate)` is the kit;
  `spawn_panel_button_with` takes the caption as a bundle instead of a string, for the
  two-text cycler rows. The caption is spawned as a child rather than passed as the scene's
  `@caption` because it comes from runtime strings.
- **Knob kit** (`ui/knob.rs`) — what panels actually call. A knob is a slider row bound
  to one field of one resource: `spawn_knob(commands, panel, label, &*resource, binding)`
  where `SliderBinding<R> { get, set, range, text }` is four function pointers, and
  `app.add_knobs::<R>()` registers the drag observer and the sync **once per resource** —
  not per knob, per panel or per field. Two shapes worth knowing: an integer knob is
  `get: |r| r.n as f32` / `set: |r, v| r.n = v as u32` with an integer step, and a
  percent knob keeps the resource in 0..1 while `text` multiplies by 100. `Knobbed` is
  just an alias for `Resource<Mutability = Mutable>` — `ResMut` needs it and repeating
  the bound in six signatures reads worse. `SliderBinding` implements `Clone`/`Copy` by
  hand: `derive` would demand `R: Copy`, which no resource is. `add_knobs::<R>()` is
  **idempotent** (it remembers registered `TypeId`s in `RegisteredKnobs`): one resource
  is knobbed from two panels at once — `HumanStyle` is `Speed spread` in Human *and*
  `Body radius` in Navigation — and each panel must register what it uses rather than
  assume its neighbour did. Without the memo the second call would simply add a second
  copy of both systems.
  **The module is `pub`, and the demo galleries call it** (`examples/demos/*`): each
  keeps a `ParamSpec { label, group, binding }` table of its own — the label and the
  group header are the gallery's, the binding is the kit's — and its `main.rs` registers
  `add_knobs::<Tuning>()` beside `init_resource::<Tuning>()`. A gallery still owns its
  **reset** button, which the kit has no notion of, and `tree_gallery` compares it knob
  by knob through `binding.get` rather than by `!=` on the whole `Tuning` —
  `CrownParams` is not `PartialEq`.
- **Slider kit** (`ui/slider.rs`) — the layer under the knob kit, and the one `crowd_demo`
  calls directly, since its fifteen knobs interleave three resources in one `const` slice
  that also feeds the presets: `spawn_slider_row` (label + value text +
  discrete `bevy_ui_widgets::Slider`), `quantize`, `apply_step` (quantize + put the thumb
  back on the stepped value), `retarget` (move the thumb when the value arrived past it —
  over BRP, from saved settings, from a lab preset). Free functions only: the file holds no
  system and no shared slider marker, and `UiPlugin` registers nothing out of it. What keeps
  a panel's thumb on its value is `sync_knob_values::<R>` in `ui/knob.rs` — registered by
  `add_knobs::<R>()` once per resource, from each panel's own plugin, it `retarget`s every
  entity carrying `SliderBinding<R>`. A caller that drives demo-local state instead of a
  resource writes that loop itself.
- **Cycle rows** — the button half of the knob kit, for the values `bevy_ui` has no input
  field for: `spawn_cycle_row(.., CycleBinding { cycle, text })` where `cycle` advances the
  field by itself. Deliberately not "next item of `ALL`": what cycles is enums, plain
  bools, colour palettes and step tables alike, and forcing them through one list would
  need a type per kind. Registered by the same `add_knobs::<R>()` as the sliders.
  `CycleBinding` is *not* a component — the click observer captures it, and the sync only
  needs it on the label. The rows that stay hand-written are the Nav tab's
  (`NavValueLabel`): their text is computed from **several** resources at once
  (`Backend` from `PolymeshDebug`, `Separation` from determinism + backend + style),
  which a binding to one resource cannot express.
- **Value-row kit** (`ui/rows.rs`) — the layer under cycle rows, and what the Navigation
  panel calls directly: `spawn_value_row` (grey label left, white value right, click on an
  observer), `row_color`, `next_in`, `on_off`. A row is a `FeathersButton` too, but with
  its own scene rather than `spawn_panel_button_with`'s: it spans the panel, its left
  padding carries the nesting indent, and its label stretches to push the value right.
  What the two share is the colour, and that comes from the theme. A row whose click
  currently does nothing gets first-party **`bevy::ui::InteractionDisabled`** from its
  panel, which both stops the highlight and swallows `Activate` — promising a reaction
  the click will not deliver is worse than not highlighting. There is no dimmed-disabled
  colour of our own: feathers simply stops repainting the row, so it keeps its *resting*
  colour — an inert row should look ordinary and merely not react.
- **Printing text**: a value that prints with a sign keeps the ASCII `+/-`, not `±` —
  the built-in font is a narrow subset and draws anything outside ASCII as an empty box.
- **Absolute nodes are the exception.** Inside a tab ordinary flex stacks the sections,
  and a section that hides itself (`Display::None`) is collapsed by the same flex; the
  only absolutely anchored nodes left are HUD ones (the left column, the panel, the
  hotkey help, the city select).
- **A demo scene that calls the kits brings the theme *and* the font itself.**
  `PanelWidgetsPlugin` exists for the first half (it is a plugin of its own precisely so an
  example can raise it without `UiPlugin`), but it installs only feathers and the theme:
  `apply_panel_font` lives in `UiPlugin`, which no example can raise. Without an
  `InheritableFont` on the scene's own plaque every label falls back to bevy's default
  20 px face, the slider rows grow out of the panel and the value text wraps. So a demo
  plaque is
  `ui_node(…)` + `panel_background()` + its own `InheritableFont { fonts::REGULAR,
  PANEL_FONT }` (`crowd_demo/panel.rs::panel_font`, `tree_gallery/panel.rs::spawn_panel`),
  its group headers `panel_block_background()` + `panel_title`, its labels `row_label` /
  `row_value`, and its plaques hang in a `Pickable::IGNORE` column anchored to both screen
  edges — the shell's own shape. No colour of its own anywhere: a scene painted past the
  theme stops being a preview of the game's panel, which is the only reason to look at it.
  And every scene adds **`qwe::ui::QuitOnEscPlugin`** next to `PanelWidgetsPlugin` — the
  game's own Esc-quit (`ui/quit.rs`: focused window only, not while typing in a text
  field), which `main.rs` raises too; a demo does not write its own copy. Beside it goes
  **`qwe::ui::AgentBadgePlugin`** — the game's red BRP badge, shown when Claude Code
  launched the window — and the scene's top-left plaque carries `qwe::ui::BelowBrpBadge`
  so the badge does not cover it.

## Camera start view

- **Camera start view** (`camera.rs`) — **`CameraPositionMode`** (`reset | save`, default
  `save`, the `Camera start` row of the Debug tab, persisted) decides where the camera stands when the world comes up:
  `reset` — the snapped portal at `START_ZOOM`; `save` — the x/y/zoom written into
  **`SavedCameraView`** (persisted, same `camera` settings group) by
  `save_camera_view_on_exit`, a `Last` system that fires on `AppExit` and saves
  synchronously. It must run **after `bevy::window::ExitSystems`**: closing the window
  writes `AppExit` from `exit_on_all_closed`, which is itself in `Last`, so without the
  ordering the save silently ran a system too early and nothing was ever written.
  `track_camera_view` (Update, only in `save` mode) covers the exits no schedule can see —
  macOS Cmd-Q, `brp quit` (its `AppExit` comes from `RemoteLast`, after `Last`), a crash —
  by writing during play, **debounced 1 s after the camera stops and throttled to one
  write per 10 s while it keeps moving**: a drag is dozens of frames, and a per-frame write
  would rewrite `settings.toml` a hundred times per gesture. The debounce runs on
  `Time<Real>` on purpose — first-party `SaveSettingsDeferred` ticks on virtual time, so it
  would never fire while paused and fire 30× early at 30x speed. The view is applied in three places, all through `start_view` + `apply_view`:
  camera spawn (`Startup`, portal *hint* — the snapped position isn't known yet),
  `place_camera_on_world_ready` (`OnEnter(Playing)`) and the `RestartEvent` observer, so R
  puts the camera exactly where an app start would. A world entry that is **not** the
  first one is a city switch and always resets to the new portal.
  **RR** — a second `RestartEvent` within `RESTART_DOUBLE_PRESS` (0.5 s of real time) —
  goes to the portal at `START_ZOOM` whatever the mode says: in `save` mode a single R is
  a no-op for the camera (the saved view follows it live), so the way back to the portal
  is the double press. `RestartEvent { to_portal: true }` asks for the same thing without
  the double press, and every restart ordered by a changed world setting uses it
  (`RestartPending`).

## Persistence & dev

- **Remembered UI options** (`prefs.rs`) — every UI-settable resource (`DebugGrid`,
  `DebugNavmesh`, `DrawMovePaths`, `PathfindingAlgorithm`, `TreeStyle`,
  `CameraPositionMode`, …) is a `bevy::settings::SettingsGroup`, so a click survives a
  restart. `SettingsPlugin` reads
  `settings.toml` from the OS settings dir (macOS:
  `~/Library/Preferences/com.github.morr.qwe/`) while the `App` is still being
  built, before any schedule; `PrefsPlugin` is registered **last** because that scan needs
  the other plugins' `register_type` calls to have run. Delete the file to reset — or press
  the Debug tab's **`reset`** button, which is the same thing from inside the game.

  **`prefs::ResetSettings`** is a `Command`, and it too is a registration rather than a
  list: it walks the type registry, keeps whatever carries `ReflectSettingsGroup`, and
  writes back that type's `ReflectDefault` — both registered by
  `#[reflect(Resource, SettingsGroup, Default)]`. Two details
  that are correctness, not polish: a group **already at its default is skipped**, because
  merely taking a `Mut` marks the resource changed and `retuned` would then order a crown /
  road / building / polymesh rebuild for nothing; and the command queues
  `SaveSettingsSync::IfChanged` **itself** instead of trusting that something among the
  reset groups happened to be `track_pref`ed (`SavedCameraView` is not). `SavedCameraView`
  is reset like everything else and then immediately re-recorded by `track_camera_view` in
  `save` mode — right, since the setting is the *mode*, not where you happen to be looking.

  **Persisting is a registration, not a list**: `app.track_pref::<T>()` next to the
  resource's own `init_resource` adds a one-line system that queues
  `SaveSettingsSync::IfChanged` when `T` changes. A hand-maintained mirror of the tracked
  list had already lost `RoadStyle`, whose edits only reached the disk if some *other*
  tracked resource happened to change in the same frame. Deliberately untracked:
  `SavedCameraView`
  (changes every frame of a camera drag; `camera::track_camera_view` debounces it and
  saves by hand). `IfChanged` re-checks the whole file's change ticks itself, so two
  resources changing in one frame still cost one write.

  **`retuned::<T>`** (same module) is the matching run condition — `changed && !added`.
  Settings are applied to resources while the `App` is built, so on the world's first
  frame every tunable reads as *changed*; a rebuild fired by that is at best redundant
  (the world was just built from those values) and at worst a panic — there is nothing
  spawned to despawn yet. Every `rebuild_*` in `map/mod.rs`, the city reload and the
  determinism restart request use it; do not hand-roll
  `resource_changed::<T>.and_then(not(resource_added::<T>))` again.
- **dev.rs** — `TakeScreenshotEvent` (BRP-triggerable) → `screenshot.png` (gitignored);
  **`OffscreenShotEvent`** → a second camera rendering into an offscreen texture (what it
  means for shots — UI and vignette out, the second `Camera2d`, the borrowed zoom — is in
  CONTEXT.md; details in `.claude/live-app-project.md`). One law this layer adds: bevy's
  fallback UI camera is the highest-order camera **targeting the primary window**
  (`bevy_ui/src/ui_node.rs::DefaultUiCamera::get`) — that is why the main camera's
  `IsDefaultUiCamera` is a statement of intent against a future second *window* camera,
  and why the image-targeting offscreen camera was never a candidate for the fallback.
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
