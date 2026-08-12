//! Состояния приложения: `Loading` (экран загрузки OSM-карты с прогрессом)
//! → `Playing`. Мир (карта, навигация, население) строится в
//! `OnEnter(Playing)`, когда `MapData` уже вставлена.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;

use crate::camera::Viewport;
use crate::city::City;
use crate::map::osm::{JobState, MapLoadJob, OVERPASS_MIRRORS, start_load_thread};
use crate::movement::{
    PathfindingRequest, PathfindingTask, SimPosition, UrgentPath, wanderers_dispatched_at_zoom,
};
use crate::navigation::{ArcNavmesh, Pathfinder};
use crate::portal::PortalPos;
use crate::ui::{UiOpacity, ui_color};

#[derive(States, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Loading,
    Playing,
}

/// «Мир существует» — условие для всего, что живёт только внутри мира.
///
/// Ресурсы мира (`Backend` прежде всего) заводятся на `OnEnter(Playing)` и
/// снимаются на выходе, поэтому система симуляции, отработавшая в `Loading`,
/// валится на валидации параметров — и однажды уже уронила запуск. Гейт от этого
/// стоял рукописным `run_if(in_state(AppState::Playing))` на каждой такой
/// системе: двадцать пять копий в одиннадцати файлах, девять из них — на одной
/// цепочке `FixedUpdate`. Забыть одну было нечем помешать.
///
/// **`Option` намеренно.** `MovementPlugin` и его соседей поднимают в тестах и
/// на стендах вообще без состояний; там `in_state` не просто ложен, а валит
/// кадр целиком. Нет `AppState` — значит мир и есть весь `App`, и гейтить
/// нечего. Тот же приём, что у `navigation::ContinuousSpace`.
pub fn in_world(state: Option<Res<State<AppState>>>) -> bool {
    state.is_none_or(|state| *state.get() == AppState::Playing)
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

/// Мир начал новый прогон: сцена собрана, симуляция сейчас поедет.
///
/// Триггерится ровно из двух мест: вход в `PlayPhase::Live` (первый запуск и
/// смена города) и рестарт по R (`restart.rs::on_restart` — тот состояний не
/// проходит и потому в `OnEnter` не попадает). Всё состояние прогона — часы,
/// тики, счётчики, замороженный бэкенд — сбрасывают обсерверы этого события,
/// каждый в своём модуле; перечисляет их `grep "On<WorldStarted>"`.
///
/// **Список никто не ведёт руками, и полагаться на него не нужно.** Членство
/// держит снаружи `tests/determinism.rs::a_restart_replays_the_run`: второй
/// прогон в ТОМ ЖЕ `App`, отпечаток в отпечаток. Забытый сброс расходится в
/// отпечатке независимо от того, вспомнил ли кто-нибудь его записать —
/// проверено мутациями: снять сброс телеметрии, спавнера демонов или тиков, и
/// тест падает. Регистрация вида `reset_on_world_start::<T>()` этой гарантии
/// не добавила бы: забыть зарегистрировать — тот же промах, что забыть
/// обсервер.
///
/// Чего отпечаток не видит — состояния, невидимого ни симуляции, ни счётчикам
/// исходов. Такому нужен свой пин рядом с обсервером: у регулятора это
/// `sim_time::tests::a_new_run_inherits_no_clock_and_no_regulator_memory`, у
/// замороженного бэкенда —
/// `determinism::tests::the_frozen_backend_is_snapped_at_the_announcement_not_at_world_entry`
/// (двор повтора пинит плоский A*, поэтому посевной и объявленный снимки в нём
/// совпадают по построению — сброс бэкенда снаружи не проверяется вовсе).
#[derive(Event, Debug)]
pub struct WorldStarted;

/// Потолок ожидания прогрева, сек: экран загрузки не должен зависнуть
/// навсегда, если пути почему-то не сходятся.
const WARMUP_TIMEOUT: f32 = 10.0;
/// Сколько ждать появления первой заявки, сек. Заявки вставляются командами в
/// конце первого кадра мира, так что «заявок нет» в первые кадры — ещё не
/// «ждать нечего»; после этого срока — уже да.
const WARMUP_GRACE: f32 = 0.5;

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

/// Как заводится мир — одной реализацией на игру и на приложение повтора.
///
/// Всё, что здесь лежит, — не «загрузка», а **жизненный цикл прогона**:
/// состояния и их порядок, порядок инициализации мира и границы прогрева.
/// Сцена повтора (`determinism::replay`) собирает всё остальное сама (своя
/// карта, свой seed, ни окна, ни экрана загрузки), но заводиться обязана так
/// же — иначе получается «мир едет, просто не тот», а молчаливого «мы забыли»
/// в нём не видно. Раньше она пересобирала эту последовательность руками, и
/// оба дефекта, на которых сработал `a_restart_replays_the_run`, были именно
/// в той пересборке.
///
/// **Прогрев идёт на паузе.** Мир уже собран, но за экраном загрузки ему
/// двигаться незачем: пусть пешки сначала получат пути. Пауза живёт здесь, а
/// не в `SimTimePlugin`, хотя крутит она `Time<Virtual>`: это свойство фазы, а
/// не регулятора скорости, — и повтор регулятор брать не может (тот меряет
/// настенные часы), из-за чего до переноса тикал в прогреве, а игра не тикала.
pub struct SimBootPlugin;

impl Plugin for SimBootPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .add_sub_state::<PlayPhase>()
            .configure_sets(
                OnEnter(AppState::Playing),
                (WorldInitSet::Navmesh, WorldInitSet::Spawn).chain(),
            )
            .add_systems(OnEnter(PlayPhase::Warmup), pause_world)
            // объявление — ПЕРЕД снятием паузы и, что важнее, до первого тика
            // ЖИВОГО мира: `OnEnter` идёт в `StateTransition`, то есть раньше
            // `RunFixedMainLoop`, и команды этого расписания применяются в его
            // же конце — так что обсерверы `WorldStarted` успевают.
            //
            // На одном этом порядке держаться нельзя, и однажды не удержалось:
            // тик В ПРОГРЕВЕ идёт раньше любого `OnEnter(Live)`, и спавнер
            // демонов выпускал на нём залп со старым счётчиком, а сброс раздавал
            // те же `PawnId` второму залпу. Прогрев держит мир на паузе, но
            // пауза — ресурс `sim_time`, снимаемый пробелом. Поэтому всё, чей
            // смысл зависит от объявленного старта, гейтится фазой `Live`
            // само (`demon::DemonPlugin`), а не полагается на этот порядок
            .add_systems(
                OnEnter(PlayPhase::Live),
                (announce_world_start, resume_world).chain(),
            );
    }
}

