use bevy::app::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy};
use bevy::prelude::*;
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

use qwe::{
    camera, city, demon, determinism, dev, diagnostics, human, loading, map, movement, navigation,
    portal, post, prefs, restart, rng, silhouette, sim_time, spatial, telemetry, ui,
};

fn main() {
    let port = dev::brp_port();
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
                        present_mode: dev::perf_present_mode(),
                        resolution: dev::perf_window_resolution(),
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
        .add_plugins((
            city::CityPlugin,
            rng::RngPlugin,
            determinism::DeterminismPlugin,
            // группа ниже уже во всю длину кортежа `Plugins` — 15 штук
            post::PostProcessPlugin,
            // атлас силуэтов — уровня процесса, как и seed: собирается один
            // раз в `Startup` и переживает любую смену мира
            silhouette::SilhouettePlugin,
        ))
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
        // кадр как значение — один счёт на кадр для всех пяти гейтов
        // видимости; отдельным плагином, потому что его поднимает и стенд
        // толпы, которому `CameraPlugin` не нужен
        .add_plugins(camera::ViewportPlugin)
        // последним: на своей сборке читает реестр типов, а типы групп
        // настроек регистрируют плагины выше (см. src/prefs.rs)
        .add_plugins(prefs::PrefsPlugin)
        .add_plugins(ui::QuitOnEscPlugin);

    // Без свободного порта HTTP-сервер не поднимается вовсе: пусть отсутствие
    // BRP будет явным, а не сервером, который молча слушает не там
    if let Some(port) = port {
        app.add_plugins(RemoteHttpPlugin::default().with_port(port));
    }

    // Порт в окружении задаёт только агент, себе отдельный от пользовательского
    // (см. `.claude/live-app-project.md`) — такой запуск помечен в углу экрана
    if std::env::var_os("BRP_PORT").is_some() {
        app.insert_resource(ui::AgentBrpSession);
    }

    app.run();
}
