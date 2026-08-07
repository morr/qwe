//! Управление скоростью симуляции (порт лесенки скоростей из
//! `zxc/src/story_time`): Space — пауза, `=`/`-` — быстрее/медленнее.
//!
//! Запрошенная скорость и фактическая — разные величины: машина не всегда
//! тянет запрошенную, и тогда время автоматически замедляется до посильного
//! (см. `throttle_speed_to_fps`). Сверху запрошенная упирается в
//! `MAX_SIM_SPEED`.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::loading::PlayPhase;
use crate::restart::RestartEvent;
use crate::settings::{
    ACTUAL_SPEED_WINDOW, MAX_FRAME_DELTA, MAX_SIM_SPEED, MIN_SIM_FPS, MIN_SIM_SPEED,
    SIM_FPS_HYSTERESIS, SPEED_DROP_RATE, SPEED_LADDER, SPEED_SETTLE_RATE,
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

impl SimClock {
    /// Часы нового мира с нуля: за точку отсчёта берём текущее виртуальное
    /// время, а не обнуляем `Time<Virtual>` — тот общий, и его сброс сдвинул бы
    /// всем таймерам их дедлайны.
    pub fn restart(&mut self, now: f64) {
        self.started_at = now;
        self.elapsed = 0.0;
    }
}

pub struct SimTimePlugin;

impl Plugin for SimTimePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SimSpeed>()
            .register_type::<SimClock>()
            .init_resource::<SimSpeed>()
            .init_resource::<SimClock>()
            .add_systems(Startup, pin_max_delta)
            // рестарт по R отстраивает мир заново — часам тоже начинать с нуля
            .add_observer(restart_sim_clock)
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
                    // пробел, `-` и `=` — символы, пока курсор в поле ввода
                    modify_time.run_if(not(crate::ui::typing_in_text_input)),
                    throttle_speed_to_fps,
                    measure_actual_speed,
                    tick_sim_clock.run_if(in_state(PlayPhase::Live)),
                )
                    .chain(),
            );
    }
}

/// Самый длинный честный кадр — константа, и от скорости симуляции она не
/// зависит (см. [`MAX_FRAME_DELTA`]). Прибита явно, чтобы молчаливая смена
/// дефолта Bevy не поменяла поведение на фризах.
fn pin_max_delta(mut time: ResMut<Time<Virtual>>) {
    time.set_max_delta(std::time::Duration::from_secs_f32(MAX_FRAME_DELTA));
}

