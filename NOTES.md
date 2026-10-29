# NOTES — improve-architecture-sizes

Worktree for the remaining menu of the split-giants plan (qwe-split-giants-plan).
Rules as in phase A: pure code movement, check.sh green per commit, test names
invariant (baseline: 1517 tests, /tmp/qwe-tests-baseline.txt), interface paths
frozen via root re-exports, docs-twin in the same change.

Lessons so far: pre-commit check must be `cargo check --all-targets` (lib-test
consumers via `use super::*` are invisible to a plain check); test-consumer
names that rode the root scope come back as `#[cfg(test)]` re-exports in the
root (turning_radius, RoadNodeKind, Fortresses); never edit files while a
check.sh is running.

## Stage a — phase B: spread build_roads inline blocks

Wave 1 done (nested fns → subject files, bodies unchanged):
bece5505 merge edges → merges.rs; bfd60b95 kerb pockets → pockets.rs
(+POCKET_OVERLAP); 6753ef0c+55e7118a turning circles → junctions.rs
(+turning_radius/TURNING_CIRCLE_*, test re-exports fix); a5eb1925 seam dash
gaps → bridges.rs; d3a30f45 ring island bands + cover → rings.rs; 030f64b7
slit aprons → aprons.rs; 1e547f17 scrap phases → scraps.rs; 2bb2e6ab junction
paint → node_paint.rs; ff437221 tram bands → tram_band.rs (+DECK_CUT_SLACK;
TRAM_BAND_COLOR stays in root — tests read it there); 95cace3c wall ribbons →
walls.rs (+Fortresses test re-export).

Wave 2 (inline families → named subject fns, same captured values as params):
bcdfe21a merge ramps → merges.rs; 58ef8e29 plaza/road-island fills →
islands.rs (`lay_areas` returns RoadIslands + Bites); f23cdac1 ring approach
flags → rings.rs; 5adac231 footbridge flares → footbridge.rs (`lay_flares`);
4d01d53d verge side + bites → verges.rs::street_verged + scraps Bites::
on_points; 7c892c10 taper wedges → tapers.rs (wedge_run/push_asphalt/
push_sidewalks, #[allow(too_many_arguments)] per repo idiom; bf39d589 restored
the allowance that drifted onto street_verged); 88ae2ea5 deck seam aprons →
Bridges::push_seam_aprons.

**Stage a closed.** roads.rs 117.9K → 86.9K (2280 → 1688 lines): 12 nested
phases and 7 inline families live in their subject files; the main ribbon
loop, medians wiring and layer assembly stay in build_roads per its
docstring contract. check.sh green on every commit; test list after stage a
identical to baseline (1517 names). Perf verdict: pure code movement, nothing
new per frame or per load.

Remaining wave-2 residue (kept deliberately): bridge curb_wedges computation
inside the deck branch, the medians wiring, the loop's seam/trim wiring —
shared-state wiring per the contract.

## Stage b — CONTEXT.md diet — done (18941d2e)

docs/context-diet-plan.md is an inventory; the execution model: tiered glossary,
not a blind split. CONTEXT.md 213.5K → 107.5K: the "OSM map pipeline" section
156.5K → ~50K (every entry compressed to term + invariants + pointer to its
osm-map reference; all bold cross-reference titles preserved). The head and
tail sections were already at the charter's 5–10-line budget and stay.
Mechanism text was verified to live in the skill references before cutting
(spot-checked: beige, contact skirt, chequerboard, balconies_fit, boxless,
straighten_tails, green_aprons, brand band, MIN_ROW_RUN, fan_head).
check.sh green (note: never pass a .md to check.sh — its fmt step runs rustfmt
on it; run it with no args for docs-only commits).

## Stage c — dense subject files >60K

Done so far (all check.sh-green, test lists identical to baseline):
- db1c4a23 gores mouths → gores/mouths.rs (10.5K), gores.rs 66.9K → 50.3K
- 2859dfe1 gores splitters → gores/splitters.rs (7.8K)
- 1c150904 tags buildings family → tags/buildings.rs (488 lines), tags.rs
  75.2K → ~44K. Re-exports at tags root keep parse consumers' paths frozen;
  building_height/colour re-exports are #[cfg(test)] (only parse/tests.rs
  reads them). BUILDING_HEIGHT_RANGE stayed in tags root (structure_height
  reads it too); buildings.rs reaches it via `use super::`.
- cdb8dade lots tests → lots/tests.rs; lots.rs 68.8K → 42.8K (pockets
  precedent: tests in a child file, path unchanged)
- 1d8982b6 pairs.rs 89.5K → 40.2K + pairs/search.rs (Поиск: new, push_run,
  beside, Tracks, RunKind) + pairs/align.rs (Общая ось: align,
  follow_moved_nodes, seam machinery); pub consts stayed root, children
  `use super::` them; pairable re-exported `pub use self::search::pairable`
