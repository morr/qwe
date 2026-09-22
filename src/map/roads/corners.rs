//! Асфальт узла: скругления бордюра между дорогами, сходящимися в общем узле,
//! наружные углы узла и торцы плеч.
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
//! **Радиус — по младшему классу пары** ([`kerb_radius`]): между проспектами
//! 10 м, с улицей 6, с проездом 2.5. Раньше он шёл от ширин, и въезд узкой
//! дороги в широкую получал 0.4 её полуширины — у разделённого проспекта, где
//! в узле сходятся половины разной полосности, углы выходили в метр-два.
//!
//! **Плечо, кончающееся в узле, кончается прямым торцом** ([`KerbReturns::
//! butt`]): круглый торец широкой дороги, упёршейся в узкую, выпирал полудиском
//! за дальний край узкой. Прямой торец лежит на оси узла, а наружный угол
//! между плечами без сквозной дороги (гнутый угол двух улиц, развилка)
//! закругляется веером от узла ([`outer_corner`]) — там, где круглые торцы
//! давали это даром. Узел здесь — три плеча и больше или два, сходящиеся
//! углом; два почти соосных торца — продолжение дороги, и торцы у них круглые,
//! как были.
//!
//! Полигон узла объединением (`i_overlay`) не строится: куски лежат под
//! лентами своего слоя, и щели между ними лента закрывает сама, а объединение
//! на каждый из девяти тысяч узлов стоило бы сотни миллисекунд загрузки.
//!
//! **Тротуар поворачивает вместе с бордюром.** Полоса тротуара шире
//! проезжей части, и её собственный угол на перекрёстке оставался прямым:
//! асфальт выкатывался дугой в угол, срезая тротуар до нитки, а за ним торчал
//! прямоугольный уступ светлой полосы — на снимке это и видно. Тот же клин, но
//! в слое тротуаров, кладётся по краям полос (полуширина плюс ширина тротуара)
//! и дугой **того же центра**: радиус меньше ровно на ширину тротуара, и за
//! бордюром идёт постоянная полоса шириной в тротуар — как на месте. Радиус
//! меньше тротуара — угол и на месте прямой (въезд с малым радиусом), дуги
//! нет. Асфальтовый клин при этом всегда лежит на тротуарном: дуги
//! концентричны, и касательные у них общие, так что клин между краями дорог и
//! бордюрной дугой — внутри клина между краями тротуаров и тротуарной.

use std::f32::consts::PI;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::node_key;
use super::network::RoadNodes;
use crate::map::meshing::arc_steps;
use crate::map::osm::{Highway, RoadClass, RoadLine};

/// Радиус бордюра по классу дороги, м; у пары берётся меньший. Между
/// проспектами (`trunk`…`secondary` и их съезды) — 10 м, с улицей
/// (`tertiary`, жилая, `unclassified`) — 6, с проездом, жилой зоной или
/// переездом через тротуар — 2.5, между пешеходными дорожками — 2.
const MAJOR_RADIUS: f32 = 10.0;
const STREET_RADIUS: f32 = 6.0;
const DRIVE_RADIUS: f32 = 2.5;
const PATH_RADIUS: f32 = 2.0;
/// Скругление меньше этого не кладётся, м: его всё равно не видно.
const MIN_RADIUS: f32 = 0.5;
/// Угол между лучами, в котором скругление имеет смысл. Острее — дуга
/// вытягивается в длинный клин; почти развёрнутый угол — это продолжение
/// дороги, а не поворот.
const MIN_ANGLE: f32 = 25.0 * PI / 180.0;
const MAX_ANGLE: f32 = 155.0 * PI / 180.0;
/// Наружный угол узла закругляется, когда просвет между плечами шире
/// развёрнутого хотя бы на столько: у сквозной дороги просветы ровно по
/// 180°, и веер там лёг бы под её же ленту.
const MIN_OUTER: f32 = 1.0 * PI / 180.0;
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
    highway: Highway,
    half: f32,
    /// Тротуар слева и справа по ходу луча: у половины разделённой улицы со
    /// стороны пары его нет.
    sidewalk: [Option<f32>; 2],
    direction: Vec2,
    /// Сколько метров край ленты идёт прямо — до следующей вершины.
    run: f32,
    /// Дорога и её торец (`0` — начало, `1` — конец), если луч — торец пути.
    end: Option<(usize, usize)>,
}

