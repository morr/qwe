//! Стоянки: асфальт с расчерченными местами и машинами на них.
//!
//! На снимке города двор со стоянкой ни с чем не спутать — прямоугольник
//! асфальта, расчерченный белыми полосками, и на половине мест что-то стоит.
//! До сих пор `amenity=parking` не запрашивался вовсе, и все эти площадки
//! были просто землёй.
//!
//! Места раскладываются **рядами вдоль длинной оси** площадки, и держится
//! раскладка одного правила: **к каждому месту машина должна доехать**.
//! Поперёк это значит `ряд — проезд — пара рядов спинами — проезд — пара
//! рядов` ([`row_bands`]): пристенный ряд выезжает в свой проезд, каждая пара —
//! в проезды по обе стороны от себя. Вдоль — что ряд не тянется через всю
//! площадку: каждые [`ROW_BLOCK`] метров его рвёт поперечный проезд
//! ([`row_places`]), и разрывы всех рядов стоят на одной линии, так что
//! площадка читается кварталами мест с проездами между ними, а не сплошной
//! штриховкой.
//!
//! Каждое место проверяется на попадание в контур, поэтому Г-образная стоянка
//! не получает мест поверх газона, а площадка, в которую не встаёт ни одно
//! место, — вовсе никаких. Маленький двор места получает, но не разметку:
//! см. [`MIN_AREA`].
//!
//! Разметка и машины делят **один** список мест: раскладка считается один раз
//! на загрузку мира в [`ParkingLayout`] (`map::spawn::spawn_map`), а краска
//! здесь и машины (`map::cars::fill_lots`) её только читают. Две независимые
//! раскладки поставили бы машину мимо её полосы.

use bevy::prelude::*;

use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::PolyArea;
use crate::map::osm::model::{point_in_area, signed_ring_area};

/// Место, м: легковая машина плюс просвет по обе стороны.
const STALL_WIDTH: f32 = 2.6;
const STALL_DEPTH: f32 = 5.2;
/// Проезд между рядами, м, — и продольный, и поперечный: это одна и та же
/// полоса асфальта, по которой машина подъезжает к месту.
const AISLE: f32 = 6.0;
/// Длина ряда между поперечными проездами, м. Ряд длиннее читается сплошной
/// штриховкой: у большой стоянки (Тула, ТРЦ «Макси», 671 × 255 м) ряд без
/// разрывов тянулся на сотни метров, тогда как на снимке такая площадка
/// разбита проездами на кварталы мест. Пятьдесят метров — это 19 мест подряд,
/// обычный квартал между проездами.
const ROW_BLOCK: f32 = 50.0;
/// Отступ разметки от края площадки, м.
const EDGE_MARGIN: f32 = 1.2;
/// Ширина полосы разметки, м, и её цвет — та же белая краска, что на улице.
const LINE_WIDTH: f32 = 0.12;
const LINE_COLOR: Color = Color::srgb(0.82, 0.82, 0.80);
/// Стоянка мельче этого пятна не получает **разметки**: две машины во дворе
/// никто не расчерчивает. Места на ней остаются — машины на них стоят
/// (`map::cars::fill_lots`), просто по неразмеченному асфальту.
const MIN_AREA: f32 = 120.0;

/// Одно место: центр и направление, в котором машина стоит.
pub struct Stall {
    pub at: Vec2,
    pub along: Vec2,
}

/// Раскладка всех стоянок карты — по списку мест на контур `MapData::parking`,
/// в том же порядке.
///
/// Кеш здесь окупается там, где у разрывов разметки (`roads::junctions`) не
/// окупился: вход раскладки — только контуры стоянок, а они меняются лишь со
/// сменой мира, тогда как слой машин пересобирается на каждое деление ползунка
/// солнца, зума и стиля дорог. Считать одно и то же по кадру — ровно та работа,
/// которой быть не должно; плата — резидентные 16 байт на место (Тула ~0.17 МБ).
#[derive(Resource, Default)]
pub struct ParkingLayout(pub Vec<Vec<Stall>>);

impl ParkingLayout {
    /// Места всех стоянок. О дорогах раскладка не знает и знать не должна:
    /// стоянка лежит поверх дорог (`Z_PARKING`) и кроет любую ленту, что
    /// заходит на неё или идёт сквозь неё, — асфальт стоянки и есть проезд.
    pub fn new(lots: &[PolyArea]) -> Self {
        Self(lots.iter().map(stalls).collect())
    }
}

