//! Что вообще решает, как выглядит кровля, — одной таблицей.
//!
//! Каждая строка панели описана здесь как [`ParamSpec`]: подпись, диапазон,
//! чтение и запись поля. Таблица, а не по функции на ручку, потому что
//! обработчик протяжки у всех один и тот же — «округли до шага, положи в поле,
//! перепиши число», — и четыре его копии отличались бы только именем поля.
//!
//! Материал, цвет и форма ручками не крутятся: они и есть сетки витрины — все
//! шесть материалов с их палитрами по блокам, все формы по контурам.
//! Ручки — то, что в игре приходит от **дома**: какой он высоты, как повёрнут,
//! какой у него посев фазы и есть ли двор, — плюс два игровых ползунка: сила
//! фактуры кровель (`RoofStyle::texture`) и час съёмки (`SunStyle`), общий для
//! всей карты.

use qwe::map::buildings::material::ROOF_TEXTURE_DEFAULT;
use qwe::map::sun::{
    SUN_AZIMUTH_DEFAULT, SUN_AZIMUTH_MAX, SUN_AZIMUTH_MIN, SUN_AZIMUTH_STEP, SUN_ELEVATION_DEFAULT,
    SUN_ELEVATION_MAX, SUN_ELEVATION_MIN, SUN_ELEVATION_STEP,
};
use qwe::ui::knob::{KnobSpec, SliderBinding};

/// Высота стен по умолчанию, м: типичная из `HOUSE_HEIGHTS` в `heights.rs` —
/// одноэтажный частный дом, то есть тот класс, которому игра вообще ставит
/// скатную крышу.
const DEFAULT_HEIGHT: f32 = 3.2;

/// Настройка витрины. `Default` — игра: сила фактуры игровая, дома стоят по
/// сторонам света, дворов нет.
#[derive(bevy::prelude::Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tuning {
    /// `RoofStyle::texture` — общий множитель амплитуд фактуры. Не геометрия:
    /// уходит юниформом в материал, меши от него не пересобираются (витрина
    /// всё равно пересобирает — ей дешевле, чем городу).
    pub(crate) texture: f32,
    /// Высота стен, м, — одна на все дома витрины. В игре её даёт тег OSM или
    /// вывод по пятну (`heights.rs`), здесь она задана прямо: сравниваются
    /// формы крыш, и разная высота стен под ними сбивала бы сравнение. От неё
    /// зависит подъём кровли над контуром (`extrusion_lift`), то есть
    /// насколько дом вообще читается домом.
    pub(crate) height: f32,
    /// Угол длинной оси домов, градусы. В игре ось даёт `min_area_rect`
    /// контура; здесь дома прямоугольные, и угол задаётся напрямую — по нему
    /// же повёрнута и рамка кровли, так что швы ковра и рёбра фальца обязаны
    /// поворачиваться вместе с домом.
    pub(crate) axis_deg: f32,
    /// Сдвиг посева всего набора: у каждого дома посев свой, ручка крутит их
    /// все разом. В игре посев берётся из первой вершины контура, и смыслов у
    /// него два: фаза — чтобы швы соседних домов не выстроились в одну линию, и
    /// **возраст** кровли (`roof.wgsl::roof_age`), от которого зависит, сколько
    /// на битуме заплат. Поэтому дома в блоке заношены по-разному, а ручка —
    /// единственный способ посмотреть здесь на другие возрасты.
    pub(crate) seed: f32,
    /// Доля стороны дома под двор; 0 — без двора. Двор здесь не украшение: у
    /// дома с дыркой игра рисует ещё и стену по дальней стороне двора, и
    /// только на нём видно, что она встаёт, — а заодно что дом с двором
    /// скатной крыши не получает вовсе (`roofs::is_pitched`).
    pub(crate) courtyard: f32,
    /// Азимут солнца, градусы — игровой `SunStyle::azimuth`. Свет тут не
    /// декорация витрины: по нему освещены рёбра фальца и профлиста в шейдере
    /// (`RoofParams::light`), по нему же тянутся тени коробок на кровле.
    pub(crate) sun_azimuth: f32,
    /// Высота солнца, градусы — игровой `SunStyle::elevation`. Единственная
    /// ручка витрины, которая меняет **длину**: на 15° тень вентшахты втрое
    /// длиннее самой шахты, и видно, обрезана ли она краем кровли.
    pub(crate) sun_elevation: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            texture: ROOF_TEXTURE_DEFAULT,
            height: DEFAULT_HEIGHT,
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
///
/// Китовым стал и сам тип: [`KnobSpec`] держит ту же тройку полей, а имя здесь
/// локальное, чтобы таблица `specs()` ниже читалась как раньше.
pub(crate) type ParamSpec = KnobSpec<Tuning>;

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
            label: "Height",
            group: Some("Дом"),
            binding: SliderBinding {
                get: |t| t.height,
                set: |t, v| t.height = v,
                range: (3.0, 24.0, 1.0),
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
