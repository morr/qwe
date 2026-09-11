//! Рендер OSM-карты: по одному слитому `Mesh2d` на слой (земля, кварталы,
//! парки, луга, песок, вода площадная и линейная) + дороги, аллеи и стены
//! (`map/roads.rs`, стиль ленты переключается панелью Roads) + здания
//! (`map/buildings/`, режим отображения высоты переключается панелью
//! Buildings) + деревья отдельными сущностями. Поверхности красит фактурный
//! материал (`map/surface.rs`): цвет слоя по-прежнему вершинный, шум кладёт
//! шейдер.

use bevy::prelude::*;

use crate::map::buildings::material::RoofMaterialHandle;
use crate::map::buildings::{self, BuildingHeightMode, BuildingZoomBucket};
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::{AreaKind, MapData, PolyArea, TreeRow, WaterLine, water_line_caps};
use crate::map::parking;
use crate::map::paths;
use crate::map::pitch;
use crate::map::roads::{self, RoadSmoothing, RoadStyle};
use crate::map::surface::{LayerMaterial, SurfaceKind, SurfaceMaterials, spawn_layer};
use crate::map::trees::TreeRowStyle;
use crate::settings::{
    MAP_SIZE, Z_GRASS, Z_GROUND, Z_LANDUSE, Z_LANDUSE_YARD, Z_PARK, Z_PARKING, Z_PARKING_LINES,
    Z_PITCH, Z_PITCH_LINES, Z_POND, Z_SAND, Z_TREE_ROW_BAND, Z_TREE_ROW_BAND_CASING, Z_WATERWAY,
    Z_WOOD, Z_WORN_PATH,
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
const WATER_COLOR: Color = Color::srgb(0.655, 0.804, 0.910);
/// Стоянка — асфальт посветлее проезжей части: полотно улицы укатано, а
/// двор со стоянкой выцветает и пылится.
const PARKING_COLOR: Color = Color::srgb(0.412, 0.408, 0.404);

/// Кайма площадного слоя вдоль его контура ([`MeshBuilder::push_inset_band`]):
/// ширина, м, и цвет на самом контуре; к дальнему краю кайма сходит в заливку.
struct Rim {
    width: f32,
    edge: Color,
}

/// Мелководье: светлая полоса вдоль берега внутри водного полигона — то, что
/// делает пруд прудом, а не синим пятном. Три метра: у Упы это шестая часть
/// ширины, у канала — треть.
const WATER_RIM: Rim = Rim {
    width: 3.0,
    edge: Color::srgb(0.78, 0.885, 0.945),
};
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
/// Кромка стоянки — бордюр: чуть светлее её асфальта.
const PARKING_RIM: Rim = Rim {
    width: 1.0,
    edge: Color::srgb(0.478, 0.475, 0.467),
};
/// Вытоптанная тропа: голая земля, светлее газона и темнее сухой земли — на
/// снимке она читается именно как **светлая** линия по тёмной траве.
const WORN_PATH_COLOR: Color = Color::srgb(0.541, 0.502, 0.435);
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

// материалов у карты теперь два комплекта (поверхности и кровли), и вместе с
// мешами, `MapData` и двумя стилями это восьмой параметр системы
#[allow(clippy::too_many_arguments)]
pub fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    surfaces: Res<SurfaceMaterials>,
    roof_material: Res<RoofMaterialHandle>,
    building_bucket: Res<BuildingZoomBucket>,
    map: Res<MapData>,
    height_mode: Res<BuildingHeightMode>,
    road_style: Res<RoadStyle>,
    mut parking_layout: ResMut<parking::ParkingLayout>,
) {
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

    let mut water = MeshBuilder::with_surface_coords();
    for area in &map.water {
        push_area(&mut water, area, WATER_COLOR, &WATER_RIM);
    }

    // стоянка — асфальт своим слоем: он темнее двора и светлее проезжей
    // части, а по нему идёт разметка мест (`map::parking`)
    let mut parking = MeshBuilder::with_surface_coords();
    for area in &map.parking {
        push_area(&mut parking, area, PARKING_COLOR, &PARKING_RIM);
    }
    *parking_layout = parking::ParkingLayout::new(&map.parking);
    let mut parking_lines = MeshBuilder::default();
    for (area, stalls) in map.parking.iter().zip(&parking_layout.0) {
        parking::push_markings(&mut parking_lines, area, stalls);
    }

    // тропы — вытоптанные дорожки от подъездов к дорогам (`map::paths`)
    let mut worn = MeshBuilder::with_surface_coords();
    let doors: usize = map.buildings.iter().map(|b| b.entrances.len()).sum();
    let worn_count = paths::push_paths(&mut worn, &map.buildings, &map.roads, WORN_PATH_COLOR);
    info!("worn paths: {worn_count} of {doors} doors");

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

    let waterways = mesh_water_lines(&map.water_lines);

    let skipped: usize = [
        &yards, &works, &parks, &woods, &grass, &sand, &pitches, &parking, &water,
    ]
    .iter()
    .map(|builder| builder.skipped_polygons())
    .sum();
    if skipped > 0 {
        warn!("map meshing: {skipped} degenerate polygons skipped");
    }

    for (builder, z, name, kind) in [
        (ground, Z_GROUND, "ground", SurfaceKind::Ground),
        (works, Z_LANDUSE, "landuse_works", SurfaceKind::Ground),
        (yards, Z_LANDUSE_YARD, "landuse_yards", SurfaceKind::Yard),
        (parks, Z_PARK, "parks", SurfaceKind::Park),
        (woods, Z_WOOD, "woods", SurfaceKind::Wood),
        (grass, Z_GRASS, "grass", SurfaceKind::Grass),
        (sand, Z_SAND, "sand", SurfaceKind::Sand),
        (worn, Z_WORN_PATH, "worn_paths", SurfaceKind::Alley),
        (pitches, Z_PITCH, "pitches", SurfaceKind::Ground),
        (parking, Z_PARKING, "parking", SurfaceKind::Street),
        (water, Z_POND, "water", SurfaceKind::Water),
        (waterways, Z_WATERWAY, "waterways", SurfaceKind::Water),
    ] {
        spawn_layer(
            &mut commands,
            &mut meshes,
            builder,
            z,
            name,
            LayerMaterial::Surface(surfaces.handle(kind)),
            (),
        );
    }

    roads::spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        &surfaces,
        *road_style,
        &map,
    );

    // разметка мест и полей — своими мешами поверх покрытия: это белая
    // краска, а не фактура покрытия, и потому плоский материал
    for (builder, z, name) in [
        (pitch_lines, Z_PITCH_LINES, "pitch_lines"),
        (parking_lines, Z_PARKING_LINES, "parking_lines"),
    ] {
        spawn_layer(
            &mut commands,
            &mut meshes,
            builder,
            z,
            name,
            LayerMaterial::Flat(materials.add(Color::WHITE)),
            (),
        );
    }

    buildings::spawn_buildings(
        &mut commands,
        &mut meshes,
        &mut materials,
        &roof_material,
        buildings::BuildingPlan {
            mode: *height_mode,
            bucket: *building_bucket,
            shadows: true,
        },
        &map.buildings,
        &map.roads,
    );
}