/// Места стоянки — рядами вдоль её длинной оси. Пусто, если площадка мелкая
/// или вырожденная.
pub fn stalls(area: &PolyArea) -> Vec<Stall> {
    let Some(rect) = min_area_rect(&area.outer) else {
        return Vec::new();
    };
    let along = (rect[1] - rect[0]).try_normalize().unwrap_or(Vec2::X);
    let across = Vec2::new(-along.y, along.x);
    let length = (rect[1] - rect[0]).length() - 2.0 * EDGE_MARGIN;
    let width = (rect[2] - rect[1]).length() - 2.0 * EDGE_MARGIN;
    if length < STALL_WIDTH || width < STALL_DEPTH {
        return Vec::new();
    }
    let origin = rect[0] + (along + across) * EDGE_MARGIN;

    let mut stalls = Vec::new();
    let places = row_places(length);
    for band in row_bands(width) {
        let depth = band + STALL_DEPTH / 2.0;
        for place in &places {
            let at = origin + along * *place + across * depth;
            // машина стоит поперёк ряда, носом в проезд
            if fits(area, at, along, across) {
                stalls.push(Stall { at, along: across });
            }
        }
    }
    stalls
}

/// Начала рядов поперёк площадки шириной `width` (уже без отступов от краёв):
/// `ряд — проезд — пара рядов — проезд — пара рядов`.
///
/// Пристенный ряд один, а не пара, и это и есть правило «к месту можно
/// подъехать». Парами с самого края (как было) первый ряд упирается спиной в
/// пару, а носом — в край площадки: заехать в него неоткуда. Сдвиг на один ряд
/// даёт каждому ряду проезд с одной из сторон: одиночному — тот, что идёт
/// следом, паре — проезды по обе стороны от неё.
///
/// Ряд кладётся, только если влезает целиком; хвост уже площадки ряда остаётся
/// просто асфальтом. **Вторым рядом пары площадка не кончается**: спина такого
/// ряда — в соседнем ряду, а перед носом метр-другой до края, и заехать в него
/// неоткуда, — поэтому он снимается, а его полоса остаётся частью проезда.
/// Одиночный ряд у края — другое дело: к нему подъезжают с улицы, и площадка
/// в одну полосу так и размечается.
fn row_bands(width: f32) -> Vec<f32> {
    let mut bands = Vec::new();
    let mut depth = 0.0;
    // первая группа у края — один ряд, дальше пары спинами
    let mut rows = 1;
    while depth + STALL_DEPTH <= width {
        for _ in 0..rows {
            if depth + STALL_DEPTH > width {
                break;
            }
            bands.push(depth);
            depth += STALL_DEPTH;
        }
        depth += AISLE;
        rows = 2;
    }
    let tail = bands.last().is_some_and(|band| {
        let before = bands.len() > 1 && band - bands[bands.len() - 2] - STALL_DEPTH < AISLE;
        before && width - band - STALL_DEPTH < AISLE
    });
    if tail {
        bands.pop();
    }
    bands
}

/// Центры мест вдоль ряда длиной `length` (уже без отступов от краёв):
/// [`ROW_BLOCK`] метров мест, поперечный проезд, снова места.
///
/// Считается один раз на площадку и одинаково для всех её рядов — разрывы
/// обязаны стоять на одной линии, иначе вместо проезда через всю стоянку
/// получится россыпь пустых мест.
fn row_places(length: f32) -> Vec<f32> {
    let mut places = Vec::new();
    let mut place = STALL_WIDTH / 2.0;
    // сколько метров ряда уже уложено от последнего поперечного проезда
    let mut block = 0.0;
    while place + STALL_WIDTH / 2.0 <= length {
        places.push(place);
        place += STALL_WIDTH;
        block += STALL_WIDTH;
        if block >= ROW_BLOCK {
            place += AISLE;
            block = 0.0;
        }
    }
    places
}

/// Место целиком внутри контура — по четырём углам, как и коробки на кровле.
fn fits(area: &PolyArea, at: Vec2, along: Vec2, across: Vec2) -> bool {
    let half_width = along * (STALL_WIDTH / 2.0);
    let half_depth = across * (STALL_DEPTH / 2.0);
    [
        at - half_width - half_depth,
        at + half_width - half_depth,
        at + half_width + half_depth,
        at - half_width + half_depth,
    ]
    .iter()
    .all(|corner| point_in_area(*corner, area))
}

