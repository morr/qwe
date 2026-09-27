//! Стоянка вдоль улицы: где машины встают у бордюра на проезжей части, где —
//! в **карман** (асфальт за кромкой, врезанный в тротуар), а где их нет.
//!
//! Один ответ на двух читателей: лента асфальта (`map::roads`) кладёт по нему
//! карман, ряд машин (`map::cars`) — ставит в него машины. Пока ответов было
//! бы два, машина вставала бы мимо кармана на тротуар.
//!
//! Тег — `parking:<side>` ([`KerbParking`]); без тега — правило
//! ([`kerb_parking`]): на магистрали (trunk, primary, secondary) на полосе не
//! стоят, и машины уходят в карманы, на остальных улицах стоят у бордюра.
//! Карман по тегу идёт вдоль всей стороны way между перекрёстками, по правилу
//! — редкий и короткий ([`sparse_pockets`]).

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::junctions::{self, MarkingBreaks, node_key};
use super::network::RoadNetwork;
use super::node_paint::ZEBRA_LENGTH;
use super::{is_carriageway, tapers};
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::{Break, miter_offsets};
use crate::map::osm::model::{
    Highway, KerbParking, distance_to_segment, point_in_area, polyline_length,
};
use crate::map::osm::{PolyArea, RoadLine, RoadNode, RoadNodeKind, TrafficSide};
use crate::map::seed::{Lcg, seed_from_point};

/// Ширина кармана за кромкой проезжей части, м: машина при параллельной
/// стоянке и полметра до бордюра.
pub const POCKET_WIDTH: f32 = 2.5;
/// Длина скоса на торце кармана, м.
pub const POCKET_TAPER: f32 = 6.0;
/// Отступ кармана от перекрёстка сверх его разрыва, м: у перехода и угла
/// кармана не делают. Шесть — скос начинается за зеброй по правилу, что
/// стоит от кромки узла на метр и тянется на четыре (`roads::node_paint`).
const POCKET_CLEARANCE: f32 = 6.0;
/// Самый короткий карман по полной ширине, м — две машины. Короче — не
/// карман, а зазубрина.
const POCKET_MIN: f32 = 10.0;
/// Доля кварталов (кусков стороны между перекрёстками), где правило ставит
/// карманы: в городе карман — у магазина, у остановки, у подъезда, а не вдоль
/// каждого квартала.
const RULE_BLOCK_SHARE: f32 = 0.4;
/// Длина кармана по правилу со скосами, м: три-пять машин.
const RULE_POCKET_LENGTH: (f32, f32) = (24.0, 42.0);
/// Просвет тротуара между карманами по правилу, м.
const RULE_POCKET_GAP: (f32, f32) = (30.0, 90.0);
/// Досягаемость стоянки от наружного края кармана, м: тротуар (до трёх
/// метров) и газон за ним. Стоянка ближе — машинам есть где встать и без
/// кармана, и асфальт кармана перед ней читался лишней полосой (Тула,
/// Советская у театра, 6040, 2900: стоянка в 4 м за краем кармана).
const LOT_REACH: f32 = 6.0;
/// Шаг, с которым край кармана проверяется на соседство со стоянкой, м.
const LOT_STEP: f32 = 2.0;
/// Ячейка индекса стоянок, м.
const LOT_CELL: f32 = 60.0;

/// Стоянки карты (`amenity=parking`), у которых карман не нужен: машины
/// встают на стоянку, а не у бордюра перед ней.
pub struct KerbLots<'a> {
    areas: &'a [PolyArea],
    grid: Grid<usize>,
}

impl<'a> KerbLots<'a> {
    pub fn new(areas: &'a [PolyArea]) -> Self {
        let mut grid = Grid::new(LOT_CELL);
        for (index, area) in areas.iter().enumerate() {
            let min = area.outer.iter().copied().fold(Vec2::MAX, Vec2::min);
            let max = area.outer.iter().copied().fold(Vec2::MIN, Vec2::max);
            if min.cmple(max).all() {
                grid.insert(min - LOT_REACH, max + LOT_REACH, index);
            }
        }
        Self { areas, grid }
    }

