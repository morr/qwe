//! Тестовая геометрия карты — на двух уровнях конвейера.
//!
//! **После разбора**: билдеры элементов `MapData` и две готовые сцены.
//! [`tiny_city`] — составной город, из которого строятся **оба** заполнения,
//! сеточное и полигональное: общая фикстура вместо ручной сборки карты в
//! каждом тесте, а инвариант «одно правило для двух заполнений» проверяется
//! паритетными тестами (`navigation/parity_tests.rs`) именно на ней.
//! [`crowded_yard`] — плотная сцена для повтора прогона, где вся толпа и
//! портал стоят друг на друге.
//!
//! **До разбора**: [`Overpass`] — ответ выгрузки, собранный из сцены в метрах
//! карты. Тесты разбора (`parse/tests.rs`) проверяют правила «такой тег с
//! такой геометрией даёт вот это», и формат ответа в них — адаптер, а не
//! предмет: раньше каждый писал свой `format!` с экранированными скобками и
//! своей четвёркой `lat±d`/`lon±d`. Теперь формат живёт здесь в одном
//! экземпляре, а тест говорит тегами и точками.
//!
//! Модуль не спрятан за `#[cfg(test)]` намеренно: интеграционные тесты
//! (`tests/navigation.rs`) собирают библиотеку без `cfg(test)` и иначе его не
//! видят. В рантайме им никто не пользуется.
//!
//! Планировка — по классам реальных инцидентов и правил заливки:
//! река через всю карту с единственным мостом; труба-кульверт, которая не
//! блокирует вовсе; здание-перегородка с аркой шире капа; сухопутный мост с
//! бордюрами и примыкающей дорогой; вода с островом-дырой, открытым мостом
//! (класс «непроходимая стартовая зона Парижа»). Зоны разнесены по карте,
//! чтобы правила не накладывались друг на друга.

use bevy::prelude::*;
use serde_json::{Value, json};

use super::model::{
    AreaKind, MapData, PolyArea, RailKind, RailLine, RoadClass, RoadLine, WallLine, WaterKind,
    WaterLine,
};
use super::overpass::GeoBounds;
use crate::city::City;
use crate::settings::MAP_SIZE;

