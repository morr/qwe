//! Материал: палитры стен и кровель по назначению, устойчивость посева,
//! облицовка по назначению и росту, и полнота таблицы облицовок.

use super::super::fixtures::{building, oblong, square};
use super::super::layers::roof_color;
use super::super::material::*;
use super::super::*;
use super::support::ORTHODOX;
use crate::map::osm::{AreaKind, BuildingUse};

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
    let wall = |b: &PolyArea| wall_look(b, None).base;
    assert_ne!(wall(&house), wall(&industrial));
    assert_ne!(wall(&church), wall(&house));
    let kremlin_plain = building(square(), None, AreaKind::Kremlin);
    assert_eq!(wall(&kremlin), wall(&kremlin_plain));

    // а крыша — по материалу: у частного дома черепица или металл, у
    // промзоны профлист или битум, и одинаковыми они не выходят
    let roof = |b: &PolyArea| roof_color(b, &roof_look(b, None), false);
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
    assert_eq!(roof_look(&house, None).kind, roof_look(&house, None).kind);
    // сдвинутый дом — другой посев, а стало быть возможен и другой слот
    let moved: Vec<Vec2> = square()
        .into_iter()
        .map(|p| p + Vec2::new(37.0, 0.0))
        .collect();
    let mut neighbour = building(moved, None, AreaKind::Building);
    neighbour.building_use = BuildingUse::House;
    let _ = roof_look(&neighbour, None);

    // храм — всегда фальцевый металл, гараж — из своей таблицы
    let mut church = building(square(), None, AreaKind::Building);
    church.building_use = ORTHODOX;
    assert_eq!(roof_look(&church, None).kind, RoofKind::Seam);

    // ось фактуры — длинная сторона контура
    let long = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(40.0, 0.0),
        Vec2::new(40.0, 8.0),
        Vec2::new(0.0, 8.0),
    ];
    let axis = roof_look(&building(long, None, AreaKind::Building), None)
        .frame
        .axis;
    assert!(axis.x.abs() > axis.y.abs(), "{axis:?}");
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
        BuildingUse::Retail,
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
/// (`run_wall_look` в `layers::roofing::house_look`), а не назначение дома, так что
/// через `wall_of` они не приходят никогда.
#[test]
fn every_cladding_reaches_the_city() {
    let _sun = crate::map::default_sun();
    let mut seen: Vec<WallKind> = Vec::new();
    for row in 0..12 {
        for column in 0..12 {
            let at = Vec2::new(column as f32 * 140.0, row as f32 * 120.0);
            // назначения по кругу, высоты по кругу — так перебираются обе
            // развилки сразу
            let building_use = [
                BuildingUse::House,
                BuildingUse::Apartments,
                BuildingUse::Commercial,
                BuildingUse::Industrial,
                BuildingUse::Public,
                BuildingUse::Other,
                ORTHODOX,
                BuildingUse::Retail,
            ][column % 8];
            // у торговли развилка не по высоте, а по пятну: кассету с фризом
            // получает только крупноформат ([`is_big_box`]), и на ленте в
            // 560 м² она не встретилась бы ни разу
            let size = match (building_use, row % 2) {
                (BuildingUse::Retail, 0) => (60.0, 60.0),
                _ => (14.0, 40.0),
            };
            let outer: Vec<Vec2> = oblong(size.0, size.1).into_iter().map(|p| p + at).collect();
            let mut area = building(outer, Some([6.0, 15.0, 27.0][row % 3]), AreaKind::Building);
            area.building_use = building_use;
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
