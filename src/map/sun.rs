//! Солнце карты: азимут и высота над горизонтом — одна пара чисел, которой
//! отвечает всё, что на карте освещено или отбрасывает тень.
//!
//! Раньше это были две константы (`SHADOW_DIR` и `SUN_ELEVATION_DEG`), и
//! менять их можно было только пересборкой. Час съёмки, между тем, — главное,
//! что задаёт вид города с воздуха: полуденный снимок с короткими тенями и
//! вечерний с длинными косыми это два разных города. Теперь это ползунки
//! секции Sun.
//!
//! **Читается глобалью, а не ресурсом**, как размер навтайла
//! (`settings::navtile_size`): направление света нужно `shade_by_light`,
//! свипу теней, коробкам на кровле, машинам — всё это чистые функции глубоко
//! внутри сборки мешей, и протаскивать `Res<SunStyle>` через каждую значило бы
//! переписать полдюжины сигнатур ради двух чисел.
//!
//! # Что лежит в глобали
//!
//! Не градусы, а уже посчитанные вектор тени и котангенс высоты. Чтение идёт
//! на всякое видимое ребро и всякую точку кольца тени — сотни тысяч раз за
//! сборку слоя, — а `sin`/`cos` из-под атомика компилятор из цикла не
//! выносит: атомик не подлежит CSE, и тригонометрия считалась бы на каждый
//! вызов заново. Поэтому её считает один раз [`apply_sun`], а чтение остаётся
//! тем дешёвым `load`, каким его и описывает эта строка.
//!
//! # Кто пишет глобаль и кто вправе её читать
//!
//! Пишет одна система — [`apply_sun`], и запускается она дважды: разово в
//! `Startup` (посев, сразу за [`seed_sun`] и до
//! `buildings::material::init_roof_material`, которое печёт свет в юниформ
//! материала кровель на всю жизнь процесса), а дальше каждый кадр в
//! `PreUpdate`. Это **не** правило навтайла (тот пишется только когда ни один
//! поток-читатель не жив): здесь запись идёт безусловно, и держится всё на
//! порядке внутри кадра. Отсюда предел, который надо знать, добавляя читателя:
//!
//! - читать солнце можно **на главном потоке — в `Startup` после посева и
//!   дальше между `PreUpdate` и концом `Update`**: там сидят и разовая сборка
//!   материалов (`Startup`), и сборка мира на входе в состояние
//!   (`StateTransition`), и пересборки слоёв (`Update`);
//! - читатель в `PostUpdate`, в рендер-мире или в фоновом потоке (загрузка
//!   OSM солнце не читает — и не должна начать) увидит четыре независимых
//!   атомика в произвольной стадии обновления, то есть направление от одного
//!   солнца с длиной от другого. Такому читателю полагается [`SunOnMap`]
//!   ресурсом, а не эти функции.
//!
//! # Ползунок и карта — разные солнца
//!
//! [`SunStyle`] — то, что стоит на ползунке; [`SunOnMap`] — то, которым карта
//! **уже собрана**, и именно его видит глобаль. Между ними [`settle_sun`]:
//! пересборка по одному делению ползунка стоит сотню миллисекунд на зданиях
//! плюс полтора десятка тысяч деревьев, а делений на шкале семь десятков, так
//! что протяжка через всю шкалу без задержки — это десяток секунд замороженных
//! кадров и столько же перезаписей `settings.toml`. Тот же довод и то же
//! лекарство, что у `camera::track_camera_view`.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{
    SUN_AZIMUTH_DEFAULT, SUN_ELEVATION_DEFAULT, SUN_ELEVATION_MAX, SUN_ELEVATION_MIN,
};

/// Сколько покоя нужно ползунку, чтобы солнце доехало до карты, с реального
/// времени: меньше четверти секунды — и внутри протяжки всё равно набегают
/// пересборки, больше полусекунды — и отпущенный ползунок ощутимо «думает».
const SUN_SETTLE: f32 = 0.35;

