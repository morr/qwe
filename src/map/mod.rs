mod along;
// публичен по той же причине, что и `trees`: витрина `roof_gallery` строит
// свои дома его же вызовами (`push_house`, `RoofShape`, `shape_facts`,
// `material::RoofLook`)
pub mod buildings;
// публичен по той же причине: витрина `car_gallery` расставляет ряды его же
// вызовом (`cars_mesh`), а стенд кузовов рисует машины его же `body`
pub mod cars;
mod fences;
pub mod footprint;
// виден всему крейту ради `navigation/navmesh.rs`: лента моста индексируется
// той же сеткой, что и всё на карте. Не `pub`, в отличие от соседей выше и
// ниже: у тех потребитель снаружи крейта (витрины, офлайн-аудит), а у сетки
// все вызывающие внутри — то же сужение, каким шов вернул `spawn_layer` и
// `LayerMaterial` в приватные
pub(crate) mod grid;
mod industry;
pub mod inspect;
mod meshing;
pub mod osm;
pub(crate) mod parallel;
mod parking;
mod pitch;
mod prepare;
mod rail;
mod roads;
mod seed;
// публичен ради стенда кузовов витрины `car_gallery`: он рисует тень машины
// тем же правилом длины, каким её рисует слой города (`shadow::offset`), а не
// своей копией выражения
pub mod shadow;
// словарь `i_overlay` — общий дом контура, фигуры, обводки и закрутки кольца,
// по той же причине, что `seed` и `grid`: копия примитива и есть дефект
mod shapes;
mod smooth;
mod spawn;
// `pub`: дефолты и границы ползунков азимута и высоты живут рядом со своим
// ресурсом, и за ними сюда ходят панель Sun и витрины кровель и стены
pub mod sun;
// публичен по той же причине, что `buildings` и `cars`: витрины кладут слои
// игровым `spawn_layers`, а ему нужен `LayerMaterials` из этого модуля
pub mod surface;
mod tram;
pub mod trees;
mod wagons;
mod water;
mod zoom;

