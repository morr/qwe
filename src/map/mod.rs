mod meshing;
pub mod osm;
mod spawn;
pub mod trees;

pub use self::meshing::MeshBuilder;
pub use self::trees::{TreeShape, TreeStyle};

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TreeStyle>()
            .register_type::<TreeStyle>()
            .register_type::<TreeShape>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn::spawn_map.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                trees::rebuild_trees
                    .run_if(in_state(AppState::Playing))
                    .run_if(resource_changed::<TreeStyle>)
                    // ресурс «изменён» и в кадре инициализации — там деревья
                    // уже спавнит spawn_map, пересобирать их незачем
                    .run_if(not(resource_added::<TreeStyle>)),
            );
    }
}
