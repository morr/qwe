// публичен по той же причине, что и `trees`: витрина `roof_gallery` строит
// свои дома его же вызовами (`push_flat_roof`, `material::RoofLook`)
pub mod buildings;
mod cars;
pub mod footprint;
mod meshing;
pub mod osm;
mod rail;
mod roads;
mod spawn;
mod surface;
mod tram;
pub mod trees;
mod zoom;

pub use self::buildings::material::RoofStyle;
pub use self::buildings::{BuildingHeightMode, extrusion_lift};
pub use self::meshing::{MeshBuilder, merge_close_points, miter_offsets};
pub use self::osm::{TREE_DENSITY_MAX, TreeRowPlacement};
pub use self::roads::{RoadJoin, RoadSmoothing, RoadStyle};
pub use self::spawn::{GROUND_COLOR, PARK_COLOR, WOOD_COLOR};
pub use self::surface::SurfaceStyle;
pub use self::tram::TramStyle;
pub use self::trees::{ConiferField, ConiferNoiseStyle, TreeRowStyle, TreeShape, TreeStyle};

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

use crate::loading::{AppState, WorldInitSet};
use crate::prefs::{TrackPrefExt, retuned};

/// Направление тени на всей карте: 30° вниз-вправо, нормировано. Один
/// источник света и на дома, и на кроны — держится здесь, у общего родителя
/// обоих, потому что разъехавшиеся тени видны на карте сразу.
const SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);

/// Высота солнца над горизонтом, градусы — вторая половина того же светила,
/// что задаёт [`SHADOW_DIR`]. Пятьдесят девять — полдень середины лета на
/// широте Тулы (54.2° с. ш.): именно в такой час и снимают город с воздуха,
/// тени коротки и ничего под ними не пропадает.
const SUN_ELEVATION_DEG: f32 = 59.0;

/// Метров тени на метр высоты — котангенс высоты солнца. Раньше здесь стояло
/// «0.6 метра тени на метр высоты» без вывода; это ровно то же число
/// (`1 / tan 59° = 0.601`), но теперь у него есть причина, и менять его
/// полагается через [`SUN_ELEVATION_DEG`].
pub fn shadow_length_scale() -> f32 {
    1.0 / SUN_ELEVATION_DEG.to_radians().tan()
}
/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`. Общий по
/// той же причине, что и [`SHADOW_DIR`].
pub const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<surface::SurfaceMaterial>::default())
            .add_plugins(Material2dPlugin::<buildings::material::RoofMaterial>::default())
            .init_resource::<TreeStyle>()
            .init_resource::<TreeRowStyle>()
            .init_resource::<ConiferField>()
            .init_resource::<ConiferNoiseStyle>()
            .init_resource::<BuildingHeightMode>()
            .init_resource::<buildings::BuildingZoomBucket>()
            .init_resource::<cars::CarZoomBucket>()
            .init_resource::<RoofStyle>()
            .init_resource::<RoadStyle>()
            .init_resource::<SurfaceStyle>()
            .init_resource::<rail::RailZoomBucket>()
            .init_resource::<tram::TramZoomBucket>()
            .init_resource::<TramStyle>()
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
            .track_pref::<TreeStyle>()
            .track_pref::<TreeRowStyle>()
            .track_pref::<ConiferNoiseStyle>()
            .track_pref::<BuildingHeightMode>()
            .track_pref::<RoofStyle>()
            .track_pref::<RoadStyle>()
            .track_pref::<SurfaceStyle>()
            .track_pref::<TramStyle>()
            // материалы поверхностей и кровель — один комплект на всё
            // приложение, слои всех городов берут хэндлы из него
            .add_systems(
                Startup,
                (
                    surface::init_surface_materials,
                    buildings::material::init_roof_material,
                ),
            )
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
                        .run_if(
                            retuned::<TreeStyle>
                                .or_else(retuned::<TreeRowStyle>)
                                .or_else(retuned::<ConiferNoiseStyle>),
                        ),
                    // ступень зума решает, стоит ли на крышах оборудование;
                    // порог редкий, а пересборка слоя — единственный способ его
                    // снять, как у пути и трамвая
                    (
                        zoom::update_zoom_bucket::<buildings::BuildingLods>,
                        buildings::rebuild_buildings.run_if(
                            retuned::<BuildingHeightMode>
                                .or_else(retuned::<buildings::BuildingZoomBucket>),
                        ),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    roads::rebuild_roads
                        .run_if(in_state(AppState::Playing))
                        .run_if(retuned::<RoadStyle>),
                    // машины — целый слой, который на общем плане не нужен
                    // вовсе; порог у него свой, ближе зданиевого
                    (
                        zoom::update_zoom_bucket::<cars::CarLods>,
                        cars::rebuild_cars.run_if(retuned::<cars::CarZoomBucket>),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                    // сила фактуры — юниформ материалов, а не меши: без
                    // привязки к состоянию, материалы живут вне мира
                    surface::retune_surface_materials.run_if(retuned::<SurfaceStyle>),
                    buildings::material::retune_roof_material.run_if(retuned::<RoofStyle>),
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
