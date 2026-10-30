use super::generated::{row_bands, row_places};
use super::*;
// полный обход кольца — то, с чем тесты сверяют ответ `Outline`; в самой
// раскладке его больше нет
use crate::map::osm::fixture;
use crate::map::osm::model::point_in_area;

fn lot(outer: Vec<Vec2>) -> PolyArea {
    fixture::area(AreaKind::Parking(LotKind::Yard), outer)
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
    let stalls = stalls(&lot, &[]);
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
    // полосы меряются по неподвижной оси: `Stall::along` у среднего ряда
    // другого знака, чем у соседей ([`row_noses`])
    let mut bands: Vec<f32> = Vec::new();
    for stall in stalls(&lot, &[]) {
        let depth = stall.at.y;
        if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
            bands.push(depth);
        }
    }
    bands.sort_by(f32::total_cmp);
    assert_eq!(bands.len(), 3, "{bands:?}");
    // пара рядов спинами (9.8 и 15.0), проезд, пристенный ряд (26.2)
    let gaps: Vec<f32> = bands.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!((gaps[0] - STALL_DEPTH).abs() < 0.01, "{gaps:?}");
    assert!((gaps[1] - (STALL_DEPTH + AISLE)).abs() < 0.01, "{gaps:?}");
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

/// Есть проезд — ряды ложатся по обе стороны от него, носом в проезд,
/// и вдоль он не рвётся: OSM уже нарезал стоянку проездами.
#[test]
fn rows_lie_along_the_osm_aisle_on_both_sides() {
    // площадка 120 × 22 и проезд по её середине
    let lot = lot(rect(22.0, 120.0));
    let aisle = fixture::parking_aisle(vec![Vec2::new(2.0, 11.0), Vec2::new(118.0, 11.0)]);
    let stalls = stalls(&lot, &[&aisle]);
    assert!(stalls.len() > 80, "{}", stalls.len());
    for stall in &stalls {
        // место стоит поперёк проезда...
        assert!(stall.along.x.abs() < 0.01, "{:?}", stall.along);
        // ...и носом в него: центр по одну сторону, нос — по другую
        let side = (stall.at.y - 11.0).signum();
        assert!((stall.along.y + side).abs() < 0.01, "{stall:?}");
        let depth = (stall.at.y - 11.0).abs();
        assert!(
            (depth - (AISLE + STALL_DEPTH) / 2.0).abs() < 0.01,
            "{depth}"
        );
    }
    // вдоль ряда разрывов нет — места идут сплошняком по [`STALL_WIDTH`]
    let mut row: Vec<f32> = stalls
        .iter()
        .filter(|stall| stall.at.y > 11.0)
        .map(|stall| stall.at.x)
        .collect();
    row.sort_by(f32::total_cmp);
    for pair in row.windows(2) {
        assert!((pair[1] - pair[0] - STALL_WIDTH).abs() < 0.01, "{row:?}");
    }
}

/// Поперечный проезд рядов не получает: он ведёт **к** рядам, а не вдоль
/// них, и его ряд лёг бы поперёк главных.
#[test]
fn a_cross_aisle_gets_no_rows_of_its_own() {
    let lot = lot(rect(60.0, 120.0));
    let main = fixture::parking_aisle(vec![Vec2::new(2.0, 30.0), Vec2::new(118.0, 30.0)]);
    let cross = fixture::parking_aisle(vec![Vec2::new(60.0, 2.0), Vec2::new(60.0, 58.0)]);
    let stalls = stalls(&lot, &[&main, &cross]);
    for stall in &stalls {
        assert!(stall.along.x.abs() < 0.01, "{stall:?}");
    }
    for (index, stall) in stalls.iter().enumerate() {
        for other in &stalls[index + 1..] {
            assert!(!overlaps(stall, other), "{stall:?} / {other:?}");
        }
    }
}

