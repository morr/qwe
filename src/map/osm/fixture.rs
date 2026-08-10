//! Тестовый город: одна `MapData`, из которой строятся **оба** заполнения —
//! сеточное и полигональное. Общая фикстура вместо ручной сборки карты в
//! каждом тесте: инвариант «одно правило для двух заполнений» проверяется
//! паритетными тестами (`navigation/parity_tests.rs`) именно на ней.
//!
//! Планировка — по классам реальных инцидентов и правил заливки:
//! река через всю карту с единственным мостом; труба-кульверт, которая не
//! блокирует вовсе; здание-перегородка с аркой шире капа; сухопутный мост с
//! бордюрами и примыкающей дорогой; вода с островом-дырой, открытым мостом
//! (класс «непроходимая стартовая зона Парижа»). Зоны разнесены по карте,
//! чтобы правила не накладывались друг на друга.

use bevy::prelude::*;

use super::model::{
    AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine, WaterKind, WaterLine,
};

pub fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

pub fn building(outer: Vec<Vec2>, holes: Vec<Vec<Vec2>>) -> PolyArea {
    PolyArea {
        outer,
        holes,
        kind: AreaKind::Building,
        height: None,
        entrances: vec![],
    }
}

pub fn water_area(outer: Vec<Vec2>, holes: Vec<Vec<Vec2>>) -> PolyArea {
    PolyArea {
        outer,
        holes,
        kind: AreaKind::Water,
        height: None,
        entrances: vec![],
    }
}

pub fn street(points: Vec<Vec2>, width: f32) -> RoadLine {
    RoadLine {
        points,
        width,
        class: RoadClass::Street,
        bridge: false,
        passage: false,
    }
}

pub fn bridge(points: Vec<Vec2>, width: f32) -> RoadLine {
    RoadLine {
        bridge: true,
        ..street(points, width)
    }
}

pub fn passage(points: Vec<Vec2>, width: f32) -> RoadLine {
    RoadLine {
        passage: true,
        ..street(points, width)
    }
}

pub fn stream(points: Vec<Vec2>, width: f32) -> WaterLine {
    WaterLine {
        points,
        width,
        kind: WaterKind::Stream,
        tunnel: false,
    }
}

pub fn culvert(points: Vec<Vec2>, width: f32) -> WaterLine {
    WaterLine {
        tunnel: true,
        ..stream(points, width)
    }
}

pub fn wall(points: Vec<Vec2>, width: f32) -> WallLine {
    WallLine { points, width }
}

/// Город и его ориентиры — мировые точки, на которых паритетные тесты
/// сверяют вердикты двух заполнений. Имена — по зонам планировки.
pub struct TinyCity {
    pub map: MapData,
    /// Открытое место западной зоны; от него сеточный прунинг заливает
    /// достижимость — как от портала в игре.
    pub portal: Vec2,

    /// Южный берег реки, у моста.
    pub south_bank: Vec2,
    /// Северный берег реки, у моста.
    pub north_bank: Vec2,
    /// Точка в русле реки — вода, непроходимо в обоих заполнениях.
    pub river_water: Vec2,

    /// Южнее трубы-кульверта.
    pub culvert_south: Vec2,
    /// Севернее трубы-кульверта.
    pub culvert_north: Vec2,
    /// Точка прямо на линии трубы: кульверт не блокирует вовсе.
    pub culvert_mouth: Vec2,

    /// Западнее здания-перегородки, перед аркой.
    pub west_gate: Vec2,
    /// Восточнее здания-перегородки, за аркой.
    pub east_gate: Vec2,
    /// Середина арочного коридора.
    pub arch_center: Vec2,
    /// Внутри здания рядом с коридором, дальше капа ширины арки.
    pub beside_arch: Vec2,
    /// Толща здания-перегородки вдали от арки.
    pub inside_wall_building: Vec2,

    /// Середина настила сухопутного моста.
    pub dry_deck: Vec2,
    /// Середина полосы бордюра сухопутного моста — блок в обоих заполнениях.
    pub dry_curb: Vec2,
    /// Сразу за бордюром, на земле — снова проходимо.
    pub past_dry_curb: Vec2,
    /// Бордюр в створе примыкающей дороги: примыкание открывает его.
    pub joined_curb_gap: Vec2,

