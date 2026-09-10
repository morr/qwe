//! Ручки витрины — одной таблицей, как у витрины кровель.
//!
//! Крутить тут почти нечего, и это правда про сам предмет: у припаркованного
//! ряда ровно одна игровая ручка — занятость мест (`CarStyle::occupancy`).
//! Вторая, поворот сцены, игровой не является: она проверяет, что ряд держится
//! за улицу, а не за оси мира, — на повёрнутой клетке видно и смещение от
//! кромки, и направление кузовов.

use qwe::map::cars::CarDetail;
use qwe::settings::{
    CAR_OCCUPANCY_DEFAULT, CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_STEP,
};

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
        match self.detail as u8 {
            0 => CarDetail::Full,
            1 => CarDetail::Silhouette,
            _ => CarDetail::Block,
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

fn detail_name(value: f32) -> String {
    match value as u8 {
        0 => "Full".to_string(),
        1 => "Silhouette".to_string(),
        _ => "Block".to_string(),
    }
}

fn degrees(value: f32) -> String {
    format!("{value:.0}°")
}

/// Все ручки по порядку сверху вниз.
pub(crate) fn specs() -> Vec<ParamSpec> {
    vec![
        ParamSpec {
            label: "Occupancy",
            range: (CAR_OCCUPANCY_MIN, CAR_OCCUPANCY_MAX, CAR_OCCUPANCY_STEP),
            get: |t| t.occupancy,
            set: |t, v| t.occupancy = v,
            format: percent,
            group: Some("Ряд"),
        },
        ParamSpec {
            label: "Detail",
            range: (0.0, 2.0, 1.0),
            get: |t| t.detail,
            set: |t, v| t.detail = v,
            format: detail_name,
            group: None,
        },
        ParamSpec {
            label: "Rotation",
            range: (0.0, 90.0, 5.0),
            get: |t| t.rotation_deg,
            set: |t, v| t.rotation_deg = v,
            format: degrees,
            group: Some("Сцена"),
        },
    ]
}
