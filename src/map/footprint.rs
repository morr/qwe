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
use super::osm::model::{RoadLine, WallLine, WaterLine, distance_to_segment, ring_bounds};
use crate::settings::PASSAGE_MAX_WIDTH;

/// Насколько близко точка одной ломаной должна лежать к другой ломаной,
/// чтобы дороги считались примыкающими. Развязка в OSM — это общий узел,
/// то есть буквально одна и та же точка в обеих ways; допуск покрывает лишь
/// потерю точности проекции.
const JOIN_EPSILON: f32 = 0.5;

/// Минимальное расстояние от точки до ломаной — по всем её сегментам.
pub fn distance_to_polyline(point: Vec2, points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| distance_to_segment(point, segment[0], segment[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Примыкают ли две ломаные — у какой-нибудь точки одной есть сосед на другой
/// ближе [`JOIN_EPSILON`].
///
/// Тест симметричен намеренно: общий узел может оказаться серединой одной из
/// ways, и односторонняя проверка его пропустит.
///
/// Живёт у футпринта, а не в каждой заливке: один и тот же вопрос задают и
/// заливка сетки, и постройка полигонального меша, и ответы обязаны совпадать
/// — разойдясь, они открыли бы бордюр моста в одном бэкенде и не открыли в
/// другом. Потребители берут его через [`CurbCoverage`].
pub fn ways_joined(first: &[Vec2], second: &[Vec2]) -> bool {
    first
        .iter()
        .any(|&point| distance_to_polyline(point, second) < JOIN_EPSILON)
        || second
            .iter()
            .any(|&point| distance_to_polyline(point, first) < JOIN_EPSILON)
}

/// Могут ли ломаные с такими AABB примыкать — коробки пересекаются с запасом
/// [`JOIN_EPSILON`].
///
/// Префильтр к [`ways_joined`], а не замена ему: `true` не утверждает ничего,
/// `false` — утверждает. Отрезок лежит внутри своей коробки, значит расстояние
/// до отрезка не меньше расстояния до коробки; разошлись коробки больше чем на
/// эпсилон — разошлись и точки. Запас обязателен: примыкание в OSM — общий
/// узел, но проекция теряет точность, и стык в 0.4 м от торца моста коробок уже
/// не пересекает.
fn boxes_may_join(first: (Vec2, Vec2), second: (Vec2, Vec2)) -> bool {
    (first.0 - JOIN_EPSILON).cmple(second.1).all() && (second.0 - JOIN_EPSILON).cmple(first.1).all()
}

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

/// Полоса: осевая + ширина. Осевая, а не готовый контур, намеренно —
/// сетке нужна именно осевая, чтобы растеризация осталась 4-связной цепочкой
/// (тонкая косая полоса из одного контура рассыпалась бы в шахматку).
/// Что полоса значит на земле — блокирует или прорезает — решает потребитель
/// порядком заливки, самой полосе роль не нужна.
pub struct Band {
    pub line: Vec<Vec2>,
    pub width: f32,
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
        })
    }

    /// Прорезь арки: осевая прохода с шириной, капнутой
    /// [`PASSAGE_MAX_WIDTH`] — way обычно `service` (5 м), а сама арка у́же,
    /// и некапнутый коридор съедал бы фасад с обеих сторон.
    pub fn passage_band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width.min(PASSAGE_MAX_WIDTH),
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
        })
    }
}

impl WallLine {
    pub fn band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width,
        }
    }
}

/// Входы решения «какая часть бордюра составного моста открыта»: мосты и
/// примыкающие к ним не-мосты, отобранные одним предикатом ([`ways_joined`])
/// для обеих заливок.
///
/// Общие здесь именно **входы**. Само решение у заливок осознанно разное, и
/// это не долг, а два ответа на один случай-ловушку (номинальная ширина
/// primary 16 м заглатывает свой параллельный тротуар целиком): сетка решает
/// **направленным щупом** «есть ли лента снаружи от меня» (см.
/// `navmesh::fill_from_mapdata`), меш — **полигональной разностью**, у которой
/// от заглатывания выживают тонкие внешние обрезки-барьеры (см.
/// `polymesh/build.rs`). Точечный тест покрытия, общий для обеих, не
/// воспроизводит ни то, ни другое: с допуском он открывает внешний барьер, без
/// допуска — крошит бордюрную цепочку в пунктир. Проверено анализом при
/// попытке унификации; менять любую из стратегий — только через пин-тесты
/// бордюров (`navmesh/tests.rs`) и паритетные (`navigation/parity_tests.rs`).
pub struct CurbCoverage<'a> {
    bridges: Vec<&'a RoadLine>,
    joining: Vec<&'a RoadLine>,
}

