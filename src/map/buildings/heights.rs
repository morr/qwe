//! Сколько этажей у дома, о котором OSM молчит.
//!
//! Высота есть у 31 % контуров Тулы и у 5 % Токио — у остальных её приходится
//! **выводить**, и до сих пор вывод состоял из трёх чисел: частный дом 6 м,
//! гараж 3 м, всё прочее 15 м. На карте это видно сразу: две трети города
//! стоят ровно в пять этажей, отбрасывают тень одной длины и поднимаются в
//! 2.5D на один и тот же сдвиг. Города такими не бывают — а на снимке сверху
//! именно разнобой высот и читается как город.
//!
//! Вывод идёт по **пятну и назначению**, как его читает глаз на аэрофото:
//! длинная узкая коробка — панельная секция, компактная и крупная — башня,
//! огромное пятно с малой высотой — торговый зал или цех. Внутри каждой
//! группы этажность берётся посевом дома (тем же, что выбирает материал
//! кровли, [`super::material::building_seed`]), поэтому она не зависит ни от
//! порядка домов, ни от режима отрисовки и не меняется между запусками.
//!
//! **Тег всегда сильнее вывода.** Всё это включается ровно там, где
//! `PolyArea::height` пуст.

use bevy::math::Vec2;

use super::material::building_seed;
use super::roofs::{SHED_FOOTPRINT_MAX, SMALL_FOOTPRINT_MAX};
use crate::map::meshing::min_area_rect;
use crate::map::osm::model::{is_big_box, is_fortress_tower, signed_ring_area};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea, Sacred, SacredForm};

/// Высота этажа, м — то же число, которым парсер переводит
/// `building:levels` в метры, и по нему же считаются серии ниже.
const STOREY: f32 = 3.0;

/// Гараж — одна коробка в один этаж, и никакого разброса: ряды гаражных
/// боксов на снимке все одной высоты.
const GARAGE_HEIGHT: f32 = 3.0;

/// Частный дом, высота стен до карниза: в основном **один этаж**, и чердак
/// над ним живёт уже в крыше (двускатной или ломаной, `roofs.rs`). Двухэтажный
/// коттедж — один дом из пяти. Прежняя таблица (5–8 м) делила стену на два
/// этажа окон у каждого дома, и частный сектор выходил кварталом двухэтажек.
const HOUSE_HEIGHTS: [f32; 10] = [3.0, 3.0, 3.0, 3.0, 3.2, 3.2, 3.4, 3.4, 6.0, 6.4];
/// Сарай, баня, летняя кухня: одна низкая коробка.
const SHED_HEIGHTS: [f32; 4] = [2.4, 2.6, 2.8, 3.0];

