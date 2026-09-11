//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе: каждый тест здесь строит освещённую геометрию, поэтому каждый
//! берёт `default_sun()` — гард, который держит солнце на дефолте и не пускает
//! к нему соседа (`map/sun.rs`).

use super::arches::*;
use super::layers::*;
use super::material::*;
use super::roofs::*;
use super::*;
use crate::map::meshing::{WallMark, unpack_material};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, BuildingUse, fixture};
use crate::map::shadow_dir;
use crate::settings::ARCH_HEIGHT;

/// Стена ли это, если смотреть на слот материала так, как смотрит шейдер, —
/// числом с плавающей точкой из вершинного атрибута. В слоте лежат два числа
/// сразу (код и этажность), кровля и стена делят его на двоих, и разбирать
/// его в каждом тесте по-своему — верный способ разойтись со словарём.
fn is_wall(slot: f32) -> bool {
    WallKind::is_code(unpack_material(slot).0)
}

/// Сколько этажей записано в слоте.
fn slot_storeys(slot: f32) -> f32 {
    unpack_material(slot).1
}

/// Помечена ли поверхность как «стена без проёмов» — так, как эту метку
/// читает шейдер, но через продакшн-разбор ([`WallMark::of_seed`], зеркало
/// `roof.wgsl::wall_shade`): порог кодировки лежит там, а повторить его здесь
/// своим литералом — верный способ разойтись со словарём.
fn is_solid(seed: f32) -> bool {
    WallMark::of_seed(seed) == WallMark::Solid
}

/// Целое ли это число клеток — с допуском на арифметику подъёма.
fn whole_cells(cells: f32) -> bool {
    (cells - cells.round()).abs() < 5e-3
}

fn square() -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ]
}

/// Подробность слоя для теста: рампа тона по вкусу, оборудование на кровле
/// выключено — эти тесты про геометрию домов, а коробки на крышах только
/// добавили бы им вершин.
fn detail(tinted: bool) -> RoofDetail {
    RoofDetail {
        tinted,
        clutter: false,
    }
}

fn building(outer: Vec<Vec2>, height: Option<f32>, kind: AreaKind) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use: BuildingUse::Other,
        height,
        entrances: Vec::new(),
    }
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
        extrusion_builder(list, &[], detail(false))
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
    church.building_use = BuildingUse::Church;
    let mut industrial = building(square(), None, AreaKind::Building);
    industrial.building_use = BuildingUse::Industrial;
    let mut kremlin = building(square(), None, AreaKind::Kremlin);
    kremlin.building_use = BuildingUse::Church;

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
    church.building_use = BuildingUse::Church;
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
    let builder = extrusion_builder(&[block], &[], detail(false));
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
    let builder = extrusion_builder(&[block], &[], detail(false));
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
    kremlin.building_use = BuildingUse::Church;
    assert_eq!(wall_of(&kremlin).kind, WallKind::Brick, "Кремль");
    assert_eq!(of(BuildingUse::Church, 12.0), WallKind::Plaster, "храм");

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

/// Все пять облицовок обязаны встречаться в городе. Тест не про красоту: он
/// ловит и мёртвый слот в таблице, и слабый разбор посева — сетка домов ровным
/// шагом это ровно тот вход, на котором плохо перемешанный хеш выстраивается в
/// узор, и в витрине это выглядело бы как «на девяти этажах всегда
/// штукатурка».
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
            ][column % 6];
            let kind = wall_of(&area).kind;
            if !seen.contains(&kind) {
                seen.push(kind);
            }
        }
    }
    for kind in WallKind::ALL {
        assert!(seen.contains(&kind), "{kind:?} не встретилась ни разу");
    }
}

