//! Guard регистрации настроек: парность «группа настроек в реестре ↔ track_pref».
//!
//! Нормальная форма — один вызов `register_pref::<T>()` у владельца ресурса,
//! и забыть в нём одно из трёх действий невозможно. Тест держит оставшиеся
//! пути, обходя реестр так же, как ходит `prefs::ResetSettings` (по
//! `ReflectSettingsGroup`), а отслеженное — по `prefs::TrackedPrefs`, куда
//! `track_pref` пишет каждый свой вызов:
//!
//! * группа настроек, что не отслежена, — её правки молча не попадают в
//!   `settings.toml` (тот же класс, от которого когда-то ушёл ручной список
//!   отслеживаемого: `RoadStyle` в нём не хватало);
//! * отслеженное, что не группа настроек, — его правки только будят запись
//!   файла, а сами в него не уезжают.
//!
//! Само присутствие типа в реестре здесь не «register_type у владельца»:
//! фича `reflect_auto_register` (приезжает с `default_app`) кладёт в
//! `AppTypeRegistry` каждый Reflect-тип с его `#[reflect(..)]`-данными сама,
//! так что `ResetSettings` и `bevy_settings` видят группу и без ручного
//! `register_type`. Проверка потому и сверяет именно парность двух реестров —
//! видимого настройками и отслеженного.

use bevy::asset::{AssetApp, AssetPlugin};
use bevy::prelude::*;
use bevy::settings::ReflectSettingsGroup;
use bevy::shader::Shader;
use bevy::state::app::StatesPlugin;
use qwe::camera::SavedCameraView;
use qwe::map::sun::{SunOnMap, SunStyle};
use qwe::map::{RoadShape, RoadShapeOnMap};
use qwe::prefs::TrackedPrefs;
use qwe::{
    camera, city, demon, determinism, dev, diagnostics, frame_phases, human, loading, map,
    movement, navigation, portal, post, restart, rng, silhouette, sim_time, spatial, telemetry, ui,
};
use std::any::TypeId;

/// Приложение из плагинов игры — список повторяет `main.rs` минус то, что
/// тесту не нужно: `DefaultPlugins` (ни окна, ни рендера — расписания не
/// запускаются), `RemotePlugin`, `QuitOnEscPlugin` и `PrefsPlugin` (его
/// `SettingsPlugin` читает пользовательский `settings.toml`; реестру типов и
/// списку отслеженного он без надобности). Планируя новый плагин в `main.rs`,
/// добавь его и сюда, иначе его группы останутся вне проверки.
fn game_app() -> App {
    let mut app = App::new();
    // AssetPlugin: сборка плагинов игры спрашивает `AssetServer` (тема перьев
    // и атлас силуэтов берут шрифты); расписания не запускаются, так что до
    // настоящей загрузки ассетов дело не доходит
    app.add_plugins((MinimalPlugins, StatesPlugin, AssetPlugin::default()))
        // `Shader` — Material2dPlugin выделяет хэндл шейдера на своей сборке
        // (в игре его заводит ImagePlugin из DefaultPlugins)
        .init_asset::<Shader>()
        .add_plugins((
            city::CityPlugin,
            rng::RngPlugin,
            determinism::DeterminismPlugin,
            post::PostProcessPlugin,
            silhouette::SilhouettePlugin,
            // кортеж `Plugins` кончается на двадцати — вложенный, как в main.rs
            (
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
                (
                    dev::DevPlugin,
                    frame_phases::FramePhasesPlugin,
                    camera::ViewportPlugin,
                ),
            ),
        ));
    app
}

#[test]
fn every_registered_settings_group_is_tracked() {
    let app = game_app();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let types = registry.read();
    let tracked = app.world().resource::<TrackedPrefs>();

    // Осознанные не-отслеживаемые группы: у каждой запись запускает другой
    // ресурс, и отслеживать саму группу — значит будить запись не вовремя.
    // `SavedCameraView` меняется каждый кадр протяжки камеры — пишет его
    // дебаунс `camera::track_camera_view`; `SunStyle` и `RoadShape` —
    // пишут на каждом пройденном делении ползунка, в файл уезжают через
    // оседание, отслеживается их осевшая копия (`SunOnMap`/`RoadShapeOnMap`).
    const UNTRACKED: [TypeId; 3] = [
        TypeId::of::<SavedCameraView>(),
        TypeId::of::<SunStyle>(),
        TypeId::of::<RoadShape>(),
    ];

    let mut untracked = Vec::new();
    for registration in types.iter() {
        if registration.data::<ReflectSettingsGroup>().is_none() {
            continue;
        }
        if UNTRACKED.contains(&registration.type_id()) {
            continue;
        }
        if !tracked.contains(&registration.type_id()) {
            untracked.push(registration.type_info().ty().path());
        }
    }

    assert!(
        untracked.is_empty(),
        "группы зарегистрированы, но не отслежены — их правки не попадут в \
         settings.toml: {untracked:?}"
    );
}

#[test]
fn every_tracked_resource_is_a_registered_group() {
    let app = game_app();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let types = registry.read();
    let tracked = app.world().resource::<TrackedPrefs>();

    // Осознанные не-группы среди отслеженного: осевшие копии ползуночных
    // настроек. Правки самой группы (`SunStyle`/`RoadShape`) будили бы запись
    // на каждом делении, поэтому отслеживают копию — она меняется один раз,
    // на оседании, а в файл уезжает весь файл целиком.
    const SAVE_TRIGGERS: [TypeId; 2] = [TypeId::of::<SunOnMap>(), TypeId::of::<RoadShapeOnMap>()];

    let mut not_groups = Vec::new();
    let mut not_registered = Vec::new();
    for type_id in tracked.iter() {
        let Some(registration) = types.get(type_id) else {
            not_registered.push(format!("{type_id:?}"));
            continue;
        };
        if SAVE_TRIGGERS.contains(&type_id) {
            continue;
        }
        if registration.data::<ReflectSettingsGroup>().is_none() {
            not_groups.push(registration.type_info().ty().path());
        }
    }

    assert!(
        not_registered.is_empty(),
        "отслеженные типы вне реестра — register_type у них не вызван: \
         {not_registered:?}"
    );
    assert!(
        not_groups.is_empty(),
        "отслеженные, но не группы настроек — их правки будят запись, а сами в \
         settings.toml не попадают: {not_groups:?}"
    );
}
