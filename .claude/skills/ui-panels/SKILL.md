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

## Layout

The UI is two layers over the map: a **HUD** that is always on screen, and **one settings
panel** with four tabs (`ui/shell.rs`). Before this there were eight panels stacked into two
bottom columns whose `bottom` had to be computed from measured heights; the right column
did not fit 1080 px and ran off the top of the screen.

- **`ui/shell.rs`** — the shell. `SettingsTab { Map, Nav, Sim, Debug }` — **declaration
  order is strip order**; `UiShellState { tab, collapsed }` is a persisted settings group
  (`group = "ui"`), so the open tab survives a restart and `ResetSettings` puts it back to
  `Map`, expanded. `SettingsPanes` hands every panel the entity of its tab (`pane`) and the
  left column (`column`, for HUD blocks above the panel).
  Three ways to collapse to the bare tab strip: the `-`/`+` button, a click on the open tab
  (`on_tab_click`, a pure function with tests), and **`Tab`** — free precisely because
  `TabNavigationPlugin` is deliberately not installed (see the theme note below), and gated
  on `typing_in_text_input` like every other hotkey.
  The left column is `top`/`bottom`-anchored so the body has a ceiling to scroll against,
  and carries **`Pickable::IGNORE`**: it is taller than its content, and an invisible
  260 px strip eating the map's clicks would be exactly what "UI input never reaches the
  world" forbids. Its children stay pickable — the component is per-entity and does not
  propagate, so a subtree that must be transparent carries it on **every** node (the hotkey
  help does: plaque, rows and both labels, `ui/hotkeys.rs`).
  Wheel over the panel scrolls the body (`Overflow::scroll_y` + a `Scroll` entity event
  ported from bevy's `scroll_and_overflow` example; `clamp_scroll` is the tested part) —
  and `camera.rs::zoom_to_cursor` now runs under `not(hovering_ui)`, so it no longer zooms
  the map through the panel. That gate is a fix in its own right: it was missing before the
  panel could scroll at all.
- **Sections.** A panel asks for `spawn_section(commands, pane, slot, header, name)` — a
  column with a header band — or `spawn_block` for one without a header (the Nav tab, whose
  headers *are* rows: `Algo`, the `Separation` toggle, the `Slots` label). The returned
  entity is the parent it hands to the kits, so panel bodies did not change when they moved
  into tabs. **`SectionSlot`'s declaration order is the order inside a tab**
  (`sort_sections`, one pass at the end of `Startup`): the sections are spawned by eight
  systems in eight plugins, and system order inside `UiBuildSet::Sections` is unspecified —
  without the enum the Map tab came out shuffled on every run. A section that arrives
  **without** a `SectionSlot` sorts **last** (`slot_order`) and is named in a `warn!`: the
  enum exists so a section's place cannot be forgotten, and sorting a slotless one to the
  top would show that mistake as a mysteriously first panel body. `UiBuildSet` chains
  `Shell → Sections → Sort`.

## Panels

- **HUD counters** (`ui/stats.rs`) — Pawns (`With<Human>`, i.e. alive: the component is
  stripped on death), Demons, Souls reaped (`Telemetry::killed`), first in the left column,
  **outside** the tabs: they are watched continuously, and putting them behind a tab choice
  would mean watching the simulation through a keyhole. The counters use `iter().len()`,
  not `count()`: with a purely archetypal filter `QueryIter` is an `ExactSizeIterator`, so
  the length is a sum over archetypes rather than a walk over 20 000 entities every frame.
  In agent runs the red **BRP badge** owns that corner, and `offset_below_brp_badge`
  measures it and pushes the whole column below — `ComputedNode::size` is in *physical* px,
  so multiply by `inverse_scale_factor` or the offset doubles on a retina screen.
- **Telemetry panel** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight /
  avg ms, entity count, camera. Fixed width + right-padded digits (no jitter) — which only
  works in a monospace face, so this root brings its **own** `InheritableFont` (FiraMono).
  `apply_panel_font` therefore skips roots that already have one (`Without<InheritableFont>`);
  without that filter it overwrote FiraMono with FiraSans on the first frame and the columns
  wobbled.
- **Speed button** (`ui/speed.rs`) — left of that panel, a `Speed <value>` row-button.
  Left click walks the ladder up and wraps to 1x from its top step (`MAX_SIM_SPEED`), right
  click steps down; `Primary` while paused. It reads `Pointer<Click>` itself instead of
  `Activate`, which fires for *any* mouse button and would make one right click move both
  ways.
- **Sim tab** (`ui/stats.rs`) — three sections. **World**: `Deterministic` and the `Seed`
  field + `new`. **Demon**: the four `DemonStyle` knobs — **Max demons** (0…500, step 5),
  **Spawn every** (0.1…10 s, step 0.1), **Speed** (100…200%, step 5) and **Lunge boost**
  (+0…+100%, step 5); both percent rows print as percent, a bare `1.3` on the panel says
  nothing. **Human**: **Speed spread** (0…35%, step 5) — printed with a sign because it is a
  half-width, and a bare `15%` would read as "everyone 15% faster". The sign is the ASCII
  `+/-`, not `±`: the built-in font is a narrow subset and draws anything outside ASCII as
  an empty box. **Body radius** stood here and the crowd knobs in World until all six moved
  into the Nav tab's crowd groups — they are about movement.
- **Map tab** — Trees → Tree rows → Buildings → Roads → Noise.
  **Trees** (`ui/trees.rs`): shape / foliage / crown details / color variance, one button
  per row cycling through a fixed palette (`bevy_ui` has no text input, so hex fields became
  cycles), plus **slider rows** — **density** over `TREE_DENSITY_MIN..MAX`, **conifer share**
  over `TREE_CONIFER_SHARE_MIN..MAX` and **noise mix** over `TREE_NOISE_MIX_MIN..MAX`. The
  kit's drag observer quantizes to the step and writes `TreeStyle` only when the step
  actually changes, so one drag rebuilds the crowns a handful of times, not once per pixel.
  The conifer-share and mix rows are `Display::None`d outside `TreeShape::Mixed`
  (`sync_mixed_row_visibility`) — they mean nothing for the other shapes. Also settable over
  BRP: `res set TreeStyle .shape '"Conifer"'`.
  **Noise** (`ui/noise.rs`): the conifer-field fbm knobs (`ConiferNoiseStyle`: wavelength /
  octaves / lacunarity / persistence). In the Map tab, not Debug: the field decides where
  conifer stands land, so it is the look of the map, tuned next to Trees whose conifer share
  it distributes. Its first row is `Show` — the **same** `DebugConiferNoise` the Debug tab's
  overlay row carries, because tuning the field without the overlay is blind and sending the
  user to another tab for the switch would leave the section without one. The knobs (not the
  section) are `Display::None`d while the overlay is off. Noise *mix* is deliberately not
  here: it is a gameplay look knob, so it sits in Trees.
- **Nav tab** (`ui/navigation/` — `mod.rs` the rows, `knobs.rs` the crowd knobs,
  `overlay.rs` the polymesh overlay) — one block, **one UI for both pathfinding backends**.
  The top row **`Algo`** cycles `Navmesh` ⇄ `Polymesh`: pawns always walk one of the two, so
  it is a choice, not two toggles that could both read `Off` while the grid quietly served
  every request. Its single source of truth is `PolymeshDebug::enabled`, which defaults to
  `Polymesh` — and which the `Separation` row below follows, since separation does not run
  on the grid backend (see the navigation-deep skill): picking `Navmesh` here greys that row
  out the way determinism does. Under it stand the settings **of the selected backend only**
  — the other set is `Display::None`d out of the layout (`sync_section_visibility`), because
  an agent radius means nothing while pawns walk tiles, and a grid search algorithm means
  nothing while they walk the mesh:
  - `Navmesh` → **`Pathfind`** (`PathfindingAlgorithm`, cycles A*/Dijkstra/Fringe/BFS/
    HPA*/Theta*), **`Show`** (the grid fill overlay, `DebugNavmesh`);
  - `Polymesh` → **`Show`** (mesh overlay, draws nothing else), **`Chunks`** (default on)
    — the chunk hierarchy: it switches the *build* between layered and one flat layer
    (`FLAT_CHUNK_METERS`) and therefore triggers a rebuild, and it is what puts the grid
    on the overlay; one toggle for both halves on purpose, since a grid drawn over a
    search that does not use it is a picture of something untrue — and an **agent
    radius** slider (`POLYMESH_AGENT_RADIUS_MIN..MAX`, step 0.1 m) inflating obstacles at
    triangulation time.
  The radius minimum is deliberately non-zero (0.2 m) now that pawns
  walk the mesh, and it is read through `PolymeshDebug::radius()`, which clamps — the
  minimum was raised after the setting was already being persisted, so an older prefs
  file holds 0.0. The overlay is one merged mesh at z 5.3 (above the grid navmesh fill
  at 5.2):
  **blocked contours filled** in the *same* red as `sync_navmesh_overlay`, then **all
  polygon edges** of the built mesh stroked over it (shared edges deduped, so a
  translucent seam is never double-painted). Same colour is the point — the two layers
  paint the same claim, and only an identical fill makes their accuracy comparable by
  eye; with a non-zero agent radius the gap between fill and edges *is* the inflation.
  The chunk-grid boundaries go on top of the mesh edges — dark, half-transparent, and the
  same 0.4 m stroke width as an edge, since the grid is a partition drawn over the
  geometry, not another layer of world. They are drawn unconditionally from the built
  grid, which is 1×1 (no lines) for a flat mesh — the overlay states what the search
  actually walks, not what the toggle asks for.
  Below the backend settings sit two **groups**, `Separation` and `Slots` (`KnobGroup` —
  a plain enum, no resource and no component: it only sorts the knobs under their
  headers), holding every knob about how pawns get past each other and how they divide up
  end points. They live here rather than in World because both are about movement; World
  is the run (seed, determinism, counters), and the crowd knobs only ever sat there
  because the mechanism is species-independent. **Both groups are always expanded** —
  crowd knobs are tuned together, and hiding half of them would mean clicking back and
  forth mid-tuning; hiding is for what the current settings make irrelevant (the
  unselected backend's rows above), and these two always run. The `Separation` header
  *is* the toggle, exactly like `Algo`: `on`/`off` on the right, dimmed and inert (no
  hover highlight either) under determinism and on the grid backend
  (`separation_allowed_by_mode`), and its **knobs disappear** whenever separation is not
  running — determinism, the grid backend, or its own `off` — since there is nothing to
  tune while the mechanism never starts, the same rule that removes the unselected
  backend's rows. The header row stays: it *is* the toggle that brings separation back,
  and hiding it would lock you out. Their initial `Display` is set at spawn rather than
  left to `sync_separation_knob_visibility`, which runs under `resource_changed` and so
  does nothing on the first frame — with separation off at startup the sliders would have
  hung there until something else changed.
  Slots have no toggle — they run in both modes always —
  so `Slots` is a plain label, spawned without `Button` or an observer. Under the headers:
  **`Pass squeeze`**, **`Left share`** / **`Body radius`**, **`Slot search`**.
  Body radius is here despite living on `HumanStyle`: it sets both the rest
  distance and the slot side, so tuning wants it beside the other crowd knobs, not half a
  screen away in Human. All four are knob-kit rows, and each binds its **own** resource
  (`SeparationLab`, `HumanStyle`, `SlotSearch`) — the `Knob` enum that used to
  cover the group had to carry them all in one `SystemParam` for every drag.
  (`SlotLab` stays a demo-stand resource: the `Regroup` row that bound it was dropped
  together with the game-side `regroup_onto_slots`, which the wander invariant —
  every settled pawn carries `NeedsWanderTarget` — made unreachable in the game.)
  Nested *slider* rows are indented by `indent_slider_row`, which
  patches the padding the shared `ui/slider.rs` kit knows nothing about; without it a
  section's slider sat left of that same section's button rows — `Agent radius` had been
  sitting unindented since it was added.
  Cache key (build generation + radius bits) lives on the overlay marker, the
  conifer-overlay idiom; chunks are absent from it because flipping them moves the
  generation. The two overlays can no longer collide — two red fills over one map read as
  a single layer at double alpha, and each is now drawn **only while its backend is the
  selected one** (`sync_navmesh_overlay` returns early when `polymesh.enabled`), so the
  mutual-exclusion system that used to push the toggles apart is gone.

## Shared kits & layout

- **Widgets and theme** (`ui/theme.rs`) — the controls are first-party `bevy_feathers`
  (grey buttons, blue `Primary`, 4 px rounded corners, FiraSans); the theme is
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
    titles `PANE_HEADER_TEXT`. Hand-written colours (`ui_color`, `UiOpacity`,
    `TOGGLE_ACTIVE_COLOR`, `ROW_LABEL_COLOR`, `DIMMED_VALUE`) are gone: half the screen
    following the theme and half not is worse than either.
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
  - **The font is feathers' 14 px** (`PANEL_FONT` = `size::MEDIUM_FONT`). It was 12 while
    the panels stood in two columns from screen edge to screen edge; with one tabbed panel
    the reason is gone, and a caption you have to peer at is worse than two extra pixels.
    One `apply_panel_font` system puts `InheritableFont` on every `GameUiRoot` — except
    roots that brought their own (`Without<InheritableFont>`, i.e. the monospace telemetry),
    which it used to overwrite on the first frame. The kits patch the same size onto the
    widgets (which carry an `InheritableFont` of their own and would otherwise win).
  - **A container *between a font source and a label* comes from `ui_node` / `ui_row` /
    `ui_column`, never from a bare `Node`.** Font propagation runs
    `HierarchyPropagatePlugin::<TextFont, With<ThemedText>>`, and the traversal *stops* at
    the first entity without `ThemedText` — a wrapper between a root and its labels that
    lacks the marker leaves everything below at the default 20 px. That was not
    hypothetical: the World counters sat at 20 px in a different face for days, because the
    marker was something to remember rather than something a constructor brought.
    **The source itself is not such a container**: `InheritableFont` is
    `#[require(ThemedText, PropagateOver::<TextFont>)]`, so a root that brings its own font
    (the monospace telemetry) or is handed one by `apply_panel_font` (every `GameUiRoot`)
    carries the marker already and *starts* the chain instead of standing in it — which is
    why the roots of `ui/hotkeys.rs`, `ui/speed.rs` and `ui/city.rs` are bare `Node`s and
    are right to be. Nor is a `Node` patched onto a widget's own `bsn!` scene (`ui/rows.rs`,
    `ui/city.rs`, `ui/slider.rs` — the widget brings the font, and the slider adds a bare
    `ThemedText` because `FeathersSlider` does not) or a `Node` bundled with the very text
    it lays out (the `flex_grow: 1.` spacers). `warn_broken_font_chain` (debug builds,
    `Added<Text>`, after `apply_panel_font`) is the arbiter: it walks the chain, names the
    offending node in the log, and stays silent in exactly the cases above.
  `spawn_panel_button(commands, parent, marker, label, is_active, on_activate)` is the kit;
  `spawn_panel_button_with` takes the caption as a bundle instead of a string, for the
  two-text cycler rows. The caption is spawned as a child rather than passed as the scene's
  `@caption` because it comes from runtime strings.
- **City select** (`ui/city.rs`) — one `FeathersMenu` (button + `FeathersMenuPopup` +
  `FeathersMenuItem` per city), not a button per city: there are seven of them, the row took
  a third of the bottom edge and grew with each new city, and exactly one is ever chosen —
  that is a select. The popup flips itself above the button when there is no room below.
  `sync_city_label` keeps the button caption on the `City` resource, which `reset` and BRP
  also write.
- **Knob kit** (`ui/knob.rs`) — what panels actually call. A knob is a slider row bound
  to one field of one resource: `spawn_knob(commands, panel, label, &*resource, binding)`
  where `SliderBinding<R> { get, set, range, text }` is four function pointers, and
  `app.add_knobs::<R>()` registers the sync **once per resource** — not per knob, per
  panel or per field. Every panel used to hand-write the same drag observer (quantize,
  compare, write the field) and the same sync system (labels + `retarget` the thumbs):
  thirteen observers and eight systems that differed only in which field of which
  resource they touched. Two shapes worth knowing: an integer knob is `get: |r| r.n as
  f32` / `set: |r, v| r.n = v as u32` with an integer step (`Max demons`, `Octaves`), and
  a percent knob keeps the resource in 0..1 while `text` multiplies by 100 (`Speed`,
  `Conifer share`). `Knobbed` is just an alias for `Resource<Mutability = Mutable>` —
  `ResMut` needs it and repeating the bound in six signatures reads worse.
  `SliderBinding` implements `Clone`/`Copy` by hand: `derive` would demand `R: Copy`,
  which no resource is. `add_knobs::<R>()` is **idempotent** (it remembers registered
  `TypeId`s in `RegisteredKnobs`): one resource is knobbed from two panels at once —
  `HumanStyle` is `Speed spread` in Human *and* `Body radius` in Navigation — and each
  panel must register what it uses rather than assume its neighbour did, which is the
  mirror-of-a-list habit this whole change removes. Without the memo the second call
  would simply add a second copy of both systems.
- **Slider kit** (`ui/slider.rs`) — the layer under the knob kit, and the one the examples
  (`crowd_demo`, `tree_gallery`) call directly, since their sliders drive
  demo-local state rather than a resource: `spawn_slider_row` (label + value text +
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
  (`NavValueLabel`): their text is computed from **several** resources at once (`Backend`
  from `PolymeshDebug`, `Separation` from determinism + backend + style), which a binding
  to one resource cannot express — that enum is the right tool there, not a leftover.
- **Value-row kit** (`ui/rows.rs`) — the layer under cycle rows, and what the Navigation
  panel calls directly: `spawn_value_row` (grey label left, white value right, click on an
  observer), `row_color`, `next_in`, `on_off`. A row is a `FeathersButton` too, but with its
  own scene rather than `spawn_panel_button_with`'s: it spans the panel, its left padding
  carries the nesting indent, and its label stretches to push the value right. What the two
  share is the colour, and that comes from the theme. There is no highlight system left —
  feathers paints hover and press itself. A row whose click currently does nothing gets
  first-party **`bevy::ui::InteractionDisabled`** from its panel, which both stops the
  highlight and swallows `Activate` — promising a reaction the click will not deliver is
  worse than not highlighting. `BUTTON_BG_DISABLED` is deliberately the *resting* colour,
  not a dimmed one: an inert row should look ordinary and merely not react. The only carrier
  today is the Separation toggle under **Deterministic** or grid navigation.
- **The bottom columns are gone.** `stack_bottom_columns`, `column_bottom`, `Stacked`,
  `UiRightColumn` / `UiLeftColumn` / `UiPanelGapBelow`, `panel_node`, `right_panel` /
  `left_panel` and the four column-arithmetic tests were deleted with the tabs: inside a
  tab ordinary flex stacks the sections, and a section that hides itself
  (`Display::None`) is collapsed by the same flex. Only two absolute nodes are left, both
  HUD: the hotkey help (bottom right) and the city select (bottom centre). What survived
  the deletion is the retina trap — `ComputedNode::size` is in *physical* pixels, and
  `offset_below_brp_badge` still multiplies by `inverse_scale_factor`.
- **Debug tab** (`ui/debug/`) — the overlay rows (grid / doors / move paths / noise field),
  the `Camera start` and `Navtile` cyclers, and `reset`. All of them are knob-kit rows
  (`spawn_cycle_row` + `add_knobs::<R>()`) now, which is why `DebugToggleButton`,
  `CyclerButton`, `cycler_state`, `sync_toggle_buttons`, `sync_cycler_buttons` and
  `sync_cycler_labels` are gone: the kit already keeps a label on its resource. The old row
  of bare buttons (`grid`, `doors`, `camera:`) had no value text at all, so it needed a
  green fill to say "on" and "at its default" — a row that prints `Off` or `save` says it
  outright, and the colour became a second, weaker channel for the same claim.
  **`reset`** (`prefs::ResetSettings`) stays an action **button**, not a row: it has no state
  of its own to print — every settings group goes back to its `Default`, world settings
  included, so an off-baseline city / seed / navtile / determinism reloads the map on the
  click. Navtile is in this tab and not under `Navmesh` because the world is *always* built
  in tiles of that size — the passability fill, the unreachable prune, the portal snap, the
  entrance generation — whichever backend the pawns then walk; hiding it with the grid
  settings would call a global setting a local one. The navmesh overlay this module still
  owns is **one merged mesh** — per-tile entities once cost 330 k entities; the noise overlay
  is one sprite with a CPU-built texture (see the osm-map skill's `references/trees.md`,
  Conifer stands).

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
  writes back that type's `ReflectDefault` — both of which every group already registers
  through `#[reflect(Resource, SettingsGroup, Default)]`. A new tunable is therefore
  resettable the day it is declared, and there is no second place to forget it. Two details
  that are correctness, not polish: a group **already at its default is skipped**, because
  merely taking a `Mut` marks the resource changed and `retuned` would then order a crown /
  road / building / polymesh rebuild for nothing; and the command queues
  `SaveSettingsSync::IfChanged` **itself** instead of trusting that something among the
  reset groups happened to be `track_pref`ed (`SavedCameraView` is not). `SavedCameraView`
  is reset like everything else and then immediately re-recorded by `track_camera_view` in
  `save` mode — right, since the setting is the *mode*, not where you happen to be looking.

  **Persisting is a registration, not a list**: `app.track_pref::<T>()` next to the
  resource's own `init_resource` adds a one-line system that queues
  `SaveSettingsSync::IfChanged` when `T` changes. It replaced a 20-branch `or_else`
  chain inside `prefs.rs` — a mirror of a list maintained elsewhere, and it had lost
  `RoadStyle`, whose edits only reached the disk if some *other* tracked resource
  happened to change in the same frame. Deliberately untracked: `SavedCameraView`
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
  `SpawnTestWalkerEvent` for A/B path checks; frame-time diagnostics.
- **BRP** — `RemoteHttpPlugin` on port 15702; drive it via the `live-app` skill's `brp`
  script only.
