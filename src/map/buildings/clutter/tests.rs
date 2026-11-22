//! Раскладка оборудования на кровле: попадание в контур, занятость, решётка
//! фонарей и её шаг, приточная группа целиком, устойчивость посева и тени на
//! низком солнце.

use super::*;
use crate::map::osm::model::is_big_box;
use crate::map::osm::{AreaKind, BuildingUse};
use crate::map::sun::SUN_ELEVATION_MIN;

fn building(outer: Vec<Vec2>, building_use: BuildingUse) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind: AreaKind::Building,
        building_use,
        height: Some(15.0),
        storeys: None,
        entrances: Vec::new(),
        colours: Default::default(),
        osm_id: None,
    }
}

fn block(width: f32, length: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(length, 0.0),
        Vec2::new(length, width),
        Vec2::new(0.0, width),
    ]
}

/// Луч тени останавливается на краю кровли: азимут 270° кладёт тень строго
/// на восток, и от коробки в `x = 4..6` до стены `x = 20` остаётся 14 м —
/// по ближайшему из углов, а не по дальнему.
#[test]
fn the_shadow_reach_stops_at_the_roof_edge() {
    let _sun = crate::map::sun_at(270.0, 45.0);
    assert!(
        (shadow_dir() - Vec2::X).length() < 1e-5,
        "{:?}",
        shadow_dir()
    );
    let slab = building(block(20.0, 20.0), BuildingUse::Other);
    let base = rect(Vec2::new(5.0, 10.0), Vec2::splat(2.0), Vec2::X, Vec2::Y);
    let reach = shadow_reach(&slab, Vec2::ZERO, &base);
    assert!((reach - 14.0).abs() < 1e-3, "{reach}");
}

/// На низком солнце тень оборудования обязана остаться на крыше: рисуется
/// она непрозрачной и цветом этой кровли, так что съехавшая полоса легла бы
/// тёмной чертой на соседний дом и на землю.
#[test]
fn a_low_sun_keeps_the_equipment_shadow_on_the_roof() {
    let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
    let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
    let look = super::super::material::roof_look(&slab, None);
    let mut items = flat_roof_items(&slab, &look, Vec2::ZERO);
    assert!(!items.is_empty());
    // и отдельно — машинное помещение у самого края с наветренной стороны:
    // разложенное оборудование до края может и не достать, а зажим нужен
    // именно ему
    let base = rect(Vec2::new(50.0, 8.0), PENTHOUSE_SIZE, Vec2::X, Vec2::Y);
    items.push(RoofItem {
        base,
        height: PENTHOUSE_HEIGHT,
        top: PENTHOUSE_TOP,
        wall: PENTHOUSE_WALL,
        reach: shadow_reach(&slab, Vec2::ZERO, &base),
    });
    let mut clamped = 0;
    for item in &items {
        let wanted = shadow::length(item.height);
        // тот самый вылет, который увидит `push_items`, — из предмета, а
        // не пересчитанный тестом заново
        let reach = item.reach;
        if reach < wanted {
            clamped += 1;
        }
        let offset = shadow_dir() * wanted.min(reach);
        for corner in item.base {
            // зажатая тень кончается ровно на стене, и точка на самом
            // контуре в `point_in_area` уже наружу — отступаем на промилле
            let far = corner + offset * 0.999;
            assert!(point_in_area(far, &slab), "{far:?} is off the roof");
        }
    }
    // без зажима хоть одна тень с этой крыши уходит: машинное помещение в
    // три метра даёт на 15° одиннадцать метров при шестнадцатиметровой
    // ширине корпуса
    assert!(clamped > 0, "nothing was clamped, the case is not covered");
}

