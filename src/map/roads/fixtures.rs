//! Общие сцены и извлекатели тестов дорог: то, чем пользуется больше одной
//! темы — корневые `tests.rs` и тесты рядом с модулями. Хелпер, нужный одной
//! теме, живёт в тестах его темы, не здесь.

use super::network::RoadNetwork;
use super::paint;
use super::{RoadReport, RoadShape, RoadStyle, VergeZoomBucket, mesh_roads};
use crate::map::osm::{AreaKind, LotKind, MapData, RoadClass, RoadLine, fixture};
use crate::map::surface::LayerMesh;
use bevy::prelude::*;

pub(crate) fn road(points: Vec<Vec2>, width: f32, passage: bool) -> RoadLine {
    RoadLine {
        class: RoadClass::Alley,
        passage,
        ..fixture::street(points, width)
    }
}

/// Двадцать пять дорожных слоёв снизу вверх, ровно в том порядке, в каком
/// они уходят в мир: газон островов колец и их трава без канта, газон широких
/// обочин лугом и травой двора, двенадцать лент и восемь слоёв краски над своим асфальтом — колея
/// траекторий узла (маска, потом наложение) ниже линий, островки колец над
/// асфальтом стоянок. Грунтовки — под асфальтом улиц, обочины — под всей
/// зеленью, их газон — под их плиткой, газон острова — под всем, его трава —
/// над замапленной травой. Настил переездов (`rail_crossings`) — между
/// шпалами и сталью путей, то есть над всей краской улиц и под мостами.
pub(crate) const LAYERS: [&str; 25] = [
    "ring_islands",
    "road_verge_lawns",
    "road_verge_yards",
    "road_verges",
    "ring_island_grass",
    "alleys",
    "sidewalks",
    "road_medians",
    "unpaved_roads",
    "roads",
    paint::PAINT_WEAR_MASK,
    paint::PAINT_WEAR,
    paint::PAINT_ZEBRAS,
    paint::PAINT_LANES,
    paint::PAINT_AXES,
    "lot_sidewalks",
    "lot_lines",
    paint::PAINT_ISLANDS,
    "rail_crossings",
    "bridge_shadows",
    "bridge_casings",
    "bridges",
    paint::BRIDGE_PAINT_LANES,
    paint::BRIDGE_PAINT_AXES,
    "walls",
];

pub(crate) fn one_street() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)],
        12.0,
    ));
    map
}

pub(crate) fn layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a LayerMesh {
    layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("слой {name} описан на любом стиле"))
}

/// Улица с вершиной посередине и вторая, выходящая из неё **этой же** точкой:
/// у Overpass нет id нод, и перекрёсток восстанавливается по совпадению
/// координат, так что общая вершина обязана быть у обеих.
pub(crate) fn a_tee() -> MapData {
    let mut map = MapData::default();
    map.roads.push(fixture::street(
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(350.0, 100.0),
            Vec2::new(600.0, 100.0),
        ],
        12.0,
    ));
    map.roads.push(fixture::street(
        vec![Vec2::new(350.0, 100.0), Vec2::new(350.0, 400.0)],
        10.0,
    ));
    map
}

pub(crate) fn junctions_with(map: &MapData, markings: bool) -> usize {
    let style = RoadStyle {
        markings,
        ..RoadStyle::default()
    };
    mesh_roads(
        map,
        style,
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    )
    .1
    .junctions
    .count
}

/// Отчёт без часов: два прогона одной карты обязаны совпасть до поля.
pub(crate) fn timeless(map: &MapData) -> RoadReport {
    let (_, mut report) = mesh_roads(
        map,
        RoadStyle::default(),
        RoadShape::default(),
        VergeZoomBucket::for_zoom(0.0),
    );
    report.network = Default::default();
    report.elapsed = Default::default();
    report
}

pub(crate) fn with_network(roads: Vec<RoadLine>) -> MapData {
    MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..default()
    }
}

/// Кольцо радиусом 12 м в начале координат и два односторонних подхода к нему,
/// сходящиеся в одну дорогу в полусотне метров, — въезд и съезд одной улицы.
///
/// `tagged` — несёт ли кольцо `junction=roundabout`, `oneway` — одностороннее ли
/// оно: кольцо без тега в OSM обычное дело.
pub(crate) fn roundabout_with_an_approach(tagged: bool, oneway: bool) -> MapData {
    let circle: Vec<Vec2> = (0..=24)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 24.0) * 12.0)
        .collect();
    let arm = |turn: f32, side: f32| RoadLine {
        oneway: true,
        ..fixture::street(
            vec![circle[(24.0 + turn) as usize % 24], Vec2::new(55.0, side)],
            5.0,
        )
    };
    let mut map = MapData::default();
    map.roads.push(RoadLine {
        oneway,
        roundabout: tagged,
        ..fixture::street(circle.clone(), 8.0)
    });
    map.roads.push(arm(2.0, 1.5));
    map.roads.push(arm(-2.0, -1.5));
    map
}

