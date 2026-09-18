//! Мягкое расталкивание пешек (anti-overlap): пешки в кадре не стоят друг на
//! друге. Пути идут через центры навтайлов, спавн и паника сгоняют толпу в
//! одну точку — и без «личного пространства» пешки регулярно сливаются в одну.
//!
//! Механизм намеренно локальный и косметический:
//! - **только полигональная навигация** ([`separation_runs`]) — на сетке
//!   waypoint'ы стоят в центрах навтайлов, и ходьба возвращает разведённую пару
//!   на них же; там расталкивать некуда;
//! - **только вьюпорт и близкий зум** — за кадром и на отдалении перекрытие не
//!   видно, и симуляция за него не платит; пачка, въехавшая в кадр при движении
//!   камеры, расходится на глазах за доли секунды;
//! - **не чаще раза в кадр** — система живёт в `FixedUpdate` (ей нужен момент
//!   после `move_moving_entities`), но на 30x там ~1920 тиков в секунду, и
//!   даже 0.03 мс на тик съели бы ~6% реальной секунды. Чаще кадра
//!   расталкивание физически не видно, а на скорости 1x кадр ≈ тик, так что в
//!   режиме «медленно разглядываю толпу» гейт ничего не отнимает;
//! - **мягкое и одинаковое на всех скоростях** — перекрытие затухает как
//!   `exp(-SEPARATION_RATE · t)` виртуального времени под потолком скорости
//!   [`SEPARATION_MAX_SPEED`](crate::settings::SEPARATION_MAX_SPEED);
//!   [`SEPARATION_MAX_STEP`](crate::settings::SEPARATION_MAX_STEP) — страховка от
//!   телепорта, не потолок. Прогон инвариантен к нарезке dt: один кадр на
//!   30× и тридцать кадров на 1× дают одну траекторию
//!   ([`relaxation_fraction`], [`clamped_step`]). В давке мгновенное
//!   перекрытие возможно, но живёт доли виртуальной секунды; жёсткая
//!   релаксация до полного разведения разлеталась бы цепными толчками, как
//!   телепорт.
//!
//! Одно намеренное исключение из «косметического»: пешка, чей курс упирается в
//! перекрытого соседа, до следующего прогона ходит ослабленным шагом
//! ([`SeparationHolds`], читает `move_moving_entities`) — иначе ходьба и
//! расталкивание гасят друг друга, и затор стоит вечно. В детерминированном
//! режиме набор придержанных пуст вместе со всем механизмом.
//!
//! Толчок пишется в `SimPosition` после шага движения: снимок
//! `PreviousSimPosition` сделан в начале тика, так что интерполяция доводит
//! сдвиг до экрана плавно, а троттлимая перепрокладка путей (0.4–1.2 с) съедает
//! боковой дрейф — накапливаться ему не во что. Демон в броске
//! (`DemonLungeTag`) исключён целиком: он двигает `SimPosition` сам и обязан
//! сомкнуться до `KILL_DISTANCE`. Пожирающий (`DemonDevourTag`) стоит над
//! трупом неподвижно (mobility 0), но толпу от себя отталкивает. Трупы вне
//! механизма по построению — у них нет `SimPosition`.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use super::VIEW_MARGIN;
use crate::camera::Viewport;
use crate::demon::{Demon, DemonDevourTag, DemonLungeTag};
use crate::grid::world_to_tile;
use crate::human::{HUMAN_BODY_RADIUS, Human};
use crate::loading::WorldStarted;
use crate::movement::components::SimPosition;
use crate::navigation::ContinuousSpace;
use crate::settings::{DEMON_BODY_RADIUS, DEMON_MOBILITY, SEPARATION_CELL, SEPARATION_MAX_ZOOM};
use crate::spatial::SpatialGrid;

// Дефолты ручек уехали в `tuning`, и константы под ними этот файл больше не
// читает — а `tests.rs` их через `use super::*` читает по-прежнему (пин
// «дефолт равен константе»). Импорт под `cfg(test)`, чтобы в обычной сборке он
// не висел неиспользованным.
#[cfg(test)]
use crate::settings::{
    SEPARATION_LEFT_SHARE, SEPARATION_MAX_SPEED, SEPARATION_MAX_STEP, SEPARATION_PASS_SQUEEZE,
    SEPARATION_RATE, SEPARATION_SIDESTEP, SEPARATION_STEER,
};

mod pairs;
mod ports;
mod solver;
mod tuning;

// Ручки и порты лежат отдельно, но снаружи механизм остаётся одним модулем:
// читатели (`movement/systems.rs`, вкладка Nav, стенд) ходят за именами сюда.
pub use self::ports::*;
pub use self::tuning::*;

