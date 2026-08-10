//! Футпринт линейной геометрии карты — полосы, которые дороги, водотоки и
//! стены занимают на земле, и политика их ширин (кант, бордюр).
//!
//! Одна конструкция на всех потребителей, каждый берёт полосу своим способом:
//! заливка сетки растеризует осевую (`set_polyline` — гарантия 4-связной
//! цепочки), постройка меша строит контур (`ribbon_outline`), рендер кладёт
//! ленту (`push_ribbon` — по **сглаженной копии** осевой, поэтому геометрию
//! полос он не берёт, только ширины). До этого модуля каждая сторона выводила
//! полосы из `points + width` заново, и «одно правило для двух заполнений»
//! держалось на дисциплине; расходились — тайл за тайлом (см. паритетные
//! тесты `navigation/parity_tests.rs`).
//!
//! Ширины живут здесь, а не в рендере: бордюр не только рисуется — он
//! блокирует проходимость, и нарисованная полоса обязана совпадать с
//! заблокированной по построению.

use std::ops::RangeInclusive;

use bevy::prelude::*;

use super::meshing::miter_offsets;
use super::osm::model::{RoadLine, WallLine, WaterLine};
use crate::settings::PASSAGE_MAX_WIDTH;

/// Кант — 8% ширины дороги в разумных пределах.
const CASING_SCALE: f32 = 0.08;
const CASING_RANGE: RangeInclusive<f32> = 0.3..=1.0;

/// Бордюр моста — толще и темнее канта (12%), чтобы никогда с ним не
/// сливаться.
const BRIDGE_CURB_SCALE: f32 = 0.12;
const BRIDGE_CURB_RANGE: RangeInclusive<f32> = 0.8..=2.0;

/// Толщина канта для ленты такой ширины. Общая с подложкой аллей
/// (`map::spawn`) и клиренсом посадки деревьев (`planting/index.rs`), чтобы
/// кант везде на карте был одной толщины.
pub fn casing_width(width: f32) -> f32 {
    (width * CASING_SCALE).clamp(*CASING_RANGE.start(), *CASING_RANGE.end())
}

/// Толщина бордюра моста для дороги такой ширины.
pub fn bridge_curb_width(width: f32) -> f32 {
    (width * BRIDGE_CURB_SCALE).clamp(*BRIDGE_CURB_RANGE.start(), *BRIDGE_CURB_RANGE.end())
}

/// Чем полоса является на земле.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BandRole {
    /// Проезжая часть моста — прорезается проходимой.
    Deck,
    /// Бордюр моста — блокирует сход с настила вбок.
    Curb,
    /// Русло линейного водотока — блокирует (трубы полосы не имеют).
    Channel,
    /// Городская стена — блокирует.
    Wall,
    /// Арка сквозь здание — прорезается проходимой, ширина капнута.
    Passage,
}

/// Полоса: осевая + ширина + роль. Осевая, а не готовый контур, намеренно —
/// сетке нужна именно осевая, чтобы растеризация осталась 4-связной цепочкой
/// (тонкая косая полоса из одного контура рассыпалась бы в шахматку).
pub struct Band {
    pub line: Vec<Vec2>,
    pub width: f32,
    pub role: BandRole,
}

impl RoadLine {
    /// Толщина бордюра этого моста.
    pub fn curb_width(&self) -> f32 {
        bridge_curb_width(self.width)
    }

    /// Полуширина всей мостовой полосы: настил + бордюр. Ею меряют «накрыт ли
    /// сосед лентой моста» и щуп сетки, и разность меша; рендер рисует
    /// бордюрную подложку шириной `2 × curb_reach`.
    pub fn curb_reach(&self) -> f32 {
        self.width / 2.0 + self.curb_width()
    }

    /// Настил моста — ровно проезжая часть, как её рисует рендер. Сеточная
    /// поправка на блуждание тайловых центров (`− tile·√2`) сюда не входит:
    /// она — свойство растеризации, не футпринта.
    pub fn deck_band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width,
            role: BandRole::Deck,
        }
    }

    /// Две бордюрные полосы моста: осевая каждой — кромка настила плюс
    /// полбордюра (`miter_offsets`, общие с рендером), ширина — бордюр.
    /// Осмысленно только для `bridge`-дорог; обе заливки фильтруют по флагу
    /// до вызова.
    pub fn curb_bands(&self) -> [Band; 2] {
        let curb = self.curb_width();
        let offsets = miter_offsets(&self.points, false, (self.width + curb) / 2.0);
        [-1.0f32, 1.0].map(|side| Band {
            line: self
                .points
                .iter()
                .zip(&offsets)
                .map(|(&point, &offset)| point + side * offset)
                .collect(),
            width: curb,
            role: BandRole::Curb,
        })
    }

    /// Прорезь арки: осевая прохода с шириной, капнутой
    /// [`PASSAGE_MAX_WIDTH`] — way обычно `service` (5 м), а сама арка у́же,
    /// и некапнутый коридор съедал бы фасад с обеих сторон.
    pub fn passage_band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width.min(PASSAGE_MAX_WIDTH),
            role: BandRole::Passage,
        }
    }
}

impl WaterLine {
    /// Русло как полоса; `None` — труба: не рисуется и не блокирует, над
    /// кульвертом земля. Капы торцов (плоский срез у портала трубы —
    /// `water_line_caps`) остаются у потребителей: это правило пары линий, а
    /// не одной.
    pub fn channel_band(&self) -> Option<Band> {
        (!self.tunnel).then(|| Band {
            line: self.points.clone(),
            width: self.width,
            role: BandRole::Channel,
        })
    }
}

impl WallLine {
    pub fn band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width,
            role: BandRole::Wall,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casing_stays_within_its_range_on_every_road_class() {
        for width in [3.5_f32, 5.0, 8.0, 16.0] {
            assert!(casing_width(width) >= *CASING_RANGE.start());
            assert!(casing_width(width) <= *CASING_RANGE.end());
        }
    }

    /// Бордюр обязан торчать из-под канта на любом классе — иначе при
    /// включённом канте мост неотличим от окантованной дороги.
    #[test]
    fn bridge_curb_is_thicker_than_a_casing() {
        for width in [3.5_f32, 5.0, 8.0, 16.0] {
            assert!(bridge_curb_width(width) > casing_width(width));
            assert!(bridge_curb_width(width) >= *BRIDGE_CURB_RANGE.start());
            assert!(bridge_curb_width(width) <= *BRIDGE_CURB_RANGE.end());
        }
    }

    /// Кромки бордюрных полос отстоят от осевой ровно на полширины настила
    /// плюс полбордюра — та самая конструкция, которой пользуются обе заливки
    /// и рендер.
    #[test]
    fn curb_bands_sit_on_both_deck_edges() {
        let road = RoadLine {
            points: vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)],
            width: 8.0,
            class: super::super::osm::model::RoadClass::Street,
            bridge: true,
            passage: false,
        };
        let [left, right] = road.curb_bands();
        let offset = (road.width + road.curb_width()) / 2.0;
        assert_eq!(left.width, road.curb_width());
        assert_eq!(left.line[0].y, -offset);
        assert_eq!(right.line[0].y, offset);
        assert_eq!(
            road.curb_reach(),
            road.width / 2.0 + road.curb_width(),
            "щуп сетки и разность меша меряют одну и ту же полуширину"
        );
    }
}