/// Гипермаркет от магазина у дома отличается кровлей, и это разница по
/// **размеру**, а не по тегу: решётка зенитных фонарей и группа приточных
/// установок появляются только на крупноформатном пятне, а один и тот же
/// `shop=supermarket` на 300 м² остаётся обычной мелкой коробкой.
#[test]
fn a_hypermarket_roof_is_a_grid_of_skylights_and_a_corner_shop_is_not() {
    let _sun = crate::map::default_sun();
    let hyper = building(block(90.0, 110.0), BuildingUse::Retail);
    assert!(is_big_box(&hyper));
    let look = super::super::material::roof_look(&hyper, None);
    let items = flat_roof_items(&hyper, &look, Vec2::ZERO);
    let skylights = items.iter().filter(|item| item.top == SKYLIGHT_TOP).count();
    // 110 × 90 м с отступом от края — не меньше шести шагов по 13 м вдоль
    // и не меньше пяти поперёк
    assert!(skylights >= 30, "{skylights} skylights on a hypermarket");
    assert!(skylights <= GRID_SKYLIGHT_MAX);
    // группа приточных установок стоит одним рядом
    let plant = items
        .iter()
        .filter(|item| (item.height - PLANT_HEIGHT).abs() < 1e-3)
        .count();
    assert!(plant >= 2, "{plant} plant units");

    // тот же класс на мелком пятне: ни одного фонаря и ни одной установки
    let shop = building(block(14.0, 20.0), BuildingUse::Retail);
    assert!(!is_big_box(&shop));
    let look = super::super::material::roof_look(&shop, None);
    let items = flat_roof_items(&shop, &look, Vec2::ZERO);
    assert!(items.iter().all(|item| item.top != SKYLIGHT_TOP));
    assert!(
        items
            .iter()
            .all(|item| (item.height - PLANT_HEIGHT).abs() > 1e-3)
    );
}

/// Группа приточных установок кладётся **целым рядом**, и попытка на это
/// тратится: бросок, чей габарит влез в контур, но чьи агрегаты упёрлись в
/// уже стоящую решётку фонарей, — это промах, а не место ([`place`] тратит
/// попытки ровно так же). Пока такой бросок считался местом, на узкой
/// коробке 26 × 70 м ряд выходил пустым на двух раскладках из ста, а на
/// 90 × 110 м из трёх агрегатов оставалось меньше трёх почти на половине.
#[test]
fn the_plant_group_is_laid_whole_or_the_try_is_spent() {
    let _sun = crate::map::default_sun();
    // ширина, длина и сколько агрегатов просит `PLANT_AREA_PER` на этом
    // пятне: 1820 м² — нижний зажим в два, 9900 м² — три
    for (width, length, count) in [(26.0, 70.0, 2), (90.0, 110.0, 3)] {
        let mut whole = 0;
        // посев берётся от первой вершины контура, так что одна кровля —
        // это одна раскладка из многих: дом переставляется по карте, и
        // правило проверяется на сотне посевов
        for step in 0..100 {
            let at = Vec2::new(step as f32 * 13.0, step as f32 * 7.0);
            let outer = block(width, length)
                .into_iter()
                .map(|corner| corner + at)
                .collect();
            let house = building(outer, BuildingUse::Retail);
            let look = super::super::material::roof_look(&house, None);
            let items = flat_roof_items(&house, &look, Vec2::ZERO);
            let plant = items
                .iter()
                .filter(|item| (item.height - PLANT_HEIGHT).abs() < 1e-3)
                .count();
            assert!(plant > 0, "{width}x{length} at {at:?}: no plant at all");
            whole += usize::from(plant == count);
        }
        // целый ряд — правило, а не удача: место под него находится почти
        // на каждом посеве, восьми попыток на это хватает
        assert!(whole >= 90, "{width}x{length}: {whole} whole rows of 100");
    }
}

/// Фонари решётки стоят **по линейке**: шаг между соседями в ряду —
/// ровно [`GRID_SKYLIGHT_PITCH`]. Это тот случай, когда сетка и есть
/// вещь, и дрожание шага тут было бы ошибкой, а не живостью.
#[test]
fn the_skylight_grid_keeps_its_pitch() {
    let _sun = crate::map::default_sun();
    let hyper = building(block(90.0, 110.0), BuildingUse::Retail);
    let look = super::super::material::roof_look(&hyper, None);
    let items = flat_roof_items(&hyper, &look, Vec2::ZERO);
    let mut centres: Vec<Vec2> = items
        .iter()
        .filter(|item| item.top == SKYLIGHT_TOP)
        .map(|item| item.base.iter().sum::<Vec2>() / 4.0)
        .collect();
    assert!(centres.len() > 2);
    // контур осепараллелен, так что рама совпадает с мировыми осями:
    // сортируем по строке, потом по столбцу
    centres.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
    let mut checked = 0;
    for pair in centres.windows(2) {
        if (pair[1].y - pair[0].y).abs() > 1e-3 {
            continue;
        }
        assert!(
            (pair[1].x - pair[0].x - GRID_SKYLIGHT_PITCH).abs() < 1e-3,
            "{:?} → {:?}",
            pair[0],
            pair[1]
        );
        checked += 1;
    }
    assert!(checked > 0, "no two skylights shared a row");
}