/// Тень дефолтного солнца (300° / 59°), посчитанная заранее: статики
/// инициализируются в компайл-тайме, а `sin`/`cos`/`tan` в `const fn` нет.
/// Что эти числа — ровно формула от [`SUN_AZIMUTH_DEFAULT`] и
/// [`SUN_ELEVATION_DEFAULT`], пинует `the_precomputed_default_is_the_formula`.
const DEFAULT_SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);
/// `cot 59°` — метров тени на метр высоты при дефолтном солнце. Все длины
/// теней на карте калибровались под это число, поэтому оно же и делитель
/// [`sun_stretch`].
const DEFAULT_SHADOW_PER_METER: f32 = 0.600_860_6;

/// Готовые числа, а не градусы, — см. «Что лежит в глобали».
static SHADOW_X: AtomicU32 = AtomicU32::new(DEFAULT_SHADOW_DIR.x.to_bits());
static SHADOW_Y: AtomicU32 = AtomicU32::new(DEFAULT_SHADOW_DIR.y.to_bits());
static SHADOW_PER_METER: AtomicU32 = AtomicU32::new(DEFAULT_SHADOW_PER_METER.to_bits());
static SHADOW_STRETCH: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

fn load(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Ordering::Relaxed))
}

/// Куда падает тень в плане, единичный вектор. Азимут отсчитывается как на
/// компасе — от севера по часовой стрелке, — и тень падает **от** солнца:
/// солнце на юго-востоке (135°) кладёт тень на северо-запад.
pub fn shadow_dir() -> Vec2 {
    Vec2::new(load(&SHADOW_X), load(&SHADOW_Y))
}

/// Направление **на солнце** в плане — то, к чему повёрнута освещённая грань.
pub fn sun_light() -> Vec2 {
    -shadow_dir()
}

/// Метров тени на метр высоты — котангенс высоты солнца.
pub fn shadow_length_scale() -> f32 {
    load(&SHADOW_PER_METER)
}

/// Во сколько раз тень длиннее той, что даёт дефолтное солнце. То же число,
/// что [`shadow_length_scale`], только отнесённое к калибровке: длины, которые
/// подбирались на глаз под 59° — зажим теней зданий, высоты теней крон, —
/// умножаются на него и тем самым едут за высотой солнца, не переставая
/// давать при дефолте ровно прежнюю картинку.
pub fn sun_stretch() -> f32 {
    load(&SHADOW_STRETCH)
}

fn store_sun(azimuth: f32, elevation: f32) {
    let azimuth = azimuth.to_radians();
    // север это +Y, восток +X; тень направлена противоположно солнцу
    let dir = -Vec2::new(azimuth.sin(), azimuth.cos());
    let per_meter = 1.0 / elevation.to_radians().tan();
    SHADOW_X.store(dir.x.to_bits(), Ordering::Relaxed);
    SHADOW_Y.store(dir.y.to_bits(), Ordering::Relaxed);
    SHADOW_PER_METER.store(per_meter.to_bits(), Ordering::Relaxed);
    SHADOW_STRETCH.store(
        (per_meter / DEFAULT_SHADOW_PER_METER).to_bits(),
        Ordering::Relaxed,
    );
}

/// Час съёмки: азимут и высота солнца, градусы. Ползунки секции Sun,
/// сохраняются между запусками; правка пересобирает всё, что освещено, —
/// зданиевые слои с тенями, кроны, машины и юниформ материала кровель.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "sun")]
pub struct SunStyle {
    /// Откуда светит, градусы по часовой стрелке от севера. Дефолтные 300°
    /// (запад-северо-запад) дают тень вправо-вниз под 30° — ровно прежнюю
    /// константу `SHADOW_DIR`: свет из верхнего левого угла, как рисуют тени
    /// картографы.
    pub azimuth: f32,
    /// Высота над горизонтом, градусы. 59° — полдень середины лета на широте
    /// Тулы, тот час, когда город и снимают с воздуха.
    pub elevation: f32,
}

impl Default for SunStyle {
    fn default() -> Self {
        Self {
            azimuth: SUN_AZIMUTH_DEFAULT,
            elevation: SUN_ELEVATION_DEFAULT,
        }
    }
}

impl SunStyle {
    /// Азимут, приведённый к обороту: 400° это 40°, −30° это 330°.
    pub fn azimuth(&self) -> f32 {
        if self.azimuth.is_finite() {
            self.azimuth.rem_euclid(360.0)
        } else {
            SUN_AZIMUTH_DEFAULT
        }
    }

