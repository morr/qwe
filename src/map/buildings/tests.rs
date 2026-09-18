//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе: каждый тест здесь строит освещённую геометрию, поэтому каждый
//! берёт `default_sun()` — гард, который держит солнце на дефолте и не пускает
//! к нему соседа (`map/sun.rs`).

use super::fixtures::{
    building, detail, extruded_mesh, ground_shadows, house, is_solid, is_wall, mesh_points, oblong,
    rect, square, whole_cells,
};
use super::layers::*;
use super::material::*;
use super::roofs::*;
use super::*;
use crate::map::meshing::{min_area_rect, unpack_material};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, BuildingUse, Faith, Sacred, SacredForm};
use crate::map::shadow_dir;

/// Православный храм — назначение, которое тестам кровель и стен нужно чаще
/// прочих вер.
const ORTHODOX: BuildingUse = BuildingUse::Church(Sacred {
    faith: Faith::Orthodox,
    form: SacredForm::Nave,
    complex: 0,
    floor_dm: 0,
});

/// Сколько этажей записано в слоте.
fn slot_storeys(slot: f32) -> f32 {
    unpack_material(slot).1
}

#[test]
fn silhouette_picks_edges_facing_the_shadow() {
    let _sun = crate::map::default_sun();
    // свет сверху-слева, тень вправо-вниз: силуэт — нижнее и правое рёбра
    let edges = silhouette_edges(&square(), shadow_dir());
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|(a, b)| {
        let bottom = a.y == 0.0 && b.y == 0.0;
        let right = a.x == 10.0 && b.x == 10.0;
        bottom || right
    }));
}

#[test]
fn silhouette_is_winding_independent() {
    let _sun = crate::map::default_sun();
    let ccw = square();
    let cw: Vec<Vec2> = square().into_iter().rev().collect();
    let mut ccw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&ccw, shadow_dir());
    let mut cw_edges: Vec<(Vec2, Vec2)> = silhouette_edges(&cw, shadow_dir())
        .into_iter()
        .map(|(a, b)| (b, a))
        .collect();
    let key = |(a, b): &(Vec2, Vec2)| (a.x + a.y).min(b.x + b.y);
    ccw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
    cw_edges.sort_by(|left, right| key(left).total_cmp(&key(right)));
    assert_eq!(ccw_edges.len(), 2);
    for (ccw_edge, cw_edge) in ccw_edges.iter().zip(&cw_edges) {
        let matches = (ccw_edge.0 == cw_edge.0 && ccw_edge.1 == cw_edge.1)
            || (ccw_edge.0 == cw_edge.1 && ccw_edge.1 == cw_edge.0);
        assert!(matches, "{ccw_edge:?} vs {cw_edge:?}");
    }
}

#[test]
fn extrusion_walls_face_away_from_the_lift() {
    let _sun = crate::map::default_sun();
    // подъём вверх-вправо: у квадрата видимы южная и западная стены
    let lift = Lean::of().dir();
    assert!(
        lift.x > 0.0 && lift.y > 0.0,
        "the lift is oblique: {lift:?}"
    );
    let edges = silhouette_edges(&square(), -lift);
    assert_eq!(edges.len(), 2);
    assert!(
        edges.iter().any(|(a, b)| a.y == 0.0 && b.y == 0.0),
        "south wall"
    );
    assert!(
        edges.iter().any(|(a, b)| a.x == 0.0 && b.x == 0.0),
        "west wall"
    );
}

#[test]
fn the_wall_facing_the_light_is_lighter_than_the_one_facing_away() {
    let _sun = crate::map::default_sun();
    let lift = Lean::of().dir();
    let facade = Srgba::new(0.6, 0.6, 0.6, 1.0);
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    // свет из верхнего левого угла: западная стена (нормаль −X) освещена,
    // южная (нормаль −Y) в тени
    let (west, _) = wall_colors(facade, Vec2::new(0.0, 10.0), Vec2::ZERO, lift);
    let (south, _) = wall_colors(facade, Vec2::ZERO, Vec2::new(10.0, 0.0), lift);
    assert!(luminance(west) > luminance(facade.into()));
    assert!(luminance(south) < luminance(facade.into()));
    // обход ребра тон не меняет
    let (west_reversed, _) = wall_colors(facade, Vec2::ZERO, Vec2::new(0.0, 10.0), lift);
    assert_eq!(west, west_reversed);
}

#[test]
fn extrusion_sorts_the_far_end_of_the_lift_first() {
    let _sun = crate::map::default_sun();
    let north = building(
        square()
            .iter()
            .map(|p| *p + Vec2::new(0.0, 100.0))
            .collect(),
        Some(30.0),
        AreaKind::Building,
    );
    let south = building(square(), Some(3.0), AreaKind::Building);
    let positions = |list: &[PolyArea]| {
        extruded_mesh(list, &[], detail(false))
            .build()
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec()
    };
    let sorted = positions(&[south.clone(), north.clone()]);
    let reversed = positions(&[north, south]);
    // порядок входа не важен: painter's sort всегда пишет дальний по подъёму
    // (северный) дом первым, поэтому буферы вершин совпадают, а первая
    // вершина — северная
    assert_eq!(sorted, reversed);
    assert!(
        sorted[0][1] >= 100.0,
        "north building must be written first"
    );
}

#[test]
fn the_palette_follows_the_building_use_and_spares_the_kremlin() {
    let _sun = crate::map::default_sun();
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let mut church = building(square(), None, AreaKind::Building);
    church.building_use = ORTHODOX;
    let mut industrial = building(square(), None, AreaKind::Building);
    industrial.building_use = BuildingUse::Industrial;
    let mut kremlin = building(square(), None, AreaKind::Kremlin);
    kremlin.building_use = ORTHODOX;

    // Стена — тоже по материалу, как и крыша: у частного дома штукатурка или
    // кирпич, у склада профлист, у храма побелка, и совпасть им неоткуда.
    // Этажность на этой развилке одна на всех — сравниваются назначения.
    let wall = |b: &PolyArea| wall_look(b, 2.0).base;
    assert_ne!(wall(&house), wall(&industrial));
    assert_ne!(wall(&church), wall(&house));
    let kremlin_plain = building(square(), None, AreaKind::Kremlin);
    assert_eq!(wall(&kremlin), wall(&kremlin_plain));

    // а крыша — по материалу: у частного дома черепица или металл, у
    // промзоны профлист или битум, и одинаковыми они не выходят
    let roof = |b: &PolyArea| roof_color(b, &roof_look(b), false);
    assert_ne!(roof(&house), roof(&industrial));
    assert_ne!(roof(&church), roof(&house));
    // назначение Кремля крышу тоже не трогает
    assert_eq!(roof(&kremlin), roof(&kremlin_plain));
}

