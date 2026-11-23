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
//!
//! Вторая пара тестов — запись файла (`prefs::SavePrefs`): целое без знака
//! выше `i64` TOML не хранит, а библиотека сериализует группу через
//! `unwrap()`. Мусор раскладывает по **всем** группам обход по реестру, так
//! что поле, объявленное после этих строк, попадает в проверку само.

use bevy::asset::{AssetApp, AssetPlugin};
use bevy::ecs::change_detection::Tick;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::system::Command;
use bevy::prelude::*;
use bevy::reflect::{PartialReflect, ReflectMut};
use bevy::settings::ReflectSettingsGroup;
use bevy::shader::Shader;
use bevy::state::app::StatesPlugin;
use qwe::camera::SavedCameraView;
use qwe::map::sun::{SunOnMap, SunStyle};
use qwe::map::{RoadShape, RoadShapeOnMap};
use qwe::prefs::{SavePrefs, TrackedPrefs};
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

/// Целые без знака шириной 64 бита во всех группах настроек приложения —
/// путь до каждого. С `plant` каждое получает `u64::MAX` и возвращаются все;
/// без — только те, что выше потолка TOML. Обход свой, рядом с `prefs`-овским:
/// тест не должен наследовать его слепые места.
fn settings_unsigned_leaves(world: &mut World, plant: bool) -> Vec<String> {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let types = registry.read();
    let mut found = Vec::new();
    for registration in types.iter() {
        if registration.data::<ReflectSettingsGroup>().is_none() {
            continue;
        }
        let Some(component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(component_id) = world.components().get_id(registration.type_id()) else {
            continue;
        };
        let Some(entity) = world.resource_entities().get(component_id) else {
            continue;
        };
        let Some(mut value) = component.reflect_mut(world.entity_mut(entity)) else {
            continue;
        };
        let name = registration.type_info().type_path_table().short_path();
        found.extend(unsigned_leaves(&mut *value, name.to_string(), plant));
    }
    found
}

/// Группы настроек, помеченные изменёнными после тика `since`.
fn settings_groups_changed_since(world: &World, since: Tick) -> Vec<String> {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let types = registry.read();
    let mut marked = Vec::new();
    for registration in types.iter() {
        if registration.data::<ReflectSettingsGroup>().is_none() {
            continue;
        }
        let Some(component_id) = world.components().get_id(registration.type_id()) else {
            continue;
        };
        let Some(ticks) = world.get_resource_change_ticks_by_id(component_id) else {
            continue;
        };
        if ticks.is_changed(since, world.read_change_tick()) {
            marked.push(
                registration
                    .type_info()
                    .type_path_table()
                    .short_path()
                    .to_string(),
            );
        }
    }
    marked
}

fn unsigned_leaves(value: &mut dyn PartialReflect, path: String, plant: bool) -> Vec<String> {
    /// Один лист типа `T`: с `plant` получает `max`, без — отдаётся только
    /// выше потолка TOML.
    fn integer<T: Copy + 'static>(
        value: &mut dyn PartialReflect,
        path: String,
        plant: bool,
        max: T,
        as_u64: impl Fn(T) -> u64,
    ) -> Option<String> {
        let slot = value.try_downcast_mut::<T>()?;
        if plant {
            *slot = max;
            return Some(path);
        }
        (as_u64(*slot) > i64::MAX as u64).then_some(path)
    }

    fn nested<P: ?Sized>(
        len: usize,
        parent: &mut P,
        path: &str,
        plant: bool,
        mut at: impl FnMut(&mut P, usize) -> Option<&mut dyn PartialReflect>,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for i in 0..len {
            if let Some(field) = at(parent, i) {
                found.extend(unsigned_leaves(field, format!("{path}.{i}"), plant));
            }
        }
        found
    }

    match value.reflect_mut() {
        ReflectMut::Struct(s) => {
            let mut found = Vec::new();
            for i in 0..s.field_len() {
                let name = s.name_at(i).unwrap_or("?").to_string();
                if let Some(field) = s.field_at_mut(i) {
                    found.extend(unsigned_leaves(field, format!("{path}.{name}"), plant));
                }
            }
            found
        }
        ReflectMut::TupleStruct(s) => nested(s.field_len(), s, &path, plant, |s, i| s.field_mut(i)),
        ReflectMut::Tuple(t) => nested(t.field_len(), t, &path, plant, |t, i| t.field_mut(i)),
        ReflectMut::Enum(e) => nested(e.field_len(), e, &path, plant, |e, i| e.field_at_mut(i)),
        ReflectMut::List(l) => nested(l.len(), l, &path, plant, |l, i| l.get_mut(i)),
        ReflectMut::Array(a) => nested(a.len(), a, &path, plant, |a, i| a.get_mut(i)),
        ReflectMut::Opaque(raw) => {
            // `u64` и `usize` — единственные целые без знака, что бывают выше потолка
            integer::<u64>(raw, path.clone(), plant, u64::MAX, |v| v)
                .or_else(|| integer::<usize>(raw, path, plant, usize::MAX, |v| v as u64))
                .into_iter()
                .collect()
        }
        ReflectMut::Map(_) | ReflectMut::Set(_) => Vec::new(),
    }
}