pub use self::buildings::material::RoofStyle;
// `measure_layers` (и `measure_cars` ниже) наружу — офлайн-бенчу
// `examples/bench/map_meshing`: замер сборки слоёв без мира и без GPU. Форма
// входа у него своя, не `BuildingPlan`, — см. док `measure_layers`. Строку
// замера, `LayerCost`, бенч берёт из `surface` — она общая для всех слоёв
pub use self::buildings::{BuildingHeightMode, measure_layers};
pub use self::cars::{CarStyle, measure_cars};
pub use self::industry::IndustryStyle;
// Остальные замеры бенча. В отличие от двух выше они **не повторяют** сборку:
// каждый зовёт игровой `mesh_*` и раскладывает его слои в строки
// (`surface::layer_costs`). Ради этого шов и делался — до него эти слои
// мерились только строкой из лога живого приложения, которую на macOS решает
// App Nap.
pub use self::rail::measure_rails;
pub use self::roads::measure_roads;
pub use self::spawn::measure_surfaces;
pub use self::tram::measure_tram;
// `RibbonCap`/`RibbonJoin` наружу — витринам, которые кладут ленту сами
// (`car_gallery` рисует под рядами саму проезжую часть)
pub use self::meshing::{
    MeshBuilder, RibbonCap, RibbonJoin, merge_close_points, min_area_rect, miter_offsets,
};
pub use self::osm::{TREE_DENSITY_MAX, TreeRowPlacement};
// `ROAD_COLOR` наружу по той же причине: ряд машин витрины обязан стоять на
// том же асфальте, что в городе
pub use self::roads::{CrossingMode, ROAD_COLOR, RoadJoin, RoadStyle};
// форма дорог: ресурс ручек формы (панель игры и витрины `roads`) и
// глобаль ширины полосы, которую витрина ставит перед разбором сама
pub use self::roads::shape::{
    CORNER_RADIUS_MAX, CORNER_RADIUS_MIN, CORNER_RADIUS_STEP, CURVE_TOLERANCE_MAX,
    CURVE_TOLERANCE_MIN, CURVE_TOLERANCE_STEP, LANE_WIDTH_MAX, LANE_WIDTH_MIN, LANE_WIDTH_STEP,
    MEDIAN_GAP_MAX, MEDIAN_GAP_MIN, MEDIAN_GAP_STEP, RoadShape, RoadShapeOnMap, TAPER_MAX,
    TAPER_MIN, TAPER_STEP, lane_width, set_lane_width, settle_road_shape,
};
// краска и колея: ресурс ручек Paint/Wear (панель игры и витрины `roads`), а
// материал краски — витринам, которые поднимают материалы поверхностей сами
pub use self::roads::paint::{
    PAINT_MAX, PAINT_MIN, PAINT_STEP, PaintMaterial, RoadPaintStyle, TURN_WEAR_MAX, TURN_WEAR_MIN,
    TURN_WEAR_STEP, WEAR_MAX, WEAR_MIN, WEAR_STEP,
};
// сеть улиц — тип поля `MapData::network`; наружу его читает оверлей сети
// витрины `roads`
pub use self::roads::network::{RoadNetwork, Street, StreetWay};
// оверлей сети — строке Debug игры и строке `Network` витрины `roads`
pub use self::roads::network::overlay::mesh_network_overlay;
// оверлей контуров OSM — строке Debug игры и строке `Contours` витрины `roads`
pub use self::osm::contours::mesh_osm_contours;
// оверлей линий колеи — строке `Rut lines` вкладки Debug и строке `Ruts` витрины
pub use self::roads::DrawnPieces;
pub use self::roads::ruts::{RutLines, mesh_rut_overlay};
// `smooth_path` со `Smoothing` — витрине машин (`car_gallery` кладёт асфальт
// под ряд по сглаженной осевой) и панели аллей (`TreeRowStyle::smoothing`)
pub use self::smooth::{Smoothing, smooth_path};
pub use self::spawn::{GROUND_COLOR, PARK_COLOR, WOOD_COLOR};
// Готовые слои из потока загрузки: ресурс разбирает вход в мир, снимок
// стилей берёт `loading.rs` на старте потока
pub use self::prepare::{PreparedMeshes, StyleSnapshot, WorldMeshes};
// `apply_sun_style` наружу — тому же офлайн-бенчу: тени он собирает игровым
// билдером, а солнце тому билдеру приходит процессной глобалью
pub use self::sun::{
    SunOnMap, SunStyle, apply_sun, apply_sun_style, shadow_dir, shadow_length_scale, sun_light,
    sun_stretch,
};
#[cfg(test)]
pub(crate) use self::sun::{default_sun, sun_at};
pub use self::surface::{LayerCost, LayerMesh, SurfaceStyle};
pub use self::tram::TramStyle;
// Двери слоёв наружу — витрине `examples/demos/roads`: она ведёт вырезку OSM
// через тот же `parse` и те же `mesh_*`/`spawn_*`, что и игра, а собирает их в
// своей системе, потому что у неё карт не одна на мир, а по одной на пример.
// Ступени зума идут следом: `mesh_fences` и `mesh_rails` берут ступень, а не
// развёрнутую из таблицы ширину
pub use self::cars::mesh_map_cars;
pub use self::fences::{FenceZoomBucket, measure_fences, mesh_fences};
pub use self::parking::ParkingLayout;
pub use self::rail::{RailZoomBucket, mesh_rails};
pub use self::roads::tram_lay::TramTracks;
pub use self::roads::{
    RoadBuild, VergeLods, VergeZoomBucket, build_roads, corner_paving_contours, mesh_roads,
    spawn_road_meshes, spawn_road_paint,
};
pub use self::spawn::{mesh_surfaces, mesh_tree_row_band};
pub use self::tram::{TramZoomBucket, mesh_tram};
pub use self::trees::{ConiferField, ConiferNoiseStyle, TreeRowStyle, TreeShape, TreeStyle};

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

use crate::loading::{AppState, WorldInitSet};
use crate::prefs::PrefsExt;

