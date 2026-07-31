mod buildings;
mod meshing;
pub mod osm;
mod roads;
mod spawn;
pub mod trees;

pub use self::buildings::{BuildingHeightMode, extrusion_lift};
pub use self::meshing::MeshBuilder;
pub use self::osm::{TREE_DENSITY_MAX, TreeRowPlacement};
pub use self::roads::{RoadJoin, RoadSmoothing, RoadStyle};
pub use self::trees::{ConiferField, TreeShape, TreeStyle};

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};

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
            .init_resource::<ConiferField>()
            .init_resource::<BuildingHeightMode>()
            .init_resource::<RoadStyle>()
            .register_type::<TreeStyle>()
            .register_type::<TreeShape>()
            .register_type::<TreeRowPlacement>()
            .register_type::<BuildingHeightMode>()
            .register_type::<RoadStyle>()
            .add_systems(
                OnEnter(AppState::Playing),
                // набор деревьев собирается первым (лес плюс аллеи выбранной
                // политики), поле хвои решает форму кроны и потому считается по
                // уже собранному набору и до крон. Сами кроны спавнит
                // `rebuild_trees` — в свежем мире деспавнить ему нечего, а спавн
                // из одного места избавляет `spawn_map` от стиля деревьев и поля
                // хвои разом
                (
                    trees::recompose_row_trees,
                    trees::build_conifer_field,
                    spawn::spawn_map,
                    spawn::rebuild_tree_row_band,
                    trees::rebuild_trees,
                )
                    .chain()
                    .in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    // тумблер политики аллей меняет сам набор деревьев, так что
                    // пересборка идёт до крон; сама система выходит сразу, если
                    // политика не поехала, — отдельного условия на неё не надо
                    (
                        trees::recompose_row_trees,
                        spawn::rebuild_tree_row_band,
                        trees::rebuild_trees,
                    )
                        .chain()
                        .run_if(in_state(AppState::Playing))
                        .run_if(resource_changed::<TreeStyle>)
                        // ресурс «изменён» и в кадре, где он появился, — там
                        // кроны ещё не спавнены и пересобирать нечего
                        .run_if(not(resource_added::<TreeStyle>)),
                    buildings::rebuild_buildings
                        .run_if(in_state(AppState::Playing))
                        .run_if(resource_changed::<BuildingHeightMode>)
                        .run_if(not(resource_added::<BuildingHeightMode>)),
                    roads::rebuild_roads
                        .run_if(in_state(AppState::Playing))
                        .run_if(resource_changed::<RoadStyle>)
                        .run_if(not(resource_added::<RoadStyle>)),
                ),
            );
    }
}
