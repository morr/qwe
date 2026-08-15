---
name: ui-panels
description: Use when working on qwe's UI — panels (World/Demon/Human, Navigation, Trees, Noise, speed/telemetry), the slider and value-row kits, bottom column stacking, debug toggles, the camera start view, prefs persistence. Deep detail behind CONTEXT.md's UI summary.
---

# UI & debug panels — deep detail

Detail behind `src/ui/` and `camera.rs`. The panel inventory and the UI-input rule live
in `CONTEXT.md`; the "UI input must not reach the game world" rule itself is in
`CLAUDE.md` and is non-negotiable.

## Panels

- **Telemetry panel** (`ui/speed.rs`) — top-right: sim clock, pathfinding in-flight /
  avg ms, entity count, camera. Fixed width + right-padded digits (no jitter).
- **World, Demon and Human panels** (`ui/stats.rs`) — top-left, the only corner the other panels
  leave free, one under the other in a plain flex column (it grows *downward* from the
  screen edge, so unlike `stack_bottom_columns` nothing has to measure heights).
  **World** holds three live counters — **Pawns** (`With<Human>`, i.e. alive: the
  component is stripped on death), **Demons**, **Souls reaped** (`Telemetry::killed`), on
  their own `Heavy` backing the way slider rows have one. **Demon** holds the four
  `DemonStyle` knob rows from the same `ui/knob.rs` kit as Trees and Noise —
  **Max demons** (0…500, step 5), **Spawn every** (0.1…10 s, step 0.1), **Speed**
  (100…200%, step 5) and **Lunge boost** (+0…+100%, step 5); both percent rows print as
  percent, a bare `1.3` on the panel says nothing. **Human** holds the single
  `HumanStyle` row, **Speed spread** (0…35%, step 5) — printed with a sign because it is
  a half-width, and a bare `15%` would read as "everyone 15% faster". The sign is the
  ASCII `+/-`, not `±`: the built-in font (the `default_font` feature) is a narrow subset
  and draws anything outside ASCII as an empty box. **Body radius** stood here
  and the `Separation` toggle, `Slot search` and the three crowd knobs stood in World
  until all six moved into the Navigation panel's crowd groups — they are about
  movement, and World had stopped reading as a summary of the run.
  The counters use `iter().len()`, not
  `count()`: with a purely archetypal filter `QueryIter` is an `ExactSizeIterator`, so the
  length is a sum over archetypes rather than a walk over 20 000 entities every frame.
  In agent runs the red **BRP badge** owns that same corner, and `offset_below_brp_badge`
  measures it and pushes the column below — the `ComputedNode` physical-vs-logical px trap
  is the same one `stack_bottom_columns` documents.
- **Speed button** (`ui/speed.rs`) — left of that panel, a `Speed <value>` row-button in
  the Buildings-panel style. Left click walks the ladder up and wraps to 1x from its
  top step (`MAX_SIM_SPEED`), right click steps down; green while
  paused. It reads `Pointer<Click>` itself instead of `Activate`, which fires for *any*
  mouse button and would make one right click move both ways.
- **Tree style panel** (`ui/trees.rs`) — bottom-right: shape / foliage / crown details /
  color variance, one button per row cycling through a fixed palette (`bevy_ui` has no
  text input, so hex fields became cycles), plus **slider rows** built by the shared
  `ui/knob.rs::spawn_knob` kit — **density** over `TREE_DENSITY_MIN..MAX`,
  **conifer share** over `TREE_CONIFER_SHARE_MIN..MAX` and **noise mix** over
  `TREE_NOISE_MIX_MIN..MAX`. The kit's drag observer quantizes to the step and writes
  `TreeStyle` only when the step actually changes, so one drag rebuilds the crowns
  a handful of times, not once per pixel. The conifer-share and mix rows are
  `Display::None`ed outside `TreeShape::Mixed` (`sync_mixed_row_visibility`) — they mean
  nothing for the other shapes. Writes `TreeStyle`; `map::trees::rebuild_trees` picks the
  change up. Also settable over BRP: `res set TreeStyle .shape '"Conifer"'`.
