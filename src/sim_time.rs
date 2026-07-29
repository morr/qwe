//! Управление скоростью симуляции (порт лесенки скоростей из
//! `zxc/src/story_time`): Space — пауза, `=`/`-` — быстрее/медленнее.
//!
//! Запрошенная скорость и фактическая — разные величины: машина не всегда
//! тянет запрошенную, и тогда время автоматически замедляется до посильного
//! (см. `throttle_speed_to_fps`).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::loading::PlayPhase;
use crate::settings::{
    ACTUAL_SPEED_WINDOW, MAX_FRAME_DELTA, MIN_SIM_SPEED, SPEED_DROP_RATE, SPEED_SETTLE_RATE,
};

/// Скорость симуляции: `requested` крутит пользователь лесенкой, `effective`
/// выставляется в `Time<Virtual>` после ограничения по fps, `actual` — то, что
/// в итоге получилось (замер виртуального времени против реального).
///
/// Расходятся все три: `effective` — команда регулятора, а не факт. Bevy
/// режет виртуальную дельту кадра по `max_delta`, поэтому фриз или затык
/// (например, пока фоново строится сетка northstar) отнимает у симуляции
/// время помимо регулятора — видно это только в `actual`.
#[derive(Resource, Reflect, Debug)]
#[reflect(Resource)]
pub struct SimSpeed {
    pub requested: f32,
    pub effective: f32,
    pub actual: f32,
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self {
            requested: 1.0,
            effective: 1.0,
            actual: 1.0,
        }
    }
}

impl SimSpeed {
    /// Замедлено ли время против запрошенного (с запасом на дребезг замера).
    pub fn is_throttled(&self) -> bool {
        self.actual < self.requested * 0.95
    }
}

/// Часы симуляции: сколько виртуального времени прожил текущий мир.
///
/// Отсчёт идёт от входа в `PlayPhase::Live`, а не от старта приложения:
/// загрузка карты и прогрев проходят в реальном времени и к моменту симуляции
/// отношения не имеют. Смена города перезапускает мир, значит и часы.
///
/// Время виртуальное — стоит на паузе и бежит быстрее на ускорении. Это
/// «сколько прожил мир», а не сколько просидел за ним игрок.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
pub struct SimClock {
    /// `Time<Virtual>::elapsed` на момент входа в `Live`.
    started_at: f64,
    /// Прошедшее время симуляции, сек.
    pub elapsed: f64,
}

pub struct SimTimePlugin;

impl Plugin for SimTimePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SimSpeed>()
            .register_type::<SimClock>()
            .init_resource::<SimSpeed>()
            .init_resource::<SimClock>()
            .add_systems(Startup, pin_max_delta)
            // прогрев идёт на паузе: мир уже собран, но за экраном загрузки
            // ему двигаться незачем — пусть пешки сначала получат пути
            .add_systems(OnEnter(PlayPhase::Warmup), pause_simulation)
            .add_systems(
                OnEnter(PlayPhase::Live),
                (resume_simulation, start_sim_clock),
            )
            .add_systems(
                Update,
                (
                    modify_time,
                    throttle_speed_to_fps,
                    measure_actual_speed,
                    tick_sim_clock.run_if(in_state(PlayPhase::Live)),
                )
                    .chain(),
            );
    }
}

/// `Time<Virtual>::max_delta` — сколько виртуального времени отдаётся
/// `FixedUpdate` за один кадр. Значение прибито явно: из него выведен потолок
/// скорости в `throttle_speed_to_fps`, и молчаливая смена дефолта Bevy сломала
/// бы расчёт.
fn pin_max_delta(mut time: ResMut<Time<Virtual>>) {
    time.set_max_delta(std::time::Duration::from_secs_f32(MAX_FRAME_DELTA));
}

/// Пауза на время прогрева. Заявки на путь при этом идут: их подача и
/// диспетчеризация живут в `Update`, а стоит только `FixedUpdate`.
fn pause_simulation(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

fn resume_simulation(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

/// Часы нового мира с нуля: за точку отсчёта берём текущее виртуальное время,
/// а не обнуляем `Time<Virtual>` — тот общий, и его сброс сдвинул бы всем
/// таймерам их дедлайны.
fn start_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time<Virtual>>) {
    clock.started_at = time.elapsed_secs_f64();
    clock.elapsed = 0.0;
}

/// На паузе виртуальная дельта нулевая, поэтому часы сами стоят.
fn tick_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time<Virtual>>) {
    clock.elapsed = time.elapsed_secs_f64() - clock.started_at;
}

fn modify_time(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        toggle_pause(&mut time);
    }
    if keys.just_pressed(KeyCode::Equal) {
        speed.requested = next_time_scale(speed.requested);
    }
    if keys.just_pressed(KeyCode::Minus) {
        speed.requested = previous_time_scale(speed.requested);
    }
}