/// Асфальт всех узлов.
#[derive(Default)]
pub struct KerbReturns {
    /// Контур и класс дорог, в чей слой заливки он ляжет: скругления и
    /// наружные углы. Каждый — веер из первой вершины.
    pub roads: Vec<(RoadClass, Vec<Vec2>)>,
    /// Контуры в слое тротуаров, так же.
    pub sidewalks: Vec<Vec<Vec2>>,
    /// Сколько из `roads` и `sidewalks` — наружные углы, а не скругления.
    pub outer: [usize; 2],
    /// Торцы, кончающиеся в узле, по дорогам: `[начало, конец]` — ленты с
    /// таким торцом кладутся с прямым, а не круглым.
    pub butt: Vec<[bool; 2]>,
}

impl KerbReturns {
    /// Торцы дороги `road`; вне узлов (или без скруглений вовсе) — круглые.
    pub fn butt(&self, road: usize) -> [bool; 2] {
        self.butt.get(road).copied().unwrap_or_default()
    }
}

/// Асфальт всех узлов: скругления проезжей части и тротуаров, наружные углы и
/// прямые торцы плеч.
///
/// `paths` — **нарисованные** осевые (после сглаживания, до стежков), по
/// индексу дороги; `None` — дорога не участвует (мост, арка). `sidewalk` —
/// ширина тротуара дороги (по индексу), если он рисуется; `paired(дорога,
/// длина по оси)` — лежит ли там рядом вторая половина разделённой улицы и
/// слева ли (`roads/network/pairs.rs`): с её стороны тротуара нет, и угол по
/// нему не скругляется.
pub fn kerb_returns(
    roads: &[&RoadLine],
    paths: &[Option<&[Vec2]>],
    nodes: &RoadNodes,
    sidewalk: impl Fn(usize) -> Option<f32>,
    paired: impl Fn(usize, f32) -> Option<bool>,
) -> KerbReturns {
    let mut arms: HashMap<(i32, i32), (Vec2, Vec<Arm>)> = HashMap::new();
    for (index, (&road, path)) in roads.iter().zip(paths).enumerate() {
        let Some(path) = *path else {
            continue;
        };
        if path.len() < 2 {
            continue;
        }
        let closed = path[0] == path[path.len() - 1];
        let last = path.len() - 1;
        let mut along = 0.0;
        for (vertex, &node) in path.iter().enumerate() {
            if vertex > 0 {
                along += node.distance(path[vertex - 1]);
            }
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
                let at_end = !closed && (vertex == 0 || vertex == last);
                // стороны луча: слева по пути — слева по лучу вперёд и справа
                // по лучу назад
                let own = sidewalk(index);
                let mut sides = [own; 2];
                // тротуар по тегу — слева или справа по пути (`sidewalk=*`)
                for (side, present) in road.sidewalks.into_iter().enumerate() {
                    if !present {
                        sides[usize::from((side == 0) != forward)] = None;
                    }
                }
                if let Some(left) = paired(index, along) {
                    sides[usize::from(left != forward)] = None;
                }
                entry.1.push(Arm {
                    class: road.class,
                    highway: road.highway,
                    half: road.width / 2.0,
                    sidewalk: sides,
                    direction,
                    run,
                    end: at_end.then_some((index, usize::from(vertex == last))),
                });
            }
        }
    }

    let mut returns = KerbReturns {
        butt: vec![[false; 2]; roads.len()],
        ..default()
    };
    for (node, mut found) in arms.into_values() {
        if found.len() < 2 {
            continue;
        }
        found.sort_by(|a, b| a.direction.to_angle().total_cmp(&b.direction.to_angle()));
        for class in [RoadClass::Street, RoadClass::Alley] {
            let group: Vec<&Arm> = found.iter().filter(|arm| arm.class == class).collect();
            if !is_junction(&group) {
                continue;
            }
            for arm in &group {
                if let Some((road, end)) = arm.end {
                    returns.butt[road][end] = true;
                }
            }
            for (first, second) in pairs(&group) {
                let radius = kerb_radius(first, second);
                let halves = (first.half, second.half);
                if let Some(outline) = fillet(node, first, second, halves, radius) {
                    returns.roads.push((class, outline));
                } else if let Some(outline) = outer_corner(node, first, second, halves) {
                    returns.roads.push((class, outline));
                    returns.outer[0] += 1;
                }
            }
        }
        // Тротуары — свои соседи: проезд без тротуара не рвёт полосу улицы,
        // через которую он выходит, и угол считается между улицами по обе
        // стороны от него.
        let walked: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.sidewalk.iter().any(Option::is_some))
            .collect();
        if !is_junction(&walked) {
            continue;
        }
        for (first, second) in pairs(&walked) {
            // угол от левого края первого луча к правому краю второго
            let (Some(a), Some(b)) = (first.sidewalk[0], second.sidewalk[1]) else {
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
            } else if let Some(outline) = outer_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
                returns.outer[1] += 1;
            }
        }
    }
    returns
}