/// Прогрев: мир стоит. Заявки на путь при этом идут — их подача и
/// диспетчеризация живут в `Update`, а стоит только `FixedUpdate`. В
/// детерминированном режиме заявки на паузе не идут (там весь конвейер в
/// `FixedUpdate`), поэтому пешечного прогрева в нём нет вовсе — см.
/// [`poll_warmup`]; паузу это не отменяет.
fn pause_world(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

/// Прогрев кончился — мир поехал. Скорость нового прогона выставляет обсервер
/// `WorldStarted` того же перехода.
fn resume_world(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SimBootPlugin)
            .init_resource::<WarmupProgress>()
            .add_systems(
                OnEnter(AppState::Loading),
                (
                    spawn_loader_ui,
                    (sync_navtile_size, start_job).chain(),
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
            // `Loading`: мир строится раньше, чем по нему можно ходить.
            // Само объявление старта мира — в `SimBootPlugin`: оно нужно и
            // сцене повтора, у которой этого экрана нет
            .add_systems(OnEnter(PlayPhase::Live), despawn_loader_ui);
    }
}

fn start_job(mut commands: Commands, navmesh: Res<ArcNavmesh>, city: Res<City>) {
    let job = MapLoadJob::default();
    start_load_thread(job.clone(), navmesh.0.clone(), *city);
    commands.insert_resource(job);
}

/// Единственная точка записи атомика размера навтайла — перед стартом потока
/// загрузки, когда ни заливка, ни генерация входов ещё не живы. Покрывает и
/// первый запуск (настройки восстановлены при сборке `App`, до расписаний),
/// и каждую перезагрузку мира.
fn sync_navtile_size(base: Res<crate::settings::NavtileBase>) {
    crate::settings::set_navtile_size(base.size());
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
                Text::new("Waiting for Overpass..."),
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
    *job.0.lock().unwrap() = JobState::Connecting { attempt: 1 };
    start_load_thread(job.clone(), navmesh.0.clone(), *city);
    for mut visibility in &mut buttons {
        *visibility = Visibility::Hidden;
    }
}

/// Хвост строки загрузки со скоростью. Пусто, пока не набралось первое окно
/// замера (`SPEED_WINDOW`) — иначе первые кадры показывали бы «0.0 MB/s».
/// Делитель тот же, что и у счётчика мегабайт, — мебибайт под подписью «MB».
/// Разделитель — ASCII: в `default_font` нет `·`, он рисовался квадратом.
fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        return String::new();
    }
    if bytes_per_sec >= 1_048_576.0 {
        format!(" - {:.1} MB/s", bytes_per_sec / 1_048_576.0)
    } else {
        format!(" - {:.0} KB/s", bytes_per_sec / 1024.0)
    }
}

