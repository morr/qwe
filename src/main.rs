use bevy::app::{AppExit, TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy};
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

use qwe::{
    camera, city, demon, dev, diagnostics, human, loading, map, movement, navigation, portal,
    prefs, restart, sim_time, spatial, telemetry, ui,
};

/// Порт BRP: `BRP_PORT` из окружения, иначе дефолтный 15702. `None` — порт занят
/// чужим экземпляром, BRP у этого запуска не будет.
///
/// Порт из окружения нужен, чтобы экземпляр агента (`BRP_PORT=15703 cargo run`)
/// не дрался за порт с тем, который пользователь держит открытым. Занятость
/// проверяется здесь, до старта, потому что `bevy_remote` спавнит сервер через
/// `.detach()` и ошибку bind не видит никто: второй экземпляр молча остаётся без
/// BRP, а клиент получает ответы от первого — и это выглядит как «изменение не
/// сработало», а не как «я разговариваю с чужим приложением».
///
/// Явно заданный порт при этом обязателен: если он занят, запуск падает, а не
/// продолжается без BRP. Дефолтный порт — только предупреждение, чтобы второе
/// окно, открытое руками, всё-таки запускалось.
fn brp_port() -> Option<u16> {
    let requested = std::env::var("BRP_PORT").ok();
    let port = match &requested {
        Some(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("BRP_PORT is not a port number: {value:?}")),
        None => bevy::remote::http::DEFAULT_PORT,
    };
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        // Сокет закрывается сразу, bind'ит его уже сам RemoteHttpPlugin
        Ok(_) => Some(port),
        Err(err) if requested.is_some() => {
            panic!(
                "BRP port {port} is busy ({err}) — another qwe already holds it; pick a free BRP_PORT"
            )
        }
        Err(err) => {
            eprintln!(
                "qwe: BRP port {port} is busy ({err}) — remote protocol off for this instance"
            );
            None
        }
    }
}

fn main() {
    let port = brp_port();
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.72, 0.71, 0.68)))
        .add_plugins(
            DefaultPlugins
                // A*-таскам по умолчанию достаётся 25% ядер (максимум 4) —
                // на высоких скоростях симуляции этого мало
                .set(TaskPoolPlugin {
                    task_pool_options: TaskPoolOptions {
                        async_compute: TaskPoolThreadAssignmentPolicy {
                            min_threads: 2,
                            max_threads: 8,
                            percent: 0.5,
                            on_thread_spawn: None,
                            on_thread_destroy: None,
                        },
                        ..default()
                    },
                })
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        // Порт в заголовке: два открытых окна qwe иначе не
                        // отличить, а `brp raise` наводится именно по окну
                        title: match port {
                            Some(port) => format!("qwe :{port}"),
                            None => "qwe (no brp)".to_string(),
                        },
                        position: WindowPosition::Automatic,
                        mode: bevy::window::WindowMode::Windowed,
                        present_mode: bevy::window::PresentMode::AutoVsync,
                        resolution: (1920, 1080).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    level: bevy::log::Level::TRACE,
                    filter: "info,qwe=trace".to_string(),
                    ..default()
                }),
        )
        .add_plugins(RemotePlugin::default())
        .add_plugins(city::CityPlugin)
        .add_plugins((
            loading::LoadingPlugin,
            camera::CameraPlugin,
            map::MapPlugin,
            navigation::NavigationPlugin,
            movement::MovementPlugin,
            portal::PortalPlugin,
            spatial::SpatialPlugin,
            telemetry::TelemetryPlugin,
            demon::DemonPlugin,
            human::HumanPlugin,
            restart::RestartPlugin,
            sim_time::SimTimePlugin,
            diagnostics::GameDiagnosticsPlugin,
            ui::UiPlugin,
            dev::DevPlugin,
        ))
        // последним: на своей сборке читает реестр типов, а типы групп
        // настроек регистрируют плагины выше (см. src/prefs.rs)
        .add_plugins(prefs::PrefsPlugin)
        .add_systems(
            Update,
            close_on_esc.run_if(input_just_pressed(KeyCode::Escape)),
        );

    // Без свободного порта HTTP-сервер не поднимается вовсе: пусть отсутствие
    // BRP будет явным, а не сервером, который молча слушает не там
    if let Some(port) = port {
        app.add_plugins(RemoteHttpPlugin::default().with_port(port));
    }

    app.run();
}

/// Gated by `input_just_pressed(Escape)` in the schedule — the window-focus
/// check stays here so Esc in another app's window doesn't quit this one.
fn close_on_esc(focused_windows: Query<&Window>, mut event_writer: MessageWriter<AppExit>) {
    if focused_windows.iter().any(|window| window.focused) {
        event_writer.write(AppExit::Success);
    }
}
