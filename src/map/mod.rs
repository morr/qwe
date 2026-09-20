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
mod meshing;
pub mod osm;
mod parking;
mod pitch;
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
pub use self::buildings::{BuildingHeightMode, extrusion_lift, measure_layers};
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
pub use self::roads::{ROAD_COLOR, RoadJoin, RoadStyle};
// а `smooth_path` со `Smoothing` — потому, что асфальт под ним лежит на той же
// сглаженной осевой
pub use self::smooth::{Smoothing, smooth_path};
pub use self::spawn::{GROUND_COLOR, PARK_COLOR, WOOD_COLOR};
// `apply_sun_style` наружу — тому же офлайн-бенчу: тени он собирает игровым
// билдером, а солнце тому билдеру приходит процессной глобалью
pub use self::sun::{
    SunOnMap, SunStyle, apply_sun, apply_sun_style, shadow_dir, shadow_length_scale, sun_light,
    sun_stretch,
};
#[cfg(test)]
pub(crate) use self::sun::{default_sun, sun_at};
pub use self::surface::{LayerCost, SurfaceStyle};
pub use self::tram::TramStyle;
pub use self::trees::{ConiferField, ConiferNoiseStyle, TreeRowStyle, TreeShape, TreeStyle};

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

use crate::loading::{AppState, WorldInitSet};
use crate::prefs::{TrackPrefExt, retuned};

/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`. Общий и для
/// домов, и для крон по той же причине, что и само солнце ([`sun`]).
pub const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<surface::SurfaceMaterial>::default())
            .add_plugins(Material2dPlugin::<buildings::material::RoofMaterial>::default())
            .add_plugins(Material2dPlugin::<trees::CrownMaterial>::default())
            .init_resource::<SunStyle>()
            .init_resource::<SunOnMap>()
            .init_resource::<TreeStyle>()
            .init_resource::<TreeRowStyle>()
            .init_resource::<ConiferField>()
            .init_resource::<ConiferNoiseStyle>()
            .init_resource::<BuildingHeightMode>()
            .init_resource::<buildings::BuildingZoomBucket>()
            .init_resource::<cars::CarZoomBucket>()
            .init_resource::<CarStyle>()
            .init_resource::<wagons::WagonZoomBucket>()
            .init_resource::<parking::ParkingLayout>()
            .init_resource::<fences::FenceZoomBucket>()
            .init_resource::<RoofStyle>()
            .init_resource::<RoadStyle>()
            .init_resource::<SurfaceStyle>()
            .init_resource::<rail::RailZoomBucket>()
            .init_resource::<tram::TramZoomBucket>()
            .init_resource::<TramStyle>()
            .init_resource::<IndustryStyle>()
            .register_type::<SunStyle>()
            .register_type::<SunOnMap>()
            .register_type::<TreeStyle>()
            .register_type::<TreeRowStyle>()
            .register_type::<ConiferNoiseStyle>()
            .register_type::<TreeShape>()
            .register_type::<TreeRowPlacement>()
            .register_type::<BuildingHeightMode>()
            .register_type::<RoofStyle>()
            .register_type::<RoadStyle>()
            .register_type::<SurfaceStyle>()
            .register_type::<TramStyle>()
            .register_type::<IndustryStyle>()
            .register_type::<CarStyle>()
            .track_pref::<TreeStyle>()
            .track_pref::<TreeRowStyle>()
            .track_pref::<ConiferNoiseStyle>()
            .track_pref::<SunOnMap>()
            .track_pref::<BuildingHeightMode>()
            .track_pref::<RoofStyle>()
            .track_pref::<RoadStyle>()
            .track_pref::<SurfaceStyle>()
            .track_pref::<TramStyle>()
            .track_pref::<IndustryStyle>()
            .track_pref::<CarStyle>()
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
                    (
                        surface::init_surface_materials,
                        surface::init_flat_materials,
                        buildings::material::init_roof_material,
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
            .add_systems(PreUpdate, (sun::settle_sun, sun::apply_sun).chain())
            .add_systems(
                OnEnter(AppState::Playing),
                // набор деревьев собирается первым (лес плюс аллеи выбранной
                // политики), поле хвои решает форму кроны и потому считается по
                // уже собранному набору и до крон. Сами кроны спавнит
                // `rebuild_trees` — в свежем мире деспавнить ему нечего, а спавн
                // из одного места избавляет `spawn_map` от стиля деревьев и поля
                // хвои разом. Рельсы и трамвай спавнят `rebuild_rails` /
                // `rebuild_tram` по той же причине: ступень зума остаётся их
                // личным делом. Ступень перед сборкой ставится по камере, а
                // камера на стартовый вид — тоже в `Spawn`, отсюда `after`
                (
                    trees::recompose_row_trees,
                    trees::build_conifer_field,
                    zoom::seed_zoom_bucket::<buildings::BuildingLods>,
                    spawn::spawn_map,
                    zoom::seed_zoom_bucket::<cars::CarLods>,
                    cars::rebuild_cars,
                    zoom::seed_zoom_bucket::<wagons::WagonLods>,
                    wagons::rebuild_wagons,
                    zoom::seed_zoom_bucket::<fences::FenceLods>,
                    fences::rebuild_fences,
                    industry::rebuild_industry,
                    zoom::seed_zoom_bucket::<rail::RailLods>,
                    rail::rebuild_rails,
                    zoom::seed_zoom_bucket::<tram::TramLods>,
                    tram::rebuild_tram,
                    spawn::rebuild_tree_row_band,
                    trees::rebuild_trees,
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
                    // условий на них не надо
                    (
                        trees::recompose_row_trees,
                        trees::retune_conifer_field,
                        spawn::rebuild_tree_row_band,
                        trees::rebuild_trees,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing))
                        .run_if(trees::rebuilds_on()),
                    (
                        zoom::update_zoom_bucket::<buildings::BuildingLods>,
                        buildings::rebuild_buildings.run_if(buildings::rebuilds_on()),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    roads::rebuild_roads
                        .run_if(in_state(AppState::Playing))
                        .run_if(roads::rebuilds_on()),
                    // три слоя со своими таблицами зума, одной группой: у
                    // каждого ступень считается своим `update_zoom_bucket`, и
                    // пересборка идёт следом
                    (
                        zoom::update_zoom_bucket::<cars::CarLods>,
                        cars::rebuild_cars.run_if(cars::rebuilds_on()),
                        zoom::update_zoom_bucket::<wagons::WagonLods>,
                        wagons::rebuild_wagons.run_if(wagons::rebuilds_on()),
                        zoom::update_zoom_bucket::<fences::FenceLods>,
                        fences::rebuild_fences.run_if(fences::rebuilds_on()),
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
                    surface::retune_surface_materials.run_if(retuned::<SurfaceStyle>),
                    buildings::material::retune_roof_material
                        .run_if(buildings::material::retunes_on()),
                    // ступень зума считается каждый кадр (одно чтение камеры и
                    // сравнение), но пересборку запускает только её фактическая
                    // смена. Таблицы у путей и трамвая свои, и пороги в них не
                    // совпадают, поэтому и ступени считаются порознь
                    (
                        zoom::update_zoom_bucket::<rail::RailLods>,
                        rail::rebuild_rails.run_if(rail::rebuilds_on()),
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