    /// Стоянка внутри или ближе [`LOT_REACH`] к точке.
    fn near(&self, point: Vec2) -> bool {
        self.grid.at(point).iter().any(|&index| {
            let area = &self.areas[index];
            point_in_area(point, area)
                || std::iter::once(&area.outer).chain(&area.holes).any(|ring| {
                    (0..ring.len()).any(|i| {
                        let next = ring[(i + 1) % ring.len()];
                        distance_to_segment(point, ring[i], next) <= LOT_REACH
                    })
                })
        })
    }

    /// Куски осевой `path`, у которых за наружным краем кармана — на `edge`
    /// от оси со стороны `side` — стоянка: `(from, to)` по длине осевой, по
    /// возрастанию.
    fn frontage(&self, path: &[Vec2], side: f32, edge: f32) -> Vec<(f32, f32)> {
        let (along, total) = arclengths(path);
        let mut found: Vec<(f32, f32)> = Vec::new();
        let steps = (total / LOT_STEP).ceil() as usize;
        for step in 0..=steps {
            let at = (step as f32 * LOT_STEP).min(total);
            let Some((point, direction)) = place_on_path(path, &along, at) else {
                break;
            };
            if !self.near(point + direction.perp() * side * edge) {
                continue;
            }
            match found.last_mut() {
                Some(run) if at - run.1 <= LOT_STEP + 1e-3 => run.1 = at,
                _ => found.push((at, at)),
            }
        }
        found
    }
}

/// Одна сторона улицы, вдоль которой может стоять ряд.
#[derive(Clone, Debug)]
pub struct Kerbside {
    /// Знак поперечной `direction.perp()`: `-1` — правая сторона по ходу
    /// точек, `1` — левая.
    pub side: f32,
    /// Стоят ли машины у бордюра на самой проезжей части.
    pub lane: bool,
    /// Карманы вдоль стороны — по длине осевой.
    pub pockets: Vec<Pocket>,
}

/// Карман: `from..to` по длине осевой, со скосами на торцах, которые не
/// упираются в торец дороги (там карман продолжается на следующем way).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pocket {
    pub from: f32,
    pub to: f32,
    pub tapers: [bool; 2],
}

impl Pocket {
    /// Часть кармана полной ширины — где в нём помещается машина.
    pub fn full(&self) -> (f32, f32) {
        let taper = |on: bool| if on { POCKET_TAPER } else { 0.0 };
        (
            self.from + taper(self.tapers[0]),
            self.to - taper(self.tapers[1]),
        )
    }
}

impl Kerbside {
    /// Карман, в полную ширину которого попадает длина `at`.
    pub fn pocket_at(&self, at: f32) -> Option<&Pocket> {
        self.pockets.iter().find(|pocket| {
            let (from, to) = pocket.full();
            (from..=to).contains(&at)
        })
    }
}

/// Улица, вдоль которой паркуются: настоящая проезжая часть — то же
/// [`is_carriageway`], которым отбирает тротуары и разметку `map::roads`, —
/// но не мост (на мосту не стоят) и не кольцо (по кольцу едут, а не
/// паркуются). Разметке мост и кольцо нужны, машинам нет, поэтому оба
/// условия здесь, а не внутри предиката.
pub fn parkable(road: &RoadLine) -> bool {
    is_carriageway(road) && !road.bridge && !road.is_roundabout()
}

/// Стоянка стороны `side` (`0` — слева по ходу точек, `1` — справа): тег или
/// правило. Правило — по классу: на магистрали машина на полосе мешает
/// движению, и её место в кармане, если есть тротуар, в который его врезать;
/// на автомагистрали и съездах развязок не стоят вовсе.
pub fn kerb_parking(road: &RoadLine, side: usize) -> KerbParking {
    match road.parking[side] {
        KerbParking::Untagged => match road.highway {
            Highway::Motorway => KerbParking::No,
            highway if highway.is_link() => KerbParking::No,
            Highway::Trunk | Highway::Primary | Highway::Secondary => {
                if road.sidewalks[side] {
                    KerbParking::Pocket
                } else {
                    KerbParking::No
                }
            }
            _ => KerbParking::Lane,
        },
        tagged => tagged,
    }
}

