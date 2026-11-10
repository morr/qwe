//! Сборка слоёв карты без мира — работа потока загрузки.
//!
//! Правило world-lifecycle «тяжёлая инициализация — в поток загрузки, а не в
//! `OnEnter(Playing)`» распространяется и на мешинг: слой-функции чистые
//! (`MapData → Vec<LayerMesh>`, `MaterialSpec` вместо хэндла ровно для этого),
//! поэтому поток собирает всё до входа в мир, а `OnEnter(Playing)` только
//! кладёт готовое. До выноса кадр перехода на Париже замирал на 15.7 с —
//! мешинг шёл на главном потоке при замершем экране загрузки (запись 10).
//!
//! Группы мешатся **параллельно, по потоку на группу**: здания, дороги,
//! полимеш и «мелкая» дорожка (поверхности, деревья, рельсы, машины). Карта
//! одна на всех и к началу дорожек финальна (гейты и прунинг уже отыграли,
//! состав деревьев собран до них — он один мутирует карту), списки у групп
//! свои — от раскладки по ядрам не зависит ничего.
//!
//! Режутся куски карты тоже здесь: разрез непрозрачных слоёв — та же работа,
//! что раньше стояла в спавне рядом со сборкой. На входе в мир остаётся
//! положить готовое.

use std::time::Instant;

use bevy::prelude::*;

use crate::map::buildings::{
    BuildingHeightMode, BuildingMeshes, BuildingPlan, BuildingReport, mesh_buildings,
};
use crate::map::cars::{CarPlacement, CarStyle};
use crate::map::osm::model::MapData;
use crate::map::osm::{JobState, MapLoadJob};
use crate::map::parking::ParkingLayout;
use crate::map::rail::{RAIL_LODS, RailReport, RailZoomBucket, mesh_rails};
use crate::map::roads::{RoadBuild, RoadStyle, VergeLods, VergeZoomBucket, build_roads};
use crate::map::spawn::{SurfaceReport, mesh_surfaces};
use crate::map::surface::{LayerMesh, MAP_CHUNK};
use crate::map::trees::{
    ConiferField, ConiferNoiseStyle, TreeMeshes, TreeReport, TreeRowStyle, TreeStyle,
};
use crate::map::zoom::ZoomLods;
use crate::navigation::PolymeshBuild;

/// Снимок ручек стилей, под которые поток мешит карту: у потока ECS нет, и
/// ресурсы ему недоступны. Снимок делается при старте потока; правка ручек
/// во время загрузки подхватывается обычной пересборкой (`rebuilds_on`) на
/// первом кадре мира — так же, как правка ручки после входа.
#[derive(Clone)]
pub struct StyleSnapshot {
    pub height_mode: BuildingHeightMode,
    pub road_style: RoadStyle,
    pub road_shape: crate::map::RoadShape,
    pub trees: TreeStyle,
    pub tree_rows: TreeRowStyle,
    pub noise: ConiferNoiseStyle,
    pub cars: CarStyle,
    /// Зум, с которым мир откроется (`camera::next_entry_zoom`): заборы
    /// мешатся под ступень этого зума. Прогноз разъехался с камерой — слой
    /// пересобирается на входе в кадре, как до переноса.
    pub fence_zoom: f32,
    /// Полимеш: радиус агента и тумблер чанков — если панель Navmesh
    /// включена. Выключенная панель и сегодня не строит ничего.
    pub polymesh: Option<(f32, bool)>,
}

/// Собранные слои карты — то, что поток загрузки отдаёт миру.
///
/// Списки лежат в текущем порядке спавна входа в мир; отчёты едут вместе со
/// списками — печатает их тот же спавн, что печатал их при сборке на месте.
/// Поток добавляет к этому свои строки-тайминги по группам.
#[derive(Default)]
pub struct WorldMeshes {
    /// Раскладка стоянок: вход сборки разметки и машин, ресурс мира.
    pub parking_layout: ParkingLayout,
    pub surfaces: Vec<LayerMesh>,
    pub surface_report: SurfaceReport,
    /// Дорожная сборка по ступеням обочин, в порядке ступеней: спавн кладёт
    /// обе в мир заранее, видима та, что под камеру (`show_road_steps`).
    pub roads: Vec<RoadBuild>,
    pub buildings: (BuildingMeshes, BuildingReport),
    pub trees: (TreeMeshes, TreeReport),
    /// Рельсы по ступеням зума, в порядке ступеней: все лежат в мире сразу,
    /// порог зума переключает видимость.
    pub rails: Vec<(Vec<LayerMesh>, RailReport)>,
    /// Расстановка машин с ключом, под который она считалась: на входе в мир
    /// ключ совпадает с ручками, и пересборка платит только меш кузовов.
    pub parked: CarPlacement,
    /// Заборы входной ступени — ступень, под которую собрано (зум входа из
    /// снимка), слои и отчёт; `None`, если разбор уже забрал. Сборка шага на
    /// большом городе стоит секунды (запись 17), и в кадре входа ей нечего
    /// делать. `rebuild_fences` берёт готовое, когда его ступень совпала, и
    /// строит в кадре, когда нет.
    pub fences: Option<(usize, Vec<LayerMesh>, crate::map::fences::FenceReport)>,
    /// Полимеш — `None`, если панель Navmesh выключена.
    pub polymesh: Option<PolymeshBuild>,
}