/// Узел ли это для группы лучей одного класса: три плеча и больше или два,
/// сходящиеся углом. Два почти соосных плеча — продолжение дороги (другой
/// класс, другой way), и острые — развилка без третьего плеча: там торцы
/// остаются круглыми.
fn is_junction(group: &[&Arm]) -> bool {
    match group {
        [] | [_] => false,
        [first, second] => {
            let angle = ccw_angle(first, second);
            let turn = |angle: f32| (MIN_ANGLE..=MAX_ANGLE).contains(&angle);
            turn(angle) || turn(2.0 * PI - angle)
        }
        _ => true,
    }
}

/// Угол от луча `first` против часовой стрелки до луча `second`, (0, 2π].
fn ccw_angle(first: &Arm, second: &Arm) -> f32 {
    let angle = second.direction.to_angle() - first.direction.to_angle();
    if angle <= 0.0 {
        angle + 2.0 * PI
    } else {
        angle
    }
}

/// Соседние по углу пары лучей группы — по кругу, чтобы последний луч встретил
/// первый. Меньше двух лучей — пар нет.
fn pairs<'a>(group: &'a [&'a Arm]) -> impl Iterator<Item = (&'a Arm, &'a Arm)> {
    let count = if group.len() < 2 { 0 } else { group.len() };
    (0..count).map(move |index| (group[index], group[(index + 1) % group.len()]))
}

/// Радиус бордюра между двумя лучами узла — по младшему классу пары (см.
/// [`MAJOR_RADIUS`]).
fn kerb_radius(first: &Arm, second: &Arm) -> f32 {
    class_radius(first).min(class_radius(second))
}

fn class_radius(arm: &Arm) -> f32 {
    if arm.class == RoadClass::Alley {
        return PATH_RADIUS;
    }
    match arm.highway {
        Highway::Motorway
        | Highway::Trunk
        | Highway::Primary
        | Highway::Secondary
        | Highway::MotorwayLink
        | Highway::TrunkLink
        | Highway::PrimaryLink
        | Highway::SecondaryLink => MAJOR_RADIUS,
        Highway::Tertiary
        | Highway::TertiaryLink
        | Highway::Residential
        | Highway::Unclassified => STREET_RADIUS,
        // жилая зона, проезд и переезд через тротуар (дорожка, нарисованная
        // асфальтом, — `network::driveway_crossings`)
        Highway::LivingStreet | Highway::Service | Highway::Path => DRIVE_RADIUS,
    }
}

