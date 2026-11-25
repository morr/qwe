# Context-diet inventory — problem files, measured

This is an **inventory, not the refactor plan**. It lists the files whose size makes
agent sessions expensive, with the measurements that implicate them. The detailed
refactor plan — what splits into what, in which order, with which migration of
cross-references — is written by the agent that executes this work, starting from
this list.

## Status — 2026-10-30, updated 2026-11-22

Executed since the inventory was written: the roads/parse/reference splits and the
first CONTEXT.md slimming (8ee63d30); the skills water pass with osm-map and
ui-panels SKILL.md back under the 30K one-Read line (f55e9652, 419c58d3); the module
splits of `osm/model.rs` (b6386635), `parking.rs` (4bab1cdc), `meshing.rs`
(dde7d19e) and `inspect.rs` (71d92498) into the `foo.rs` + `foo/` subject pattern;
the `buildings/material.rs` → `material/kinds.rs` (26d156f7) and
`buildings/clutter.rs` → `clutter/{placement,tests}.rs` splits; the `roofs.rs`
split into `roofs/gable.rs` and the `temples.rs` split into
`temples/{domes,towers}.rs` (2026-11-22, same pattern); and a Reading map
at the top of CONTEXT.md (load the map + the touched section, not
the file).

The 2026-11-22 wave executed the whole candidate list of the split report
(`/tmp/qwe-split-report-2026-11-22.md`; coordinator + worker sessions in
`.worktrees/`, pure code motion, `tools/check.sh` green per commit): `roads.rs` →
`roads/{common,raw,build}.rs` (0f46e407), `roads/tests.rs` → per-subject
`roads/tests/` (041000eb), `meshing.rs` impl spread → `meshing/{walls,chunks}.rs`
(92ac6eec) and `meshing/ribbon.rs` → `ribbon/{profile,geometry}.rs` (16de83d4),
`buildings/layers.rs` → `layers/{walls,garages,roofing}.rs` (a0e3ccc3),
`buildings/tests.rs` → per-subject `tests/` (598a3fcf), `osm/parse.rs` →
`parse/{ways,relations,finish,elements}.rs` (7c6641d9), `cars/mod.rs` →
`cars/street.rs` (19dd5f3c), `human/systems.rs` →
`systems/{spawn,wander,tests}.rs` and `human/lod.rs` tests → `lod/tests.rs`
(0afb6725, b273c1fa), `sim_time.rs` → `sim_time/regulator.rs` (f561d9ed),
`trees.rs` → `trees/mesh.rs` + conifer fns and `trees/tests.rs` → per-subject
(2035deb6, a8228ed2), `footprint.rs` → `footprint/{fences,decks}.rs` + tests
(a2380da9, 2fd92428) — the material/clutter/roofs/temples splits of the same
wave are the ones named above. The one candidate judged not-to-split:
`roads/node_paint/cluster.rs` — its mass is a single ~620-line `paint_cluster`;
a file split would scatter one mechanism.

Remaining above the ~40K line — every file across `src/**`, `examples/**` and
`.claude/skills/**`, measured `wc -c` on 2026-11-22 after the wave, not a top
selection:

- 60K+ (5): `roads/corners/tests.rs` 73.9K, `roads/gores.rs` 67.1K,
  `roads/corners/gather.rs` 67.1K, `roads/node_paint/cluster.rs` 64.7K,
  `buildings/temples.rs` 61.8K.
- 50–60K (12): `roads/medians.rs` 60.8K, `roads/rings.rs` 60.3K, `roads/bridges.rs`
  60.0K, `buildings/garages.rs` 59.5K, `examples/demos/roads/main.rs` 58.0K,
  `roads/scraps.rs` 56.9K, `roads/node_paint/tests.rs` 55.3K,
  `roads/bridges/tests.rs` 55.0K, `trees/crown.rs` 54.8K, `camera.rs` 53.9K,
  `meshing/tests.rs` 52.2K, `roads/paint/tests.rs` 52.1K.
- 40–50K (29): `roads/common.rs` 49.9K, `ui/inspector.rs` 49.7K,
  `movement/tests.rs` 48.9K, `osm/parse/tags.rs` 48.9K, `buildings/mod.rs` 48.6K,
  `roads/paint/painter.rs` 48.3K, `roads/rings/tests.rs` 48.2K,
  `human/corpse.rs` 47.3K, `roads/node_paint.rs` 46.1K, `osm/parse/pockets.rs`
  46.0K, `silhouette/mod.rs` 45.5K, `movement/pathfinding.rs` 45.2K,
  `settings.rs` 45.0K, `navigation/polymesh/stitch.rs` 44.2K,
  `examples/demos/roof_gallery/main.rs` 43.8K, `roads/network/pairs/tests.rs`
  43.6K, `parking.rs` 43.4K, `osm/parse/lots.rs` 42.8K, `loading.rs` 42.6K,
  `dev.rs` 42.3K, `meshing.rs` 42.2K, `movement/destination.rs` 42.1K,
  `examples/demos/wall_gallery/main.rs` 42.1K, `roads/axis.rs` 41.8K,
  `demon/behavior.rs` 41.2K, `roads.rs` 40.8K, `buildings/roofs.rs` 40.5K,
  `examples/demos/crowd_demo/metrics.rs` 40.5K, `roads/network/pairs.rs` 40.2K.

