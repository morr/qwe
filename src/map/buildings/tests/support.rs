//! Хелперы, общие для тестов зданиевого слоя: веры, коробки, полотна дверей,
//! метки стен, пары домов и планы сборки.

use super::super::fixtures::{building, is_wall, square};
use super::super::material::{DOOR_CODE, RoofKind, RoofLook, WallKind, WallLook};
use super::super::roofs::RoofShape;
use super::super::*;
use crate::map::meshing::{MeshBuilder, unpack_material};
use crate::map::osm::{AreaKind, BuildingUse, Faith, Sacred, SacredForm};

/// Православный храм — назначение, которое тестам кровель и стен нужно чаще
/// прочих вер.
pub(super) const ORTHODOX: BuildingUse = BuildingUse::Church(Sacred {
    faith: Faith::Orthodox,
    form: SacredForm::Nave,
    complex: 0,
    floor_dm: 0,
});

/// Цвет фриза, размеченный тегом: `layers::walls::brand_color` предпочитает
/// `building:colour` палитре, так что полоса красная при любом посеве — а фасад
/// коробки серый, и по красному каналу её геометрия отличается от всей прочей.
pub(super) const BRAND: [u8; 3] = [200, 30, 30];

/// Частный дом — пятно без высоты, высота выводится по форме и посеву.
pub(super) fn house(outer: Vec<Vec2>) -> PolyArea {
    let mut house = building(outer, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    house
}

/// Торговая коробка: пятно больше `BIG_BOX_AREA_MIN`, назначение — торговля,
/// цвет фриза размечен.
pub(super) fn big_box(outer: Vec<Vec2>, height: f32) -> PolyArea {
    let mut retail = building(outer, Some(height), AreaKind::Building);
    retail.building_use = BuildingUse::Retail;
    retail.colours.wall = Some(BRAND);
    retail
}

/// Где на стене оказались дверные полотна: `x` каждой вершины кода
/// [`DOOR_CODE`], в порядке сборки.
pub(super) fn door_vertices(building: &PolyArea, kind: WallKind) -> Vec<Vec2> {
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

/// Метки стен одного дома при **заказанной** облицовке. Заказана она потому,
/// что материал дому выбирает посев, а эти тесты не про лотерею: без заказа
/// пятиэтажка раз в десять выпадений оказалась бы штукатуркой, и тест про
/// балконы падал бы по причине, к балконам отношения не имеющей.
pub(super) fn wall_marks(building: &PolyArea, kind: WallKind) -> Vec<f32> {
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

/// Тульская пара из отчёта: Г-образная пятиэтажка (её северо-восточное крыло
/// уходит за соседа) и девятиэтажка к востоку. Числа — пятна 493864392 и
/// 493864388, сдвинутые в ноль. Высоты в OSM у них не проставлены и в игре
/// выводятся по форме и посеву; здесь они заданы явно — те самые 15 и 27 м,
/// чтобы тест держал геометрию, а не таблицу этажности.
pub(super) fn leaning_pair() -> (PolyArea, PolyArea) {
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

/// Дом со ступенчатым фасадом: западная секция выдвинута к зрителю на 3 м,
/// восточная стоит за ней. Ровно тот случай, на котором обход контура даёт
/// обратный порядок — южные стены секций встречаются на экране на ширину
/// ступени.
pub(super) fn stepped_facade() -> PolyArea {
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

/// Квадрат 10 × 10 с высотой — тот же дом, на котором стоят тесты билдеров
/// выше.
pub(super) fn one_building() -> PolyArea {
    building(square(), Some(9.0), AreaKind::Building)
}

/// План сборки на дальней ступени зума: оборудование на кровле выключено —
/// эти тесты про состав слоёв, а коробки на крышах только добавили бы вершин
/// (то же решение, что у хелпера [`super::super::fixtures::detail`]).
pub(super) fn building_plan(mode: BuildingHeightMode, shadows: bool) -> BuildingPlan {
    BuildingPlan {
        mode,
        bucket: BuildingZoomBucket::default(),
        shadows,
    }
}

pub(super) fn layer_names(layers: &[LayerMesh]) -> Vec<&'static str> {
    layers.iter().map(|layer| layer.name).collect()
}
