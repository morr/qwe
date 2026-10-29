//! Кварталы (`landuse`), дотянутые до нарисованных полотен дорог
//! ([`pull_areas_to_roads`]), — вершину за вершиной, зеленью только наружу.

use bevy::math::Vec2;

use super::verges;
use super::{LANDUSE_OVERLAP, Link, SIDEWALK_CELL};
use crate::map::grid::Grid;
use crate::map::osm::model::{MapData, RoadClass, RoadLine, closest_on_segment, signed_ring_area};

/// Зазор между краем квартала и внешним краем нарисованного полотна, м,
/// который ещё дотягивается. Больше — это уже не щель, а настоящий промежуток
/// (палисадник, обочина, полоса отвода), и зелень туда лезть не должна.
/// На Туле в предел попадает четверть вершин кварталов из 2756: по метрам
/// 261 / 174 / 130 / 88 / 99, и дальше пяти метров их ещё 200 с лишним. Было
/// 3 м, поднято по взгляду на кадр: на четвёртом и пятом метре полоска земли
/// вдоль улицы всё ещё читается швом, а не обочиной.
const LANDUSE_GAP_MAX: f32 = 5.0;
/// Тот же предел для **угла** квартала у перекрёстка — вершины, под которой
/// сходятся полотна двух улиц поперёк друг друга, м. Угол отступает от
/// перекрёстка по биссектрисе, то есть дальше, чем край от каждой из улиц, и
/// на пяти метрах треугольник земли у скругления оставался (пример 13). Угол
/// встаёт под оба полотна сразу, так что больший предел не выводит зелень из-под
/// дорог.
const LANDUSE_CORNER_GAP_MAX: f32 = 8.0;
/// Длиннее этого, м, ребро квартала рядом с дорогой разбивается на части.
/// Между своими вершинами ребро прямое, а дорога гнётся, и на выпуклости
/// поворота щель осталась бы посреди ребра, где двигать нечего.
const LANDUSE_STEP: f32 = 8.0;
/// Насколько далеко от тротуара-дорожки, м, край квартала между ней и улицей
/// ещё уводится под неё ([`pull_vertex`]). Дальше — уже не обрезок мощения, а
/// своя полоса: газон между тротуаром и проезжей частью.
const SIDEWALK_TUCK_MAX: f32 = 3.0;

/// Полотна дорог для [`pull_ring`]: звенья с тем, что за дорога у каждого,
/// и их сетка.
struct Edges<'a> {
    edges: &'a [Edge],
    lines: &'a Grid<usize>,
}

/// Звено полотна и что за дорога у него — одним значением, а не двумя
/// массивами рядом, так что «та же длина, тот же порядок» ломаться нечему.
#[derive(Clone, Copy)]
struct Edge {
    link: Link,
    kind: EdgeKind,
    /// Полуширина проезжей части — кромка там, где тротуара нет.
    kerb: f32,
    /// `[слева, справа]` по ходу звена: у проезжей части с этой стороны
    /// тротуар не рисуется (`sidewalk=no|separate`), и за кромкой лежит голая
    /// земля или обочина до дорожки, а не плитка тротуара.
    bare: [bool; 2],
}

impl Edge {
    /// Зазор от точки до края полотна, **нарисованного с её стороны**: у
    /// голой стороны — до кромки, у прочих — до края по карте (`link.reach`).
    fn gap(&self, point: Vec2, axis: Vec2) -> f32 {
        let Link { from, to, reach } = self.link;
        // `perp` смотрит влево по ходу точек — сторона 0
        let side = usize::from((to - from).perp_dot(point - from) < 0.0);
        point.distance(axis) - if self.bare[side] { self.kerb } else { reach }
    }
}

/// Что за дорога у звена [`Edge`]. Варианты не пересекаются: проезжая часть
/// бывает только у улицы ([`RoadLine::is_carriageway`]), дорожка — сама класс.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    /// Проезжая часть ([`RoadLine::is_carriageway`]).
    Carriageway,
    /// Дорожка ([`RoadClass::Alley`]): тротуар, замапленный отдельно, и
    /// прочие пешеходные пути.
    Walkway,
    /// Улица без проезжей части — проезд.
    Drive,
}