fn oblong(width: f32, length: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(length, 0.0),
        Vec2::new(length, width),
        Vec2::new(0.0, width),
    ]
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
    for ((a, b), apex) in roof.gables {
        assert!((b - a).length() < 8.1, "gable on the short side");
        assert!(apex.distance((a + b) / 2.0 + expected) < 1e-4);
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
fn shadow_length_scales_with_height() {
    let _sun = crate::map::default_sun();
    let low = building(square(), Some(6.0), AreaKind::Building);
    let high = building(square(), Some(60.0), AreaKind::Building);
    let reach = |list: &[PolyArea]| {
        let mesh = shadow_builder(list, &[], false).build();
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .to_vec();
        positions
            .iter()
            .map(|p| Vec2::new(p[0], p[1]).dot(shadow_dir()))
            .fold(f32::NEG_INFINITY, f32::max)
    };
    assert!(reach(std::slice::from_ref(&high)) > reach(std::slice::from_ref(&low)) + 10.0);
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

    let shadows = shadow_builder(&list, &[], false);
    assert!(!shadows.is_empty());

    let extruded = extrusion_builder(&list, &[], detail(false));
    assert!(!extruded.is_empty());
    assert_eq!(extruded.skipped_polygons(), 0);
    // комбинированный режим: рампа меняет цвета, но не геометрию
    let tinted = extrusion_builder(&list, &[], detail(true));
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

/// Прямоугольник по двум углам — для домов, у которых важно пятно, а не
/// форма.
fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
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
    let mesh = extrusion_builder(&[stepped], &[], detail(false)).build();
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

fn house(outer: Vec<Vec2>) -> PolyArea {
    let mut house = building(outer, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    house
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
fn an_l_shaped_house_gets_a_hip_roof_instead_of_a_flat_one() {
    let _sun = crate::map::default_sun();
    // Г-образный дом двускатную не принимает — прямоугольник торчал бы из
    // него, — и до сих пор оставался плоским среди скатных соседей
    let ell = house(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(14.0, 0.0),
        Vec2::new(14.0, 6.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(6.0, 14.0),
        Vec2::new(0.0, 14.0),
    ]);
    assert!(!matches!(
        roofing(&ell, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE, 0),
        Roofing::Flat
    ));
    // а плоская кровля так и остаётся у того, кому она положена
    let mut block = building(oblong(30.0, 80.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    assert!(matches!(
        roofing(&block, Vec2::ZERO, |_| Vec2::ZERO, Srgba::WHITE, 0),
        Roofing::Flat
    ));
}

/// Вальма с посевом, который её гарантирует.
fn hip_only(building: &PolyArea, base: Srgba) -> Roofing {
    roofing(building, Vec2::ZERO, |_| Vec2::ZERO, base, 1 << 5)
}

#[test]
fn the_shadow_length_follows_the_sun_elevation() {
    let _sun = crate::map::default_sun();
    // 1 / tan 59° — то самое «0.6 метра тени на метр высоты», которое раньше
    // стояло константой без вывода
    let scale = crate::map::shadow_length_scale();
    assert!((scale - 0.6).abs() < 0.01, "{scale}");
}

/// Сумма площадей **непрозрачных** треугольников меша — тела тени, без
/// мягкого края: у каймы внутренние вершины сходят в нулевую альфу, и
/// треугольник каймы всегда несёт хотя бы одну такую. Двойное наложение
/// внутри тела дало бы сумму больше площади самой фигуры.
fn shadow_area(mesh: &Mesh) -> f32 {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .to_vec();
    let bevy::mesh::VertexAttributeValues::Float32x4(colors) =
        mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
    else {
        panic!("цвет вершины — Float32x4");
    };
    let indices: Vec<usize> = match mesh.indices().unwrap() {
        bevy::mesh::Indices::U32(list) => list.iter().map(|&i| i as usize).collect(),
        bevy::mesh::Indices::U16(list) => list.iter().map(|&i| i as usize).collect(),
    };
    indices
        .chunks_exact(3)
        .filter(|triangle| triangle.iter().all(|&i| colors[i][3] > 0.0))
        .map(|triangle| {
            let point = |i: usize| Vec2::new(positions[triangle[i]][0], positions[triangle[i]][1]);
            (point(1) - point(0)).perp_dot(point(2) - point(0)).abs() / 2.0
        })
        .sum()
}

/// Вершины меша плоскими точками — тело и кайма вместе.
fn mesh_points(mesh: &Mesh) -> Vec<Vec2> {
    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .iter()
        .map(|point| Vec2::new(point[0], point[1]))
        .collect()
}

/// Свип цепочки силуэта — то, из чего union собирает тело тени.
fn sweep_of(chain: &[Vec2], height: f32) -> Vec<Vec2> {
    let offset = shadow_dir() * height * crate::map::shadow_length_scale();
    let mut sweep = chain.to_vec();
    sweep.extend(chain.iter().rev().map(|point| *point + offset));
    sweep
}

#[test]
fn square_shadow_is_one_swept_polygon() {
    let _sun = crate::map::default_sun();
    // одна цепочка низ+право даёт свип из шести вершин — по вершине на угол
    // цепочки и столько же на сдвинутую копию, без квадов на ребро
    let chains = silhouette_chains(&square(), shadow_dir());
    assert_eq!(chains.len(), 1);
    assert_eq!(chains[0].len() * 2, 6);

    // и в самом слое тело тени — ровно этот свип: у одного дома объединять
    // нечего, а мягкий край альфу тела не трогает
    let list = [building(square(), Some(15.0), AreaKind::Building)];
    let mesh = shadow_builder(&list, &[], false).build();
    let expected = signed_ring_area(&sweep_of(&chains[0], 15.0)).abs();
    assert!((shadow_area(&mesh) - expected).abs() < 0.5, "{expected}");
}

#[test]
fn the_penumbra_stays_off_the_lit_side_and_softens_the_far_edge() {
    // тень примыкает к дому жёстко: метровая кайма по контуру примыкания
    // обводила дом мягким пятном с солнечной стороны — тем самым контактным
    // затенением, которое из объединения убрали
    let list = [building(square(), Some(15.0), AreaKind::Building)];
    let mesh = shadow_builder(&list, &[], false).build();
    let along = |points: &[Vec2]| {
        points
            .iter()
            .map(|point| point.dot(shadow_dir()))
            .fold((f32::MAX, f32::MIN), |(low, high), value| {
                (low.min(value), high.max(value))
            })
    };
    let (footprint_near, footprint_far) = along(&square());
    let (mesh_near, mesh_far) = along(&mesh_points(&mesh));
    assert!(
        mesh_near > footprint_near - 0.01,
        "кайма не заходит против света за контур дома: {mesh_near} против {footprint_near}"
    );

    // а дальний край, наоборот, размыт на всю ширину — с запасом на miter:
    // на прямом углу свипа офсет вершины длиннее ширины каймы в корень из двух
    let offset = 15.0 * crate::map::shadow_length_scale();
    let soft = mesh_far - (footprint_far + offset);
    assert!(
        (PENUMBRA_WIDTH..=PENUMBRA_WIDTH * 1.5).contains(&soft),
        "дальний край размыт на {soft} м вместо {PENUMBRA_WIDTH}"
    );
}

#[test]
fn staircase_shadow_has_no_double_darkening() {
    let _sun = crate::map::default_sun();
    // ступенчатый юго-восточный фасад: раньше квады ступеней перекрывались
    // вдоль тени и полупрозрачность складывалась в полосы. Свип монотонной
    // цепочки покрывает ровно |сдвиг| × перп-протяжённость — без нахлёстов
    let staircase = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(6.0, 0.0),
        Vec2::new(6.0, 3.0),
        Vec2::new(9.0, 3.0),
        Vec2::new(9.0, 6.0),
        Vec2::new(12.0, 6.0),
        Vec2::new(12.0, 9.0),
        Vec2::new(0.0, 9.0),
    ];
    let chains = silhouette_chains(&staircase, shadow_dir());
    assert_eq!(chains.len(), 1, "лестница — одна непрерывная цепочка");
    assert_eq!(chains[0].len(), 7);

    // свип цепочки самопересечься не может, поэтому его площадь — ровно
    // «длина сдвига × размах контура поперёк тени»
    let offset_length = 20.0 * crate::map::shadow_length_scale();
    let perp_span = Vec2::new(12.0, 9.0).dot(shadow_dir().perp());
    let sweep = sweep_of(&chains[0], 20.0);
    assert!((signed_ring_area(&sweep).abs() - offset_length * perp_span).abs() < 0.5);

    // и ровно столько же в слое: union не съел свип и не удвоил его
    let list = [building(staircase, Some(20.0), AreaKind::Building)];
    let mesh = shadow_builder(&list, &[], false).build();
    assert!((shadow_area(&mesh) - offset_length * perp_span).abs() < 0.5);
}

#[test]
fn neighbour_shadows_union_without_double_darkening() {
    let _sun = crate::map::default_sun();
    // два корпуса в ряд: тень левого дотягивается до правого, и без
    // union суммарная площадь меша была бы суммой двух свипов — с
    // перекрытием, читающимся как пятно двойной темноты
    let left = building(square(), Some(15.0), AreaKind::Building);
    let right = building(
        square().iter().map(|p| *p + Vec2::new(12.0, 0.0)).collect(),
        Some(15.0),
        AreaKind::Building,
    );
    let alone =
        |b: &PolyArea| shadow_area(&shadow_builder(std::slice::from_ref(b), &[], false).build());
    let separate = alone(&left) + alone(&right);
    let together = shadow_area(&shadow_builder(&[left, right], &[], false).build());
    assert!(
        together < separate - 1.0,
        "union must remove the overlap: {together} vs {separate}"
    );
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

fn passage(points: Vec<Vec2>, passage: bool) -> RoadLine {
    if passage {
        fixture::passage(points, 5.0)
    } else {
        fixture::street(points, 5.0)
    }
}

/// Проём режется только под дорогой с флагом `passage`, и только если
/// она действительно идёт сквозь дом.
#[test]
fn only_a_building_passage_cuts_an_arch() {
    let _sun = crate::map::default_sun();
    let house = vec![building(square(), Some(15.0), AreaKind::Building)];
    let through = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        true,
    )];
    let alongside = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        false,
    )];
    let elsewhere = vec![passage(
        vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 10.0)],
        true,
    )];

    let solid = extrusion_builder(&house, &[], detail(false)).vertex_count();
    assert!(extrusion_builder(&house, &through, detail(false)).vertex_count() > solid);
    assert_eq!(
        extrusion_builder(&house, &alongside, detail(false)).vertex_count(),
        solid
    );
    assert_eq!(
        extrusion_builder(&house, &elsewhere, detail(false)).vertex_count(),
        solid
    );
}

