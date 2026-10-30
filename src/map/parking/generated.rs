//! Выдуманная раскладка — для площадок без проездов OSM: ряды вдоль самой
//! длинной стороны контура, парковочные карманы и ряды вдоль бордюра.

use bevy::prelude::*;

use super::{
    AISLE, EDGE_MARGIN, MIN_ROW_RUN, Outline, PAIR_AISLE, PARALLEL_LENGTH, PARALLEL_WIDTH, Placed,
    ROW_BLOCK, STALL_DEPTH, STALL_DEPTH_MIN, STALL_WIDTH, STRIP_MARGIN, Stall, Surroundings,
    axis_bounds, fits, fits_box, reachable,
};
use crate::map::meshing::{equivalent_width, ring_perimeter};
use crate::map::osm::model::{AreaKind, LotKind, PolyArea, TrafficSide, signed_ring_area};

/// Выдуманная раскладка: ряды вдоль самой длинной стороны контура. Полоса в
/// один ряд — ряд поперёк неё ([`pocket_rows`]), а карман вдоль улицы тоньше
/// места поперёк — ряд вдоль бордюра ([`parallel_rows`]).
pub(super) fn generated_rows(
    outline: &Outline,
    through: &Surroundings,
    traffic: TrafficSide,
) -> Vec<Stall> {
    let area = outline.area;
    let Some(along) = area.longest_side() else {
        return Vec::new();
    };
    if let Some(depth) = pocket_depth(area) {
        return pocket_rows(outline, through, depth);
    }
    if area.kind == AreaKind::Parking(LotKind::Kerbside)
        && thickness(&area.outer) - 2.0 * STRIP_MARGIN < STALL_DEPTH_MIN
    {
        return parallel_rows(outline, through, traffic);
    }
    let across = Vec2::new(-along.y, along.x);
    let (low, high) = axis_bounds(&area.outer, along, across);
    let length = (high.x - low.x) - 2.0 * EDGE_MARGIN;
    let width = (high.y - low.y) - 2.0 * EDGE_MARGIN;
    if length < STALL_WIDTH || width < STALL_DEPTH {
        return Vec::new();
    }
    let depth = STALL_DEPTH;
    let origin = along * (low.x + EDGE_MARGIN) + across * (low.y + EDGE_MARGIN);

    let mut stalls = Vec::new();
    let places = row_places(length);
    let bands = row_bands(width);
    // площадку в один ряд `reachable` не спрашивают: к ней подъезжают с улицы,
    // а улицы контур площадки не видит — то же исключение, что у
    // `every_row_has_an_aisle_to_drive_in_from`
    let single_row = bands.len() == 1;
    let mut noses = row_noses(&bands, width);
    if single_row {
        // единственный ряд смотрит на улицу, с которой в него заезжают, — если
        // она нашлась рядом с серединой площадки
        let centre = origin + along * (length / 2.0) + across * (depth / 2.0);
        let reach = depth / 2.0 + PAIR_AISLE / 2.0;
        if let Some(sign) = [1.0f32, -1.0]
            .into_iter()
            .find(|sign| through.paved(centre + across * (sign * reach)))
        {
            noses = vec![sign];
        }
    }
    for (band, sign) in bands.iter().zip(&noses) {
        let middle = band + depth / 2.0;
        // машина стоит поперёк ряда, носом в свой проезд
        let nose = across * *sign;
        let mut row: Vec<Stall> = Vec::new();
        // кусок ряда — места подряд; обрывок короче [`MIN_ROW_RUN`] не
        // размечается вовсе, ровно как у ряда по проезду (`Frame::push_row`).
        // Разрыв по `row_places` (поперечный проезд) кусок намеренно не рвёт
        let mut run: Vec<Stall> = Vec::new();
        for place in &places {
            let at = origin + along * *place + across * middle;
            if !through.cover(at, nose, depth)
                && fits(outline, at, along, across, depth)
                && (single_row || reachable(outline, through, at, nose, depth))
            {
                run.push(Stall {
                    at,
                    along: nose,
                    depth,
                    parallel: false,
                });
            } else {
                if run.len() >= MIN_ROW_RUN {
                    row.append(&mut run);
                }
                run.clear();
            }
        }
        if run.len() >= MIN_ROW_RUN {
            row.append(&mut run);
        }
        // порядок мест в списке — тот, в котором их ждёт разметка
        // (`push_markings`): вдоль `-perp(Stall::along)`, а это при носе назад
        // убывающий `place`
        if *sign < 0.0 {
            row.reverse();
        }
        stalls.append(&mut row);
    }
    stalls
}