impl EdgeKind {
    fn of(road: &RoadLine) -> Self {
        if road.is_carriageway() {
            Self::Carriageway
        } else if road.class == RoadClass::Alley {
            Self::Walkway
        } else {
            Self::Drive
        }
    }
}

/// Квартал (`landuse`), край которого не доходит до дороги считаные метры,
/// **дотягивается под полотно**. Возвращает [`StretchedAreas`] — сдвинутые
/// вершины кварталов.
///
/// Ширина улицы в модели — константа класса, тротуар добавляет рендер, а
/// границу квартала в OSM рисуют по красным линиям или по заборам участков,
/// и между двором и нарисованным тротуаром остаётся полоска голой земли в
/// метр-полтора (Тула, `landuse=residential` 185117817 вдоль улицы
/// Воздухофлотской: граница в 6.1 м от оси при 5.76 м полосы). На снимке это
/// читается как непрокрашенный шов, а не как обочина.
///
/// Двигается **вершина**, а не квартал целиком: край подтягивается к оси
/// ближайшей дороги до [`super::LANDUSE_OVERLAP`] внутрь её полотна. Квартал лежит
/// ниже всего, что на нём нарисовано (`Z_LANDUSE` 0.25 против `Z_SIDEWALK`
/// 1.6), так что заведённая под асфальт зелень не видна — видно только то,
/// что щель закрылась.
///
/// **Зелень только растёт**: вершина идёт к дороге, лишь если этот сдвиг ведёт
/// **наружу от заливки** ([`pull_ring`]). Иначе улица, проходящая внутри
/// квартала, сжала бы его границу к себе; а улица в дырке, наоборот, дырку
/// сжимает — зелени там нет, и подходить к полотну обязан её край.
///
/// Мосты и арки пропущены: под мостом квартал и так рисуется, а проезд сквозь
/// дом — это не край двора.
///
/// **Стоянки дотягиваются тем же шагом разбора, но другой конструкцией**
/// ([`super::lots::pave_lots`], вызван отдельной строкой `finish_parse` сразу за этим
/// проходом): пустырь между краем площадки и проездом вдоль неё — то же самое,
/// что полоска земли между двором и тротуаром, только шире, и закрывается он
/// не сдвигом вершин, а замыканием площадки с полотнами дорог. Повершинное
/// правило у стоянок было и снято: точки контура уезжали порознь, и край
/// выходил зубцами и иглами.
pub(super) fn pull_areas_to_roads(map: &mut MapData) -> StretchedAreas {
    // `reach` — внешний край нарисованного полотна от оси
    let mut edges: Vec<Edge> = Vec::new();
    for road in &map.roads {
        if road.bridge || road.passage {
            continue;
        }
        // тротуар — только тот, что рисуется: у `sidewalk=separate|no` его нет,
        // и квартал, дотянутый под несуществующую полосу, вставал за бордюром
        let sidewalk = road.sidewalk();
        let reach = sidewalk.mapped_edge(road.width / 2.0);
        let kind = EdgeKind::of(road);
        let bare = [0, 1].map(|side| kind == EdgeKind::Carriageway && sidewalk.on(side).is_none());
        for link in road.points.windows(2) {
            edges.push(Edge {
                link: Link {
                    from: link[0],
                    to: link[1],
                    reach,
                },
                kind,
                kerb: road.width / 2.0,
                bare,
            });
        }
    }
    // звено кладётся в ячейки с запасом на своё полотно и наибольший из
    // пределов, так что спрашивающему хватает ячейки самой вершины; запас
    // больше нужного лишь добавляет кандидатов, которые отсеет сам предел
    let mut lines: Grid<usize> = Grid::new(SIDEWALK_CELL);
    for (index, edge) in edges.iter().enumerate() {
        let Link { from, to, reach } = edge.link;
        lines.insert_segment(from, to, reach + LANDUSE_CORNER_GAP_MAX, index);
    }
    let roads = Edges {
        edges: &edges,
        lines: &lines,
    };

    let mut stretched = StretchedAreas::default();
    for area in &mut map.landuse {
        area.outer = pull_ring(&area.outer, false, &roads, &mut stretched.blocks);
        area.holes = area
            .holes
            .iter()
            .map(|hole| pull_ring(hole, true, &roads, &mut stretched.blocks))
            .collect();
    }
    // вершины решают порознь, а ребро между ними прямое: на плитке обочины
    // двор ещё мог остаться — вырезается площадью
    let started = std::time::Instant::now();
    stretched.cut = verges::cut_verges_from_blocks(map);
    stretched.cutting = started.elapsed();
    stretched
}