/// Стороны улицы, вдоль которых стоит ряд, в том порядке, в каком их
/// обходит `map::cars` (от него зависит поток ГПСЧ): односторонняя — одна,
/// у бордюра своей стороны движения; двусторонняя — правая, потом левая.
/// `path` — нарисованная осевая, `breaks` — разрывы ряда на перекрёстках
/// (`map::cars` зовёт их `junctions`), `lots` — стоянки, перед которыми
/// кармана нет: по правилу карман рядом с ними не ставится, по тегу —
/// прерывается на их длину.
pub fn kerbsides(
    road: &RoadLine,
    path: &[Vec2],
    breaks: &[Break],
    traffic: TrafficSide,
    lots: &KerbLots,
) -> Vec<Kerbside> {
    let sides: &[f32] = if road.oneway {
        &[traffic.kerb()]
    } else {
        &[-1.0, 1.0]
    };
    sides
        .iter()
        .map(|&side| {
            let index = usize::from(side < 0.0);
            let parking = kerb_parking(road, index);
            let pockets = if parking != KerbParking::Pocket {
                Vec::new()
            } else {
                let frontage = lots.frontage(path, side, road.width / 2.0 + POCKET_WIDTH);
                if road.parking[index] == KerbParking::Untagged {
                    // бухты раскладываются как без стоянок и потом снимаются:
                    // поток ГПСЧ не сдвигается, и остальные бухты квартала
                    // стоят там же, где стояли
                    let mut pockets = sparse_pockets(pockets_along(path, breaks, &[]), road, index);
                    pockets.retain(|pocket| {
                        frontage
                            .iter()
                            .all(|&(from, to)| to < pocket.from || from > pocket.to)
                    });
                    pockets
                } else {
                    pockets_along(path, breaks, &frontage)
                }
            };
            Kerbside {
                side,
                lane: parking == KerbParking::Lane,
                pockets,
            }
        })
        .collect()
}

/// Карманы по правилу — редкие и короткие: на [`RULE_BLOCK_SHARE`] кварталов,
/// по [`RULE_POCKET_LENGTH`] через [`RULE_POCKET_GAP`] внутри куска `runs`,
/// который оставили перекрёстки. Тег `parking=street_side` говорит о всей
/// стороне way, а правило ничего не знает и не выдумывает больше, чем бывает
/// на самом деле: карман во весь квартал без машин читался как лишняя полоса.
///
/// Посев — от первой точки улицы и стороны ([`seed_from_point`]): лента и ряд
/// машин спрашивают одно и то же и получают одно и то же, и пересборка ничего
/// не сдвигает.
fn sparse_pockets(runs: Vec<Pocket>, road: &RoadLine, side: usize) -> Vec<Pocket> {
    let Some(&start) = road.points.first() else {
        return Vec::new();
    };
    let side_salt = if side == 0 { 0 } else { 0x9e37_79b9 };
    let mut rng = Lcg::new(seed_from_point(start) ^ side_salt);
    let mut pockets = Vec::new();
    for run in runs {
        if rng.next_f32() >= RULE_BLOCK_SHARE {
            continue;
        }
        let mut cursor = run.from + rng.range(0.0, RULE_POCKET_GAP.0);
        loop {
            let length = rng.range(RULE_POCKET_LENGTH.0, RULE_POCKET_LENGTH.1);
            if cursor + length > run.to {
                break;
            }
            pockets.push(Pocket {
                from: cursor,
                to: cursor + length,
                tapers: [true, true],
            });
            cursor += length + rng.range(RULE_POCKET_GAP.0, RULE_POCKET_GAP.1);
        }
    }
    pockets
}

/// Разрывы ряда у бордюра по дорогам: перекрёстки (без стежков — ряд их не
/// видит), переходы и клинья между сечениями, где бордюр ближе к оси. Один
/// расчёт на ряд машин и на ленту с карманами; `taper` — длина клина на метр
/// разницы ширин (ручка `Taper`), `nodes` — точки дорог карты, из которых
/// берутся переходы.
///
/// В перекрёстках участвуют и проезды, не только улицы разметки: во двор, к
/// стоянке съезжают через ряд, и машина на съезде его перегораживала (Тула,
/// Ф. Энгельса у 3976, 1236).
pub fn row_breaks(
    roads: &[RoadLine],
    network: &RoadNetwork,
    nodes: &[RoadNode],
    taper: f32,
) -> MarkingBreaks {
    let mut found = junctions::marking_breaks(roads, is_row_participant, &[]);
    for (road, clearing) in tapers::car_clearings(roads, network, taper) {
        found.breaks[road].push(clearing);
    }
    for (road, crossing) in crossing_breaks(roads, nodes) {
        found.breaks[road].push(crossing);
    }
    found
}