/// Высота проёма задана в настоящих метрах, а рисуется проекция:
/// трёхметровая арка обязана занять ту же долю нарисованной стены, какую
/// три метра занимают в настоящей высоте дома.
#[test]
fn an_arch_opening_is_three_real_metres_of_the_drawn_wall() {
    let _sun = crate::map::default_sun();
    // 40 м высоты, подъём 14 м: арка обязана занять 14 × 3/40 = 1.05 м
    let tall = building(square(), Some(40.0), AreaKind::Building);
    let lift = extrusion_lift(&tall, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &tall, &[&road], lift);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        let low = values.iter().copied().fold(f32::INFINITY, f32::min);
        let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        high - low
    };

    let expected = lift.y * ARCH_HEIGHT / 40.0;
    assert!(
        (span(|p| p[1]) - expected).abs() < 0.01,
        "opening is {} m tall, expected {expected} m of a {} m wall",
        span(|p| p[1]),
        lift.y
    );
}

/// У низкого дома нарисованный метр стоит других настоящих метров:
/// подъём обрезан `EXTRUDE_RANGE`, и пересчёт через `EXTRUDE_SCALE` дал бы
/// не ту долю. Проверяем, что доля считается от высоты самого дома.
#[test]
fn a_clamped_wall_still_gets_a_proportional_opening() {
    let _sun = crate::map::default_sun();
    // 4 м высоты: подъём 4 × 0.35 = 1.4 обрезается снизу до 2.5 м
    let low = building(square(), Some(4.0), AreaKind::Building);
    let lift = extrusion_lift(&low, BuildingHeightMode::Extrusion);
    assert_eq!(lift.y, *EXTRUDE_RANGE.start());

    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);
    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &low, &[&road], lift);

    let heights: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .collect();
    let opening = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - heights.iter().copied().fold(f32::INFINITY, f32::min);
    // арка выше самого дома (6 > 4) — проём режется по стене целиком
    assert!(
        (opening - lift.y).abs() < 0.01,
        "opening {opening} m, expected the whole {} m wall",
        lift.y
    );
}

