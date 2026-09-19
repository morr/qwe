//! Общие фикстуры тестов зданий: пятна домов, их меши и наземные тени.
//!
//! Здесь лежит ровно то, что понадобилось **больше чем одному** набору
//! тестов, — после того как теневые, храмовые и арочные тесты разъехались по
//! своим модулям. Копия такого хелпера в каждом из них расходится с
//! оригиналом на первой же правке модели, а тест после этого проверяет не то,
//! что в игре.
//!
//! Всё, что нужно только одному набору (`shadow_rect` теням, `passage` аркам),
//! остаётся при нём: общим оно не становится от того, что могло бы им быть.

use bevy::prelude::*;

use super::material::WallKind;
use super::order;
use super::{Lean, RoofDetail};
use crate::map::meshing::{MeshBuilder, WallMark, unpack_material};
use crate::map::osm::{AreaKind, BuildingUse, Colours, PolyArea, RoadLine};

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

/// Продолговатое пятно `width × length` от начала координат — корпус, лента,
/// корабль храма: всё, у чего есть длинная сторона.
pub(super) fn oblong(width: f32, length: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(length, 0.0),
        Vec2::new(length, width),
        Vec2::new(0.0, width),
    ]
}

/// Стена ли это, если смотреть на слот материала так, как смотрит шейдер, —
/// числом с плавающей точкой из вершинного атрибута. В слоте лежат два числа
/// сразу (код и этажность), кровля и стена делят его на двоих, и разбирать
/// его в каждом тесте по-своему — верный способ разойтись со словарём.
pub(super) fn is_wall(slot: f32) -> bool {
    WallKind::is_code(unpack_material(slot).0)
}

/// Помечена ли поверхность как «стена без проёмов» — так, как эту метку
/// читает шейдер, но через продакшн-разбор ([`WallMark::of_seed`], зеркало
/// `roof.wgsl::wall_shade`): порог кодировки лежит там, а повторить его здесь
/// своим литералом — верный способ разойтись со словарём.
pub(super) fn is_solid(seed: f32) -> bool {
    WallMark::of_seed(seed) == WallMark::Solid
}

/// Целое ли это число клеток — с допуском на арифметику подъёма.
pub(super) fn whole_cells(cells: f32) -> bool {
    (cells - cells.round()).abs() < 5e-3
}

pub(super) fn building(outer: Vec<Vec2>, height: Option<f32>, kind: AreaKind) -> PolyArea {
    PolyArea {
        outer,
        holes: Vec::new(),
        kind,
        building_use: BuildingUse::Other,
        height,
        storeys: None,
        entrances: Vec::new(),
        colours: Colours::default(),
    }
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
