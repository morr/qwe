//! Автоснимок витрины — общий на все витрины `examples/demos/`.
//!
//! Вынесен из витрин, а не скопирован в каждую, потому что три числа кадров и
//! подъём окна — знание, добытое отладкой (коммит 21853a3), и поправленное в
//! одной копии в другой осталось бы прежним.
//!
//! Поднимать окно обязательно: macOS снимает **настоящую поверхность окна**, и
//! перекрытое чужим окном оно отдаёт чёрный прямоугольник. `Window::focused =
//! true` уходит в `winit::focus_window` (`bevy_winit::system::changed_windows`),
//! поэтому поднятие — это одно присваивание, а не osascript снаружи.
//!
//! Числа: первый кадр уходит на сборку сетки (она идёт в `Update`, а не в
//! `Startup`), дальше нужно дать шейдеру, шрифту и самому поднятию доехать до
//! экрана; после снимка — столько же, потому что на диск его пишет
//! наблюдатель, а не эта система.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;

const SHOT_RAISE_FRAME: u32 = 5;
const SHOT_FRAME: u32 = 30;
const SHOT_EXIT_FRAME: u32 = SHOT_FRAME + 30;

/// Куда класть автоснимок. Ресурс, а не чтение окружения из системы: гейт
/// расписания и сама система читали `std::env::var` по два раза на кадр всю
/// жизнь процесса.
#[derive(Resource)]
pub(crate) struct ShotRequest(String);

/// Заказан ли снимок переменной окружения витрины. Ставится в `Startup`.
pub(crate) fn request_shot(var: &'static str) -> impl Fn(Commands) {
    move |mut commands: Commands| {
        if let Ok(path) = std::env::var(var) {
            commands.insert_resource(ShotRequest(path));
        }
    }
}

/// Снимок витрины и выход — единственный способ посмотреть на неё из сессии:
/// BRP у примера нет.
pub(crate) fn auto_shot(
    mut commands: Commands,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    request: Res<ShotRequest>,
) {
    *frame += 1;
    if *frame == SHOT_RAISE_FRAME {
        // трогаем ровно на одном кадре: всякое взятие `&mut Window` метит его
        // изменённым, и `changed_windows` перебирал бы окно каждый кадр
        window.focused = true;
    }
    if *frame == SHOT_FRAME {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(request.0.clone()));
    }
    if *frame == SHOT_EXIT_FRAME {
        exit.write(AppExit::Success);
    }
}
