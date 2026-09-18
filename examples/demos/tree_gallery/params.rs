//! Что вообще решает, как выглядит крона, — одной таблицей.
//!
//! Каждая строка панели описана здесь как [`ParamSpec`]: подпись, диапазон,
//! чтение и запись поля. Таблица, а не по функции на ручку, потому что за
//! протяжкой всех шестнадцати стоит один и тот же наблюдатель кита
//! (`qwe::ui::knob`), заведённый по разу на ресурс.

use qwe::map::trees::CrownParams;
use qwe::ui::knob::{KnobSpec, SliderBinding};

/// Настройка витрины: ручки геометрии кроны плюс те немногие ручки вида,
/// которые живут не в них. `Default` — игра по геометрии, с единственным
/// намеренным отступлением по цвету: см. `variance` ниже.
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
            // зелень у всех клеток честнее показывает форму — на ней глазами
            // и выбирался набор 5. Это единственное место, где дефолт витрины
            // расходится с игрой, и ровно поэтому кнопка зовётся «Сброс», а не
            // «Сброс к игре»; игровые 0.35 — одна протяжка ползунка «Variance»
            variance: 0.0,
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
            group: Some("Форма"),
            binding: SliderBinding {
                get: |t| t.crown.points,
                set: |t, v| t.crown.points = v,
                range: (0.25, 2.0, 0.25),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Radius jitter",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.radius_jitter,
                set: |t, v| t.crown.radius_jitter = v,
                range: (0.0, 2.0, 0.1),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Lobe",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.lobe,
                set: |t, v| t.crown.lobe = v,
                range: (0.3, 2.5, 0.1),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Seed",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.seed as f32,
                set: |t, v| t.crown.seed = v as u32,
                range: (0.0, 15.0, 1.0),
                text: whole,
            },
        },
        ParamSpec {
            label: "Band lift",
            group: Some("Кольца и штрихи"),
            binding: SliderBinding {
                get: |t| t.crown.band_lift,
                set: |t, v| t.crown.band_lift = v,
                range: (0.0, 3.0, 0.1),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Band scale",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.band_scale,
                set: |t, v| t.crown.band_scale = v,
                range: (0.4, 1.6, 0.05),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Shade weight",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.shade_weight,
                set: |t, v| t.crown.shade_weight = v,
                range: (0.0, 2.0, 0.1),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Outline",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.outline_stroke,
                set: |t, v| t.crown.outline_stroke = v,
                range: (0.02, 0.30, 0.01),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Detail",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.detail_stroke,
                set: |t, v| t.crown.detail_stroke = v,
                range: (0.01, 0.20, 0.01),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Spike floor",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.spike_floor,
                set: |t, v| t.crown.spike_floor = v,
                range: (0.0, 3.0, 0.1),
                text: multiplier,
            },
        },
        ParamSpec {
            label: "Stretch",
            group: Some("Тень"),
            binding: SliderBinding {
                get: |t| t.crown.shadow_stretch,
                set: |t, v| t.crown.shadow_stretch = v,
                range: (1.0, 3.0, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Backshift",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.shadow_backshift,
                set: |t, v| t.crown.shadow_backshift = v,
                range: (-1.5, 0.5, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Height base",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.shadow_height_base,
                set: |t, v| t.crown.shadow_height_base = v,
                range: (0.0, 1.5, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Height spread",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.shadow_height_spread,
                set: |t, v| t.crown.shadow_height_spread = v,
                range: (0.0, 2.0, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Long at",
            group: None,
            binding: SliderBinding {
                get: |t| t.crown.long_shadow_height,
                set: |t, v| t.crown.long_shadow_height = v,
                range: (0.0, 1.5, 0.05),
                text: fraction,
            },
        },
        ParamSpec {
            label: "Variance",
            group: Some("Цвет"),
            binding: SliderBinding {
                get: |t| t.variance,
                set: |t, v| t.variance = v,
                range: (0.0, 1.0, 0.05),
                text: fraction,
            },
        },
    ]
}