/// На гиганте решётка **разрежается, а не обрывается**: шаг подгоняется
/// так, чтобы вся она уложилась в предел и накрыла кровлю целиком. Пока
/// шаг был константой, ТРЦ «Макси» (493 × 298 м) забирал предел первыми
/// рядами, и дальняя половина кровли оставалась пустой — сообщено по
/// кадру как «квадратики размещены странно».
#[test]
fn a_giant_roof_thins_the_grid_instead_of_cutting_it_off() {
    let _sun = crate::map::default_sun();
    let maxi = building(block(298.0, 493.0), BuildingUse::Retail);
    let look = super::super::material::roof_look(&maxi, None);
    let items = flat_roof_items(&maxi, &look, Vec2::ZERO);
    let centres: Vec<Vec2> = items
        .iter()
        .filter(|item| item.top == SKYLIGHT_TOP)
        .map(|item| item.base.iter().sum::<Vec2>() / 4.0)
        .collect();
    assert!(centres.len() > GRID_SKYLIGHT_MAX / 2, "{}", centres.len());
    assert!(centres.len() <= GRID_SKYLIGHT_MAX);
    // решётка дотягивается до обоих концов: контур осепараллелен, так что
    // крайние узлы обязаны стоять у обоих краёв рамы, а не в одной трети
    let (lo, hi) = centres.iter().fold((f32::MAX, f32::MIN), |(lo, hi), at| {
        (lo.min(at.x), hi.max(at.x))
    });
    assert!(lo < 60.0 && hi > 433.0, "grid spans {lo}..{hi} of 0..493");
    // и шаг остался единым — она реже, но по-прежнему решётка
    let mut rows: Vec<f32> = centres.iter().map(|at| at.x).collect();
    rows.sort_by(f32::total_cmp);
    rows.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    for pair in rows.windows(2) {
        let step = pair[1] - pair[0];
        assert!(
            step >= GRID_SKYLIGHT_PITCH - 1e-3,
            "{step} m between columns"
        );
    }
}

#[test]
fn a_block_gets_equipment_and_a_shed_does_not() {
    let _sun = crate::map::default_sun();
    let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
    let look = super::super::material::roof_look(&slab, None);
    let items = flat_roof_items(&slab, &look, Vec2::ZERO);
    assert!(!items.is_empty(), "a 960 m2 block carries equipment");

    // сарай 3 × 3: после отступа от края ставить некуда
    let shed = building(block(3.0, 3.0), BuildingUse::Other);
    let look = super::super::material::roof_look(&shed, None);
    assert!(flat_roof_items(&shed, &look, Vec2::ZERO).len() <= 1);
}

#[test]
fn equipment_stays_inside_the_footprint() {
    let _sun = crate::map::default_sun();
    // Г-образный контур: рама прямоугольна, дом — нет
    let ell = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(60.0, 0.0),
            Vec2::new(60.0, 14.0),
            Vec2::new(20.0, 14.0),
            Vec2::new(20.0, 50.0),
            Vec2::new(0.0, 50.0),
        ],
        BuildingUse::Apartments,
    );
    let look = super::super::material::roof_look(&ell, None);
    for item in flat_roof_items(&ell, &look, Vec2::ZERO) {
        for corner in item.base {
            assert!(point_in_area(corner, &ell), "{corner:?} is off the roof");
        }
    }
}