/// Поперечный проезд OSM рисует **разрывом**: ход проезда разрезан надвое,
/// и между кусками оставлена полоса под проезд. Места на неё не встают.
#[test]
fn a_gap_shared_by_two_aisles_is_kept_as_a_cross_drive() {
    let lot = lot(rect(38.0, 120.0));
    let near_low = fixture::parking_aisle(vec![Vec2::new(2.0, 9.0), Vec2::new(57.0, 9.0)]);
    let far_low = fixture::parking_aisle(vec![Vec2::new(63.0, 9.0), Vec2::new(118.0, 9.0)]);
    let near_high = fixture::parking_aisle(vec![Vec2::new(2.0, 29.0), Vec2::new(57.0, 29.0)]);
    let far_high = fixture::parking_aisle(vec![Vec2::new(63.0, 29.0), Vec2::new(118.0, 29.0)]);
    let stalls = stalls(&lot, &[&near_low, &far_low, &near_high, &far_high]);
    assert!(stalls.len() > 60, "{}", stalls.len());
    for stall in &stalls {
        assert!(
            (stall.at.x - 60.0).abs() >= (AISLE + STALL_WIDTH) / 2.0 - 0.01,
            "{stall:?}"
        );
    }
    // проезд один — ряды по обе стороны от него целы
    let before = stalls
        .iter()
        .filter(|stall| stall.at.x < 60.0)
        .fold(f32::NEG_INFINITY, |far, stall| far.max(stall.at.x));
    let after = stalls
        .iter()
        .filter(|stall| stall.at.x > 60.0)
        .fold(f32::INFINITY, |near, stall| near.min(stall.at.x));
    assert!(before > 40.0 && after < 80.0, "{before} / {after}");
    assert!(
        after - before < 2.0 * (AISLE + STALL_WIDTH),
        "{before} / {after}"
    );
}

/// А разрыв на одной-единственной полосе — это обрыв рисования, а не
/// проезд: ряды через него идут сплошняком.
#[test]
fn a_gap_on_a_single_aisle_is_not_a_cross_drive() {
    let lot = lot(rect(38.0, 120.0));
    let near = fixture::parking_aisle(vec![Vec2::new(2.0, 9.0), Vec2::new(57.0, 9.0)]);
    let far = fixture::parking_aisle(vec![Vec2::new(63.0, 9.0), Vec2::new(118.0, 9.0)]);
    let whole = fixture::parking_aisle(vec![Vec2::new(2.0, 29.0), Vec2::new(118.0, 29.0)]);
    let stalls = stalls(&lot, &[&near, &far, &whole]);
    assert!(
        stalls.iter().any(|stall| (stall.at.x - 60.0).abs() < 3.0),
        "одиночный разрыв сошёл за проезд"
    );
}

/// Колено проезда — отворот к соседнему проезду, а не ряд: у больничной
/// стоянки (way 344589378) с него начинается каждый проезд.
#[test]
fn the_elbow_that_leads_to_an_aisle_gets_no_rows() {
    let lot = lot(rect(60.0, 120.0));
    let elbow = fixture::parking_aisle(vec![
        Vec2::new(20.0, 2.0),
        Vec2::new(30.0, 30.0),
        Vec2::new(110.0, 30.0),
    ]);
    let stalls = stalls(&lot, &[&elbow]);
    for stall in &stalls {
        assert!(stall.along.x.abs() < 0.01, "{stall:?}");
    }
    // отворот — дорога к ряду, а не поперечный проезд: дыры в ряду он не
    // пробивает ([`cross_drives_of`] читает только разрывы между соосными
    // ходами одной полосы)
    assert!(
        stalls
            .iter()
            .any(|stall| (18.0..32.0).contains(&stall.at.x)),
        "отворот пробил дыру в ряду"
    );
}

/// Излом главного хода не рвёт ряд надвое: подряд идущие звенья проезда
/// склеиваются в один отрезок.
#[test]
fn a_kink_in_the_aisle_does_not_break_the_row_in_two() {
    let lot = lot(rect(30.0, 120.0));
    let bent = fixture::parking_aisle(vec![
        Vec2::new(2.0, 14.0),
        Vec2::new(60.0, 15.0),
        Vec2::new(118.0, 16.0),
    ]);
    let stalls = stalls(&lot, &[&bent]);
    // ряд — места на одной глубине; проезд наклонный, так что по `y` их
    // не разобрать
    let mut rows: Vec<Vec<Vec2>> = Vec::new();
    for stall in &stalls {
        let band = stall.at.dot(stall.along);
        match rows
            .iter_mut()
            .find(|row| (row[0].dot(stall.along) - band).abs() < 0.01)
        {
            Some(row) => row.push(stall.at),
            None => rows.push(vec![stall.at]),
        }
    }
    let row = rows
        .iter_mut()
        .max_by_key(|row| row.len())
        .expect("мест нет вовсе");
    row.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert!(row.len() > 40, "{}", row.len());
    for pair in row.windows(2) {
        let step = pair[0].distance(pair[1]);
        assert!((step - STALL_WIDTH).abs() < 0.01, "{step} в {row:?}");
    }
}

