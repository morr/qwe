//! Что вообще решает, как выглядит крона, — одной таблицей.
//!
//! Каждая строка панели описана здесь как [`ParamSpec`]: подпись, диапазон,
//! чтение и запись поля. Таблица, а не по функции на ручку, потому что
//! обработчик протяжки у всех один и тот же — «округли до шага, положи в поле,
//! перепиши число» — и шестнадцать его копий отличались бы только именем поля.

use qwe::map::trees::CrownParams;

/// Настройка витрины: ручки геометрии кроны плюс те немногие ручки вида,
/// которые живут не в них. `Default` — ровно игра.
#[derive(bevy::prelude::Resource, Clone, Debug)]
pub(crate) struct Tuning {
    pub(crate) crown: CrownParams,
    /// `TreeStyle::variance` — разброс яркости листвы. Не геометрия: материал
    /// дерева домножает вершинные цвета на квантованный множитель, поэтому
    /// ручка живёт в стиле, а не в [`CrownParams`].
    pub(crate) variance: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            crown: CrownParams::default(),
            // ноль, а не игровые 0.35: витрина про геометрию, и одинаковая
            // зелень у всех клеток честнее показывает форму. Ползунок рядом
            variance: 0.0,
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

/// Множитель к величине, своей у каждой формы: 1.00 — как в игре.
fn multiplier(value: f32) -> String {
    format!("×{value:.2}")
}

/// Доля радиуса кроны.
fn fraction(value: f32) -> String {
    format!("{value:.2}")
}

fn whole(value: f32) -> String {
    format!("{value:.0}")
}

/// Все ручки по порядку сверху вниз.
///
/// Разделение на **множители** и **абсолюты** не косметическое: вершин базы у
/// хвои 16, а у облака 12; джиттер радиуса 1/4 против 1/3; подъём колец
/// 0.15/0.12/0.1. Абсолютная ручка стёрла бы разницу форм и потребовала бы
/// по три ползунка на каждую величину, множитель же двигает все три формы
/// разом и сохраняет их пропорции. Абсолютны те величины, у которых своего
/// значения по форме нет: толщины линий и геометрия тени.
pub(crate) fn specs() -> Vec<ParamSpec> {
    vec![
        ParamSpec {
            label: "Points",
            range: (0.25, 2.0, 0.25),
            get: |t| t.crown.points,
            set: |t, v| t.crown.points = v,
            format: multiplier,
            group: Some("Форма"),
        },
        ParamSpec {
            label: "Radius jitter",
            range: (0.0, 2.0, 0.1),
            get: |t| t.crown.radius_jitter,
            set: |t, v| t.crown.radius_jitter = v,
            format: multiplier,
            group: None,
        },
        ParamSpec {
            label: "Lobe",
            range: (0.3, 2.5, 0.1),
            get: |t| t.crown.lobe,
            set: |t, v| t.crown.lobe = v,
            format: multiplier,
            group: None,
        },
        ParamSpec {
            label: "Seed",
            range: (0.0, 15.0, 1.0),
            get: |t| t.crown.seed as f32,
            set: |t, v| t.crown.seed = v as u32,
            format: whole,
            group: None,
        },
        ParamSpec {
            label: "Band lift",
            range: (0.0, 3.0, 0.1),
            get: |t| t.crown.band_lift,
            set: |t, v| t.crown.band_lift = v,
            format: multiplier,
            group: Some("Кольца и штрихи"),
        },
        ParamSpec {
            label: "Band scale",
            range: (0.4, 1.6, 0.05),
            get: |t| t.crown.band_scale,
            set: |t, v| t.crown.band_scale = v,
            format: multiplier,
            group: None,
        },
        ParamSpec {
            label: "Shade weight",
            range: (0.0, 2.0, 0.1),
            get: |t| t.crown.shade_weight,
            set: |t, v| t.crown.shade_weight = v,
            format: multiplier,
            group: None,
        },
        ParamSpec {
            label: "Outline",
            range: (0.02, 0.30, 0.01),
            get: |t| t.crown.outline_stroke,
            set: |t, v| t.crown.outline_stroke = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Detail",
            range: (0.01, 0.20, 0.01),
            get: |t| t.crown.detail_stroke,
            set: |t, v| t.crown.detail_stroke = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Spike floor",
            range: (0.0, 3.0, 0.1),
            get: |t| t.crown.spike_floor,
            set: |t, v| t.crown.spike_floor = v,
            format: multiplier,
            group: None,
        },
        ParamSpec {
            label: "Stretch",
            range: (1.0, 3.0, 0.05),
            get: |t| t.crown.shadow_stretch,
            set: |t, v| t.crown.shadow_stretch = v,
            format: fraction,
            group: Some("Тень"),
        },
        ParamSpec {
            label: "Backshift",
            range: (-1.5, 0.5, 0.05),
            get: |t| t.crown.shadow_backshift,
            set: |t, v| t.crown.shadow_backshift = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Height base",
            range: (0.0, 1.5, 0.05),
            get: |t| t.crown.shadow_height_base,
            set: |t, v| t.crown.shadow_height_base = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Height spread",
            range: (0.0, 2.0, 0.05),
            get: |t| t.crown.shadow_height_spread,
            set: |t, v| t.crown.shadow_height_spread = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Long at",
            range: (0.0, 1.5, 0.05),
            get: |t| t.crown.long_shadow_height,
            set: |t, v| t.crown.long_shadow_height = v,
            format: fraction,
            group: None,
        },
        ParamSpec {
            label: "Variance",
            range: (0.0, 1.0, 0.05),
            get: |t| t.variance,
            set: |t, v| t.variance = v,
            format: fraction,
            group: Some("Цвет"),
        },
    ]
}
