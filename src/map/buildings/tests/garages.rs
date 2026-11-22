//! Гаражный прогон: ворота меряются боксами своего куска и стоят под швом
//! кровли, целые на обоих концах стены.

use super::super::fixtures::{building, detail, extruded_mesh, is_solid, oblong, whole_cells};
use super::super::material::*;
use super::super::*;
use crate::map::meshing::unpack_material;
use crate::map::osm::{AreaKind, BuildingUse};

/// Бокс гаражного прогона одет в **ворота**, и ячейка его стены — сам бокс, а
/// не панель в 3.2 м: створка обязана прийтись под свой шов на кровле.
/// Отдельного полотна по входу из OSM у него при этом нет — ворота уже в
/// каждой ячейке.
#[test]
fn a_garage_run_wears_gates_measured_in_bays() {
    let _sun = crate::map::default_sun();
    let mut ribbon = building(oblong(8.0, 40.0), None, AreaKind::Building);
    ribbon.building_use = BuildingUse::GarageBlock;
    ribbon.entrances = vec![Vec2::new(12.0, 0.0)];
    let builder = extruded_mesh(&[ribbon], &[], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");

    let gates: Vec<[f32; 4]> = frames
        .iter()
        .copied()
        .filter(|frame| unpack_material(frame[2]).0 == WallKind::GarageDoors.code())
        .collect();
    assert!(!gates.is_empty(), "стены прогона одеты в ворота");
    assert!(
        frames
            .iter()
            .all(|frame| unpack_material(frame[2]).0 != DOOR_CODE),
        "отдельного полотна по входу у гаража нет: створка уже в каждом боксе"
    );
    // 40 м по 3.9 — десять боксов, и край стены приходится ровно на десятый
    let far = gates.iter().fold(0.0_f32, |far, gate| far.max(gate[0]));
    assert!((far - 10.0).abs() < 1e-3, "край стены — целый бокс: {far}");
    // гараж в один этаж, и балконов ему не полагается — посев отрицательный
    assert!(
        gates.iter().all(|gate| gate[3] < 0.0),
        "балконов на гараже не бывает"
    );
    // Ворота — только вдоль прогона. Лента идёт по X, значит створки на южной
    // стене, а западный торец несёт тот же рисунок боксов без единого проёма
    // (`WallMark::Solid`): пока облицовка выбиралась на дом целиком, створки
    // вставали и на торце, и на углу две из них упирались друг в друга.
    assert!(
        gates.iter().any(|gate| is_solid(gate[3])),
        "торец прогона глухой"
    );
    assert!(
        gates.iter().any(|gate| !is_solid(gate[3])),
        "фасад прогона в воротах"
    );
}

/// Ворота на гаражной стене **всегда целые**: сетка кончается на её углах, а
/// не посреди створки.
///
/// Стена, посаженная на фазу своего куска, начинается и кончается посреди
/// клетки — и крайние ворота выходят обрезанными с обоих концов, по половине
/// створки на каждом зубе гребёнки. Проверяется это по мешу: у каждой вершины
/// основания гаражной стены номер бокса обязан быть целым.
#[test]
fn a_garage_wall_holds_whole_gates() {
    let _sun = crate::map::default_sun();
    let mut bent = building(
        vec![
            Vec2::ZERO,
            Vec2::new(64.0, 0.0),
            Vec2::new(64.0, 8.0),
            Vec2::new(8.0, 8.0),
            Vec2::new(8.0, 52.0),
            Vec2::new(0.0, 52.0),
        ],
        None,
        AreaKind::Building,
    );
    bent.building_use = BuildingUse::GarageBlock;
    let builder = extruded_mesh(&[bent], &[], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");

    let gates: Vec<[f32; 4]> = frames
        .iter()
        .copied()
        .filter(|frame| unpack_material(frame[2]).0 == WallKind::GarageDoors.code())
        .collect();
    assert!(!gates.is_empty(), "стены прогона одеты в ворота");
    for gate in &gates {
        assert!(
            whole_cells(gate[0]),
            "клетка стены кончается на её углу: {}",
            gate[0]
        );
    }
}

/// Ворота стоят **под своим же швом кровли**: шаг у стены тот же, что у куска
/// над ней, и меряется он в тех же метрах.
///
/// Пока стена мерила себя общей меркой — `длина / round(длина / BAY)`, — шаги
/// сходились только там, где длина стены равна длине куска; у разрезанного
/// контура стена вдвое короче, и ворота уезжали от гребёнки.
#[test]
fn a_gate_stands_under_its_own_roof_seam() {
    let _sun = crate::map::default_sun();
    // лента 15 × 5: боксов на неё встаёт три по 5.0 м, а общей меркой BAY их
    // вышло бы четыре по 3.75 — ворота разошлись бы с гребёнкой над ними
    let mut ribbon = building(oblong(5.0, 15.0), None, AreaKind::Building);
    ribbon.building_use = BuildingUse::GarageBlock;
    let bay = super::super::garages::garage_runs(std::slice::from_ref(&ribbon))[&0]
        .main()
        .bay;
    assert!((bay - 5.0).abs() < 1e-3, "шаг бокса ленты: {bay}");

    let builder = extruded_mesh(&[ribbon], &[], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");
    let points = builder.positions_for_test();

    // ширина ячейки стены — прямо из меша: два соседних угла одной стены и
    // разница их номеров бокса
    let mut cells = Vec::new();
    for at in 1..frames.len() {
        let (mine, next) = (frames[at - 1], frames[at]);
        if unpack_material(mine[2]).0 != WallKind::GarageDoors.code()
            || unpack_material(next[2]).0 != WallKind::GarageDoors.code()
        {
            continue;
        }
        let step = (next[0] - mine[0]).abs();
        let span = Vec2::new(
            points[at][0] - points[at - 1][0],
            points[at][1] - points[at - 1][1],
        );
        if step > 1e-3 && span.length() > 1e-3 {
            cells.push(span.length() / step);
        }
    }
    assert!(!cells.is_empty(), "гаражные стены есть в меше");
    for cell in &cells {
        assert!(
            (cell - bay).abs() < 1e-2,
            "ячейка стены {cell} м против бокса кровли {bay} м"
        );
    }
}