/// `SavePrefs` подрезает целые, которые TOML не хранит, во **всех** группах —
/// включая объявленные после этого теста: мусор раскладывает обход по реестру,
/// а не список полей. (`SettingsPlugin` в приложении нет — его запись без
/// реестра файлов только предупреждает, пользовательский `settings.toml`
/// не тронут.)
#[test]
fn a_save_clamps_oversized_integers_in_every_settings_group() {
    let mut app = game_app();
    let planted = settings_unsigned_leaves(app.world_mut(), true);

    // известные листья — вдруг обход перестанет их видеть
    for leaf in [
        "DemonStyle.cap",
        "HordeStyle.wave_size",
        "HordeStyle.wave_len",
        "WorldSeed.0",
    ] {
        assert!(planted.iter().any(|p| p == leaf), "{leaf:?} не заложен: {planted:?}");
    }

    app.world_mut().clear_trackers();
    let since = app.world().read_change_tick();
    // граница тика, которую в игре даёт кадр между началом и записью:
    // изменение видно, только если его тик строго новее опорного — отметка,
    // поставленная тем же тиком, что и `since`, для `is_changed` невидима
    app.world_mut().increment_change_tick();

    SavePrefs.apply(app.world_mut());

    // подрезанное отмечено изменённым: по метке файл видит правку, а владельцы
    // ручки — новое значение
    let marked = settings_groups_changed_since(app.world(), since);
    for group in ["DemonStyle", "HordeStyle", "WorldSeed"] {
        assert!(marked.iter().any(|m| m == group), "{group} не отмечен: {marked:?}");
    }
    let leftover = settings_unsigned_leaves(app.world_mut(), false);
    assert!(
        leftover.is_empty(),
        "после записи остались целые выше потолка TOML: {leftover:?}"
    );
}

/// Записи, которой нечего подрезать, нечего и метить группы изменёнными: по
/// метке переписывается файл и пересобираются кроны, дороги и здания.
#[test]
fn a_save_with_nothing_to_clamp_marks_no_group() {
    let mut app = game_app();
    app.world_mut().clear_trackers();
    let since = app.world().read_change_tick();
    app.world_mut().increment_change_tick();

    SavePrefs.apply(app.world_mut());

    let marked = settings_groups_changed_since(app.world(), since);
    assert!(marked.is_empty(), "группы отмечены изменёнными: {marked:?}");

    // контроль: та же проверка видит отметку, когда она есть, — обход с
    // правкой метит группы сам. Без этого пустой список выше ничего не держит
    settings_unsigned_leaves(app.world_mut(), false);
    let marked = settings_groups_changed_since(app.world(), since);
    assert!(marked.iter().any(|m| m == "WorldSeed"), "нет ни одной отметки: {marked:?}");
}