/// Между двумя проездами — **пара рядов спинами**, а не по одному ряду на
/// проезд: у больничной стоянки (way 344589378) проезды идут через 14.8 м,
/// и на фиксированном отступе от осевой второй ряд каждый раз снимался.
#[test]
fn a_pocket_between_two_aisles_holds_a_pair_of_rows_back_to_back() {
    let lot = lot(rect(40.0, 100.0));
    let lanes = [
        fixture::parking_aisle(vec![Vec2::new(2.0, 12.0), Vec2::new(98.0, 12.0)]),
        fixture::parking_aisle(vec![Vec2::new(2.0, 26.8), Vec2::new(98.0, 26.8)]),
    ];
    let stalls = stalls(&lot, &[&lanes[0], &lanes[1]]);
    let mut bands: Vec<f32> = Vec::new();
    for stall in &stalls {
        let depth = stall.at.y;
        if !bands.iter().any(|band| (band - depth).abs() < 0.01) {
            bands.push(depth);
        }
    }
    bands.sort_by(f32::total_cmp);
    // в кармане между двумя проездами — пара рядов спинами по его середине
    let pair: Vec<f32> = bands
        .iter()
        .copied()
        .filter(|band| (12.0..26.8).contains(band))
        .collect();
    assert_eq!(pair.len(), 2, "{bands:?}");
    assert!(
        (pair[0].midpoint(pair[1]) - 19.4).abs() < 0.01,
        "пара не по середине кармана: {bands:?}"
    );
    // места ужаты ради проезда: 14.8 = 4.8 + 4.8 + 5.2 проезда
    let depth = pair[1] - pair[0];
    assert!((depth - STALL_DEPTH_MIN).abs() < 0.01, "{depth}");
    assert!(14.8 - 2.0 * depth >= PAIR_AISLE - 0.01, "{depth}");
    // и каждое место носом в свой проезд
    let to_lane = |at: Vec2| (at.y - 12.0).abs().min((at.y - 26.8).abs());
    for stall in &stalls {
        if !(12.0..26.8).contains(&stall.at.y) {
            continue;
        }
        let nose = stall.at + stall.along * (stall.depth / 2.0);
        assert!(to_lane(nose) < to_lane(stall.at), "{stall:?}");
    }
}

/// Обрывок ряда в одно место не размечается: у скошенной кромки такое
/// место читается полоской на пустом асфальте.
#[test]
fn a_run_of_one_stall_is_not_striped() {
    // треугольный клин: к острому углу ряды сходят на одно место
    let wedge = lot(vec![
        Vec2::ZERO,
        Vec2::new(100.0, 0.0),
        Vec2::new(100.0, 40.0),
    ]);
    let aisle = fixture::parking_aisle(vec![Vec2::new(10.0, 12.0), Vec2::new(95.0, 12.0)]);
    let stalls = stalls(&wedge, &[&aisle]);
    assert!(!stalls.is_empty());
    let mut rows: Vec<(f32, usize)> = Vec::new();
    for stall in &stalls {
        let band = stall.at.dot(stall.along);
        match rows.iter_mut().find(|row| (row.0 - band).abs() < 0.01) {
            Some(row) => row.1 += 1,
            None => rows.push((band, 1)),
        }
    }
    for (band, count) in &rows {
        assert!(*count >= MIN_ROW_RUN, "обрывок в {count} место на {band}");
    }
}

