// публичен по той же причине, что и `trees`: витрина `roof_gallery` строит
// свои дома его же вызовами (`push_house`, `RoofShape`, `shape_facts`,
// `material::RoofLook`)
pub mod buildings;
// публичен по той же причине: витрина `car_gallery` расставляет ряды его же
// вызовом (`cars_mesh`)
pub mod cars;
pub mod footprint;
mod meshing;
pub mod osm;
mod parking;
mod paths;
mod pitch;
mod rail;
mod roads;
mod seed;
mod spawn;
mod sun;
mod surface;
mod tram;
pub mod trees;
mod zoom;

pub use self::buildings::material::RoofStyle;
pub use self::buildings::{
    BuildingHeightMode, BuildingPlan, BuildingZoomBucket, LayerCost, extrusion_lift, measure_layers,
};
pub use self::cars::{CarStyle, measure_cars};
// `RibbonCap`/`RibbonJoin` наружу — витринам, которые кладут ленту сами
// (`car_gallery` рисует под рядами саму проезжую часть)
pub use self::meshing::{
    MeshBuilder, RibbonCap, RibbonJoin, merge_close_points, min_area_rect, miter_offsets,
};
pub use self::osm::{TREE_DENSITY_MAX, TreeRowPlacement};
// `ROAD_COLOR` и `smooth_path` наружу по той же причине: ряд машин витрины
// обязан стоять на том же асфальте, что в городе, а асфальт — на той же
// сглаженной осевой
pub use self::roads::{ROAD_COLOR, RoadJoin, RoadSmoothing, RoadStyle, smooth_path};
pub use self::spawn::{GROUND_COLOR, PARK_COLOR, WOOD_COLOR};
pub use self::sun::{
    SunOnMap, SunStyle, apply_sun, shadow_dir, shadow_length_scale, sun_light, sun_stretch,
};
#[cfg(test)]
pub(crate) use self::sun::{default_sun, sun_at};
pub use self::surface::SurfaceStyle;
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
            .init_resource::<RoofStyle>()
            .init_resource::<RoadStyle>()
            .init_resource::<SurfaceStyle>()
            .init_resource::<rail::RailZoomBucket>()
            .init_resource::<tram::TramZoomBucket>()
            .init_resource::<TramStyle>()
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
            .add_systems(
                Update,
                (
                    // тумблеры состава (лес/аллеи/одиночные) и политика аллей
                    // меняют сам набор деревьев, так что пересборка идёт до
                    // крон; параметры шума и примесь пересемплируют поле хвои
                    // (`retune_conifer_field`) — тоже до крон. Обе системы
                    // выходят сразу, если их вход не поехал, — отдельных
                    // условий на них не надо
                    (
                        trees::recompose_row_trees,
                        trees::retune_conifer_field,
                        spawn::rebuild_tree_row_band,
                        trees::rebuild_trees,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing))
                        // `retuned`, а не `resource_changed`: в кадре, где
                        // настройки легли на ресурс, кроны ещё не спавнены и
                        // пересобирать нечего
                        // солнце меняет и кроны: тень дерева строится по нему
                        // же, только запечена в шаблон варианта
                        .run_if(
                            retuned::<TreeStyle>
                                .or_else(retuned::<TreeRowStyle>)
                                .or_else(retuned::<ConiferNoiseStyle>)
                                .or_else(retuned::<SunOnMap>),
                        ),
                    // ступень зума решает, стоит ли на крышах оборудование;
                    // порог редкий, а пересборка слоя — единственный способ его
                    // снять, как у пути и трамвая
                    (
                        zoom::update_zoom_bucket::<buildings::BuildingLods>,
                        buildings::rebuild_buildings.run_if(
                            retuned::<BuildingHeightMode>
                                .or_else(retuned::<SunOnMap>)
                                .or_else(retuned::<buildings::BuildingZoomBucket>),
                        ),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    roads::rebuild_roads
                        .run_if(in_state(AppState::Playing))
                        .run_if(retuned::<RoadStyle>),
                    // машины — целый слой, который на общем плане не нужен
                    // вовсе; порог у него свой, ближе зданиевого. Тумблер и
                    // ручка занятости идут одной регистрацией через `or_else`:
                    // две в одном расписании могли бы сработать в одном кадре
                    // и заспавнить слой дважды. `RoadStyle` здесь же: ряд стоит
                    // по сглаженной осевой, и смена Smoothing двигает его
                    // вместе с асфальтом
                    (
                        zoom::update_zoom_bucket::<cars::CarLods>,
                        cars::rebuild_cars.run_if(
                            retuned::<cars::CarZoomBucket>
                                .or_else(retuned::<CarStyle>)
                                .or_else(retuned::<RoadStyle>)
                                .or_else(retuned::<SunOnMap>),
                        ),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    // сила фактуры — юниформ материалов, а не меши: без
                    // привязки к состоянию, материалы живут вне мира
                    surface::retune_surface_materials.run_if(retuned::<SurfaceStyle>),
                    buildings::material::retune_roof_material
                        .run_if(retuned::<RoofStyle>.or_else(retuned::<SunOnMap>)),
                    // ступень зума считается каждый кадр (одно чтение камеры и
                    // сравнение), но пересборку запускает только её фактическая
                    // смена. Таблицы у путей и трамвая свои, и пороги в них не
                    // совпадают, поэтому и ступени считаются порознь
                    (
                        zoom::update_zoom_bucket::<rail::RailLods>,
                        rail::rebuild_rails.run_if(retuned::<rail::RailZoomBucket>),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    (
                        zoom::update_zoom_bucket::<tram::TramLods>,
                        // тумблер видимости идёт через ту же пересборку: она и
                        // деспавнит слой, и строит его заново
                        tram::rebuild_tram
                            .run_if(retuned::<tram::TramZoomBucket>.or_else(retuned::<TramStyle>)),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                ),
            );
    }
}
