//! Сборка мешей зданиевых слоёв: полоса фасада с крышами, длинные тени,
//! 2.5D-экструзия. Каждый билдер отдаёт готовый [`MeshBuilder`], а какие из
//! них спавнить в текущем режиме — решает `spawn_buildings` в родителе.

use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;

use super::arches::{arch_openings, arches_by_building, push_arches, push_wall_with_openings};
use super::{
    BuildingHeightMode, SHADOW_COLOR, SHADOW_DIR, base_colors, extrusion_lift, height_or_default,
};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{ring_bounds, signed_ring_area};
use crate::map::osm::{AreaKind, PolyArea, RoadLine};

/// Доля реальной высоты, уходящая в полосу фасада. Рисовать все 60 м башни —
/// значит закрасить полквартала: карта сверху, а не изометрия. При 0.2
/// пятиэтажка (15 м) даёт прежние 3 м, и разница этажности всё равно читается.
const FACADE_SCALE: f32 = 0.2;
/// Границы полосы, м: сарай не должен потерять кромку, небоскрёб — накрыть
/// соседний квартал.
const FACADE_HEIGHT_RANGE: RangeInclusive<f32> = 1.5..=12.0;

/// Метров тени на метр высоты. Пятиэтажка (15 м) отбрасывает 9 м — тень
/// перечёркивает типичную улицу (8–16 м), но не глотает соседний квартал.
pub(super) const SHADOW_LENGTH_SCALE: f32 = 0.6;
/// Границы длины тени, м: у сарая тень обязана остаться заметной, у башни —
/// не накрыть полкарты.
const SHADOW_LENGTH_RANGE: RangeInclusive<f32> = 3.0..=45.0;

/// Высота, на которой рампа тона крыш выходит в максимум: Тула почти вся
/// ниже 75 м, и sqrt в формуле отдаёт разрешение диапазону 5–30 м.
const ROOF_TINT_MAX_HEIGHT: f32 = 60.0;
/// Цвет крыши «в пределе»: темнее и глуше базового, но всё ещё тёплый —
/// высотки читаются с общего плана, не превращая карту в теплокарту.
const ROOF_TALL_COLOR: Color = Color::srgb(0.71, 0.63, 0.55);
/// Насколько рампа может увести крышу к `ROOF_TALL_COLOR` в пределе.
const ROOF_TINT_MAX_MIX: f32 = 0.7;

/// Осветление верхних вершин стены — дешёвый вертикальный градиент.
const WALL_TOP_LIGHTEN: f32 = 0.15;

/// Цвет крыши: базовый по типу, при `tinted` — рампа по высоте (Кремль и
/// здания без высоты рампу пропускают), поверх — лёгкая вариация тона по
/// индексу, чтобы кварталы не сливались.
pub(super) fn roof_color(building: &PolyArea, index: usize, tinted: bool) -> LinearRgba {
    let (roof_base, _) = base_colors(building);
    let ramped = match building.height {
        Some(height) if tinted && building.kind != AreaKind::Kremlin => {
            let t = (height / ROOF_TINT_MAX_HEIGHT).clamp(0.0, 1.0).sqrt();
            roof_base.mix(&ROOF_TALL_COLOR, t * ROOF_TINT_MAX_MIX)
        }
        _ => roof_base,
    };
    let tint = 1.0 - (index % 3) as f32 * 0.025;
    LinearRgba::from(ramped.to_srgba() * tint)
}