/// Панельная секция: пятиэтажка, девятиэтажка, изредка двенадцать. Доли —
/// по тому, чего в русском городе больше.
const SLAB_STOREYS: [f32; 10] = [5.0, 5.0, 5.0, 5.0, 5.0, 9.0, 9.0, 9.0, 12.0, 12.0];
/// Башня — компактное **крупное** пятно: в основном девять, изредка
/// шестнадцать. Первый вариант таблицы был вдвое выше, и город вышел
/// небоскрёбным: у Тулы p90 поднялась до 27 м, тогда как девятиэтажка там
/// уже редкость.
const TOWER_STOREYS: [f32; 8] = [5.0, 9.0, 9.0, 9.0, 9.0, 12.0, 12.0, 16.0];
/// Всё остальное крупное — кирпичный корпус, школа, контора: два-пять этажей.
/// Это самая населённая ветка вывода, и она обязана быть низкой.
const MID_STOREYS: [f32; 6] = [2.0, 3.0, 3.0, 4.0, 4.0, 5.0];
/// Старый дом в центре: два-четыре этажа.
const LOW_STOREYS: [f32; 6] = [2.0, 2.0, 3.0, 3.0, 3.0, 4.0];
/// Храм: не этажами, а сразу метрами — у него один «этаж» до карниза.
const CHURCH_HEIGHTS: [f32; 4] = [12.0, 14.0, 18.0, 22.0];
/// Колокольня и минарет — до карниза яруса звона; шпиль или глава над ним
/// рисуются уже сверх этой высоты (`temples.rs`).
const BELL_TOWER_HEIGHTS: [f32; 3] = [24.0, 30.0, 36.0];
/// Крепостная стена и башня, м: у Тульского кремля 12.7 и 30 по тегам.
const FORTRESS_WALL_HEIGHTS: [f32; 2] = [10.0, 12.0];
const FORTRESS_TOWER_HEIGHTS: [f32; 2] = [20.0, 26.0];
/// Цех и склад — тоже метрами: один пролёт, но высокий.
const HALL_HEIGHTS: [f32; 4] = [7.0, 8.0, 10.0, 12.0];
/// Торговый зал: один-два этажа под большой крышей.
const STORE_HEIGHTS: [f32; 3] = [6.0, 7.0, 9.0];
/// Гипермаркет и ТЦ — оболочка, а не этажи: один высокий торговый зал, над ним
/// техэтаж и парапет. Восемь метров — ровно то, что `parse::building_height`
/// собирает из `building:levels=1` (4.5 торгового уровня + 3.5 оболочки), и
/// это намеренно одна высота с двух сторон: у половины крупноформатной торговли
/// тега нет вовсе (Тула: «Верный», ТЦ «Перспектива»), и коробка рядом с
/// такой же коробкой не должна оказаться вдвое ниже из-за отсутствия тега.
const BIG_BOX_HEIGHTS: [f32; 4] = [8.0, 8.0, 9.5, 11.0];
/// Магазин у дома: отдельно стоящий павильон в один-два уровня. Выше жилого
/// этажа — у торгового зала потолки, — но это не коробка в поле.
const SHOP_HEIGHTS: [f32; 4] = [4.5, 5.0, 5.0, 6.5];

/// Длина пятна, от которой оно читается как секция, и предельная ширина
/// такой секции, м. Панельный дом — это лента 12–16 м в ширину и от сорока в
/// длину; всё, что шире, уже корпус.
const SLAB_MIN_LENGTH: f32 = 35.0;
const SLAB_MAX_WIDTH: f32 = 18.0;
/// Пятно, ниже которого дом без назначения считается старым малоэтажным, м².
const LOW_FOOTPRINT_MAX: f32 = 300.0;
/// Пятно, выше которого «торговое» здание — это зал, а не контора, м².
const STORE_FOOTPRINT_MIN: f32 = 800.0;
/// Башней считается пятно не меньше этого и не вытянутее этого отношения
/// сторон: у настоящей башни план почти квадратный.
const TOWER_FOOTPRINT_MIN: f32 = 500.0;
const TOWER_MAX_RATIO: f32 = 1.7;

/// Высота дома для отрисовки: тег из OSM, иначе вывод по пятну и назначению.
///
/// Наружу из `buildings` её читает ещё слой машин: плотность ряда у бордюра и
/// на стоянке зависит от этажности квартала вокруг (`cars::district`), и
/// мерить её обязан **тот же** вывод, которым дом нарисован, — иначе
/// нарисованная девятиэтажка стояла бы в кварталах частного сектора.
pub(crate) fn height_or_default(building: &PolyArea) -> f32 {
    building
        .height
        .unwrap_or_else(|| inferred_height(building, building_seed(building)))
}

/// Слот дома в таблице группы: посев решает, какой из вариантов ему достался.
/// Одна выборка на все таблицы — иначе разряды посева расходятся по веткам и
/// «тот же дом — та же высота» приходится доказывать заново в каждой.
fn pick(table: &[f32], seed: u32) -> f32 {
    table[(seed >> 3) as usize % table.len()]
}

