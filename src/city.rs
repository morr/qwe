//! Выбранный город: гео-центр выгрузки OSM, хинт портала, сердце и имя кеша.
//! Город со срезом ([`Slice`], пока одна Тула) кладёт карту не от центра, а
//! от сердца: портал у края, сердце на [`HEART_DEPTH`] глубины. Смена
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
use crate::map::osm::overpass::GeoBounds;
use crate::prefs::{TrackPrefExt, retuned};
use crate::settings::{
    BERLIN_GEO_CENTER, DEVILS_LAKE_GEO_CENTER, HEART_DEPTH, LONDON_GEO_CENTER,
    MAP_CENTER_PORTAL_POS, MAP_SIZE, METERS_PER_DEG_LAT, NY_GEO_CENTER, NY_PORTAL_POS, NavtileBase,
    PARIS_GEO_CENTER, PORTAL_EDGE_MARGIN, TOKYO_GEO_CENTER, TULA_HEART_GEO, TULA_PORTAL_ACROSS,
};

/// Край карты.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    North,
    South,
    East,
    West,
}

impl Edge {
    pub const ALL: [Self; 4] = [Self::North, Self::South, Self::East, Self::West];

    /// Ось глубины среза идёт вдоль y (север/юг) или вдоль x (восток/запад).
    pub fn along_y(self) -> bool {
        matches!(self, Self::North | Self::South)
    }
}

/// Срез города: карта положена от **сердца**, а не от гео-центра. Сердце —
/// на [`HEART_DEPTH`] глубины от края портала, портал — у противоположного
/// края на `portal_across` вдоль него. Та же рамка, что считает
/// `tools/osm_audit/slice_audit.py::Frame`.
#[derive(Debug, Clone, Copy)]
pub struct Slice {
    /// Цель вторжения, `(широта, долгота)`.
    pub heart: DVec2,
    /// Край карты, у которого стоит портал.
    pub portal_edge: Edge,
    /// Где на этом краю стоит портал, 0…1 с запада на восток (север/юг) или
    /// с юга на север (восток/запад).
    pub portal_across: f32,
}

impl Slice {
    /// Протяжённость карты вдоль оси глубины, м.
    fn extent(&self) -> f32 {
        if self.portal_edge.along_y() {
            MAP_SIZE.y
        } else {
            MAP_SIZE.x
        }
    }

    /// Гео-центр bbox: сердце, сдвинутое **к** краю портала на
    /// `(HEART_DEPTH − 0.5) × протяжённость` — тогда сердце оказывается на
    /// `HEART_DEPTH` от того края. Метры → градусы через `METERS_PER_DEG_LAT`
    /// и масштаб долготы на широте сердца. Округлено до 1e-5° (~1 м): центр
    /// попадает в имя файла кеша, и хвост из 14 знаков там ни к чему.
    pub fn geo_center(&self) -> DVec2 {
        let shift = (HEART_DEPTH - 0.5) as f64 * self.extent() as f64;
        let lon_scale = METERS_PER_DEG_LAT * self.heart.x.to_radians().cos();
        let (dlat, dlon) = match self.portal_edge {
            Edge::North => (shift / METERS_PER_DEG_LAT, 0.0),
            Edge::South => (-shift / METERS_PER_DEG_LAT, 0.0),
            Edge::East => (0.0, shift / lon_scale),
            Edge::West => (0.0, -shift / lon_scale),
        };
        ((self.heart + DVec2::new(dlat, dlon)) * 1e5).round() / 1e5
    }

    /// Хинт портала в метрах карты: на краю портала, `portal_across` вдоль
    /// него, [`PORTAL_EDGE_MARGIN`] от кромки.
    pub fn portal_hint(&self) -> Vec2 {
        let across = self.portal_across;
        match self.portal_edge {
            Edge::North => Vec2::new(across * MAP_SIZE.x, MAP_SIZE.y - PORTAL_EDGE_MARGIN),
            Edge::South => Vec2::new(across * MAP_SIZE.x, PORTAL_EDGE_MARGIN),
            Edge::East => Vec2::new(MAP_SIZE.x - PORTAL_EDGE_MARGIN, across * MAP_SIZE.y),
            Edge::West => Vec2::new(PORTAL_EDGE_MARGIN, across * MAP_SIZE.y),
        }
    }
}

/// Срез Тулы: сердце в кремле, портал на северном краю в Заречье. Ось
/// север → юг выбрана по спайку шага 1: с запада Упа шла бы вдоль оси, и до
/// кремля на южном берегу можно было бы дойти, не пересекая реку.
const TULA_SLICE: Slice = Slice {
    heart: TULA_HEART_GEO,
    portal_edge: Edge::North,
    portal_across: TULA_PORTAL_ACROSS,
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
    DevilsLake,
}