- **Noise panel** (`ui/noise.rs`) — the conifer-field fbm knobs (`ConiferNoiseStyle`:
  wavelength / octaves / lacunarity / persistence), same knob kit; sits bottom-left
  **above the debug-toggles row** (the right column is already packed with style
  panels) and is `Display::None`ed while the `noise` debug toggle is off — tuning the
  field without the overlay showing it is pointless. Noise mix is deliberately *not*
  here: it is a gameplay look knob, so it sits in the Trees panel.
- **Navigation panel** (`ui/navigation/` — `mod.rs` the panel, `knobs.rs` the crowd
  knobs, `overlay.rs` the polymesh overlay) — slot 2 of the left column, always visible,
  **one UI for both pathfinding backends** (the Roads/Trees row-button idiom — label
  left, value right). The top row **`Algo`** cycles `Navmesh` ⇄ `Polymesh`: pawns always
  walk one of the two, so it is a choice, not two toggles that could both read `Off`
  while the grid quietly served every request. Its single source of truth is
  `PolymeshDebug::enabled`, which defaults to `Polymesh` — and which the `Separation`
  row below follows, since separation does not run on the grid backend (see the
  navigation-deep skill): picking `Navmesh` here greys that row out the way determinism
  does. Under it stand the settings **of the selected backend only** — the other set is
  `Display::None`d out of the layout (`sync_section_visibility`), because an agent radius
  means nothing while pawns walk tiles, and a grid search algorithm means nothing while
  they walk the mesh:
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
- **Slider kit** (`ui/slider.rs`) — the layer under the knob kit, and the one the crowd
  demo (`examples/demos/crowd_demo/`) still calls directly, since its sliders drive
  demo-local state rather than a resource: `spawn_slider_row` (label + value text +
  discrete `bevy_ui_widgets::Slider`), `quantize`, `apply_step` (quantize + put the thumb
  back on the stepped value), `retarget` (move the thumb when the value arrived past it —
  over BRP, from saved settings, from a lab preset), and one `sync_slider_thumbs` for all
  panels (sliders carry the shared `UiSlider` marker; registered once in `UiPlugin`).
- **Cycle rows** — the button half of the knob kit, for the values `bevy_ui` has no input
  field for: `spawn_cycle_row(.., CycleBinding { cycle, text })` where `cycle` advances the
  field by itself. Deliberately not "next item of `ALL`": what cycles is enums, plain
  bools, colour palettes and step tables alike, and forcing them through one list would
  need a type per kind. Registered by the same `add_knobs::<R>()` as the sliders.
  `CycleBinding` is *not* a component — the click observer captures it, and the sync only
  needs it on the label. The rows that stay hand-written are the Navigation panel's
  (`NavValueLabel`): their text is computed from **several** resources at once (`Backend`
  from `PolymeshDebug`, `Separation` from determinism + backend + style), which a binding
  to one resource cannot express — that enum is the right tool there, not a leftover.
- **Value-row kit** (`ui/rows.rs`) — the layer under cycle rows, and what the Navigation
  panel calls directly: `spawn_value_row` (grey label left, white value right, click on an observer),
  `row_color`, `next_in`, `on_off`, and one `highlight_value_rows` for every panel (rows
  carry the shared `ValueRow` marker; registered once in `UiPlugin`). A row whose click
  currently does nothing gets `RowInert` from its panel and stops highlighting — promising
  a reaction the click will not deliver is worse than not highlighting. The only carrier
  today is the Separation toggle under **Deterministic** or grid navigation.
  Highlighting writes through `set_if_neq`: the system runs every frame over every row of
  every panel, and at most one of them — the one under the cursor — actually changes.
