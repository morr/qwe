//! Сколько этажей у дома, о котором OSM молчит.
//!
//! Высота есть у 31 % контуров Туры и у 5 % Токио — у остальных её приходится
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
use super::roofs::min_area_rect;
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{BuildingUse, PolyArea};

/// Высота этажа, м — то же число, которым парсер переводит
/// `building:levels` в метры, и по нему же считаются серии ниже.
const STOREY: f32 = 3.0;

/// Гараж — одна коробка в один этаж, и никакого разброса: ряды гаражных
/// боксов на снимке все одной высоты.
const GARAGE_HEIGHT: f32 = 3.0;

/// Частный дом: один-два этажа с мансардой. Разброс небольшой, но он есть —
/// в частном секторе соседние дома никогда не одного роста.
const HOUSE_HEIGHTS: [f32; 4] = [5.0, 6.0, 7.0, 8.0];

/// Панельная секция: пятиэтажка, девятиэтажка, изредка двенадцать. Доли —
/// по тому, чего в русском городе больше.
const SLAB_STOREYS: [f32; 10] = [5.0, 5.0, 5.0, 5.0, 5.0, 9.0, 9.0, 9.0, 12.0, 12.0];
/// Башня — компактное **крупное** пятно: в основном девять, изредка
/// шестнадцать. Первый вариант таблицы был вдвое выше, и город вышел
/// небоскрёбным: у Тулы p90 поднялась до 27 м, тогда как девятиэтажка там
/// уже редкость.
const TOWER_STOREYS: [f32; 8] = [5.0, 9.0, 9.0, 9.0, 12.0, 12.0, 16.0, 16.0];
/// Всё остальное крупное — кирпичный корпус, школа, контора: два-пять этажей.
/// Это самая населённая ветка вывода, и она обязана быть низкой.
const MID_STOREYS: [f32; 6] = [2.0, 3.0, 3.0, 4.0, 4.0, 5.0];
/// Старый дом в центре: два-четыре этажа.
const LOW_STOREYS: [f32; 6] = [2.0, 2.0, 3.0, 3.0, 3.0, 4.0];
/// Казённое здание — школа, поликлиника, контора.
const PUBLIC_STOREYS: [f32; 6] = [2.0, 3.0, 3.0, 4.0, 4.0, 5.0];
/// Храм: не этажами, а сразу метрами — у него один «этаж» до карниза.
const CHURCH_HEIGHTS: [f32; 4] = [12.0, 14.0, 18.0, 22.0];
/// Цех и склад — тоже метрами: один пролёт, но высокий.
const HALL_HEIGHTS: [f32; 4] = [7.0, 8.0, 10.0, 12.0];
/// Торговый зал: один-два этажа под большой крышей.
const STORE_HEIGHTS: [f32; 3] = [6.0, 7.0, 9.0];

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
pub(super) fn height_or_default(building: &PolyArea) -> f32 {
    building
        .height
        .unwrap_or_else(|| inferred_height(building, building_seed(building)))
}

/// Выведенная высота — та самая, которой в OSM не нашлось.
fn inferred_height(building: &PolyArea, seed: u32) -> f32 {
    let (length, width) = footprint_size(&building.outer);
    let area = signed_ring_area(&building.outer).abs();
    let slot = |count: usize| (seed >> 3) as usize % count;

    match building.building_use {
        BuildingUse::Garage => GARAGE_HEIGHT,
        BuildingUse::House => HOUSE_HEIGHTS[slot(HOUSE_HEIGHTS.len())],
        BuildingUse::Church => CHURCH_HEIGHTS[slot(CHURCH_HEIGHTS.len())],
        BuildingUse::Industrial => HALL_HEIGHTS[slot(HALL_HEIGHTS.len())],
        BuildingUse::Commercial if area >= STORE_FOOTPRINT_MIN => {
            STORE_HEIGHTS[slot(STORE_HEIGHTS.len())]
        }
        BuildingUse::Public => STOREY * PUBLIC_STOREYS[slot(PUBLIC_STOREYS.len())],
        // жильё, контора и половина города без назначения — по форме пятна
        _ => STOREY * storeys_by_shape(length, width, area, seed),
    }
}

/// Этажность по форме пятна: лента — секция, компактное крупное — башня,
/// мелкое — старый малоэтажный дом.
fn storeys_by_shape(length: f32, width: f32, area: f32, seed: u32) -> f32 {
    let slot = |count: usize| (seed >> 3) as usize % count;
    let squarish = width > 0.0 && length / width <= TOWER_MAX_RATIO;
    if area <= LOW_FOOTPRINT_MAX {
        LOW_STOREYS[slot(LOW_STOREYS.len())]
    } else if length >= SLAB_MIN_LENGTH && width <= SLAB_MAX_WIDTH {
        SLAB_STOREYS[slot(SLAB_STOREYS.len())]
    } else if area >= TOWER_FOOTPRINT_MIN && squarish {
        TOWER_STOREYS[slot(TOWER_STOREYS.len())]
    } else {
        MID_STOREYS[slot(MID_STOREYS.len())]
    }
}

/// Длина и ширина пятна — стороны минимального описанного прямоугольника,
/// длинная первой. Тот же `min_area_rect`, что даёт ось двускатной крыши и
/// ось фактуры кровли; контуры короткие, и третий его вызов на дом стоит
/// доли миллисекунды на весь город.
fn footprint_size(ring: &[Vec2]) -> (f32, f32) {
    let Some(rect) = min_area_rect(ring) else {
        return (0.0, 0.0);
    };
    ((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length())
}

/// Разброс высот города одной строкой — доля тега и квантили того, что
/// получилось. Единственный способ увидеть работу вывода без глаз: «median 15,
/// p90 27» это живой город, «median 15, p90 15» — прежние картонные коробки.
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
            entrances: Vec::new(),
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
}
