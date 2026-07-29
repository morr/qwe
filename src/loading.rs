//! Состояния приложения: `Loading` (экран загрузки OSM-карты с прогрессом)
//! → `Playing`. Мир (карта, навигация, население) строится в
//! `OnEnter(Playing)`, когда `MapData` уже вставлена.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;

use crate::city::City;
use crate::map::osm::{JobState, MapLoadJob, start_load_thread};
use crate::movement::{PathfindingRequest, PathfindingTask, SimPosition};
use crate::navigation::ArcNavmesh;
use crate::portal::PortalPos;
use crate::ui::{UiOpacity, ui_color};

#[derive(States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Loading,
    Playing,
}

/// Подсостояние `Playing`: `Warmup` — мир уже живёт, но экран загрузки ещё
/// висит, пока пешкам в кадре не просчитаны пути. Иначе первую секунду сцена
/// стоит колом: 20 000 заявок на поиск пути подаются в один кадр.
#[derive(SubStates, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[source(AppState = AppState::Playing)]
pub enum PlayPhase {
    #[default]
    Warmup,
    Live,
}

/// Потолок ожидания прогрева, сек: экран загрузки не должен зависнуть
/// навсегда, если пути почему-то не сходятся.
const WARMUP_TIMEOUT: f32 = 10.0;

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
            .add_sub_state::<PlayPhase>()
            .init_resource::<WarmupProgress>()
            .configure_sets(
                OnEnter(AppState::Playing),
                (WorldInitSet::Navmesh, WorldInitSet::Spawn).chain(),
            )
            .add_systems(
                OnEnter(AppState::Loading),
                (
                    spawn_loader_ui,
                    start_job,
                    reset_warmup,
                    warn_leftover_world_entities,
                ),
            )
            .add_systems(
                Update,
                (
                    bevy::dev_tools::states::log_transitions::<AppState>,
                    poll_job.run_if(in_state(AppState::Loading)),
                    poll_warmup.run_if(in_state(PlayPhase::Warmup)),
                ),
            )
            // экран загрузки живёт до конца прогрева, а не до выхода из
            // `Loading`: мир строится раньше, чем по нему можно ходить
            .add_systems(OnEnter(PlayPhase::Live), despawn_loader_ui);
    }
}

fn start_job(mut commands: Commands, navmesh: Res<ArcNavmesh>, city: Res<City>) {
    let job = MapLoadJob::default();
    start_load_thread(job.clone(), navmesh.0.clone(), *city);
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
    navmesh: Res<ArcNavmesh>,
    city: Res<City>,
    mut buttons: Query<&mut Visibility, With<RetryButton>>,
) {
    *job.0.lock().unwrap() = JobState::Connecting;
    start_load_thread(job.clone(), navmesh.0.clone(), *city);
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
        JobState::BuildingNavmesh => ("Building navmesh...".to_string(), false),
        JobState::Pruning => ("Pruning unreachable areas...".to_string(), false),
        JobState::Done(world) => {
            let world = *world.take().expect("world already taken");
            let map = world.map;
            info!(
                "osm map: {} buildings, {} water, {} parks, {} woods, {} grass, {} sand, \
                 {} roads, {} walls, {} trees",
                map.buildings.len(),
                map.water.len(),
                map.parks.len(),
                map.woods.len(),
                map.grass.len(),
                map.sand.len(),
                map.roads.len(),
                map.walls.len(),
                map.trees.len(),
            );
            commands.insert_resource(map);
            commands.insert_resource(PortalPos(world.portal));
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

/// Прогрев: держим экран загрузки, пока у пешек в кадре есть незакрытые
/// заявки на поиск пути. Ждать заявок вне кадра бессмысленно — диспетчер их
/// и не запускает, пока камера не приедет.
///
/// `seen_requests` нужен, потому что в первый кадр после спавна заявки ещё
/// не вставлены (команды `pick_wander_targets` применяются в конце кадра),
/// и без него прогрев закончился бы, не начавшись.
fn poll_warmup(
    time: Res<Time<Real>>,
    mut progress: ResMut<WarmupProgress>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    pending: Query<&SimPosition, Or<(With<PathfindingRequest>, With<PathfindingTask>)>>,
    mut texts: Query<&mut Text, With<LoaderText>>,
    mut next: ResMut<NextState<PlayPhase>>,
) {
    let WarmupProgress {
        elapsed,
        seen_requests,
    } = &mut *progress;
    *elapsed += time.delta_secs();

    let camera_position = camera.translation.truncate();
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x;
    let waiting = pending
        .iter()
        .filter(|sim_position| {
            let offset = (sim_position.0 - camera_position).abs();
            offset.x <= half_view.x && offset.y <= half_view.y
        })
        .count();
    *seen_requests |= waiting > 0;

    if (*seen_requests && waiting == 0) || *elapsed > WARMUP_TIMEOUT {
        if waiting > 0 {
            warn!("warmup timed out with {waiting} pawns still routing");
        } else {
            info!("warmup: pawns on screen routed in {:.2}s", *elapsed);
        }
        next.set(PlayPhase::Live);
        return;
    }

    for mut text in &mut texts {
        text.set_if_neq(Text(format!("Routing pawns... {waiting} left")));
    }
}

/// Состояние прогрева между кадрами: сколько он идёт и видели ли мы хоть
/// одну заявку на путь. Ресурс, а не `Local`: при смене города прогрев
/// начинается заново, а `Local` унёс бы в него истёкший таймаут прошлого.
#[derive(Resource, Default)]
struct WarmupProgress {
    elapsed: f32,
    seen_requests: bool,
}

fn reset_warmup(mut progress: ResMut<WarmupProgress>) {
    *progress = WarmupProgress::default();
}

/// Страховка от забытого `DespawnOnExit(AppState::Playing)`: к моменту, когда
/// мы снова в `Loading`, от старого города не должно остаться ни одной
/// сущности сцены. Всё, что имеет `Transform` и не является камерой или
/// UI-нодой, — это мир; если он пережил выход из `Playing`, при смене города
/// он останется поверх новой карты (или, хуже, будет ходить по ней).
fn warn_leftover_world_entities(
    leftovers: Query<(Entity, Option<&Name>), (With<Transform>, Without<Camera>, Without<Node>)>,
) {
    let total = leftovers.iter().count();
    if total == 0 {
        return;
    }
    let sample: Vec<String> = leftovers
        .iter()
        .take(5)
        .map(|(entity, name)| match name {
            Some(name) => name.to_string(),
            None => format!("{entity}"),
        })
        .collect();
    warn!(
        "world reload: {total} scene entities survived Playing (missing DespawnOnExit): {sample:?}"
    );
}

fn despawn_loader_ui(mut commands: Commands, roots: Query<Entity, With<LoaderUiRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}
