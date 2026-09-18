//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (земля, кварталы,
//! парки, луга, песок, вода площадная и линейная) + дороги, аллеи и стены
//! (`map/roads.rs`, стиль ленты переключается панелью Roads) + здания
//! (`map/buildings/`, режим отображения высоты переключается панелью
//! Buildings) + деревья отдельными сущностями. Поверхности красит фактурный
//! материал (`map/surface.rs`): цвет слоя по-прежнему вершинный, шум кладёт
//! шейдер.

use bevy::prelude::*;

use crate::map::buildings::{self, BuildingHeightMode, BuildingZoomBucket};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{AreaKind, MapData, PolyArea, TreeRow};
use crate::map::parking;
use crate::map::pitch;
use crate::map::roads::{self, RoadStyle};
use crate::map::surface::{
    self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, SurfaceKind, spawn_layers,
};
use crate::map::trees::TreeRowStyle;
use crate::map::water::{mesh_water_areas, mesh_water_lines};
use crate::settings::{
    MAP_SIZE, Z_GRASS, Z_GROUND, Z_LANDUSE, Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PARKING_LINES,
    Z_PITCH, Z_PITCH_LINES, Z_POND, Z_SAND, Z_TREE_ROW_BAND, Z_TREE_ROW_BAND_CASING, Z_WATERWAY,
    Z_WOOD,
};

pub const GROUND_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
/// Кварталы `landuse` делят город на жильё и промзону. Промзона осталась
/// холодным серым на полтона от земли — там бетон и утоптанная площадка.
///
/// Жилой квартал — это **двор**, а не подложка: между домами трава,
/// вытоптанная у подъездов и проездов, но трава. Прежние полтона от земли
/// были осторожностью, а на снимке (#27) весь город из-за них лежал ровным
/// бежевым листом, на котором расставлены дома. Зелень приглушённая, темнее
/// газона и сильно темнее парка: двор — это не луг.
const RESIDENTIAL_COLOR: Color = Color::srgb(0.427, 0.451, 0.376);
const INDUSTRIAL_COLOR: Color = Color::srgb(0.843, 0.843, 0.835);
pub const PARK_COLOR: Color = Color::srgb(0.769, 0.878, 0.580);
/// Лес внутри парка — темнее парковой подложки (osm-carto `#ADD19E`), под ним
/// и растут кроны; открытая часть парка так читается как поле.
pub const WOOD_COLOR: Color = Color::srgb(0.678, 0.820, 0.620);
/// Ширина зелёной полосы под аллеей (`natural=tree_row`), м. Аллея — тот же лес,
/// только вытянутый в линию, поэтому и подложка у неё лесная: без неё ряд крон
/// висит на голом асфальте, тогда как каждое дерево в парке стоит на зелени.
///
/// Чуть уже полного вылета кроны (2 · 4 · `TREE_CROWN_REACH` = 12 м): кроны
/// должны свешиваться за край полосы, как свешиваются за контур лесного
/// полигона, иначе видно саму полосу, а не деревья на ней.
const TREE_ROW_BAND_WIDTH: f32 = 10.0;
/// Кант подложки аллеи — та же зелень на пару тонов темнее. У дорожного канта
/// роль «отделить полотно от фона», здесь — «показать край зарослей», поэтому
/// цвет берётся из семейства леса, а не серый, как у улицы.
const TREE_ROW_CASING_COLOR: Color = Color::srgb(0.565, 0.729, 0.510);
/// Луг — заметно светлее парка (`#DDEFBE`): поле без деревьев обязано читаться
/// поверх парковой заливки, иначе газон сливается с лесом.
const GRASS_COLOR: Color = Color::srgb(0.867, 0.937, 0.745);
/// Песок/пляж (osm-carto `#F5E9C6`).
const SAND_COLOR: Color = Color::srgb(0.961, 0.914, 0.776);
/// Стоянка — тот же асфальт, что и проезжая часть: стоянка лежит поверх дорог
/// (`Z_PARKING`), и въезд, проезд и сама площадка обязаны быть одним полотном.
/// Свой, более тёмный тон рисовал на месте каждой стоянки заплату поверх
/// улицы, к которой она примыкает.
const PARKING_COLOR: Color = roads::ROAD_COLOR;