/// Глубина места в **парковочном кармане** — полосе в один ряд вдоль улицы
/// (Тула, way 702257069, 354 × 5.7 м); `None` — площадка не карман.
///
/// Карман узнаётся по толщине, `2 · площадь / периметр` (у длинной полосы это
/// её ширина): места с [`EDGE_MARGIN`] по обе стороны в такую полосу не
/// встаёт, а без мест это просто тёмная лента у тротуара. Карман размечают от
/// бордюра до проезжей части, так что отступ тут — только зазор на счёт
/// ([`STRIP_MARGIN`]), а место берёт глубину самой полосы.
fn pocket_depth(area: &PolyArea) -> Option<f32> {
    let depth = thickness(&area.outer) - 2.0 * STRIP_MARGIN;
    (STALL_DEPTH_MIN..STALL_DEPTH + 2.0 * (EDGE_MARGIN - STRIP_MARGIN))
        .contains(&depth)
        .then_some(depth.min(STALL_DEPTH))
}

/// Толщина полосы, `2 · площадь / периметр`: у длинной полосы это её ширина.
fn thickness(ring: &[Vec2]) -> f32 {
    2.0 * signed_ring_area(ring).abs() / ring_perimeter(ring)
}

/// Ширина полосы — короткая сторона прямоугольника с той же площадью и тем же
/// периметром ([`crate::map::meshing::equivalent_width`]). [`thickness`] её
/// занижает на торцах (у 4 × 60 м — 3.75), а ряд
/// вдоль бордюра встаёт по середине полосы, и на четверть метра мимо — это
/// машина, торчащая из кармана.
fn strip_width(ring: &[Vec2]) -> f32 {
    equivalent_width(ring_perimeter(ring), signed_ring_area(ring).abs())
}

/// Ряд **вдоль бордюра** — в кармане вдоль улицы ([`LotKind::Kerbside`]),
/// который тоньше места поперёк: машина встаёт в него параллельно улице, как в
/// ряду у бордюра (`map::cars`), по [`PARALLEL_LENGTH`] на место. Так стоят в
/// lay-by из OSM: в Туле у 42 из 56 таких контуров толщина от 1.6 до 5.1 м, и
/// места поперёк, по [`pocket_rows`], в них не встают — 30 из них оставались
/// тёмной лентой у тротуара без единой машины, а у остальных выдуманная
/// раскладка ставила места поперёк там, где контур на изломе шире. С рядом
/// вдоль бордюра места есть у 47 из 56.
///
/// Места — вдоль сторон контура, от длинной к короткой, по середине полосы;
/// ряд вдоль противоположной стороны ложится поверх уже стоящего и снимается
/// ([`Placed`]). Машина смотрит по движению своей стороны улицы: улица — за
/// той кромкой полосы, где [`Surroundings::paved`] нашёл её полотно, и при
/// правостороннем движении она у машины слева. Разметки нет
/// ([`Stall::parallel`]).
pub(super) fn parallel_rows(
    outline: &Outline,
    roads: &Surroundings,
    traffic: TrafficSide,
) -> Vec<Stall> {
    let ring = &outline.area.outer;
    let half = strip_width(ring) / 2.0;
    // наружу от заливки — по знаку площади кольца
    let outward = if signed_ring_area(ring) > 0.0 {
        -1.0
    } else {
        1.0
    };
    let mut sides: Vec<(Vec2, Vec2)> = (0..ring.len())
        .map(|index| (ring[index], ring[(index + 1) % ring.len()]))
        .filter(|(from, to)| from.distance(*to) >= PARALLEL_LENGTH + 2.0 * STRIP_MARGIN)
        .collect();
    sides.sort_by(|a, b| b.0.distance(b.1).total_cmp(&a.0.distance(a.1)));

    let mut placed = Placed::default();
    for (from, to) in sides {
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let inward = along.perp() * -outward;
        let middle = from.midpoint(to) + inward * half;
        let ahead = half + PARALLEL_WIDTH;
        let street =
            if !roads.paved(middle - inward * ahead) && roads.paved(middle + inward * ahead) {
                inward
            } else {
                -inward
            };
        // улица слева от машины при правостороннем движении: `kerb()` там -1
        let heading = street.perp() * traffic.kerb();
        let length = from.distance(to) - 2.0 * STRIP_MARGIN;
        let count = (length / PARALLEL_LENGTH).floor() as usize;
        let start = STRIP_MARGIN + (length - count as f32 * PARALLEL_LENGTH) / 2.0;
        for index in 0..count {
            let place = start + (index as f32 + 0.5) * PARALLEL_LENGTH;
            let at = from + along * place + inward * half;
            if fits_box(
                outline,
                at,
                along * (PARALLEL_LENGTH / 2.0),
                inward * (PARALLEL_WIDTH / 2.0),
            ) {
                placed.push(Stall {
                    at,
                    along: heading,
                    depth: PARALLEL_LENGTH,
                    parallel: true,
                });
            }
        }
    }
    placed.stalls
}