/// Выведенная высота — та самая, которой в OSM не нашлось.
fn inferred_height(building: &PolyArea, seed: u32) -> f32 {
    let area = signed_ring_area(&building.outer).abs();

    // крепость меряется не назначением (у башни это `building=yes`), а тем,
    // стена это или башня
    if building.kind == AreaKind::Kremlin {
        return match is_fortress_tower(building) {
            true => pick(&FORTRESS_TOWER_HEIGHTS, seed),
            false => pick(&FORTRESS_WALL_HEIGHTS, seed),
        };
    }
    match building.building_use {
        BuildingUse::Garage | BuildingUse::GarageBlock => GARAGE_HEIGHT,
        BuildingUse::House => pick(&HOUSE_HEIGHTS, seed),
        BuildingUse::Church(Sacred {
            form: SacredForm::Tower,
            ..
        }) => pick(&BELL_TOWER_HEIGHTS, seed),
        BuildingUse::Church(_) => pick(&CHURCH_HEIGHTS, seed),
        BuildingUse::Industrial => pick(&HALL_HEIGHTS, seed),
        // торговля меряется не этажами, а размером: гипермаркет — оболочка
        // над одним высоким залом, магазин у дома — павильон
        BuildingUse::Retail if is_big_box(building) => pick(&BIG_BOX_HEIGHTS, seed),
        BuildingUse::Retail => pick(&SHOP_HEIGHTS, seed),
        BuildingUse::Commercial if area >= STORE_FOOTPRINT_MIN => pick(&STORE_HEIGHTS, seed),
        // казённое здание — школа, поликлиника, контора: та же таблица, что у
        // прочего крупного корпуса, но **в обход проверки формы**: школа в
        // 900 м² с почти квадратным планом иначе вышла бы башней в 9–16
        // этажей. Общее имя, а не копия: две таблицы одного числа разошлись бы
        // на первой же правке «прочего корпуса», и молча
        BuildingUse::Public => STOREY * pick(&MID_STOREYS, seed),
        // мелочь без назначения — постройки частного сектора: сарай во дворе
        // и сам дом, а не старая двух-трёхэтажка. Граница дома — та же, по
        // которой `roofs` кроет его двускатной, а `material` одевает стенами
        // дома: одна граница, один смысл
        BuildingUse::Other if area <= SHED_FOOTPRINT_MAX => pick(&SHED_HEIGHTS, seed),
        BuildingUse::Other if area <= SMALL_FOOTPRINT_MAX => pick(&HOUSE_HEIGHTS, seed),
        // жильё, контора и половина города без назначения — по форме пятна
        _ => STOREY * storeys_by_shape(&building.outer, area, seed),
    }
}

/// Этажность по форме пятна: лента — секция, компактное крупное — башня,
/// мелкое — старый малоэтажный дом.
///
/// **Площадь спрашивается первой**: контур до `LOW_FOOTPRINT_MAX` — старый
/// малоэтажный дом, даже если он лента по сторонам (40 × 7 м — сарай, а не
/// панельная секция). Правило секции поэтому читается «≥ 35 м, ≤ 18 м и
/// крупнее 300 м²».
///
/// Стороны пятна берутся **только там, где они решают**: мелкий контур
/// отвечает одной площадью, и `min_area_rect` (перебор рёбер по всем точкам,
/// то есть O(n²)) на нём не считается вовсе. Ветка вывода зовётся на дом
/// трижды за сборку слоя — фасад/экструзия, тени, строка `heights:` — и
/// умножать на три стоит только ту работу, без которой не обойтись.
fn storeys_by_shape(ring: &[Vec2], area: f32, seed: u32) -> f32 {
    if area <= LOW_FOOTPRINT_MAX {
        return pick(&LOW_STOREYS, seed);
    }
    let (length, width) = footprint_size(ring);
    let squarish = width > 0.0 && length / width <= TOWER_MAX_RATIO;
    if length >= SLAB_MIN_LENGTH && width <= SLAB_MAX_WIDTH {
        pick(&SLAB_STOREYS, seed)
    } else if area >= TOWER_FOOTPRINT_MIN && squarish {
        pick(&TOWER_STOREYS, seed)
    } else {
        pick(&MID_STOREYS, seed)
    }
}

