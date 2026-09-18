//! Общие фикстуры тестов зданий: дом, его пятно и оба теневых слоя.
//!
//! Здесь лежит ровно то, что понадобилось **больше чем одному** набору
//! тестов, — после того как теневые, храмовые и арочные тесты разъехались по
//! своим модулям. Копия такого хелпера в каждом из них расходится с
//! оригиналом на первой же правке модели, а тест после этого проверяет не то,
//! что в игре.
//!
//! Всё, что нужно только одному набору (`shadow_rect`, `oblong`, `passage`),
//! остаётся при нём: общим оно не становится от того, что могло бы им быть.

use bevy::prelude::*;

use super::order;
use super::{Lean, RoofDetail};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{
    AreaKind, BuildingUse, Colours, Faith, PolyArea, RoadLine, Sacred, SacredForm,
};

/// Квадрат 10 × 10 в начале координат — дом, у которого важно, что он есть.
pub(super) fn square() -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 0.0),
        Vec2::new(10.0, 10.0),
        Vec2::new(0.0, 10.0),
    ]
}

/// Прямоугольник по двум углам — для домов, у которых важно пятно, а не
/// форма.
pub(super) fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

pub(super) fn building(outer: Vec<Vec2>, height: Option<f32>, kind: AreaKind) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use: BuildingUse::Other,
        height,
        entrances: Vec::new(),
        colours: Colours::default(),
    }
}

pub(super) fn church(outer: Vec<Vec2>, height: f32, faith: Faith, form: SacredForm) -> PolyArea {
    let mut area = building(outer, Some(height), AreaKind::Building);
    area.building_use = BuildingUse::Church(Sacred {
        faith,
        form,
        complex: 0,
        floor_dm: 0,
    });
    area
}

/// Подробность слоя для теста: рампа тона по вкусу, оборудование на кровле
/// выключено — эти тесты про геометрию домов, а коробки на крышах только
/// добавили бы им вершин.
pub(super) fn detail(tinted: bool) -> RoofDetail {
    RoofDetail {
        tinted,
        clutter: false,
    }
}

/// Наземные тени тестовому списку домов. Развёртки строит вызывающий: в игре
/// это делает `mesh_buildings`, один раз на оба теневых слоя.
pub(super) fn ground_shadows(
    list: &[PolyArea],
    passages: &[RoadLine],
    extruded: bool,
) -> MeshBuilder {
    use super::shadows::{ShadowSweeps, shadow_builder};

    shadow_builder(list, passages, &ShadowSweeps::of(list), extruded)
}

/// Тени на кровлях. `extruded` — 2.5D, и порядок отрисовки строится здесь
/// ровно потому, что в игре его делят меш экструзии и этот слой.
pub(super) fn roof_shadows(list: &[PolyArea], extruded: bool) -> MeshBuilder {
    use super::shadows::{ShadowSweeps, roof_shadow_builder};

    let order = extruded.then(|| order::draw_order(list, Lean::of()));
    roof_shadow_builder(list, &ShadowSweeps::of(list), order.as_deref())
}

/// Меш 2.5D-экструзии — с тем же порядком отрисовки, который в игре достаётся
/// заодно и теням на кровлях.
pub(super) fn extruded_mesh(
    list: &[PolyArea],
    passages: &[RoadLine],
    detail: RoofDetail,
) -> MeshBuilder {
    use super::layers::extrusion_builder;

    extrusion_builder(list, passages, detail, &order::draw_order(list, Lean::of())).0
}

/// Вершины меша плоскими точками — тело и кайма вместе.
pub(super) fn mesh_points(mesh: &Mesh) -> Vec<Vec2> {
    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .iter()
        .map(|point| Vec2::new(point[0], point[1]))
        .collect()
}