#[test]
fn the_roof_material_is_stable_and_follows_the_use() {
    let _sun = crate::map::default_sun();
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    // посев берётся от геометрии, а не от места в списке: два прогона дают
    // один материал, иначе переключение режима высот перекрашивало бы город
    assert_eq!(roof_look(&house).kind, roof_look(&house).kind);
    // сдвинутый дом — другой посев, а стало быть возможен и другой слот
    let moved: Vec<Vec2> = square()
        .into_iter()
        .map(|p| p + Vec2::new(37.0, 0.0))
        .collect();
    let mut neighbour = building(moved, None, AreaKind::Building);
    neighbour.building_use = BuildingUse::House;
    let _ = roof_look(&neighbour);

    // храм — всегда фальцевый металл, гараж — из своей таблицы
    let mut church = building(square(), None, AreaKind::Building);
    church.building_use = ORTHODOX;
    assert_eq!(roof_look(&church).kind, RoofKind::Seam);

    // ось фактуры — длинная сторона контура
    let long = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(40.0, 0.0),
        Vec2::new(40.0, 8.0),
        Vec2::new(0.0, 8.0),
    ];
    let axis = roof_look(&building(long, None, AreaKind::Building))
        .frame
        .axis;
    assert!(axis.x.abs() > axis.y.abs(), "{axis:?}");
}