/// Кайма площадного слоя вдоль его контура ([`MeshBuilder::push_inset_band`]):
/// ширина, м, и цвет на самом контуре; к дальнему краю кайма сходит в заливку.
struct Rim {
    width: f32,
    edge: Color,
}

/// Кромки зелени и песка — та же заливка на несколько процентов темнее:
/// полигон читается как вырезанная фигура, а не как пятно, разлитое по земле.
/// У леса кромка шире и темнее — под пологом у края тень.
const PARK_RIM: Rim = Rim {
    width: 2.5,
    edge: Color::srgb(0.707, 0.808, 0.534),
};
const WOOD_RIM: Rim = Rim {
    width: 3.0,
    edge: Color::srgb(0.610, 0.738, 0.558),
};
const GRASS_RIM: Rim = Rim {
    width: 2.0,
    edge: Color::srgb(0.806, 0.871, 0.693),
};
const SAND_RIM: Rim = Rim {
    width: 2.0,
    edge: Color::srgb(0.913, 0.868, 0.737),
};
/// Кромка площадки — бортик коробки или бровка поля: темнее любого покрытия,
/// один на все виды, потому что на снимке это тень борта, а не краска.
const PITCH_RIM: Rim = Rim {
    width: 1.0,
    edge: Color::srgb(0.322, 0.310, 0.286),
};

/// Полигон слоя с каймой по контуру, дырки включительно (у дырки кайма лежит
/// снаружи её контура — внутри заливки). Кайма кладётся после заливки в тот
/// же меш: в одном слое побеждает нарисованное позже (depth `GreaterEqual`),
/// так что своего z ей не нужно. Ширину у дырок зажимает толщина внешнего
/// контура — та же, что зажала кайму на нём самом.
fn push_area(builder: &mut MeshBuilder, area: &PolyArea, fill: Color, rim: &Rim) {
    let fill = fill.to_linear();
    let edge = rim.edge.to_linear();
    builder.push_polygon(&area.outer, &area.holes, fill);
    let Some(width) = builder.push_inset_band(&area.outer, rim.width, false, edge, fill) else {
        return;
    };
    for hole in &area.holes {
        builder.push_inset_band(hole, width, true, edge, fill);
    }
}

// Все слои карты идут через один `materials: LayerMaterials`: своих
// `Assets<ColorMaterial>` и кровельного хэндла системе больше не надо — их
// держит и разворачивает шов (`map/surface.rs`). Это сняло два параметра из
// десяти; оставшиеся восемь — команды, меши, материалы, ступень зума кровель,
// сама карта, две ручки стиля и раскладка стоянок, которую система считает и
// кладёт ресурсом, — всё это настоящие входы и выходы разового поднятия мира,
// и сводить их в тип ради линта нечего
#[allow(clippy::too_many_arguments)]
pub fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    building_bucket: Res<BuildingZoomBucket>,
    map: Res<MapData>,
    height_mode: Res<BuildingHeightMode>,
    road_style: Res<RoadStyle>,
    mut parking_layout: ResMut<parking::ParkingLayout>,
) {
    // Раскладка стоянок — вход сборки, а не её выход: по ней рисуется и
    // разметка мест, и ряды машин (`map/cars`), так что живёт она ресурсом и
    // считается один раз на загрузку мира.
    *parking_layout = parking::ParkingLayout::new(&map.parking);
    let (surfaces, surface_report) = mesh_surfaces(&map, &parking_layout);
    info!("{surface_report}");
    if surface_report.skipped > 0 {
        warn!(
            "map meshing: {} degenerate polygons skipped",
            surface_report.skipped
        );
    }
    // Тега у этих слоёв нет (`()`), и это не упущение шва, а состояние дел:
    // их никто не запрашивает, а значит и не пересобирает — они живут ровно
    // столько, сколько живёт город. Дать им метку имело бы смысл вместе с
    // причиной пересобирать, а её пока нет.
    spawn_layers(&mut commands, &mut meshes, &materials, surfaces, ());

    roads::spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        roads::mesh_roads(&map, *road_style),
    );

    let plan = buildings::BuildingPlan {
        mode: *height_mode,
        bucket: *building_bucket,
        shadows: true,
    };
    buildings::spawn_building_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        buildings::mesh_buildings(plan, &map.buildings, &map.roads),
    );
}