/// Фасадная полоса + крыши (режимы Facade / Shadows / ShadowsTint).
pub(super) fn facade_and_roof_builders(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    tinted: bool,
) -> (MeshBuilder, MeshBuilder) {
    let arches = arches_by_building(buildings, passages);
    let mut facades = MeshBuilder::default();
    let mut roofs = MeshBuilder::default();
    for (index, building) in buildings.iter().enumerate() {
        let (_, facade_color) = base_colors(building);

        // фасад — тот же контур, сдвинутый вниз: тёмная кромка видна
        // только вдоль южных граней любого полигона. Сдвиг — по высоте из
        // OSM, так что этажность города видна прямо на карте
        let facade_height = (height_or_default(building) * FACADE_SCALE)
            .clamp(*FACADE_HEIGHT_RANGE.start(), *FACADE_HEIGHT_RANGE.end());
        let offset = Vec2::new(0.0, -facade_height);
        let facade_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + offset).collect();
        let facade_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + offset).collect())
            .collect();
        facades.push_polygon(&facade_outer, &facade_holes, facade_color.to_linear());
        // крыши — отдельный слой поверх фасадов, так что вырезать проём из
        // полосы достаточно: над аркой крыша останется целой сама собой
        if let Some(passages) = arches.get(&index) {
            push_arches(&mut facades, building, passages, offset);
        }
        roofs.push_polygon(
            &building.outer,
            &building.holes,
            roof_color(building, index, tinted),
        );
    }
    (facades, roofs)
}

/// Тени зданий: на каждую непрерывную цепочку рёбер-силуэта внешнего кольца —
/// **один** свип-полигон `[цепочка, цепочка + сдвиг в обратном порядке]`.
/// Не квады на ребро: у ступенчатого фасада квады соседних ступеней
/// перекрываются вдоль тени, и полупрозрачность складывалась в полосы двойной
/// темноты. Свип цепочки самопересечься не может: перп-шаг ребра силуэта
/// равен `outward·SHADOW_DIR > 0`, то есть цепочка монотонна вдоль
/// перпендикуляра тени.
///
/// Затем **все** свипы карты объединяются булевым union (`i_overlay`) в набор
/// непересекающихся фигур с дырками: тени смежных корпусов и соседних зданий
/// перекрываются на земле, а любое наложение внутри одного полупрозрачного
/// слоя читается как пятно двойной темноты. После union альфа везде ровно
/// одна. Часть тени под зданиями закрывают их непрозрачные слои. Дыры (дворы)
/// пропускаются: их тень падает внутрь футпринта.
/// `extruded` — арки в 2.5D прорезаны по-настоящему, и сквозь дыру видна
/// голая дорога: без заплатки тени проём светится, хотя физически он затенён
/// перемычкой. Заплатка кладётся сюда, в теневой слой: он ниже зданий и
/// просвечивает ровно сквозь вырез.
pub(super) fn shadow_builder(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    extruded: bool,
) -> MeshBuilder {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let mut sweeps: Vec<Vec<[f32; 2]>> = Vec::new();
    for building in buildings {
        let length = (height_or_default(building) * SHADOW_LENGTH_SCALE)
            .clamp(*SHADOW_LENGTH_RANGE.start(), *SHADOW_LENGTH_RANGE.end());
        let offset = SHADOW_DIR * length;
        for chain in silhouette_chains(&building.outer, SHADOW_DIR) {
            let mut sweep: Vec<Vec2> = chain.clone();
            sweep.extend(chain.iter().rev().map(|&point| point + offset));
            // NonZero гасит контуры противоположного обхода — свипы обязаны
            // быть одинаково закручены, а обход source-колец OSM произволен
            if signed_ring_area(&sweep) < 0.0 {
                sweep.reverse();
            }
            sweeps.push(sweep.into_iter().map(|point| [point.x, point.y]).collect());
        }
    }

    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    for shape in sweeps.simplify_shape(FillRule::NonZero) {
        let mut rings = shape.into_iter().map(|contour| {
            contour
                .into_iter()
                .map(Vec2::from_array)
                .collect::<Vec<Vec2>>()
        });
        let Some(outer) = rings.next() else {
            continue;
        };
        let holes: Vec<Vec<Vec2>> = rings.collect();
        builder.push_polygon(&outer, &holes, color);
    }

    if extruded {
        for (index, passages) in arches_by_building(buildings, passages) {
            let building = &buildings[index];
            let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
            for opening in arch_openings(building, &passages, lift) {
                let Some(along) = (opening.b - opening.a).try_normalize() else {
                    continue;
                };
                let (p0, p1) = (
                    opening.a + along * opening.low,
                    opening.a + along * opening.high,
                );
                builder.push_quad([p0, p1, p1 + opening.sill, p0 + opening.sill], color);
            }
        }
    }
    builder
}