/// Разметка мест в меш: по полоске между соседними местами. Полоса, а не
/// прямоугольник места: расчерчивают именно границы.
///
/// Полоса кладётся слева от каждого места (у соседей они совпадают — это
/// дешевле, чем искать соседа) плюс **закрывающая** справа там, где кусок ряда
/// начинается: у поперечного проезда и у края контура ряд обязан быть
/// закрыт, иначе крайнее место квартала выглядит распахнутым в проезд.
pub fn push_markings(builder: &mut MeshBuilder, area: &PolyArea, stalls: &[Stall]) {
    if signed_ring_area(&area.outer).abs() < MIN_AREA {
        return;
    }
    let color = LINE_COLOR.to_linear();
    // места одного ряда идут по порядку, так что сосед справа — предыдущее
    // место списка; у первого места куска его там нет
    let mut previous: Option<Vec2> = None;
    for stall in stalls {
        let along = stall.along;
        let across = Vec2::new(-along.y, along.x);
        let half_depth = along * (STALL_DEPTH / 2.0);
        let half_line = across * (LINE_WIDTH / 2.0);
        let mut bar = |edge: Vec2| {
            builder.push_quad(
                [
                    edge - half_depth - half_line,
                    edge + half_depth - half_line,
                    edge + half_depth + half_line,
                    edge - half_depth + half_line,
                ],
                color,
            );
        };
        bar(stall.at - across * (STALL_WIDTH / 2.0));
        let neighbour = stall.at + across * STALL_WIDTH;
        if previous.is_none_or(|at| at.distance(neighbour) > LINE_WIDTH) {
            bar(stall.at + across * (STALL_WIDTH / 2.0));
        }
        previous = Some(stall.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::{AreaKind, fixture};

    fn lot(outer: Vec<Vec2>) -> PolyArea {
        fixture::area(AreaKind::Parking, outer)
    }

    fn rect(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::ZERO,
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    #[test]
    fn a_lot_is_filled_with_rows_of_stalls() {
        let lot = lot(rect(20.0, 40.0));
        let stalls = stalls(&lot);
        // 40 × 20 держит ровно пару рядов спинами: на проезд и следующую пару
        // нужно 2 · 1.2 + 2 · 5.2 + 6.0 + 5.2 = 24 м поперёк
        assert!(stalls.len() > 20, "{}", stalls.len());
        for stall in &stalls {
            assert!(point_in_area(stall.at, &lot), "{:?}", stall.at);
        }
    }

    /// Поперёк: пристенный ряд, проезд, пара рядов спинами — и на 40 × 20 из
    /// теста выше пара не влезает целиком, поэтому площадка здесь шире.
    #[test]
    fn a_single_row_at_the_edge_then_pairs_behind_an_aisle() {
        let lot = lot(rect(30.0, 40.0));
        // полосы по глубине (проекция центра на `Stall::along`)
        let mut bands: Vec<f32> = Vec::new();
        for stall in stalls(&lot) {
            let depth = stall.at.dot(stall.along);
            if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
                bands.push(depth);
            }
        }
        bands.sort_by(f32::total_cmp);
        assert_eq!(bands.len(), 3, "{bands:?}");
        let gaps: Vec<f32> = bands.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!((gaps[0] - (STALL_DEPTH + AISLE)).abs() < 0.01, "{gaps:?}");
        assert!((gaps[1] - STALL_DEPTH).abs() < 0.01, "{gaps:?}");
    }

    /// Правило раскладки: у каждого ряда с одной из сторон есть проезд шириной
    /// [`AISLE`]. Исключение одно — площадка в один ряд: подъезжают к нему с
    /// улицы, своего проезда у такой полосы нет и быть не может.
    #[test]
    fn every_row_has_an_aisle_to_drive_in_from() {
        for width in [5.0, 6.0, 12.0, 17.0, 22.0, 28.0, 40.0, 61.5, 120.0] {
            let bands = row_bands(width);
            if bands.len() < 2 {
                continue;
            }
            for (index, band) in bands.iter().enumerate() {
                let before = index
                    .checked_sub(1)
                    .map_or(*band, |previous| band - bands[previous] - STALL_DEPTH);
                let after = bands
                    .get(index + 1)
                    .map_or(width - band - STALL_DEPTH, |next| next - band - STALL_DEPTH);
                assert!(
                    before >= AISLE - 0.01 || after >= AISLE - 0.01,
                    "width {width}, band {index} of {bands:?}: {before} / {after}"
                );
            }
        }
    }

    /// Вдоль ряда: [`ROW_BLOCK`] метров мест, поперечный проезд, снова места.
    #[test]
    fn a_long_row_is_broken_by_cross_aisles() {
        let places = row_places(200.0);
        let gaps: Vec<f32> = places
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .filter(|gap| *gap > STALL_WIDTH + 0.01)
            .collect();
        assert_eq!(gaps.len(), 3, "{places:?}");
        for gap in gaps {
            assert!((gap - (STALL_WIDTH + AISLE)).abs() < 0.01);
        }
        // а короткий ряд не рвётся вовсе
        let short = row_places(40.0);
        assert!(
            short
                .windows(2)
                .all(|pair| (pair[1] - pair[0] - STALL_WIDTH).abs() < 0.01),
            "{short:?}"
        );
    }

    #[test]
    fn a_yard_corner_gets_nothing() {
        // 4 × 6 — двор на пару машин, размечать нечего
        assert!(stalls(&lot(rect(4.0, 6.0))).is_empty());
    }

    #[test]
    fn stalls_stay_out_of_a_notch() {
        // Г-образная площадка: места не должны лечь в вырез
        let ell = lot(vec![
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 12.0),
            Vec2::new(18.0, 12.0),
            Vec2::new(18.0, 26.0),
            Vec2::new(0.0, 26.0),
        ]);
        for stall in stalls(&ell) {
            assert!(point_in_area(stall.at, &ell), "{:?}", stall.at);
        }
    }
}
