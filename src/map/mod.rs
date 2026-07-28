pub mod data;
pub mod osm;
mod systems;

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(AppState::Playing),
            systems::spawn_map.in_set(WorldInitSet::Spawn),
        );
    }
}
