# live-app: QWE specifics

Companion to the shared `live-app` skill (`.claude/skills/live-app/SKILL.md`), which
covers the mechanics of BRP and the `brp` CLI. This file is the inventory of *this* app:
what is registered, what actually drives time, how the world reloads. `.claude/live-app.json`
next to it configures `brp shot` / `brp speed` / `brp raise`.

Shorthand below: `b=.claude/skills/live-app/scripts/brp`.

## Launch: port 15703, always

```
Bash(command: "BRP_PORT=15703 cargo run 2>&1", run_in_background: true)
```

The user keeps his own qwe open on the default 15702 most of the time, and a
second app on the same port does not fail loudly — it just makes every `brp` call
answer from *his* window. `main.rs::brp_port` reads `BRP_PORT` (default 15702),
`.claude/live-app.json` points the client at 15703, and the port is in the window
title (`qwe :15703`) so the two windows are told apart on screen.

If 15703 is taken too — a parallel agent session in the same repo — the launch
panics on the first lines of the task output instead of running BRP-less:

```
BRP port 15703 is busy (Address already in use) — another qwe already holds it;
pick a free BRP_PORT
```

Then launch on 15704 and prefix every client call with the same port, because the
config still points at 15703: `BRP_PORT=15704 $b count Human`. A busy *default*
port only warns and disables BRP (`qwe (no brp)` in the title) — a second window
started by hand still runs.

```bash
$b alive     # alive: qwe 0.1.0 on http://127.0.0.1:15703/ (pid 40321) — pid must be the task's
$b procs     # both copies, with their ports and start times
```

Never `pkill -f target/debug/qwe` and never `brp quit` on 15702 — that kills the
user's session. Stop your own instance with `TaskStop`.

## Ready markers in the log

`brp wait` only proves the port answers — the map is still loading at that point. The
log goes through, in order:

```
Overpass: …                                    # only on a cold cache: HTTP download
osm map: 7475 buildings (…), 17 water, …       # parsing done
navmesh: filled in …                           # navtiles rasterised
navmesh: pruned 18391 unreachable … in …       # prune pass
warmup: pawns on screen routed in 1.23s        # <- Playing/Live: the world is running
```

`warmup: pawns on screen routed` (or `warmup timed out`) is the ready line — before it the
simulation is deliberately paused behind the loader screen. Waiting for it beats any fixed
sleep, especially after a city switch:

```bash
until grep -qE 'warmup: (pawns on screen routed|timed out)' $f; do sleep 2; done
```

## Time: `SimSpeed`, not `Time<Virtual>`

`sim_time.rs::throttle_speed_to_fps` writes `Time<Virtual>::set_relative_speed` every
frame from `SimSpeed.effective`, so a direct `Time<Virtual>` write survives one frame.
`brp speed N` is wired through `.claude/live-app.json` to `SimSpeed.requested` and prints
the resource back:

```bash
$b speed 15
# ok: SimSpeed.requested = 15
# {"actual": 7.41, "effective": 8.39, "requested": 15.0}
```

Three different numbers, all meaningful: `requested` is the ask, `effective` is what the
governor allows (`fps × MAX_FRAME_DELTA` — 15x at 60 fps, less on a loaded map), `actual`
is measured virtual-over-real. **Budget simulated time by `actual`**: 20 s of wall clock at
`actual=7` is ~140 s of simulation, not 300. `brp time` shows the `Time<Virtual>` side of
the same thing.

`brp pause` / `resume` still work directly (the governor only writes speed).

## Screenshots

```bash
$b cam 2284 1969        # frame the portal first
$b shot                 # raise window, trigger TakeScreenshotEvent, wait for the png
```

`TakeScreenshotEvent` (also F12) is in `dev.rs`; the file is `screenshot.png` in the repo
root, overwritten every time. The window **must be visible** — `shot` raises it, but a
full-screen editor over it still wins; a black png means exactly that, not a broken
renderer.

## Camera

- `brp cam <x> <y>` moves the camera and sticks — coordinates are map metres, the map is
  `MAP_SIZE = 5600 × 3700` with the origin at the bottom-left corner.
- **The `zoom` argument does not stick.** `PanCamera` (upstream `bevy_camera_controller`)
  re-applies its own `zoom_factor` to the transform scale every frame, and it derives
  `Component` without `Reflect` — so it is invisible to BRP and cannot be written. To
  actually zoom, feed the wheel and let `zoom_to_cursor` do it:

