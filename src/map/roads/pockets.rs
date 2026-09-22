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

use bevy::prelude::*;

use super::junctions::{self, MarkingBreaks};
use super::network::RoadNetwork;
use super::{is_carriageway, tapers};
use crate::map::meshing::Break;
use crate::map::osm::model::{Highway, KerbParking};
use crate::map::osm::{RoadLine, TrafficSide};
use crate::map::seed::{Lcg, seed_from_point};

/// Ширина кармана за кромкой проезжей части, м: машина при параллельной
/// стоянке и полметра до бордюра.
pub const POCKET_WIDTH: f32 = 2.5;
/// Длина скоса на торце кармана, м.
pub const POCKET_TAPER: f32 = 6.0;
/// Отступ кармана от перекрёстка сверх его разрыва, м: у перехода и угла
/// кармана не делают.
const POCKET_CLEARANCE: f32 = 4.0;
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
/// (`map::cars` зовёт их `junctions`).
pub fn kerbsides(
    road: &RoadLine,
    path: &[Vec2],
    breaks: &[Break],
    traffic: TrafficSide,
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
            } else if road.parking[index] == KerbParking::Untagged {
                sparse_pockets(pockets_along(path, breaks), road, index)
            } else {
                pockets_along(path, breaks)
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

/// Разрывы ряда у бордюра по дорогам: перекрёстки проезжих частей (без
/// стежков — ряд их не видит) и клинья между сечениями, где бордюр ближе к
/// оси. Один расчёт на ряд машин и на ленту с карманами; `taper` — длина
/// клина на метр разницы ширин (ручка `Taper`).
pub fn row_breaks(roads: &[RoadLine], network: &RoadNetwork, taper: f32) -> MarkingBreaks {
    let mut found = junctions::marking_breaks(roads, is_carriageway, &[]);
    for (road, clearing) in tapers::car_clearings(roads, network, taper) {
        found.breaks[road].push(clearing);
    }
    found
}

/// Карманы вдоль осевой: вся её длина, кроме окрестностей перекрёстков.
fn pockets_along(path: &[Vec2], breaks: &[Break]) -> Vec<Pocket> {
    let total: f32 = path.windows(2).map(|link| link[0].distance(link[1])).sum();
    let mut closed: Vec<(f32, f32)> = breaks
        .iter()
        .map(|found| {
            let at = station(path, found.at);
            let reach = found.reach + POCKET_CLEARANCE;
            (at - reach, at + reach)
        })
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

/// Длина осевой до ближайшей к `point` её точки.
fn station(path: &[Vec2], point: Vec2) -> f32 {
    let mut best = (f32::INFINITY, 0.0);
    let mut run = 0.0;
    for link in path.windows(2) {
        let span = link[1] - link[0];
        let length = span.length();
        let t = ((point - link[0]).dot(span) / span.length_squared().max(1e-9)).clamp(0.0, 1.0);
        let distance = (link[0] + span * t).distance(point);
        if distance < best.0 {
            best = (distance, run + t * length);
        }
        run += length;
    }
    best.1
}

/// Контур кармана со стороны `side`: внутренний край на `inner` от оси по
/// всей длине, наружный на `outer` — между скосами. Годится и для асфальта,
/// и для тротуара, отодвинутого за карман.
pub fn outline(path: &[Vec2], pocket: &Pocket, side: f32, [inner, outer]: [f32; 2]) -> Vec<Vec2> {
    let (full_from, full_to) = pocket.full();
    let edge = |from: f32, to: f32, shift: f32| -> Vec<Vec2> {
        let piece = tapers::cut(path, from, to);
        let offsets = crate::map::meshing::miter_offsets(&piece, false, side * shift);
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
        let sides = kerbsides(&road, &road.points, &[junction], TrafficSide::Right);
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
        let sides = kerbsides(&road, &road.points, &breaks, TrafficSide::Right);
        let again = kerbsides(&road, &road.points, &breaks, TrafficSide::Right);
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
