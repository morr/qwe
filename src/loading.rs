//! Состояния приложения: `Loading` (экран загрузки OSM-карты с прогрессом)
//! → `Playing`. Мир (карта, навигация, население) строится в
//! `OnEnter(Playing)`, когда `MapData` уже вставлена.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::map::osm::{JobState, MapLoadJob, start_load_thread};
use crate::ui::{UiOpacity, ui_color};

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

#[derive(Component)]
struct LoaderUiRoot;

#[derive(Component)]
struct LoaderText;

#[derive(Component)]
struct RetryButton;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .configure_sets(
                OnEnter(AppState::Playing),
                (WorldInitSet::Navmesh, WorldInitSet::Spawn).chain(),
            )
            .add_systems(OnEnter(AppState::Loading), (spawn_loader_ui, start_job))
            .add_systems(
                Update,
                (
                    bevy::dev_tools::states::log_transitions::<AppState>,
                    poll_job.run_if(in_state(AppState::Loading)),
                ),
            )
            .add_systems(OnExit(AppState::Loading), despawn_loader_ui);
    }
}

fn start_job(mut commands: Commands) {
    let job = MapLoadJob::default();
    start_load_thread(job.clone());
    commands.insert_resource(job);
}

fn spawn_loader_ui(mut commands: Commands) {
    let root = commands
        .spawn((
            LoaderUiRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.),
                height: percent(100.),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(16.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Heavy)),
            Name::new("loader"),
            children![(
                LoaderText,
                Text::new("Connecting to Overpass..."),
                TextFont {
                    font_size: FontSize::Px(22.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .id();

    let retry = commands
        .spawn((
            RetryButton,
            Button,
            Pickable::default(),
            Hovered::default(),
            Visibility::Hidden,
            Node {
                padding: UiRect {
                    top: px(6.),
                    right: px(14.),
                    bottom: px(6.),
                    left: px(14.),
                },
                ..default()
            },
            BackgroundColor(Color::srgb(0.25, 0.27, 0.3)),
            children![(
                Text::new("Retry"),
                TextFont {
                    font_size: FontSize::Px(16.),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ))
        .observe(on_retry)
        .id();
    commands.entity(root).add_child(retry);
}

/// Кнопка Retry: сброс состояния и новый поток загрузки.
fn on_retry(
    _activate: On<Activate>,
    job: Res<MapLoadJob>,
    mut buttons: Query<&mut Visibility, With<RetryButton>>,
) {
    *job.0.lock().unwrap() = JobState::Connecting;
    start_load_thread(job.clone());
    for mut visibility in &mut buttons {
        *visibility = Visibility::Hidden;
    }
}

fn poll_job(
    mut commands: Commands,
    job: Res<MapLoadJob>,
    mut next: ResMut<NextState<AppState>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<LoaderText>>,
    mut retry_buttons: Query<&mut Visibility, With<RetryButton>>,
) {
    let mut state = job.0.lock().unwrap();

    let (message, failed) = match &mut *state {
        JobState::Connecting => ("Connecting to Overpass...".to_string(), false),
        JobState::Downloading { bytes, total } => {
            let (bytes, total) = (*bytes, *total);
            let megabytes = bytes as f32 / 1_048_576.0;
            let percent = total
                .filter(|&total| bytes <= total && total > 0)
                .map(|total| format!(" ({:.0}%)", bytes as f32 / total as f32 * 100.0))
                .unwrap_or_default();
            (
                format!("Downloading map... {megabytes:.1} MB{percent}"),
                false,
            )
        }
        JobState::Parsing => ("Parsing map...".to_string(), false),
        JobState::Done(map) => {
            let map = map.take().expect("map already taken");
            info!(
                "osm map: {} buildings, {} water, {} parks, {} roads, {} walls, {} trees",
                map.buildings.len(),
                map.water.len(),
                map.parks.len(),
                map.roads.len(),
                map.walls.len(),
                map.trees.len(),
            );
            commands.insert_resource(*map);
            next.set(AppState::Playing);
            return;
        }
        JobState::Failed(message) => (format!("Map load failed: {message}"), true),
    };

    for (mut text, mut color) in &mut texts {
        if text.0 != message {
            text.0 = message.clone();
        }
        color.set_if_neq(TextColor(if failed {
            Color::srgb(0.95, 0.35, 0.3)
        } else {
            Color::WHITE
        }));
    }
    for mut visibility in &mut retry_buttons {
        let target = if failed {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
}

fn despawn_loader_ui(mut commands: Commands, roots: Query<Entity, With<LoaderUiRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}
