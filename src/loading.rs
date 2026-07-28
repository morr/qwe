//! Состояния приложения: `Loading` (экран загрузки карты) → `Playing`.
//! Мир (карта, навигация, население) строится в `OnEnter(Playing)`.

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Loading,
    Playing,
}

/// Порядок инициализации мира в `OnEnter(Playing)`: navmesh заполняется
/// раньше спавнов — иначе население высадится в реку и стены.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorldInitSet {
    Navmesh,
    Spawn,
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .configure_sets(
                OnEnter(AppState::Playing),
                (WorldInitSet::Navmesh, WorldInitSet::Spawn).chain(),
            )
            .add_systems(
                Update,
                (
                    bevy::dev_tools::states::log_transitions::<AppState>,
                    finish_loading.run_if(in_state(AppState::Loading)),
                ),
            );
    }
}

/// Заглушка: сразу в `Playing`; настоящая загрузка OSM появится отдельно.
fn finish_loading(mut next: ResMut<NextState<AppState>>) {
    next.set(AppState::Playing);
}