/// Непрерывные (циклически) цепочки рёбер-силуэта кольца — рёбер, чья
/// наружная нормаль смотрит по `direction`. Обход начинается после
/// освещённого ребра, чтобы цепочка не рвалась на шве кольца.
pub(super) fn silhouette_chains(ring: &[Vec2], direction: Vec2) -> Vec<Vec<Vec2>> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_ring_area(ring).signum();
    let count = ring.len();
    let is_silhouette = |index: usize| {
        let edge = ring[(index + 1) % count] - ring[index];
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        outward.dot(direction) > 0.0
    };
    let Some(lit) = (0..count).find(|&index| !is_silhouette(index)) else {
        // у простого кольца все рёбра силуэтными быть не могут — кривой
        // контур OSM остаётся без тени, а не роняет карту
        return Vec::new();
    };

    let mut chains: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for step in 1..=count {
        let index = (lit + step) % count;
        if is_silhouette(index) {
            if current.is_empty() {
                current.push(ring[index]);
            }
            current.push(ring[(index + 1) % count]);
        } else if !current.is_empty() {
            chains.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chains.push(current);
    }
    chains
}

/// 2.5D-экструзия: painter's algorithm внутри одного меша — треугольники
/// растеризуются в порядке index-буфера, поэтому здания пишутся с севера на
/// юг (южное поверх), на здание сначала стены, потом крыша. Фасадной полосы
/// в этом режиме нет — её заменяют настоящие стены; `tinted` включает рампу
/// тона крыш, как в `ShadowsTint`.
pub(super) fn extrusion_builder(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    tinted: bool,
) -> MeshBuilder {
    let arches = arches_by_building(buildings, passages);
    let mut order: Vec<usize> = (0..buildings.len()).collect();
    order.sort_by(|&a, &b| {
        let center_y = |building: &PolyArea| {
            let (min, max) = ring_bounds(&building.outer);
            min.y + max.y
        };
        center_y(&buildings[b]).total_cmp(&center_y(&buildings[a]))
    });

    let mut builder = MeshBuilder::default();
    for index in order {
        let building = &buildings[index];
        let (_, facade_color) = base_colors(building);
        // через тот же хелпер, что и оверлей дверей, — иначе они разъедутся
        let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);

        let wall_bottom = facade_color.to_linear();
        let wall_top = facade_color
            .mix(&Color::WHITE, WALL_TOP_LIGHTEN)
            .to_linear();
        // арки вырезаются из стен по-настоящему: сквозь проём видны нижние
        // слои — дорога, идущая сквозь дом, и всё, что движок рисует под ней
        let openings = arches
            .get(&index)
            .map(|passages| arch_openings(building, passages, lift))
            .unwrap_or_default();
        // при сдвиге крыши строго вверх видимы только стены южных рёбер
        for (a, b) in silhouette_edges(&building.outer, Vec2::NEG_Y) {
            push_wall_with_openings(&mut builder, a, b, lift, &openings, wall_bottom, wall_top);
        }
        // двор: видима внутренняя стена его северной стороны — та, чья
        // наружная (для кольца дыры) нормаль смотрит вверх
        for hole in &building.holes {
            for (a, b) in silhouette_edges(hole, Vec2::Y) {
                push_wall_with_openings(&mut builder, a, b, lift, &openings, wall_bottom, wall_top);
            }
        }

        let roof_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + lift).collect();
        let roof_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + lift).collect())
            .collect();
        builder.push_polygon(
            &roof_outer,
            &roof_holes,
            roof_color(building, index, tinted),
        );
    }
    builder
}

/// Рёбра кольца, чья наружная нормаль смотрит по `direction` — силуэт с
/// подветренной стороны. Обход кольца (CW/CCW) учитывается по знаковой
/// площади, так что результат от него не зависит.
pub(super) fn silhouette_edges(ring: &[Vec2], direction: Vec2) -> Vec<(Vec2, Vec2)> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_ring_area(ring).signum();
    let mut edges = Vec::new();
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let edge = b - a;
        // для CCW-кольца наружная нормаль ребра — правый перпендикуляр
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        if outward.dot(direction) > 0.0 {
            edges.push((a, b));
        }
    }
    edges
}
