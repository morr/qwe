# SWEEP-LOG — механический заход по «Обзору №8» (dead pub surface), 2026-11-20

Ветка `perf24` (база master e8de0c63), worktree `.worktrees/perf24`.

Метод: каждый из 89 item-ов списка «Обзора №8» (docs/deepening-plan.md) перемерен
заново — grep с границами слова по `src/`, `examples/`, `tests/` минус файл
определения. Правила: потребители вне своего файла → остаётся `pub`; используется
только в своём файле → `pub(crate)`; только свои тесты → `#[cfg(test)]`; мёртв
вовсе → удалить. Однострочные замены, поведение не менялось: 87 строк в 42 файлах
(44 файла в проверке; `paint/material.rs` и `city.rs` сошлись к нулю), плюс 5 строк
переноса сигнатуры от rustfmt в `paint/frames.rs`.

Первая проверка поймала четыре пункта, где затяжка ругается компилятором
(`private_interfaces`/E0446): у PaintKey и CityLabel носитель обязан остаться `pub`
(его зовут витрины отдельным крейтом) — оба item-а вернулись в `pub`; у
BuildingLayerTag/CarLayerTag носители — `rebuild_buildings`/`rebuild_cars`,
затянуты вместе с ними (регистрация в `map/mod.rs` — тот же крейт).

Итог: **затянуто до `pub(crate)` 78 item-ов списка + 9 сопроводительных носителей** —
pub-функций и констант, чья сигнатура несёт затянутый тип (иначе ругается
`private_interfaces`); **осталось `pub` 11** (у всех нашлись реальные потребители
вне своего файла — измерение №8 их не видело), **`#[cfg(test)]` — 0, удалений — 0**
(мёртвых item-ов нет: у каждого нашёлся обычный потребитель в своём файле).

## Остались `pub` (потребитель вне файла)

| item | потребитель |
|---|---|
| `water.rs::OpenChannels` | `map/spawn.rs` через `split_channels`/`mesh_water_lines` |
| `footprint.rs::FenceGap` | `navigation/polymesh/build.rs`, `navigation/navmesh/fill.rs`, `examples/audit/fence_prune_audit` через `fence_gaps`/`gaps`/`fence_pieces` |
| `roads/pockets.rs::KerbPocketOutline` | `map/inspect.rs` через `kerb_pocket_outlines` |
| `roads/node_paint.rs::PaintPockets` | `map/roads.rs`, `roads/ribbon_lane.rs` через `pockets()` |
| `roads/node_paint.rs::PaintMarks` | `map/roads.rs`, тесты `medians`/`merges`/`rings` через `marks()` |
| `roads.rs::RoadSkeleton` | `map/prepare.rs` через `road_skeleton`/`build_roads_steps` |
| `tram.rs::TramReport` | несётся `pub fn mesh_tram`, который зовёт `examples/demos/roads` |
| `trees.rs::CrownVariant` | `examples/demos/tree_gallery` через `crown_variant` |
| `movement/separation/ports.rs::SeparationReader` | `movement/systems.rs` потребляет значение через `reader()` |
| `roads/paint/material.rs::PaintKey` | несёт `#[bind_group_data]`-impl `Material2d` для `pub PaintMaterial`, который зовут витрины (`temple_gallery`, `roads`) |
| `ui/city.rs::CityLabel` | несётся `pub fn sync_city_label`, ре-экспортом `qwe::ui` идёт в `examples/demos/roads` |

## Затянуто до `pub(crate)`

Формат: файл — item-ы. «→ носитель» — сопроводительное затяжка pub-функции/константы,
чья сигнатура несёт затянутый тип (иначе ругается `private_interfaces`).

- `sim_time.rs` — SIM_TICK_DEBT_CAP, SIM_LOAD_SMOOTHING, SPEED_PROBE_TIME, SPEED_LADDER,
  ACTUAL_SPEED_WINDOW, tick_probe, TickSpikeThreshold
- `portal.rs` — Portal, PortalMaterial
- `dev.rs` — TakeScreenshotEvent, SpawnTestWalkerEvent, LayerProbe, VirtualKey
- `camera.rs` — cursor_offset
- `gpu_cut.rs` — GpuSpikeThreshold
- `ui/inspector.rs` — InspectorHighlight
- `ui/brp.rs` — BrpBadge
- `ui/knob.rs` — CycleValueLabel (поле осталось `pub`)
- `ui/debug/mod.rs` — DebugToggle, AddDebugToggleExt
- `determinism/replay.rs` — TICK, fingerprint
- `rng.rs` — seed_for (свои тесты в `rng/tests.rs` продолжают видеть: тот же крейт)
- `silhouette/mod.rs` — Glyph::COUNT, Silhouette::drawn_size, resize_silhouettes_on_zoom
- `human/look.rs` — PANIC_COLOR; `human/decide.rs` — ESCAPE_MARGIN;
  `human/corpse.rs` — CORPSE_HEADINGS
- `movement/pathfinding.rs` — priority::WANDER_ON_SCREEN;
  `movement/separation/tuning.rs` — SEPARATION_HOLD
- `map/fences.rs` — FenceLod, FenceLayerTag → носитель FENCE_LODS
- `map/rail.rs` — RailPalette, RailTieLod, RailSteelLod, RailDashLod, RailLod → носитель RAIL_LODS
- `map/grid.rs` — MAX_INSERT_CELLS
- `map/surface.rs` — SURFACE_TEXTURE_DEFAULT, FlatMaterials
- `map/meshing.rs` — STOREY_STRIDE
- `map/trees.rs` — TREE_DENSITY_DEFAULT, TREE_CONIFER_SHARE_DEFAULT, TreeLod → носитель TREE_LODS
- `map/spawn.rs` — TreeRowBandTag
- `map/wagons.rs` — WagonLayerTag, WagonReport → носитель mesh_wagons
- `map/z.rs` — Y_SORT_FACTOR, Z_UNIT_BASE
- `map/tram.rs` — TramTieLod, TramLod, TramLayerTag → носитель TRAM_LODS
- `map/industry.rs` — IndustryLayerTag, IndustryReport → носитель mesh_industry
- `map/buildings/material.rs` — RoofKind::CODES, WallKind::CODES (оба impl-блока)
- `map/buildings/mod.rs` — BuildingLayerTag, BuildingShadowTag → носитель rebuild_buildings
  (регистрируется в `map/mod.rs`, тот же крейт)
- `map/trees/conifer.rs` — CONIFER_NOISE_OCTAVES, CONIFER_NOISE_LACUNARITY,
  CONIFER_NOISE_PERSISTENCE, CONIFER_NOISE_SEED
- `map/osm/parse.rs` — skips_passes; `map/meshing/ribbon.rs` — push_inset_band_with
- `map/cars/mod.rs` — CarLayerTag, mesh_parked → носитель rebuild_cars
  (регистрируется в `map/mod.rs`, тот же крейт)
- `map/roads/merges.rs` — MergeBand → носитель merge_bands (зовут только свои тесты)
- `map/roads/node_paint.rs` — CLUSTER_ZONE; `roads/junctions.rs` — is_one_road;
  `roads/turns.rs` — ARROW_MIN_LANES;
  `roads/paint/frames.rs` — seam_origin; `roads/network/sections.rs` — SPIKE_MAX_LENGTH,
  default_lanes; `roads/ruts.rs` — lane_centres; `roads/tram_band.rs` — band_cover,
  deck_cuts, split_cover

Проверка: `tools/check.sh` по всем 44 файлам — `check: OK` (хвост вывода в отчёте).
