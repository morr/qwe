# qwe

A 2D demon-invasion simulation prototype on **Bevy 0.19**: the Tula city centre generated
from OpenStreetMap data, 20 000 wandering humans, demons spawning from a portal.

```bash
cargo run
```

The first run downloads the OSM extract from Overpass into `assets/osm/` (gitignored
cache); later runs are offline. Deleting the cache file forces a re-download.

Domain glossary and invariants live in [`CONTEXT.md`](CONTEXT.md), the OSM coverage audit
in [`.claude/skills/osm-map/references/osm-coverage.md`](.claude/skills/osm-map/references/osm-coverage.md),
working conventions in [`CLAUDE.md`](CLAUDE.md).

## Examples

Examples are grouped by purpose. Names are unchanged by the grouping, so
`cargo run --example <name>` works regardless of the directory:

| directory | what it is |
|---|---|
| `examples/demos/` | **windowed scenes to run and watch** |
| `examples/acceptance/` | headless pass/fail runs (print `OK` / `FAILED`, exit code) |
| `examples/bench/` | offline timing measurements |
| `examples/audit/` | offline geometry reports |

### Separation demo — the crowd anti-overlap scene

```bash
cargo run --example crowd_demo
```

A crowd on an empty map, close up, with the real `movement/separation/` running on it —
no OSM download, no UI panels. It exists because separation only works inside the
viewport, below zoom `SEPARATION_MAX_ZOOM` and once per rendered frame, so the interesting
cases (a funnel, counter-flowing columns, a walled corridor, high sim speed) otherwise
have to be *waited for* in the game. Here each is one keystroke away, with a live count of
overlapping pairs next to it.

Every pawn is drawn as its sprite plus a gizmo circle of its **real body radius** —
green when the pair distance is kept, red on a genuine overlap, with a line to the
offender. The circle matters: sprite edges touching is not the same as bodies overlapping.

| key | |
|---|---|
| `1` … `5` | scenario: pile / funnel / counter-flowing columns / walled corridor / real wander AI |
| `R` | rebuild the current scenario |
| `S` | separation on/off — the A/B that shows what it is worth |
| `Space` | pause |
| `-` `=` | simulation speed, 1 … 30× |
| wheel | zoom (past 0.75 separation switches off by design — the run counter stops) |

The same numbers go to stdout every two seconds, and BRP is up on its own port
(`BRP_PORT`, 15704 by default — *not* the game's 15702), so scenarios can be driven from
outside:

```bash
b=.claude/skills/live-app/scripts/brp
BRP_PORT=15704 $b res set Scenario . '"Funnel"'
BRP_PORT=15704 $b res set DemoSpeed .0 20
BRP_PORT=15704 $b res get Overlaps
```

Two things to know before trusting any measurement of separation: **count only pawns
inside the camera rect** — off-screen pawns are never separated by design, and including
them makes on/off indistinguishable — and **allow a millimetre tail**, because the solver
is soft and a converged crowd still reports pairs a few mm inside the radius sum.

Demos never touch the game's config: no `PrefsPlugin` / `SettingsPlugin` / `CameraPlugin`
/ `DevPlugin` / `MapPlugin`, so nothing can read or overwrite
`settings.toml`. Each demo keeps its own knobs in its own file.
