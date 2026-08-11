mod buildings;
pub mod footprint;
mod meshing;
pub mod osm;
mod roads;
mod spawn;
mod tram;
pub mod trees;

pub use self::buildings::{BuildingHeightMode, extrusion_lift};
pub use self::footprint::bridge_curb_width;
pub use self::meshing::{MeshBuilder, miter_offsets};
pub use self::osm::{TREE_DENSITY_MAX, TreeRowPlacement};
pub use self::roads::{RoadJoin, RoadSmoothing, RoadStyle};
pub use self::trees::{ConiferField, ConiferNoiseStyle, TreeRowStyle, TreeShape, TreeStyle};

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};
use crate::prefs::{TrackPrefExt, retuned};

/// Направление тени на всей карте: 30° вниз-вправо, нормировано. Один
/// источник света и на дома, и на кроны — держится здесь, у общего родителя
/// обоих, потому что разъехавшиеся тени видны на карте сразу.
const SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);
/// Цвет тени — альфа-эквивалент watabou-шного multiply `#9699AE`. Общий по
/// той же причине, что и [`SHADOW_DIR`].
const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TreeStyle>()
            .init_resource::<TreeRowStyle>()
            .init_resource::<ConiferField>()
            .init_resource::<ConiferNoiseStyle>()
            .init_resource::<BuildingHeightMode>()
            .init_resource::<RoadStyle>()
            .init_resource::<tram::TramZoomBucket>()
            .register_type::<TreeStyle>()
            .register_type::<TreeRowStyle>()
            .register_type::<ConiferNoiseStyle>()
            .register_type::<TreeShape>()
            .register_type::<TreeRowPlacement>()
            .register_type::<BuildingHeightMode>()
            .register_type::<RoadStyle>()
            .track_pref::<TreeStyle>()
            .track_pref::<TreeRowStyle>()
            .track_pref::<ConiferNoiseStyle>()
            .track_pref::<BuildingHeightMode>()
            .track_pref::<RoadStyle>()
            .add_systems(
                OnEnter(AppState::Playing),
                // набор деревьев собирается первым (лес плюс аллеи выбранной
                // политики), поле хвои решает форму кроны и потому считается по
                // уже собранному набору и до крон. Сами кроны спавнит
                // `rebuild_trees` — в свежем мире деспавнить ему нечего, а спавн
                // из одного места избавляет `spawn_map` от стиля деревьев и поля
                // хвои разом. Трамвай спавнит `rebuild_tram` по той же причине:
                // ступень зума остаётся его личным делом
                (
                    trees::recompose_row_trees,
                    trees::build_conifer_field,
                    spawn::spawn_map,
                    tram::rebuild_tram,
                    spawn::rebuild_tree_row_band,
                    trees::rebuild_trees,
                )
                    .chain()
                    .in_set(WorldInitSet::Spawn),
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
                    buildings::rebuild_buildings
                        .run_if(in_state(AppState::Playing))
                        .run_if(retuned::<BuildingHeightMode>),
                    roads::rebuild_roads
                        .run_if(in_state(AppState::Playing))
                        .run_if(retuned::<RoadStyle>),
                    // ступень зума считается каждый кадр (одно чтение камеры и
                    // сравнение), но пересборку трамвая запускает только её
                    // фактическая смена
                    (
                        tram::update_tram_zoom_bucket,
                        tram::rebuild_tram.run_if(retuned::<tram::TramZoomBucket>),
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing)),
                ),
            );
    }
}