/// Длина и ширина пятна — стороны минимального описанного прямоугольника,
/// длинная первой. Тот же `min_area_rect`, что даёт ось двускатной крыши и
/// ось фактуры кровли; контуры короткие, но перебор в нём квадратичный, и
/// зовётся он отсюда только на крупном пятне без тега высоты — см.
/// [`storeys_by_shape`].
fn footprint_size(ring: &[Vec2]) -> (f32, f32) {
    let Some(rect) = min_area_rect(ring) else {
        return (0.0, 0.0);
    };
    ((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length())
}

/// Разброс высот города одной строкой — доля тега и квантили того, что
/// получилось. Единственный способ увидеть работу вывода без глаз: у Тулы
/// сейчас «32% tagged, median 3 m, p90 15 m, max 82 m»: медиана — это
/// одноэтажный частный дом, которым оказывается большая часть нетегированных
/// 68 %, а форму распределения держит p90. «median 15, p90 15» это прежние
/// картонные коробки, «median 8» — [`HOUSE_HEIGHTS`] до одного этажа, а
/// «p90 27» — первая, небоскрёбная версия таблицы башен (см.
/// [`TOWER_STOREYS`]). Строку надо перечитывать после каждой правки таблиц
/// здесь: она единственная показывает, куда уехало распределение.
/// Сортировка семи тысяч чисел стоит доли миллисекунды и идёт вместе со
/// сборкой слоя, а не каждый кадр.
pub(super) fn height_mix(buildings: &[PolyArea]) -> String {
    if buildings.is_empty() {
        return "no buildings".to_string();
    }
    let tagged = buildings.iter().filter(|b| b.height.is_some()).count();
    let mut heights: Vec<f32> = buildings.iter().map(height_or_default).collect();
    heights.sort_by(f32::total_cmp);
    let at = |share: f32| heights[((heights.len() - 1) as f32 * share) as usize];
    format!(
        "{}% tagged, median {:.0} m, p90 {:.0} m, max {:.0} m",
        tagged * 100 / buildings.len(),
        at(0.5),
        at(0.9),
        at(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::AreaKind;

    fn building(outer: Vec<Vec2>, building_use: BuildingUse) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use,
            height: None,
            storeys: None,
            entrances: Vec::new(),
            colours: Default::default(),
        }
    }

    fn oblong(width: f32, length: f32, at: Vec2) -> Vec<Vec2> {
        vec![
            at,
            at + Vec2::new(length, 0.0),
            at + Vec2::new(length, width),
            at + Vec2::new(0.0, width),
        ]
    }

    #[test]
    fn a_tag_always_wins() {
        let mut tagged = building(oblong(14.0, 60.0, Vec2::ZERO), BuildingUse::Apartments);
        tagged.height = Some(41.5);
        assert_eq!(height_or_default(&tagged), 41.5);
    }

    #[test]
    fn a_long_thin_block_is_a_panel_section() {
        // 60 × 14 — панельная секция: пять, девять или двенадцать этажей
        for offset in 0..12 {
            let at = Vec2::new(offset as f32 * 37.0, offset as f32 * 53.0);
            let slab = building(oblong(14.0, 60.0, at), BuildingUse::Apartments);
            let height = height_or_default(&slab);
            assert!(
                [15.0, 27.0, 36.0].contains(&height),
                "{height} m is not a section"
            );
        }
    }

    #[test]
    fn a_compact_block_is_a_tower_and_a_small_one_is_low() {
        let tower = building(oblong(26.0, 30.0, Vec2::ZERO), BuildingUse::Apartments);
        assert!(height_or_default(&tower) >= 15.0);
        // а вытянутый, но не ленточный корпус — обычный кирпичный дом
        let block = building(oblong(24.0, 60.0, Vec2::ZERO), BuildingUse::Apartments);
        assert!(height_or_default(&block) <= 15.0);
        let low = building(oblong(12.0, 20.0, Vec2::ZERO), BuildingUse::Apartments);
        assert!(height_or_default(&low) <= 12.0);
    }

    #[test]
    fn a_garage_stays_one_storey_and_a_hall_is_a_hall() {
        for offset in 0..8 {
            let at = Vec2::new(offset as f32 * 41.0, 0.0);
            let garage = building(oblong(6.0, 24.0, at), BuildingUse::Garage);
            assert_eq!(height_or_default(&garage), GARAGE_HEIGHT);
            // цех мерится метрами пролёта, а не этажами, и в этажи не растёт
            let hall = building(oblong(40.0, 90.0, at), BuildingUse::Industrial);
            assert!(HALL_HEIGHTS.contains(&height_or_default(&hall)));
        }
    }

    /// У торговли без тега высота тоже делится размером: гипермаркет — это
    /// оболочка над одним высоким залом, магазин у дома — павильон. Половина
    /// крупноформата в OSM без тега высоты («Верный», ТЦ «Перспектива»), и
    /// выведенная коробка обязана встать вровень с размеченной соседкой.
    #[test]
    fn a_hypermarket_is_inferred_as_a_shell_and_a_corner_shop_as_a_pavilion() {
        for offset in 0..8 {
            let at = Vec2::new(offset as f32 * 220.0, 0.0);
            let hyper = building(oblong(80.0, 110.0, at), BuildingUse::Retail);
            assert!(is_big_box(&hyper));
            let height = height_or_default(&hyper);
            assert!(
                BIG_BOX_HEIGHTS.contains(&height),
                "{height} m is not a hypermarket shell"
            );

            let shop = building(oblong(14.0, 20.0, at), BuildingUse::Retail);
            assert!(!is_big_box(&shop));
            let height = height_or_default(&shop);
            assert!(SHOP_HEIGHTS.contains(&height), "{height} m is not a shop");
        }
    }

    #[test]
    fn the_same_shape_still_varies_between_buildings() {
        // одинаковые по форме секции в разных местах города обязаны получаться
        // разной высоты — ради этого весь вывод и затевался
        let heights: Vec<f32> = (0..24)
            .map(|offset| {
                let at = Vec2::new(offset as f32 * 71.0, offset as f32 * 37.0);
                height_or_default(&building(oblong(14.0, 60.0, at), BuildingUse::Apartments))
            })
            .collect();
        let first = heights[0];
        assert!(
            heights.iter().any(|height| *height != first),
            "every section came out {first} m: {heights:?}"
        );
    }

    #[test]
    fn the_inference_is_stable_for_one_building() {
        let slab = building(
            oblong(14.0, 60.0, Vec2::new(123.0, 456.0)),
            BuildingUse::Other,
        );
        assert_eq!(height_or_default(&slab), height_or_default(&slab));
    }

    #[test]
    fn a_tower_is_mostly_nine_storeys() {
        let sixteen = TOWER_STOREYS.iter().filter(|s| **s == 16.0).count();
        let nine = TOWER_STOREYS.iter().filter(|s| **s == 9.0).count();
        assert!(nine * 2 >= TOWER_STOREYS.len(), "девять — не «в основном»");
        assert!(
            sixteen * 8 <= TOWER_STOREYS.len(),
            "шестнадцать — не «изредка»"
        );
    }

    #[test]
    fn a_public_building_is_never_a_tower() {
        // школа в 900 м² с почти квадратным планом: по форме — башня, по
        // назначению — казённое здание в 2–5 этажей
        let school = building(oblong(28.0, 32.0, Vec2::ZERO), BuildingUse::Public);
        assert!(height_or_default(&school) <= 15.0);
    }

    #[test]
    fn an_untagged_box_under_the_pitched_cohort_border_is_a_private_house() {
        // 12 × 20 = 240 м²: `roofs` кроет его двускатной крышей, `material` —
        // стенами дома, значит и стены у него дома, а не 2–4 этажа
        for offset in 0..16 {
            let at = Vec2::new(offset as f32 * 43.0, offset as f32 * 29.0);
            let cottage = building(oblong(12.0, 20.0, at), BuildingUse::Other);
            assert!(HOUSE_HEIGHTS.contains(&height_or_default(&cottage)));
        }
    }
}