fn poll_job(
    mut commands: Commands,
    job: Res<MapLoadJob>,
    time: Res<Time<Real>>,
    mut connecting_since: Local<Option<(usize, f32)>>,
    mut next: ResMut<NextState<AppState>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<LoaderText>>,
    mut retry_buttons: Query<&mut Visibility, With<RetryButton>>,
) {
    let mut state = job.0.lock().unwrap();

    // счётчик ожидания живёт только внутри `Connecting`, иначе следующий
    // город (или Retry) унаследовал бы отсчёт от предыдущей загрузки
    if !matches!(*state, JobState::Connecting { .. }) {
        *connecting_since = None;
    }

    let (message, failed) = match &mut *state {
        JobState::Connecting { attempt } => {
            let attempt = *attempt;
            // `Time<Real>`: во время загрузки виртуальное время на паузе
            let now = time.elapsed_secs();
            let started = match *connecting_since {
                Some((seen, started)) if seen == attempt => started,
                // новое зеркало — счёт с нуля, а не с начала всей загрузки
                _ => {
                    *connecting_since = Some((attempt, now));
                    now
                }
            };
            let mirror = if attempt > 1 {
                format!(" (mirror {attempt}/{OVERPASS_MIRRORS})")
            } else {
                String::new()
            };
            // не «Connecting»: TCP+TLS занимают ~0.2 с, всё остальное время
            // Overpass считает запрос и не отдаёт ни байта
            (
                format!(
                    "Waiting for Overpass... {:.0}s{mirror}",
                    (now - started).max(0.0)
                ),
                false,
            )
        }
        JobState::Downloading {
            bytes,
            total,
            bytes_per_sec,
        } => {
            let (bytes, total) = (*bytes, *total);
            let megabytes = bytes as f32 / 1_048_576.0;
            let percent = total
                .filter(|&total| bytes <= total && total > 0)
                .map(|total| format!(" ({:.0}%)", bytes as f32 / total as f32 * 100.0))
                .unwrap_or_default();
            let speed = format_speed(*bytes_per_sec);
            (
                format!("Downloading map... {megabytes:.1} MB{percent}{speed}"),
                false,
            )
        }
        JobState::Parsing => ("Parsing map...".to_string(), false),
        JobState::BuildingNavmesh => ("Building navmesh...".to_string(), false),
        JobState::Pruning => ("Pruning unreachable areas...".to_string(), false),
        JobState::Done(world) => {
            let world = *world.take().expect("world already taken");
            let map = world.map;
            // высота в OSM опциональна и покрыта очень неровно (Берлин 80%,
            // Токио 5%) — процент в логе объясняет плоскую карту без гаданий
            let with_height = map
                .buildings
                .iter()
                .filter(|building| building.height.is_some())
                .count();
            let entrances: usize = map
                .buildings
                .iter()
                .map(|building| building.entrances.len())
                .sum();
            // трубы считаются отдельно: только они из водотоков не блокируют
            // навмеш, и когда русло вдруг режет город, первый вопрос — сколько
            // переходов ушло в культверты
            let culverts = map.water_lines.iter().filter(|line| line.tunnel).count();
            info!(
                "osm map: {} buildings ({with_height} with height, {entrances} entrances), \
                 {} water, {} waterways ({culverts} culverts), {} parks, {} woods, {} grass, \
                 {} sand, {} roads, {} rails, {} walls, {} trees",
                map.buildings.len(),
                map.water.len(),
                map.water_lines.len(),
                map.parks.len(),
                map.woods.len(),
                map.grass.len(),
                map.sand.len(),
                map.roads.len(),
                map.rails.len(),
                map.walls.len(),
                map.trees.len(),
            );
            commands.insert_resource(map);
            commands.insert_resource(PortalPos(world.portal));
            *connecting_since = None;
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
/// По той же причине ждать можно только то, что диспетчер вообще берёт: выше
/// `WANDER_DISPATCH_MAX_ZOOM` мирные гуляющие пути не получают, и их заявки
/// не закроются никогда — экран загрузки висел бы все 10 с таймаута с
/// неподвижным счётчиком (камера, восстановленная в режиме `save` на общем
/// плане, попадала в это ровно всегда).
///
/// `seen_requests` нужен, потому что в первый кадр после спавна заявки ещё
/// не вставлены (команды `pick_wander_targets` применяются в конце кадра),
/// и без него прогрев закончился бы, не начавшись. Ждать их появления, однако,
/// можно только `WARMUP_GRACE`: на общем плане ждать нечего в принципе, и без
/// этого срока экран загрузки простоял бы весь таймаут с нулём на счётчике.
#[allow(clippy::too_many_arguments)]
fn poll_warmup(
    time: Res<Time<Real>>,
    mut progress: ResMut<WarmupProgress>,
    camera: Single<&Transform, With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    pending: Query<
        (&SimPosition, Has<UrgentPath>),
        Or<(With<PathfindingRequest>, With<PathfindingTask>)>,
    >,
    mut texts: Query<&mut Text, With<LoaderText>>,
    mut next: ResMut<NextState<PlayPhase>>,
    determinism: Option<Res<crate::determinism::Determinism>>,
    pathfinder: Pathfinder,
) {
    if determinism.is_some_and(|mode| mode.0) {
        // Детерминированный прогон идёт на одном бэкенде от начала до конца
        // (`Backend` замораживается на входе в `Live`), поэтому вход в мир
        // ждёт, пока выбранный бэкенд построится. Счётчик `elapsed` при этом
        // стоит: постройка иерархии занимает ~11–14 с, и `WARMUP_TIMEOUT` (10 с)
        // оборвал бы её на полпути — ровно то, чего ждём.
        if pathfinder.mode().is_building() {
            for mut text in &mut texts {
                text.set_if_neq(Text("Building navigation...".to_string()));
            }
            return;
        }
        // ПЕШЕЧНОГО прогрева в этом режиме нет, и это не упущение.
        //
        // Ждать здесь нечего и нечем. Нечем: весь конвейер поиска пути —
        // подача, диспетчер, приёмка — живёт в `FixedUpdate`, а тот на паузе
        // прогрева стоит; счётчик заявок физически не мог сдвинуться, и
        // прогрев выжигал все `WARMUP_TIMEOUT` в лог строкой «warmup timed out
        // with 301 pawns still routing». Нечего: «пешки в кадре» — понятие
        // камерное, а число тиков до входа в мир в этом режиме не имеет права
        // зависеть от того, куда смотрит игрок.
        //
        // Снимать вместо этого паузу нельзя: мир поехал бы за экраном
        // загрузки, пешки доходили бы до целей и просили новые пути, и
        // счётчик колебался бы у нуля бесконечно — условие «никто не ждёт» для
        // движущегося мира не наступает.
        //
        // Толпа на входе в мир при этом не стоит: диспетчер выдаёт первые
        // пути метрономом (`PATHFINDING_WANDER_UNITS_PER_TICK`), и население
        // трогается с места волной за пару секунд, а не разом.
        info!("warmup: skipped, deterministic mode routes pawns on the clock");
        next.set(PlayPhase::Live);
        return;
    }

    let WarmupProgress {
        elapsed,
        seen_requests,
    } = &mut *progress;
    *elapsed += time.delta_secs();

    // ровно один экран, без запаса диспетчера: вопрос прогрева — «видит ли
    // игрок стоящую пешку», а не «возьмёт ли её диспетчер»; заявки в полосе
    // запаса он выдаст сам, пока игрок смотрит на середину кадра
    let view = Viewport::of(&window, &camera, 1.0);
    let wanderers_dispatched = wanderers_dispatched_at_zoom(view.zoom);
    let waiting = pending
        .iter()
        .filter(|(sim_position, urgent)| {
            // мирного гуляющего на общем плане диспетчер не возьмёт. Срочность
            // — тот же признак, по которому её решает сам диспетчер
            // (`movement::UrgentPath`), а не третья копия его правила
            if !*urgent && !wanderers_dispatched {
                return false;
            }
            view.contains(sim_position.0)
        })
        .count();
    *seen_requests |= waiting > 0;

    let nothing_to_wait_for = waiting == 0 && (*seen_requests || *elapsed > WARMUP_GRACE);
    if nothing_to_wait_for || *elapsed > WARMUP_TIMEOUT {
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

fn announce_world_start(mut commands: Commands) {
    commands.trigger(WorldStarted);
}

fn despawn_loader_ui(mut commands: Commands, roots: Query<Entity, With<LoaderUiRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сколько фиксированных шагов прошло — счётчик вместо всей симуляции.
    #[derive(Resource, Default)]
    struct Ticks(u32);

    fn boot_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .add_plugins(SimBootPlugin)
            .init_resource::<Ticks>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0),
            ))
            .add_systems(FixedUpdate, |mut ticks: ResMut<Ticks>| ticks.0 += 1);
        app
    }

    fn enter(app: &mut App, state: AppState) {
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(state);
        app.update();
    }

    /// «Мир за экраном загрузки не двигается» — инвариант фазы, а не
    /// регулятора скорости.
    ///
    /// Держался он раньше на `SimTimePlugin`, которого сцена повтора брать не
    /// может (её регулятор мерил бы настенные часы), — и та тикала в прогреве,
    /// в отличие от игры. Из-за этого ей приходилось объявлять старт мира
    /// руками, до первого кадра.
    #[test]
    fn the_world_does_not_move_during_warmup() {
        let app = &mut boot_app();

        enter(app, AppState::Playing);

        assert_eq!(
            app.world().resource::<Ticks>().0,
            0,
            "прогрев обязан идти на паузе"
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
    }

    /// …и трогается ровно на входе в `Live`.
    #[test]
    fn entering_live_starts_the_world() {
        let app = &mut boot_app();
        enter(app, AppState::Playing);

        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        app.update();

        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(
            app.world().resource::<Ticks>().0,
            0,
            "снятие паузы в `StateTransition` действует со СЛЕДУЮЩЕГО кадра: \
             виртуальное время текущего посчитано раньше, в `First`"
        );

        app.update();

        assert!(app.world().resource::<Ticks>().0 > 0, "а дальше мир едет");
    }

    /// Старт мира объявляется до первого шага симуляции: на первом тике
    /// спавнер выпускает залп демонов, и сброс спавнера после него раздал бы
    /// второму залпу те же `PawnId`.
    #[test]
    fn the_world_start_is_announced_before_the_first_tick() {
        #[derive(Resource, Default)]
        struct AnnouncedAt(Option<u32>);

        let app = &mut boot_app();
        app.init_resource::<AnnouncedAt>().add_observer(
            |_: On<WorldStarted>, ticks: Res<Ticks>, mut at: ResMut<AnnouncedAt>| {
                at.0 = Some(ticks.0);
            },
        );
        enter(app, AppState::Playing);

        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        app.update();

        assert_eq!(
            app.world().resource::<AnnouncedAt>().0,
            Some(0),
            "объявление обязано случиться раньше первого фиксированного шага"
        );
    }
}