pub fn rect(min: Vec2, max: Vec2) -> Vec<Vec2> {
    vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

/// Квадрат со стороной `2 · half` вокруг точки — самая частая фигура фикстур
/// разбора: массив леса, пруд, дом.
pub fn square(center: Vec2, half: f32) -> Vec<Vec2> {
    rect(center - Vec2::splat(half), center + Vec2::splat(half))
}

/// Кольцо в виде way: OSM замыкает площадь повторением первой точки, и
/// разбор отличает площадь от линии именно по этому повтору.
pub fn closed(mut ring: Vec<Vec2>) -> Vec<Vec2> {
    if let Some(&first) = ring.first() {
        ring.push(first);
    }
    ring
}

/// Ответ Overpass, собранный из сцены в метрах карты.
///
/// Точки задаются в том же пространстве, в котором тест потом проверяет
/// результат (`MAP_SIZE / 2.0` — центр карты), и переводятся в гео-координаты
/// на выходе — [`GeoBounds::unproject`]. Идентификаторы элементов раздаются
/// по порядку: разбору они безразличны, но в настоящей выгрузке они есть.
pub struct Overpass {
    city: City,
    bounds: GeoBounds,
    elements: Vec<Value>,
}

impl Overpass {
    pub fn new(city: City) -> Self {
        Self {
            city,
            bounds: GeoBounds::for_city(city),
            elements: vec![],
        }
    }

    /// Линия: дорога, путь, водоток, ряд деревьев.
    pub fn way(mut self, tags: &[(&str, &str)], points: Vec<Vec2>) -> Self {
        let element = json!({
            "type": "way",
            "id": self.next_id(),
            "tags": Self::tags(tags),
            "geometry": self.geometry(&points),
        });
        self.elements.push(element);
        self
    }

    /// Площадь: тот же way, но замкнутый.
    pub fn area(self, tags: &[(&str, &str)], ring: Vec<Vec2>) -> Self {
        self.way(tags, closed(ring))
    }

    /// Нода: вход в здание, одиночное дерево. Координаты у неё лежат прямо в
    /// элементе, а не в `geometry`.
    pub fn node(mut self, tags: &[(&str, &str)], point: Vec2) -> Self {
        let geo = self.bounds.unproject(point);
        let element = json!({
            "type": "node",
            "id": self.next_id(),
            "tags": Self::tags(tags),
            "lat": geo.lat,
            "lon": geo.lon,
        });
        self.elements.push(element);
        self
    }

    /// Мультиполигон: члены с ролями `outer` / `inner`. Куски контура тут не
    /// замыкаются — в OSM внешнее кольцо часто разрезано на несколько way, и
    /// сборка колец из обрывков как раз проверяется.
    pub fn relation(mut self, tags: &[(&str, &str)], members: &[(&str, Vec<Vec2>)]) -> Self {
        let members: Vec<Value> = members
            .iter()
            .map(|(role, points)| {
                json!({
                    "type": "way",
                    "role": role,
                    "geometry": self.geometry(points),
                })
            })
            .collect();
        let element = json!({
            "type": "relation",
            "id": self.next_id(),
            "tags": Self::tags(tags),
            "members": members,
        });
        self.elements.push(element);
        self
    }

    pub fn json(&self) -> String {
        json!({ "elements": self.elements }).to_string()
    }

    /// Разобранная карта. Идёт через текст, а не мимо него: десериализация
    /// ответа — часть того, что проверяют тесты разбора.
    pub fn parse(&self) -> MapData {
        super::parse::parse(&self.json(), self.city).expect("fixture must parse")
    }

    fn next_id(&self) -> u64 {
        self.elements.len() as u64 + 1
    }

    fn tags(pairs: &[(&str, &str)]) -> Value {
        Value::Object(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
                .collect(),
        )
    }

    fn geometry(&self, points: &[Vec2]) -> Value {
        Value::Array(
            points
                .iter()
                .map(|&point| {
                    let geo = self.bounds.unproject(point);
                    json!({ "lat": geo.lat, "lon": geo.lon })
                })
                .collect(),
        )
    }
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

pub fn wood(outer: Vec<Vec2>) -> PolyArea {
    PolyArea {
        outer,
        holes: vec![],
        kind: AreaKind::Wood,
        height: None,
        entrances: vec![],
    }
}

pub fn rail(points: Vec<Vec2>, width: f32) -> RailLine {
    RailLine {
        points,
        width,
        kind: RailKind::Active,
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

/// Полустрона двора [`crowded_yard`], м.
const YARD_HALF: f32 = 60.0;

/// Плотная сцена для повтора прогона (`determinism::replay`): глухая застройка
/// во всю карту с единственным двором, в центре которого стоит портал.
///
/// Плотность здесь — не декорация, а условие проверки. На настоящей карте
/// пешки расходятся по километрам, никто ни с кем не встречается, и прогон
/// вырождается в шаг по путям — а он **линеен по времени**: сколько
/// виртуальных секунд подано, столько и пройдено, и подать их одним кадром или
/// тридцатью безразлично. На таком прогоне повтор совпадает даже у симуляции,
/// сломанной покадровой системой (проверено: перенос `move_moving_entities` в
/// `Update` не менял отпечаток). Расхождение родится только на **порогах** —
/// радиус паники, бросок демона, расталкивание, — а чтобы они срабатывали,
/// толпа и демоны должны находиться друг на друге.
///
/// Двор — единственная проходимая область карты, поэтому отбор проходимых
/// тайлов в `spawn_population` расселяет всё население именно в него, а
/// демоны выходят из портала прямо в толпу. Два домика внутри дают целям
/// блуждания вершины контуров на расстоянии, которое проходится за секунды.
pub struct Yard {
    pub map: MapData,
    /// Центр двора: и портал, и точка, от которой прунится достижимость.
    pub portal: Vec2,
}

pub fn crowded_yard() -> Yard {
    let center = MAP_SIZE / 2.0;
    let yard = rect(
        center - Vec2::splat(YARD_HALF),
        center + Vec2::splat(YARD_HALF),
    );

    let mut map = MapData::default();
    map.buildings
        .push(building(rect(Vec2::ZERO, MAP_SIZE), vec![yard]));
    for corner in [Vec2::new(-1.0, -1.0), Vec2::new(1.0, 1.0)] {
        let hut = center + corner * (YARD_HALF * 0.6);
        map.buildings.push(building(
            rect(hut - Vec2::splat(8.0), hut + Vec2::splat(8.0)),
            vec![],
        ));
    }

    Yard {
        map,
        portal: center,
    }
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
