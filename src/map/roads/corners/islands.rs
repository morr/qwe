use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::map::grid::Grid;
use crate::map::osm::model::{point_in_area, ring_area, ring_bounds};
use crate::map::osm::{PolyArea, RoadClass};
use crate::map::roads::drawn::Axis;

use super::{
    Arm, Drawn, OUTER_OVERLAP, TILE_OVERLAP_MOUTH, TILE_OVERLAP_OCCUPIED, TILE_OVERLAP_REACH,
    TILE_OVERLAP_WIDTH, node_key,
};

/// Дворы с рамками по сетке — префильтр точной пробы [`point_in_area`]:
/// узлов слияний в городе тысячи, и пробирать каждый полигон двора — даже
/// рамкой — не нужно: точка спрашивает свою ячейку, и только дворы, чья
/// рамка её касается, идут в точный тест.
pub(super) struct YardBounds<'a> {
    /// Квадрат двора в метрах: проба уходит от узла не дальше пары шагов
    /// наезда, и ячейка в пару десятков метров держит списки короткими.
    grid: Grid<usize>,
    yards: Vec<&'a PolyArea>,
}

impl<'a> YardBounds<'a> {
    pub(super) fn of(yards: impl IntoIterator<Item = &'a PolyArea>) -> Self {
        let yards: Vec<&PolyArea> = yards.into_iter().collect();
        let boxes: Vec<(Vec2, Vec2)> = yards.iter().map(|yard| ring_bounds(&yard.outer)).collect();
        let mut grid = Grid::new(YARD_CELL);
        for (index, (min, max)) in boxes.iter().enumerate() {
            grid.insert(*min, *max, index);
        }
        Self { grid, yards }
    }

    /// Двор под точкой — по ячейке сетки, с рамкой как префильтром точного
    /// теста.
    fn any_yard(&self, at: Vec2) -> bool {
        self.grid
            .at(at)
            .iter()
            .any(|&index| point_in_area(at, self.yards[index]))
    }
}

/// Ячейка сетки дворов [`YardBounds`], м.
const YARD_CELL: f32 = 32.0;

/// Наезд плитки на дворе у слияния мощёных дорожек ([`KerbReturns`], вызов в
/// [`kerb_returns`]): для каждой стороны каждого луча — полоса плитки
/// [`TILE_OVERLAP_WIDTH`] на [`TILE_OVERLAP_REACH`] вдоль кромки от узла,
/// если за кромкой двор (`Drawn::yards`) — у устья полосы его нет, а у
/// ближнего конца он уходит от кромки вглубь на ширину полосы, не остриём
/// клина, — и сторона не занята другим лучом ([`TILE_OVERLAP_OCCUPIED`]).
/// Внутренняя сторона заходит под ленту на [`OUTER_OVERLAP`], как у
/// наружного угла; контур — выпуклый четырёхугольник, как и прочие куски
/// слоя тротуаров. Проходной узел — крест, где каждый луч со встречным
/// ([`continues`]) — отсечён вызывающим: там нет щели, которую полоса
/// закрывала бы.
pub(super) fn tile_overlaps(node: Vec2, group: &[&Arm], bounds: &YardBounds) -> Vec<Vec<Vec2>> {
    let mut found = Vec::new();
    for (own, arm) in group.iter().enumerate() {
        for (side, normal) in [arm.direction.perp(), -arm.direction.perp()]
            .into_iter()
            .enumerate()
        {
            // сторона занята: в неё уходит другой луч узла — угол здесь
            // закрыт скруглением, носом или веером
            let occupied = group.iter().enumerate().any(|(other, rest)| {
                other != own && normal.dot(rest.direction) >= TILE_OVERLAP_OCCUPIED.cos()
            });
            if occupied {
                continue;
            }
            // двор где-то в полосе наезда за кромкой — не только вплотную:
            // между кромкой и двором бывает голая земля, и наезд закрывает её
            // вместе с краем двора. Пробы — сеткой (вдоль кромки × от кромки),
            // по рамкам дворов из [`YardBounds`]
            let yard_in_band = |along: f32, off: f32| {
                let at = node + arm.direction * along + normal * (arm.half[side] + off);
                bounds.any_yard(at)
            };
            // у устья полосы двора быть не должно: щель раскрывается от узла
            // (голая земля, дальше двор). Двор вплотную к кромке у узла —
            // не щель, и полоса ложилась бы плитой на газон
            if [TILE_OVERLAP_WIDTH * 0.5, TILE_OVERLAP_WIDTH]
                .into_iter()
                .any(|off| yard_in_band(TILE_OVERLAP_MOUTH, off))
            {
                continue;
            }
            // двор у ближнего конца наезда должен уходить от кромки на ширину
            // полосы: клин, вошедший в кромку остриём глубиной меньше полосы,
            // раскрывается дальше — и ближняя половина полосы ложилась бы на
            // газонный угол слияния плитой (Тула, Первомайская × проспект
            // Ленина, западный угол)
            if ![(0.4, 0.5), (0.4, 1.0)]
                .into_iter()
                .all(|(share_along, share_off)| {
                    yard_in_band(
                        TILE_OVERLAP_REACH * share_along,
                        TILE_OVERLAP_WIDTH * share_off,
                    )
                })
            {
                continue;
            }
            let far = arm.direction * TILE_OVERLAP_REACH;
            found.push(vec![
                node + normal * (arm.half[side] - OUTER_OVERLAP),
                node + normal * (arm.half[side] + TILE_OVERLAP_WIDTH),
                node + far + normal * (arm.half[side] + TILE_OVERLAP_WIDTH),
                node + far + normal * (arm.half[side] - OUTER_OVERLAP),
            ]);
        }
    }
    found
}

