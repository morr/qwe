mod meshing;
pub mod osm;
mod spawn;
mod trees;

pub use self::meshing::MeshBuilder;

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(AppState::Playing),
            spawn::spawn_map.in_set(WorldInitSet::Spawn),
        );
    }
}