/// Что дотянул проход: вершин у кварталов — **площадки** у стоянок
/// ([`super::lots::pave_lots`], второй строкой того же шага `finish_parse`) растут
/// полигоном, и вершины у них считать нечего.
#[derive(Default)]
pub(super) struct StretchedAreas {
    pub(super) blocks: usize,
    /// Кварталов, из которых вырезаны обочины ([`super::verges`]).
    pub(super) cut: usize,
    /// Сколько длился вырез — доля общего времени прохода.
    pub(super) cutting: std::time::Duration,
}

/// Кольцо квартала с дотянутыми к дорогам вершинами.
///
/// Ребро длиннее [`LANDUSE_STEP`] рядом с дорогой разбивается, и вставленная
/// точка остаётся в кольце, только если ей нашлось куда сдвинуться, — иначе
/// кольцо копило бы лишние вершины на каждой перестройке геометрии.
///
/// «Наружу от зелени» считается **локально**, по самому кольцу: для внешнего
/// контура это прочь из квартала, для дырки — внутрь неё (зелень лежит снаружи
/// такого кольца). Локально, а не вопросом «лежит ли дорога вне квартала»: у
/// полосы газона между двумя улицами ближайшая улица бывает **за
/// противоположным** краем, и такой ответ сжал бы полосу вместо того, чтобы её
/// растянуть.
fn pull_ring(ring: &[Vec2], hole: bool, roads: &Edges, pulled: &mut usize) -> Vec<Vec2> {
    // ориентация колец в OSM произвольная, так что сторону задаёт знак площади
    let sign = if (signed_ring_area(ring) > 0.0) == hole {
        1.0
    } else {
        -1.0
    };
    let normal = |from: Vec2, to: Vec2| ((to - from).perp() * sign).normalize_or_zero();
    let mut out: Vec<Vec2> = Vec::with_capacity(ring.len());
    for (index, &point) in ring.iter().enumerate() {
        let mut push =
            |point: Vec2, outward: Vec2, inserted: bool| match pull_vertex(point, outward, roads) {
                Some(shifted) => {
                    out.push(shifted);
                    *pulled += 1;
                }
                None if inserted => {}
                None => out.push(point),
            };
        let previous = ring[(index + ring.len() - 1) % ring.len()];
        let next = ring[(index + 1) % ring.len()];
        // у вершины — биссектриса её рёбер, у вставленной точки — нормаль
        // самого ребра
        let along = normal(point, next);
        push(
            point,
            (normal(previous, point) + along).normalize_or_zero(),
            false,
        );
        let length = point.distance(next);
        if length <= LANDUSE_STEP
            || roads
                .lines
                .near(point.min(next), point.max(next))
                .is_empty()
        {
            continue;
        }
        let steps = (length / LANDUSE_STEP).ceil() as usize;
        for step in 1..steps {
            push(point.lerp(next, step as f32 / steps as f32), along, true);
        }
    }
    out
}

