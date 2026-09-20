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
/// Отмостка: на сколько асфальт стоянки не доходит до стены **любого** дома, м.
/// Контур в OSM сплошь и рядом рисуют внахлёст с домом или впритык к нему, а
/// замыкание затягивает и щель между ними, — и площадка лезла под стену, места
/// вставали в дом (отчёт автора: зелёный корпус и будка кассы у «Макси»).
const BUILDING_APRON: f32 = 1.0;
/// Обрезок площадки мельче этого, м², оставшийся после вычитания домов, —
/// не стоянка.
const MIN_LOT_PART: f32 = 30.0;

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
    /// Все дома, и мелкие тоже: под стену не лезет ничей асфальт
    /// ([`BUILDING_APRON`]).
    buildings: &'a [PolyArea],
    building_grid: Grid<usize>,
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
    let results: Vec<Option<Vec<Rings>>> = std::thread::scope(|scope| {
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
    // дом поперёк площадки режет её надвое: первая часть остаётся на месте
    // площадки, остальные встают в конец списка такими же стоянками
    let mut parts: Vec<PolyArea> = Vec::new();
    for (lot, result) in map.parking.iter_mut().zip(results) {
        let Some(rings) = result else {
            continue;
        };
        grown += 1;
        let mut rings = rings.into_iter();
        let Some((outer, holes)) = rings.next() else {
            continue;
        };
        lot.outer = outer;
        lot.holes = holes;
        parts.extend(rings.map(|(outer, holes)| PolyArea {
            outer,
            holes,
            ..lot.clone()
        }));
    }
    map.parking.extend(parts);
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
        let mut building_grid: Grid<usize> = Grid::new(SIDEWALK_CELL);
        for (index, area) in map.buildings.iter().enumerate() {
            let (low, high) = ring_bounds(&area.outer);
            building_grid.insert(low - BUILDING_APRON, high + BUILDING_APRON, index);
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
            buildings: &map.buildings,
            building_grid,
            fences,
            fence_grid,
        }
    }

    /// Площадка, какой она рисуется: дотянутая до дорог ([`Around::grown`]) и
    /// отступившая от домов на [`BUILDING_APRON`]. Частей бывает несколько —
    /// дом поперёк площадки режет её; `None` — ничего не изменилось.
    fn paved(&self, lot: &PolyArea) -> Option<Vec<Rings>> {
        let grown = self.grown(lot);
        let shape: Shape = grown.clone().unwrap_or_else(|| area_contours(lot));
        let walls = self.walls(&shape);
        if walls.is_empty() {
            return grown.map(|shape| vec![rings_of(shape)]);
        }
        let before = shape_area(&shape) - holes_area(&shape);
        let mut parts: Vec<Shape> = vec![shape]
            .overlay(&walls, OverlayRule::Difference, FillRule::NonZero)
            .into_iter()
            .filter(|part| shape_area(part) >= MIN_LOT_PART)
            .collect();
        // стена рядом, но асфальта не задела — площадка та же, что была
        let after: f32 = parts
            .iter()
            .map(|part| shape_area(part) - holes_area(part))
            .sum();
        if parts.is_empty() || (grown.is_none() && (before - after).abs() < MIN_PIECE_AREA) {
            return grown.map(|shape| vec![rings_of(shape)]);
        }
        parts.sort_by(|left, right| shape_area(right).total_cmp(&shape_area(left)));
        Some(parts.into_iter().map(rings_of).collect())
    }

    /// Дома у площадки, раздутые на отмостку, — одной фигурой на вычитание.
    /// Пусто, если ни один к ней не подходит.
    fn walls(&self, shape: &Shape) -> Vec<Shape> {
        let Some((low, high)) = shape.first().map(contour_bounds) else {
            return Vec::new();
        };
        let rings: Vec<Vec<Vec2>> = shape
            .iter()
            .map(|contour| contour.iter().copied().map(Vec2::from_array).collect())
            .collect();
        let contours: Vec<Contour> = self
            .building_grid
            .near(low, high)
            .into_iter()
            .map(|index| &self.buildings[index])
            .filter(|building| {
                let (area_low, area_high) = ring_bounds(&building.outer);
                area_low.cmple(high + BUILDING_APRON).all()
                    && area_high.cmpge(low - BUILDING_APRON).all()
                    && comes_near(&rings, building)
            })
            .flat_map(area_contours)
            .collect();
        if contours.is_empty() {
            return Vec::new();
        }
        contours
            .simplify_shape(FillRule::NonZero)
            .outline(&OutlineStyle::new(BUILDING_APRON).line_join(LineJoin::Round(ARC)))
    }

    /// Контур площадки вместе с асфальтом, добавленным между ней и дорогами;
    /// `None` — добавлять нечего.
    fn grown(&self, lot: &PolyArea) -> Option<Shape> {
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
        let bands: Vec<Contour> = pieces
            .iter()
            .flat_map(|piece| stroke(&piece.path, piece.width))
            .flatten()
            .collect();
        let mut base: Vec<Contour> = lot_contours.clone();
        base.extend(bands.iter().cloned());
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
        // щель (срез, а не дуга: на полуметре дуга — десяток вершин ни за что).
        // Заводится **только под полотно**: раздутый во все стороны, кусок
        // вылезал на полметра и там, где его край — свободный (дуга замыкания,
        // стена дома, газон), и у стыка с кромкой самой площадки выходила
        // ступенька — «рывки» на границе из отчёта автора
        let under: Vec<Shape> = kept
            .outline(&OutlineStyle::new(LANDUSE_OVERLAP).line_join(LineJoin::Bevel))
            .overlay(&bands, OverlayRule::Intersect, FillRule::NonZero);
        whole.extend(kept.into_iter().flatten());
        whole.extend(under.into_iter().flatten());
        // кусок мог остаться отрезанным от площадки — остаётся самая большая
        // фигура, то есть она сама
        whole
            .simplify_shape(FillRule::NonZero)
            .into_iter()
            .max_by(|left, right| shape_area(left).total_cmp(&shape_area(right)))
            .filter(|shape| shape.first().is_some_and(|outer| outer.len() >= 3))
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

/// Кольца фигуры: внешнее и дырки, без шума офсета.
fn rings_of(shape: Shape) -> Rings {
    let mut rings = shape.into_iter().map(|contour| {
        contour
            .into_iter()
            .map(Vec2::from_array)
            .collect::<Vec<Vec2>>()
    });
    let outer = rings.next().unwrap_or_default();
    let holes = rings
        .filter(|ring| ring.len() >= 3 && ring_area(ring) >= MIN_PIECE_AREA)
        .collect();
    (outer, holes)
}

/// Подходит ли дом к площадке ближе отмостки: вершина одного внутри другого
/// или ближе [`BUILDING_APRON`] к его ребру. Кратчайшее расстояние между двумя
/// многоугольниками всегда держится на чьей-то вершине.
fn comes_near(rings: &[Vec<Vec2>], building: &PolyArea) -> bool {
    let Some((outer, holes)) = rings.split_first() else {
        return false;
    };
    fn edges(ring: &[Vec2]) -> impl Iterator<Item = (Vec2, Vec2)> + '_ {
        (0..ring.len()).map(move |index| (ring[index], ring[(index + 1) % ring.len()]))
    }
    // у замощённой площадки за тысячу рёбер — в счёт идут те, что у самого дома
    let (low, high) = ring_bounds(&building.outer);
    let (low, high) = (low - BUILDING_APRON, high + BUILDING_APRON);
    let lot_edges: Vec<(Vec2, Vec2)> = rings
        .iter()
        .flat_map(|ring| edges(ring))
        .filter(|(from, to)| from.min(*to).cmple(high).all() && from.max(*to).cmpge(low).all())
        .collect();
    let wall_edges: Vec<(Vec2, Vec2)> = std::iter::once(&building.outer)
        .chain(&building.holes)
        .flat_map(|ring| edges(ring))
        .collect();
    let close = |point: Vec2, edges: &[(Vec2, Vec2)]| {
        edges
            .iter()
            .any(|(from, to)| distance_to_segment(point, *from, *to) <= BUILDING_APRON)
    };
    let in_lot = |point: Vec2| {
        point_in_polygon(point, outer) && !holes.iter().any(|hole| point_in_polygon(point, hole))
    };
    wall_edges
        .iter()
        .any(|(point, _)| in_lot(*point) || close(*point, &lot_edges))
        || lot_edges
            .iter()
            .any(|(point, _)| point_in_area(*point, building) || close(*point, &wall_edges))
}

fn holes_area(shape: &Shape) -> f32 {
    shape
        .iter()
        .skip(1)
        .map(|hole| {
            let ring: Vec<Vec2> = hole.iter().copied().map(Vec2::from_array).collect();
            ring_area(&ring)
        })
        .sum()
}

fn shape_area(shape: &Shape) -> f32 {
    shape.first().map_or(0.0, |outer| {
        let ring: Vec<Vec2> = outer.iter().copied().map(Vec2::from_array).collect();
        ring_area(&ring)
    })
}