/// Наружный угол от луча `first` против часовой стрелки до луча `second`, если
/// просвет между ними шире развёрнутого: веер от узла с радиусом от одной
/// полуширины к другой — то, что раньше давали круглые торцы лент. Первая
/// вершина — центр веера, чуть позади узла, стороны заходят под торцы лент
/// на [`OVERLAP`].
fn outer_corner(node: Vec2, first: &Arm, second: &Arm, halves: (f32, f32)) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= PI + MIN_OUTER {
        return None;
    }
    let sweep = angle - PI;
    let from = first.direction.perp();
    let steps = arc_steps(halves.0.max(halves.1), sweep).max(1);
    let middle = Vec2::from_angle(sweep / 2.0).rotate(from);
    let mut outline = Vec::with_capacity(steps + 2);
    outline.push(node - middle * OVERLAP);
    outline.push(node + from * halves.0 + first.direction * OVERLAP);
    for step in 1..steps {
        let share = step as f32 / steps as f32;
        let radius = halves.0 + (halves.1 - halves.0) * share;
        outline.push(node + Vec2::from_angle(sweep * share).rotate(from) * radius);
    }
    outline.push(node - second.direction.perp() * halves.1 + second.direction * OVERLAP);
    Some(outline)
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
    let angle = ccw_angle(first, second);
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
        kerb_returns(
            &drawn,
            &paths,
            &nodes,
            |index| sidewalk(&roads[index]),
            |_, _| None,
        )
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
            // радиус двух жилых улиц: все точки дуги на нём от центра
            let centre = corner + Vec2::splat(STREET_RADIUS) * outline[0].signum();
            for point in &outline[2..outline.len() - 1] {
                let radius = point.distance(centre);
                assert!((radius - STREET_RADIUS).abs() < 1e-2, "{radius}");
            }
        }
    }

    /// Радиус скругления прямого угла: касательная от угла краёв до дуги
    /// равна ему. Третья вершина контура — точка касания на первом луче.
    fn right_angle_radius(outline: &[Vec2]) -> f32 {
        let corner = outline[0] + (outline[0].signum() * OVERLAP);
        outline[2].distance(corner)
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
            // с проездом радиус проезда: дуга уходит вдоль проезда на него от
            // края улицы
            let reach = outline.iter().map(|point| point.y).fold(0.0, f32::max);
            assert!((reach - (4.0 + DRIVE_RADIUS)).abs() < 0.1, "{reach}");
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
        // биссектриса на r/sin 45°
        let centre = Vec2::splat(4.0 + STREET_RADIUS);
        for outline in &found.sidewalks {
            let corner = outline[0].signum() * Vec2::splat(4.0 + SIDEWALK - OVERLAP);
            assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
            // дуга — того же центра, радиусом на тротуар меньше бордюрной, так
            // что за бордюром идёт ровная полоса в ширину тротуара
            let quadrant = outline[0].signum() * centre;
            for point in &outline[2..outline.len() - 1] {
                let radius = point.distance(quadrant);
                assert!(
                    (radius - (STREET_RADIUS - SIDEWALK)).abs() < 1e-2,
                    "{radius}"
                );
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
                    on_band || point.distance(quadrant) >= STREET_RADIUS - SIDEWALK - 1e-3,
                    "{point:?}"
                );
            }
        }
    }

    #[test]
    fn a_kerb_radius_under_the_sidewalk_width_leaves_the_corner_square() {
        // проезд с тротуаром выходит в магистраль: радиус проезда 2.5 м меньше
        // трёхметрового тротуара, и внешний угол полосы на месте такой же
        // прямой
        let avenue = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            16.0,
        );
        let side = RoadLine {
            highway: Highway::Service,
            ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0)
        };
        let found = walked_returns_of(&[avenue, side], |_| Some(3.0));
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
        // скругление внутри угла и наружный угол по другую сторону узла
        assert_eq!(found.sidewalks.len(), 2);
        assert_eq!(found.outer[1], 1);
        let outline = &found.sidewalks[0];
        let corner = Vec2::splat(4.0 + SIDEWALK - OVERLAP);
        assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
    }

    #[test]
    fn no_sidewalk_corner_on_the_side_of_the_paired_half() {
        // сквозная — половина разделённой улицы, вторая половина слева (к
        // северу): с той стороны тротуара нет, и углов по нему тоже
        let roads = crossing();
        let nodes = RoadNodes::new(&roads);
        let paths: Vec<Option<&[Vec2]>> = roads
            .iter()
            .map(|road| Some(road.points.as_slice()))
            .collect();
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        let found = kerb_returns(
            &drawn,
            &paths,
            &nodes,
            |_| Some(SIDEWALK),
            |road, _| (road == 0).then_some(true),
        );
        assert_eq!(found.roads.len(), 4, "асфальт скругляется, как был");
        assert_eq!(found.sidewalks.len(), 2);
        for outline in &found.sidewalks {
            assert!(outline[0].y < 0.0, "{:?}", outline[0]);
        }
    }

    fn with_highway(road: RoadLine, highway: Highway) -> RoadLine {
        RoadLine { highway, ..road }
    }

    #[test]
    fn the_radius_follows_the_minor_class_of_the_pair() {
        let nearest_to_corner = |found: &[(RoadClass, Vec<Vec2>)]| right_angle_radius(&found[0].1);
        let [through, side] = crossing();
        let avenues = returns_of(&[
            with_highway(through.clone(), Highway::Primary),
            with_highway(side.clone(), Highway::Secondary),
        ]);
        assert!((nearest_to_corner(&avenues) - MAJOR_RADIUS).abs() < 0.1);
        let street_into_avenue = returns_of(&[
            with_highway(through.clone(), Highway::Primary),
            with_highway(side.clone(), Highway::Residential),
        ]);
        assert!((nearest_to_corner(&street_into_avenue) - STREET_RADIUS).abs() < 0.1);
        let drive = returns_of(&[
            with_highway(through, Highway::Primary),
            with_highway(side, Highway::Service),
        ]);
        assert!((nearest_to_corner(&drive) - DRIVE_RADIUS).abs() < 0.1);
    }

    #[test]
    fn an_arm_ending_in_a_junction_ends_square_and_a_seam_stays_round() {
        // Т-образный узел: торец примыкающей улицы в узле — прямой, у
        // сквозной там не торец
        let through = east_west();
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = walked_returns_of(&[through, side], |_| None);
        assert_eq!(found.butt, vec![[false; 2], [false, true]]);
        // шов двух ways одной улицы — не узел, торцы круглые
        let first = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
        let second = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let found = walked_returns_of(&[first, second], |_| None);
        assert_eq!(found.butt, vec![[false; 2]; 2]);
        assert!(found.roads.is_empty());
    }

    #[test]
    fn two_streets_meeting_at_a_corner_get_a_rounded_outer_side() {
        // угол двух улиц без сквозной: скругление внутри, веер снаружи — тот
        // же, что прежде давали круглые торцы
        let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
        let found = walked_returns_of(&[east, north], |_| None);
        assert_eq!(found.roads.len(), 2);
        assert_eq!(found.outer[0], 1);
        assert_eq!(found.butt, vec![[true, false]; 2]);
        let outer = &found.roads[1].1;
        // веер — в третьем квадранте, радиус полуширины
        for point in &outer[2..outer.len() - 1] {
            assert!(point.x < 0.0 && point.y < 0.0, "{point:?}");
            assert!((point.length() - 4.0).abs() < 1e-3, "{point:?}");
        }
        assert!(point_in_polygon(Vec2::new(-2.0, -2.0), outer));
    }
}