/// Куда встаёт вершина квартала, которой до полотна дороги остался зазор не
/// больше [`LANDUSE_GAP_MAX`]; `None` — двигать нечего или некуда. `outward` —
/// куда от этой вершины прибывает заливка (см. [`pull_ring`]).
fn pull_vertex(point: Vec2, outward: Vec2, roads: &Edges) -> Option<Vec2> {
    // зазор мерится **до края полотна**, а не до оси: узкий проезд рядом ближе
    // широкой улицы, а щель оставляет улица. Берётся ближайшая дорога вообще;
    // лежит вершина под её полотном — это ответ «тянуть некуда», и никакая
    // другая дорога его не отменяет
    let gaps = || {
        roads.lines.near(point, point).into_iter().map(|index| {
            let Link { from, to, reach } = roads.edges[index].link;
            let axis = closest_on_segment(point, from, to);
            (point.distance(axis) - reach, axis, index)
        })
    };
    // кроме одного случая: вершина между тротуаром, замапленным дорожкой, и
    // улицей за ним. Квартал в OSM нарисован до бордюра, а наша проезжая
    // часть уже (Берлин, витрина 03: край квартала в 6–7 м от оси при
    // полуширине 3.8), и двор торчал из-под тротуара серпом у каждого
    // скругления угла. Полоса между тротуаром и бордюром — мощение, не двор:
    // край уходит под дорожку — до её оси, а не на [`super::LANDUSE_OVERLAP`] за
    // кромку: край между вершинами прямой, а дорожка гнётся, и полметра
    // запаса оставляли вдоль неё волосяную нить двора (Тула, витрина 15)
    let tuck = |gap: f32, axis: Vec2, index: usize| -> Option<Vec2> {
        let direction = (axis - point).try_normalize()?;
        // улица за ней — по нарисованному краю: у стороны без тротуара
        // вершина может стоять в полосе тротуара по карте, которого там нет
        let street_beyond = gaps().any(|(gap, axis, index)| {
            roads.edges[index].kind == EdgeKind::Carriageway
                && roads.edges[index].gap(point, axis) > 0.0
                && gap <= LANDUSE_GAP_MAX
                && (axis - point).dot(outward) > 0.0
        });
        (roads.edges[index].kind == EdgeKind::Walkway
            && gap <= SIDEWALK_TUCK_MAX
            && direction.dot(outward) <= 0.0
            && street_beyond)
            .then_some(axis)
    };
    let (gap, axis, index) = gaps().min_by(|a, b| a.0.total_cmp(&b.0))?;
    if gap <= 0.0 && roads.edges[index].kind == EdgeKind::Walkway {
        // угол квартала под **другой** дорожкой — под кольцом перехода, к
        // которому подходит тротуар: соседние точки края ушли под тротуар, а
        // угол остался, где был, и край от него к ним вылезал из-под тротуара
        // клином двора (Тула, витрина 21). Угол уходит под тротуар тоже
        return gaps()
            .filter(|&(gap, _, index)| gap > 0.0 && roads.edges[index].kind == EdgeKind::Walkway)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .and_then(|(gap, axis, index)| tuck(gap, axis, index));
    }
    if gap <= 0.0 && roads.edges[index].kind == EdgeKind::Carriageway {
        return in_bare_verge(point, outward, roads, gaps().collect(), &tuck);
    }
    if gap <= 0.0 || gap > LANDUSE_CORNER_GAP_MAX {
        return None;
    }
    // зелень только прибывает: сдвиг к дороге, уводящий край внутрь заливки,
    // не делается вовсе — так улица, идущая внутри квартала, его не сжимает
    let direction = (axis - point).try_normalize()?;
    if direction.dot(outward) > 0.0 {
        let shifted = point + direction * (gap + LANDUSE_OVERLAP);
        let corner = pull_corner(shifted, direction, outward, roads);
        // дальше обычного предела тянется только угол у двух улиц
        return match corner {
            Some(corner) => Some(corner),
            None if gap <= LANDUSE_GAP_MAX => Some(shifted),
            None => None,
        };
    }
    if gap > LANDUSE_GAP_MAX {
        return None;
    }
    tuck(gap, axis, index)
}