/// Место у самой кромки не размечается, если заехать на него неоткуда:
/// продолженная сетка полос ставит ряд так, что его проезд остаётся за
/// пределами площадки.
#[test]
fn a_stall_with_no_asphalt_in_front_of_it_is_not_striped() {
    // площадка ровно в ряд мест плюс проезд с одной стороны
    let lot = lot(rect(14.0, 60.0));
    let aisle = fixture::parking_aisle(vec![Vec2::new(2.0, 11.4), Vec2::new(58.0, 11.4)]);
    for stall in &stalls(&lot, &[&aisle]) {
        let ahead = stall.at + stall.along * (stall.depth / 2.0 + PAIR_AISLE / 2.0);
        assert!(point_in_area(ahead, &lot), "заехать неоткуда: {stall:?}");
    }
}

/// Сетка проездов продолжается до краёв контура: в OSM проезд обрывается,
/// не дойдя до края, и вдоль двух сторон площадки оставалась широкая
/// полоса голого асфальта, а вдоль двух других мест не было вовсе.
#[test]
fn the_lane_grid_runs_out_to_the_edges_of_the_lot() {
    let lot = lot(rect(60.0, 100.0));
    // проезд в углу площадки, коротким куском
    let aisle = fixture::parking_aisle(vec![Vec2::new(4.0, 12.0), Vec2::new(40.0, 12.0)]);
    let stalls = stalls(&lot, &[&aisle]);
    let far = stalls
        .iter()
        .map(|stall| stall.at.y)
        .fold(f32::MIN, f32::max);
    assert!(far > 40.0, "раскладка не дошла до дальнего края: {far}");
    let end = stalls
        .iter()
        .map(|stall| stall.at.x)
        .fold(f32::MIN, f32::max);
    assert!(end > 90.0, "ряд оборван по длине проезда: {end}");
}

/// Ряды по обе стороны одного проезда стоят напротив друг друга: обе
/// стороны считаются от начала отрезка.
#[test]
fn the_two_rows_of_an_aisle_line_up_with_each_other() {
    let lot = lot(rect(22.0, 100.7));
    // длина проезда нарочно не кратна месту
    let aisle = fixture::parking_aisle(vec![Vec2::new(2.3, 11.0), Vec2::new(98.4, 11.0)]);
    let stalls = stalls(&lot, &[&aisle]);
    let row = |above: bool| {
        let mut places: Vec<f32> = stalls
            .iter()
            .filter(|stall| (stall.at.y > 11.0) == above)
            .map(|stall| stall.at.x)
            .collect();
        places.sort_by(f32::total_cmp);
        places
    };
    let (near, far) = (row(false), row(true));
    assert_eq!(near.len(), far.len(), "{near:?} / {far:?}");
    for (left, right) in near.iter().zip(&far) {
        assert!((left - right).abs() < 0.01, "{left} / {right}");
    }
}

/// Нет проездов — раскладка выдумывается, и ряды идут вдоль самой длинной
/// стороны контура, а не вдоль оси описанного прямоугольника.
#[test]
fn without_aisles_rows_follow_the_longest_side() {
    // трапеция: низ 80 м вдоль x, верх скошен — у минимального
    // прямоугольника ось уезжает от длинной стороны
    let lot = lot(vec![
        Vec2::ZERO,
        Vec2::new(80.0, 0.0),
        Vec2::new(66.0, 26.0),
        Vec2::new(6.0, 22.0),
    ]);
    let stalls = stalls(&lot, &[]);
    assert!(!stalls.is_empty());
    for stall in &stalls {
        // ряд вдоль нижней стороны — место стоит поперёк неё
        assert!(stall.along.x.abs() < 0.01, "{:?}", stall.along);
    }
}

/// Близнец `a_stall_with_no_asphalt_in_front_of_it_is_not_striped` на
/// выдуманной раскладке: проверка носа нужна и там, где проездов нет.
#[test]
fn a_stall_with_no_asphalt_in_front_of_it_is_not_striped_without_an_aisle() {
    let lot = lot(rect(20.0, 40.0));
    let stalls = stalls(&lot, &[]);
    assert!(!stalls.is_empty());
    for stall in &stalls {
        let ahead = stall.at + stall.along * (stall.depth / 2.0 + PAIR_AISLE / 2.0);
        assert!(point_in_area(ahead, &lot), "заехать неоткуда: {stall:?}");
    }
}