// Приватные реэкспорты: набор имён снаружи тот же, что до разрезания, а
// `use super::*` в `tests.rs` продолжает доставать `Pawn` и `resolve_pushes`.
use self::pairs::{Pawn, damp_along_heading};
use self::solver::{
    SeparationState, Tuning, advance_stuck, clamped_step, relaxation_fraction, resolve_pushes,
};

/// Работает ли расталкивание в текущем режиме мира — вопрос не тумблера
/// [`SeparationStyle`], а того, чем считается путь и повторяем ли прогон.
///
/// Два «нет», и оба не про вкус пользователя:
/// - **детерминированный режим** — механизм завязан на камеру, зум и
///   `FrameCount`, то есть на всё, от чего повтор прогона обязан не зависеть;
/// - **тайловое пространство** ([`ContinuousSpace`] — бэкенд `Navmesh` в
///   строке `Algo` вкладки Nav) — путь
///   по сетке идёт центрами навтайлов, и `move_moving_entities` ставит пешку
///   на waypoint каждый шаг: разведённая пара возвращается на те же два
///   центра к следующему тику, а всё, что успело набежать, — это дрожь и
///   придержки ([`SeparationHolds`]) на ровном месте. Личное пространство
///   имеет смысл там, где waypoint'ы метрические; почему отвечает тумблер, а
///   не готовность меша — док [`ContinuousSpace`].
///
/// Единственное место, где режим читается не множеством
/// [`SimPipeline`](crate::determinism::SimPipeline): условие нужно и в
/// **отрицании** — «расталкивания нет, почисти его следы», — а отрицать
/// множество нельзя. Обе стороны правила поэтому сведены сюда, а не разложены
/// по двум веткам конвейера.
pub fn separation_runs(
    determinism: Res<crate::determinism::Determinism>,
    space: ContinuousSpace,
) -> bool {
    separation_allowed_by_mode(determinism.0, space.is_continuous())
}

/// То же правило в чистом виде — для вкладки Nav: строка тумблера гаснет
/// ровно тогда, когда система не работает, и это должно быть одно правило, а
/// не две разъезжающиеся копии (`ui/stats.rs`).
pub fn separation_allowed_by_mode(deterministic: bool, polymesh_nav: bool) -> bool {
    !deterministic && polymesh_nav
}

/// Во столько раз радиус демона больше человеческого — как и спрайты
/// ([`DEMON_SIZE`] против [`HUMAN_SIZE`]).
pub const DEMON_RADIUS_RATIO: f32 = DEMON_BODY_RADIUS / HUMAN_BODY_RADIUS;

/// Радиус тела демона по радиусу тела человека — не отдельная ручка: он всегда
/// вдвое больше, как и спрайт.
pub fn demon_radius(human_radius: f32) -> f32 {
    human_radius * DEMON_RADIUS_RATIO
}

/// Сторона одноразовой мелкой сетки соседей. Считается от радиуса, а не
/// берётся константой: ячейка ОБЯЗАНА быть не меньше максимальной суммы
/// радиусов (демон+демон), иначе перекрывшаяся пара не попадёт в общие
/// 3 × 3 ячейки и её не найдут. С ручкой радиуса константа рано или поздно
/// оказалась бы мала.
pub fn separation_cell(human_radius: f32) -> f32 {
    (demon_radius(human_radius) * 2.0).max(SEPARATION_CELL)
}

/// Итоги ПРОГОНА МИРА — для стенда: сколько работы сделано и сколько движения
/// ушло в толчки, а не в ходьбу. Пишется одной строкой в конце
/// [`separate_pawns`]; в игре не читается никем.
///
/// Все поля копятся, поэтому обнуляет их [`on_world_started`]: это состояние
/// прогона, как `Telemetry`, а не выдача шагу движения (`SeparationHolds` и
/// соседи), которую чистят на входе в мир из-за мёртвых сущностей.
#[derive(Resource, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Resource, Default)]
pub struct SeparationStats {
    pub runs: u64,
    /// Пары, которым выдан толчок перекрытия.
    pub overlapping_pairs: u64,
    /// Пары, попавшие только в упреждение (сближаются, но ещё не перекрылись).
    pub anticipated_pairs: u64,
    /// Суммарная длина применённых толчков, м. Это движение, которое пешки
    /// потратили НЕ на дорогу.
    pub push_metres: f64,
    /// Самый длинный одиночный толчок за прогон мира, м — детектор телепорта.
    /// «За прогон», а не за сеанс: иначе одна плохая секунда в первом городе
    /// объявляла бы телепорт во всех следующих.
    pub worst_push: f32,
}