/// Остров внутри треугольника узлов заливается, когда от него за вычетом
/// полотен остаётся полоса уже этого, м: вписанный радиус треугольника минус
/// наибольшая полуширина его улиц. Развилка в Туле (витрина 06) — треугольник
/// в 17–31 м со сторон, вписанный радиус 5.2 м при полуширине 3.8: от острова
/// оставалась линза в метр с небольшим, и в ней серпом светлела земля.
const ISLAND_FILL: f32 = 2.0;
/// Периметр треугольника, выше которого остров не трогается, м: большой
/// треугольник развилки — настоящий остров, со своим газоном или домом.
const ISLAND_PERIMETER_MAX: f32 = 120.0;

/// Острова-крошки: треугольник из трёх общих узлов, попарно соединённых
/// кусками улиц, от которого за полотнами почти ничего не остаётся
/// ([`ISLAND_FILL`]). Контур — по нарисованным осям `paths` (`None` — дорога
/// не участвует); кладётся асфальтом под ленты, как перепонки колец.
pub fn small_islands(drawn: &Drawn) -> Vec<Vec<Vec2>> {
    let nodes = drawn.nodes();
    // рёбра: куски улиц между соседними общими узлами
    struct Edge<'a> {
        ends: [(i32, i32); 2],
        path: &'a [Vec2],
        half: f32,
    }
    let mut edges: Vec<Edge> = Vec::new();
    for index in 0..drawn.len() {
        let Some(path) = rounded(drawn, index) else {
            continue;
        };
        let road = drawn.road(index);
        if road.class != RoadClass::Street || path.len() < 2 {
            continue;
        }
        let shared: Vec<usize> = (0..path.len())
            .filter(|&index| nodes.is_shared(path[index]))
            .collect();
        for pair in shared.windows(2) {
            edges.push(Edge {
                ends: [node_key(path[pair[0]]), node_key(path[pair[1]])],
                path: &path[pair[0]..=pair[1]],
                half: road.width / 2.0,
            });
        }
    }
    let mut at: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if edge.ends[0] != edge.ends[1] {
            at.entry(edge.ends[0]).or_default().push(index);
            at.entry(edge.ends[1]).or_default().push(index);
        }
    }
    let other = |edge: usize, end: (i32, i32)| {
        let [a, b] = edges[edge].ends;
        if a == end { b } else { a }
    };
    // ребро, пройденное от узла `from`
    let walk = |edge: usize, from: (i32, i32)| -> Vec<Vec2> {
        let mut points = edges[edge].path.to_vec();
        if edges[edge].ends[0] != from {
            points.reverse();
        }
        points
    };
    let mut islands = Vec::new();
    for (first, edge) in edges.iter().enumerate() {
        let [a, b] = edge.ends;
        for &second in at.get(&b).into_iter().flatten() {
            let c = other(second, b);
            if second <= first || c == a {
                continue;
            }
            for &third in at.get(&c).into_iter().flatten() {
                if third <= first || third == second || other(third, c) != a {
                    continue;
                }
                let mut outline = walk(first, a);
                for (edge, from) in [(second, b), (third, c)] {
                    outline.pop();
                    outline.extend(walk(edge, from));
                }
                outline.pop();
                let perimeter: f32 = outline
                    .iter()
                    .zip(outline.iter().cycle().skip(1))
                    .map(|(p, q)| p.distance(*q))
                    .sum();
                let area = ring_area(&outline);
                let half = [first, second, third]
                    .map(|edge| edges[edge].half)
                    .into_iter()
                    .fold(0.0, f32::max);
                if perimeter < ISLAND_PERIMETER_MAX && 2.0 * area / perimeter - half < ISLAND_FILL {
                    islands.push(outline);
                }
            }
        }
    }
    islands
}

/// Узловая осевая дороги, если она участвует в узлах: мост и арка — нет, их
/// торцы стоят ровным срезом бордюра и у стен дома.
pub(super) fn rounded<'d>(drawn: &'d Drawn, index: usize) -> Option<&'d [Vec2]> {
    (!drawn.road(index).carves_navmesh()).then(|| drawn.axis(index, Axis::Nodal))
}
