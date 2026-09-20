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
//!
//! **Тротуар поворачивает вместе с бордюром.** Полоса тротуара шире
//! проезжей части, и её собственный угол на перекрёстке оставался прямым:
//! асфальт выкатывался дугой в угол, срезая тротуар до нитки, а за ним торчал
//! прямоугольный уступ светлой полосы — на снимке это и видно. Тот же клин, но
//! в слое тротуаров, кладётся по краям полос (полуширина плюс ширина тротуара)
//! и дугой **того же центра**: радиус меньше ровно на ширину тротуара, и за
//! бордюром идёт постоянная полоса шириной в тротуар — как на месте. Радиус
//! меньше тротуара — угол и на месте прямой (въезд с малым радиусом), дуги
//! нет. Асфальтовый клин при этом всегда лежит на тротуарном: круг тротуарной
//! дуги вложен в круг бордюрной, пока радиус не больше [`SIDEWALK_COVER`]
//! ширин тротуара, а он там и ограничен.

use std::f32::consts::PI;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::node_key;
use super::network::RoadNodes;
use crate::map::meshing::arc_steps;
use crate::map::osm::{RoadClass, RoadLine};

/// Радиус бордюра между дорогами одной ширины — доля суммы полуширин и его
/// пределы, м: две жилые улицы по 8 м — 4.8 м, две магистрали — 9 м, два
/// проезда по 5 м — 3 м, пешеходные дорожки — около двух.
const KERB_RADIUS_SHARE: f32 = 0.6;
const KERB_RADIUS_RANGE: std::ops::RangeInclusive<f32> = 1.5..=9.0;
/// Дорога уже другой больше чем на столько по полуширине, м, — второстепенная,
/// входящая в большую: проезд 5 м в улицу 8 м, жилая 8 м в магистраль 16 м.
const MINOR_WIDTH_STEP: f32 = 0.5;
/// Радиус въезда второстепенной дороги — доля её полуширины: у проезда 5 м
/// это метр, у жилой улицы 8 м в магистраль — 1.6 м.
const MINOR_RADIUS_SHARE: f32 = 0.4;
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
///
/// На тульских улицах ограничение не срабатывает (у восьмиметровой улицы
/// тротуар 1.76 м, потолок 5.98 м против радиуса 4.8), но именно оно держит
/// вложенность кругов, на которой стоит тротуарное скругление, — см. доку
/// модуля.
const SIDEWALK_COVER: f32 = 3.4;
/// Луч меряет направление по звену не короче этого, м.
const MIN_ARM: f32 = 0.5;
/// Насколько вершина может отойти вбок от прямой луча и всё ещё продолжать
/// его прямой край, м.
const STRAIGHT_TOLERANCE: f32 = 0.15;
/// На сколько прямые стороны скругления заходят под ленты дорог, м.
const OVERLAP: f32 = 0.05;

/// Одна дорога, выходящая из узла.
struct Arm {
    class: RoadClass,
    half: f32,
    sidewalk: Option<f32>,
    direction: Vec2,
    /// Сколько метров край ленты идёт прямо — до следующей вершины.
    run: f32,
}

/// Скругления всех перекрёстков.
#[derive(Default)]
pub struct KerbReturns {
    /// Контур и класс дорог, в чей слой заливки он ляжет.
    pub roads: Vec<(RoadClass, Vec<Vec2>)>,
    /// Контуры в слое тротуаров.
    pub sidewalks: Vec<Vec<Vec2>>,
}