/// Две коробки не садятся одна на другую. Места берутся из ГПСЧ, и пока
/// проверялось только попадание в контур, блок кондиционера вставал на
/// вентшахту — на детском саду 9 × 17 м (Тула, way 234273437), где кровля
/// несёт ровно эти две коробки и промахнуться мимо соседа почти не в чем.
#[test]
fn equipment_never_sits_on_equipment() {
    let _sun = crate::map::default_sun();
    // посев берётся от первой вершины контура, так что одна кровля — это
    // одна раскладка из многих: дом переставляется по карте, и правило
    // проверяется на сотне посевов, а не на том единственном, который
    // случайно лёг удачно
    for (width, length, building_use) in [
        (9.0, 17.0, BuildingUse::Public),
        (16.0, 60.0, BuildingUse::Apartments),
        (40.0, 60.0, BuildingUse::Commercial),
        (30.0, 90.0, BuildingUse::Industrial),
        // гипермаркет — единственная кровля, где предмет кладётся **не
        // броском, а по сетке**, и первая её версия ставила фонарь прямо
        // на уже стоящее машинное помещение (сообщено по кадру). Правило
        // общее, и проверяться оно обязано и на этой раскладке тоже
        (90.0, 110.0, BuildingUse::Retail),
        // узкая коробка: сетка почти упирается в отступ от края, а
        // агрегатам приточной группы остаётся одна полоса
        (26.0, 70.0, BuildingUse::Retail),
    ] {
        let mut laid = 0;
        for step in 0..100 {
            let at = Vec2::new(step as f32 * 13.0, step as f32 * 7.0);
            let outer = block(width, length)
                .into_iter()
                .map(|corner| corner + at)
                .collect();
            let house = building(outer, building_use);
            let look = super::super::material::roof_look(&house, None);
            let items = flat_roof_items(&house, &look, Vec2::ZERO);
            laid += items.len();
            for (index, item) in items.iter().enumerate() {
                for other in &items[index + 1..] {
                    assert!(
                        !boxes_overlap(&item.base, &other.base),
                        "{width}x{length} {building_use:?} at {at:?}: \
                         {:?} sits on {:?}",
                        item.base,
                        other.base
                    );
                }
            }
        }
        assert!(laid > 100, "{width}x{length} {building_use:?} is empty");
    }
}

/// Пересекаются ли два выпуклых четырёхугольника — по разделяющей оси, на
/// нормалях рёбер обоих. Тест намеренно не знает про общую раму, в которой
/// раскладывается оборудование, и поймал бы промах и в повёрнутой паре.
fn boxes_overlap(a: &[Vec2; 4], b: &[Vec2; 4]) -> bool {
    let span = |quad: &[Vec2; 4], dir: Vec2| {
        quad.iter().fold((f32::MAX, f32::MIN), |(lo, hi), corner| {
            let at = corner.dot(dir);
            (lo.min(at), hi.max(at))
        })
    };
    for quad in [a, b] {
        for index in 0..quad.len() {
            let edge = quad[(index + 1) % quad.len()] - quad[index];
            let Some(normal) = Vec2::new(-edge.y, edge.x).try_normalize() else {
                continue;
            };
            let (a_lo, a_hi) = span(a, normal);
            let (b_lo, b_hi) = span(b, normal);
            if a_hi <= b_lo || b_hi <= a_lo {
                return false;
            }
        }
    }
    true
}

#[test]
fn the_placement_is_stable() {
    let _sun = crate::map::default_sun();
    let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
    let look = super::super::material::roof_look(&slab, None);
    let first = flat_roof_items(&slab, &look, Vec2::ZERO);
    let second = flat_roof_items(&slab, &look, Vec2::ZERO);
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.base, b.base);
    }
}

/// Тень трубы на коньке обязана остаться на крыше. Наклон конька сдвигает
/// базовую точку вверх; передача сдвига в shadow_reach обязана быть полной
/// (lift + ridge_offset).
#[test]
fn ridge_chimney_shadow_stays_on_roof_with_low_sun() {
    let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
    let house = building(block(8.0, 10.0), BuildingUse::Other);
    let look = super::super::material::roof_look(&house, None);
    // конёк двускатной крыши проходит по средней линии короткой стороны
    let ridge = (Vec2::new(4.0, 0.0), Vec2::new(4.0, 10.0));
    // ridge_offset на типовой крыше — несколько дециметров; тест ищет баг,
    // когда сдвиг конька не передан, поэтому lift=Vec2::ZERO даст краткие
    // тени. С правильным ridge_offset эта область покрывается.
    let lift = Vec2::new(0.5, 0.5);
    if let Some(item) = ridge_chimney(&house, &look, ridge, lift) {
        let offset = shadow::offset(item.height).clamp_length_max(item.reach);
        for corner in item.base {
            let far = corner + offset * 0.999;
            assert!(
                point_in_area(far, &house),
                "{far:?} is off the house boundary"
            );
        }
    }
}