/// Лента открытых русел одним мешем. **Трубы не рисуются вовсе**: под землёй
/// воды не видно, а пунктир вдоль улицы читался как ручей поверх неё. Тем, что
/// человек проходит там, где на карте «ручей», управляет не эта отрисовка, а
/// её отсутствие: русло обрывается на портале культверта и продолжается за ним
/// (`water_line_caps`), и между порталами воды на карте просто нет.
fn mesh_water_lines(lines: &[WaterLine]) -> MeshBuilder {
    let color = WATER_COLOR.to_linear();
    let mut open = MeshBuilder::with_surface_coords();

    for line in lines.iter().filter(|line| !line.tunnel) {
        // сглаживание как у дорог: русло в OSM — ломаная по точкам съёмки, и на
        // её изломах лента без сглаживания заметно гранёная
        let points = roads::smooth_path(&line.points, line.width, RoadSmoothing::Light);
        // круглые стыки, и круглые торцы там, где вода продолжается: два way
        // одного русла встречаются в общем узле, и полудиски на торцах
        // сливаются в непрерывную реку. Портал культверта — исключение: за ним
        // воды нет, и полудиск торчал бы на полуширину русла в сухую землю
        let caps = water_line_caps(line, lines).map(|round| {
            if round {
                RibbonCap::Round
            } else {
                RibbonCap::Butt
            }
        });
        open.push_ribbon_capped(&points, false, line.width, color, RibbonJoin::Round, caps);
    }

    open
}

/// Зелёная полоса под аллеей — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
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
pub fn spawn_tree_row_band(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    surfaces: &SurfaceMaterials,
    rows: &[TreeRow],
    style: &TreeRowStyle,
) {
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

    let flat = materials.add(Color::WHITE);
    for (builder, z, name, material) in [
        (
            casing,
            Z_TREE_ROW_BAND_CASING,
            "tree_row_band_casing",
            LayerMaterial::Flat(flat),
        ),
        (
            fill,
            Z_TREE_ROW_BAND,
            "tree_row_band",
            LayerMaterial::Surface(surfaces.handle(SurfaceKind::Wood)),
        ),
    ] {
        spawn_layer(commands, meshes, builder, z, name, material, TreeRowBandTag);
    }
}

/// Пересборка подложки аллей после правки её настроек из UI.
pub fn rebuild_tree_row_band(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    surfaces: Res<SurfaceMaterials>,
    style: Res<TreeRowStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<TreeRowBandTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_tree_row_band(
        &mut commands,
        &mut meshes,
        &mut materials,
        &surfaces,
        &map.tree_rows,
        &style,
    );
}