impl<'a> CurbCoverage<'a> {
    pub fn build(roads: &'a [RoadLine]) -> Self {
        let bridges: Vec<&RoadLine> = roads.iter().filter(|road| road.bridge).collect();
        // AABB-прекомпьют по мостам — тем же приёмом, что у
        // `parse::drop_buildings_in_water`. Дорог на карте десятки тысяч
        // (Лондон 43 000), мостов сотни, а примыкает ~1%, и короткое замыкание
        // `.any()` срабатывает ровно у этого процента: остальные честно мерили
        // расстояние до каждого моста, точка за отрезком. Лондон — 1.00 с на
        // вызов против 19 мс, и вызова два, оба на загрузке (заливка сетки и
        // постройка полигонального меша)
        let bridge_boxes: Vec<(Vec2, Vec2)> = bridges
            .iter()
            .map(|bridge| ring_bounds(&bridge.points))
            .collect();
        let joining = roads
            .iter()
            .filter(|road| !road.bridge)
            .filter(|road| {
                let road_bounds = ring_bounds(&road.points);
                bridges
                    .iter()
                    .zip(&bridge_boxes)
                    .any(|(bridge, &bridge_bounds)| {
                        boxes_may_join(road_bounds, bridge_bounds)
                            && ways_joined(&road.points, &bridge.points)
                    })
            })
            .collect();
        Self { bridges, joining }
    }

    /// Мосты в порядке обхода `roads` — обе заливки нумеруют владельцев
    /// бордюров этим же порядком.
    pub fn bridges(&self) -> &[&'a RoadLine] {
        &self.bridges
    }

    /// Не-мосты, примыкающие хотя бы к одному мосту (общий узел, не близость
    /// в плане): их полотно открывает бордюр, который накрывает.
    pub fn joining(&self) -> &[&'a RoadLine] {
        &self.joining
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    /// Общий узел посреди одной из ways ловится с любой стороны — ровно ради
    /// этого случая предикат симметричен.
    #[test]
    fn a_way_ending_in_the_middle_of_another_still_joins_it() {
        let through = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let stub = [Vec2::new(50.0, 0.0), Vec2::new(50.0, 40.0)];
        assert!(ways_joined(&through, &stub));
        assert!(ways_joined(&stub, &through));
    }

    /// Тропа, прошедшая под пролётом, узла не делит: примыкание — это общая
    /// точка, а не близость в плане.
    #[test]
    fn a_way_passing_by_does_not_join() {
        let bridge = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let under = [Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)];
        assert!(!ways_joined(&bridge, &under));
    }

    /// Отбор примыкающих: общий узел — да; проход ПОД пролётом — нет, хотя
    /// коробки у него и у моста совпадают (решает по-прежнему `ways_joined`,
    /// а не префильтр); дальняя дорога — нет.
    #[test]
    fn coverage_takes_only_ways_sharing_a_node_with_a_bridge() {
        let roads = vec![
            fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            fixture::street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 60.0)], 8.0),
            fixture::street(vec![Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)], 3.5),
            fixture::street(vec![Vec2::new(500.0, 500.0), Vec2::new(600.0, 500.0)], 8.0),
        ];
        let coverage = CurbCoverage::build(&roads);
        assert_eq!(coverage.bridges().len(), 1);
        assert_eq!(coverage.joining().len(), 1);
        assert_eq!(coverage.joining()[0].points[0], Vec2::new(100.0, 0.0));
    }

    /// Отбор идёт с запасом [`JOIN_EPSILON`] на обеих сторонах: дорога,
    /// начинающаяся в 0.4 м от торца моста, примыкает — хотя её коробка
    /// коробку моста не пересекает, и любой отбор по голому AABB её потеряет.
    #[test]
    fn a_way_ending_just_short_of_a_bridge_still_joins_it() {
        let roads = vec![
            fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            fixture::street(vec![Vec2::new(100.4, 0.0), Vec2::new(200.0, 0.0)], 8.0),
        ];
        assert_eq!(CurbCoverage::build(&roads).joining().len(), 1);
    }

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