/// Пауза на время прогрева. Заявки на путь при этом идут: их подача и
/// диспетчеризация живут в `Update`, а стоит только `FixedUpdate`.
///
/// В **детерминированном** режиме заявки на паузе не идут: там весь конвейер —
/// и `pick_wander_targets`, и диспетчер, и приёмка — живёт в `FixedUpdate`.
/// Пешечного прогрева в этом режиме поэтому нет вовсе, см.
/// `loading::poll_warmup`; паузу это не отменяет — мир за экраном загрузки не
/// двигается ни в одном из режимов.
fn pause_simulation(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

fn resume_simulation(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

fn start_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time<Virtual>>) {
    clock.restart(time.elapsed_secs_f64());
}

fn restart_sim_clock(
    _event: On<RestartEvent>,
    mut clock: ResMut<SimClock>,
    time: Res<Time<Virtual>>,
) {
    clock.restart(time.elapsed_secs_f64());
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

/// Авто-замедление под fps: **срабатывает только ниже [`MIN_SIM_FPS`]**, и
/// тогда снижает скорость ровно настолько, чтобы кадры вернулись к этому
/// числу.
///
/// Симуляция на скорости `S` требует `64 × S` тиков в реальную секунду. Чем
/// их больше, тем длиннее кадр; когда кадров становится меньше
/// [`MIN_SIM_FPS`], скорость и есть та единственная ручка, которой это можно
/// вернуть — вот регулятор её и крутит.
///
/// **Причём тут `max_delta`: ни при чём.** Прежний порог считался как
/// `fps × MAX_FRAME_DELTA` и опирался на прочтение «за кадр Bevy отдаёт не
/// больше `max_delta` виртуального времени». Прочтение неверное: клампится
/// сырая дельта, до умножения на скорость
/// (`bevy_time/src/virt.rs::advance_with_raw_delta`). Скорости эта константа
/// не ограничивает вовсе, а формула означала не то, чем выглядела: её
/// равновесие `S = fps × 0.5` — это `fps = 2 × S`, кадры жёстко назначались
/// скоростью.
///
/// У регулятора три зоны, а не две, и средняя обязательна:
///
/// - `fps < MIN_SIM_FPS` — режем пропорционально недобору (вдвое меньше
///   кадров — вдвое меньше скорость);
/// - `fps > MIN_SIM_FPS × SIM_FPS_HYSTERESIS` — разгоняем к запрошенной;
/// - между — **не трогаем ничего**.
///
/// Полоса посредине — не вкусовщина, без неё петля автоколеблется; почему
/// именно так и почему сглаживанием это не лечится, разобрано у
/// [`SIM_FPS_HYSTERESIS`]. Внутри своих зон шаг всё равно плавный
/// (`SPEED_SETTLE_RATE` вверх, более быстрый `SPEED_DROP_RATE` вниз).
fn throttle_speed_to_fps(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    diagnostics: Res<DiagnosticsStore>,
) {
    // Сырое значение, а не `smoothed()`: сглаживание в петле должно быть
    // ровно одно, и оно уже есть — `approach`. Экспоненциальное среднее
    // диагностики (окно ~2 с) добавляло бы второе, причём с фазовым
    // запаздыванием: команда считалась бы по fps, который принадлежит уже
    // другой скорости. Замер это и показал — вместо ровных 30 кадры ходили
    // 17…63. Собственный шум сырого fps съедает `approach` за ~20 кадров.
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diagnostic| diagnostic.value())
        .unwrap_or_default() as f32;
    if fps <= 0.0 {
        return;
    }

    // Лесенка выше `MAX_SIM_SPEED` не поднимается, но `requested` пишут и
    // напрямую (по BRP) — режем здесь, чтобы потолок был один на все входы и
    // панель не показывала запрошенное число, которого не бывает.
    if speed.requested > MAX_SIM_SPEED {
        speed.requested = MAX_SIM_SPEED;
    }

    let target = speed.requested.min(affordable_speed(speed.effective, fps));
    speed.effective = approach(speed.effective, target);

    if time.relative_speed() != speed.effective {
        time.set_relative_speed(speed.effective);
    }
}

/// Скорость, посильную при таком fps, — от текущей, а не от нуля.
///
/// Ниже [`MIN_SIM_FPS`] — пропорциональный срез; выше полосы гистерезиса —
/// потолка нет вовсе (пусть растёт к запрошенной); внутри полосы — ровно
/// текущая, то есть «не трогать».
///
/// Замедление при `fps ≥ MIN_SIM_FPS` невозможно в принципе: возвращаемое
/// значение там не меньше текущей скорости, а вызывающий берёт
/// `min(requested, …)`. Пол `MIN_SIM_SPEED` не даёт петле схлопнуться в ноль,
/// из которого умножением уже не выбраться.
fn affordable_speed(effective: f32, fps: f32) -> f32 {
    if fps < MIN_SIM_FPS {
        return (effective * fps / MIN_SIM_FPS).max(MIN_SIM_SPEED);
    }
    if fps > MIN_SIM_FPS * SIM_FPS_HYSTERESIS {
        return MAX_SIM_SPEED;
    }
    effective.max(MIN_SIM_SPEED)
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

/// Ступень для кнопки Speed: та же лесенка, но по кругу — с верхней ступени
/// возвращаемся к 1x. Кнопка одна, и без замыкания сверху было бы не
/// выбраться иначе как хоткеем.
pub fn cycle_time_scale(speed: f32) -> f32 {
    if speed >= MAX_SIM_SPEED {
        1.0
    } else {
        next_time_scale(speed)
    }
}

/// Следующая ступень лесенки: первая строго выше текущей скорости, с верхней
/// ступени — остаёмся на ней. Произвольное значение (по BRP `requested` пишут
/// любым) прижимается к ближайшей ступени сверху.
pub fn next_time_scale(speed: f32) -> f32 {
    SPEED_LADDER
        .into_iter()
        .find(|&step| step > speed)
        .unwrap_or(MAX_SIM_SPEED)
}

/// Предыдущая ступень лесенки: последняя строго ниже текущей скорости,
/// с нижней ступени — остаёмся на ней.
pub fn previous_time_scale(speed: f32) -> f32 {
    SPEED_LADDER
        .into_iter()
        .rev()
        .find(|&step| step < speed)
        .unwrap_or(SPEED_LADDER[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// То, ради чего регулятор переделан: пока кадров не меньше цели,
    /// замедления нет вовсе, какой бы ни была скорость.
    #[test]
    fn nothing_is_throttled_at_or_above_the_target_fps() {
        for effective in [MIN_SIM_SPEED, 1.0, 10.0, MAX_SIM_SPEED] {
            for fps in [MIN_SIM_FPS, 60.0, 144.0] {
                assert!(
                    affordable_speed(effective, fps) >= effective,
                    "{effective}x при {fps} fps не имеет права замедляться"
                );
            }
        }
    }

    /// Ниже цели срез пропорционален недобору кадров, а не абсолютен: вдвое
    /// меньше кадров — вдвое меньше скорость.
    #[test]
    fn the_throttle_scales_with_the_fps_shortfall() {
        assert_eq!(affordable_speed(10.0, MIN_SIM_FPS / 2.0), 5.0);
        assert_eq!(affordable_speed(10.0, MIN_SIM_FPS / 10.0), 1.0);
        // из нуля умножением уже не выбраться — отсюда пол
        assert_eq!(affordable_speed(0.0, 0.0), MIN_SIM_SPEED);
    }

    /// Полоса гистерезиса: внутри неё регулятор обязан отдавать ровно текущую
    /// скорость, иначе он продолжает толкать систему и она автоколеблется.
    #[test]
    fn inside_the_hysteresis_band_the_speed_is_left_alone() {
        let inside = MIN_SIM_FPS * (1.0 + SIM_FPS_HYSTERESIS) / 2.0;
        for effective in [1.0, 5.0, 17.5] {
            assert_eq!(affordable_speed(effective, inside), effective);
        }
        // на самой цели — тоже не трогаем
        assert_eq!(affordable_speed(5.0, MIN_SIM_FPS), 5.0);
    }

    /// Выше полосы потолка нет: скорость идёт к запрошенной, а не к нынешней,
    /// умноженной на что-то.
    #[test]
    fn above_the_band_the_speed_climbs_to_the_request() {
        assert_eq!(
            affordable_speed(1.0, MIN_SIM_FPS * SIM_FPS_HYSTERESIS + 1.0),
            MAX_SIM_SPEED
        );
    }

    /// Петля обязана **прийти и встать**, а не бегать пилой между ступенями
    /// vsync — это регрессия ровно на ту жалобу, ради которой заведён
    /// [`SIM_FPS_HYSTERESIS`].
    ///
    /// Модель кадра честная: `max_delta` клампит **реальную** длительность, а
    /// кадр несёт `длительность × скорость` виртуальных секунд.
    #[test]
    fn the_regulator_settles_inside_the_band_without_ringing() {
        // машина осиливает 102.4 тика в секунду, то есть 1.6x по симуляции
        let tick_cost = 1.0 / 102.4;
        // и ещё 5 мс на кадр уходит мимо симуляции — отрисовка, UI, ввод
        let render_cost = 1.0 / 200.0;
        let requested = 10.0f32;

        let mut effective = requested;
        let mut fps = 60.0f32;
        let mut tail = Vec::new();
        for step in 0..4000 {
            let carried = (1.0 / fps).min(MAX_FRAME_DELTA) * effective;
            fps = 1.0 / (carried * 64.0 * tick_cost + render_cost);
            effective = approach(effective, requested.min(affordable_speed(effective, fps)));
            if step >= 3800 {
                tail.push(fps);
            }
        }

        let low = tail.iter().copied().fold(f32::MAX, f32::min);
        let high = tail.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            high - low < 0.5,
            "петля не встала: кадры ходят {low}…{high}"
        );
        assert!(
            (MIN_SIM_FPS..=MIN_SIM_FPS * SIM_FPS_HYSTERESIS).contains(&low),
            "встали на {low} fps, мимо полосы {MIN_SIM_FPS}…{}",
            MIN_SIM_FPS * SIM_FPS_HYSTERESIS
        );
        // и это не «замерли на месте»: скорость выросла от посильного минимума
        assert!(
            effective > 1.0 && effective < requested,
            "скорость {effective} не похожа на посильную"
        );
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