/// Близнец `a_run_of_one_stall_is_not_striped` на выдуманной раскладке:
/// двор с выкусом в нижней кромке оставлял одинокое место в (3.7, 5.0).
#[test]
fn a_run_of_one_stall_is_not_striped_without_an_aisle() {
    let yard = lot(vec![
        Vec2::ZERO,
        Vec2::new(5.0, 0.0),
        Vec2::new(5.0, 10.0),
        Vec2::new(7.6, 10.0),
        Vec2::new(7.6, 0.0),
        Vec2::new(40.0, 0.0),
        Vec2::new(40.0, 20.0),
        Vec2::new(0.0, 20.0),
    ]);
    let stalls = stalls(&yard, &[]);
    assert!(!stalls.is_empty());
    for stall in &stalls {
        let side = Vec2::new(-stall.along.y, stall.along.x);
        assert!(
            stalls.iter().any(|other| {
                let step = other.at - stall.at;
                step.dot(stall.along).abs() < 0.01
                    && (step.dot(side).abs() - STALL_WIDTH).abs() < 0.01
            }),
            "обрывок в одно место: {stall:?}"
        );
    }
}

/// Нос ряда берётся по зазорам ([`row_noses`]), а не один на всю площадку:
/// пара рядов стоит спинами, носами наружу, пристенный — в свой проезд.
#[test]
fn each_row_of_an_invented_layout_faces_its_aisle() {
    let lot = lot(rect(30.0, 40.0));
    // полоса и её нос по `y`: ряды идут вдоль `x`
    let mut rows: Vec<(f32, f32)> = Vec::new();
    for stall in stalls(&lot, &[]) {
        assert!(stall.along.x.abs() < 0.01, "{stall:?}");
        if !rows
            .iter()
            .any(|(band, _)| (band - stall.at.y).abs() < 0.01)
        {
            rows.push((stall.at.y, stall.along.y));
        }
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(rows.len(), 3, "{rows:?}");
    // пара 9.8 + 15.0 сходится спинами на 12.4, носами расходится
    let backs: Vec<f32> = rows[..2]
        .iter()
        .map(|(band, nose)| band - nose * (STALL_DEPTH / 2.0))
        .collect();
    assert!((backs[0] - backs[1]).abs() < 0.01, "{rows:?} / {backs:?}");
    assert!((backs[0] - 12.4).abs() < 0.01, "{backs:?}");
    assert!(rows[0].1 < 0.0 && rows[1].1 > 0.0, "{rows:?}");
    // а пристенный ряд смотрит назад, в проезд перед собой
    assert!(rows[2].1 < 0.0, "{rows:?}");
}

#[test]
fn a_yard_corner_gets_nothing() {
    // 4 × 6 — двор на пару машин, размечать нечего
    assert!(stalls(&lot(rect(4.0, 6.0)), &[]).is_empty());
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
    for stall in stalls(&ell, &[]) {
        assert!(point_in_area(stall.at, &ell), "{:?}", stall.at);
    }
}

/// Дорога сквозь стоянку режет ряды: ни одно место не задевает ни её
/// полотна, ни бордюра, а по обе стороны от неё места остаются.
#[test]
fn a_road_through_the_lot_cuts_the_rows() {
    let lot = lot(rect(80.0, 120.0));
    let through = RoadLine {
        oneway: true,
        ..fixture::street(vec![Vec2::new(-10.0, 40.0), Vec2::new(130.0, 40.0)], 5.0)
    };
    assert!(is_through(&through));
    let stalls = stalls_beside(&lot, &[], &[&through], &[], TrafficSide::Right);
    let clear = through.sidewalk().kerb_edge(through.width / 2.0, LOT_KERB);
    assert!(stalls.iter().any(|stall| stall.at.y < 40.0 - clear));
    assert!(stalls.iter().any(|stall| stall.at.y > 40.0 + clear));
    for stall in &stalls {
        let reach = (stall.along.y.abs() * stall.depth + stall.along.x.abs() * STALL_WIDTH) / 2.0;
        assert!(
            (stall.at.y - 40.0).abs() - reach >= clear,
            "место на дороге: {stall:?}"
        );
    }
}

/// Проезды по две стороны дороги сквозь площадку друг против друга не
/// стоят — и размечаются каждый своей сеткой: в общей сетке южная полоса
/// делила северный карман, и в нём вставал один ряд вместо пары.
#[test]
fn aisles_across_a_through_road_keep_their_own_lane_grids() {
    let lot = lot(rect(100.0, 102.0));
    let through = RoadLine {
        oneway: true,
        ..fixture::street(vec![Vec2::new(-10.0, 50.0), Vec2::new(112.0, 50.0)], 5.0)
    };
    let step = 17.0;
    let north: Vec<RoadLine> = (1..=5)
        .map(|lane| {
            let x = lane as f32 * step;
            fixture::parking_aisle(vec![Vec2::new(x, 50.0), Vec2::new(x, 96.0)])
        })
        .collect();
    let south: Vec<RoadLine> = (1..=4)
        .map(|lane| {
            let x = (lane as f32 + 0.5) * step;
            fixture::parking_aisle(vec![Vec2::new(x, 4.0), Vec2::new(x, 50.0)])
        })
        .collect();
    let aisles: Vec<&RoadLine> = north.iter().chain(&south).collect();
    let stalls = stalls_beside(&lot, &aisles, &[&through], &[], TrafficSide::Right);
    assert!(stalls.iter().any(|stall| stall.at.y > 60.0));
    assert!(stalls.iter().any(|stall| stall.at.y < 40.0));
    // пара рядов спинами посреди кармана в 17 м: ряд в 5.9 м от полосы.
    // За крайними проездами сетка продолжена до кромки — там свой счёт
    for stall in &stalls {
        let shift = if stall.at.y > 50.0 { 0.0 } else { step / 2.0 };
        if stall.at.x < step + shift || stall.at.x > 5.0 * step - shift {
            continue;
        }
        let offset = (stall.at.x - shift).rem_euclid(step);
        let to_lane = offset.min(step - offset);
        assert!((to_lane - 5.9).abs() < 0.2, "{to_lane}: {stall:?}");
    }
}

/// Дорога наискось режет два ряда пары по-разному — пара всё равно
/// прямоугольник: у каждого места есть сосед за спиной.
#[test]
fn a_pair_of_rows_cut_askew_is_trimmed_to_a_rectangle() {
    let lot = lot(rect(100.0, 120.0));
    let through = RoadLine {
        oneway: true,
        ..fixture::street(vec![Vec2::new(-10.0, 45.0), Vec2::new(130.0, 105.0)], 5.0)
    };
    let aisles: Vec<RoadLine> = [43.0, 60.0, 77.0]
        .into_iter()
        .map(|x| fixture::parking_aisle(vec![Vec2::new(x, 4.0), Vec2::new(x, 60.0)]))
        .collect();
    let aisles: Vec<&RoadLine> = aisles.iter().collect();
    let stalls = stalls_beside(&lot, &aisles, &[&through], &[], TrafficSide::Right);
    // пара в кармане между первыми двумя проездами, под дорогой
    let row = |nose: f32| -> Vec<i32> {
        let mut places: Vec<i32> = stalls
            .iter()
            .filter(|stall| (43.0..60.0).contains(&stall.at.x) && stall.at.y < 60.0)
            .filter(|stall| stall.along.x * nose > 0.5)
            .map(|stall| (stall.at.y * 10.0).round() as i32)
            .collect();
        places.sort_unstable();
        places
    };
    let (left, right) = (row(-1.0), row(1.0));
    assert!(left.len() >= MIN_ROW_RUN, "{left:?}");
    assert_eq!(left, right);
}

/// Парковочный карман — полоса в один ряд: с отступами место в неё не
/// встаёт, и размечается она от кромки до кромки, носом к улице.
#[test]
fn a_pocket_along_a_street_gets_one_row_facing_it() {
    let pocket = lot(rect(5.7, 60.0));
    let street = fixture::street(vec![Vec2::new(-20.0, -4.0), Vec2::new(80.0, -4.0)], 8.0);
    let stalls = stalls_beside(&pocket, &[], &[], &[&street], TrafficSide::Right);
    assert!(stalls.len() >= 20, "{}", stalls.len());
    for stall in &stalls {
        assert!((stall.at.y - 2.85).abs() < 0.3, "{stall:?}");
        assert!(stall.along.y < -0.99, "носом не к улице: {stall:?}");
    }
    // и полосы между местами у него есть, хотя запаса до кромки нет
    let mut builder = MeshBuilder::default();
    push_markings(&mut builder, &pocket, &stalls);
    assert!(!builder.is_empty());
}

/// Двор-полоса тоньше места поперёк (4 м) — ни ряда поперёк, ни выдуманной
/// раскладки: мест нет. Вдоль бордюра встают только в кармане вдоль улицы.
#[test]
fn a_strip_thinner_than_a_stall_gets_no_stalls() {
    let strip = lot(rect(4.0, 60.0));
    let street = fixture::street(vec![Vec2::new(-20.0, -4.0), Vec2::new(80.0, -4.0)], 8.0);
    assert!(stalls_beside(&strip, &[], &[], &[&street], TrafficSide::Right).is_empty());
}

/// Карман вдоль улицы в 4 м (`parking=street_side`) — ряд **вдоль**
/// бордюра по середине полосы, без разметки; машина смотрит по движению
/// своей стороны: улица южнее, при правостороннем движении — на запад.
#[test]
fn a_thin_kerbside_strip_parks_along_the_kerb() {
    let strip = fixture::area(AreaKind::Parking(LotKind::Kerbside), rect(4.0, 60.0));
    let street = fixture::street(vec![Vec2::new(-20.0, -4.0), Vec2::new(80.0, -4.0)], 8.0);
    let stalls = stalls_beside(&strip, &[], &[], &[&street], TrafficSide::Right);
    assert_eq!(stalls.len(), 9, "{stalls:?}");
    for stall in &stalls {
        assert!(stall.parallel);
        assert!((stall.at.y - 2.0).abs() < 0.01, "{stall:?}");
        assert!(stall.along.x < -0.99, "не по движению: {stall:?}");
    }
    let left = stalls_beside(&strip, &[], &[], &[&street], TrafficSide::Left);
    assert!(left.iter().all(|stall| stall.along.x > 0.99));
    let mut builder = MeshBuilder::default();
    push_markings(&mut builder, &strip, &stalls);
    assert!(builder.is_empty(), "места вдоль бордюра не размечают");
}

/// Асфальт перед носом — это и полотно улицы у кромки: двор в два ряда, у
/// которого второй ряд выезжает прямо на проезд вдоль площадки.
#[test]
fn a_stall_facing_a_street_beside_the_lot_is_reachable() {
    let outline = lot(rect(20.0, 40.0));
    let outline = Outline::of(&outline);
    let at = Vec2::new(20.0, 16.0);
    let nose = Vec2::Y;
    assert!(!reachable(
        &outline,
        &Surroundings::default(),
        at,
        nose,
        STALL_DEPTH
    ));
    let street = fixture::street(vec![Vec2::new(-20.0, 22.5), Vec2::new(60.0, 22.5)], 5.0);
    let roads = Surroundings::near(outline.area, &[], &[&street]);
    assert!(reachable(&outline, &roads, at, nose, STALL_DEPTH));
}

/// Две площадки, дотянутые до одного проезда, перекрываются, а места на
/// общей земле ставит одна — иначе машины двух раскладок стоят друг на друге.
#[test]
fn overlapping_lots_do_not_share_stalls() {
    let big = lot(rect(40.0, 60.0));
    let small = lot(vec![
        Vec2::new(30.0, 10.0),
        Vec2::new(80.0, 25.0),
        Vec2::new(75.0, 45.0),
        Vec2::new(25.0, 30.0),
    ]);
    let layout = ParkingLayout::new(&[small, big], &[], TrafficSide::Right);
    assert!(!layout.0[0].is_empty() && !layout.0[1].is_empty());
    for ours in &layout.0[0] {
        for theirs in &layout.0[1] {
            assert!(!overlaps(ours, theirs), "{ours:?} / {theirs:?}");
        }
    }
}

/// Индекс рёбер отвечает то же, что обход всего кольца, — и у дырки тоже.
///
/// Площадки перечислены те, какие делает мощение (`parse/lots.rs`), а не
/// одно опрятное кольцо: отмостка режет лот на **части** и оставляет в нём
/// **дырки** (будка кассы у ТРЦ «Макси»), а полосы индекса разложены по
/// габариту **внешнего** кольца. Дырка лежит внутри него, так что полоса
/// для неё найдётся всегда; случай, где это не очевидно, — дырка, дошедшая
/// до самого края габарита, и он тут отдельной площадкой.
#[test]
fn the_outline_index_agrees_with_the_full_ring() {
    let concave = PolyArea {
        holes: vec![vec![
            Vec2::new(4.0, 14.0),
            Vec2::new(12.0, 14.0),
            Vec2::new(12.0, 22.0),
            Vec2::new(4.0, 22.0),
        ]],
        ..lot(vec![
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 12.0),
            Vec2::new(18.0, 12.0),
            Vec2::new(30.0, 33.0),
            Vec2::new(0.0, 26.0),
        ])
    };
    // дырка до края габарита плюс вторая, мелкая: так режет отмостка
    let notched = PolyArea {
        holes: vec![
            vec![
                Vec2::new(6.0, 0.0),
                Vec2::new(14.0, 0.0),
                Vec2::new(14.0, 9.0),
                Vec2::new(6.0, 9.0),
            ],
            vec![
                Vec2::new(22.0, 13.0),
                Vec2::new(24.5, 13.0),
                Vec2::new(24.5, 15.5),
                Vec2::new(22.0, 15.5),
            ],
        ],
        ..lot(rect(30.0, 40.0))
    };
    // кусок, отрезанный отмосткой: тонкий и стоящий не в начале координат
    let sliver = lot(vec![
        Vec2::new(-17.0, 41.0),
        Vec2::new(13.0, 44.5),
        Vec2::new(12.0, 49.0),
        Vec2::new(-17.5, 45.5),
    ]);
    for area in [concave, notched, sliver] {
        let (low, high) = ring_bounds(&area.outer);
        let outline = Outline::of(&area);
        for x in 0..96 {
            for y in 0..96 {
                let point = low - 2.0
                    + (high - low + 4.0) * Vec2::new(x as f32, y as f32) / 95.0
                    + Vec2::new(0.013, 0.007);
                assert_eq!(
                    outline.contains(point),
                    point_in_area(point, &area),
                    "{point:?}"
                );
            }
        }
    }
}

