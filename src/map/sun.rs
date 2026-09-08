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
//! переписать полдюжины сигнатур ради двух чисел. Правило то же, что у
//! навтайла: **пишет глобаль только `apply_sun`, и в том же кадре
//! пересобирается всё, что от неё зависит** (`map/mod.rs`).

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use crate::settings::{SUN_AZIMUTH_DEFAULT, SUN_ELEVATION_DEFAULT};

/// Азимут и высота, как их видит остальная карта. `AtomicU32` — биты `f32`:
/// значения меняются раз в несколько кадров, а читаются сотнями тысяч раз за
/// сборку слоя.
static AZIMUTH: AtomicU32 = AtomicU32::new(SUN_AZIMUTH_DEFAULT.to_bits());
static ELEVATION: AtomicU32 = AtomicU32::new(SUN_ELEVATION_DEFAULT.to_bits());

/// Куда падает тень в плане, единичный вектор. Азимут отсчитывается как на
/// компасе — от севера по часовой стрелке, — и тень падает **от** солнца:
/// солнце на юго-востоке (135°) кладёт тень на северо-запад.
pub fn shadow_dir() -> Vec2 {
    let azimuth = f32::from_bits(AZIMUTH.load(Ordering::Relaxed)).to_radians();
    // север это +Y, восток +X; тень направлена противоположно солнцу
    -Vec2::new(azimuth.sin(), azimuth.cos())
}

/// Направление **на солнце** в плане — то, к чему повёрнута освещённая грань.
pub fn sun_light() -> Vec2 {
    -shadow_dir()
}

/// Метров тени на метр высоты — котангенс высоты солнца. Высота зажата
/// снизу: на пяти градусах тень уходит за край карты и всякий дом накрывает
/// полквартала.
pub fn shadow_length_scale() -> f32 {
    let elevation = f32::from_bits(ELEVATION.load(Ordering::Relaxed));
    1.0 / elevation.to_radians().tan()
}

/// Час съёмки: азимут и высота солнца, градусы. Ползунки секции Sun,
/// сохраняются между запусками; правка пересобирает всё, что освещено, —
/// зданиевые слои с тенями, машины и юниформ материала кровель.
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

/// Ресурс — в глобаль. Единственное место, которое её пишет.
pub fn apply_sun(sun: Res<SunStyle>) {
    AZIMUTH.store(sun.azimuth.to_bits(), Ordering::Relaxed);
    ELEVATION.store(sun.elevation.to_bits(), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Тесты идут в одном процессе и делят глобаль — каждый ставит своё
    /// солнце сам и возвращает дефолт за собой.
    fn with_sun(azimuth: f32, elevation: f32, body: impl FnOnce()) {
        AZIMUTH.store(azimuth.to_bits(), Ordering::Relaxed);
        ELEVATION.store(elevation.to_bits(), Ordering::Relaxed);
        body();
        AZIMUTH.store(SUN_AZIMUTH_DEFAULT.to_bits(), Ordering::Relaxed);
        ELEVATION.store(SUN_ELEVATION_DEFAULT.to_bits(), Ordering::Relaxed);
    }

    #[test]
    fn the_shadow_falls_away_from_the_sun() {
        // солнце на востоке — тень на запад
        with_sun(90.0, 45.0, || {
            let shadow = shadow_dir();
            assert!(shadow.x < -0.99, "{shadow:?}");
            assert!(sun_light().x > 0.99);
        });
        // на юге — тень на север
        with_sun(180.0, 45.0, || {
            let shadow = shadow_dir();
            assert!(shadow.y > 0.99, "{shadow:?}");
        });
    }

    #[test]
    fn the_default_keeps_the_light_in_the_upper_left_corner() {
        // прежняя константа: тень вправо-вниз на 30°
        let shadow = shadow_dir();
        assert!((shadow.x - 0.866).abs() < 0.01, "{shadow:?}");
        assert!((shadow.y + 0.5).abs() < 0.01, "{shadow:?}");
    }

    #[test]
    fn a_lower_sun_makes_a_longer_shadow() {
        with_sun(315.0, 30.0, || assert!(shadow_length_scale() > 1.7));
        with_sun(315.0, 60.0, || assert!(shadow_length_scale() < 0.6));
        // дефолт — прежние 0.6 метра тени на метр высоты
        assert!((shadow_length_scale() - 0.6).abs() < 0.01);
    }
}