/// Ряд парковочного кармана — **вдоль сторон контура**, а не по сетке
/// описанного прямоугольника: полоса в треть километра гнётся вместе с улицей,
/// и на изломе в один градус прямая сетка уходит из неё на метры.
///
/// Стороны берутся от длинной к короткой; ряд вдоль противоположной стороны
/// ложится поверх уже стоящего и снимается ([`Placed`]). Носом место стоит к
/// улице, если она нашлась перед ним ([`Surroundings::paved`]), иначе — от
/// своей стороны внутрь: с другой стороны кармана и подъезжают.
fn pocket_rows(outline: &Outline, roads: &Surroundings, depth: f32) -> Vec<Stall> {
    let ring = &outline.area.outer;
    // наружу от заливки — по знаку площади кольца
    let outward = if signed_ring_area(ring) > 0.0 {
        -1.0
    } else {
        1.0
    };
    let mut sides: Vec<(Vec2, Vec2)> = (0..ring.len())
        .map(|index| (ring[index], ring[(index + 1) % ring.len()]))
        .filter(|(from, to)| from.distance(*to) >= MIN_ROW_RUN as f32 * STALL_WIDTH)
        .collect();
    sides.sort_by(|a, b| b.0.distance(b.1).total_cmp(&a.0.distance(a.1)));

    let mut placed = Placed::default();
    for (from, to) in sides {
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let inward = along.perp() * -outward;
        let count = (from.distance(to) / STALL_WIDTH).floor() as usize;
        let start = (from.distance(to) - count as f32 * STALL_WIDTH) / 2.0;
        // нос один на ряд: по середине стороны, а не по каждому месту, иначе у
        // перекрёстка ряд встал бы вразнобой
        let middle = from.midpoint(to) + inward * (STRIP_MARGIN + depth / 2.0);
        let ahead = depth / 2.0 + PAIR_AISLE / 2.0;
        let nose = if roads.paved(middle - inward * ahead) {
            -inward
        } else {
            inward
        };
        let mut run: Vec<Stall> = Vec::new();
        let mut row: Vec<Stall> = Vec::new();
        for index in 0..count {
            let place = start + (index as f32 + 0.5) * STALL_WIDTH;
            let at = from + along * place + inward * (STRIP_MARGIN + depth / 2.0);
            if !roads.cover(at, nose, depth) && fits(outline, at, along, inward, depth) {
                run.push(Stall {
                    at,
                    along: nose,
                    depth,
                    parallel: false,
                });
            } else {
                if run.len() >= MIN_ROW_RUN {
                    row.append(&mut run);
                }
                run.clear();
            }
        }
        if run.len() >= MIN_ROW_RUN {
            row.append(&mut run);
        }
        // порядок — тот, которого ждёт разметка: вдоль `-perp(Stall::along)`
        if row
            .first()
            .is_some_and(|stall| (-stall.along.perp()).dot(along) < 0.0)
        {
            row.reverse();
        }
        for stall in row {
            placed.push(stall);
        }
    }
    placed.stalls
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
pub(super) fn row_bands(width: f32) -> Vec<f32> {
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

/// Куда каждый ряд смотрит носом: ±1 вдоль `across`. Ряд выезжает в тот зазор,
/// который шире, — пристенный в свой проезд, первый ряд пары назад в проезд
/// перед собой, второй вперёд в проезд за собой.
///
/// Без этого у всей площадки один нос, и первый ряд каждой пары стоит носом в
/// спину второму: на `rect(20, 40)` таких мест половина. Арифметика зазоров —
/// та же, что уже формализует `every_row_has_an_aisle_to_drive_in_from`.
fn row_noses(bands: &[f32], width: f32) -> Vec<f32> {
    bands
        .iter()
        .enumerate()
        .map(|(index, band)| {
            let before = index
                .checked_sub(1)
                .map_or(*band, |previous| band - bands[previous] - STALL_DEPTH);
            let after = bands
                .get(index + 1)
                .map_or(width - band - STALL_DEPTH, |next| next - band - STALL_DEPTH);
            if after >= before { 1.0 } else { -1.0 }
        })
        .collect()
}

/// Центры мест вдоль ряда длиной `length` (уже без отступов от краёв):
/// [`ROW_BLOCK`] метров мест, поперечный проезд, снова места.
///
/// Считается один раз на площадку и одинаково для всех её рядов — разрывы
/// обязаны стоять на одной линии, иначе вместо проезда через всю стоянку
/// получится россыпь пустых мест.
pub(super) fn row_places(length: f32) -> Vec<f32> {
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
