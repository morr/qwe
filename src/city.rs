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

use crate::grid::NavtileBase;
use crate::loading::AppState;
use crate::map::RoadShapeOnMap;
use crate::prefs::{TrackPrefExt, retuned};
use crate::settings::MAP_SIZE;

/// Гео-центр Тулы (от Мясново на западе до Пролетарского моста на востоке,
/// Кремль — в правой части кадра). Проекция — локальная равнопромежуточная:
/// метры от юго-западного угла bbox. `(широта, долгота)`.
const TULA_GEO_CENTER: DVec2 = DVec2::new(54.18969, 37.59148);
/// Рязань: исторический центр, Кремль и Трубеж — в правой части кадра.
const RYAZAN_GEO_CENTER: DVec2 = DVec2::new(54.62800, 39.73500);
/// Калуга: центр у Театральной улицы, Ока — вдоль южного края.
const KALUGA_GEO_CENTER: DVec2 = DVec2::new(54.51600, 36.26000);
/// Орёл: слияние Оки и Орлика, центр по обоим берегам.
const ORYOL_GEO_CENTER: DVec2 = DVec2::new(52.96700, 36.07000);
/// Ростов-на-Дону: Большая Садовая, Дон — в южной части кадра.
const ROSTOV_GEO_CENTER: DVec2 = DVec2::new(47.22700, 39.71500);
/// Белгород: Соборная площадь, Везелка — по южному краю центра.
const BELGOROD_GEO_CENTER: DVec2 = DVec2::new(50.59600, 36.58700);
/// Москва, центр: Кремль, Садовое кольцо почти целиком в кадре.
const MOSCOW_CENTER_GEO_CENTER: DVec2 = DVec2::new(55.75200, 37.61750);
/// Москва, север: Коптево и Войковская.
const MOSCOW_NORTH_GEO_CENTER: DVec2 = DVec2::new(55.82000, 37.52000);
/// Москва, северо-восток: Свиблово и Бабушкинский, край Лосиного Острова.
const MOSCOW_NORTH_EAST_GEO_CENTER: DVec2 = DVec2::new(55.86000, 37.65000);
/// Москва, восток: Перово и Измайлово.
const MOSCOW_EAST_GEO_CENTER: DVec2 = DVec2::new(55.77000, 37.78000);
/// Москва, юго-восток: Текстильщики и Кузьминки.
const MOSCOW_SOUTH_EAST_GEO_CENTER: DVec2 = DVec2::new(55.70000, 37.75000);
/// Москва, юг: Чертаново и Царицыно.
const MOSCOW_SOUTH_GEO_CENTER: DVec2 = DVec2::new(55.62500, 37.62500);
/// Москва, юго-запад: Черёмушки и Академический.
const MOSCOW_SOUTH_WEST_GEO_CENTER: DVec2 = DVec2::new(55.67000, 37.56000);
/// Москва, запад: Фили и Кунцево.
const MOSCOW_WEST_GEO_CENTER: DVec2 = DVec2::new(55.73300, 37.47000);
/// Москва, северо-запад: Щукино и Южное Тушино, канал имени Москвы.
const MOSCOW_NORTH_WEST_GEO_CENTER: DVec2 = DVec2::new(55.81500, 37.45500);
/// Гео-центр Нью-Йорка: Ист-Ривер у Бруклинского моста, в кадре — Даунтаун
/// Манхэттена и северо-западный Бруклин.
const NY_GEO_CENTER: DVec2 = DVec2::new(40.70979, -73.97284);
/// Париж: Иль-де-ла-Сите, Сена делит кадр надвое.
const PARIS_GEO_CENTER: DVec2 = DVec2::new(48.85565, 2.34612);
/// Берлин: Митте, Музейный остров.
const BERLIN_GEO_CENTER: DVec2 = DVec2::new(52.51900, 13.40133);
/// Лондон: Ковент-Гарден, Темза в южной части кадра.
const LONDON_GEO_CENTER: DVec2 = DVec2::new(51.51190, -0.12240);
/// Токио: Ёцуя между Синдзюку и Императорским дворцом — сплошная застройка,
/// Токийский залив за восточным краем bbox.
const TOKYO_GEO_CENTER: DVec2 = DVec2::new(35.68950, 139.72900);
/// Devils Lake, Северная Дакота: американский городок на 7 тысяч жителей —
/// сетка улиц в середине кадра, вокруг поля и озёра.
const DEVILS_LAKE_GEO_CENTER: DVec2 = DVec2::new(48.11379, -98.85592);

/// Центр портала — хинт; при загрузке снапится к ближайшему проходимому
/// тайлу. Тула: гео-точка (54.1908, 37.5836) — перекрёсток к северу от
/// Центрального парка.
///
/// Точка в **метрах от юго-западного угла bbox**, то есть она держится не за
/// город, а за [`MAP_SIZE`](crate::settings::MAP_SIZE): раздвинули карту —
/// угол уехал, и хинт обязан уехать с ним (последний раз на +1000 по обеим
/// осям вместе с раздвигом до 7600 × 5700). Гео-точка в комментарии — то, чем
/// такой сдвиг проверяется.
const TULA_PORTAL_POS: Vec2 = Vec2::new(3284.0, 2969.0);
/// Нью-Йорк: Чайнатаун / Ист-Сайд. В самом центре bbox — Ист-Ривер, поэтому
/// хинт сдвинут на сушу, а не взят серединой карты.
const NY_PORTAL_POS: Vec2 = Vec2::new(2352.0, 3263.0);
/// Остальные города центрированы по суше — хинту хватает середины карты.
///
/// `pub` в отличие от соседей: середину карты как хинт портала берёт ещё и
/// демо-сцена толпы (`examples/demos/crowd_demo`), у которой города нет вовсе.
pub const MAP_CENTER_PORTAL_POS: Vec2 = Vec2::new(MAP_SIZE.x / 2.0, MAP_SIZE.y / 2.0);