/// Готовые к спавну слои карты из потока загрузки. Вставляется `poll_job`
/// на `Done`, разбирается цепочкой `WorldInitSet::Spawn`: каждая система
/// забирает свои списки `std::mem::take` — то, что не взято, осталось бы
/// держать до следующей загрузки зря.
#[derive(Resource)]
pub struct PreparedMeshes(pub(crate) WorldMeshes);

/// Мешинг карты целиком — после финализации `MapData` (гейты, прунинг,
/// состав деревьев). Группы — по потоку; паника в группе — `Err`, чтобы
/// поток загрузки написал `Failed` и экран загрузки сказал об этом, а не
/// замер навсегда.
pub fn build_world_meshes(
    map: &mut MapData,
    styles: &StyleSnapshot,
    job: &MapLoadJob,
) -> Result<WorldMeshes, String> {
    let started = Instant::now();
    job.set(JobState::BuildingMeshes);

    // Ширина полосы — та, с которой разобран мир: ленты и краска читают
    // глобаль. `adopt_lane_width` на входе в мир пишет ровно то же число из
    // того же `MapData::knobs`.
    crate::map::roads::shape::set_lane_width(map.knobs.lane_width);

    // Состав деревьев — до дорожек: он один мутирует карту, дорожки читают
    // её целиком. Тот же проверка-и-сбор, что у `recompose_row_trees`: набор,
    // собранный разбором, с этими ручками совпадает — пересборки не будет.
    let compose = crate::map::trees::tree_compose(&styles.trees, &styles.tree_rows);
    if map.composed_for != Some(compose) {
        map.compose_trees(compose);
    }

    let map = &*map;
    let lanes = std::thread::scope(|scope| -> Result<_, String> {
        let buildings = scope.spawn(|| {
            timed("buildings", || {
                mesh_buildings(
                    BuildingPlan::game(styles.height_mode),
                    &map.buildings,
                    &map.roads,
                )
            })
        });
        // Ступени обочин — по потоку на ступень: сборки независимы (одна и та
        // же финальная карта, у каждой свои списки), а дорожная дорожка —
        // самая долгая группа; одной ступени хватает на стену. Порядок ступеней
        // держит сборка результатов по номеру ступени.
        let roads: Vec<_> = (0..VergeLods::max_zooms().count())
            .map(|step| {
                scope.spawn(move || {
                    timed(&format!("roads (verge step {step})"), || {
                        build_roads(
                            map,
                            styles.road_style,
                            styles.road_shape,
                            VergeZoomBucket::at(step),
                        )
                    })
                })
            })
            .collect();
        let small =
            scope.spawn(|| timed("surfaces, trees, rails, cars", || small_lane(map, styles)));
        let fences = scope.spawn(|| {
            timed("fences", || {
                crate::map::fences::entry_layers(styles.fence_zoom, &map.fences, &map.roads)
            })
        });
        let polymesh = scope.spawn(|| {
            timed("polymesh", || {
                styles.polymesh.and_then(|(radius, chunked)| {
                    crate::navigation::build_polymesh_for(map, radius, chunked)
                })
            })
        });

        let buildings = join(buildings, "buildings")?;
        let roads = roads
            .into_iter()
            .map(|lane| join(lane, "roads step"))
            .collect::<Result<Vec<_>, String>>()?;
        let small = join(small, "small layers")?;
        let fences = join(fences, "fences")?;
        let polymesh = join(polymesh, "polymesh")?;
        Ok((buildings, roads, small, fences, polymesh))
    })?;

    let (
        buildings,
        roads,
        (parking_layout, surfaces, surface_report, trees, rails, parked),
        (fence_step, fence_layers, fence_report),
        polymesh,
    ) = lanes;
    info!("map meshes: whole set in {:.1?}", started.elapsed());
    Ok(WorldMeshes {
        parking_layout,
        surfaces,
        surface_report,
        roads,
        buildings,
        trees,
        rails,
        parked,
        fences: Some((fence_step, fence_layers, fence_report)),
        polymesh,
    })
}

