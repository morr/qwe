//! Что вообще решает, как выглядит кровля, — одной таблицей.
//!
//! Каждая строка панели описана здесь как [`ParamSpec`]: подпись, диапазон,
//! чтение и запись поля. Таблица, а не по функции на ручку, потому что
//! обработчик протяжки у всех один и тот же — «округли до шага, положи в поле,
//! перепиши число», — и четыре его копии отличались бы только именем поля.
//!
//! Материал и цвет ручками не крутятся: они и есть сетка витрины, все шесть
//! материалов и все их палитры разложены по блокам. Ручки — то, что в игре
//! приходит от **дома**: как он повёрнут, какой у него посев фазы и есть ли
//! двор, — плюс единственный игровой ползунок кровель, `RoofStyle::texture`.

use qwe::settings::ROOF_TEXTURE_DEFAULT;

/// Настройка витрины. `Default` — игра: сила фактуры игровая, дома стоят по
/// сторонам света, дворов нет.
#[derive(bevy::prelude::Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tuning {
    /// `RoofStyle::texture` — общий множитель амплитуд фактуры. Не геометрия:
    /// уходит юниформом в материал, меши от него не пересобираются (витрина
    /// всё равно пересобирает — ей дешевле, чем городу).
    pub(crate) texture: f32,
    /// Угол длинной оси домов, градусы. В игре ось даёт `min_area_rect`
    /// контура; здесь дома прямоугольные, и угол задаётся напрямую — по нему
    /// же повёрнута и рамка кровли, так что швы ковра и рёбра фальца обязаны
    /// поворачиваться вместе с домом.
    pub(crate) axis_deg: f32,
    /// Сдвиг посева всего набора: у каждого дома фаза своя, ручка крутит их
    /// все разом. В игре посев берётся из первой вершины контура, и его смысл
    /// ровно этот — чтобы швы соседних домов не выстроились в одну линию.
    pub(crate) seed: f32,
    /// Доля стороны дома под двор; 0 — без двора. Двор здесь не украшение:
    /// парапет идёт и по его кольцу, и только на дырке видно, что кайма
    /// смотрит внутрь двора.
    pub(crate) courtyard: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            texture: ROOF_TEXTURE_DEFAULT,
            axis_deg: 0.0,
            seed: 0.0,
            courtyard: 0.0,
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
            label: "Axis",
            range: (0.0, 90.0, 5.0),
            get: |t| t.axis_deg,
            set: |t, v| t.axis_deg = v,
            format: degrees,
            group: Some("Дом"),
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
    ]
}