```bash
w=$($b window | grep -oE 'entity=[0-9]+' | cut -d= -f2)
$b hover 640 360 >/dev/null                     # zoom is cursor-anchored
$b msg MouseWheel "{\"unit\":\"Line\",\"x\":0.0,\"y\":-3.0,\"window\":$w,\"phase\":\"Moved\"}"
$b cam                                          # read the resulting zoom back
```

  Negative `y` zooms out. Range is clamped to `0.05 … 4.5`.

- Portal hint for Tula is `2284, 1969`; other cities put it at the map centre
  (`2800, 1850`) unless `city.rs` says otherwise.

## Registered types

Components / tags — `Human`, `Demon`, `Portal`, `Movable`, `SimPosition`,
`PreviousSimPosition`, `CorpseTag`, `TestWalker`, `PathfindingRequest`, `ChaseTarget`,
`ChaseRepath`, `FleeRepath`, `DevourUntil`, `WanderHeading`, `WanderPause`,
`MovableStateMovingTag`, `HumanWanderTag`, `HumanFirstWanderTag`, `HumanFleeTag`,
`DemonWanderTag`, `DemonChaseTag`, `DemonLungeTag`, `DemonDevourTag`.

Resources — `City`, `SimSpeed`, `Telemetry`, `PortalPos`, `PathfindingAlgorithm`,
`BuildingHeightMode`, `TreeStyle` / `TreeShape`, `TreeRowStyle`, `ConiferNoiseStyle`,
`DrawMovePaths`, `DebugGrid`, `DebugNavmesh`, `DebugDoors`.

Events — `TakeScreenshotEvent`, `SpawnTestWalkerEvent`, `RestartEvent`.

Anything not in this list is invisible to `get` / `res get` until it gets
`#[derive(Reflect)]` + `#[reflect(Component)]`/`#[reflect(Resource)]` + `register_type`.

## Everyday queries

```bash
$b count Human Demon CorpseTag              # the workhorse
$b res get Telemetry                        # {"escaped": 0, "killed": 220}
$b texts pathfinding                        # "pathfinding: 259 in flight, 19268 queued, 0.16 ms avg\nentities: 23272"
$b with Portal --ids
$b count HumanFleeTag DemonChaseTag DemonDevourTag MovableStateMovingTag
```

The telemetry invariant is worth checking after any behaviour change —
`killed + escaped + alive == HUMAN_COUNT` (CONTEXT.md). **Pause first** (`$b pause`): at
high sim speed the two BRP reads land in different ticks and the sum looks broken when it
isn't.

The `texts pathfinding` overlay is the cheapest perf read there is — in-flight and queued
requests plus the average search time, already computed by the app.

## Toggles and modes

```bash
$b res set DebugDoors .0 true         # tuple structs: field .0
$b res set DebugGrid .0 true
$b res set DebugNavmesh .0 true
$b res set DrawMovePaths .0 true
$b res set BuildingHeightMode . '"Shadows"'      # whole resource: path `.`
$b res set TreeStyle .shape '"Conifer"'
$b res set TreeStyle .woods false                 # источники деревьев: лес
$b res set TreeStyle .standalone false            # …одиночные natural=tree
$b res set TreeRowStyle .enabled false            # …аллеи (панель Tree rows)
$b res set PathfindingAlgorithm . '"Hpa"'
```

Hotkey equivalents in the app: `R` restart, `G` gizmos (doors + movepath), `N` navmesh,
`M` movepath, `Space` pause, `=`/`-` speed. The bottom-left panel has the same toggles as
buttons, the bottom-centre one switches city — find them with `brp texts` / `brp ui-at`
rather than hardcoding pixels, the layout moves.

## Restart and city switch

```bash
$b event RestartEvent                      # same as R: respawn population, reset Telemetry
$b res set City . '"Paris"'                # full reload: Playing -> Loading -> Playing
```

A city switch re-downloads the Overpass extract on a cold cache (slow, network) and
re-runs navmesh fill + prune, so wait for the ready marker above, not a fixed sleep. It
also exercises the `DespawnOnExit(AppState::Playing)` rule — a
`world reload: N scene entities survived Playing` warning in the log means a spawn site is
missing the component (see CLAUDE.md).
