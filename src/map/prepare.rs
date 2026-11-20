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
//! заборы, полимеш и «мелкая» дорожка (поверхности, деревья, рельсы,
//! машины). Карта одна на всех и к началу дорожек финальна (гейты и прунинг
//! уже отыграли, состав деревьев собран до них — он один мутирует карту),
//! списки у групп свои — от раскладки по ядрам не зависит ничего.
//!
//! Режутся куски карты тоже здесь: разрез непрозрачных слоёв — та же работа,
//! что раньше стояла в спавне рядом со сборкой. На входе в мир остаётся
//! положить готовое.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use bevy::prelude::*;

use crate::city::City;
use crate::map::buildings::{
    BuildingHeightMode, BuildingMeshes, BuildingPlan, BuildingReport, mesh_buildings,
};
use crate::map::cars::{CarPlacement, CarStyle};
use crate::map::inspect::InspectIndex;
use crate::map::osm::model::MapData;
use crate::map::osm::{JobState, LoadedWorld, MapLoadJob};
use crate::map::parking::ParkingLayout;
use crate::map::rail::{RAIL_LODS, RailReport, RailZoomBucket, mesh_rails};
use crate::map::roads::{
    RoadStepsBuild, RoadStyle, VergeLods, VergeZoomBucket, build_roads_steps, road_skeleton,
};
use crate::map::spawn::{SurfaceReport, mesh_surfaces};
use crate::map::surface::{LayerMesh, MAP_CHUNK};
use crate::map::trees::{
    ConiferField, ConiferNoiseStyle, TreeMeshes, TreeReport, TreeRowStyle, TreeStyle,
};
use crate::map::zoom::ZoomLods;
use crate::navigation::{Navmesh, PolymeshBuild, snap_portal_position};

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
    /// Дорожная сборка сплит-двери (`build_roads_steps`): общие слои — один
    /// набор на город (спавн кладёт их без `LodStep`, ступень обочин их не
    /// прячет), ступенезависимые — в порядке ступеней, обе в мире заранее,
    /// видима та, что под камеру (`show_road_steps`).
    pub roads: RoadStepsBuild,
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

/// Мешинг карты целиком после финализации `MapData` (гейты, прунинг, состав
/// деревьев) — **серийный референс пина эквивалентности, только тесты**:
/// продовая загрузка ходит перекрытым путём ([`build_overlapped_world`]),
/// здесь группы идут по потоку подряд, чтобы пин сравнивал расписание, а не
/// тела дорожек. Паника в группе — `Err`, как у перекрытых лейнов.
#[cfg(test)]
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
        let buildings = scope.spawn(|| timed("buildings", || buildings_lane(map, styles)));
        let roads = scope.spawn(|| timed("roads", || roads_lane(map, styles)));
        let small =
            scope.spawn(|| timed("surfaces, trees, rails, cars", || small_lane(map, styles)));
        let fences = scope.spawn(|| timed("fences", || fences_lane(map, styles)));
        let polymesh = scope.spawn(|| timed("polymesh", || polymesh_lane(map, styles)));

        let buildings = join(buildings, "buildings")?;
        let roads = join(roads, "roads")?;
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
    let layout = ParkingLayout::new(map);
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
    crate::map::trees::resample_field(&mut field, &map.trees, &styles.trees, &styles.noise);
    let trees = crate::map::trees::mesh_game_trees(&styles.trees, &map.trees, &field);

    // Все ступени рельсов разом: пересечение порога переключает видимость,
    // а не пересобирает слой.
    let rails = (0..RAIL_LODS.len())
        .map(|index| mesh_rails(RailZoomBucket::at(index), &map.rails))
        .collect();

    let parked = crate::map::cars::place(styles.cars, styles.road_shape, map, &layout);
    (layout, surfaces, surface_report, trees, rails, parked)
}

// Тела дорожек — общие перекрытой загрузке ([`build_overlapped_world`]) и её
// серийному тестовому референсу ([`build_world_meshes`]): обе цепочки
// дорожек запускают одни и те же функции над `&MapData`, различается только
// расписание вокруг них.