/// Дорога, что рвёт ряд у бордюра, встретившись с улицей: проезжая часть или
/// проезд — всё, по чему ездят, кроме дорожек.
fn is_row_participant(road: &RoadLine) -> bool {
    road.highway != Highway::Path
}

/// До торца way ближе этого, м, — переход рвёт ряд и на продолжении улицы за
/// торцом: OSM режет улицу у перехода, и зебра у самого торца короткого way
/// (Первомайская в Туле, 3279, 2799) без этого стояла вплотную к карману
/// соседнего, а тот обрывался без скоса.
const CROSSING_SPILL: f32 = 10.0;

/// Разрывы на размеченных переходах OSM: полдлины зебры вокруг узла на его
/// дороге, а если до торца её way ближе [`CROSSING_SPILL`] — и на дороге,
/// что продолжает улицу за этим торцом, от торца на остаток.
fn crossing_breaks(roads: &[RoadLine], nodes: &[RoadNode]) -> Vec<(usize, Break)> {
    let reach = ZEBRA_LENGTH / 2.0;
    let crossings: HashMap<(i32, i32), Vec2> = nodes
        .iter()
        .filter(|node| matches!(node.kind, RoadNodeKind::Crossing { marked: true, .. }))
        .map(|node| (node_key(node.pos), node.pos))
        .collect();
    if crossings.is_empty() {
        return Vec::new();
    }
    let mut ends: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (index, road) in roads.iter().enumerate() {
        if !is_carriageway(road) || road.points.len() < 2 {
            continue;
        }
        for end in [road.points[0], road.points[road.points.len() - 1]] {
            ends.entry(node_key(end)).or_default().push(index);
        }
    }
    let mut found = Vec::new();
    for (index, road) in roads.iter().enumerate() {
        if !is_carriageway(road) {
            continue;
        }
        for &point in &road.points {
            let Some(&at) = crossings.get(&node_key(point)) else {
                continue;
            };
            found.push((index, Break { at, reach }));
            let (along, total) = arclengths(&road.points);
            let Some(station) = road
                .points
                .iter()
                .position(|&vertex| vertex == point)
                .map(|vertex| along[vertex])
            else {
                continue;
            };
            for (end, left) in [
                (road.points[0], station),
                (road.points[road.points.len() - 1], total - station),
            ] {
                if left <= 0.0 || left >= CROSSING_SPILL {
                    continue;
                }
                let key = node_key(end);
                for &next in ends.get(&key).into_iter().flatten() {
                    if next != index {
                        found.push((
                            next,
                            // клиренс ряда отмеряется от разрыва, и торцу
                            // хватает остатка полузебры за ним
                            Break {
                                at: end,
                                reach: (reach - left).max(0.0),
                            },
                        ));
                    }
                }
            }
        }
    }
    found
}

/// Карманы вдоль осевой: вся её длина, кроме окрестностей перекрёстков и
/// кусков `frontage` перед стоянками (по длине осевой; скосы — за ними).
fn pockets_along(path: &[Vec2], breaks: &[Break], frontage: &[(f32, f32)]) -> Vec<Pocket> {
    let total = polyline_length(path);
    let mut closed: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = nearest_on_path(path, found.at).map_or(0.0, |(_, along)| along);
            let reach = found.reach + POCKET_CLEARANCE;
            (at - reach, at + reach)
        })
        .chain(frontage.iter().copied())
        .collect();
    closed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut pockets = Vec::new();
    let mut cursor = 0.0;
    let mut push = |from: f32, to: f32| {
        let pocket = Pocket {
            from,
            to,
            tapers: [from > 0.0, to < total],
        };
        let (start, end) = pocket.full();
        if end - start >= POCKET_MIN {
            pockets.push(pocket);
        }
    };
    for (from, to) in closed {
        if from > cursor {
            push(cursor, from.min(total));
        }
        cursor = cursor.max(to);
    }
    if cursor < total {
        push(cursor, total);
    }
    pockets
}