#[test]
fn every_vertex_of_a_roofed_layer_carries_a_frame() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(14.0, 40.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    let builder = extruded_mesh(&[block], &[], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");
    // атрибут обязан быть у каждой вершины, иначе меш материал не примет
    assert_eq!(frames.len(), builder.vertex_count());
    // у стены своя рамка и свой код — по ней шейдер кладёт швы и проёмы; ноль
    // остаётся тому, у чего фактуры нет вовсе (оборудование), а у этой коробки
    // нет ни того, ни другого
    assert!(frames.iter().any(|frame| is_wall(frame[2])), "walls");
    assert!(
        frames
            .iter()
            .any(|frame| frame[2] > 0.0 && !is_wall(frame[2])),
        "roof"
    );
}

#[test]
fn gables_carry_frames_like_walls() {
    let _sun = crate::map::default_sun();
    // форма заказана явно: на «как решит игра» этому дому выпадает вальма, и
    // фронтонов в меше не оказывается вовсе — тест тогда проверял бы пустоту
    let mut house = building(oblong(9.0, 18.0), Some(6.0), AreaKind::Building);
    house.building_use = BuildingUse::House;
    let look = RoofLook::new(RoofKind::Tile, Srgba::WHITE, Vec2::X, 0.0);
    let mut builder = MeshBuilder::with_roof_coords();
    let drawn = push_house(
        &mut builder,
        &house,
        &look,
        &WallLook::new(WallKind::Plaster, Srgba::WHITE),
        look.base,
        RoofShape::Gable,
        false,
    );
    assert_eq!(drawn, RoofShape::Gable, "тест про фронтон");
    let frames = builder.roof_coords_for_test().expect("roof coords");
    // нуля не должно быть ни на одной вершине: и стена, и фронтон берут
    // `wall_frame`, а оборудование кровли на этом доме не стоит
    assert!(
        frames.iter().all(|frame| frame[2] > 0.0),
        "у каждой вершины должен быть код рамки"
    );
    // Фронтон строится от уже поднятого карниза, поэтому этажи в нём идут с
    // нуля — и швы всё равно продолжают стенные: верх стены приходится ровно
    // на целый этаж, а целое смещение сетке безразлично.
    //
    // Метка едет посевом: у стены без балконов он в `(-2, -1]`, у фронтона в
    // `(-4, -3]` — на нём нет и окон, они резались бы скатом.
    let marks: Vec<f32> = frames
        .iter()
        .filter(|frame| is_wall(frame[2]))
        .map(|frame| frame[3])
        .collect();
    assert!(
        marks.iter().all(|seed| *seed < 0.0),
        "у частного дома в два этажа балконов нет ни на стене, ни на фронтоне"
    );
    assert!(
        marks.iter().any(|seed| *seed < -2.5),
        "фронтон помечен своей меткой, а не общей «без балконов»"
    );
    assert!(
        marks.iter().any(|seed| (-2.5..0.0).contains(seed)),
        "стена под ним — обычная глухая, окна на ней есть"
    );
}

/// Стена считается **в своих ячейках**, и целость их числа — главное свойство
/// рамы: у края стены не бывает обрезанной панели, под карнизом — полуэтажа.
#[test]
fn a_wall_holds_a_whole_number_of_panels_and_storeys() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(20.0, 40.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    let builder = extruded_mesh(&[block], &[], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");
    let cells: Vec<[f32; 4]> = frames.iter().copied().filter(|f| is_wall(f[2])).collect();
    assert!(!cells.is_empty(), "стены должны нести свою раму");

    let far = cells.iter().fold(0.0_f32, |far, cell| far.max(cell[0]));
    let top = cells.iter().fold(0.0_f32, |top, cell| top.max(cell[1]));
    let bottom = cells.iter().fold(0.0_f32, |low, cell| low.min(cell[1]));
    // 15 м это пять этажей по три, а **верх стены** приходится выше пятого:
    // над последним этажом лежит запас под карниз (`PARAPET_CELLS`), и он
    // настоящий кусок стены, а не полоса, закрашенная внутри верхнего этажа.
    // Целость счёта это не ломает — она про границы этажей, и пятая по-прежнему
    // целая.
    let expected_top = 5.0 + crate::map::meshing::PARAPET_CELLS;
    assert!(
        (top - expected_top).abs() < 1e-3,
        "верх стены — пятый этаж плюс карниз: {top}"
    );
    assert!(bottom.abs() < 1e-3, "низ стены — ноль: {bottom}");
    // длинная стена 40 м по 3.2 — тринадцать панелей, и её край ровно на них
    assert!(
        (far - 13.0).abs() < 1e-3,
        "край стены — целая панель: {far}"
    );
    // пятиэтажный жилой дом — как раз тот, у кого балконы бывают, и рама несёт
    // это положительным посевом. По **длинному фасаду**: у дома 20 на 40 есть
    // и торец, а он глухой — про него отдельный тест
    // (`a_gable_end_carries_no_balconies`), здесь же речь про целость ячеек
    assert!(
        cells.iter().any(|cell| cell[3] >= 0.0),
        "у длинного фасада этого дома балконы должны быть"
    );
    // Этажность едет в том же слоте, что и код, и должна совпасть с той, по
    // которой посчитана координата: иначе шейдер поставит карниз не там, где
    // кончается стена, — а без неё он вовсе не знает, где у стены верх.
    assert!(
        cells.iter().all(|cell| slot_storeys(cell[2]) == 5.0),
        "в слоте материала должны лежать пять этажей"
    );
}

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
    let bay = super::garages::garage_runs(std::slice::from_ref(&ribbon))[&0]
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

/// Где на стене оказались дверные полотна: `x` каждой вершины кода
/// [`DOOR_CODE`], в порядке сборки.
fn door_vertices(building: &PolyArea, kind: WallKind) -> Vec<Vec2> {
    let look = RoofLook::new(RoofKind::Bitumen, Srgba::WHITE, Vec2::X, 0.0);
    let wall = WallLook::new(kind, Srgba::WHITE);
    let mut builder = MeshBuilder::with_roof_coords();
    push_house(
        &mut builder,
        building,
        &look,
        &wall,
        look.base,
        RoofShape::Flat,
        false,
    );
    let frames = builder.roof_coords_for_test().expect("roof coords");
    builder
        .positions_for_test()
        .iter()
        .zip(frames)
        .filter(|(_, frame)| unpack_material(frame[2]).0 == DOOR_CODE)
        .map(|(point, _)| Vec2::new(point[0], point[1]))
        .collect()
}

/// Дверь рисуется **там, где стоит вход из данных**, и это вся суть перемены:
/// пока шейдер разыгрывал вход сам, нарисованная дверь не совпадала ни с
/// гизмой дверей, ни с точкой, к которой идёт пешка.
#[test]
fn a_door_is_drawn_where_the_entrance_stands() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(20.0, 40.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    // южная грань — та, что видна при подъёме вверх-вправо
    block.entrances = vec![Vec2::new(12.0, 0.0)];

    let doors = door_vertices(&block, WallKind::Panel);
    assert!(!doors.is_empty(), "вход из данных должен дойти до меша");
    // низ полотна — на земле, у самого основания стены; верх уехал по
    // подъёму, и мерить ширину по нему нельзя: стена — параллелограмм
    let base: Vec<Vec2> = doors.into_iter().filter(|at| at.y.abs() < 1e-3).collect();
    assert_eq!(base.len(), 2, "у полотна две вершины на земле: {base:?}");
    let left = base.iter().fold(f32::MAX, |left, at| left.min(at.x));
    let right = base.iter().fold(f32::MIN, |right, at| right.max(at.x));
    let half = door_size(WallKind::Panel).x / 2.0;
    assert!(
        (left - (12.0 - half)).abs() < 1e-3 && (right - (12.0 + half)).abs() < 1e-3,
        "полотно стоит на входе: {left}..{right}"
    );
}

/// Дома без размеченного входа — а это дом до `osm::entrances` и всякий дом
/// витрины — не носят дверей вовсе: рисовать нечего.
#[test]
fn a_building_without_entrances_gets_no_doors() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(20.0, 40.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    assert!(door_vertices(&block, WallKind::Panel).is_empty());
}

/// Метки стен одного дома при **заказанной** облицовке. Заказана она потому,
/// что материал дому выбирает посев, а эти тесты не про лотерею: без заказа
/// пятиэтажка раз в десять выпадений оказалась бы штукатуркой, и тест про
/// балконы падал бы по причине, к балконам отношения не имеющей.
fn wall_marks(building: &PolyArea, kind: WallKind) -> Vec<f32> {
    let look = RoofLook::new(RoofKind::Bitumen, Srgba::WHITE, Vec2::X, 0.0);
    let wall = WallLook::new(kind, Srgba::WHITE);
    let mut builder = MeshBuilder::with_roof_coords();
    push_house(
        &mut builder,
        building,
        &look,
        &wall,
        look.base,
        RoofShape::Flat,
        false,
    );
    builder
        .roof_coords_for_test()
        .expect("roof coords")
        .iter()
        .filter(|frame| is_wall(frame[2]))
        .map(|frame| frame[3])
        .collect()
}

/// Балкон — примета жилого дома в несколько этажей, а не всякой стены: на
/// частном доме и на узком простенке его быть не должно.
#[test]
fn balconies_skip_low_houses_and_narrow_walls() {
    let _sun = crate::map::default_sun();

    // частный дом: два этажа, балконам взяться неоткуда
    let mut house = building(oblong(9.0, 18.0), Some(6.0), AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(
        wall_marks(&house, WallKind::Panel)
            .iter()
            .all(|seed| *seed < 0.0),
        "частный дом"
    );

    // тот же дом ростом с пятиэтажку — балконы появляются. По длинному фасаду:
    // короткая сторона у этого плана уже торец, и о ней отдельный тест
    // (`a_gable_end_carries_no_balconies`)
    let mut block = building(oblong(9.0, 18.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    assert!(
        wall_marks(&block, WallKind::Panel)
            .iter()
            .any(|seed| *seed >= 0.0),
        "пятиэтажка"
    );

    // а вот простенок в две панели остаётся глухим и у неё. План тут нарочно
    // почти квадратный (8 на 6 — меньше `GABLE_PLAN_RATIO_MIN`), торцов у него
    // нет вовсе, и порог ширины отвечает за эту стену один: 8 м — три панели,
    // балконы есть; 6 м — две, балконов нет
    let mut stepped = building(oblong(6.0, 8.0), Some(15.0), AreaKind::Building);
    stepped.building_use = BuildingUse::Apartments;
    let seeds = wall_marks(&stepped, WallKind::Panel);
    assert!(seeds.iter().any(|seed| *seed >= 0.0), "стена в три панели");
    assert!(
        seeds.iter().any(|seed| *seed < 0.0),
        "простенок в две панели"
    );
}

/// «Глухие торцы» из постановки: у панельной секции балконы идут по **длинному
/// фасаду**, а торец их не несёт. Отличает их не ширина стены — торец секции
/// это её глубина, 12–14 м, то есть четыре панели, и порог
/// `BALCONY_COLUMNS_MIN` его пропускает, — а то, что торец стоит поперёк
/// длинной оси плана.
#[test]
fn a_gable_end_carries_no_balconies() {
    let _sun = crate::map::default_sun();

    // секция 13 × 44 м: торец вчетверо шире порога ширины и всё равно глухой
    let mut section = building(oblong(13.0, 44.0), Some(15.0), AreaKind::Building);
    section.building_use = BuildingUse::Apartments;
    let seeds = wall_marks(&section, WallKind::Panel);
    assert!(seeds.iter().any(|seed| *seed >= 0.0), "длинный фасад");
    assert!(seeds.iter().any(|seed| *seed < 0.0), "торец секции");

    // а у башни торца нет: план квадратный, длинного фасада в нём не выделить,
    // и балконы несут все стены — иначе половина дома глохла бы по броску
    // округления
    let mut tower = building(oblong(24.0, 30.0), Some(27.0), AreaKind::Building);
    tower.building_use = BuildingUse::Apartments;
    assert!(
        wall_marks(&tower, WallKind::Panel)
            .iter()
            .all(|seed| *seed >= 0.0),
        "башня"
    );
}

/// Пятиэтажка `building=house` — та самая, на которой порог этажности не
/// срабатывает, а балконов всё равно не бывает: `house` это отдельный дом с
/// участком, и ряд балконов на нём читался бы как ошибка разбора.
#[test]
fn a_tall_house_still_gets_no_balconies() {
    let _sun = crate::map::default_sun();
    let mut mansion = building(oblong(12.0, 30.0), Some(15.0), AreaKind::Building);
    mansion.building_use = BuildingUse::House;
    assert!(
        wall_marks(&mansion, WallKind::Panel)
            .iter()
            .all(|seed| *seed < 0.0),
        "у `building=house` балконов нет ни при какой высоте"
    );
}

/// Балкон полагается панели и кирпичу, и только им: штукатурка, витраж и
/// профлист — это частный сектор, торговля и склад, где балкона не бывает по
/// самому смыслу материала.
#[test]
fn only_panel_and_brick_carry_balconies() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(12.0, 40.0), Some(27.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    for kind in WallKind::ALL {
        let marks = wall_marks(&block, kind);
        assert!(!marks.is_empty(), "стены должны нести раму: {kind:?}");
        let with = marks.iter().any(|seed| *seed >= 0.0);
        let expected = matches!(kind, WallKind::Panel | WallKind::Brick);
        assert_eq!(with, expected, "балконы у {kind:?}");
    }
}

/// Облицовку выбирают назначение и **рост**, причём рост первым: низкий дом не
/// бывает ни панельным, ни витражным, чем бы он ни был по тегу.
#[test]
fn the_cladding_follows_the_use_and_the_height() {
    let _sun = crate::map::default_sun();
    let of = |use_: BuildingUse, height: f32| {
        let mut area = building(oblong(14.0, 40.0), Some(height), AreaKind::Building);
        area.building_use = use_;
        wall_of(&area).kind
    };
    // Кремль кирпичный, храм белёный — оба мимо таблиц, как и у кровель
    let mut kremlin = building(oblong(14.0, 40.0), Some(12.0), AreaKind::Kremlin);
    kremlin.building_use = ORTHODOX;
    assert_eq!(wall_of(&kremlin).kind, WallKind::Brick, "Кремль");
    assert_eq!(of(ORTHODOX, 12.0), WallKind::Sacred, "храм");

    // частный дом и гараж идут по своим таблицам мимо развилки по росту, и
    // панели среди них нет ни в одном слоте
    for height in [6.0, 27.0] {
        assert_ne!(of(BuildingUse::House, height), WallKind::Panel, "дом");
        assert_ne!(of(BuildingUse::Garage, height), WallKind::Panel, "гараж");
    }

    // а всё остальное низкое уходит в малоэтажную таблицу, где панели тоже нет
    for use_ in [
        BuildingUse::Apartments,
        BuildingUse::Commercial,
        BuildingUse::Public,
        BuildingUse::Other,
    ] {
        assert_ne!(of(use_, 6.0), WallKind::Panel, "низкий {use_:?}");
    }
}

/// Каждая облицовка **из таблиц** обязана встречаться в городе. Тест не про
/// красоту: он ловит и мёртвый слот в таблице, и слабый разбор посева — сетка
/// домов ровным шагом это ровно тот вход, на котором плохо перемешанный хеш
/// выстраивается в узор, и в витрине это выглядело бы как «на девяти этажах
/// всегда штукатурка».
///
/// Ворота из перебора исключены, и по той же причине, по какой гаражные ленты
/// не входят в `RoofKind::ALL`: их выбирает **геометрия прогона**
/// (`layers::wall_of_run`), а не назначение дома, так что через `wall_of` они
/// не приходят никогда.
#[test]
fn every_cladding_reaches_the_city() {
    let _sun = crate::map::default_sun();
    let mut seen: Vec<WallKind> = Vec::new();
    for row in 0..12 {
        for column in 0..12 {
            let at = Vec2::new(column as f32 * 31.0, row as f32 * 23.0);
            let outer: Vec<Vec2> = oblong(14.0, 40.0).into_iter().map(|p| p + at).collect();
            // назначения по кругу, высоты по кругу — так перебираются обе
            // развилки сразу
            let mut area = building(outer, Some([6.0, 15.0, 27.0][row % 3]), AreaKind::Building);
            area.building_use = [
                BuildingUse::House,
                BuildingUse::Apartments,
                BuildingUse::Commercial,
                BuildingUse::Industrial,
                BuildingUse::Public,
                BuildingUse::Other,
                ORTHODOX,
            ][column % 7];
            let kind = wall_of(&area).kind;
            if !seen.contains(&kind) {
                seen.push(kind);
            }
        }
    }
    for kind in WallKind::ALL {
        if kind == WallKind::GarageDoors {
            continue;
        }
        assert!(seen.contains(&kind), "{kind:?} не встретилась ни разу");
    }
}

#[test]
fn a_pitched_roof_goes_on_houses_and_small_boxes_only() {
    let _sun = crate::map::default_sun();
    let mut house = building(oblong(8.0, 600.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(is_pitched(&house), "a house of any size");
    let small = building(square(), None, AreaKind::Building);
    assert!(is_pitched(&small), "an untagged small box");
    let big = building(oblong(20.0, 20.0), None, AreaKind::Building);
    assert!(!is_pitched(&big), "an untagged big box");
    let mut flats = building(square(), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(!is_pitched(&flats));
    let tower = building(square(), None, AreaKind::Kremlin);
    assert!(!is_pitched(&tower), "the kremlin keeps its flat roof");
    let mut yard = house.clone();
    yard.holes.push(vec![
        Vec2::new(4.0, 2.0),
        Vec2::new(6.0, 2.0),
        Vec2::new(6.0, 4.0),
        Vec2::new(4.0, 4.0),
    ]);
    assert!(!is_pitched(&yard), "a courtyard has no ridge");
}

#[test]
fn the_ridge_runs_along_the_long_axis_of_the_rotated_footprint() {
    let _sun = crate::map::default_sun();
    // прямоугольник 20 × 6, повёрнутый на 30°, обойдённый по часовой
    let rotate = |p: Vec2| Vec2::from_angle(30f32.to_radians()).rotate(p);
    let mut ring: Vec<Vec2> = oblong(6.0, 20.0).into_iter().map(rotate).collect();
    ring.reverse();
    let rect = min_area_rect(&ring).unwrap();
    assert!(
        (rect[1] - rect[0]).length() > 19.9,
        "long side first: {rect:?}"
    );
    assert!(
        (rect[2] - rect[1]).length() < 6.1,
        "short side second: {rect:?}"
    );
    assert!(signed_ring_area(&rect) > 0.0, "CCW");
    // конёк — вдоль длинной оси
    let mut house = building(ring, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let roof = gable_roof(&house, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).unwrap();
    let ridge = roof.slopes[0].0[2] - roof.slopes[0].0[3];
    let long = rect[1] - rect[0];
    assert!(ridge.normalize().dot(long.normalize()).abs() > 0.999);
}

#[test]
fn an_l_shaped_house_is_pitched_but_takes_no_gable() {
    let _sun = crate::map::default_sun();
    let l_shape = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(12.0, 0.0),
        Vec2::new(12.0, 5.0),
        Vec2::new(5.0, 5.0),
        Vec2::new(5.0, 12.0),
        Vec2::new(0.0, 12.0),
    ];
    let mut house = building(l_shape, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(is_pitched(&house));
    assert!(gable_roof(&house, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_none());
}

#[test]
fn a_skewed_quad_takes_no_gable_whose_corner_hangs_off_the_wall() {
    let _sun = crate::map::default_sun();
    // Тула, way 968419942: заполняет свой прямоугольник на 0.91, а угол крыши
    // над ним висит в 2.2 м от стены — торец дома читался срезанным
    let skewed = vec![
        Vec2::new(412.56, 3158.32),
        Vec2::new(426.85, 3167.85),
        Vec2::new(420.76, 3178.18),
        Vec2::new(407.79, 3169.97),
    ]
    .into_iter()
    .map(|point| point - Vec2::new(400.0, 3150.0))
    .collect();
    let mut house = building(skewed, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let facts = shape_facts(&house).unwrap();
    assert!(
        facts.rect_fill > 0.85,
        "fill alone would let it through: {}",
        facts.rect_fill
    );
    assert!(facts.gable_overhang > 2.0, "{}", facts.gable_overhang);
    assert!(gable_roof(&house, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_none());
    // посев, по которому дому выпадает двускатная (`(seed >> 5) % 10 >= 4`)
    let gabled_seed = 5 << 5;
    assert!(matches!(
        roofing(
            &house,
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            gabled_seed
        ),
        Roofing::Hip(_)
    ));

    // слегка неровная обводка прямоугольника двускатную не теряет
    let mut sloppy = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(12.0, 0.2),
            Vec2::new(12.0, 8.0),
            Vec2::new(0.1, 8.0),
        ],
        None,
        AreaKind::Building,
    );
    sloppy.building_use = BuildingUse::House;
    assert!(gable_roof(&sloppy, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_some());
}

#[test]
fn gable_roof_itself_refuses_a_building_that_is_not_gabled() {
    let _sun = crate::map::default_sun();
    let mut flats = building(oblong(8.0, 20.0), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(gable_roof(&flats, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE).is_none());
}

#[test]
fn the_slope_facing_the_light_is_lighter_and_the_ridge_is_lifted() {
    let _sun = crate::map::default_sun();
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    let mut house = building(oblong(8.0, 20.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let base = Srgba::rgb(0.5, 0.5, 0.5);
    let roof = gable_roof(&house, Vec2::ZERO, |rise| Lean::of().ridge(rise), base).unwrap();
    // скаты: южный (карниз y = 0) отвёрнут от света, северный повёрнут
    let (south, north) = (&roof.slopes[0], &roof.slopes[1]);
    assert_eq!(south.0[0].y, 0.0);
    assert!(luminance(south.1) < luminance(base.into()));
    assert!(luminance(north.1) > luminance(base.into()));
    // конёк поднят по вектору подъёма на масштаб стен
    let ridge = south.0[3] - Vec2::new(0.0, 4.0);
    let expected = Lean::of().ridge(ridge_rise(8.0));
    assert!(ridge.distance(expected) < 1e-4, "{ridge:?} vs {expected:?}");
    assert!(expected.y > 0.0 && expected.x > 0.0);
    // конёк — вдоль длинной оси, оба фронтона стоят на торцах
    for ((a, b), face) in roof.gables {
        assert!((b - a).length() < 8.1, "gable on the short side");
        assert!(face[2].distance((a + b) / 2.0 + expected) < 1e-4);
    }
}

#[test]
fn a_house_without_height_stays_low() {
    let _sun = crate::map::default_sun();
    // квадрат 10 × 10 — мелкое пятно: частный дом это один-два этажа, дом без
    // назначения на таком пятне тоже низкий, но выше. Разбор вывода — в
    // тестах `heights`, здесь достаточно, что дефолт больше не один на всех
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let other = building(square(), None, AreaKind::Building);
    assert!(height_or_default(&house) <= 8.0);
    assert!(height_or_default(&other) <= 12.0);
}

#[test]
fn every_mode_builds_geometry_for_mixed_input() {
    let _sun = crate::map::default_sun();
    let mut with_hole = building(square(), Some(20.0), AreaKind::Building);
    with_hole.holes.push(vec![
        Vec2::new(4.0, 4.0),
        Vec2::new(6.0, 4.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(4.0, 6.0),
    ]);
    let list = [
        with_hole,
        building(square(), None, AreaKind::Building),
        building(square(), Some(12.0), AreaKind::Kremlin),
    ];

    let (facades, roofs) = facade_and_roof_builders(&list, &[], detail(true));
    assert!(!facades.is_empty());
    assert!(!roofs.is_empty());
    assert_eq!(facades.skipped_polygons(), 0);

    let shadows = ground_shadows(&list, &[], false);
    assert!(!shadows.is_empty());

    let extruded = extruded_mesh(&list, &[], detail(false));
    assert!(!extruded.is_empty());
    assert_eq!(extruded.skipped_polygons(), 0);
    // комбинированный режим: рампа меняет цвета, но не геометрию
    let tinted = extruded_mesh(&list, &[], detail(true));
    assert!(!tinted.is_empty());
    assert_eq!(tinted.skipped_polygons(), 0);
}

#[test]
fn the_painter_order_puts_the_far_side_first() {
    let _sun = crate::map::default_sun();
    // «дальше» — вдоль отклонения верха: верх дальнего дома уезжает на
    // ближний, и ближний обязан лечь поверх, то есть попасть в буфер позже
    let lean = Lean::of();
    let dir = lean.dir();
    let near = lean.depth(Vec2::ZERO);
    let far = lean.depth(dir * 900.0);
    assert!(far > near);
    // поперёк отклонения глубина не меняется: сортировать там нечего
    assert_eq!(lean.depth(dir.perp() * 700.0), near);
}

/// Тульская пара из отчёта: Г-образная пятиэтажка (её северо-восточное крыло
/// уходит за соседа) и девятиэтажка к востоку. Числа — пятна 493864392 и
/// 493864388, сдвинутые в ноль. Высоты в OSM у них не проставлены и в игре
/// выводятся по форме и посеву; здесь они заданы явно — те самые 15 и 27 м,
/// чтобы тест держал геометрию, а не таблицу этажности.
fn leaning_pair() -> (PolyArea, PolyArea) {
    let low = building(
        vec![
            Vec2::new(14.2, 0.9),
            Vec2::new(4.2, 0.0),
            Vec2::new(0.0, 42.4),
            Vec2::new(30.3, 45.4),
            Vec2::new(31.2, 36.6),
            Vec2::new(10.9, 34.6),
        ],
        Some(15.0),
        AreaKind::Building,
    );
    let tall = building(
        vec![
            Vec2::new(44.3, 23.4),
            Vec2::new(45.1, 6.9),
            Vec2::new(21.9, 5.7),
            Vec2::new(20.6, 31.8),
            Vec2::new(52.5, 33.5),
            Vec2::new(52.9, 23.8),
        ],
        Some(27.0),
        AreaKind::Building,
    );
    (low, tall)
}

#[test]
fn the_order_puts_the_leaning_neighbour_over_the_wing_it_covers() {
    let _sun = crate::map::default_sun();
    let (low, tall) = leaning_pair();
    let lean = Lean::of();
    // случай ловится только тогда, когда прежний ключ на нём и ошибается:
    // центр пятна девятиэтажки дальше по подъёму, значит она писалась первой
    assert!(lean.depth(building_center(&tall)) > lean.depth(building_center(&low)));
    // а кроет она: её кровля поднята на 27 м и ложится на крыло соседа
    assert_eq!(
        order::draw_order(&[low.clone(), tall.clone()], lean),
        vec![0, 1]
    );
    // порядок входа на это не влияет — решает геометрия, а не индекс
    assert_eq!(order::draw_order(&[tall, low], lean), vec![1, 0]);
}

#[test]
fn the_order_writes_every_building_once() {
    let _sun = crate::map::default_sun();
    let (low, tall) = leaning_pair();
    let mut list = vec![low, tall];
    // ряд сцепленных домов: пары спорят друг с другом, и порядок обязан
    // остаться полным даже там, где отношение зациклилось
    for step in 1..6 {
        let shift = Vec2::new(step as f32 * 9.0, step as f32 * 7.0);
        let mut next = building(
            rect(shift, shift + Vec2::new(24.0, 18.0)),
            Some(9.0 * step as f32),
            AreaKind::Building,
        );
        next.building_use = BuildingUse::Apartments;
        list.push(next);
    }
    let order = order::draw_order(&list, Lean::of());
    let mut seen = order.clone();
    seen.sort_unstable();
    assert_eq!(seen, (0..list.len()).collect::<Vec<_>>());
}

/// Дом со ступенчатым фасадом: западная секция выдвинута к зрителю на 3 м,
/// восточная стоит за ней. Ровно тот случай, на котором обход контура даёт
/// обратный порядок — южные стены секций встречаются на экране на ширину
/// ступени.
fn stepped_facade() -> PolyArea {
    building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(20.0, 3.0),
            Vec2::new(40.0, 3.0),
            Vec2::new(40.0, 20.0),
            Vec2::new(0.0, 20.0),
        ],
        Some(15.0),
        AreaKind::Building,
    )
}

#[test]
fn the_wall_order_puts_the_stepped_back_section_first() {
    let _sun = crate::map::default_sun();
    let stepped = stepped_facade();
    let lean = Lean::of();
    let walls = silhouette_edges(&stepped.outer, -lean.dir());
    // видимы три стены: два южных фасада секций и западный торец
    let near = walls
        .iter()
        .position(|(a, b)| a.y == 0.0 && b.y == 0.0)
        .expect("near section wall");
    let far = walls
        .iter()
        .position(|(a, b)| a.y == 3.0 && b.y == 3.0)
        .expect("far section wall");
    // обход контура кладёт ближнюю раньше дальней — это и есть та ошибка
    assert!(near < far, "ring order walks the near wall first");

    let lift = extrusion_lift(&stepped, BuildingHeightMode::Extrusion);
    let order = order::wall_order(&walls, lean, lift);
    let at = |wall: usize| order.iter().position(|&index| index == wall).unwrap();
    assert!(
        at(far) < at(near),
        "the near wall must cover the far one: {order:?}"
    );
    let mut seen = order.clone();
    seen.sort_unstable();
    assert_eq!(seen, (0..walls.len()).collect::<Vec<_>>());
}

#[test]
fn the_stepped_facade_reaches_the_mesh_near_wall_last() {
    let _sun = crate::map::default_sun();
    let stepped = stepped_facade();
    let mesh = extruded_mesh(&[stepped], &[], detail(false)).build();
    let points = mesh_points(&mesh);
    // угол, который есть только у своей стены: (20, 0) — у ближней,
    // (40, 3) — у дальней
    let first = |corner: Vec2| {
        points
            .iter()
            .position(|p| p.distance(corner) < 0.01)
            .unwrap_or_else(|| panic!("no vertex at {corner:?}"))
    };
    assert!(
        first(Vec2::new(40.0, 3.0)) < first(Vec2::new(20.0, 0.0)),
        "the far section's wall goes into the buffer first"
    );
}

/// Площадь плоской фигуры по её квадам и полигонам — для проверки, что крыша
/// накрыла пятно целиком и ровно один раз.
fn hip_area(roof: &HipRoof) -> f32 {
    let quad = |corners: &[Vec2; 4]| {
        (corners[1] - corners[0])
            .perp_dot(corners[2] - corners[0])
            .abs()
            / 2.0
            + (corners[2] - corners[0])
                .perp_dot(corners[3] - corners[0])
                .abs()
                / 2.0
    };
    roof.slopes
        .iter()
        .map(|(slope, _)| quad(slope))
        .sum::<f32>()
        + signed_ring_area(&roof.ridge.0).abs()
}

#[test]
fn a_hip_roof_covers_the_footprint_exactly_once() {
    let _sun = crate::map::default_sun();
    // плоский режим: скаты и площадка конька лежат в одной плоскости, и их
    // площади обязаны сложиться в площадь пятна — ни дыр, ни нахлёстов
    let plot = oblong(10.0, 20.0);
    let base = Srgba::WHITE;
    let Roofing::Hip(roof) = hip_only(&house(plot.clone()), base) else {
        panic!("a rectangle must accept a hip roof");
    };
    let footprint = signed_ring_area(&plot).abs();
    assert!(
        (hip_area(&roof) - footprint).abs() < 0.5,
        "{} vs {footprint}",
        hip_area(&roof)
    );
    // скатов ровно по ребру контура
    assert_eq!(roof.slopes.len(), plot.len());
}

#[test]
fn an_l_shaped_house_gets_a_cross_gable_instead_of_a_hip_or_a_flat_one() {
    let _sun = crate::map::default_sun();
    // Г-образный дом одной двускатной не накрыть — прямоугольник торчал бы из
    // него, — и в частном секторе он кроется двумя с ендовой, а не вальмой
    let ell = house(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(14.0, 0.0),
        Vec2::new(14.0, 6.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(6.0, 14.0),
        Vec2::new(0.0, 14.0),
    ]);
    for seed in [0, 1 << 5, 7 << 5, 12345] {
        let Roofing::Gable(roof) = roofing(&ell, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE, seed)
        else {
            panic!("an L takes a cross gable");
        };
        assert_eq!(RoofShape::of_gable(&roof), RoofShape::Cross);
        // корпус — два ската и два фронтона, крыло — ещё два и один торец
        assert_eq!(roof.slopes.len(), 4);
        assert_eq!(roof.gables.len(), 3);
    }
    // а плоская кровля так и остаётся у того, кому она положена
    let mut block = building(oblong(30.0, 80.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    assert!(matches!(
        roofing(&block, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE, 0),
        Roofing::Flat
    ));
}

#[test]
fn a_private_house_is_gabled_and_almost_never_hipped_or_flat() {
    let _sun = crate::map::default_sun();
    // частный дом 10 × 8: на снимке частного сектора вальмы и плоской кровли
    // у такого почти не бывает
    let cottage = house(oblong(8.0, 10.0));
    let (mut gabled, mut other) = (0, 0);
    for seed in 0..400u32 {
        match roofing(
            &cottage,
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            seed.wrapping_mul(2654435761),
        ) {
            Roofing::Gable(roof) => match roof.form {
                GableForm::Gable => gabled += 1,
                _ => other += 1,
            },
            Roofing::Hip(_) => panic!("a small house took a hip"),
            Roofing::Flat | Roofing::Tent(_) => panic!("a house lost its pitched roof"),
        }
    }
    assert!(
        gabled * 2 > gabled + other,
        "{gabled} gables of {}",
        gabled + other
    );
    assert!(other > 0, "no half-hips or gambrels at all");
}

#[test]
fn every_gable_form_covers_its_rectangle_exactly_once() {
    let _sun = crate::map::default_sun();
    // плоский режим: грани крыши лежат в плане и обязаны сложиться в
    // прямоугольник дома — ни дыр, ни нахлёстов
    let cottage = house(oblong(8.0, 12.0));
    for shape in [
        RoofShape::Gable,
        RoofShape::HalfHip,
        RoofShape::Gambrel,
        RoofShape::LeanTo,
    ] {
        let Roofing::Gable(roof) =
            roofing_of(shape, &cottage, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE, 0)
        else {
            panic!("{shape:?} did not build");
        };
        assert_eq!(RoofShape::of_gable(&roof), shape);
        let covered: f32 = roof
            .slopes
            .iter()
            .map(|(face, _)| signed_ring_area(face).abs())
            .sum();
        assert!((covered - 96.0).abs() < 0.1, "{shape:?} covers {covered}");
    }
}

#[test]
fn the_cross_gable_wing_meets_the_main_ridge_no_higher_than_it() {
    let _sun = crate::map::default_sun();
    let ell = house(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(14.0, 0.0),
        Vec2::new(14.0, 6.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(6.0, 14.0),
        Vec2::new(0.0, 14.0),
    ]);
    // подъём уводит точку на тысячу метров вверх за метр высоты: план (до
    // 14 м) в сдвиге теряется, и высота читается прямо из `y`
    let Roofing::Gable(roof) = roofing_of(
        RoofShape::Cross,
        &ell,
        Vec2::ZERO,
        |rise| Vec2::new(0.0, rise * 1000.0),
        Srgba::WHITE,
        0,
    ) else {
        panic!("an L takes a cross gable");
    };
    let top = |faces: &[(Vec<Vec2>, LinearRgba)]| {
        faces
            .iter()
            .flat_map(|(face, _)| face.iter())
            .map(|point| point.y / 1000.0)
            .fold(f32::MIN, f32::max)
    };
    let (main, wing) = (top(&roof.slopes[..2]), top(&roof.slopes[2..]));
    assert!((main - ridge_rise(6.0)).abs() < 0.02, "main ridge {main}");
    assert!(
        wing <= main + 0.02,
        "wing ridge {wing} over the main {main}"
    );
    assert!(
        wing > 0.5 * main,
        "wing ridge {wing} sank into the main roof"
    );
}

#[test]
fn dormers_stand_on_a_lifted_roof_only() {
    let _sun = crate::map::default_sun();
    let cottage = house(oblong(9.0, 12.0));
    let lean = Lean::of();
    let Roofing::Gable(lifted) = roofing_of(
        RoofShape::Dormer,
        &cottage,
        Vec2::ZERO,
        |rise| lean.ridge(rise),
        Srgba::WHITE,
        0,
    ) else {
        panic!("a dormer roof did not build");
    };
    assert!(!lifted.dormers.is_empty());
    assert_eq!(RoofShape::of_gable(&lifted), RoofShape::Dormer);
    // в плоском режиме окно было бы заплатой на скате
    let Roofing::Gable(flat) = roofing_of(
        RoofShape::Dormer,
        &cottage,
        Vec2::ZERO,
        |_| Vec2::ZERO,
        Srgba::WHITE,
        0,
    ) else {
        panic!("a dormer roof did not build flat");
    };
    assert!(flat.dormers.is_empty());
}

#[test]
fn a_yard_shed_is_low_and_mostly_lean_to() {
    let _sun = crate::map::default_sun();
    let (mut lean_to, mut total) = (0, 0);
    for index in 0..60 {
        let at = Vec2::new(index as f32 * 23.0, index as f32 * 17.0);
        let ring: Vec<Vec2> = oblong(4.0, 6.0).into_iter().map(|p| p + at).collect();
        let shed = building(ring, None, AreaKind::Building);
        assert!(height_or_default(&shed) <= 3.0, "a shed is one low box");
        if let Roofing::Gable(roof) = roofing(
            &shed,
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            building_seed(&shed),
        ) {
            total += 1;
            lean_to += usize::from(roof.form == GableForm::LeanTo);
        }
    }
    assert_eq!(total, 60);
    assert!(lean_to * 2 > total, "{lean_to} lean-to of {total}");
}

#[test]
fn a_private_house_is_mostly_one_storey() {
    let _sun = crate::map::default_sun();
    let mut one = 0;
    for index in 0..80 {
        let at = Vec2::new(index as f32 * 31.0, index as f32 * 19.0);
        let ring: Vec<Vec2> = oblong(9.0, 11.0).into_iter().map(|p| p + at).collect();
        let cottage = house(ring.clone());
        let untagged = building(ring, None, AreaKind::Building);
        for building in [cottage, untagged] {
            let height = height_or_default(&building);
            assert!(height <= 6.5, "{height} m is not a private house");
            one += usize::from(height < 4.5);
        }
    }
    assert!(one * 10 >= 160 * 6, "only {one} of 160 are one storey");
}

/// Вальма, заказанная явно: посевом она частному дому почти не выпадает.
fn hip_only(building: &PolyArea, base: Srgba) -> Roofing {
    roofing_of(
        RoofShape::Hip,
        building,
        Vec2::ZERO,
        |_| Vec2::ZERO,
        base,
        0,
    )
}

#[test]
fn roof_tint_darkens_tall_buildings_and_spares_the_kremlin() {
    let _sun = crate::map::default_sun();
    let color = |height, tinted| {
        let b = building(square(), height, AreaKind::Building);
        roof_color(&b, &roof_look(&b), tinted)
    };
    let base = color(None, true);
    let tall = color(Some(60.0), true);
    // именно темнее в сумме, а не в каждом канале: рампа ведёт к нейтральному
    // тёмному, и у зеленоватой кровли её зелёный канал почти не двигается
    let luminance = |color: Srgba| color.red + color.green + color.blue;
    assert!(luminance(tall) < luminance(base), "{tall:?} vs {base:?}");

    let kremlin = |height, tinted| {
        let b = building(square(), height, AreaKind::Kremlin);
        roof_color(&b, &roof_look(&b), tinted)
    };
    assert_eq!(kremlin(Some(60.0), true), kremlin(None, false));
}

#[test]
fn the_tint_ramp_darkens_every_palette_colour() {
    let _sun = crate::map::default_sun();
    // Цель рампы обязана быть темнее любого цвета любой палитры, иначе на
    // высоком доме рампа переворачивается и осветляет: у насыщенной красной
    // черепицы сумма каналов ниже, чем у среднего серого. Проверяется на
    // каждом цвете — палитры теперь не одни серые.
    let tall = building(square(), Some(60.0), AreaKind::Building);
    let luminance = |color: Srgba| color.red + color.green + color.blue;
    let palettes = RoofKind::ALL
        .iter()
        .map(|kind| kind.palette())
        .chain(std::iter::once(&CHURCH_ROOF_COLORS as &[Color]));
    for palette in palettes {
        for color in palette {
            let look = RoofLook::new(RoofKind::Tile, color.to_srgba(), Vec2::X, 0.0);
            let ramped = roof_color(&tall, &look, true);
            let flat = roof_color(&tall, &look, false);
            assert!(
                luminance(ramped) < luminance(flat),
                "{color:?}: ramped {ramped:?} vs flat {flat:?}"
            );
        }
    }
}

// --- слои целиком ------------------------------------------------------
//
// Тесты на `mesh_buildings`. До шва слои собирались внутри системы Bevy, и
// достать сборку из теста было нечем: проверять можно было только билдеры под
// ней. Здания — единственный модуль карты, отдающий **два** списка
// (`BuildingMeshes { layers, shadows }`) под разными метками и с разным
// расписанием пересборки, и держит это разделение только здешняя секция.

/// Квадрат 10 × 10 с высотой — тот же дом, на котором стоят тесты билдеров
/// выше.
fn one_building() -> PolyArea {
    building(square(), Some(9.0), AreaKind::Building)
}

/// План сборки на дальней ступени зума: оборудование на кровле выключено —
/// эти тесты про состав слоёв, а коробки на крышах только добавили бы вершин
/// (то же решение, что у хелпера [`detail`]).
fn building_plan(mode: BuildingHeightMode, shadows: bool) -> BuildingPlan {
    BuildingPlan {
        mode,
        bucket: BuildingZoomBucket::default(),
        shadows,
    }
}

fn layer_names(layers: &[LayerMesh]) -> Vec<&'static str> {
    layers.iter().map(|layer| layer.name).collect()
}

#[test]
fn extrusion_builds_one_layer_and_the_flat_modes_split_facade_from_roof() {
    let _sun = crate::map::default_sun();
    let buildings = [one_building()];

    // 2.5D: стены и кровля в одном меше, и весь он идёт кровельным материалом
    let (extruded, _) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &buildings,
        &[],
    );
    assert_eq!(layer_names(&extruded.layers), ["building_extruded"]);
    assert_eq!(extruded.layers[0].material, MaterialSpec::Roof);

    // плоские режимы: полоса фасада отдельным слоем и **ниже** кровли — крыша
    // соседа сверху прикрывает полосу
    let (flat, _) = mesh_buildings(
        building_plan(BuildingHeightMode::Facade, false),
        &buildings,
        &[],
    );
    assert_eq!(
        layer_names(&flat.layers),
        ["building_facades", "building_roofs"]
    );
    assert_eq!(flat.layers[0].material, MaterialSpec::Flat);
    assert_eq!(flat.layers[1].material, MaterialSpec::Roof);
    assert!(
        flat.layers[0].z < flat.layers[1].z,
        "фасад лежит ниже крыши"
    );
    assert!(flat.layers.iter().all(|layer| !layer.builder.is_empty()));
}

#[test]
fn both_shadow_layers_ride_their_own_list_around_the_building_ones() {
    let _sun = crate::map::default_sun();
    let (built, report) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, true),
        &[one_building()],
        &[],
    );

    assert_eq!(
        layer_names(&built.shadows),
        ["building_shadows", "roof_shadows"]
    );
    // обе тени полупрозрачны, отсюда `Blend`
    assert!(
        built
            .shadows
            .iter()
            .all(|layer| layer.material == MaterialSpec::Blend)
    );
    // наземная тень — под всеми зданиевыми слоями (её маскирует стена соседа),
    // тень на кровле — над ними: это единственная часть тени, которая обязана
    // лежать поверх крыши
    let ground = built.shadows[0].z;
    let on_roofs = built.shadows[1].z;
    assert!(
        built
            .layers
            .iter()
            .all(|layer| ground < layer.z && layer.z < on_roofs),
        "теневые рунги обязаны обнимать зданиевые"
    );
    assert!(report.vertices > 0);
}

#[test]
fn the_shadow_flag_leaves_the_list_empty_without_touching_the_building_layers() {
    let _sun = crate::map::default_sun();
    let buildings = [one_building()];
    let (kept, quiet) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, false),
        &buildings,
        &[],
    );
    let (whole, full) = mesh_buildings(
        building_plan(BuildingHeightMode::ExtrusionShadowsTint, true),
        &buildings,
        &[],
    );

    // `shadows: false` — «теневой слой оставить как есть»: список пуст, а не с
    // пустыми мешами, потому что адаптер в этом случае старые теневые сущности
    // не деспавнит, и класть поверх них нечего
    assert!(kept.shadows.is_empty());
    // а сами зданиевые слои от тумблера не зависят
    assert_eq!(layer_names(&kept.layers), layer_names(&whole.layers));
    // и вершины теней считаются в общий отчёт, а не мимо него
    assert!(full.vertices > quiet.vertices);
}

#[test]
fn a_mode_without_long_shadows_builds_none_however_the_plan_asks() {
    let _sun = crate::map::default_sun();
    let (built, _) = mesh_buildings(
        building_plan(BuildingHeightMode::Facade, true),
        &[one_building()],
        &[],
    );

    // длинные тени рисуют не все режимы (`casts_shadows`), и план их не
    // переспорит
    assert!(built.shadows.is_empty());
}