/// Скругления всех перекрёстков: дуги проезжей части и дуги тротуаров.
///
/// `paths` — **нарисованные** осевые (после сглаживания, до стежков), по
/// индексу дороги; `None` — дорога не участвует (мост, арка). `sidewalk` —
/// ширина тротуара дороги, если он рисуется.
pub fn kerb_returns(
    roads: &[&RoadLine],
    paths: &[Option<&[Vec2]>],
    nodes: &RoadNodes,
    sidewalk: impl Fn(&RoadLine) -> Option<f32>,
) -> KerbReturns {
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
                    at = index;
                    if path[index].distance(node) >= MIN_ARM {
                        next = Some(path[index]);
                        break;
                    }
                }
                let Some(next) = next else {
                    continue;
                };
                let direction = (next - node).normalize();
                // край идёт прямо и через вершины, лежащие на той же прямой: OSM
                // ставит их где угодно — узел пересечения с тротуаром в двух
                // метрах от улицы, излом в полградуса, — и дуга, обрезанная по
                // первой из них, не ложилась совсем
                let mut run = next.distance(node);
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    let offset = path[index] - node;
                    let along = offset.dot(direction);
                    if along <= run || direction.perp_dot(offset).abs() > STRAIGHT_TOLERANCE {
                        break;
                    }
                    run = along;
                    at = index;
                }
                entry.1.push(Arm {
                    class: road.class,
                    half: road.width / 2.0,
                    sidewalk: sidewalk(road),
                    direction,
                    run,
                });
            }
        }
    }

    let mut returns = KerbReturns::default();
    for (node, mut found) in arms.into_values() {
        if found.len() < 2 {
            continue;
        }
        found.sort_by(|a, b| a.direction.to_angle().total_cmp(&b.direction.to_angle()));
        for class in [RoadClass::Street, RoadClass::Alley] {
            let group: Vec<&Arm> = found.iter().filter(|arm| arm.class == class).collect();
            for (first, second) in pairs(&group) {
                let radius = kerb_radius(first, second);
                let halves = (first.half, second.half);
                if let Some(outline) = fillet(node, first, second, halves, radius) {
                    returns.roads.push((class, outline));
                }
            }
        }
        // Тротуары — свои соседи: проезд без тротуара не рвёт полосу улицы,
        // через которую он выходит, и угол считается между улицами по обе
        // стороны от него.
        let walked: Vec<&Arm> = found.iter().filter(|arm| arm.sidewalk.is_some()).collect();
        for (first, second) in pairs(&walked) {
            let (Some(a), Some(b)) = (first.sidewalk, second.sidewalk) else {
                continue;
            };
            // тот же центр, что у бордюрной дуги: радиус меньше на тротуар,
            // полоса шире на него же. Берётся больший из двух тротуаров — при
            // разной их ширине одной дугой обе полосы не обойти, а меньший
            // радиус оставляет асфальтовый клин внутри тротуарного
            let radius = kerb_radius(first, second) - a.max(b);
            let halves = (first.half + a, second.half + b);
            if let Some(outline) = fillet(node, first, second, halves, radius) {
                returns.sidewalks.push(outline);
            }
        }
    }
    returns
}

/// Соседние по углу пары лучей группы — по кругу, чтобы последний луч встретил
/// первый. Меньше двух лучей — пар нет.
fn pairs<'a>(group: &'a [&'a Arm]) -> impl Iterator<Item = (&'a Arm, &'a Arm)> {
    let count = if group.len() < 2 { 0 } else { group.len() };
    (0..count).map(move |index| (group[index], group[(index + 1) % group.len()]))
}

/// Радиус бордюра между двумя лучами узла.
fn kerb_radius(first: &Arm, second: &Arm) -> f32 {
    let (narrow, wide) = (first.half.min(second.half), first.half.max(second.half));
    let mut radius = if wide - narrow > MINOR_WIDTH_STEP {
        // второстепенная дорога входит в большую: въезд с неё почти прямоугольный,
        // широкая дуга делала из каждого проезда воронку
        MINOR_RADIUS_SHARE * narrow
    } else {
        (KERB_RADIUS_SHARE * (first.half + second.half))
            .clamp(*KERB_RADIUS_RANGE.start(), *KERB_RADIUS_RANGE.end())
    };
    if let (Some(a), Some(b)) = (first.sidewalk, second.sidewalk) {
        radius = radius.min(SIDEWALK_COVER * a.min(b));
    }
    radius
}

