//! Площадка стоянки, дотянутая до дорог, — **полигонами, а не вершинами**.
//!
//! Контур `amenity=parking` в OSM рисуют по кромке мест, а въезжают на стоянку
//! с проезда в нескольких метрах от неё, и пустырь между ними на снимке —
//! тот же асфальт. Раньше край дотягивался повершинно (`Stretch::Lot` в
//! `pull_areas_to_roads`): каждая точка контура сама искала себе дорогу, и
//! соседние уезжали на разное расстояние. Выходили зубцы земли у торцов
//! проездов, иглы асфальта в сторону дальней дороги и незалитые клинья там,
//! где кольцо заходило само за себя, — отчёт автора по стоянке ТРЦ «Макси».
//!
//! Здесь то же самое сказано одной операцией: площадка и полотна дорог рядом с
//! ней **замыкаются** (офсет наружу на радиус и обратно внутрь), и всё, что
//! замыкание добавило **между площадкой и дорогой**, становится асфальтом.
//! Карман между торцами проездов, полоса вдоль объездной, угол у въезда —
//! один и тот же случай, и скругляется он сам. Зубцам взяться неоткуда: у
//! результата нет вершин, которые двигались бы порознь.
//!
//! Само полотно дороги в контур **не входит** — кроме проезда, у которого
//! асфальт стоянки лёг по обе стороны ([`sandwiched`]): с дороги у кромки на
//! стоянку въезжают, и местам на ней не место, а проезд внутри площадки — её
//! же асфальт, и без него она распадалась бы на полосы, куда ряд не встаёт.

use bevy::math::Vec2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::stroke::offset::StrokeOffset;
use i_overlay::mesh::style::{LineCap, LineJoin, OutlineStyle, StrokeStyle};

use super::{LANDUSE_OVERLAP, SIDEWALK_CELL};
use crate::map::grid::Grid;
use crate::map::osm::model::{
    MapData, PolyArea, RoadClass, distance_to_segment, point_in_area, point_in_polygon, ring_area,
    ring_bounds, signed_ring_area,
};
use crate::map::parking::is_ground;
use crate::map::roads::{is_carriageway, sidewalk_width};

/// Радиус замыкания, м: зазор между площадкой и полотном **уже двух радиусов**
/// заливается. Четырнадцать метров — это ряд мест с проездом (5.2 + 6) и
/// запас: полоса шире — уже своя площадка, а не пустырь у кромки. Карманы между
/// торцами проездов (у «Макси» — 12 м между полотнами) сюда попадают.
const CLOSING_RADIUS: f32 = 7.0;
/// То же для **большой** стоянки ([`is_ground`]): объездная вокруг неё стоит в
/// 15–20 м от контура, и вся эта полоса на снимке — асфальт площадки.
const GROUND_CLOSING_RADIUS: f32 = 12.0;
/// Шаг, которым полотно дороги режется на куски, м: кусок берётся в замыкание,
/// только если он рядом с площадкой, и длинная улица не тянет за собой асфальт
/// на весь свой way.
const ROAD_PIECE: f32 = 3.0;
/// Скругление офсетов: длина хорды в долях радиуса (`LineJoin::Round`).
const ARC: f32 = 0.3;
/// Насколько близко вершина добавленного куска к контуру площадки или к
/// полотну дороги, чтобы считаться **касающейся** его, м.
const TOUCH: f32 = 0.25;
/// Насколько в сторону от кромки полотна ставится проба «лежит ли тут асфальт
/// стоянки» ([`sandwiched`]), м.
const BESIDE: f32 = 0.4;
/// Кусок мельче этого, м², — шум офсета, а не асфальт.
const MIN_PIECE_AREA: f32 = 1.0;
/// Дом мельче этого пятна, м², стоянку не останавливает: будка кассы посреди
/// площадки (Тула, 1435094568, 6 × 7 м) сама стоит на этом асфальте, а у
/// магазина (764017758, 36 × 51 м) за стеной чужой двор.
const KEEP_BUILDING_AREA: f32 = 100.0;
/// Полуширина полосы, которой забор вырезается из добавленного асфальта, м.
/// Шире [`TOUCH`] с запасом: кусок по ту сторону ограды, лежащей по кромке
/// площадки, обязан перестать её касаться.
const FENCE_HALF: f32 = 0.75;

