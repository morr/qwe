//! Радиусы бордюра на перекрёстках: скругление угла между двумя дорогами,
//! сходящимися в общем узле.
//!
//! Ленты дорог лежат внахлёст, и их края встречаются в перекрёстке прямым
//! (или острым) углом — так рисует osm-carto, но так не бывает на месте:
//! бордюр на повороте всегда скруглён, иначе машина не повернёт, не заехав на
//! тротуар. На снимке сверху именно эти дуги и делают перекрёсток
//! перекрёстком, а не крестом из двух полос.
//!
//! Скругление — «вогнутый треугольник» между краями двух соседних по углу
//! лучей узла и дугой, касательной к обоим краям. Кладётся **той же заливкой**
//! в тот же слой, что и сами дороги, и раньше них: любая лента поверх
//! скругления его кроет, так что разметка и чужие ленты остаются целы.
//! Скругляются только лучи одного класса — асфальтовая дуга между улицей и
//! пешеходной дорожкой легла бы серым поверх песочного.

use std::collections::HashMap;
use std::f32::consts::PI;

use bevy::prelude::*;

use super::junctions::node_key;
use super::network::RoadNodes;
use crate::map::meshing::arc_steps;
use crate::map::osm::{RoadClass, RoadLine};

/// Радиус бордюра — доля суммы полуширин двух дорог и его пределы, м: две
/// жилые улицы по 8 м — 4.8 м, магистраль 16 м с жилой — 7.2 м, два проезда по
/// 5 м — 3 м, пешеходные дорожки — около двух.
const KERB_RADIUS_SHARE: f32 = 0.6;
const KERB_RADIUS_RANGE: std::ops::RangeInclusive<f32> = 1.5..=9.0;
/// Скругление меньше этого не кладётся, м: его всё равно не видно.
const MIN_RADIUS: f32 = 0.5;
/// Угол между лучами, в котором скругление имеет смысл. Острее — дуга
/// вытягивается в длинный клин; почти развёрнутый угол — это продолжение
/// дороги, а не поворот.
const MIN_ANGLE: f32 = 25.0 * PI / 180.0;
const MAX_ANGLE: f32 = 155.0 * PI / 180.0;
/// Во сколько ширин тротуара умещается радиус, пока скругление целиком лежит
/// на тротуарах обеих улиц: дуга радиуса `r` отходит от угла на
/// `r·(1 − 1/√2)`, и за тротуарами обеих улиц она показалась бы при
/// `r > s·√2/(√2 − 1) ≈ 3.41·s`. Иначе серый клин лёг бы на газон за углом.
const SIDEWALK_COVER: f32 = 3.4;
/// Луч меряет направление по звену не короче этого, м.
const MIN_ARM: f32 = 0.5;

/// Одна дорога, выходящая из узла.
struct Arm {
    class: RoadClass,
    half: f32,
    sidewalk: Option<f32>,
    direction: Vec2,
    /// Сколько метров край ленты идёт прямо — до следующей вершины.
    run: f32,
}

/// Скругления всех перекрёстков: контур и класс дорог, в чей слой он ляжет.
///
/// `paths` — **нарисованные** осевые (после сглаживания, до стежков), по
/// индексу дороги; `None` — дорога не участвует (мост, арка). `sidewalk` —
/// ширина тротуара дороги, если он рисуется.
pub fn kerb_returns(
    roads: &[&RoadLine],
    paths: &[Option<&[Vec2]>],
    nodes: &RoadNodes,
    sidewalk: impl Fn(&RoadLine) -> Option<f32>,
) -> Vec<(RoadClass, Vec<Vec2>)> {
    let mut arms: HashMap<(i32, i32), (Vec2, Vec<Arm>)> = HashMap::new();
    for (&road, path) in roads.iter().zip(paths) {
        let Some(path) = *path else {
            continue;
        };
        if path.len() < 2 {
            continue;
        }
        let closed = path[0] == path[path.len() - 1];
        let last = path.len() - 1;
        for (vertex, &node) in path.iter().enumerate() {
            if closed && vertex == last {
                continue;
            }
            if !nodes.is_shared(node) {
                continue;
            }
            let entry = arms
                .entry(node_key(node))
                .or_insert_with(|| (node, Vec::new()));
            // вперёд по пути и назад; у замкнутого кольца — через шов (его
            // последняя вершина повторяет первую и пропущена выше)
            let ring = last;
            let step = |from: usize, forward: bool| {
                if closed {
                    Some(if forward {
                        (from + 1) % ring
                    } else {
                        (from + ring - 1) % ring
                    })
                } else if forward {
                    (from < last).then_some(from + 1)
                } else {
                    from.checked_sub(1)
                }
            };
            for forward in [true, false] {
                let mut at = vertex;
                let mut next = None;
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    if path[index].distance(node) >= MIN_ARM {
                        next = Some(path[index]);
                        break;
                    }
                    at = index;
                }
                let Some(next) = next else {
                    continue;
                };
                entry.1.push(Arm {
                    class: road.class,
                    half: road.width / 2.0,
                    sidewalk: sidewalk(road),
                    direction: (next - node).normalize(),
                    run: next.distance(node),
                });
            }
        }
    }

    let mut returns = Vec::new();
    for (node, mut found) in arms.into_values() {
        if found.len() < 2 {
            continue;
        }
        found.sort_by(|a, b| a.direction.to_angle().total_cmp(&b.direction.to_angle()));
        for class in [RoadClass::Street, RoadClass::Alley] {
            let group: Vec<&Arm> = found.iter().filter(|arm| arm.class == class).collect();
            if group.len() < 2 {
                continue;
            }
            for (index, first) in group.iter().enumerate() {
                let second = group[(index + 1) % group.len()];
                if let Some(outline) = kerb_return(node, first, second) {
                    returns.push((class, outline));
                }
            }
        }
    }
    returns
}

