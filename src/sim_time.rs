//! Управление скоростью симуляции (порт лесенки скоростей из
//! `zxc/src/story_time`): Space — пауза, `=`/`-` — быстрее/медленнее.
//!
//! Запрошенная скорость и фактическая — разные величины: машина не всегда
//! тянет запрошенную, и тогда время автоматически замедляется до посильного
//! (см. `throttle_speed_to_fps`).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::settings::{MAX_FRAME_DELTA, SPEED_SETTLE_RATE};

/// Скорость симуляции: `requested` крутит пользователь лесенкой, `effective`
/// выставляется в `Time<Virtual>` после ограничения по fps.
#[derive(Resource, Reflect, Debug)]
#[reflect(Resource)]
pub struct SimSpeed {
    pub requested: f32,
    pub effective: f32,
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self {
            requested: 1.0,
            effective: 1.0,
        }
    }
}

impl SimSpeed {
    /// Замедлено ли время против запрошенного (с запасом на дребезг
    /// регулятора).
    pub fn is_throttled(&self) -> bool {
        self.effective < self.requested * 0.98
    }
}

pub struct SimTimePlugin;

impl Plugin for SimTimePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SimSpeed>()
            .init_resource::<SimSpeed>()
            .add_systems(Startup, pin_max_delta)
            .add_systems(Update, (modify_time, throttle_speed_to_fps).chain());
    }
}

/// `Time<Virtual>::max_delta` — сколько виртуального времени отдаётся
/// `FixedUpdate` за один кадр. Значение прибито явно: из него выведен потолок
/// скорости в `throttle_speed_to_fps`, и молчаливая смена дефолта Bevy сломала
/// бы расчёт.
fn pin_max_delta(mut time: ResMut<Time<Virtual>>) {
    time.set_max_delta(std::time::Duration::from_secs_f32(MAX_FRAME_DELTA));
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
/// Регулятор замкнут по измеренному fps, поэтому идёт к цели плавно
/// (`SPEED_SETTLE_RATE`): резкий скачок раскачал бы петлю fps → потолок → fps.
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

    // ниже 1x не замедляемся никогда: реальное время — нижняя граница
    let affordable = (fps * MAX_FRAME_DELTA).max(1.0);
    let target = speed.requested.min(affordable);

    speed.effective += (target - speed.effective) * SPEED_SETTLE_RATE;
    // у цели — садимся точно, иначе экспонента вечно недотягивает и UI
    // показывает «15x → 14.97x»
    if (target - speed.effective).abs() < 0.05 {
        speed.effective = target;
    }

    if time.relative_speed() != speed.effective {
        time.set_relative_speed(speed.effective);
    }
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
