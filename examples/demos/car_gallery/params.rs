//! Ручки витрины — одной таблицей, как у витрины кровель.
//!
//! Крутить тут почти нечего, и это правда про сам предмет: у припаркованного
//! ряда ровно одна игровая ручка — занятость мест (`CarStyle::occupancy`).
//! Вторая, поворот сцены, игровой не является: она проверяет, что ряд держится
//! за улицу, а не за оси мира, — на повёрнутой клетке видно и смещение от
//! кромки, и направление кузовов.

use qwe::map::cars::{self, CarDetail};
use qwe::settings::{
    CAR_OCCUPANCY_DEFAULT, CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_STEP,
};
use qwe::ui::knob::SliderBinding;

/// Настройка витрины. `Default` — игра: занятость игровая, сцена не повёрнута.
#[derive(bevy::prelude::Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tuning {
    /// `CarStyle::occupancy` — доля занятых мест. Ноль оставляет улицы пустыми,
    /// единица выстраивает сплошной ряд: только на ней видно **все** места,
    /// которые расстановка вообще нашла, и потому все дыры, которые оставили
    /// перекрёстки и излом.
    pub(crate) occupancy: f32,
    /// Ступень подробности кузова (`CarDetail`), которую в игре выбирает зум:
    /// 0 — стёкла и зеркала, 1 — силуэт, 2 — габаритный прямоугольник. Ручкой,
    /// а не зумом, потому что смотреть на них надо рядом и вблизи, а в игре
    /// две дальние ступени видны только издали.
    pub(crate) detail: f32,
    /// Поворот всей сцены, градусы.
    pub(crate) rotation_deg: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            occupancy: CAR_OCCUPANCY_DEFAULT,
            detail: 0.0,
            rotation_deg: 0.0,
        }
    }
}

impl Tuning {
    /// Ступень подробности, которой строятся ряды клеток.
    pub(crate) fn car_detail(&self) -> CarDetail {
        detail_at(self.detail)
    }
}

/// Ступень подробности по значению ручки: 0 — Full, 1 — Silhouette, дальше
/// Block. Общая для [`Tuning::car_detail`] и [`detail_name`], чтобы ступень
/// ручки и её подпись не могли разойтись.
///
/// Таблица берётся игровая (`cars::detail_for`) — ручка витрины обязана
/// показывать ту же лестницу, что выбирает зум, а своя копия разошлась бы с
/// ней на первой же новой ступени. Последняя ступень зума («слоя нет») ручке
/// не нужна, и `Block` тут — то же, что давала ветка `_`.
fn detail_at(value: f32) -> CarDetail {
    cars::detail_for(value as usize).unwrap_or(CarDetail::Block)
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

fn detail_name(value: f32) -> String {
    format!("{:?}", detail_at(value))
}

fn degrees(value: f32) -> String {
    format!("{value:.0}°")
}

/// Все ручки по порядку сверху вниз.
pub(crate) fn specs() -> Vec<ParamSpec> {
    vec![
        ParamSpec {
            label: "Occupancy",
            group: Some("Ряд"),
            binding: SliderBinding {
                get: |t| t.occupancy,
                set: |t, v| t.occupancy = v,
                range: (CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_STEP),
                text: percent,
            },
        },
        ParamSpec {
            label: "Detail",
            group: None,
            binding: SliderBinding {
                get: |t| t.detail,
                set: |t, v| t.detail = v,
                range: (0.0, 2.0, 1.0),
                text: detail_name,
            },
        },
        ParamSpec {
            label: "Rotation",
            group: Some("Сцена"),
            binding: SliderBinding {
                get: |t| t.rotation_deg,
                set: |t, v| t.rotation_deg = v,
                range: (0.0, 90.0, 5.0),
                text: degrees,
            },
        },
    ]
}