/// Скругление угла от луча `first` против часовой стрелки до луча `second`.
fn kerb_return(node: Vec2, first: &Arm, second: &Arm) -> Option<Vec<Vec2>> {
    let mut angle = second.direction.to_angle() - first.direction.to_angle();
    if angle <= 0.0 {
        angle += 2.0 * PI;
    }
    if !(MIN_ANGLE..=MAX_ANGLE).contains(&angle) {
        return None;
    }
    let (along_first, along_second) = (first.direction, second.direction);
    // края, смотрящие друг на друга: у первого слева, у второго справа
    let (side_first, side_second) = (along_first.perp(), -along_second.perp());
    // угол, где встречаются края: first.half·n₁ + t·u₁ = second.half·n₂ + s·u₂
    let rhs = side_second * second.half - side_first * first.half;
    let determinant = -along_first.perp_dot(along_second);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-along_second) / determinant;
    let s = along_first.perp_dot(rhs) / determinant;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    let corner = node + side_first * first.half + along_first * t;

    let mut radius = (KERB_RADIUS_SHARE * (first.half + second.half))
        .clamp(*KERB_RADIUS_RANGE.start(), *KERB_RADIUS_RANGE.end());
    if let (Some(a), Some(b)) = (first.sidewalk, second.sidewalk) {
        radius = radius.min(SIDEWALK_COVER * a.min(b));
    }
    let half_angle = angle / 2.0;
    // касательная не длиннее прямого края ленты: за следующей вершиной край
    // уже повернул, и дуга легла бы мимо
    let tangent = (radius / half_angle.tan())
        .min(first.run - t)
        .min(second.run - s);
    if tangent <= 0.0 {
        return None;
    }
    radius = tangent * half_angle.tan();
    if radius < MIN_RADIUS {
        return None;
    }
    let on_first = corner + along_first * tangent;
    let on_second = corner + along_second * tangent;
    let centre = corner + (along_first + along_second).normalize() * (radius / half_angle.sin());

    let sweep = PI - angle;
    let from = on_first - centre;
    let turn = from.perp_dot(on_second - centre).signum();
    let steps = arc_steps(radius, sweep);
    let mut outline = Vec::with_capacity(steps + 2);
    outline.push(corner);
    outline.push(on_first);
    for step in 1..steps {
        let rotation = Vec2::from_angle(turn * sweep * step as f32 / steps as f32);
        outline.push(centre + rotation.rotate(from));
    }
    outline.push(on_second);
    Some(outline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;
    use crate::map::osm::model::point_in_polygon;

    fn returns_of(roads: &[RoadLine]) -> Vec<(RoadClass, Vec<Vec2>)> {
        let nodes = RoadNodes::new(roads);
        let paths: Vec<Option<&[Vec2]>> = roads
            .iter()
            .map(|road| Some(road.points.as_slice()))
            .collect();
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        kerb_returns(&drawn, &paths, &nodes, |_| None)
    }

    #[test]
    fn a_crossing_gets_four_rounded_corners_outside_both_ribbons() {
        let east_west = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        );
        let north_south = street(
            vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
            8.0,
        );
        let found = returns_of(&[east_west, north_south]);
        assert_eq!(found.len(), 4);
        for (_, outline) in &found {
            // угол — ровно на пересечении краёв
            assert!(
                (outline[0].abs() - Vec2::splat(4.0)).length() < 1e-3,
                "{:?}",
                outline[0]
            );
            // и сама дуга за краями обеих лент
            for point in &outline[1..] {
                assert!(point.x.abs() >= 4.0 - 1e-3 && point.y.abs() >= 4.0 - 1e-3);
            }
            // радиус 4.8: ближе всего к углу середина дуги, в r·(√2 − 1) от него
            let nearest = outline[1..]
                .iter()
                .map(|point| point.distance(outline[0]))
                .fold(f32::INFINITY, f32::min);
            let expected = 4.8 * (2.0_f32.sqrt() - 1.0);
            assert!((nearest - expected).abs() < 0.1, "{nearest}");
        }
    }

    #[test]
    fn a_t_junction_rounds_only_the_two_turning_corners() {
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        );
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = returns_of(&[through, side]);
        assert_eq!(found.len(), 2);
        // обе дуги со стороны примыкания
        for (_, outline) in &found {
            assert!(outline.iter().all(|point| point.y >= 4.0 - 1e-3));
        }
        let corner = &found[0].1;
        assert!(!point_in_polygon(Vec2::new(0.0, 10.0), corner));
    }

    #[test]
    fn a_street_and_a_footway_are_not_rounded_together() {
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        );
        let path = RoadLine {
            class: RoadClass::Alley,
            ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 3.5)
        };
        assert!(returns_of(&[through, path]).is_empty());
    }

    #[test]
    fn a_short_arm_limits_the_radius() {
        // вторая вершина поперечной улицы в двух метрах за краем — дуга не
        // длиннее этого
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        );
        let side = street(
            vec![Vec2::new(30.0, 30.0), Vec2::new(0.0, 6.0), Vec2::ZERO],
            8.0,
        );
        for (_, outline) in returns_of(&[through, side]) {
            for point in &outline {
                assert!(point.y <= 6.0 + 1e-3, "{point:?}");
            }
        }
    }
}