/// Авто-замедление под fps.
///
/// За один кадр Bevy отдаёт `FixedUpdate` не больше `max_delta` виртуального
/// времени, то есть `max_delta × 64` тиков. Симуляция на скорости S требует
/// `64 × S` тиков в секунду, значит `64 × S ≤ fps × max_delta × 64`, откуда
/// **S ≤ fps × max_delta**. Выше этого потолка запрошенная скорость всё равно
/// не выдаётся: тики упираются в кадры, `Update` (диспетчер путей, ввод, UI)
/// начинает отставать от симуляции, и пешки, закончившие маршрут, стоят в
/// ожидании следующего кадра. Так что скорость лучше честно снизить, чем
/// делать вид, что идёт 15x.
///
/// Потолок не ограничен снизу единицей: на 4 fps посильна ровно 1x, а ниже
/// симуляция не тянет и реальное время. Делать вид, что идёт 1x, там нельзя —
/// Bevy всё равно обрежет кадровую дельту по `max_delta`, только молча; лучше
/// честно снизить команду (до `MIN_SIM_SPEED`), тогда кадр считает меньше
/// тиков и fps получает шанс подняться.
///
/// Регулятор замкнут по измеренному fps, поэтому идёт к цели плавно
/// (`SPEED_SETTLE_RATE`): резкий скачок раскачал бы петлю fps → потолок → fps.
/// Вниз — быстрее (`SPEED_DROP_RATE`), см. константу.
fn throttle_speed_to_fps(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    diagnostics: Res<DiagnosticsStore>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diagnostic| diagnostic.smoothed())
        .unwrap_or_default() as f32;
    if fps <= 0.0 {
        return;
    }

    let target = speed.requested.min(affordable_speed(fps));
    speed.effective = approach(speed.effective, target);

    if time.relative_speed() != speed.effective {
        time.set_relative_speed(speed.effective);
    }
}

/// Скорость, посильная при таком fps: `S ≤ fps × MAX_FRAME_DELTA`, но не ниже
/// `MIN_SIM_SPEED` — на нуле симуляция стоит, а не идёт медленно.
fn affordable_speed(fps: f32) -> f32 {
    (fps * MAX_FRAME_DELTA).max(MIN_SIM_SPEED)
}

/// Шаг регулятора к целевой скорости.
fn approach(current: f32, target: f32) -> f32 {
    let rate = if target < current {
        SPEED_DROP_RATE
    } else {
        SPEED_SETTLE_RATE
    };
    let stepped = current + (target - current) * rate;
    // у цели — садимся точно, иначе экспонента вечно недотягивает и UI
    // показывает «15x → 14.97x». Порог относительный: на 0.5x абсолютные 0.05
    // были бы десятой частью скорости.
    if (target - stepped).abs() < target * 0.01 {
        target
    } else {
        stepped
    }
}

/// Замер фактической скорости: сколько виртуального времени набежало на
/// секунду реального.
///
/// Считается по окну реального времени (`ACTUAL_SPEED_WINDOW`), а не по
/// кадрам: просадка — это как раз несколько длинных кадров, и в среднем
/// по кадрам они весят столько же, сколько быстрые, то есть теряются.
/// Окно ловит и то, чего не знает регулятор, — обрезку дельты по `max_delta`.
fn measure_actual_speed(
    virtual_time: Res<Time<Virtual>>,
    real_time: Res<Time<Real>>,
    mut speed: ResMut<SimSpeed>,
    mut window: Local<SpeedWindow>,
) {
    // на паузе мерить нечего, и накопленное окно к моменту снятия паузы
    // протухнет — сбрасываем
    if virtual_time.is_paused() {
        *window = SpeedWindow::default();
        return;
    }

    window.real += real_time.delta_secs();
    window.virtual_elapsed += virtual_time.delta_secs();
    if window.real < ACTUAL_SPEED_WINDOW {
        return;
    }

    speed.actual = window.virtual_elapsed / window.real;
    *window = SpeedWindow::default();
}

/// Накопитель окна замера фактической скорости.
#[derive(Default)]
struct SpeedWindow {
    real: f32,
    virtual_elapsed: f32,
}

pub fn toggle_pause(time: &mut Time<Virtual>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
}

/// Следующая ступень лесенки скоростей.
pub fn next_time_scale(speed: f32) -> f32 {
    speed
        + if speed < 5. {
            2.
        } else if speed < 15. {
            5.
        } else if speed < 20. {
            10.
        } else if speed < 100. {
            25.
        } else if speed < 200. {
            50.
        } else if speed < 500. {
            100.
        } else if speed < 2000. {
            500.
        } else {
            1000.
        }
}

/// Предыдущая ступень лесенки скоростей.
pub fn previous_time_scale(speed: f32) -> f32 {
    if speed == 1.0 {
        return speed;
    }

    speed
        - if speed <= 5. {
            2.
        } else if speed <= 15. {
            5.
        } else if speed <= 25. {
            10.
        } else if speed <= 100. {
            25.
        } else if speed <= 200. {
            50.
        } else if speed <= 500. {
            100.
        } else if speed <= 2000. {
            500.
        } else {
            1000.
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affordable_speed_falls_below_real_time() {
        // 60 fps тянут 15x, 4 fps — ровно реальное время
        assert_eq!(affordable_speed(60.0), 15.0);
        assert_eq!(affordable_speed(4.0), 1.0);
        // ниже 4 fps не тянется и 1x — замедляемся честно
        assert_eq!(affordable_speed(2.0), 0.5);
        assert_eq!(affordable_speed(0.4), MIN_SIM_SPEED);
    }

    #[test]
    fn regulator_reaches_target_below_one() {
        let mut speed = 1.0;
        for _ in 0..100 {
            speed = approach(speed, 0.5);
        }
        assert_eq!(speed, 0.5);
    }

    #[test]
    fn regulator_drops_faster_than_it_climbs() {
        let down = 1.0 - approach(1.0, 0.0);
        let up = approach(1.0, 2.0) - 1.0;
        assert!(down > up, "down {down} should outpace up {up}");
    }
}
