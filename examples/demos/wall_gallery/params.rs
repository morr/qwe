//! Что решает, как выглядит стена, помимо самого материала, — одной таблицей.
//!
//! Устройство то же, что у `roof_gallery/params.rs`, и это не лень: строка
//! панели описана как [`ParamSpec`] потому, что обработчик протяжки у всех
//! ручек один — «округли до шага, положи в поле, перепиши число».
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

use qwe::settings::{
    ROOF_TEXTURE_DEFAULT, SUN_AZIMUTH_DEFAULT, SUN_AZIMUTH_MAX, SUN_AZIMUTH_MIN, SUN_AZIMUTH_STEP,
    SUN_ELEVATION_DEFAULT, SUN_ELEVATION_MAX, SUN_ELEVATION_MIN, SUN_ELEVATION_STEP,
};

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

/// Одна ручка панели: как её звать, в каких пределах крутить и куда писать.
pub(crate) struct ParamSpec {
    pub(crate) label: &'static str,
    /// `(min, max, step)` — как у строк-ползунков в панелях игры.
    pub(crate) range: (f32, f32, f32),
    pub(crate) get: fn(&Tuning) -> f32,
    pub(crate) set: fn(&mut Tuning, f32),
    pub(crate) format: fn(f32) -> String,
    /// Заголовок группы, если эта ручка её открывает.
    pub(crate) group: Option<&'static str>,
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
            range: (0.0, 1.5, 0.05),
            get: |t| t.texture,
            set: |t, v| t.texture = v,
            format: percent,
            group: Some("Фактура"),
        },
        ParamSpec {
            label: "Length",
            range: (8.0, 60.0, 2.0),
            get: |t| t.length,
            set: |t, v| t.length = v,
            format: metres,
            group: Some("Дом"),
        },
        ParamSpec {
            label: "Axis",
            range: (0.0, 90.0, 5.0),
            get: |t| t.axis_deg,
            set: |t, v| t.axis_deg = v,
            format: degrees,
            group: None,
        },
        ParamSpec {
            label: "Seed",
            range: (0.0, 1.0, 0.05),
            get: |t| t.seed,
            set: |t, v| t.seed = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Courtyard",
            range: (0.0, 0.5, 0.05),
            get: |t| t.courtyard,
            set: |t, v| t.courtyard = v,
            format: courtyard,
            group: None,
        },
        ParamSpec {
            label: "Azimuth",
            range: (SUN_AZIMUTH_MIN, SUN_AZIMUTH_MAX, SUN_AZIMUTH_STEP),
            get: |t| t.sun_azimuth,
            set: |t, v| t.sun_azimuth = v,
            format: degrees,
            group: Some("Солнце"),
        },
        ParamSpec {
            label: "Elevation",
            range: (SUN_ELEVATION_MIN, SUN_ELEVATION_MAX, SUN_ELEVATION_STEP),
            get: |t| t.sun_elevation,
            set: |t, v| t.sun_elevation = v,
            format: degrees,
            group: None,
        },
    ]
}