/// Что вышло из сборки поверхностей — значением, а не тремя строками в логе.
///
/// Вода и водотоки печатались двумя своими `info!` изнутри сборки; здесь они
/// два поля, потому что оба шага — самые дорогие в слое, а `info!` на macOS
/// меряет то, что решит App Nap.
pub struct SurfaceReport {
    pub water_areas: usize,
    pub water_lines: usize,
    pub vertices: usize,
    /// Вырожденные контуры, которые билдеры пропустили: ненулевое значение —
    /// повод посмотреть в парс, а не в сборку.
    pub skipped: usize,
    /// Отмель площадной воды — вложенные офсеты по всем полигонам сразу.
    pub water: std::time::Duration,
    /// Русла: резка по берегам площадной воды.
    pub waterways: std::time::Duration,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for SurfaceReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            water_areas,
            water_lines,
            vertices,
            water,
            waterways,
            elapsed,
            ..
        } = self;
        write!(
            f,
            "surface meshing: {vertices} verts in {elapsed:.1?} \
             (water {water_areas} areas in {water:.1?}, \
             waterways {water_lines} ways in {waterways:.1?})"
        )
    }
}

/// Покрытия и разметка — тринадцать слоёв города, собранные **без мира**.
///
/// Это те самые слои, что живут ровно столько, сколько живёт город: земля,
/// кварталы, зелень, песок, площадки, стоянки, вода и разметка по ним. До шва
/// они собирались прямо в теле [`spawn_map`], и достать их из теста или из
/// офлайн-замера было нечем — единственной дверью в них была система Bevy.
///
/// Раскладка стоянок приходит **готовой**: по ней рисуется не только разметка
/// мест, но и ряды машин, поэтому она ресурс мира, а не выход этой сборки.
pub fn mesh_surfaces(
    map: &MapData,
    parking_layout: &parking::ParkingLayout,
) -> (Vec<LayerMesh>, SurfaceReport) {
    let started = std::time::Instant::now();
    // земля — квад на всю карту тем же фактурным материалом, что и прочие
    // поверхности: спрайту с плоским цветом фактуру не положить
    let mut ground = MeshBuilder::with_surface_coords();
    ground.push_rect(Vec2::ZERO, MAP_SIZE, GROUND_COLOR.to_linear());

    // Квартал — самая нижняя заливка; каймы у него нет намеренно: кромка
    // спорила бы и с зеленью, и с домами. Слоя два, потому что фактура у
    // жилого и промышленного квартала разная по смыслу: во дворе трава с
    // проплешинами, в промзоне утоптанная земля, и одна `SurfaceKind` на
    // оба означала бы траву на бетонной площадке.
    let mut yards = MeshBuilder::with_surface_coords();
    let mut works = MeshBuilder::with_surface_coords();
    for area in &map.landuse {
        let (builder, color) = match area.kind {
            AreaKind::Industrial => (&mut works, INDUSTRIAL_COLOR),
            _ => (&mut yards, RESIDENTIAL_COLOR),
        };
        builder.push_polygon(&area.outer, &area.holes, color.to_linear());
    }

    let mut parks = MeshBuilder::with_surface_coords();
    for park in &map.parks {
        push_area(&mut parks, park, PARK_COLOR, &PARK_RIM);
    }

    let mut woods = MeshBuilder::with_surface_coords();
    for area in &map.woods {
        push_area(&mut woods, area, WOOD_COLOR, &WOOD_RIM);
    }

    let mut grass = MeshBuilder::with_surface_coords();
    for area in &map.grass {
        push_area(&mut grass, area, GRASS_COLOR, &GRASS_RIM);
    }

    let mut sand = MeshBuilder::with_surface_coords();
    for area in &map.sand {
        push_area(&mut sand, area, SAND_COLOR, &SAND_RIM);
    }

    // вода — не каймой по контуру, как зелень: отмель у неё — расстояние до
    // ближайшего берега по всем полигонам сразу (`water::mesh_water_areas`)
    let water_started = std::time::Instant::now();
    let water = mesh_water_areas(&map.water);
    let water_took = water_started.elapsed();

    // стоянка — асфальт своим слоем, того же тона, что проезжая часть; по
    // нему идёт разметка мест (`map::parking`)
    let mut parking = MeshBuilder::with_surface_coords();
    for area in &map.parking {
        // Без кромки: у стоянки нет края, который видно сверху, — асфальт
        // въезда переходит в асфальт площадки, и любая кайма по контуру
        // рисовала на въезде градиент поперёк дороги.
        parking.push_polygon(&area.outer, &area.holes, PARKING_COLOR.to_linear());
    }
    let mut parking_lines = MeshBuilder::default();
    for (area, stalls) in map.parking.iter().zip(&parking_layout.0) {
        parking::push_markings(&mut parking_lines, area, stalls);
    }

    // площадка — покрытие своего цвета, и на нём разметка (`map::pitch`).
    // Кант тот же, что у прочих зон: у поля на снимке всегда есть кромка
    let mut pitches = MeshBuilder::with_surface_coords();
    for area in &map.pitches {
        let AreaKind::Pitch(kind) = area.kind else {
            continue;
        };
        push_area(&mut pitches, area, pitch::color(kind), &PITCH_RIM);
    }
    let mut pitch_lines = MeshBuilder::default();
    for area in &map.pitches {
        pitch::push_markings(&mut pitch_lines, area);
    }

    let waterways_started = std::time::Instant::now();
    let waterways = mesh_water_lines(&map.water_lines, &map.water);
    let waterways_took = waterways_started.elapsed();

    let skipped: usize = [
        &yards, &works, &parks, &woods, &grass, &sand, &pitches, &parking, &water,
    ]
    .iter()
    .map(|builder| builder.skipped_polygons())
    .sum();

    // Покрытия и разметка одним списком. Разметка мест и полей — плоская:
    // это белая краска, а не фактура покрытия.
    let layers: Vec<LayerMesh> = [
        (ground, Z_GROUND, "ground", SurfaceKind::Ground),
        (works, Z_LANDUSE, "landuse_works", SurfaceKind::Ground),
        (yards, Z_LANDUSE_YARD, "landuse_yards", SurfaceKind::Yard),
        (parks, Z_PARK, "parks", SurfaceKind::Park),
        (woods, Z_WOOD, "woods", SurfaceKind::Wood),
        (grass, Z_GRASS, "grass", SurfaceKind::Grass),
        (sand, Z_SAND, "sand", SurfaceKind::Sand),
        (pitches, Z_PITCH, "pitches", SurfaceKind::Ground),
        (parking, Z_PARKING, "parking", SurfaceKind::Street),
        (water, Z_POND, "water", SurfaceKind::Water),
        (waterways, Z_WATERWAY, "waterways", SurfaceKind::Water),
    ]
    .into_iter()
    .map(|(builder, z, name, kind)| LayerMesh::new(builder, z, name, MaterialSpec::Surface(kind)))
    .chain(
        [
            (pitch_lines, Z_PITCH_LINES, "pitch_lines"),
            (parking_lines, Z_PARKING_LINES, "parking_lines"),
        ]
        .into_iter()
        .map(|(builder, z, name)| LayerMesh::new(builder, z, name, MaterialSpec::Flat)),
    )
    .collect();

    let report = SurfaceReport {
        water_areas: map.water.len(),
        water_lines: map.water_lines.len(),
        vertices: layers
            .iter()
            .map(|layer| layer.builder.vertex_count())
            .sum(),
        skipped,
        water: water_took,
        waterways: waterways_took,
        elapsed: started.elapsed(),
    };
    (layers, report)
}