/// Проём лежит в плоскости стены и шириной с проход, а не растянут вдоль
/// дороги: у дороги, подходящей к южной грани под углом, вырез всё равно
/// ровно по грани.
#[test]
fn an_arch_is_cut_along_the_wall_not_along_the_road() {
    let _sun = crate::map::default_sun();
    let house = building(square(), Some(15.0), AreaKind::Building);
    // `push_arches` — фасадный режим: полоса сдвинута строго вниз. Косой
    // подъём 2.5D сюда не годится — его x-составляющая растянула бы проём
    let band = Vec2::new(0.0, -3.0);
    // дорога идёт наискось и коротка: до стены дотягивается один конец
    let slanted = passage(vec![Vec2::new(4.0, -2.0), Vec2::new(9.0, 20.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&slanted], band);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - values.iter().copied().fold(f32::INFINITY, f32::min)
    };
    // ширина дороги, спроецированная углом входа: наклонная дорога
    // дырявит стену уже собственной ширины
    let entry = (Vec2::new(9.0, 20.0) - Vec2::new(4.0, -2.0)).normalize();
    let expected = slanted.width * entry.perp_dot(Vec2::X).abs();
    assert!(
        (span(|p| p[0]) - expected).abs() < 0.01,
        "opening is {} m wide, expected {expected} m",
        span(|p| p[0])
    );
}