- **Bottom UI columns** (`ui/mod.rs::stack_bottom_columns`, `UiRightColumn` /
  `UiLeftColumn`) — right: Tree rows → Trees → Buildings → Roads → hotkey help;
  left: debug toggles → Noise → Navigation; both bottom-up. **The order is the enum's
  declaration order**, not a number each panel writes for itself — with integers spread
  over seven files nothing caught two panels claiming one slot, or a gap. A settings
  panel takes its `Node` and its slot together from `right_panel` / `left_panel`; the
  debug-toggle row and the hotkey help keep their own nodes (a row, not a column, and
  their own paddings). The panels are absolute (`bevy_ui` does
  not stack them), and the columns change height at runtime (Trees grows two rows on
  `Mixed`, Noise exists only with the `noise` toggle), so each panel's `bottom` is the
  summed **measured** height of those below it instead of a hardcoded constant;
  `Display::None` panels are skipped by their `Node.display`, not their last-frame
  `ComputedNode`. `ComputedNode::size` is in *physical* pixels — multiply by
  `inverse_scale_factor` or every offset doubles on a retina screen. The arithmetic
  itself is `column_bottom`, a pure function over the visible panels — the four tests
  on it (bottom panel at the edge, clearing everything below, a hidden panel leaving no
  hole, a marked panel pushing itself *and everything above it* up) need no `App`.
  Panels sit flush by default — a column of map-style panels reads as one block. A
  **`UiPanelGapBelow`** marker inserts one gap under a panel, and the gap is
  `UI_SCREEN_EDGE_PX_OFFSET`, the same distance the UI keeps from the screen edge, so
  every space in the layout is the same width. Two panels carry it, both where the
  *kind* of UI changes: Navigation (the button row below it is not a panel) and the
  hotkey help (the panels below it are map settings).
- **Debug toggles** (`ui/debug/`) — grid / doors / movepath / noise buttons
  (`bevy_ui_widgets::Button` + `Activate` observers, `Hovered`/`Pressed` highlight). The
  navmesh overlay it still owns is **one merged mesh** — per-tile entities once cost 330 k
  entities; the noise overlay is one sprite with a CPU-built texture (see the osm-map
  skill's `references/trees.md`, Conifer stands). The
  *backend* settings — the grid overlay toggle, `pathfind:`, the agent radius — moved
  into the **Navigation panel** above, next to the other backend's settings; the row keeps
  the cycling buttons that are not about one backend's layer: **`camera:`** (start view)
  and **`navtile:`** (`NavtileBase`, 2 m ⇄ 1 m, reloads the world). Navtile is here and
  not under `Navmesh` because the world is *always* built in tiles of that size — the
  passability fill, the unreachable prune, the portal snap, the entrance generation —
  whichever backend the pawns then walk; hiding it with the grid settings would call a
  global setting a local one. A cycler goes green
  (`TOGGLE_ACTIVE_COLOR`, the same "on" colour as a toggle) while its resource equals
  `Default::default()` — `save`, `2m` — so a setting steered away from the baseline is
  visible at a glance; the check is against the `Default` impl, not a hardcoded variant, so
  moving `#[default]` moves the highlight with it. Label text and green-ness come from one
  `cycler_state` used by both the spawn and the sync system, so they cannot drift apart.
  The row closes with **`reset`** (`prefs::ResetSettings`) — every settings group back to
  its `Default`, world settings included, so an off-baseline city / seed / navtile /
  determinism reloads the map on the click. It sits here because the two cyclers beside it
  already answer half of "how far have I drifted from the baseline?". It is an **action**
  button, not a toggle and not a cycler: green in this row means "*this* resource is at its
  default", and the same green on a button that speaks about *other* resources would be a
  different claim — so it carries `ui::ActionButton` and only lights up under the cursor.
  That marker (and its one `highlight_action_buttons` in `UiPlugin`, the
  `sync_slider_thumbs` idiom) exists because `spawn_panel_button` paints the background
  once and every highlight system picks its buttons **by marker** — an unmarked button is
  inert under the cursor, which is what the World panel's `new` (seed reroll) had been.

## Camera start view

- **Camera start view** (`camera.rs`) — **`CameraPositionMode`** (`reset | save`, default
  `save`, the `camera:` button, persisted) decides where the camera stands when the world comes up:
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
  **`reset`** in the toggles row, which is the same thing from inside the game.

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