/// Офлайн-замер слоёв поверхностей — строками `LayerCost`, как у зданий и
/// машин. Своей сборки у него нет: он зовёт тот же [`mesh_surfaces`], что и
/// игра, — ради этого шов и делался.
pub fn measure_surfaces(map: &MapData) -> Vec<LayerCost> {
    let layout = parking::ParkingLayout::new(&map.parking);
    let (layers, report) = mesh_surfaces(map, &layout);
    surface::layer_costs(&layers, report.elapsed)
}

/// Зелёная полоса под аллеей — чтобы пересборка стиля знала, что деспавнить.
///
/// `Copy` — метку получают оба слоя подложки, кант и заливка, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct TreeRowBandTag;

/// Подложка аллей: лента лесного цвета вдоль каждого `natural=tree_row`, со
/// своими **тремя** ручками — стык, сглаживание и кант, — теми же самыми, что у
/// дорожных лент (`RoadJoin` / `RoadSmoothing` / `casing`), но своими: ломаная
/// аллеи и ломаная улицы приходят из разных данных, и подложка обязана выглядеть
/// лесом даже там, где дороги оставлены нетронутыми.
///
/// Отдельная сущность, а не часть слитого меша лесов, ровно потому, что эти
/// ручки переключаются на лету, а слой лесов собирается один раз на город и
/// пересобирать его на каждый клик незачем.
/// **Чистая функция и единственная дверь в слой** — как `fences::mesh_fences` и
/// `rail::mesh_rails`. Отчёта у неё нет, и это не пропуск: подложка ничего не
/// печатает в лог, а сочинять отчёт ради одинаковости — значит заводить число,
/// которое никто не читает.
pub fn mesh_tree_row_band(rows: &[TreeRow], style: &TreeRowStyle) -> Vec<LayerMesh> {
    let mut casing = MeshBuilder::default();
    // заливка — лесная поверхность, с той же фактурой, что лес под кронами
    let mut fill = MeshBuilder::with_surface_coords();
    // тумблер панели выключает аллеи целиком: без деревьев ряда полоса под
    // ними — просто зелёная линия поперёк города
    let rows = if style.enabled { rows } else { &[] };
    for row in rows {
        // Chaikin тот же, что у дорог: ломаная из OSM на повороте даёт полосе
        // заметный угол, которого у лесного контура не бывает
        let path = roads::smooth_path(&row.points, TREE_ROW_BAND_WIDTH, style.smoothing);
        if style.casing {
            let width = TREE_ROW_BAND_WIDTH
                + 2.0 * crate::map::footprint::casing_width(TREE_ROW_BAND_WIDTH);
            roads::push_ribbon(
                &mut casing,
                &path,
                width,
                TREE_ROW_CASING_COLOR.to_linear(),
                style.join,
            );
        }
        roads::push_ribbon(
            &mut fill,
            &path,
            TREE_ROW_BAND_WIDTH,
            WOOD_COLOR.to_linear(),
            style.join,
        );
    }

    // кант — плоский белый, заливка — та же лесная фактура, что под кронами
    vec![
        LayerMesh::new(
            casing,
            Z_TREE_ROW_BAND_CASING,
            "tree_row_band_casing",
            MaterialSpec::Flat,
        ),
        LayerMesh::new(
            fill,
            Z_TREE_ROW_BAND,
            "tree_row_band",
            MaterialSpec::Surface(SurfaceKind::Wood),
        ),
    ]
}

/// Пересборка подложки аллей после правки её настроек из UI.
pub fn rebuild_tree_row_band(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    style: Res<TreeRowStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<TreeRowBandTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials,
        mesh_tree_row_band(&map.tree_rows, &style),
        TreeRowBandTag,
    );
}

#[cfg(test)]
mod tests;