type Contour = Vec<[f32; 2]>;
type Shape = Vec<Contour>;
type Rings = (Vec<Vec2>, Vec<Vec<Vec2>>);

/// Звено полотна: отрезок оси и расстояние от неё до внешнего края
/// нарисованного — у проезжей части вместе с тротуаром.
struct RoadLink {
    from: Vec2,
    to: Vec2,
    reach: f32,
    /// Проезд без тротуара: его асфальт — тот же, что у площадки, и кусок,
    /// примкнувший к нему, примкнул к стоянке ([`paved`]).
    drive: bool,
}

/// Кусок полотна рядом с площадкой: ломаная, её ширина и проезд ли это.
struct RoadPiece {
    path: Vec<Vec2>,
    width: f32,
    drive: bool,
}

/// Всё вокруг стоянок, что нужно проходу, — собранное один раз на карту.
struct Around<'a> {
    links: Vec<RoadLink>,
    road_grid: Grid<usize>,
    keep: Vec<&'a PolyArea>,
    keep_grid: Grid<usize>,
    fences: Vec<(Vec2, Vec2)>,
    fence_grid: Grid<usize>,
}

/// Дотянуть все стоянки карты до дорог. Возвращает, сколько площадок выросло.
///
/// Площадки друг от друга не зависят и считаются **по потокам**: на каждую
/// уходит с полдюжины булевых операций `i_overlay`, у которых цена — не
/// геометрия, а сам вызов (≈ 0.3 мс на площадке в четыре вершины), и на 349
/// площадках Тулы это полсекунды в один поток.
pub(super) fn pave_lots(map: &mut MapData) -> usize {
    let around = Around::of(map);
    let lots = &map.parking;
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = lots.len().div_ceil(workers).max(1);
    let results: Vec<Option<Rings>> = std::thread::scope(|scope| {
        let handles: Vec<_> = lots
            .chunks(chunk)
            .map(|lots| {
                let around = &around;
                scope.spawn(move || lots.iter().map(|lot| around.paved(lot)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("поток прохода стоянок упал"))
            .collect()
    });
    drop(around);

    let mut grown = 0;
    for (lot, result) in map.parking.iter_mut().zip(results) {
        if let Some((outer, holes)) = result {
            lot.outer = outer;
            lot.holes = holes;
            grown += 1;
        }
    }
    grown
}

impl<'a> Around<'a> {
    fn of(map: &'a MapData) -> Self {
        // дорожки не в счёт: асфальт стоянки сходится с асфальтом, а тропинка
        // вдоль кромки — не то, с чего на неё въезжают
        let links: Vec<RoadLink> = map
            .roads
            .iter()
            .filter(|road| road.class == RoadClass::Street && !road.bridge && !road.passage)
            .flat_map(|road| {
                let sidewalk = if is_carriageway(road) {
                    sidewalk_width(road.width).unwrap_or_default()
                } else {
                    0.0
                };
                let reach = road.width / 2.0 + sidewalk;
                let drive = !is_carriageway(road);
                road.points.windows(2).map(move |pair| RoadLink {
                    from: pair[0],
                    to: pair[1],
                    reach,
                    drive,
                })
            })
            .collect();
        let mut road_grid: Grid<usize> = Grid::new(SIDEWALK_CELL);
        for (index, link) in links.iter().enumerate() {
            let pad = link.reach + 2.0 * GROUND_CLOSING_RADIUS;
            road_grid.insert_segment(link.from, link.to, pad, index);
        }

        let keep: Vec<&PolyArea> = map
            .buildings
            .iter()
            .filter(|area| ring_area(&area.outer) >= KEEP_BUILDING_AREA)
            .chain(
                [&map.parks, &map.woods, &map.grass, &map.sand, &map.water]
                    .into_iter()
                    .flatten(),
            )
            .collect();
        let mut keep_grid: Grid<usize> = Grid::new(SIDEWALK_CELL);
        for (index, area) in keep.iter().enumerate() {
            let (low, high) = ring_bounds(&area.outer);
            keep_grid.insert(low, high, index);
        }
        let fences: Vec<(Vec2, Vec2)> = map
            .fences
            .iter()
            .flat_map(|fence| fence.points.windows(2).map(|pair| (pair[0], pair[1])))
            .collect();
        let mut fence_grid: Grid<usize> = Grid::new(SIDEWALK_CELL);
        for (index, (from, to)) in fences.iter().enumerate() {
            fence_grid.insert_segment(*from, *to, FENCE_HALF, index);
        }
        Self {
            links,
            road_grid,
            keep,
            keep_grid,
            fences,
            fence_grid,
        }
    }

    /// Контур площадки вместе с асфальтом, добавленным между ней и дорогами;
    /// `None` — добавлять нечего.
    fn paved(&self, lot: &PolyArea) -> Option<Rings> {
        let radius = if is_ground(lot) {
            GROUND_CLOSING_RADIUS
        } else {
            CLOSING_RADIUS
        };
        let (low, high) = ring_bounds(&lot.outer);
        let links: Vec<&RoadLink> = self
            .road_grid
            .near(low, high)
            .into_iter()
            .map(|index| &self.links[index])
            .collect();
        let pieces = road_pieces(lot, &links, 2.0 * radius);
        if pieces.is_empty() {
            return None;
        }
        let lot_contours = area_contours(lot);
        let mut base: Vec<Contour> = lot_contours.clone();
        for piece in &pieces {
            base.extend(stroke(&piece.path, piece.width).into_iter().flatten());
        }
        let base: Vec<Shape> = base.simplify_shape(FillRule::NonZero);

        // замыкание: наружу на радиус и обратно; всё уже двух радиусов затянуто
        let round = LineJoin::Round(ARC);
        let closed: Vec<Shape> = base
            .outline(&OutlineStyle::new(radius).line_join(round.clone()))
            .outline(&OutlineStyle::new(-radius).line_join(round));

        // асфальтом становится только то, что легло **между площадкой и
        // дорогой**: выемка в самом контуре (газон в углу, Г-образная площадка)
        // дороги не касается, клин между двумя улицами не касается площадки.
        // Проезд без тротуара — сторона площадки: его асфальт тот же, и полоса
        // между ним и улицей за ним — такой же пустырь у стоянки, как полоса
        // перед ним
        let between = |shape: &Shape| {
            let Some(outer) = shape.first() else {
                return false;
            };
            let ring: Vec<Vec2> = outer.iter().copied().map(Vec2::from_array).collect();
            let on_road = |point: &Vec2, drive: bool| {
                links.iter().any(|link| {
                    (link.drive || !drive)
                        && distance_to_segment(*point, link.from, link.to) <= link.reach + TOUCH
                })
            };
            ring_area(&ring) >= MIN_PIECE_AREA
                && ring
                    .iter()
                    .any(|point| touches_lot(*point, lot) || on_road(point, true))
                && ring.iter().any(|point| on_road(point, false))
        };
        let mut kept: Vec<Shape> = closed
            .overlay(&base, OverlayRule::Difference, FillRule::NonZero)
            .into_iter()
            .filter(between)
            .collect();
        // препятствия вычитаются из уже отобранного — и отбор повторяется:
        // забор по кромке площадки отрезает кусок от неё, дом — от дороги
        let obstacles = self.obstacles(shapes_bounds(&kept)?);
        if !obstacles.is_empty() {
            kept = kept
                .overlay(
                    &obstacles.simplify_shape(FillRule::NonZero),
                    OverlayRule::Difference,
                    FillRule::NonZero,
                )
                .into_iter()
                .filter(between)
                .collect();
        }
        if kept.is_empty() {
            return None;
        }

        let mut whole: Vec<Contour> = lot_contours;
        for piece in pieces.iter().filter(|piece| piece.drive) {
            // торец срезан: отрезок кончается там, где кончился асфальт по
            // сторонам, и скруглённый торец вылез бы за него — на тротуар улицы
            for run in sandwiched(piece, lot, &kept) {
                let path: Contour = run.iter().map(Vec2::to_array).collect();
                let style = StrokeStyle::new(piece.width).line_join(LineJoin::Round(ARC));
                whole.extend(path.stroke(style, false).into_iter().flatten());
            }
        }
        // край заводится под полотно: лента рисуется по сглаженной оси, а зазор
        // мерился по сырым точкам OSM, и без запаса на повороте осталась бы
        // щель (срез, а не дуга: на полуметре дуга — десяток вершин ни за что)
        let kept: Vec<Shape> =
            kept.outline(&OutlineStyle::new(LANDUSE_OVERLAP).line_join(LineJoin::Bevel));
        whole.extend(kept.into_iter().flatten());
        // кусок мог остаться отрезанным от площадки — остаётся самая большая
        // фигура, то есть она сама
        let shape = whole
            .simplify_shape(FillRule::NonZero)
            .into_iter()
            .max_by(|left, right| shape_area(left).total_cmp(&shape_area(right)))?;
        let mut rings = shape.into_iter().map(|contour| {
            contour
                .into_iter()
                .map(Vec2::from_array)
                .collect::<Vec<Vec2>>()
        });
        let outer = rings.next().filter(|ring| ring.len() >= 3)?;
        let holes: Vec<Vec<Vec2>> = rings
            .filter(|ring| ring.len() >= 3 && ring_area(ring) >= MIN_PIECE_AREA)
            .collect();
        Some((outer, holes))
    }

    /// Через что асфальт не переползает, в пределах габарита: дома, зелень с
    /// водой — всё, что стоянка, лежащая выше них, закрасила бы, — и заборы,
    /// полосой в [`FENCE_HALF`] по обе стороны.
    fn obstacles(&self, (low, high): (Vec2, Vec2)) -> Vec<Contour> {
        self.keep_grid
            .near(low, high)
            .into_iter()
            .map(|index| self.keep[index])
            .filter(|area| {
                let (area_low, area_high) = ring_bounds(&area.outer);
                area_low.cmple(high).all() && area_high.cmpge(low).all()
            })
            .flat_map(area_contours)
            .chain(
                self.fence_grid
                    .near(low, high)
                    .into_iter()
                    .flat_map(|index| {
                        let (from, to) = self.fences[index];
                        stroke(&[from, to], 2.0 * FENCE_HALF)
                    })
                    .flatten(),
            )
            .collect()
    }
}

/// Куски полотна рядом с площадкой.
///
/// Звено режется на отрезки по [`ROAD_PIECE`], и отрезок берётся, если от его
/// середины до площадки (считая от края полотна) не больше `gap`. Подряд
/// идущие взятые отрезки склеены в одну ломаную — **и через стык звеньев
/// одной дороги**: звенья приходят по порядку, и полоса на ломаную выходит
/// одна, а не по обрубку со скруглёнными торцами на каждое звено.
fn road_pieces(lot: &PolyArea, links: &[&RoadLink], gap: f32) -> Vec<RoadPiece> {
    let (low, high) = ring_bounds(&lot.outer);
    let mut pieces: Vec<RoadPiece> = Vec::new();
    let mut run: Vec<Vec2> = Vec::new();
    let mut last: Option<&RoadLink> = None;
    let mut flush = |run: &mut Vec<Vec2>, link: Option<&RoadLink>| {
        if let (false, Some(link)) = (run.is_empty(), link) {
            pieces.push(RoadPiece {
                path: std::mem::take(run),
                width: 2.0 * link.reach,
                drive: link.drive,
            });
        }
    };
    for link in links {
        let continues = last.is_some_and(|last| last.to == link.from && last.reach == link.reach);
        if !continues {
            flush(&mut run, last);
        }
        last = Some(*link);
        let reach = Vec2::splat(gap + link.reach);
        let steps = (link.from.distance(link.to) / ROAD_PIECE).ceil().max(1.0) as usize;
        for step in 0..steps {
            let from = link.from.lerp(link.to, step as f32 / steps as f32);
            let to = link.from.lerp(link.to, (step + 1) as f32 / steps as f32);
            let middle = from.midpoint(to);
            let near = middle.cmpge(low - reach).all()
                && middle.cmple(high + reach).all()
                && distance_to_lot(middle, lot) - link.reach <= gap;
            if !near {
                flush(&mut run, last);
                continue;
            }
            if run.is_empty() {
                run.push(from);
            } else if step > 0 && run.len() >= 2 {
                // прямое звено остаётся одним отрезком: лишние вершины на
                // прямой офсету ни к чему
                run.pop();
            }
            run.push(to);
        }
    }
    flush(&mut run, last);
    pieces
}

/// Отрезки проезда, у которых асфальт стоянки — сама площадка или добавленный
/// кусок — лежит **по обе стороны** полотна.
fn sandwiched(piece: &RoadPiece, lot: &PolyArea, kept: &[Shape]) -> Vec<Vec<Vec2>> {
    let asphalt = |point: Vec2| {
        point_in_area(point, lot)
            || kept.iter().any(|shape| {
                let mut rings = shape.iter().map(|contour| {
                    contour
                        .iter()
                        .copied()
                        .map(Vec2::from_array)
                        .collect::<Vec<Vec2>>()
                });
                rings
                    .next()
                    .is_some_and(|outer| point_in_polygon(point, &outer))
                    && !rings.any(|hole| point_in_polygon(point, &hole))
            })
    };
    let beside = piece.width / 2.0 + BESIDE;
    let mut runs: Vec<Vec<Vec2>> = Vec::new();
    let mut run: Vec<Vec2> = Vec::new();
    for pair in piece.path.windows(2) {
        let Some(normal) = (pair[1] - pair[0]).perp().try_normalize() else {
            continue;
        };
        let steps = (pair[0].distance(pair[1]) / ROAD_PIECE).ceil().max(1.0) as usize;
        for step in 0..steps {
            let from = pair[0].lerp(pair[1], step as f32 / steps as f32);
            let to = pair[0].lerp(pair[1], (step + 1) as f32 / steps as f32);
            let middle = from.midpoint(to);
            if asphalt(middle + normal * beside) && asphalt(middle - normal * beside) {
                if run.is_empty() {
                    run.push(from);
                }
                run.push(to);
            } else if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// Расстояние от точки до площадки: ноль внутри, иначе до ближайшего ребра.
fn distance_to_lot(point: Vec2, lot: &PolyArea) -> f32 {
    if point_in_area(point, lot) {
        return 0.0;
    }
    distance_to_rings(point, lot)
}

fn distance_to_rings(point: Vec2, lot: &PolyArea) -> f32 {
    std::iter::once(&lot.outer)
        .chain(&lot.holes)
        .flat_map(|ring| {
            (0..ring.len()).map(move |index| (ring[index], ring[(index + 1) % ring.len()]))
        })
        .map(|(from, to)| distance_to_segment(point, from, to))
        .fold(f32::INFINITY, f32::min)
}

fn touches_lot(point: Vec2, lot: &PolyArea) -> bool {
    distance_to_rings(point, lot) <= TOUCH
}

/// Кольца площади в закрутке, которой ждёт `i_overlay`: внешнее против часовой,
/// дырки по ней — в OSM порядок точек какой придётся.
fn area_contours(area: &PolyArea) -> Vec<Contour> {
    std::iter::once(oriented(&area.outer, true))
        .chain(area.holes.iter().map(|hole| oriented(hole, false)))
        .collect()
}

fn oriented(ring: &[Vec2], counterclockwise: bool) -> Contour {
    let mut points: Contour = ring.iter().map(Vec2::to_array).collect();
    if (signed_ring_area(ring) > 0.0) != counterclockwise {
        points.reverse();
    }
    points
}

/// Полоса вокруг ломаной — со скруглёнными стыками и торцами.
fn stroke(path: &[Vec2], width: f32) -> Vec<Shape> {
    let path: Contour = path.iter().map(Vec2::to_array).collect();
    let style = StrokeStyle::new(width)
        .line_join(LineJoin::Round(ARC))
        .start_cap(LineCap::Round(ARC))
        .end_cap(LineCap::Round(ARC));
    path.stroke(style, false)
}

fn contour_bounds(contour: &Contour) -> (Vec2, Vec2) {
    contour.iter().fold(
        (Vec2::INFINITY, Vec2::NEG_INFINITY),
        |(low, high), point| {
            let point = Vec2::from_array(*point);
            (low.min(point), high.max(point))
        },
    )
}

/// Габарит фигур по их внешним кольцам; `None` — фигур нет.
fn shapes_bounds(shapes: &[Shape]) -> Option<(Vec2, Vec2)> {
    shapes
        .iter()
        .filter_map(|shape| shape.first())
        .map(contour_bounds)
        .reduce(|(low, high), (next_low, next_high)| (low.min(next_low), high.max(next_high)))
}

fn shape_area(shape: &Shape) -> f32 {
    shape.first().map_or(0.0, |outer| {
        let ring: Vec<Vec2> = outer.iter().copied().map(Vec2::from_array).collect();
        ring_area(&ring)
    })
}