/// Большая стоянка с дорогой сквозь неё: `through` — односторонняя, уходит за
/// оба края площадки; проезд ряда лежит внутри целиком.
pub(crate) fn ground_with_roads(lot_side: f32) -> MapData {
    let mut map = MapData::default();
    // вид решает разбор по площади: 100 × 100 — большая, 60 × 60 — двор
    let kind = if lot_side * lot_side >= 8000.0 {
        LotKind::Ground
    } else {
        LotKind::Yard
    };
    map.parking.push(fixture::area(
        AreaKind::Parking(kind),
        vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0 + lot_side, 100.0),
            Vec2::new(100.0 + lot_side, 100.0 + lot_side),
            Vec2::new(100.0, 100.0 + lot_side),
        ],
    ));
    let middle = 100.0 + lot_side / 2.0;
    map.roads.push(RoadLine {
        oneway: true,
        ..fixture::street(
            vec![Vec2::new(0.0, middle), Vec2::new(200.0 + lot_side, middle)],
            5.0,
        )
    });
    map.roads.push(fixture::parking_aisle(vec![
        Vec2::new(middle, 110.0),
        Vec2::new(middle, 90.0 + lot_side),
    ]));
    map
}

/// Разделённый проспект (`fixture::divided_avenue`) картой **без сети** —
/// «собранной тестом руками» ([`MapData::network`]): слоям дороги сеть не
/// нужна. Возвращает карту и расстояние между осями.
pub(crate) fn divided_avenue(gap: f32) -> (MapData, f32) {
    let (roads, apart) = fixture::divided_avenue(gap, None);
    (MapData { roads, ..default() }, apart)
}

/// Проспект из двух половин, поперечная жилая пересекает обе: узлы — вершины
/// на половинах, как в OSM.
pub(crate) fn an_avenue_crossed_by_a_street() -> (MapData, f32) {
    let (mut map, apart) = divided_avenue(0.6);
    let (south, north) = (Vec2::new(300.0, 100.0), Vec2::new(300.0, 100.0 + apart));
    map.roads[0].points.insert(1, south);
    map.roads[1].points.insert(1, north);
    map.roads.push(fixture::street(
        vec![
            Vec2::new(300.0, 30.0),
            south,
            north,
            Vec2::new(300.0, 190.0),
        ],
        8.0,
    ));
    (with_network(map.roads), apart)
}

/// Точка на круге радиуса `radius` под углом `degrees`.
pub(crate) fn polar(radius: f32, degrees: f32) -> Vec2 {
    Vec2::from_angle(degrees.to_radians()) * radius
}

/// Вершины слоя цвета `color` — `[x, y]`.
pub(crate) fn vertices_of(layers: &[LayerMesh], name: &str, color: Color) -> Vec<[f32; 2]> {
    let builder = &layer(layers, name).builder;
    let wanted = color.to_linear().to_f32_array();
    builder
        .positions_for_test()
        .iter()
        .zip(builder.colors_for_test())
        .filter(|(_, color)| **color == wanted)
        .map(|(at, _)| [at[0], at[1]])
        .collect()
}

/// «До разрыва» (`ATTRIBUTE_RIBBON`) у вершин слоя `name`, что прошли
/// фильтр; полигоны (нули) не в счёт. У асфальта это второе число, у полосы
/// краски — третье (`MeshBuilder::push_paint_strip`).
pub(crate) fn to_break_where(
    layers: &[LayerMesh],
    name: &str,
    keep: impl Fn(&[f32; 3]) -> bool,
) -> Vec<f32> {
    let builder = &layer(layers, name).builder;
    let slot = if name == "roads" { 1 } else { 2 };
    let coords = builder
        .ribbon_coords_for_test()
        .expect("лента с координатами");
    builder
        .positions_for_test()
        .iter()
        .zip(coords)
        .filter(|(at, coord)| **coord != [0.0; 4] && keep(at))
        .map(|(_, coord)| coord[slot])
        .collect()
}