Skills (`.claude/skills/**`, the real directory — `.zcode/skills` is a symlink
to it, and a scan along the symlink's name descends nowhere, which is how the
2026-11-22 pass missed these): one file above the line,
`.claude/skills/osm-map/references/layers.md` (43.5K); the next,
`osm-map/references/parse-geometry.md`, sits exactly on it (40.0K). The three
engine skills inside (`bevy`, `git`, `live-app`) are symlinks into zxc — not
this repo's files.

`CONTEXT.md` (137.3K whole-file) stays out of this list by design: the Reading
map above is its fix, and the success criterion measures it by section, not by
whole file. One section is itself still over that line: «OSM map pipeline»
(58.0K), which the reading map still makes a map session load whole. Splitting
it is next-phase work — the cut line is a glossary design decision, not a blind
split.

## Why: the measured cost model

One autonomous session (roads work, S1–S7 list, 2026-10-11/12) grew its context from
40K to 570K tokens over 409 model calls and burned **114.6M tokens, 99.6% of them
cache re-reads** of accumulated context. The content it actually added — tool results,
its own output — totalled ~530K tokens; the rest was the same context paid for 409
times. Context grows ~1.3K tokens per turn here, and every file that enters it is
re-paid by every following turn. The three biggest levers, in order:

1. auto-compact never fired (fixed at the zclaude level: window 250K);
2. four giant files were re-read in full 11–13 times each;
3. one agent ran the whole work list instead of one agent per item.

Lever 2 and the per-session floor of lever 1 are what this repo can fix structurally —
by making the big files small enough that a session never needs them whole.

## The problem files, measured

Sizes in characters (÷4 ≈ tokens). "reads" = full `Read` calls in that one measured
session; a file read N times adds its size N times to the context of every following
turn of a session.

| File | Size | reads | Why it is a problem |
|---|---|---|---|
| `.claude/skills/osm-map/references/roads.md` | 260.8K (~65K tok) | 13 | The single heaviest re-read. Loaded by the `osm-map` skill for any `map/roads/*` work; carries several distinct subjects (sidewalks, markings/breaks, junction paint, kerb returns, gores, bridges, the `roads` gallery) that a given session usually needs one or two of. |
| `CONTEXT.md` | 201.8K (~50K tok) | — (CLAUDE.md-mandated) | Fixed per-session floor: "load it before touching any domain or simulation code". Not re-read often, but every session pays ~50K tokens before the first edit, and it re-enters after every compaction. |
| `src/map/roads/tests.rs` | 171.8K (~43K tok) | 11 | Tests for the whole roads module in one file; a session fixing one junction behaviour scrolls/reads all of it to find its contexts. |
| `src/map/roads.rs` | 157.7K (~39K tok) | 5 | The module root carries more than wiring — its size suggests logic that belongs to the subject files under `src/map/roads/`. |
| `src/map/roads/corners.rs` | 153.7K (~38K tok) | 12 | The junction/corner generation in one file; the S7/N1/N6 fixes of the measured session all lived here, with whole-file re-reads between attempts. |
| `src/map/meshing.rs` | 131.7K (~33K tok) | 4 | Whole-pipeline file (parse → mesh → render); road-meshing sessions read it whole. |
| `.claude/skills/osm-map/references/buildings.md` | 123.9K | — | Same pattern as roads.md, not yet implicated by a measured session — same fix will apply eventually. |
| `src/map/roads/node_paint.rs` | 116.4K (~29K tok) | 13 | Junction paint rules; the most re-read source file of the measured session. |
| `.claude/skills/osm-map/SKILL.md` | 80.7K | — | The skill's map itself exceeds the ~30K guideline; every `map/*` session loads it before any reference. |
| `.claude/skills/osm-map/references/parking.md` | 91.8K | — | Same pattern as roads.md. |

Also in the family, below the top: `src/map/roads/paint.rs` (105.1K),
`references/parse.md` (74.2K), `src/map/roads/medians.rs` (72.1K),
`src/map/roads/gores.rs` (65.4K), `src/map/roads/bridges.rs` (53.9K).

## Constraints the refactor plan must respect

- **The skill gating moves with the paths.** `.claude/hooks/require-skill.sh` gates
  `src/map/**` behind `osm-map`; `CLAUDE.md`'s skill table and the `osm-map` skill
  text describe what each reference carries. Any split of a reference or a module
  updates those pointers in the same change — the "agent-facing docs in the same
  change" rule of CLAUDE.md applies in both tiers.
- **`CONTEXT.md` is a glossary, not a dump** (its own words): terms and invariants
  only. Its 201.8K suggest it has drifted past that charter; the plan should consider
  a tiered reading path (a short index section a session can stop at) rather than a
  blind split — glossary cross-references are load-bearing.
- **Tests split along their subject**, not mechanically by size: `tests.rs` covering
  the roads module splits where its `describe`/module boundaries already are, and
  `guard-bash.sh`/`check.sh` invocations keep working.
- **Behavior is frozen during the move**: a split commit changes no rules, no
  constants, no formats — `tools/check.sh` stays green per commit, and the measured
  performance verdict (the per-task performance check in CLAUDE.md) is "no change:
  pure code motion".

## Success criteria

- No agent-facing file (skill, reference, CONTEXT.md section a session must load)
  exceeds ~40K characters, and each is readable in one `Read` call of its own.
- A roads-fix session can load only the subject file it needs (junction paint, or
  markings, or kerb) — measured as: a fresh session's fixed floor before the first
  edit drops from ~90K+ tokens (CLAUDE.md + CONTEXT.md + osm-map + roads.md) toward
  ~30–40K.
- The full `Read` count of any one file in a work session stays in single digits
  (vs 13 in the measured session), with re-reads going through `offset`/`limit`.
