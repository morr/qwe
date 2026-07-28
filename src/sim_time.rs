//! Управление скоростью симуляции (порт лесенки скоростей из
//! `zxc/src/story_time`): Space — пауза, `=`/`-` — быстрее/медленнее.

use bevy::prelude::*;

pub struct SimTimePlugin;

impl Plugin for SimTimePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, modify_time);
    }
}

fn modify_time(mut time: ResMut<Time<Virtual>>, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::Space) {
        toggle_pause(&mut time);
    }
    if keys.just_pressed(KeyCode::Equal) {
        increase_time_scale(&mut time);
    }
    if keys.just_pressed(KeyCode::Minus) {
        decrease_time_scale(&mut time);
    }
}

pub fn toggle_pause(time: &mut Time<Virtual>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
}

pub fn increase_time_scale(time: &mut Time<Virtual>) {
    let speed = time.relative_speed();
    time.set_relative_speed(
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
            },
    );
}

pub fn decrease_time_scale(time: &mut Time<Virtual>) {
    let speed = time.relative_speed();
    if speed == 1.0 {
        return;
    }

    time.set_relative_speed(
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
            },
    );
}
