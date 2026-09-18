//! Что решает, как выглядит стена, помимо самого материала, — одной таблицей.
//!
//! Устройство то же, что у `roof_gallery/params.rs`, и это не лень: строка
//! панели описана как [`ParamSpec`] потому, что за протяжкой всех ручек стоит
//! один и тот же наблюдатель кита (`qwe::ui::knob`).
//!
//! Материал и этажность ручками не крутятся: они и есть сетки витрины — все
//! пять облицовок по блокам, лестница этажей внутри блока. Ручки — то, что в
//! игре приходит от **дома**: какой он длины, как повёрнут, какой у него посев
//! и есть ли двор, — плюс два игровых ползунка: сила фактуры
//! (`RoofStyle::texture`, она же амплитуда всего, что рисует стена) и час
//! съёмки (`SunStyle`).
//!
//! Поворот тут не украшение и стоит вторым по важности после материала:
//! ракурс сжимает стену, `fwidth` в шейдере знает об этом сам, и окна на
//! западной стене гаснут раньше, чем на южной. На неподвижной витрине это не
//! проверить никак.

use qwe::map::buildings::material::ROOF_TEXTURE_DEFAULT;
use qwe::map::sun::{
    SUN_AZIMUTH_DEFAULT, SUN_AZIMUTH_MAX, SUN_AZIMUTH_MIN, SUN_AZIMUTH_STEP, SUN_ELEVATION_DEFAULT,
    SUN_ELEVATION_MAX, SUN_ELEVATION_MIN, SUN_ELEVATION_STEP,
};
use qwe::ui::knob::SliderBinding;

/// Длина дома по умолчанию, м. Восемь панелей по 3.2 — столько, чтобы столбец
/// балконов читался столбцом, а не парой пятен.
const DEFAULT_LENGTH: f32 = 26.0;

/// Настройка витрины. `Default` — игра: сила фактуры игровая, дома стоят по
/// сторонам света, дворов нет.
#[derive(bevy::prelude::Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tuning {
    /// `RoofStyle::texture` — общий множитель амплитуд. У стены он глушит всё
    /// разом, окна включительно: на нуле стена снова ровная заливка, и это
    /// единственный честный способ увидеть, сколько именно фактура добавляет.
    pub(crate) texture: f32,
    /// Длина дома, м, — одна на все дома витрины. От неё зависит **число
    /// панелей** в стене (`layers::PANEL_WIDTH` — цель, а не делитель), то
    /// есть и ширина окна в метрах: на 10-метровой стене панель шире, чем на
    /// 40-метровой, и окно вместе с ней.
    pub(crate) length: f32,
    /// Угол длинной оси домов, градусы. Решает, какие стены вообще видимы и
    /// насколько ракурс их сжимает, — а сжатие в шейдере гасит рисунок по
    /// производной координаты.
    pub(crate) axis_deg: f32,
    /// Сдвиг посева всего набора. У каждой **стены** посев свой (в игре — от её
    /// первой точки), и от него зависят столбцы балконов и разброс окон;
    /// ручка крутит их все разом.
    pub(crate) seed: f32,
    /// Доля стороны дома под двор; 0 — без двора. Двор не украшение: у дома с
    /// дыркой игра рисует ещё и стену по дальней стороне двора, и только на
    /// ней видно, что её рисунок такой же, а посев другой.
    pub(crate) courtyard: f32,
    /// Азимут солнца, градусы — игровой `SunStyle::azimuth`. Он решает, какая
    /// из двух видимых стен освещена, а какая в тени (`wall_colors`), то есть
    /// на каком фоне вообще читаются окна.
    pub(crate) sun_azimuth: f32,
    /// Высота солнца, градусы — игровой `SunStyle::elevation`.
    pub(crate) sun_elevation: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            texture: ROOF_TEXTURE_DEFAULT,
            length: DEFAULT_LENGTH,
            axis_deg: 0.0,
            seed: 0.0,
            courtyard: 0.0,
            sun_azimuth: SUN_AZIMUTH_DEFAULT,
            sun_elevation: SUN_ELEVATION_DEFAULT,
        }
    }
}

/// Одна ручка панели: как её звать, под каким заголовком она стоит и к какому
/// полю привязана.
///
/// Привязка — китовая ([`SliderBinding`]), ровно та же, что у ручек панелей
/// игры: диапазон, чтение, запись и подпись. За протяжкой и синхронизацией
/// тогда стоит наблюдатель кита, заведённый по разу на ресурс, а не свой на
/// витрину.
pub(crate) struct ParamSpec {
    pub(crate) label: &'static str,
    /// Заголовок группы, если эта ручка её открывает.
    pub(crate) group: Option<&'static str>,
    pub(crate) binding: SliderBinding<Tuning>,
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn metres(value: f32) -> String {
    format!("{value:.0} м")
}

fn degrees(value: f32) -> String {
    format!("{value:.0}°")
}

fn fraction(value: f32) -> String {
    format!("{value:.2}")
}

/// Доля стороны под двор; ноль — двора нет вовсе, и так и написано.
fn courtyard(value: f32) -> String {
    if value <= 0.0 {
        "нет".to_string()
    } else {
        format!("{value:.2}")
    }
}

/// Все ручки по порядку сверху вниз.
pub(crate) fn specs() -> Vec<ParamSpec> {
    vec![
        ParamSpec {
            label: "Texture",
            group: Some("Фактура"),
            binding: SliderBinding {
                get: |t| t.texture,
                set: |t, v| t.texture = v,
                range: (0.0, 1.5, 0.05),
                text: percent,
            },
        },
        ParamSpec {
            label: "Length",
            group: Some("Дом"),
            binding: SliderBinding {
                get: |t| t.length,
                set: |t, v| t.length = v,
                range: (8.0, 60.0, 2.0),
                text: metres,
            },
        },
        ParamSpec {
            label: "Axis",
            group: None,
            binding: SliderBinding {
                get: |t| t.axis_deg,
                set: |t, v| t.axis_deg = v,
                range: (0.0, 90.0, 5.0),
                text: degrees,
            },
        },
        ParamSpec {
            label: "Seed",
            group: None,
            binding: SliderBinding {
                get: |t| t.seed,
                set: |t, v| t.seed = v,
                range: (0.0, 1.0, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Courtyard",
            group: None,
            binding: SliderBinding {
                get: |t| t.courtyard,
                set: |t, v| t.courtyard = v,
                range: (0.0, 0.5, 0.05),
                text: courtyard,
            },
        },
        ParamSpec {
            label: "Azimuth",
            group: Some("Солнце"),
            binding: SliderBinding {
                get: |t| t.sun_azimuth,
                set: |t, v| t.sun_azimuth = v,
                range: (SUN_AZIMUTH_MIN, SUN_AZIMUTH_MAX, SUN_AZIMUTH_STEP),
                text: degrees,
            },
        },
        ParamSpec {
            label: "Elevation",
            group: None,
            binding: SliderBinding {
                get: |t| t.sun_elevation,
                set: |t, v| t.sun_elevation = v,
                range: (SUN_ELEVATION_MIN, SUN_ELEVATION_MAX, SUN_ELEVATION_STEP),
                text: degrees,
            },
        },
    ]
}