impl City {
    pub const ALL: [Self; 7] = [
        Self::Tula,
        Self::NewYork,
        Self::Paris,
        Self::Berlin,
        Self::London,
        Self::Tokyo,
        Self::DevilsLake,
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
            Self::DevilsLake => "Devils Lake",
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
            Self::DevilsLake => "devils_lake",
        }
    }

    /// Срез города — `Some` только у Тулы. Остальные шесть живут от гео-центра:
    /// их кеши не перекачиваются, хинты порталов не устаревают.
    pub fn slice(self) -> Option<Slice> {
        match self {
            Self::Tula => Some(TULA_SLICE),
            _ => None,
        }
    }

    /// Центр bbox выгрузки — `(широта, долгота)`. У среза вычисляется от
    /// сердца ([`Slice::geo_center`]); имя кеша несёт его в себе, так что
    /// сдвиг сердца перекачивает выгрузку сам.
    pub fn geo_center(self) -> DVec2 {
        match self {
            Self::Tula => TULA_SLICE.geo_center(),
            Self::NewYork => NY_GEO_CENTER,
            Self::Paris => PARIS_GEO_CENTER,
            Self::Berlin => BERLIN_GEO_CENTER,
            Self::London => LONDON_GEO_CENTER,
            Self::Tokyo => TOKYO_GEO_CENTER,
            Self::DevilsLake => DEVILS_LAKE_GEO_CENTER,
        }
    }

    /// Хинт позиции портала в метрах карты (снапится к проходимому тайлу).
    pub fn portal_hint(self) -> Vec2 {
        match self {
            Self::Tula => TULA_SLICE.portal_hint(),
            Self::NewYork => NY_PORTAL_POS,
            _ => MAP_CENTER_PORTAL_POS,
        }
    }

    /// Хинт сердца в метрах карты (снапится к проходимому тайлу в потоке
    /// загрузки). Сердце есть у всех: без среза — середина карты, иначе цель
    /// M1 существовала бы только в Туле.
    pub fn heart_hint(self) -> Vec2 {
        match self.slice() {
            Some(slice) => GeoBounds::for_city(self).project(slice.heart.x, slice.heart.y),
            None => MAP_SIZE / 2.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(edge: Edge) -> Slice {
        Slice {
            heart: TULA_HEART_GEO,
            portal_edge: edge,
            portal_across: 0.25,
        }
    }

    /// Глубина точки от края портала, 0 у самого края, 1 у противоположного.
    fn depth(edge: Edge, point: Vec2) -> f32 {
        match edge {
            Edge::North => (MAP_SIZE.y - point.y) / MAP_SIZE.y,
            Edge::South => point.y / MAP_SIZE.y,
            Edge::East => (MAP_SIZE.x - point.x) / MAP_SIZE.x,
            Edge::West => point.x / MAP_SIZE.x,
        }
    }

    #[test]
    fn the_heart_sits_at_heart_depth_from_the_portal_edge_for_every_edge() {
        for edge in Edge::ALL {
            let slice = slice(edge);
            let bounds = GeoBounds::around(slice.geo_center());
            let heart = bounds.project(slice.heart.x, slice.heart.y);
            let extent = slice.extent();
            assert!(
                (depth(edge, heart) - HEART_DEPTH).abs() * extent < 1.0,
                "{edge:?}: heart at {heart:?}"
            );
            // поперёк оси сердце остаётся на середине карты
            let across = if edge.along_y() {
                heart.x - MAP_SIZE.x / 2.0
            } else {
                heart.y - MAP_SIZE.y / 2.0
            };
            assert!(across.abs() < 1.0, "{edge:?}: heart at {heart:?}");
        }
    }

    #[test]
    fn the_geo_center_shifts_toward_the_portal_edge() {
        let heart = TULA_HEART_GEO;
        assert!(slice(Edge::North).geo_center().x > heart.x);
        assert!(slice(Edge::South).geo_center().x < heart.x);
        assert!(slice(Edge::East).geo_center().y > heart.y);
        assert!(slice(Edge::West).geo_center().y < heart.y);
    }

    #[test]
    fn the_portal_hint_stands_a_margin_inside_the_portal_edge() {
        for edge in Edge::ALL {
            let slice = slice(edge);
            let hint = slice.portal_hint();
            assert!(
                (depth(edge, hint) * slice.extent() - PORTAL_EDGE_MARGIN).abs() < 0.01,
                "{edge:?}: portal at {hint:?}"
            );
            assert!(hint.x > 0.0 && hint.x < MAP_SIZE.x && hint.y > 0.0 && hint.y < MAP_SIZE.y);
        }
    }

    /// Числа спайка шага 1 (`ROADMAP.md`, `slice_audit.py`): сердце Тулы в
    /// метрах карты (2800, 1110), хинт портала (1400, 3450). Без среза сердце —
    /// середина карты.
    #[test]
    fn tula_matches_the_slice_audit() {
        let heart = City::Tula.heart_hint();
        assert!(
            (heart - Vec2::new(2800.0, 1110.0)).length() < 1.0,
            "{heart:?}"
        );
        assert_eq!(City::Tula.portal_hint(), Vec2::new(1400.0, 3450.0));
        assert_eq!(City::Paris.heart_hint(), MAP_SIZE / 2.0);
    }
}
