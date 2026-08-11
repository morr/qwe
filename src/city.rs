//! Выбранный город: гео-центр выгрузки OSM, хинт портала и имя кеша. Смена
//! города — полная перезагрузка мира: сущности сцены живут под
//! `DespawnOnExit(AppState::Playing)`, поэтому достаточно вернуть приложение
//! в `Loading` — оно само despawn'ит мир, качает новую выгрузку, заново
//! заливает navmesh и спавнит население.
//!
//! Панель выбора — `ui/city.rs`; выбор запоминается между запусками
//! (`prefs.rs`).

use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::loading::AppState;
use crate::prefs::{TrackPrefExt, retuned};
use crate::settings::{
    BERLIN_GEO_CENTER, LONDON_GEO_CENTER, MAP_CENTER_PORTAL_POS, NY_GEO_CENTER, NY_PORTAL_POS,
    NavtileBase, PARIS_GEO_CENTER, TOKYO_GEO_CENTER, TULA_GEO_CENTER, TULA_PORTAL_POS,
};

/// Город, по которому строится карта.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "world", key = "city")]
pub enum City {
    #[default]
    Tula,
    NewYork,
    Paris,
    Berlin,
    London,
    Tokyo,
}

impl City {
    pub const ALL: [Self; 6] = [
        Self::Tula,
        Self::NewYork,
        Self::Paris,
        Self::Berlin,
        Self::London,
        Self::Tokyo,
    ];

    /// Подпись на кнопке.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tula => "Tula",
            Self::NewYork => "NY",
            Self::Paris => "Paris",
            Self::Berlin => "Berlin",
            Self::London => "London",
            Self::Tokyo => "Tokyo",
        }
    }

    /// Префикс файла кеша выгрузки.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Tula => "tula",
            Self::NewYork => "ny",
            Self::Paris => "paris",
            Self::Berlin => "berlin",
            Self::London => "london",
            Self::Tokyo => "tokyo",
        }
    }

    /// Центр bbox выгрузки — `(широта, долгота)`.
    pub fn geo_center(self) -> DVec2 {
        match self {
            Self::Tula => TULA_GEO_CENTER,
            Self::NewYork => NY_GEO_CENTER,
            Self::Paris => PARIS_GEO_CENTER,
            Self::Berlin => BERLIN_GEO_CENTER,
            Self::London => LONDON_GEO_CENTER,
            Self::Tokyo => TOKYO_GEO_CENTER,
        }
    }

    /// Хинт позиции портала в метрах карты (снапится к проходимому тайлу).
    pub fn portal_hint(self) -> Vec2 {
        match self {
            Self::Tula => TULA_PORTAL_POS,
            Self::NewYork => NY_PORTAL_POS,
            _ => MAP_CENTER_PORTAL_POS,
        }
    }
}

pub struct CityPlugin;

impl Plugin for CityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<City>()
            .register_type::<City>()
            .track_pref::<City>()
            .add_systems(
                Update,
                reload_world
                    .run_if(in_state(AppState::Playing))
                    // `retuned` по каждому: ресурс числится изменённым и в
                    // кадре, где его вставили настройки
                    .run_if(retuned::<City>.or_else(retuned::<NavtileBase>)),
            );
    }
}

/// Возврат в `Loading` под новый город или размер навтайла. Гейт
/// `in_state(Playing)` тут не только про UI: перезапускать загрузку поверх
/// уже идущей — значит пустить два потока в один и тот же navmesh. Смена
/// navtile по BRP во время `Loading` по той же причине не подхватывается на
/// лету: мир доедет консистентным на старом размере, атомик и ресурс
/// сойдутся на следующей перезагрузке.
fn reload_world(city: Res<City>, navtile: Res<NavtileBase>, mut next: ResMut<NextState<AppState>>) {
    info!(
        "world reload: city {:?}, navtile {}",
        *city,
        navtile.label()
    );
    // сбросы — не здесь: состояние прогона (спавнер, счётчики, часы) чистят
    // обсерверы `WorldStarted` на входе нового мира в `Live`, производное от
    // карты — его владельцы на `OnExit(Playing)` (`navigation`, `determinism`)
    next.set(AppState::Loading);
}