/// Вершина под краем проезжей части **по карте**, но, может быть, за её
/// кромкой — с той стороны, где тротуара нет (`sidewalk=no|separate`), а край
/// по карте один на обе стороны. Там лежит голая полоса или обочина до
/// дорожки, а обочина рисуется под кварталом: угол двора, обведённый вокруг
/// изгиба дорожки, торчал из её плитки тёмным штрихом (Тула, витрина 15, угол
/// Ленина и Советской), полоска двора — полосой в плитке (Тула 21, Орёл 03).
/// Такая вершина уходит к ближнему из двух краёв нарисованного: к дорожке —
/// под неё ([`pull_vertex`], `tuck`), к кромке — под асфальт, наружу от
/// заливки. `None` — вершина под нарисованным полотном какой-нибудь улицы,
/// двигать нечего.
fn in_bare_verge(
    point: Vec2,
    outward: Vec2,
    roads: &Edges,
    all: Vec<(f32, Vec2, usize)>,
    tuck: &dyn Fn(f32, Vec2, usize) -> Option<Vec2>,
) -> Option<Vec2> {
    // зазоры до нарисованного края у проезжих частей
    let kerbs: Vec<(f32, Vec2)> = all
        .iter()
        .filter(|&&(_, _, index)| roads.edges[index].kind == EdgeKind::Carriageway)
        .map(|&(_, axis, index)| (roads.edges[index].gap(point, axis), axis))
        .collect();
    // под нарисованным — асфальтом улицы, дорожкой, проездом: двигать нечего
    let covered = all
        .iter()
        .any(|&(gap, _, index)| roads.edges[index].kind != EdgeKind::Carriageway && gap <= 0.0);
    if covered || kerbs.iter().any(|&(gap, _)| gap <= 0.0) {
        return None;
    }
    let (kerb, axis) = kerbs.into_iter().min_by(|a, b| a.0.total_cmp(&b.0))?;
    let walkway = all
        .iter()
        .copied()
        .filter(|&(gap, _, index)| gap > 0.0 && roads.edges[index].kind == EdgeKind::Walkway)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((gap, walk_axis, index)) = walkway
        && gap < kerb
        && let Some(tucked) = tuck(gap, walk_axis, index)
    {
        return Some(tucked);
    }
    let direction = (axis - point).try_normalize()?;
    (direction.dot(outward) > 0.0 && kerb <= LANDUSE_GAP_MAX)
        .then(|| point + direction * (kerb + LANDUSE_OVERLAP))
}

/// Вершина угла квартала у перекрёстка, уже дотянутая под полотно одной
/// улицы (`first` — куда её тянули), — ещё и под полотно второй, если та
/// лежит поперёк, наружу от заливки и в пределах [`LANDUSE_CORNER_GAP_MAX`]. Иначе
/// между углом квартала и скруглением перекрёстка оставался треугольник голой
/// земли (пример 13, частный сектор: два угла из четырёх). Сдвиг — вдоль
/// первой улицы, так что из-под её полотна вершина не уходит.
fn pull_corner(point: Vec2, first: Vec2, outward: Vec2, roads: &Edges) -> Option<Vec2> {
    let along = first.perp();
    roads
        .lines
        .near(point, point)
        .into_iter()
        .filter_map(|index| {
            let Link { from, to, reach } = roads.edges[index].link;
            let axis = closest_on_segment(point, from, to);
            let gap = point.distance(axis) - reach;
            let direction = (axis - point).try_normalize()?;
            // поперёк первой: вдоль неё тянуть — та же улица или параллельная
            let slide = direction.dot(along);
            (gap > 0.0
                && gap <= LANDUSE_CORNER_GAP_MAX
                && direction.dot(outward) > 0.0
                && slide.abs() > 0.5)
                .then(|| {
                    (
                        gap,
                        along * slide.signum() * (gap + LANDUSE_OVERLAP) / slide.abs(),
                    )
                })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, shift)| point + shift)
}

#[cfg(test)]
mod tests;