/// «Мелкая» дорожка: слои, чья сборка дешевле зданий и дорог, — на одном
/// потоке. Раскладка стоянок считается первой: по ней рисуется и разметка
/// мест, и ряды машин.
fn small_lane(
    map: &MapData,
    styles: &StyleSnapshot,
) -> (
    ParkingLayout,
    Vec<LayerMesh>,
    SurfaceReport,
    (TreeMeshes, TreeReport),
    Vec<(Vec<LayerMesh>, RailReport)>,
    CarPlacement,
) {
    let layout_started = Instant::now();
    let layout = ParkingLayout::new(&map.parking, &map.roads, map.traffic_side);
    info!(
        "parking layout: {} lots, {} stalls in {:.1?}",
        map.parking.len(),
        layout.0.iter().map(Vec::len).sum::<usize>(),
        layout_started.elapsed()
    );

    let (surfaces, surface_report) = mesh_surfaces(map, &layout);
    let surfaces = chunked(surfaces);

    // Поле хвои — по только что собранному набору деревьев: его же при входе
    // в мир пересемплирует ресурс поля (`build_conifer_field`), значения
    // совпадают — шум и примесь детерминированы позицией ствола.
    let mut field = ConiferField::default();
    field.resample(map.trees.positions(), &styles.noise, styles.trees.noise_mix);
    field.set_share(styles.trees.conifer_share);
    let trees = crate::map::trees::mesh_game_trees(&styles.trees, &map.trees, &field);

    // Все ступени рельсов разом: пересечение порога переключает видимость,
    // а не пересобирает слой.
    let rails = (0..RAIL_LODS.len())
        .map(|index| mesh_rails(RailZoomBucket::at(index), &map.rails))
        .collect();

    let parked = crate::map::cars::place(styles.cars, styles.road_shape, map, &layout);
    (layout, surfaces, surface_report, trees, rails, parked)
}

/// Непрозрачные слои — куски карты: тот же разрез, что делал спавн
/// (`spawn_layers` поверх сборки), только в потоке. Полупрозрачные слои и
/// краска режущий материал возвращает как есть — правило живёт в
/// `LayerMesh::into_chunks`.
fn chunked(layers: Vec<LayerMesh>) -> Vec<LayerMesh> {
    layers
        .into_iter()
        .flat_map(|layer| layer.into_chunks(MAP_CHUNK))
        .collect()
}

/// Замер группы — строка в лог с длительностью, значение — дальше.
fn timed<T>(what: &str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = work();
    info!("map meshes: {what} in {:.1?}", started.elapsed());
    result
}

/// Паника в группе — ошибка загрузки, а не замерший экран: поток с упавшей
/// сборкой больше ничего не напишет в состояние.
fn join<T>(lane: std::thread::ScopedJoinHandle<'_, T>, what: &str) -> Result<T, String> {
    lane.join().map_err(|_| format!("{what} meshing panicked"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> StyleSnapshot {
        StyleSnapshot {
            height_mode: BuildingHeightMode::default(),
            road_style: RoadStyle::default(),
            road_shape: crate::map::RoadShape::default(),
            trees: TreeStyle::default(),
            tree_rows: TreeRowStyle::default(),
            noise: ConiferNoiseStyle::default(),
            cars: CarStyle::default(),
            // стартовый зум камеры: та же ступень заборов, что у входа в мир
            fence_zoom: 0.4,
            polymesh: None,
        }
    }

    /// Пустая карта: групп столько, сколько ступеней, слоёв — ни одного,
    /// отчёты — про пустой вход. Проверяет и то, что сборка не паникует на
    /// пустоте: поток получает карту любой плотности.
    #[test]
    fn an_empty_map_makes_the_whole_set_of_groups_with_no_layers() {
        let mut map = MapData::default();
        let job = MapLoadJob::default();

        let meshes = build_world_meshes(&mut map, &snapshot(), &job).expect("meshing failed");

        assert_eq!(meshes.roads.len(), VergeLods::max_zooms().count());
        assert_eq!(meshes.rails.len(), RAIL_LODS.len());
        assert!(meshes.buildings.0.layers.is_empty());
        assert!(meshes.trees.0.crowns.is_empty() && meshes.trees.0.merged.is_empty());
        assert!(meshes.polymesh.is_none());
        // состав деревьев собран и зафиксирован: `recompose_row_trees` на
        // входе в мир увидит тот же состав и не будет пересобирать
        assert_eq!(
            map.composed_for,
            Some(crate::map::trees::tree_compose(
                &TreeStyle::default(),
                &TreeRowStyle::default(),
            ))
        );
    }

    /// Выключенная панель Navmesh не строит полимеш — включённая строит:
    /// поток обязан повторять решение панели, а не строить всегда.
    #[test]
    fn the_polymesh_lane_follows_the_panel_toggle() {
        let mut map = MapData::default();
        let job = MapLoadJob::default();
        let mut styles = snapshot();
        styles.polymesh = Some((0.4, true));

        let meshes = build_world_meshes(&mut map, &styles, &job).expect("meshing failed");

        assert!(meshes.polymesh.is_some(), "панель включена — меш построен");
    }
}