/// Город, по которому строится карта.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "world", key = "city")]
pub enum City {
    #[default]
    Tula,
    Ryazan,
    Kaluga,
    Oryol,
    Rostov,
    Belgorod,
    MoscowCenter,
    MoscowNorth,
    MoscowNorthEast,
    MoscowEast,
    MoscowSouthEast,
    MoscowSouth,
    MoscowSouthWest,
    MoscowWest,
    MoscowNorthWest,
    NewYork,
    Paris,
    Berlin,
    London,
    Tokyo,
    DevilsLake,
}

impl City {
    pub const ALL: [Self; 21] = [
        Self::Tula,
        Self::Ryazan,
        Self::Kaluga,
        Self::Oryol,
        Self::Rostov,
        Self::Belgorod,
        Self::MoscowCenter,
        Self::MoscowNorth,
        Self::MoscowNorthEast,
        Self::MoscowEast,
        Self::MoscowSouthEast,
        Self::MoscowSouth,
        Self::MoscowSouthWest,
        Self::MoscowWest,
        Self::MoscowNorthWest,
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
            Self::Ryazan => "Ryazan",
            Self::Kaluga => "Kaluga",
            Self::Oryol => "Oryol",
            Self::Rostov => "Rostov",
            Self::Belgorod => "Belgorod",
            Self::MoscowCenter => "Moscow C",
            Self::MoscowNorth => "Moscow N",
            Self::MoscowNorthEast => "Moscow NE",
            Self::MoscowEast => "Moscow E",
            Self::MoscowSouthEast => "Moscow SE",
            Self::MoscowSouth => "Moscow S",
            Self::MoscowSouthWest => "Moscow SW",
            Self::MoscowWest => "Moscow W",
            Self::MoscowNorthWest => "Moscow NW",
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
            Self::Ryazan => "ryazan",
            Self::Kaluga => "kaluga",
            Self::Oryol => "oryol",
            Self::Rostov => "rostov",
            Self::Belgorod => "belgorod",
            Self::MoscowCenter => "moscow_c",
            Self::MoscowNorth => "moscow_n",
            Self::MoscowNorthEast => "moscow_ne",
            Self::MoscowEast => "moscow_e",
            Self::MoscowSouthEast => "moscow_se",
            Self::MoscowSouth => "moscow_s",
            Self::MoscowSouthWest => "moscow_sw",
            Self::MoscowWest => "moscow_w",
            Self::MoscowNorthWest => "moscow_nw",
            Self::NewYork => "ny",
            Self::Paris => "paris",
            Self::Berlin => "berlin",
            Self::London => "london",
            Self::Tokyo => "tokyo",
            Self::DevilsLake => "devils_lake",
        }
    }

    /// Центр bbox выгрузки — `(широта, долгота)`.
    pub fn geo_center(self) -> DVec2 {
        match self {
            Self::Tula => TULA_GEO_CENTER,
            Self::Ryazan => RYAZAN_GEO_CENTER,
            Self::Kaluga => KALUGA_GEO_CENTER,
            Self::Oryol => ORYOL_GEO_CENTER,
            Self::Rostov => ROSTOV_GEO_CENTER,
            Self::Belgorod => BELGOROD_GEO_CENTER,
            Self::MoscowCenter => MOSCOW_CENTER_GEO_CENTER,
            Self::MoscowNorth => MOSCOW_NORTH_GEO_CENTER,
            Self::MoscowNorthEast => MOSCOW_NORTH_EAST_GEO_CENTER,
            Self::MoscowEast => MOSCOW_EAST_GEO_CENTER,
            Self::MoscowSouthEast => MOSCOW_SOUTH_EAST_GEO_CENTER,
            Self::MoscowSouth => MOSCOW_SOUTH_GEO_CENTER,
            Self::MoscowSouthWest => MOSCOW_SOUTH_WEST_GEO_CENTER,
            Self::MoscowWest => MOSCOW_WEST_GEO_CENTER,
            Self::MoscowNorthWest => MOSCOW_NORTH_WEST_GEO_CENTER,
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
                    .run_if(
                        retuned::<City>
                            .or_else(retuned::<NavtileBase>)
                            .or_else(lane_width_moved),
                    ),
            );
    }
}

/// Осевшая ручка `Lane width` разошлась с шириной, с которой разобран мир:
/// ширину читает разбор (дома отодвигаются от тротуаров, стоянки
/// подтягиваются к дорогам), так что её смена — тот же возврат в загрузку,
/// что смена города. Простое сравнение, без окна `is_changed`: расхождение
/// держится, пока мир не перезагружен, и пропустить его нельзя. Без ресурса
/// (сцена без `MapPlugin`) не срабатывает.
fn lane_width_moved(shape: Option<Res<RoadShapeOnMap>>) -> bool {
    shape.is_some_and(|shape| shape.0.lane_width() != crate::map::lane_width())
}

/// Возврат в `Loading` под новый город, размер навтайла или ширину полосы.
/// Гейт `in_state(Playing)` тут не только про UI: перезапускать загрузку поверх
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