- a6726da3 paint.rs 115K → 27.8K hub + paint/{frames,spans,ground,material,
  painter}.rs. Root keeps shared consts, RoadPaintStyle, LineKind/LineBreaks,
  ArrowMarks+ARROW consts (node_paint reads the fields), MergeRamp (private
  methods used by painter), axis_offset, Station, PaintTag. paint/tests.rs got
  its own import block for the moved names; Painter fields pub(super).
- 1092fc67 node_paint.rs 128.2K → 43.3K + node_paint/{cluster,zebras,ends}.rs.
  cluster.rs holds paint_cluster (~45K — one closure environment, unsplittable
  without redesign; 60.2K, right at the limit) + edge machinery + Context;
  zebras.rs plank joins; ends.rs spills/short runs. NodePaint fields are
  root-private and children reach them (descendant visibility).

Commit messages: "split X into ... subjects" / "move ... tests into ...".
Sizes verified with wc -c after each split.

Remaining in stage c:
- parse/tests.rs 175.7K — c4, mapped at 1092fc67: 109 tests, families beside
  their passes: rails 94-196; verges 1578-1701 + probes 1520-1577; sidewalks
  1177-1269 + infer 1378-1433 + helpers 1358-1377 + 1423 (infer_sidewalks
  lives in sidewalks.rs); pavements 1270-1357; faiths 2286-2572; squaring
  2616-2892 + 3481-3542 + 3784-3857; house_pull 2893-3062 + 3594-3650;
  block_pull 3063-3186 + 3234-3360 + 3651-3783 + 3906-4067; tags family:
  measures 844-858, tags() helper 859-868 → parse/fixtures.rs (16 uses),
  path width 1019-1051, heights 1747-1978, kremlin/use/shop 2052-2214,
  fortress 2573-2615, colours 3361-3413, WESTERN_CHURCH 26-32 (only
  building_use test reads it); lots parse-level 358-428 → lots/tests.rs.
  Line numbers valid at 1092fc67 — recheck after each cut. After the moves,
  tests.rs keeps seam/trees/areas/roads-core/entrances (~100K) — split
  further into tests/ children by subject if it stays >60K.

Then stage d (#[ignore] diagnostics ~530 lines in roads tests — plan
recommendation: leave as is, only the author decides), then per user:
rebase onto master, merge the branch, remove the worktree.

Handoff facts: branch improve-architecture-sizes at 1092fc67 (32 commits
ahead of master 828a6853), worktree
.claude/worktrees/improve-architecture-sizes, target/ seeded, check.sh green
throughout, test list identical to baseline (/tmp/qwe-baseline-sorted.txt is
the sorted raw-format baseline, 1531 lines with `: test` suffix). Artifact
plan updated for stage a only
(https://artifacts.spacehubstore.com/a/qwe/qwe-split-giants-plan — NEW url;
the update endpoint on the legacy name 409s at any rev, worked around by
rename to -prev + save under project "qwe"; stage b/c updates still pending).

Lessons (see top): --all-targets before commit; test-consumer names come back
as cfg(test) re-exports; never edit while check.sh runs; never pass .md to
check.sh; `tags.rs` + `tags/` subdir coexist — mkdir before writing a child
module file; python line-split indexing is 0-based, file line N is lines[N-1]
(count the boundaries, don't trust memory); rustfmt runs at the END of
check.sh even when the build failed — re-check the file before patching a
failed split, multi-line replace patterns die on the reflowed text; the
recurring failure is a range that fell out of ALL files — the compiler catches
it, recover the block from `git show HEAD:<file>`; guard-bash denies a whole
command if any part pipes cargo — run the python patch and the check as
separate calls.
## Stage d — #[ignore] diagnostics fate

Left as is. Both are DIAG-TEMP harnesses driven by env vars (QWE_DIAG_JSON=...
in roads/tests.rs and roads/drawn/tests.rs) — author tooling, not rot. The
plan's recommendation stands: only the author decides.

## Rebase and merge — DONE via squash

master crossed the branch with bevy 0.20 / i_overlay 9 / rand 0.10 and doc
retargets (13 overlapping files). A per-commit rebase would resolve the same
API conflicts 35 times, so the branch was squashed onto master instead:
- improve-architecture-sizes-pre-rebase keeps all 35 original commits;
- improve-architecture-sizes = master 09d8bec0 + one squash commit (8ee63d30)
  + the API port (80486a4e: i_overlay mesh paths moved under mesh::float,
  Material2d::specialize takes the pipeline);
- check.sh OK after a cold 0.20 build: 1490 passed + 3 ignored, and the bare
  test names are identical to master's own list (1545 = 1545), so the squash
  lost nothing from master's new inspector and osm-id tests.