    /// Берег у моста на остров.
    pub island_bank: Vec2,
    /// Остров — дыра водного мультиполигона, открытая настилом моста.
    pub island: Vec2,
    /// Вода вокруг острова.
    pub island_water: Vec2,
}

/// Ширина реки и сухопутного моста подобраны так, чтобы полосы (настил,
/// бордюры) были заметно шире навтайла и радиуса агента — пробные точки
/// стоят от кромок дальше, чем дотягиваются и дискретизация сетки, и
/// инфляция меша.
pub fn tiny_city() -> TinyCity {
    let mut map = MapData::default();

    // Река: рассекает карту на юг/север; единственный переход — мост x=1000.
    map.water_lines.push(stream(
        vec![Vec2::new(0.0, 500.0), Vec2::new(5600.0, 500.0)],
        8.0,
    ));
    map.roads.push(bridge(
        vec![Vec2::new(1000.0, 440.0), Vec2::new(1000.0, 560.0)],
        6.0,
    ));

    // Кульверт: та же линия водотока, но труба — не блокирует вовсе.
    map.water_lines.push(culvert(
        vec![Vec2::new(0.0, 900.0), Vec2::new(5600.0, 900.0)],
        4.0,
    ));

    // Здание-перегородка на всю высоту северной половины (и дальше краёв,
    // чтобы вдоль кромки карты не оставалось щели), арка — единственный
    // проход восток-запад. Ширина way больше капа арки.
    map.buildings.push(building(
        rect(Vec2::new(3000.0, -10.0), Vec2::new(3060.0, 3710.0)),
        vec![],
    ));
    map.roads.push(passage(
        vec![Vec2::new(2950.0, 2000.0), Vec2::new(3110.0, 2000.0)],
        6.0,
    ));

    // Сухопутный мост (эстакада): бордюры блокируют сход с настила вбок.
    // Средняя вершина — узел, которым примыкает боковая улица: она
    // открывает бордюр в своём створе.
    map.roads.push(bridge(
        vec![
            Vec2::new(800.0, 2000.0),
            Vec2::new(800.0, 2150.0),
            Vec2::new(800.0, 2300.0),
        ],
        8.0,
    ));
    map.roads.push(street(
        vec![Vec2::new(800.0, 2150.0), Vec2::new(600.0, 2150.0)],
        5.0,
    ));

    // Вода с островом-дырой; мост с юга разрезает водное кольцо и открывает
    // остров (иначе он — дыра результата и отбрасывается как недостижимый).
    map.water.push(water_area(
        rect(Vec2::new(1500.0, 2500.0), Vec2::new(2100.0, 3100.0)),
        vec![rect(Vec2::new(1700.0, 2700.0), Vec2::new(1900.0, 2900.0))],
    ));
    map.roads.push(bridge(
        vec![Vec2::new(1800.0, 2400.0), Vec2::new(1800.0, 2800.0)],
        20.0,
    ));

    let curb = crate::map::footprint::bridge_curb_width(8.0);
    TinyCity {
        map,
        portal: Vec2::new(400.0, 1200.0),

        south_bank: Vec2::new(1000.0, 400.0),
        north_bank: Vec2::new(1000.0, 620.0),
        river_water: Vec2::new(2500.0, 500.0),

        culvert_south: Vec2::new(2000.0, 800.0),
        culvert_north: Vec2::new(2000.0, 1000.0),
        culvert_mouth: Vec2::new(2000.0, 900.0),

        west_gate: Vec2::new(2900.0, 2000.0),
        east_gate: Vec2::new(3160.0, 2000.0),
        arch_center: Vec2::new(3030.0, 2000.0),
        beside_arch: Vec2::new(3030.0, 2007.0),
        inside_wall_building: Vec2::new(3030.0, 2600.0),

        dry_deck: Vec2::new(800.0, 2075.0),
        dry_curb: Vec2::new(800.0 + 4.0 + curb / 2.0, 2075.0),
        past_dry_curb: Vec2::new(800.0 + 4.0 + curb + 3.0, 2075.0),
        joined_curb_gap: Vec2::new(800.0 - 4.0 - curb / 2.0, 2150.0),

        island_bank: Vec2::new(1800.0, 2350.0),
        island: Vec2::new(1870.0, 2850.0),
        island_water: Vec2::new(1600.0, 2600.0),
    }
}
