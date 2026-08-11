---
name: sim-speed
description: Use when working on simulation speed, pause, or the speed regulator in qwe — sim_time.rs, SimSpeed/SimLoad/TickDebt, the pipeline ceiling, the frame-budget guard, SimClock, the sim/*_ms diagnostics. Deep detail behind CONTEXT.md's speed summary.
---

# Simulation speed — the regulator

Detail behind `sim_time.rs`. The summary (speed ladder, the 30× cap, SimClock) lives in
`CONTEXT.md`. Remember the project constraint: **30× is the deliberate product cap**
(ladder 1-2-5-10-20-30); the optimization target is bigger populations, not a higher
top speed.

- **Speed ladder** — Space pauses, `=`/`-` walk `SPEED_LADDER`
  (1 → 2 → 5 → 10 → 20 → 30; the button's `cycle_time_scale` wraps to 1x from the top
  step; an arbitrary BRP-written speed snaps to the nearest step on the next press).
- **SimSpeed** — `{requested, pipeline, affordable, effective, actual}`. `requested` is
  what the ladder says and is the only field a new run keeps: the `WorldStarted`
  observer (`on_world_started`) resets `pipeline`/`affordable` to `MAX_SIM_SPEED`,
  `effective` to `requested` and `SimLoad` to unmeasured on every restart and city
  switch — the backoff and the smoothed tick cost belong to the previous run (after a
  city switch, to a different world entirely), and inheriting them started the new
  world throttled by measurements taken about the old one. Starting too high is safe:
  the regulator steps down instantly and `guard_frame_budget` caps the frame
  structurally. Pinned by `a_new_run_inherits_no_clock_and_no_regulator_memory`.
  `pipeline` is the pathfinding-pipeline ceiling, the one
  regulator value with memory (see below); `affordable` is what the regulator computed
  the machine can carry (already `min`-ed with `pipeline`);
  `effective` is its command, what reaches `Time<Virtual>`; `actual` is measured —
  virtual seconds per real second, averaged over `ACTUAL_SPEED_WINDOW` (0.5 s of *real*
  time, so long frames weigh what they cost). `actual` is the only honest one: Bevy
  clips a frame's virtual delta at `max_delta`, so a stall eats simulated time behind
  the regulator's back. The panel and `is_throttled` read `actual`.
- **SimLoad — what one tick costs, split in two.** `begin_sim_load` / `end_sim_load`
  bracket the fixed loop (`RunFixedMainLoopSystems::BeforeFixedMainLoop` /
  `AfterFixedMainLoop`) and divide the wall time of the frame's `FixedUpdate` run by the
  `SimTick` delta over the same bracket, smoothed with `SIM_LOAD_SMOOTHING` (0.5 s of
  real time, so the filter does not change with the frame rate). `SimTick` zeroes on
  restart and city switch, so the delta is a `saturating_sub` and a frame whose counter
  went backwards is skipped.
  **The split is the point.** `tick_ms` is CPU work — a property of the world, not of
  the speed, since speed changes how many steps a frame runs and not what a step costs.
  `wait_ms` is per-frame time inside the fixed loop that is not per-tick work — chiefly
  the main thread standing in `block_on` waiting for the pathfinding pool
  (`apply_pathfinding_results` reports it through `SimLoad::add_frame_cost`) — and that
  one depends on the speed directly: the answer's deadline is measured in **ticks**
  (`PATHFINDING_RETIRE_TICKS`), so faster ticks give the pool less real time for the
  same work. Blending the two into one number closes the regulator on a quantity it
  controls itself — measured live as tick cost swinging 2.9…7.6 ms with a 2–4 s period
  and the speed following it 1.3…3.4×. `wait_peak_ms` is the **peak-hold** companion of
  `wait_ms`: it jumps to any raw sample at once and decays toward the mean over
  `SIM_LOAD_PEAK_DECAY` (3 s) — bursts arrive in packs seconds apart, and the mean
  dilutes them before the pipeline ceiling can answer. Published as `sim/tick_ms`,
  `sim/wait_ms` and `sim/wait_peak_ms`, all on the panel's third line
  (`tick 1.20 + 3.50 ms wait (pk 8.10)`).
- **The regulator solves where it can, integrates where it cannot.** Two independent
  bounds, the smaller wins.
  *By CPU it solves*: a frame of length `d` carries `d × S × 64` ticks, so allowing the
  simulation `SIM_FRAME_SHARE` of any frame gives `S = 1000 × share / (64 × tick_ms)` —
  `d` cancels, so the answer does not depend on which frame just happened, on vsync
  quantisation, or on history.
  *By pipeline it integrates* (`pipeline_limit`, state in `SimSpeed::pipeline`): the
  pathfinding pool is a queue, and near saturation the wait grows as `1/(1 − load)` —
  unbounded gain, so any one-step formula overshoots (measured live: wait swinging
  0.9…8.0 ms/tick with a ~1 s period over a flat CPU cost). Instead the ceiling steps
  against the busy-per-tick ratio (`tick_ms + wait_peak_ms` vs the share's per-tick
  allowance), **proportionally to the overrun** — 10 % over cuts 10 % per
  `SPEED_BACKOFF_TIME` (1 s), double cuts half; a constant step keeps cutting while the
  queue drains and makes its own sawtooth (measured: 1.1…1.8× with a ~2 s period).
  Probing back up runs on `SPEED_PROBE_TIME` (6 s). Sizing to the **peak** wait, not
  the mean, is a deliberate speed-for-smoothness trade.
  `SIM_FRAME_SHARE` is derived, not tuned: `1 − SIM_RENDER_BUDGET × MIN_SIM_FPS`
  (13 ms per frame reserved for everything that is not simulation → 0.61). Frames then
  settle at `rest / (1 − share)` — a contraction with gain `share < 1`, stable by
  construction. Applied asymmetrically: **down at once, up by doubling every
  `SPEED_CLIMB_DOUBLE_TIME` (0.75 s)**, with a **down-only** `SPEED_DEADBAND` (2 %). The
  climb limit is the one thing the solver cannot compute — tick cost lags the speed,
  because a speed-up spawns path requests whose cost lands a second or two later. The
  band belongs on the down side alone, where the move is instantaneous and measurement
  noise would otherwise ratchet the speed down; the climb is already rate-limited, so a
  band there only stopped it 2 % short of the request — a world permanently running at
  0.98x.
  On top of the solved target, `frame_overrun` breaks the "long frame carries more ticks
  carries a longer frame" self-amplification — but it measures the overrun on a frame
  **reassembled from what the simulation is accountable for**: its own fixed-loop run
  (`SimLoad::frame_sim_ms`, raw and unsmoothed) plus the reserved `SIM_RENDER_BUDGET`,
  against `1/MIN_SIM_FPS`. Not on the real frame length: the loop's gain *is* the
  simulation's share of the frame, so charging it for the whole frame charges it for
  other people's work, and a frame it did not stretch does not get shorter by slowing
  it. Unity while the run fits `SIM_FRAME_BUDGET_MS`; past that, exactly the factor the
  frame length would have given had the simulation filled it. `dt` drops out of the
  answer entirely, vsync quantisation with it — same property the solved branch has.
  The cost of the old length-based version was measured at world start: the first frames
  run 100–270 ms on one-off work (spawning 20 000 humans, uploading the merged city
  meshes, first GPU pipeline compilation) while ticks take single-digit percent of them;
  the regulator slammed `effective` onto the `MIN_SIM_SPEED` floor with `affordable`
  sitting at 8–18x, and the world then crawled back up by doublings for ~3 s — the
  "0.1 → 0.5 → 1.0" ramp visible on the speed button right after start. The correction
  now fires rarely by design: the frame-budget guard below already cuts the run at
  `SIM_FRAME_BUDGET_MS`, so the runaway is severed structurally and `frame_overrun` is
  the second line, for the frame whose last tick carried it past the budget.
  Floored at `MIN_SIM_SPEED` (0.1). The button shows `15x → 8.6x` when limited.
  Entering `PlayPhase::Live` resets `effective` to `requested` (`resume_simulation`):
  whatever the regulator computed over loading, spawn and warm-up frames describes a
  simulation that was paused, and after a city switch belongs to the previous world.
  Starting too high costs one frame — the down move is instantaneous.
- **The frame-budget guard** (`guard_frame_budget`, first system of `FixedUpdate`) is
  the hard backstop behind all of the above: the regulator aims at `SIM_FRAME_BUDGET_MS`
  (share × target frame ≈ 20 ms) from **smoothed** measurements, and a burst lands
  before any filter learns of it. Once a frame's fixed-loop run has eaten the budget,
  the guard strips the remaining `Time<Fixed>` overstep — the loop stops after the
  current tick — and books it into **TickDebt**, which `begin_sim_load` returns to the
  accumulator next frame (capped at `SIM_TICK_DEBT_CAP`, beyond which time is honestly
  dropped, same philosophy as `max_delta`; held while paused; zeroed on restart and
  city switch). Deferred ticks do not change game logic — how many ticks share a render
  frame floats anyway — a burst shows as a brief `actual` dip instead of a visible
  hitch. `TickDebt.deferred` (BRP-readable) counts everything ever deferred, so a live
  check can see the guard actually firing.
- **Why fps is not the feedback signal**, though the goal is stated in frames. The
  window is `PresentMode::AutoVsync`, so measured fps only takes values `refresh/n`:
  while a frame fits in 16.7 ms the reading is flat 60 and says nothing about the
  remaining headroom — the regulator learns about an overload only after it has already
  overshot. Deriving the render cost as `frame − sim` does not rescue it either: under
  vsync that difference contains the sleep, and it grows exactly as the speed is cut,
  which drives a limit cycle between the 60 and 30 steps. Both numbers are fine to show
  and unfit to close a loop on. Two earlier attempts tuned this loop's coefficients
  (smoothing, then a hysteresis band) without touching that; the sawtooth survived both.
- **`MAX_FRAME_DELTA` is not a speed ceiling**, and reading it as one is the mistake
  this loop was built on for a while. `Time<Virtual>::max_delta` clamps the **raw**
  frame delta, *before* the speed multiplies it
  (`bevy_time/src/virt.rs::advance_with_raw_delta`) — it is "the longest real frame we
  still count in full", and its only job is to stop a freeze from becoming an avalanche
  of ticks. `Time<Fixed>` has no per-frame step limit of its own
  (`bevy_time/src/fixed.rs::expend` runs while `overstep` allows), so this constant is
  the only thing between a long frame and `max_delta × S × 64` ticks inside it — at 0.5 s
  and 10× that was 320. A long frame carries more virtual time, hence more ticks, hence
  a longer frame still: the pit was self-sustaining and exactly `max_delta` deep. Hence
  **0.25 s** (Bevy's own default). The cost of the trade is that a frame longer than
  that silently hands the simulation less time than really passed, visible only in
  `actual`.
- **Requested cap** — `MAX_SIM_SPEED` (30x, the top of `SPEED_LADDER`) is a hard
  ceiling on `requested`: a deliberate product limit, not a hardware one. The ladder
  never steps past it, and `throttle_speed_to_frame_budget` clamps `requested` itself so
  a BRP write cannot exceed it either.
- Set the requested speed over BRP with `res set SimSpeed .requested N` (clamped to
  `MAX_SIM_SPEED`) — `brp speed` writes `Time<Virtual>` directly and the throttle
  overwrites it on the next frame.
- **SimClock** — `elapsed`, virtual seconds the *current world* has lived, zeroed
  (together with `TickDebt.owed`) by the `WorldStarted` observer — i.e. on `Live` entry
  and on every restart, one system for both paths (so map load and warmup don't count,
  and a city switch restarts it). Not wall-clock: it stops on pause and runs `actual`×
  faster on speedup.
  The panel's first line shows it as plain seconds (`T+8130`), and it is readable
  over BRP as `SimClock`.
- **Per-tick cost** (`sim/*_ms` diagnostics, 20 000 humans / 100 demons): with the
  entity-only incremental grid and the inverted `panic` the tick sums to ~0.1 ms —
  `flee` ~0.06 > `panic` ~0.02 > `move` ~0.01 > `spatial` (demon rebuild) ~0.004.
  History: `panic` once scanned the demon grid per wandering human (~0.8 ms/tick, the
  speed ceiling); a `DemonDangerMap` boolean prefilter cut it to ~0.15, and the
  inversion replaced the map entirely. At full zoom-out (every pawn moving) the sim
  stays ~0.2 ms/tick — the limiter there is rendering 20k sprites, not the sim.
