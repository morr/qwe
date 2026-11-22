//! Стены: тон по свету, фриз вывески, рамы и целость ячеек, фронтоны, двери
//! по входам, балконы и обшивка глубокого корпуса.

use super::super::classify::{BuildingKind, classify};
use super::super::fixtures::{building, detail, extruded_mesh, is_wall, oblong, rect};
use super::super::layers::{door_size, wall_colors};
use super::super::material::*;
use super::super::roofs::*;
use super::super::*;
use super::support::{BRAND, big_box, door_vertices, wall_marks};
use crate::map::meshing::{MeshBuilder, unpack_material};
use crate::map::osm::{AreaKind, BuildingUse};

/// Сколько этажей записано в слоте.
fn slot_storeys(slot: f32) -> f32 {
    unpack_material(slot).1
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

/// Вершины фриза в этом меше — точка и её цвет.
fn brand_band(builder: &MeshBuilder) -> Vec<(Vec2, LinearRgba)> {
    builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
        .filter(|(_, color)| color[0] - color[1] > 0.2)
        .map(|(point, color)| {
            (
                Vec2::new(point[0], point[1]),
                LinearRgba::rgb(color[0], color[1], color[2]),
            )
        })
        .collect()
}

/// Фриз — та же поверхность, что стена под ним, и свет на него падает так же.
/// Закрутку контура OSM не нормализует, и правило «нормаль видимой стены»
/// поэтому держится одним хелпером ([`wall_normal`]): написанное дважды, оно
/// разошлось, и на обратной закрутке освещённая грань несла тёмную полосу.
#[test]
fn the_brand_band_is_lit_like_the_wall_under_it_at_any_winding() {
    let _sun = crate::map::default_sun();
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    let base = luminance(crate::map::osm::model::srgba_of(BRAND).into());
    let ccw = rect(Vec2::ZERO, Vec2::new(60.0, 40.0));
    // обратная закрутка с той же первой вершиной: посев дома — от неё, и
    // сменись он, коробка разыграла бы себе другую облицовку
    let cw: Vec<Vec2> = std::iter::once(ccw[0])
        .chain(ccw.iter().skip(1).rev().copied())
        .collect();
    for (winding, ring) in [("CCW", ccw.clone()), ("CW", cw)] {
        let builder = extruded_mesh(&[big_box(ring, 9.5)], &[], detail(false));
        let band = brand_band(&builder);
        assert!(!band.is_empty(), "{winding}: фриз ложится на видимые стены");
        // свет из верхнего левого угла: западная стена освещена, южная в тени
        let west = band.iter().find(|(at, _)| at.y > 30.0).expect("западная");
        let south = band.iter().find(|(at, _)| at.x > 40.0).expect("южная");
        assert!(
            luminance(west.1) > base,
            "{winding}: полоса освещённой грани светлее своего цвета"
        );
        assert!(
            luminance(south.1) < base,
            "{winding}: полоса затенённой грани темнее своего цвета"
        );
    }
}

/// Фриз — непрозрачный квад поверх стены, и лечь он обязан **над** лентой
/// витражного стекла, а лента стоит внутри своей ячейки
/// (`roof.wesl::BIG_BOX_WINDOW_HIGH`, 0.70 яруса). Мерился он долей всей стены,
/// и у коробки выше одного яруса лента верхнего яруса уходила под него целиком.
#[test]
fn the_brand_band_stays_above_the_glass_ribbon_at_any_number_of_tiers() {
    let _sun = crate::map::default_sun();
    // верх ленты внутри ячейки — `roof.wesl::BIG_BOX_WINDOW_HIGH`
    let ribbon_top = 0.70;
    // 5.5 м на ярус (`BIG_BOX_TIER`): 8 м — один, 9.5 — два, 17 — три
    for (height, storeys) in [(8.0, 1.0), (9.5, 2.0), (17.0, 3.0)] {
        let retail = big_box(rect(Vec2::ZERO, Vec2::new(60.0, 40.0)), height);
        let lift = lift_of(&retail);
        let builder = extruded_mesh(&[retail], &[], detail(false));
        // в ячейках стены: её верх — `storeys + PARAPET_CELLS`
        let cells = |at: Vec2| at.y / lift.y * (storeys + crate::map::meshing::PARAPET_CELLS);
        let band = brand_band(&builder);
        let low = band
            .iter()
            .filter(|(at, _)| at.x > 40.0)
            .fold(f32::MAX, |low, (at, _)| low.min(cells(*at)));
        assert!(
            low > storeys - 1.0 + ribbon_top,
            "{height} м, {storeys} яруса: низ фриза {low} лёг на ленту верхнего яруса"
        );
    }
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

/// Оборудование кровли помечено отрицательным слотом материала — ровно его
/// вершины и только они: по этой метке вершинный шейдер прячет его на дальней
/// ступени зума (`material::show_roof_clutter`), не пересобирая слой, а
/// фрагменту минус — тот же ноль, фактуры у оборудования нет.
#[test]
fn roof_clutter_vertices_carry_the_clutter_mark() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(14.0, 40.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    let bare = extruded_mesh(std::slice::from_ref(&block), &[], detail(false));
    let with_clutter = RoofDetail {
        clutter: true,
        ..detail(false)
    };
    let full = extruded_mesh(&[block], &[], with_clutter);
    let frames = full.roof_coords_for_test().expect("roof coords");
    let marked = frames.iter().filter(|frame| frame[2] < 0.0).count();
    assert!(marked > 0, "у жилого дома нет оборудования");
    assert_eq!(marked, full.vertex_count() - bare.vertex_count());
    assert_eq!(unpack_material(-1.0).0, 0, "фрагменту метка — ноль");
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

/// Корпус без назначения глубже жилого (R36: Орёл, Ломоносова 6/5, 127 × 144 м,
/// `building=yes`) — не панелька: ни панели, ни кирпича, ни одного балкона, на
/// каком бы посеве он ни стоял. Жилая пластина 15 × 80 и каре из 15-метровых
/// крыльев с тем же `building=yes` остаются в жилой таблице, как были.
#[test]
fn a_hull_deeper_than_housing_carries_no_balconies() {
    let _sun = crate::map::default_sun();
    let at = |outer: Vec<Vec2>, offset: Vec2| -> Vec<Vec2> {
        outer.into_iter().map(|point| point + offset).collect()
    };
    let mut slab_kinds = Vec::new();
    let mut court_kinds = Vec::new();
    for step in 0..40 {
        let offset = Vec2::new(step as f32 * 211.0, step as f32 * 37.0);

        let hull = building(
            at(oblong(130.0, 145.0), offset),
            Some(27.0),
            AreaKind::Building,
        );
        // корпус глубже жилого — класс, а не свой перебор глубин у стен
        assert!(matches!(
            classify(&hull, None).kind(),
            BuildingKind::DeepHull(_)
        ));
        let kind = wall_of(&hull).kind;
        assert!(
            !matches!(kind, WallKind::Panel | WallKind::Brick),
            "корпус 130 × 145 одет в {kind:?}"
        );
        assert!(
            wall_marks(&hull, kind).iter().all(|seed| *seed < 0.0),
            "балконы на корпусе 130 × 145 ({kind:?})"
        );

        let slab = building(
            at(oblong(15.0, 80.0), offset),
            Some(27.0),
            AreaKind::Building,
        );
        assert!(!matches!(
            classify(&slab, None).kind(),
            BuildingKind::DeepHull(_)
        ));
        slab_kinds.push(wall_of(&slab).kind);

        // каре 60 × 60 с двором 30 × 30: габарит 60, крыло 15
        let mut court = building(
            at(oblong(60.0, 60.0), offset),
            Some(27.0),
            AreaKind::Building,
        );
        court.holes = vec![at(rect(Vec2::splat(15.0), Vec2::splat(45.0)), offset)];
        assert!(!matches!(
            classify(&court, None).kind(),
            BuildingKind::DeepHull(_)
        ));
        court_kinds.push(wall_of(&court).kind);
    }
    // жилая таблица как была: панель у пластины и у каре встречается
    assert!(slab_kinds.contains(&WallKind::Panel), "{slab_kinds:?}");
    assert!(court_kinds.contains(&WallKind::Panel), "{court_kinds:?}");
}
