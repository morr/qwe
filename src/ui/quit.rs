//! Esc закрывает окно — одинаково в игре и в каждой витрине `examples/demos/*`.
//!
//! Плагином в библиотеке, а не системой в `main.rs`: витрины не поднимают ни
//! `UiPlugin`, ни игровой `main`, и без общего плагина каждая писала бы свою
//! копию с фокусом окна и гейтом текстового поля — или забывала бы про него.

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;

use crate::ui::typing_in_text_input;

pub struct QuitOnEscPlugin;

impl Plugin for QuitOnEscPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            close_on_esc
                .run_if(input_just_pressed(KeyCode::Escape))
                // Esc в поле ввода — «снять фокус», а не «выйти из игры»
                .run_if(not(typing_in_text_input)),
        );
    }
}

/// Gated by `input_just_pressed(Escape)` in the schedule — the window-focus
/// check stays here so Esc in another app's window doesn't quit this one.
fn close_on_esc(focused_windows: Query<&Window>, mut exit: MessageWriter<AppExit>) {
    if focused_windows.iter().any(|window| window.focused) {
        exit.write(AppExit::Success);
    }
}