/// Регресс: конец прохода — общая вершина двух граней (как у любой
/// OSM-арки). Зажатый в одну грань проём выходил вдвое уже дороги;
/// теперь куски на обеих гранях продолжают друг друга.
#[test]
fn an_arch_at_a_shared_vertex_keeps_the_road_width() {
    let _sun = crate::map::default_sun();
    // южная сторона из двух граней со стыком в (5, 0)
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ],
        Some(15.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], lift);

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let width = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - xs.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        width >= road.width * 0.9,
        "opening is {width} m wide against a {} m road",
        road.width
    );
}

/// Арка у самого угла дома: проём подрезается по концу грани, а не
/// повисает половиной квада в воздухе за углом.
#[test]
fn an_arch_near_a_corner_is_trimmed_to_the_wall() {
    let _sun = crate::map::default_sun();
    let house = building(square(), Some(15.0), AreaKind::Building);
    // фасадная полоса, как в `an_arch_is_cut_along_the_wall_not_along_the_road`
    let band = Vec2::new(0.0, -3.0);
    // дорога упирается в южную грань в метре от юго-западного угла
    let road = passage(vec![Vec2::new(1.0, 0.0), Vec2::new(1.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], band);
    assert!(!builder.is_empty());

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let west = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let east = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(west >= -0.01, "opening hangs past the corner: {west}");
    // а восточный край — там, куда дотянулась полуширина
    assert!((east - 3.5).abs() < 0.01, "{east}");
}

/// Регресс на реальную арку 485488257 (Тула): проход размечен отрезком
/// **между двумя вершинами контура**, лежит внутри дома и стен касается
/// только концами. Поиск пересечения дороги с контуром здесь не находит
/// ничего — вырез обязан появиться от концов.
#[test]
fn an_arch_lying_inside_the_outline_still_cuts_an_opening() {
    let _sun = crate::map::default_sun();
    // упрощённая геометрия того дома: южная грань y = 0, арка — отрезок
    // от вершины (5, 0) вглубь до вершины (5.2, 14) северной грани
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 14.0),
            Vec2::new(5.2, 14.0),
            Vec2::new(0.0, 14.0),
        ],
        Some(42.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let inner = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.2, 14.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&inner], lift);
    assert!(
        !builder.is_empty(),
        "an outline-to-outline passage cut nothing"
    );
    // и проём сидит на южной грани — начинается на y = 0
    let bottom = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .fold(f32::INFINITY, f32::min);
    assert!(bottom.abs() < 0.01, "opening floats at y = {bottom}");
}

