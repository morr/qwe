//! Запоминание выбранных в UI опций между запусками: выбранный город,
//! дебаг-тумблеры (grid / navmesh / movepath), алгоритм поиска пути и панель
//! стиля деревьев.
//!
//! Поверх первопартийного `bevy::settings` (см. upstream-пример
//! `window/persisting_window_settings.rs`): сами ресурсы помечены
//! `#[derive(SettingsGroup)]` в своих модулях, а `SettingsPlugin` при сборке
//! `App` читает `settings.toml` из системной папки настроек и накладывает
//! значения на уже созданные ресурсы — то есть до любого расписания, так что
//! и UI-панели, и первый спавн мира стартуют с сохранённым выбором.
//!
//! Поэтому плагин регистрируется **последним**: `SettingsPlugin` сканирует
//! реестр типов на своей сборке, и `register_type` остальных плагинов должны
//! к этому моменту уже отработать.
//!
//! Запись — `save_prefs` на любое изменение этих ресурсов, откуда бы оно ни
//! пришло: клик по кнопке, хоткей, правка через BRP. Пишем синхронно, а не
//! `SaveSettingsDeferred`: кликов мало, а отложенная запись теряется, если
//! выйти из игры в ту же секунду.

use bevy::prelude::*;
use bevy::settings::{SaveSettingsSync, SettingsPlugin};

use crate::city::City;
use crate::map::{
    BuildingHeightMode, ConiferNoiseStyle, RoadStyle, TramStyle, TreeRowStyle, TreeStyle,
};
use crate::movement::DrawMovePaths;
use crate::navigation::PathfindingAlgorithm;
use crate::ui::{DebugConiferNoise, DebugDoors, DebugGrid, DebugNavmesh};

/// Обратное доменное имя из URL репозитория — как просит документация
/// `SettingsPlugin`. Определяет папку: на macOS
/// `~/Library/Preferences/com.github.morr.qwe/settings.toml`.
const APP_NAME: &str = "com.github.morr.qwe";

pub struct PrefsPlugin;

impl Plugin for PrefsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SettingsPlugin::new(APP_NAME)).add_systems(
            Update,
            save_prefs.run_if(
                resource_changed::<City>
                    .or_else(resource_changed::<DebugGrid>)
                    .or_else(resource_changed::<DebugNavmesh>)
                    .or_else(resource_changed::<DebugDoors>)
                    .or_else(resource_changed::<DebugConiferNoise>)
                    .or_else(resource_changed::<DrawMovePaths>)
                    .or_else(resource_changed::<PathfindingAlgorithm>)
                    .or_else(resource_changed::<TreeStyle>)
                    .or_else(resource_changed::<TreeRowStyle>)
                    .or_else(resource_changed::<ConiferNoiseStyle>)
                    .or_else(resource_changed::<BuildingHeightMode>)
                    // RoadStyle здесь не хватало с самого начала: его правки
                    // сохранялись, только если в тот же кадр менялся другой
                    // отслеживаемый ресурс
                    .or_else(resource_changed::<RoadStyle>)
                    .or_else(resource_changed::<TramStyle>),
            ),
        );
    }
}

fn save_prefs(mut commands: Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
}