/// Контур кармана со стороны `side`: внутренний край на `inner` от оси по
/// всей длине, наружный на `outer` — между скосами. Годится и для асфальта,
/// и для тротуара, отодвинутого за карман.
pub fn outline(path: &[Vec2], pocket: &Pocket, side: f32, [inner, outer]: [f32; 2]) -> Vec<Vec2> {
    let (full_from, full_to) = pocket.full();
    let edge = |from: f32, to: f32, shift: f32| -> Vec<Vec2> {
        let piece = tapers::cut(path, from, to);
        let offsets = miter_offsets(&piece, false, side * shift);
        piece
            .iter()
            .zip(offsets)
            .map(|(point, offset)| *point + offset)
            .collect()
    };
    let mut contour = edge(pocket.from, pocket.to, inner);
    contour.extend(edge(full_from, full_to, outer).into_iter().rev());
    contour
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;

    fn primary() -> RoadLine {
        RoadLine {
            highway: Highway::Primary,
            ..street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.0)
        }
    }

    #[test]
    fn a_primary_parks_in_pockets_and_a_residential_street_on_the_lane() {
        let road = primary();
        assert_eq!(kerb_parking(&road, 1), KerbParking::Pocket);
        let residential = street(road.points.clone(), 8.0);
        assert_eq!(kerb_parking(&residential, 1), KerbParking::Lane);
        let bare = RoadLine {
            sidewalks: [true, false],
            ..primary()
        };
        assert_eq!(kerb_parking(&bare, 1), KerbParking::No, "врезать некуда");
        let tagged = RoadLine {
            parking: [KerbParking::Lane, KerbParking::No],
            ..primary()
        };
        assert_eq!(kerb_parking(&tagged, 0), KerbParking::Lane);
        assert_eq!(kerb_parking(&tagged, 1), KerbParking::No);
    }

    #[test]
    fn a_tagged_pocket_stops_short_of_a_junction_and_runs_through_a_way_end() {
        let road = RoadLine {
            parking: [KerbParking::Pocket; 2],
            ..primary()
        };
        let junction = Break {
            at: Vec2::new(120.0, 0.0),
            reach: 8.0,
        };
        let sides = kerbsides(
            &road,
            &road.points,
            &[junction],
            TrafficSide::Right,
            &KerbLots::new(&[]),
        );
        assert_eq!(sides.len(), 2);
        let pockets = &sides[0].pockets;
        assert_eq!(pockets.len(), 2);
        let reach = 8.0 + POCKET_CLEARANCE;
        assert_eq!(pockets[0].from, 0.0);
        assert!((pockets[0].to - (120.0 - reach)).abs() < 1e-3);
        assert_eq!(pockets[0].tapers, [false, true], "у торца way — без скоса");
        assert_eq!(pockets[1].tapers, [true, false]);
        assert!(sides[0].pocket_at(60.0).is_some());
        assert!(sides[0].pocket_at(120.0).is_none());
        assert!(!sides[0].lane, "мимо кармана на магистрали не стоят");
    }

    /// Без тега карманы редкие и короткие: каждый со скосами и в пределах
    /// длины по правилу, между соседними — тротуар, на улице в два километра
    /// карманами занята малая часть бордюра, но не ноль; и тот же ответ на
    /// второй вызов — лента и машины его не разойдутся.
    #[test]
    fn rule_pockets_are_rare_short_and_repeatable() {
        let road = RoadLine {
            highway: Highway::Primary,
            ..street(vec![Vec2::new(3.7, 1.2), Vec2::new(2003.7, 1.2)], 14.0)
        };
        // перекрёсток каждые 150 м — кварталы
        let breaks: Vec<Break> = (1..13)
            .map(|block| Break {
                at: Vec2::new(3.7 + 150.0 * block as f32, 1.2),
                reach: 8.0,
            })
            .collect();
        let sides = kerbsides(
            &road,
            &road.points,
            &breaks,
            TrafficSide::Right,
            &KerbLots::new(&[]),
        );
        let again = kerbsides(
            &road,
            &road.points,
            &breaks,
            TrafficSide::Right,
            &KerbLots::new(&[]),
        );
        let mut covered = 0.0;
        for (side, repeat) in sides.iter().zip(&again) {
            assert_eq!(side.pockets, repeat.pockets, "посев от улицы");
            for pocket in &side.pockets {
                let length = pocket.to - pocket.from;
                assert!(
                    (RULE_POCKET_LENGTH.0..=RULE_POCKET_LENGTH.1).contains(&length),
                    "{pocket:?}"
                );
                assert_eq!(pocket.tapers, [true, true]);
                covered += length;
            }
            for pair in side.pockets.windows(2) {
                assert!(pair[1].from - pair[0].to >= RULE_POCKET_GAP.0 - 1e-3);
            }
        }
        let share = covered / (2.0 * 2000.0);
        assert!(
            share > 0.02 && share < 0.3,
            "доля бордюра в карманах {share}"
        );
    }

    /// Стоянка за тротуаром справа от `from` до `to` по x: её край в 2.5 м за
    /// наружным краем кармана улицы шириной 14.
    fn lot_on_the_right(from: f32, to: f32) -> PolyArea {
        crate::map::osm::fixture::area(
            crate::map::osm::AreaKind::Parking,
            vec![
                Vec2::new(from, -12.0),
                Vec2::new(to, -12.0),
                Vec2::new(to, -40.0),
                Vec2::new(from, -40.0),
            ],
        )
    }

    /// Без тега бухта рядом со стоянкой не ставится, остальные бухты — ни
    /// на той стороне, ни на другой — не сдвигаются.
    #[test]
    fn a_rule_pocket_is_not_laid_beside_a_parking_lot() {
        let road = RoadLine {
            highway: Highway::Primary,
            ..street(vec![Vec2::new(3.7, 1.2), Vec2::new(2003.7, 1.2)], 14.0)
        };
        // перекрёсток каждые 150 м — кварталы, и в каких-то из них бухты есть
        let breaks: Vec<Break> = (1..13)
            .map(|block| Break {
                at: Vec2::new(3.7 + 150.0 * block as f32, 1.2),
                reach: 8.0,
            })
            .collect();
        let bare = kerbsides(
            &road,
            &road.points,
            &breaks,
            TrafficSide::Right,
            &KerbLots::new(&[]),
        );
        let lots = [lot_on_the_right(0.0, 2000.0)];
        let beside = kerbsides(
            &road,
            &road.points,
            &breaks,
            TrafficSide::Right,
            &KerbLots::new(&lots),
        );
        assert!(!bare[0].pockets.is_empty(), "правило кладёт бухты справа");
        assert!(beside[0].pockets.is_empty(), "{:?}", beside[0].pockets);
        assert_eq!(
            beside[1].pockets, bare[1].pockets,
            "левую сторону стоянка не трогает"
        );

        let lots = [lot_on_the_right(1000.0, 2000.0)];
        let half = kerbsides(
            &road,
            &road.points,
            &breaks,
            TrafficSide::Right,
            &KerbLots::new(&lots),
        );
        let before: Vec<_> = bare[0].pockets.iter().filter(|p| p.to < 990.0).collect();
        let after: Vec<_> = half[0].pockets.iter().collect();
        assert_eq!(after, before, "бухты до стоянки остаются где были");
    }

    /// По тегу карман прерывается перед стоянкой на её длину, со скосами за
    /// её краями, и продолжается за ней.
    #[test]
    fn a_tagged_pocket_breaks_off_beside_a_parking_lot() {
        let road = RoadLine {
            parking: [KerbParking::Pocket; 2],
            ..primary()
        };
        let lots = [lot_on_the_right(80.0, 120.0)];
        let sides = kerbsides(
            &road,
            &road.points,
            &[],
            TrafficSide::Right,
            &KerbLots::new(&lots),
        );
        let right = &sides[0].pockets;
        assert_eq!(right.len(), 2, "{right:?}");
        assert!(right[0].to <= 80.0 && right[0].to > 60.0, "{right:?}");
        assert!(right[1].from >= 120.0 && right[1].from < 140.0, "{right:?}");
        assert_eq!(right[0].tapers, [false, true]);
        assert!(sides[0].pocket_at(100.0).is_none());
        assert_eq!(sides[1].pockets.len(), 1, "слева стоянки нет");
    }

    #[test]
    fn the_outline_lies_on_its_side_and_narrows_at_the_taper() {
        let road = primary();
        let pocket = Pocket {
            from: 20.0,
            to: 80.0,
            tapers: [true, true],
        };
        let contour = outline(&road.points, &pocket, -1.0, [7.0, 9.5]);
        assert!(contour.iter().all(|at| at.y < 0.0), "справа — к югу");
        let outer_x: Vec<f32> = contour
            .iter()
            .filter(|at| (at.y + 9.5).abs() < 1e-3)
            .map(|at| at.x)
            .collect();
        let min = outer_x.iter().copied().fold(f32::INFINITY, f32::min);
        assert!((min - (20.0 + POCKET_TAPER)).abs() < 1e-3);
    }
}