/// Лейн зданий.
fn buildings_lane(map: &MapData, styles: &StyleSnapshot) -> (BuildingMeshes, BuildingReport) {
    mesh_buildings(
        BuildingPlan::game(styles.height_mode),
        &map.buildings,
        &map.roads,
    )
}

/// Лейн дорог: ступени обочин — над общим скелетом, и общая середина сборки
/// у них одна (`build_roads_steps`): скелет сети, устья, разделительные,
/// ленты, краска узлов — у ступеней бит-в-бит одинаковы (отпечаток записи
/// 28), считаются один раз; ступенезависимые обочинные куски и лоскуты каждая
/// достраивает свои. Один дорожный поток вместо двух — второй финал больше
/// не молотит дубли общей середины под стеной.
fn roads_lane(map: &MapData, styles: &StyleSnapshot) -> RoadStepsBuild {
    let skeleton = road_skeleton(map, &styles.road_style, &styles.road_shape);
    let steps: Vec<VergeZoomBucket> = (0..VergeLods::max_zooms().count())
        .map(VergeZoomBucket::at)
        .collect();
    build_roads_steps(&skeleton, &steps)
}

/// Лейн заборов входной ступени: ступень по зуму, с которым мир откроется.
fn fences_lane(
    map: &MapData,
    styles: &StyleSnapshot,
) -> (usize, Vec<LayerMesh>, crate::map::fences::FenceReport) {
    crate::map::fences::entry_layers(styles.fence_zoom, &map.fences, &map.roads)
}

/// Лейн полимеша: включённая панель Navmesh строит его по карте, выключенная
/// не строит ничего.
fn polymesh_lane(map: &MapData, styles: &StyleSnapshot) -> Option<PolymeshBuild> {
    styles
        .polymesh
        .and_then(|(radius, chunked)| crate::navigation::build_polymesh_for(map, radius, chunked))
}

/// Заливка сетки по карте и снап портала — голова навмеш-дорожки перекрытой
/// загрузки. Лок навмеша уже взят вызывающим: его ожидание он меряет сам и
/// передаёт сюда для той же строки лога.
pub(crate) fn fill_navmesh_and_snap_portal(
    navmesh: &mut Navmesh,
    map: &MapData,
    city: City,
    lock_waited: Duration,
) -> Vec2 {
    let started = Instant::now();
    navmesh.fill_from_mapdata(map);
    info!(
        "navmesh filled in {:?} (write lock waited {:?})",
        started.elapsed(),
        lock_waited
    );

    let hint = city.portal_hint();
    match snap_portal_position(navmesh, hint) {
        Some(position) => {
            if position != hint {
                info!("portal snapped {hint:?} => {position:?}");
            }
            position
        }
        None => {
            warn!("no clear spot for portal near {hint:?}");
            hint
        }
    }
}

