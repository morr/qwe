//! Инструменты для отладочных сессий (skill `live-app`): скриншот в файл по
//! клавише F12 или BRP-событию `TakeScreenshotEvent`.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

const SCREENSHOT_PATH: &str = "screenshot.png";

#[derive(Event, Reflect, Debug, Default)]
#[reflect(Event)]
pub struct TakeScreenshotEvent;

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<TakeScreenshotEvent>()
            .add_observer(on_take_screenshot)
            .add_systems(
                Update,
                trigger_screenshot.run_if(bevy::input::common_conditions::input_just_pressed(
                    KeyCode::F12,
                )),
            );
    }
}

fn trigger_screenshot(mut commands: Commands) {
    commands.trigger(TakeScreenshotEvent);
}

fn on_take_screenshot(_event: On<TakeScreenshotEvent>, mut commands: Commands) {
    info!("saving screenshot to {SCREENSHOT_PATH}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(SCREENSHOT_PATH));
}