    /// Высота, зажатая в диапазон ползунка.
    ///
    /// Зажимает **чтение**, а не ползунок, — по тому же поводу, что и
    /// `PolymeshDebug::radius()`: настройка персистится, и в `settings.toml`
    /// значение может прийти откуда угодно — из записи по BRP, из правки
    /// файла руками, из версии, где границы шкалы стояли другие. Ноль здесь
    /// даёт бесконечный котангенс, то есть NaN-геометрию в `earcutr` и
    /// `i_overlay`, отрицательная высота — тень в сторону солнца, а 90 —
    /// нулевой свип.
    pub fn elevation(&self) -> f32 {
        if self.elevation.is_finite() {
            self.elevation.clamp(SUN_ELEVATION_MIN, SUN_ELEVATION_MAX)
        } else {
            SUN_ELEVATION_DEFAULT
        }
    }
}

/// Солнце, которым карта **собрана**: ползунок оседает в него через
/// [`SUN_SETTLE`] покоя, и уже его правка запускает пересборки
/// (`retuned::<SunOnMap>` в `map/mod.rs`) и запись настроек.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, Default)]
pub struct SunOnMap(pub SunStyle);

/// Стартовое солнце: настройки лежат на `SunStyle` ещё до первого расписания,
/// и карта обязана собраться с ними, а не с компайл-таймовым дефолтом.
/// `set_if_neq` — чтобы дефолтный запуск не считался правкой солнца.
pub fn seed_sun(sun: Res<SunStyle>, mut on_map: ResMut<SunOnMap>) {
    on_map.set_if_neq(SunOnMap(*sun));
}

/// Оседание ползунка: солнце доезжает до карты, когда его перестали крутить.
pub fn settle_sun(
    sun: Res<SunStyle>,
    mut on_map: ResMut<SunOnMap>,
    time: Res<Time<Real>>,
    mut idle: Local<f32>,
) {
    if on_map.0 == *sun {
        *idle = 0.0;
        return;
    }
    if sun.is_changed() {
        *idle = 0.0;
        return;
    }
    *idle += time.delta_secs();
    if *idle >= SUN_SETTLE {
        on_map.0 = *sun;
    }
}

/// Ресурс — в глобаль. Единственное место, которое её пишет.
pub fn apply_sun(sun: Res<SunOnMap>) {
    store_sun(sun.0.azimuth(), sun.0.elevation());
}

/// Замок на процессное солнце для тестов.
///
/// Глобаль одна на процесс, а `cargo test` гоняет тесты параллельными
/// потоками в одном процессе: пока один тест держит своё солнце, полтора
/// десятка тестов в `buildings/tests.rs` и `trees/tests.rs` ассертят
/// дефолтное направление. Поэтому и постановка своего солнца, и чтение
/// дефолтного идут через один и тот же гард.
#[cfg(test)]
static SUN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Держатель солнца: пока гард жив, солнце процесса стоит на заказанном
/// значении и никакой другой тест его не читает; на `Drop` возвращается
/// дефолт.
#[cfg(test)]
pub(crate) struct SunGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

#[cfg(test)]
impl Drop for SunGuard {
    fn drop(&mut self) {
        store_sun(SUN_AZIMUTH_DEFAULT, SUN_ELEVATION_DEFAULT);
    }
}

/// Солнце на время теста. Отравленный замок берётся как есть: одно упавшее
/// утверждение не должно превращаться в полтора десятка чужих падений.
#[cfg(test)]
pub(crate) fn sun_at(azimuth: f32, elevation: f32) -> SunGuard {
    let guard = SUN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    store_sun(azimuth, elevation);
    SunGuard(guard)
}