/// Единственная форма загрузки: навмеш-дорожка (заливка → снап → калитки)
/// идёт рядом с лейнами мешинга, вторая волна (прунинг ∥ полимеш ∥ заборы)
/// стартует присоединением дорожки калиток, и лейн полимеша получает клон
/// карты с открытыми калитками сразу после шага калиток, не дожидаясь
/// прунинга. Сериальный префикс (заливка, калитки, прунинг — ~0.4 с тёплого
/// входа Парижа) прячется под стену дорожной сборки целиком (запись 66:
/// −0.39 с на виде владельца, −0.27 с на Туле).
///
/// До записи 66 путь включался только на скрытой ступени заборов входного
/// зума: лейн заборов был дорог сам по себе (2.3–2.8 с Парижа), и его старт
/// не должен был сдвигаться гонкой дорожек за ядра (урок прогона №12).
/// Запись 63 срезала лейн заборов до ~1.0 с, стеной стал лейн дорог, и
/// гейт потерял основание — лейн заборов уехал во вторую волну (он читает
/// проёмы оград через `fence_gaps` — калитки и дорожные пересечения, —
/// ему нужен клон с уже открытыми калитками).
///
/// Карта читается лейнами напрямую: навмеш работает на клоне, и его
/// единственная мутация карты — калитки в `fences` — сливается в карту мира
/// после всех дорожек. Порядок «калитки до прунинга» (ограда, отрезавшая
/// участок с дверями, открывается раньше, чем его выбросят) держится и здесь:
/// прунинг стартует присоединением дорожки калиток.
pub fn build_overlapped_world(
    job: &MapLoadJob,
    mut map: MapData,
    navmesh: &RwLock<Navmesh>,
    city: City,
    styles: &StyleSnapshot,
) -> Result<LoadedWorld, String> {
    let started = Instant::now();
    job.set(JobState::BuildingNavmesh);

    // Состав деревьев — до дорожек, серийно: он один мутирует карту, а
    // перекрытая загрузка начинает лейны раньше готовности навмеша. Та же
    // проверка-и-сбор, что у `build_world_meshes` и `recompose_row_trees`.
    let compose = crate::map::trees::tree_compose(&styles.trees, &styles.tree_rows);
    if map.composed_for != Some(compose) {
        map.compose_trees(compose);
    }
    // та же глобаль, что лейнам: карманные контуры индекса отвечают той же
    // узловой осью, что рисунок
    crate::map::roads::shape::set_lane_width(map.knobs.lane_width);

    let map_ref = &map;
    let (meshes, portal, inspect, gate_lists) = std::thread::scope(
        |scope| -> Result<(WorldMeshes, Vec2, InspectIndex, Vec<Vec<Vec2>>), String> {
            // Первая волна: лейны, не читающие калиток.
            let buildings = scope.spawn(|| timed("buildings", || buildings_lane(map_ref, styles)));
            let roads = scope.spawn(|| timed("roads", || roads_lane(map_ref, styles)));
            let small = scope.spawn(|| {
                timed("surfaces, trees, rails, cars", || {
                    small_lane(map_ref, styles)
                })
            });
            let inspect = scope.spawn(|| {
                let started = Instant::now();
                let index = InspectIndex::prebuild(map_ref, &styles.road_shape);
                info!(
                    "map meshes: inspect index base in {:.1?}",
                    started.elapsed()
                );
                index
            });

            // Навмеш-дорожка: заливка, снап и калитки — на клоне карты под
            // локом навмеша. Клон она увозит с собой: его забирает лейн
            // полимеша (второй читатель калиток) и слияние после дорожек.
            let gates_lane = scope.spawn(|| {
                timed("navmesh fill and gates", || {
                    let mut scratch = map_ref.clone();
                    let lock_since = Instant::now();
                    // толерантно к отравлению: упавшая прошлая загрузка травит
                    // лок навсегда, а перезаливка с нуля переписывает всё
                    let mut nav = navmesh
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let portal = fill_navmesh_and_snap_portal(
                        &mut nav,
                        &scratch,
                        city,
                        lock_since.elapsed(),
                    );
                    let gates_started = Instant::now();
                    let gates = nav.open_sealed_fences(&mut scratch, portal);
                    (scratch, portal, gates, gates_started.elapsed())
                })
            });
            let (scratch, portal, gates, gates_took) = join(gates_lane, "navmesh")?;
            info!("navmesh: opened {gates} fence gates in {gates_took:?}");
            job.set(JobState::Pruning);

            // Списки калиток забираются из клона сразу: слиянию они нужны
            // после всех дорожек, каким бы ни был исход лейнов.
            let gate_lists: Vec<Vec<Vec2>> = scratch
                .fences
                .iter()
                .map(|fence| fence.gates.clone())
                .collect();

            // Вторая волна: прунинг карту не читает — лок навмеша хватает, —
            // полимеш и лейн заборов строится по клону с открытыми калитками
            // параллельно с ним. Заборный лейн читает проёмы оград
            // (`fence_gaps` — калитки и дорожные пересечения), серийный путь
            // строил его по карте с уже слитыми калитками; свой клон клона —
            // мегабайты и миллисекунды против стены лейнов, а move-входы
            // вторая волна держит раздельными (клона и портала больше никто
            // не читает).
            let prune_lane = scope.spawn(move || {
                let started = Instant::now();
                let mut nav = navmesh
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let pruned = nav.prune_unreachable(portal);
                nav.drop_gate_scratch();
                (pruned, started.elapsed())
            });
            let fence_map = scratch.clone();
            let fences = scope.spawn(move || timed("fences", || fences_lane(&fence_map, styles)));
            let polymesh =
                scope.spawn(move || timed("polymesh", || polymesh_lane(&scratch, styles)));

            let (pruned, prune_time) = join(prune_lane, "prune")?;
            info!("navmesh: pruned {pruned} unreachable tiles in {prune_time:?}");
            job.set(JobState::BuildingMeshes);
            let polymesh = join(polymesh, "polymesh")?;
            let fences = join(fences, "fences")?;

            let buildings = join(buildings, "buildings")?;
            let roads = join(roads, "roads")?;
            let small = join(small, "small layers")?;
            let inspect = join(inspect, "inspect index")?;

            let (parking_layout, surfaces, surface_report, trees, rails, parked) = small;
            let (fence_step, fence_layers, fence_report) = fences;
            let meshes = WorldMeshes {
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
            };
            Ok((meshes, portal, inspect, gate_lists))
        },
    )?;

    // Слияние калиток в карту мира — единственное, что навмеш-дорожка
    // поменяла в клоне (`open_sealed_fences` пишет только
    // `fences[..].gates`): мир получает те же калитки в ресурсе `MapData`,
    // что и построенный по клону полимеш.
    for (fence, gates) in map.fences.iter_mut().zip(gate_lists) {
        fence.gates = gates;
    }

    info!("map meshes: whole set in {:.1?}", started.elapsed());
    Ok(LoadedWorld {
        map,
        portal,
        meshes,
        inspect,
    })
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

        assert_eq!(meshes.roads.steps.len(), VergeLods::max_zooms().count());
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

    /// Скрытая ступень лестницы заборов: на последней черте и дальше слой
    /// скрыт (нулевая ширина ступени), до неё — рисуется.
    #[test]
    fn the_entry_fence_step_is_hidden_past_the_last_ladder_bound() {
        assert!(!crate::map::fences::entry_step_hidden(0.4));
        assert!(!crate::map::fences::entry_step_hidden(0.89));
        assert!(crate::map::fences::entry_step_hidden(0.9));
        assert!(crate::map::fences::entry_step_hidden(2.0));
    }

    /// Фикстура калиток (по форме
    /// `gates/tests.rs::a_sealed_plot_gets_its_gate_on_the_street_side`):
    /// замкнутая ограда участка, дом с дверью внутри и улица вдоль северной
    /// стороны — `open_sealed_fences` открывает непустую калитку, так что
    /// слияние перекрытой загрузки проверяется не на пустом множестве.
    fn gated_plot_map() -> MapData {
        use crate::map::osm::fixture::{building, fence, rect, street};

        let plot = Vec2::new(300.0, 300.0);
        let corners = [
            (-20.0, -20.0),
            (20.0, -20.0),
            (20.0, 20.0),
            (-20.0, 20.0),
            (-20.0, -20.0),
        ];
        let mut map = MapData::default();
        map.fences.push(fence(
            corners
                .iter()
                .map(|&(x, y)| plot + Vec2::new(x, y))
                .collect(),
        ));
        let mut house = building(
            rect(
                Vec2::new(plot.x - 5.0, plot.y - 5.0),
                Vec2::new(plot.x + 5.0, plot.y + 5.0),
            ),
            vec![],
        );
        house.entrances.push(Vec2::new(plot.x, plot.y - 5.0));
        map.buildings.push(house);
        map.roads.push(street(
            vec![
                Vec2::new(plot.x - 80.0, plot.y + 30.0),
                Vec2::new(plot.x + 80.0, plot.y + 30.0),
            ],
            8.0,
        ));
        map
    }

    /// Перекрытая загрузка даёт тот же мир, что серийная: сетка навмеша и
    /// калитки в карте бит-в-бит, полимеш построен по тем же калиткам (одни
    /// препятствия), на видимой ступени заборов — и слой заборов бит-в-бит
    /// (серийный путь строил его по карте с уже слитыми калитками, перекрытый
    /// — по клону второй волны). Обе стороны с включённой панелью полимеша —
    /// иначе его лейн ничего не строит и передача калиток дорожкой
    /// остаётся непроверенной.
    #[test]
    fn the_overlapped_load_matches_the_serial_one() {
        for fence_zoom in [0.4, 1.0] {
            let styles = {
                let mut styles = snapshot();
                styles.fence_zoom = fence_zoom;
                styles.polymesh = Some((0.4, true));
                styles
            };
            let job = MapLoadJob::default();

            // серийный путь: заливка → снап → калитки с прунингом, потом лейны
            let mut serial_map = gated_plot_map();
            let mut serial_nav = Navmesh::default();
            let serial_portal = fill_navmesh_and_snap_portal(
                &mut serial_nav,
                &serial_map,
                City::Tula,
                Duration::ZERO,
            );
            serial_nav.open_gates_and_prune(&mut serial_map, serial_portal, |_, _| {});
            let serial_meshes =
                build_world_meshes(&mut serial_map, &styles, &job).expect("serial meshing failed");

            // перекрытый путь
            let overlapped_nav = RwLock::new(Navmesh::default());
            let world = build_overlapped_world(
                &job,
                gated_plot_map(),
                &overlapped_nav,
                City::Tula,
                &styles,
            )
            .expect("overlapped load failed");

            // сетка навмеша бит-в-бит
            let overlapped = overlapped_nav.read().unwrap();
            assert_eq!(overlapped.grid_size, serial_nav.grid_size);
            assert_eq!(overlapped.tile_size, serial_nav.tile_size);
            for y in 0..serial_nav.grid_size.y {
                for x in 0..serial_nav.grid_size.x {
                    assert_eq!(
                        overlapped.is_passable(x, y),
                        serial_nav.is_passable(x, y),
                        "тайл ({x}, {y})"
                    );
                }
            }

            // калитки бит-в-бит и непустые: иначе слияние проверено впустую
            let gates_of = |map: &MapData| -> Vec<Vec<Vec2>> {
                map.fences.iter().map(|fence| fence.gates.clone()).collect()
            };
            let serial_gates = gates_of(&serial_map);
            let overlapped_gates = gates_of(&world.map);
            assert_eq!(serial_gates, overlapped_gates);
            assert!(
                serial_gates.iter().any(|gates| !gates.is_empty()),
                "фикстура обязана открыть калитку"
            );

            // портал совпал, полимеш — одни препятствия из одних калиток
            assert_eq!(world.portal, serial_portal);
            let serial_polymesh = serial_meshes.polymesh.expect("панель включена");
            let overlapped_polymesh = world.meshes.polymesh.expect("панель включена");
            assert_eq!(overlapped_polymesh.obstacles, serial_polymesh.obstacles);

            // слой заборов — бит-в-бит (на скрытой ступени обе стороны пусты:
            // сборка выходит рано, до всякого чтения проёмов)
            let serial_fence = serial_meshes.fences.as_ref().expect("слой заборов");
            let overlapped_fence = world.meshes.fences.as_ref().expect("слой заборов");
            assert_eq!(
                overlapped_fence.1.len(),
                serial_fence.1.len(),
                "число слоёв заборов, zoom {fence_zoom}"
            );
            for (serial_layer, overlapped_layer) in serial_fence.1.iter().zip(&overlapped_fence.1) {
                assert_eq!(
                    overlapped_layer.builder.positions_for_test(),
                    serial_layer.builder.positions_for_test(),
                    "вершины слоя заборов бит-в-бит, zoom {fence_zoom}"
                );
            }
        }
    }
}