/// Вырез арки шейдеру не виден: окна и балконы он ставит по центру клетки
/// стены, ничего не зная о дыре, и край проёма резал бы их пополам — и на
/// простенке сбоку от арки, и на перемычке над ней. Поэтому клетки, которые
/// проём задел, стена отдаёт ему целиком — заплатой «без проёмов», ровно как
/// под дверью.
///
/// Проверяется это тем же, чем держится вся рама стены: у **всякого** куска
/// стены, на котором проёмы рисуются, границы обязаны лежать на швах клеток —
/// целое число панелей вдоль основания и целое число этажей вверх (или самый
/// верх стены, где над последним этажом лежит запас под карниз).
#[test]
fn the_wall_around_an_arch_wears_no_half_windows() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(20.0, 40.0), Some(30.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    // проезд сквозь дом упирается в южную грань на x = 12, между швами
    let road = passage(vec![Vec2::new(12.0, -2.0), Vec2::new(12.0, 25.0)], true);
    let lift = extrusion_lift(&block, BuildingHeightMode::Extrusion);

    let builder = extrusion_builder(std::slice::from_ref(&block), &[road], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");
    // клетки этой стены: 40 м на целое число панелей, подъём на этажи с
    // запасом под карниз
    let panel = 40.0 / wall_columns(40.0);
    // этажей столько же, сколько насчитал бы `storeys_of`, — целое число
    let storeys = (30.0 / crate::settings::STOREY_HEIGHT).round().max(1.0);
    let storey = lift.y / (storeys + crate::map::meshing::PARAPET_CELLS);

    let mut patched = 0;
    for (point, frame) in builder.positions_for_test().iter().zip(frames) {
        if !is_wall(frame[2]) {
            continue;
        }
        // южная стена: основание на y = 0, верх уехал по подъёму, поэтому
        // место вдоль стены считается с поправкой на косину
        let (x, y) = (point[0], point[1]);
        let along = x - y / lift.y * lift.x;
        if !(-0.01..=lift.y + 0.01).contains(&y) || !(-0.01..=40.01).contains(&along) {
            continue;
        }
        if is_solid(frame[3]) {
            patched += 1;
            continue;
        }
        assert!(
            whole_cells(along / panel),
            "окно разрезано вдоль: стена с проёмами кончается на {along} м, \
             панель — {panel} м"
        );
        assert!(
            whole_cells(y / storey) || (y - lift.y).abs() < 0.01,
            "окно разрезано поперёк: стена с проёмами кончается на {y} м, \
             этаж — {storey} м"
        );
    }
    assert!(patched > 0, "вокруг арки должна лечь заплата без проёмов");
}

/// Два проезда, выходящие в одну грань в пределах одних панелей, — это два
/// выреза, а не один. Заплата кроится по целым панелям, и соседний проём,
/// попавший в тот же блок, отбрасывался целиком: простенок первой арки заливал
/// его сплошной стеной, хотя навмеш прорезан обоими проездами.
#[test]
fn two_arches_in_one_panel_block_each_keep_their_opening() {
    let _sun = crate::map::default_sun();
    let (a, b) = (Vec2::ZERO, Vec2::new(40.0, 0.0));
    let lift = Vec2::new(0.0, 12.0);
    let cells = WallCells {
        frame: None,
        patch: None,
        panel: 3.2,
        storey: Vec2::new(0.0, 3.0),
    };
    let sill = Vec2::new(0.0, 2.0);
    // второй проезд целиком внутри блока панелей первого: 6.2 < ceil(4.0 / 3.2) * 3.2
    let openings = vec![
        ArchOpening {
            a,
            b,
            low: 0.5,
            high: 4.0,
            sill,
        },
        ArchOpening {
            a,
            b,
            low: 5.0,
            high: 6.2,
            sill,
        },
    ];
    let span = WallSpan::new(a, b, lift, 4.0, None);

    let mut builder = MeshBuilder::default();
    push_wall_with_openings(
        &mut builder,
        &span,
        &cells,
        &openings,
        LinearRgba::WHITE,
        LinearRgba::WHITE,
    );

    // ни один кусок стены не лежит поперёк проёма ниже его перемычки
    for quad in builder.positions_for_test().chunks(4) {
        let (from, to) = quad
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), point| {
                (low.min(point[0]), high.max(point[0]))
            });
        let base = quad.iter().fold(f32::MAX, |low, point| low.min(point[1]));
        if base >= sill.y - 0.01 {
            continue;
        }
        for opening in &openings {
            assert!(
                from >= opening.high - 0.01 || to <= opening.low + 0.01,
                "стена {from}..{to} лежит поперёк проёма {}..{}",
                opening.low,
                opening.high
            );
        }
    }
}

/// Середина прохода берётся по длине, а не по числу точек: у ломаной с
/// одним длинным и одним коротким сегментом это разные точки.
#[test]
fn the_passage_middle_is_measured_along_its_length() {
    let _sun = crate::map::default_sun();
    let road = passage(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(90.0, 0.0),
            Vec2::new(100.0, 0.0),
        ],
        true,
    );
    let middle = passage_middle(&road).unwrap();
    assert!((middle.x - 50.0).abs() < 0.01, "{middle:?}");
}