/// Дефолтное солнце на время теста — для всех, кто просто читает
/// `shadow_dir()` и ждёт от него константы.
#[cfg(test)]
pub(crate) fn default_sun() -> SunGuard {
    sun_at(SUN_AZIMUTH_DEFAULT, SUN_ELEVATION_DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shadow_falls_away_from_the_sun() {
        // солнце на востоке — тень на запад
        {
            let _sun = sun_at(90.0, 45.0);
            let shadow = shadow_dir();
            assert!(shadow.x < -0.99, "{shadow:?}");
            assert!(sun_light().x > 0.99);
        }
        // на юге — тень на север
        let _sun = sun_at(180.0, 45.0);
        let shadow = shadow_dir();
        assert!(shadow.y > 0.99, "{shadow:?}");
    }

    /// Предвычисленный дефолт обязан совпадать с формулой: статики нельзя
    /// инициализировать тригонометрией, и разъехаться этим двум числам с
    /// [`SUN_AZIMUTH_DEFAULT`]/[`SUN_ELEVATION_DEFAULT`] ничто, кроме этого
    /// теста, не мешает.
    #[test]
    fn the_precomputed_default_is_the_formula() {
        let azimuth = SUN_AZIMUTH_DEFAULT.to_radians();
        let expected = -Vec2::new(azimuth.sin(), azimuth.cos());
        assert!(
            DEFAULT_SHADOW_DIR.distance(expected) < 1e-6,
            "{DEFAULT_SHADOW_DIR:?} vs {expected:?}"
        );
        let expected = 1.0 / SUN_ELEVATION_DEFAULT.to_radians().tan();
        assert!(
            (DEFAULT_SHADOW_PER_METER - expected).abs() < 1e-6,
            "{DEFAULT_SHADOW_PER_METER} vs {expected}"
        );
    }

    /// Дефолт — прежние `SHADOW_DIR` и `SHADOW_LENGTH_SCALE` дословно.
    /// Допуск на порядок жёстче прежнего: в 0.01 по длине укладывался целый
    /// градус высоты, то есть «то же число» проверялось слабее, чем звучало.
    #[test]
    fn the_default_keeps_the_light_in_the_upper_left_corner() {
        let _sun = default_sun();
        let shadow = shadow_dir();
        assert!((shadow.x - 0.866).abs() < 1e-3, "{shadow:?}");
        assert!((shadow.y + 0.5).abs() < 1e-3, "{shadow:?}");
        assert!((shadow_length_scale() - 0.6).abs() < 1e-3);
        assert!((sun_stretch() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_lower_sun_makes_a_longer_shadow() {
        {
            let _sun = sun_at(315.0, 30.0);
            assert!(shadow_length_scale() > 1.7);
            assert!(sun_stretch() > 2.8);
        }
        let _sun = sun_at(315.0, 60.0);
        assert!(shadow_length_scale() < 0.6);
        assert!(sun_stretch() < 1.0);
    }

    /// Ползунок доезжает до карты не сразу: пока его крутят, `SunOnMap` стоит
    /// на месте, и ни одна пересборка не заказана.
    #[test]
    fn the_slider_settles_into_the_map_only_after_a_pause() {
        let mut app = App::new();
        app.init_resource::<SunStyle>()
            .init_resource::<SunOnMap>()
            .init_resource::<Time<Real>>()
            .add_systems(Update, settle_sun);
        app.update();

        // деление ползунка
        app.world_mut().resource_mut::<SunStyle>().elevation = 20.0;
        app.update();
        assert_eq!(
            app.world().resource::<SunOnMap>().0.elevation,
            SUN_ELEVATION_DEFAULT
        );

        // ещё не досчитали
        let step = std::time::Duration::from_secs_f32(SUN_SETTLE / 2.0);
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(step);
        app.update();
        assert_eq!(
            app.world().resource::<SunOnMap>().0.elevation,
            SUN_ELEVATION_DEFAULT
        );

        // ползунок отпущен и покой набрался
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(step);
        app.update();
        assert_eq!(app.world().resource::<SunOnMap>().0.elevation, 20.0);
    }

    /// Высота вне шкалы приходит из `settings.toml` и по BRP; котангенс от
    /// нуля — бесконечность, а дальше NaN-геометрия в триангуляции.
    #[test]
    fn an_out_of_range_elevation_is_clamped_on_read() {
        let broken = SunStyle {
            azimuth: 400.0,
            elevation: 0.0,
        };
        assert_eq!(broken.elevation(), SUN_ELEVATION_MIN);
        assert_eq!(broken.azimuth(), 40.0);

        let nan = SunStyle {
            azimuth: f32::NAN,
            elevation: f32::NAN,
        };
        assert_eq!(nan.elevation(), SUN_ELEVATION_DEFAULT);
        assert_eq!(nan.azimuth(), SUN_AZIMUTH_DEFAULT);

        let _sun = sun_at(0.0, broken.elevation());
        assert!(shadow_length_scale().is_finite());
    }
}