/// Новый прогон мира — счётчики расталкивания с нуля.
///
/// Шов именно `WorldStarted`, а не `OnEnter(AppState::Playing)`, которым
/// чистятся карты выдачи: рестарт по R состояния не меняет вовсе
/// (`restart.rs::on_restart`), и на `OnEnter` счётчики пережили бы его —
/// а стенд читает их как числа одного прогона.
///
/// Отпечаток `a_restart_replays_the_run` этого сброса не стережёт: под
/// детерминизмом расталкивания нет вовсе ([`separation_runs`]).
pub(super) fn on_world_started(_event: On<WorldStarted>, mut stats: ResMut<SeparationStats>) {
    *stats = SeparationStats::default();
}

/// Прогон расталкивания: гейты → сбор видимых из грубых сеток → толчки →
/// применение с проверкой проходимости. Порядок в тике — строго после
/// `move_moving_entities` (см. цепочку в `movement/mod.rs`).
#[allow(clippy::too_many_arguments)]
pub fn separate_pawns(
    mut diagnostics: bevy::diagnostic::Diagnostics,
    style: Res<SeparationStyle>,
    // радиус тела живёт в настройках ЛЮДЕЙ, а не расталкивания: это свойство
    // тела, а расталкивание — лишь один из его читателей (второй — слоты
    // назначения, которые работают и когда расталкивание выключено)
    human_style: Res<crate::human::HumanStyle>,
    lab: Res<SeparationLab>,
    mut load: ResMut<crate::sim_time::SimLoad>,
    mut stats: ResMut<SeparationStats>,
    frames: Res<FrameCount>,
    time: Res<Time>,
    navmesh: Res<crate::navigation::ArcNavmesh>,
    mut humans: ResMut<SpatialGrid<Human>>,
    demons: Res<SpatialGrid<Demon>>,
    frame: Res<Viewport>,
    mut out: SeparationOutput,
    mut pawns: Query<
        (
            &mut SimPosition,
            &crate::rng::PawnId,
            &crate::movement::Movable,
            Has<crate::movement::MovableStateMovingTag>,
            Has<Demon>,
            Has<DemonDevourTag>,
        ),
        (Or<(With<Human>, With<Demon>)>, Without<DemonLungeTag>),
    >,
    mut state: Local<SeparationState>,
) {
    if !style.enabled {
        state.pending_dt = 0.0;
        out.clear();
        return;
    }
    state.pending_dt += time.delta_secs();
    // не чаще раза в кадр: остальные тики того же кадра только копят dt, а
    // придержанные остаются придержанными до следующего прогона
    if state.last_frame == Some(frames.0) {
        return;
    }
    state.last_frame = Some(frames.0);
    let dt = state.pending_dt;
    // экспонента, а не `rate · dt`: прогон инвариантен к нарезке dt,
    // см. [`relaxation_fraction`]
    let fraction = relaxation_fraction(lab.rate, dt);
    state.pending_dt = 0.0;
    // на таком отдалении пешка — 1–2 пикселя, перекрытие не читается; вместе
    // с расталкиванием выключается и придержка — иначе пешки, придержанные
    // последним прогоном перед отзумом, остались бы придержанными навсегда
    if frame.zoom >= SEPARATION_MAX_ZOOM {
        out.clear();
        return;
    }

    let started = std::time::Instant::now();
    let view = frame.with_margin(VIEW_MARGIN);

    let human_radius = human_style.body_radius;
    let demon_radius = demon_radius(human_radius);

    // разыменование ОДИН раз: через `Local` каждое обращение к полю заимствует
    // весь ресурс, и «сложить в один буфер, читая соседний» не даст компилятор
    let state = &mut *state;
    state.pawns.clear();
    {
        let pawn_buffer = &mut state.pawns;
        let stuck_seen = &state.stuck;
        let mut collect = |entity: Entity| {
            // мимо запроса — бросок, труп, пешка чужого вида в чужой сетке
            let Ok((sim_position, pawn_id, movable, is_moving, is_demon, is_devouring)) =
                pawns.get(entity)
            else {
                return;
            };
            let position = sim_position.0;
            if !view.contains(position) {
                return;
            }
            let (radius, mobility) = if is_devouring {
                (demon_radius, 0.0)
            } else if is_demon {
                (demon_radius, DEMON_MOBILITY)
            } else {
                (human_radius, 1.0)
            };
            // осевшая пешка упирается: см. [`SeparationExperiments::idle_mobility`].
            // Множителем, а не заменой, — иначе ручка перебивала бы и
            // подвижность демона, которая выражает совсем другое
            let mobility = if is_moving {
                mobility
            } else {
                mobility * lab.experiments.idle_mobility
            };
            pawn_buffer.push(Pawn {
                entity,
                pawn_id: pawn_id.0,
                position,
                radius,
                mobility,
                // у стоящей курс не берём: `last_direction` у неё остался от
                // прошлой ходьбы и увёл бы обход в сторону, куда она уже не идёт
                heading: if is_moving {
                    movable.last_direction.normalize_or_zero()
                } else {
                    Vec2::ZERO
                },
                speed: movable.speed,
                human: !is_demon,
                stuck: stuck_seen.get(&entity).copied().unwrap_or(0.0),
            });
        };
        humans.for_each_in_rect(view.min(), view.max(), &mut collect);
        demons.for_each_in_rect(view.min(), view.max(), &mut collect);
    }

    resolve_pushes(
        state,
        Tuning {
            fraction,
            dt,
            sidestep: style.sidestep,
            cell: separation_cell(human_radius),
            lab: *lab,
        },
    );

    // залипание — тот же упор, но со временем; весь учёт в [`advance_stuck`]
    advance_stuck(state, dt, &lab);

    // придержанные — с чистого листа каждый прогон: ушедший из вьюпорта или
    // разошедшийся с соседом освобождается сам, без отдельной уборки
    out.clear();
    for (index, pawn) in state.pawns.iter().enumerate() {
        if state.held[index] {
            out.holds.0.insert(pawn.entity);
        }
        let blocked = state.blocks[index].normalize_or_zero();
        if blocked != Vec2::ZERO {
            out.block.0.insert(pawn.entity, blocked);
        }
        // нормируем один раз здесь: в буфере лежит СУММА сторон по всем
        // соседям, и зажатая с двух сторон пешка получает их равнодействующую
        let lateral = state.steers[index].normalize_or_zero();
        if lateral != Vec2::ZERO {
            out.steer.0.insert(pawn.entity, lateral * lab.steer);
        }
    }

    let navmesh = navmesh.read();
    for i in 0..state.pawns.len() {
        let pawn = state.pawns[i];
        let push = damp_along_heading(state.pushes[i], pawn.heading, style.backstep);
        // ядро не давится вдоль курса: придавливать выталкивание ИЗ ТЕЛА нечем,
        // это не личное пространство, а геометрия
        let core = state.core_pushes[i];
        if push == Vec2::ZERO && core == Vec2::ZERO {
            continue;
        }
        // ядро не ограничено потолком СКОРОСТИ (иначе оно не успевает за
        // сближением и не работает вовсе); потолок, пропорциональный dt, и
        // страховку от телепорта держит [`clamped_step`]
        let step = clamped_step(push, core, &lab, dt);
        let target = pawn.position + step;
        let tile = world_to_tile(target);
        // толчок в непроходимое отбрасывается: спасение (`rescue_*`) ловит
        // только провал поиска пути, задавленную в стену пешку оно бы не нашло
        if !navmesh.is_passable(tile.x, tile.y) {
            continue;
        }
        let Ok((mut sim_position, _, _, _, is_demon, _)) = pawns.get_mut(pawn.entity) else {
            continue;
        };
        sim_position.0 = target;
        let length = step.length();
        stats.push_metres += length as f64;
        stats.worst_push = stats.worst_push.max(length);
        // сетка людей инкрементальна; толчки мелкие, но стоячую пешку они
        // могут за много прогонов увести через границу 60-метровой ячейки
        if !is_demon {
            humans.moved(pawn.entity, pawn.position, target);
        }
    }
    stats.runs += 1;
    // счёт снят проходом толчков, а не отдельным обходом `pairs`: обход стоил бы
    // столько же итераций, сколько сам решатель, ради числа, которое в игре не
    // читает никто
    stats.overlapping_pairs += state.overlapping as u64;
    stats.anticipated_pairs += (state.pairs.len() - state.overlapping) as u64;
    crate::diagnostics::measure_ms(
        &mut diagnostics,
        &crate::diagnostics::SIM_SEPARATION_MS,
        started,
    );
    // покадровая работа внутри `FixedUpdate`: делить её на число шагов нельзя
    // (см. `SimLoad::add_frame_cost`), иначе при просадке кадра она выглядела
    // бы дешевеющей
    load.add_frame_cost(started.elapsed());
}

#[cfg(test)]
mod tests;