/// Проезд между рядами — не дорога сквозь стоянку, и двусторонний
/// служебный проезд тоже: так у больших площадок нарисованы те же ряды.
#[test]
fn an_aisle_is_not_a_road_through_the_lot() {
    let points = vec![Vec2::ZERO, Vec2::new(50.0, 0.0)];
    assert!(!is_through(&fixture::parking_aisle(points.clone())));
    assert!(!is_through(&fixture::street(points, 5.0)));
}

/// У площадки два поля проездов под разными углами, и каждое размечается
/// по своим: места рядом с проездами второго поля стоят поперёк **них**, а
/// не поперёк проездов главного.
#[test]
fn a_second_field_of_aisles_is_striped_along_its_own_aisles() {
    let lot = lot(rect(100.0, 300.0));
    let mut aisles: Vec<RoadLine> = (0..6)
        .map(|index| {
            let y = 10.0 + 16.0 * index as f32;
            fixture::parking_aisle(vec![Vec2::new(5.0, y), Vec2::new(180.0, y)])
        })
        .collect();
    aisles.extend((0..4).map(|index| {
        let x = 215.0 + 17.0 * index as f32;
        fixture::parking_aisle(vec![Vec2::new(x, 5.0), Vec2::new(x, 95.0)])
    }));
    let aisles: Vec<&RoadLine> = aisles.iter().collect();
    let stalls = stalls(&lot, &aisles);
    let (mut west, mut east) = (0, 0);
    for stall in &stalls {
        if stall.at.x < 170.0 {
            assert!(stall.along.y.abs() > 0.99, "{stall:?}");
            west += 1;
        } else if stall.at.x > 225.0 {
            assert!(stall.along.x.abs() > 0.99, "{stall:?}");
            east += 1;
        }
    }
    assert!(west > 100 && east > 40, "{west} / {east}");
}