/// Скругление угла от луча `first` против часовой стрелки до луча `second`:
/// `halves` — полуширины полос, по краям которых сходится угол, `radius` —
/// радиус дуги в этом углу.
fn fillet(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    mut radius: f32,
) -> Option<Vec<Vec2>> {
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
    // угол, где встречаются края: halves.0·n₁ + t·u₁ = halves.1·n₂ + s·u₂
    let rhs = side_second * halves.1 - side_first * halves.0;
    let determinant = -along_first.perp_dot(along_second);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-along_second) / determinant;
    let s = along_first.perp_dot(rhs) / determinant;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    let corner = node + side_first * halves.0 + along_first * t;

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
    // прямые стороны заходят под ленты на `OVERLAP`: сторона, совпадающая с
    // краем ленты, но не делящая с ней вершин, растеризуется с пропусками —
    // по краю проезда шла пунктирная щель со светлым тротуаром под ней
    let mut outline = Vec::with_capacity(steps + 4);
    outline.push(corner - (side_first + side_second) * OVERLAP);
    outline.push(on_first - side_first * OVERLAP);
    outline.push(on_first);
    for step in 1..steps {
        let rotation = Vec2::from_angle(turn * sweep * step as f32 / steps as f32);
        outline.push(centre + rotation.rotate(from));
    }
    outline.push(on_second);
    outline.push(on_second - side_second * OVERLAP);
    Some(outline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;
    use crate::map::osm::model::point_in_polygon;

    fn returns_of(roads: &[RoadLine]) -> Vec<(RoadClass, Vec<Vec2>)> {
        walked_returns_of(roads, |_| None).roads
    }

    fn walked_returns_of(
        roads: &[RoadLine],
        sidewalk: impl Fn(&RoadLine) -> Option<f32>,
    ) -> KerbReturns {
        let nodes = RoadNodes::new(roads);
        let paths: Vec<Option<&[Vec2]>> = roads
            .iter()
            .map(|road| Some(road.points.as_slice()))
            .collect();
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        kerb_returns(&drawn, &paths, &nodes, sidewalk)
    }

    /// Тротуар улицы 8 м — как его считает `roads::sidewalk_width`.
    const SIDEWALK: f32 = 1.76;

    /// Перекрёсток двух улиц 8 м в начале координат.
    fn crossing() -> [RoadLine; 2] {
        [
            east_west(),
            street(
                vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
                8.0,
            ),
        ]
    }

    /// Сквозная улица 8 м по оси x через узел в начале координат.
    fn east_west() -> RoadLine {
        street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        )
    }

    #[test]
    fn a_crossing_gets_four_rounded_corners_outside_both_ribbons() {
        let found = returns_of(&crossing());
        assert_eq!(found.len(), 4);
        for (_, outline) in &found {
            // угол — на пересечении краёв, чуть под лентами
            assert!(
                (outline[0].abs() - Vec2::splat(4.0 - OVERLAP)).length() < 1e-3,
                "{:?}",
                outline[0]
            );
            let corner = Vec2::splat(4.0) * outline[0].signum();
            // и сама дуга за краями обеих лент
            for point in &outline[1..] {
                assert!(
                    point.x.abs() >= 4.0 - OVERLAP - 1e-3 && point.y.abs() >= 4.0 - OVERLAP - 1e-3
                );
            }
            // радиус 4.8: ближе всего к углу середина дуги, в r·(√2 − 1) от него
            let nearest = outline[1..]
                .iter()
                .filter(|point| point.x.abs() >= 4.0 && point.y.abs() >= 4.0)
                .map(|point| point.distance(corner))
                .fold(f32::INFINITY, f32::min);
            let expected = 4.8 * (2.0_f32.sqrt() - 1.0);
            assert!((nearest - expected).abs() < 0.1, "{nearest}");
        }
    }

    #[test]
    fn a_t_junction_rounds_only_the_two_turning_corners() {
        let through = east_west();
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = returns_of(&[through, side]);
        assert_eq!(found.len(), 2);
        // обе дуги со стороны примыкания
        for (_, outline) in &found {
            assert!(outline.iter().all(|point| point.y >= 4.0 - OVERLAP - 1e-3));
        }
        let corner = &found[0].1;
        assert!(!point_in_polygon(Vec2::new(0.0, 10.0), corner));
    }

    #[test]
    fn a_street_and_a_footway_are_not_rounded_together() {
        let through = east_west();
        let path = RoadLine {
            class: RoadClass::Alley,
            ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 3.5)
        };
        assert!(returns_of(&[through, path]).is_empty());
    }

    #[test]
    fn a_vertex_on_a_straight_arm_does_not_cut_the_corner() {
        // проезд пересекает тротуар в двух метрах от улицы — узел на прямой,
        // но скругление по-прежнему ложится полным радиусом
        let through = east_west();
        let drive = street(
            vec![
                Vec2::new(0.0, 40.0),
                Vec2::new(0.05, 6.0),
                Vec2::new(0.0, 2.0),
                Vec2::ZERO,
            ],
            5.0,
        );
        let found = returns_of(&[through, drive]);
        assert_eq!(found.len(), 2);
        for (_, outline) in &found {
            // проезд уже улицы — радиус 0.4 · 2.5 = 1 м: дуга уходит вдоль
            // проезда на него от края улицы
            let reach = outline.iter().map(|point| point.y).fold(0.0, f32::max);
            assert!((reach - 5.0).abs() < 0.1, "{reach}");
        }
    }

    #[test]
    fn a_short_arm_limits_the_radius() {
        // вторая вершина поперечной улицы в двух метрах за краем — дуга не
        // длиннее этого
        let through = east_west();
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

    #[test]
    fn the_sidewalk_turns_the_corner_on_the_kerb_arc() {
        let found = walked_returns_of(&crossing(), |_| Some(SIDEWALK));
        assert_eq!(found.sidewalks.len(), 4);
        // центр бордюрной дуги в северо-восточном углу: угол краёв (4, 4) плюс
        // биссектриса на r/sin 45°, r = 0.6 · 8 = 4.8
        let centre = Vec2::splat(4.0 + 4.8);
        for outline in &found.sidewalks {
            let corner = outline[0].signum() * Vec2::splat(4.0 + SIDEWALK - OVERLAP);
            assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
            // дуга — того же центра, радиусом на тротуар меньше бордюрной, так
            // что за бордюром идёт ровная полоса в ширину тротуара
            let quadrant = outline[0].signum() * centre;
            for point in &outline[2..outline.len() - 1] {
                let radius = point.distance(quadrant);
                assert!((radius - (4.8 - SIDEWALK)).abs() < 1e-2, "{radius}");
            }
        }
        // и асфальтовый клин по-прежнему целиком на тротуаре: где не на полосе,
        // там на её скруглении
        for (_, outline) in &found.roads {
            let quadrant = outline[0].signum() * centre;
            for point in outline {
                let band = 4.0 + SIDEWALK + 1e-3;
                let on_band = point.x.abs() <= band || point.y.abs() <= band;
                assert!(
                    on_band || point.distance(quadrant) >= 4.8 - SIDEWALK - 1e-3,
                    "{point:?}"
                );
            }
        }
    }

    #[test]
    fn a_kerb_radius_under_the_sidewalk_width_leaves_the_corner_square() {
        // жилая улица 8 м входит в магистраль 16 м: радиус въезда 0.4 · 4 =
        // 1.6 м — меньше трёхметрового тротуара магистрали, и внешний угол
        // полосы на месте такой же прямой
        let avenue = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            16.0,
        );
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = walked_returns_of(&[avenue, side], |road| {
            Some(if road.width > 8.0 { 3.0 } else { SIDEWALK })
        });
        assert_eq!(found.roads.len(), 2);
        assert!(found.sidewalks.is_empty());
    }

    #[test]
    fn a_drive_without_a_sidewalk_does_not_break_the_band() {
        // проезд 5 м выходит в улицу: асфальт скругляется с обеих сторон,
        // а полоса тротуара идёт мимо него насквозь — скруглять нечего
        let through = east_west();
        let drive = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 5.0);
        let found = walked_returns_of(&[through, drive], |road| {
            (road.width >= 8.0).then_some(SIDEWALK)
        });
        assert_eq!(found.roads.len(), 2);
        assert!(found.sidewalks.is_empty());
    }

    #[test]
    fn a_drive_between_two_streets_leaves_their_sidewalk_corner_alone() {
        // тот же проезд, но улицы сходятся углом: тротуарный угол считается
        // между ними, поверх устья проезда — его асфальт ляжет сверху
        let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
        let drive = street(vec![Vec2::ZERO, Vec2::new(35.0, 35.0)], 5.0);
        let found = walked_returns_of(&[east, north, drive], |road| {
            (road.width >= 8.0).then_some(SIDEWALK)
        });
        assert_eq!(found.sidewalks.len(), 1);
        let outline = &found.sidewalks[0];
        let corner = Vec2::splat(4.0 + SIDEWALK - OVERLAP);
        assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
    }
}