/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`. Общий и для
/// домов, и для крон по той же причине, что и само солнце ([`sun`]).
pub const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        // семантический индекс пробы точки (`inspect`): сдвиг, ресурс,
        // пересборка по смене `MapData` — та же проводка нужна витрине
        // `roads`, у которой своего `MapPlugin` нет. Отдельным стейтментом,
        // чтобы не перестраивать рифмету цепочку ниже
        app.add_plugins(inspect::InspectPlugin);
        app.add_plugins(Material2dPlugin::<surface::SurfaceMaterial>::default())
            .add_plugins(Material2dPlugin::<buildings::material::RoofMaterial>::default())
            .add_plugins(Material2dPlugin::<trees::CrownMaterial>::default())
            .add_plugins(Material2dPlugin::<roads::paint::PaintMaterial>::default())
            // сырой OSM — вход разбора (`loading::parse_knobs`); смена
            // перезагружает мир (`city::parse_knobs_diverged`)
            .register_pref::<osm::parse::RawOsm>()
            // линии колеи кладёт сборка дорог (`roads::rebuild_roads`,
            // `spawn_map`); до первой сборки — пусто
            .init_resource::<RutLines>()
            // пути трамвая по нарисованной улице — тем же проходом
            .init_resource::<TramTracks>()
            // отрисованные куски дорог — тем же проходом; их читает
            // семантический индекс (`map/inspect.rs`)
            .init_resource::<roads::DrawnPieces>()
            // метка слоёв: по ней их выбирают `brp layers`/`probe`/`hide`
            .register_type::<surface::MapLayer>()
            .register_type::<TreeShape>()
            .register_type::<TreeRowPlacement>()
            // Ползуночные настройки, что пишут на каждом пройденном делении:
            // регистрируются без отслеживания, в файл уезжают через оседание —
            // отслеживается их осевшая копия (`SunOnMap`/`RoadShapeOnMap`),
            // она же и запускает пересборки
            .init_resource::<SunStyle>()
            .register_type::<SunStyle>()
            .init_resource::<RoadShape>()
            .register_type::<RoadShape>()
            // ресурсы, которые настройки не трогают: поле до подсчёта посадки,
            // зум-ступени слоёв, кеш размещения машин
            .init_resource::<ConiferField>()
            .init_resource::<buildings::BuildingZoomBucket>()
            .init_resource::<cars::CarZoomBucket>()
            .init_resource::<cars::CarPlacement>()
            .init_resource::<wagons::WagonZoomBucket>()
            .init_resource::<parking::ParkingLayout>()
            .init_resource::<fences::FenceZoomBucket>()
            .init_resource::<fences::FenceBuild>()
            .init_resource::<roads::paint::PaintZoomBucket>()
            .init_resource::<roads::VergeZoomBucket>()
            .init_resource::<rail::RailZoomBucket>()
            .init_resource::<trees::TreeZoomBucket>()
            .init_resource::<trees::CrownStream>()
            .init_resource::<trees::TreeLodShown>()
            .init_resource::<trees::LayerWarmup>()
            .init_resource::<tram::TramZoomBucket>()
            .register_pref::<SunOnMap>()
            .register_pref::<TreeStyle>()
            .register_pref::<TreeRowStyle>()
            .register_pref::<ConiferNoiseStyle>()
            .register_pref::<BuildingHeightMode>()
            .register_pref::<RoofStyle>()
            .register_pref::<RoadStyle>()
            .register_pref::<RoadShapeOnMap>()
            .register_pref::<RoadPaintStyle>()
            .register_pref::<SurfaceStyle>()
            .register_pref::<TramStyle>()
            .register_pref::<IndustryStyle>()
            .register_pref::<CarStyle>()
            // материалы поверхностей и кровель — один комплект на всё
            // приложение, слои всех городов берут хэндлы из него.
            //
            // Солнце уезжает в глобаль **до** них, и это не порядок ради
            // порядка: материал кровель строится один раз на всё приложение, а
            // `apply_sun` из `PreUpdate` в первом прогоне `Main` идёт уже
            // после `Startup`. Без посева юниформ `light` навсегда остался бы
            // с компайл-таймовым азимутом, пока пользователь не тронет
            // ползунок, — блики фальца освещены с 300°, а стены и тени с
            // сохранённого угла
            .add_systems(
                Startup,
                (
                    (sun::seed_sun, sun::apply_sun).chain(),
                    roads::shape::seed_road_shape,
                    (
                        surface::init_surface_materials,
                        surface::init_flat_materials,
                        buildings::material::init_roof_material,
                        trees::init_crown_material,
                    ),
                )
                    .chain(),
            )
            // солнце — в глобаль, из которой его читают чистые функции сборки
            // мешей. `PreUpdate` идёт и до `StateTransition` (там строится мир
            // на входе), и до `Update` (там пересобираются слои), так что
            // всякая сборка кадра видит уже новое солнце. Перед записью —
            // оседание ползунка: пересобирать карту на каждое пройденное
            // деление слишком дорого
            .add_systems(
                PreUpdate,
                (
                    (sun::settle_sun, sun::apply_sun).chain(),
                    roads::shape::settle_road_shape,
                ),
            )
            // краска и колея асфальта ложатся по ширине полосы, с которой
            // разобран мир: глобаль краски — из снимка карты, до сборки мира;
            // материалы живут вне мира, так что после перезагрузки с другой
            // шириной их надо перенастроить
            .add_systems(
                OnEnter(AppState::Playing),
                (
                    roads::shape::adopt_lane_width,
                    surface::retune_surface_materials,
                )
                    .chain()
                    .before(WorldInitSet::Navmesh),
            )
            .add_systems(
                OnEnter(AppState::Playing),
                // Мир спавнится из собранного потоком загрузки (`map/prepare.rs`):
                // мешинг отыграл в потоке, здесь остаётся положить готовое.
                // Набор деревьев собирается первым (лес плюс аллеи выбранной
                // политики; поток собрал его же под снимок ручек — совпадает,
                // и пересборки не будет), поле хвои решает форму кроны и
                // потому считается по уже собранному набору и до крон. Кроны
                // из потока спавнит `spawn_prepared_trees`; правку стиля
                // живого мира ведёт `rebuild_trees` в `Update`. Расстановка
                // машин из потока наполняет кеш (`seed_parked_cars`) —
                // `rebuild_cars` платит только меш кузовов. Рельсы и трамвай
                // спавнятся здесь же по той причине: их ступень зума — их
                // личное дело. Ступень перед спавном ставится по камере, а
                // камера на стартовый вид — тоже в `Spawn`, отсюда `after`
                (
                    (
                        trees::recompose_row_trees,
                        trees::build_conifer_field,
                        zoom::seed_zoom_bucket::<buildings::BuildingLods>,
                        // обочины входят в дорожные слои, которые кладёт
                        // `spawn_map`, — их ступень сеется до него
                        zoom::seed_zoom_bucket::<roads::VergeLods>,
                        spawn::spawn_map,
                        zoom::seed_zoom_bucket::<cars::CarLods>,
                        cars::seed_parked_cars,
                        cars::rebuild_cars,
                    )
                        .chain(),
                    (
                        zoom::seed_zoom_bucket::<wagons::WagonLods>,
                        wagons::rebuild_wagons,
                        zoom::seed_zoom_bucket::<fences::FenceLods>,
                        // на входе в мир заборы приходят из потока загрузки
                        // (собраны под прогноз зума входа); прогноз разъехался —
                        // сборка в кадре, как до переноса. Пересечение порога
                        // ведёт `queue_fence_rebuild`
                        fences::rebuild_fences,
                        industry::rebuild_industry,
                        zoom::seed_zoom_bucket::<rail::RailLods>,
                        // все ступени рельсов лежат в мире разом (собраны
                        // потоком): пересечение порога переключает видимость
                        // (`show_lod_layers`), а не пересобирает слой
                        rail::spawn_rails,
                        zoom::seed_zoom_bucket::<tram::TramLods>,
                        tram::rebuild_tram,
                        spawn::rebuild_tree_row_band,
                        zoom::seed_zoom_bucket::<trees::TreeLods>,
                        trees::spawn_prepared_trees,
                        // прогрев слитых кусков и теней: первая их подготовка
                        // рендером уходит под экран загрузки, а не в первый
                        // кадр пересечения порога (`trees::warm_layers`)
                        trees::begin_layer_warmup,
                    )
                        .chain(),
                )
                    .chain()
                    .in_set(WorldInitSet::Spawn)
                    .after(crate::camera::place_camera_on_world_ready),
            )
            // когда пересобирать слой — дело слоя (`rebuilds_on()` рядом с его
            // `rebuild_*`), здесь только проводка. Что ни одна `rebuild_*` не
            // попала сюда дважды — правило «одно условие — одна регистрация»,
            // см. `roads::rebuilds_on` — стережёт `tests/map.rs`
            .add_systems(
                Update,
                (
                    // состав набора и поле хвои — до крон: обе системы
                    // выходят сразу, если их вход не поехал, так что отдельных
                    // условий на них не надо. Ступень зума крон считается
                    // перед связкой, а её смена идёт своей системой следом:
                    // связку она не трогает, кроны и тени только прячет
                    (
                        zoom::update_zoom_bucket::<trees::TreeLods>,
                        (
                            trees::recompose_row_trees,
                            trees::retune_conifer_field,
                            spawn::rebuild_tree_row_band,
                            trees::rebuild_trees,
                        )
                            .chain()
                            .run_if(trees::rebuilds_on()),
                        // кроны-сущности досыпаются и убираются пачками, а
                        // показанная ступень ждёт, пока встанут все
                        trees::stream_tree_crowns,
                        trees::show_tree_lod.run_if(trees::switches_on()),
                        // прогрев слоёв дерева: два кадра на входе в мир,
                        // дальше выходит сразу
                        trees::warm_layers,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    (
                        zoom::update_zoom_bucket::<buildings::BuildingLods>,
                        buildings::rebuild_buildings.run_if(buildings::rebuilds_on()),
                        // ступень зума зданий не пересобирает слой, а прячет
                        // оборудование кровли в шейдере
                        buildings::material::show_roof_clutter,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    // ступень обочин считается раньше пересборки: стиль и
                    // форма собирают текущую ступень, а смена ступени зума
                    // (`show_road_steps`, следом) только перекрашивает слои —
                    // обе ступени лежат в мире заранее
                    (
                        zoom::update_zoom_bucket::<roads::VergeLods>,
                        roads::rebuild_roads.run_if(roads::rebuilds_on()),
                        roads::show_road_steps,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    // три слоя со своими таблицами зума, одной группой: у
                    // каждого ступень считается своим `update_zoom_bucket`, и
                    // пересборка идёт следом
                    (
                        zoom::update_zoom_bucket::<cars::CarLods>,
                        cars::rebuild_cars.run_if(cars::rebuilds_on()),
                        zoom::update_zoom_bucket::<wagons::WagonLods>,
                        wagons::rebuild_wagons.run_if(wagons::rebuilds_on()),
                        zoom::update_zoom_bucket::<fences::FenceLods>,
                        // пересечение порога ставит фоновую сборку, старый слой
                        // стоит до подмены ([`fences::FenceBuild`]); подмена
                        // опрашивается каждый кадр, пока задачи нет — один `if`
                        (
                            fences::queue_fence_rebuild.run_if(fences::rebuilds_on()),
                            fences::apply_fence_rebuild,
                        )
                            .chain(),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    // ступени зума у промзоны нет, так что в группу выше её
                    // класть не за что: слой стоит сам по себе, как дороги
                    industry::rebuild_industry
                        .run_if(in_state(AppState::Playing))
                        .run_if(industry::rebuilds_on()),
                    // сила фактуры — юниформ материалов, а не меши: без
                    // привязки к состоянию, материалы живут вне мира
                    surface::retune_surface_materials.run_if(surface::retunes_on()),
                    // краска не пересобирается со ступенью: ступень только
                    // прячет меши линий, которых на этом зуме не видно
                    (
                        zoom::update_zoom_bucket::<roads::paint::PaintLods>,
                        roads::paint::show_paint,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    buildings::material::retune_roof_material
                        .run_if(buildings::material::retunes_on()),
                    // ступень зума считается каждый кадр (одно чтение камеры и
                    // сравнение). Таблицы у путей и трамвая свои, и пороги в
                    // них не совпадают, поэтому и ступени считаются порознь.
                    // У путей пересборок больше нет вовсе: собраны все ступени
                    // (`rail::spawn_rails`), порог переключает видимость
                    (
                        zoom::update_zoom_bucket::<rail::RailLods>,
                        zoom::show_lod_layers::<rail::RailLods, With<rail::RailLayerTag>>,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    (
                        zoom::update_zoom_bucket::<tram::TramLods>,
                        tram::rebuild_tram.run_if(tram::rebuilds_on()),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                ),
            );
    }
}
