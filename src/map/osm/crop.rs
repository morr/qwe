//! Срез выгрузки: окно вокруг точки, вырезанное из ответа Overpass в ответ
//! Overpass же — те же элементы с геометрией `out geom` и всеми тегами, только
//! меньше. Его ест [`super::parse::parse_response`] как есть.
//!
//! Читатель один — витрина дорог (`examples/demos/roads`): она режет окна
//! своих примеров из кеша города при запуске, а не хранит их файлами, так что
//! срез всегда равен игре, в том числе после подъёма `QUERY_VERSION`.
//!
//! Что резка делает с элементом каждого вида:
//!
//! - node — остаётся, если лежит в окне;
//! - way — остаётся **целиком**, если касается окна, и никогда не режется.
//!   Конвейер держится за собственные точки way: первая точка улицы сеет её
//!   ряд машин, контур стоянки решает её разметку, общая вершина — можно ли
//!   выпрямить дом. Обрезанный way — другой way, и пример перестаёт быть
//!   похож на игру;
//! - к оставленным дорогам добавляются дороги за окном, делящие с ними узел,
//!   на один уровень ([`joining_roads`]);
//! - мультиполигон — здание целиком; прочие (река, парк — они тянутся на
//!   километры) собираются в кольца, режутся окном как многоугольники и
//!   пишутся назад замкнутыми членами той же роли;
//! - relation без членов — вывод `is_in`, граница страны с `driving_side` —
//!   остаётся, без сотен своих `name:*`.
//!
//! Вся геометрия здесь — пары `(lon, lat)` в `f64` против гео-прямоугольника,
//! а не метры карты: исходные вершины обязаны остаться битово теми же, общие
//! узлы находятся равенством координат, и новыми бывают только концы разреза.

use bevy::platform::collections::HashMap;

use super::overpass::{Element, GeoBounds, LatLon, Member, OverpassResponse};

/// Теги границы, которые переживают резку: имя страны читает подпись
/// витрины, `driving_side` — разбор.
const KEPT_BOUNDARY_TAGS: [&str; 6] = [
    "name",
    "admin_level",
    "boundary",
    "driving_side",
    "ISO3166-1",
    "type",
];

/// Знаков после запятой у точки разреза — как у самого OSM (7 знаков,
/// сантиметр).
const CUT_SCALE: f64 = 1e7;

/// Окно в гео-координатах, границы включительно.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoRect {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl GeoRect {
    /// Квадрат `2 · half` метров вокруг `centre` — в проекции города, но без
    /// захода в `f32` метров карты.
    pub fn around(bounds: &GeoBounds, centre: LatLon, half: f64) -> Self {
        let lat = half / crate::settings::METERS_PER_DEG_LAT;
        let lon = half / bounds.meters_per_deg_lon();
        Self {
            west: centre.lon - lon,
            south: centre.lat - lat,
            east: centre.lon + lon,
            north: centre.lat + lat,
        }
    }

    fn contains(&self, p: Point) -> bool {
        self.west <= p.0 && p.0 <= self.east && self.south <= p.1 && p.1 <= self.north
    }

    fn overlaps(&self, other: &GeoRect) -> bool {
        self.west <= other.east
            && other.west <= self.east
            && self.south <= other.north
            && other.south <= self.north
    }

    fn holds(&self, other: &GeoRect) -> bool {
        self.west <= other.west
            && self.south <= other.south
            && other.east <= self.east
            && other.north <= self.north
    }
}

/// `(lon, lat)`: x — долгота, как у метров карты.
type Point = (f64, f64);

fn point(at: &LatLon) -> Point {
    (at.lon, at.lat)
}

fn points(geometry: &[LatLon]) -> Vec<Point> {
    geometry.iter().map(point).collect()
}

fn geometry(points: &[Point]) -> Vec<LatLon> {
    points
        .iter()
        .map(|&(lon, lat)| LatLon { lat, lon })
        .collect()
}

/// Ключ общего узла: координаты битами. Общий узел двух way в `out geom`
/// приходит одними и теми же числами, так что равенство — точное.
fn node_key(at: &LatLon) -> (u64, u64) {
    (at.lat.to_bits(), at.lon.to_bits())
}

/// Резчик одной выгрузки: индексы строятся один раз, окон режется сколько
/// угодно. Выгрузка Тулы — десятки тысяч элементов, окон у витрины полтора
/// десятка, и без индекса каждое окно обходило бы весь город заново.
pub struct Cropper<'a> {
    elements: &'a [Element],
    /// Рамка каждого элемента; `None` — у relation без геометрии (граница).
    frames: Vec<Option<GeoRect>>,
    /// Узел → дороги, которые через него проходят (для [`joining_roads`]).
    roads_at: HashMap<(u64, u64), Vec<usize>>,
}

impl<'a> Cropper<'a> {
    pub fn new(response: &'a OverpassResponse) -> Self {
        let elements = &response.elements[..];
        let frames = elements.iter().map(frame_of).collect();
        let mut roads_at: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
        for (index, element) in elements.iter().enumerate() {
            let Some(geometry) = road_geometry(element) else {
                continue;
            };
            for at in geometry {
                let roads = roads_at.entry(node_key(at)).or_default();
                if roads.last() != Some(&index) {
                    roads.push(index);
                }
            }
        }
        Self {
            elements,
            frames,
            roads_at,
        }
    }

    /// Окно выгрузки ответом Overpass: элементы в порядке выгрузки, за ними
    /// примыкающие дороги — тоже в порядке выгрузки.
    pub fn crop(&self, window: GeoRect) -> OverpassResponse {
        let mut kept = Vec::new();
        let mut kept_roads = Vec::new();
        for (index, element) in self.elements.iter().enumerate() {
            // рамка мимо окна — ни точки внутри, ни отрезка через него, ни
            // кольца вокруг: элемент не нужен, и дорогая проверка тоже
            if let Some(frame) = &self.frames[index]
                && !frame.overlaps(&window)
            {
                continue;
            }
            if let Some(piece) = crop_element(element, window) {
                if road_geometry(element).is_some() {
                    kept_roads.push(index);
                }
                kept.push(piece);
            }
        }
        kept.extend(
            joining_roads(self, &kept_roads)
                .into_iter()
                .map(|index| self.elements[index].clone()),
        );
        OverpassResponse { elements: kept }
    }
}

/// Геометрия way-дороги (`highway=*`), если элемент — она.
fn road_geometry(element: &Element) -> Option<&[LatLon]> {
    (element.kind == "way" && element.tags.contains_key("highway"))
        .then_some(element.geometry.as_deref())
        .flatten()
}

fn frame_of(element: &Element) -> Option<GeoRect> {
    match element.kind.as_str() {
        "node" => {
            let at = (element.lon?, element.lat?);
            Some(bounds_of([at].into_iter()))
        }
        "way" => {
            let geometry = element.geometry.as_deref().filter(|g| !g.is_empty())?;
            Some(bounds_of(geometry.iter().map(point)))
        }
        "relation" => {
            let all = element
                .members
                .as_deref()?
                .iter()
                .filter_map(|member| member.geometry.as_deref())
                .flatten()
                .map(point);
            let mut all = all.peekable();
            all.peek()?;
            Some(bounds_of(all))
        }
        _ => None,
    }
}

fn bounds_of(points: impl Iterator<Item = Point>) -> GeoRect {
    let mut rect = GeoRect {
        west: f64::INFINITY,
        south: f64::INFINITY,
        east: f64::NEG_INFINITY,
        north: f64::NEG_INFINITY,
    };
    for (x, y) in points {
        rect.west = rect.west.min(x);
        rect.south = rect.south.min(y);
        rect.east = rect.east.max(x);
        rect.north = rect.north.max(y);
    }
    rect
}

/// Что от элемента остаётся в окне; `None` — ничего.
fn crop_element(element: &Element, window: GeoRect) -> Option<Element> {
    match element.kind.as_str() {
        "node" => {
            let at = (element.lon?, element.lat?);
            window.contains(at).then(|| element.clone())
        }
        "way" => crop_way(element, window),
        "relation" => match &element.members {
            Some(members) => crop_relation(element, members, window),
            None => Some(Element {
                tags: element
                    .tags
                    .iter()
                    .filter(|(key, _)| KEPT_BOUNDARY_TAGS.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                ..element.clone()
            }),
        },
        _ => None,
    }
}

fn crop_way(element: &Element, window: GeoRect) -> Option<Element> {
    let line = points(element.geometry.as_deref()?);
    let closed = line.len() >= 4 && line.first() == line.last();
    // квартал или парк может держать всё окно внутри, не подходя к нему контуром
    let around = closed && !clip_ring(&line[..line.len() - 1], window).is_empty();
    (around || touches(&line, window)).then(|| element.clone())
}

fn crop_relation(element: &Element, members: &[Member], window: GeoRect) -> Option<Element> {
    let ways: Vec<(&Member, Vec<Point>)> = members
        .iter()
        .filter(|member| member.kind == "way")
        .filter_map(|member| Some((member, points(member.geometry.as_deref()?))))
        .collect();
    let whole = element.tags.contains_key("building")
        || ways
            .iter()
            .all(|(_, line)| window.holds(&bounds_of(line.iter().copied())));
    if whole {
        return ways
            .iter()
            .any(|(_, line)| touches(line, window))
            .then(|| element.clone());
    }
    // без проверки `touches`: вода может держать всё окно внутри контура, и на
    // этот случай отвечает `clip_ring` — от колец ничего не осталось, значит
    // и relation нет
    let mut cropped = Vec::new();
    for role in ["outer", "inner"] {
        let lines = ways
            .iter()
            .filter(|(member, _)| member.role == role)
            .map(|(_, line)| line.clone())
            .collect();
        for ring in assemble(lines) {
            let mut ring = clip_ring(&ring, window);
            if ring.is_empty() {
                continue;
            }
            ring.push(ring[0]);
            cropped.push(Member {
                kind: "way".into(),
                role: role.into(),
                geometry: Some(geometry(&ring)),
            });
        }
    }
    cropped
        .iter()
        .any(|member| member.role == "outer")
        .then(|| Element {
            members: Some(cropped),
            ..element.clone()
        })
}

/// Дороги за окном, делящие узел с дорогой в окне.
///
/// Оставленная улица цела, поэтому уходит за окно — а ряд машин проходится
/// вдоль неё всей, и его поток случайных чисел рвётся на каждом перекрёстке по
/// пути. Без улиц, которые она встречает там, этих перекрёстков нет, и ряд в
/// окне выходит с другими машинами, чем ставит игра. Одного уровня хватает:
/// нужны перекрёстки *оставленных* улиц, а не соседи соседей.
fn joining_roads(cropper: &Cropper, kept_roads: &[usize]) -> Vec<usize> {
    let mut joining: Vec<usize> = kept_roads
        .iter()
        .filter_map(|&index| cropper.elements[index].geometry.as_deref())
        .flatten()
        .filter_map(|at| cropper.roads_at.get(&node_key(at)))
        .flatten()
        .copied()
        .filter(|index| kept_roads.binary_search(index).is_err())
        .collect();
    joining.sort_unstable();
    joining.dedup();
    joining
}

fn touches(line: &[Point], window: GeoRect) -> bool {
    line.iter().any(|&at| window.contains(at))
        || line
            .windows(2)
            .any(|link| clip_segment(link[0], link[1], window).is_some_and(|(t0, t1)| t0 < t1))
}

/// Лян–Барски: доля отрезка `a–b` внутри окна как `(t0, t1)`, или `None`.
fn clip_segment(a: Point, b: Point, window: GeoRect) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0, 1.0);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    for (p, q) in [
        (-dx, a.0 - window.west),
        (dx, window.east - a.0),
        (-dy, a.1 - window.south),
        (dy, window.north - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = f64::max(t0, t);
        } else {
            t1 = f64::min(t1, t);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

fn cut(a: Point, b: Point, t: f64) -> Point {
    let round = |value: f64| (value * CUT_SCALE).round() / CUT_SCALE;
    (round(a.0 + (b.0 - a.0) * t), round(a.1 + (b.1 - a.1) * t))
}

/// Сазерленд–Ходжмен по четырём сторонам окна. Кольцо открытое (без
/// повторённой точки); меньше трёх точек в ответе — пустое.
fn clip_ring(ring: &[Point], window: GeoRect) -> Vec<Point> {
    type Side = (fn(Point, &GeoRect) -> bool, bool, fn(&GeoRect) -> f64);
    let sides: [Side; 4] = [
        (|p, w| p.0 >= w.west, true, |w| w.west),
        (|p, w| p.0 <= w.east, true, |w| w.east),
        (|p, w| p.1 >= w.south, false, |w| w.south),
        (|p, w| p.1 <= w.north, false, |w| w.north),
    ];
    let mut ring = ring.to_vec();
    for (keep, along_x, edge) in sides {
        if ring.is_empty() {
            break;
        }
        let value = edge(&window);
        let mut out = Vec::with_capacity(ring.len() + 2);
        for (i, &a) in ring.iter().enumerate() {
            let b = ring[(i + 1) % ring.len()];
            let (keep_a, keep_b) = (keep(a, &window), keep(b, &window));
            if keep_a != keep_b {
                let t = if along_x {
                    (value - a.0) / (b.0 - a.0)
                } else {
                    (value - a.1) / (b.1 - a.1)
                };
                let at = cut(a, b, t);
                let at = if along_x {
                    (value, at.1)
                } else {
                    (at.0, value)
                };
                if keep_a {
                    out.extend([a, at]);
                } else {
                    out.push(at);
                }
            } else if keep_a {
                out.push(a);
            }
        }
        // подряд идущие повторы, и последний с первым тоже
        ring = (0..out.len())
            .filter(|&i| out[i] != out[(i + out.len() - 1) % out.len()])
            .map(|i| out[i])
            .collect();
    }
    if ring.len() >= 3 { ring } else { Vec::new() }
}

/// Члены relation — концами друг к другу в кольца. Незамкнутую цепь (её
/// порвал край выгрузки города) замыкает сама отдача: как и разбор, кольцо
/// берётся как есть.
fn assemble(lines: Vec<Vec<Point>>) -> Vec<Vec<Point>> {
    let mut lines: Vec<Vec<Point>> = lines.into_iter().filter(|line| line.len() >= 2).collect();
    let mut rings = Vec::new();
    while let Some(mut ring) = lines.pop() {
        while ring.first() != ring.last() {
            let tail = *ring.last().unwrap();
            let Some(index) = lines
                .iter()
                .position(|line| line[0] == tail || *line.last().unwrap() == tail)
            else {
                break;
            };
            let mut line = lines.remove(index);
            if line[0] != tail {
                line.reverse();
            }
            ring.extend_from_slice(&line[1..]);
        }
        if ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings
}

/// Срез файлом: по элементу на строку — файл читают глазами и сравнивают
/// диффом. Читается назад как обычный ответ Overpass.
pub fn to_json(response: &OverpassResponse) -> String {
    let lines: Vec<String> = response
        .elements
        .iter()
        .map(|element| format!("    {}", serde_json::to_string(element).unwrap_or_default()))
        .collect();
    format!("{{\n  \"elements\": [\n{}\n  ]\n}}\n", lines.join(",\n"))
}

#[cfg(test)]
mod tests {
    // теги у `Element` — std: их заполняет serde
    use std::collections::HashMap as StdHashMap;

    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> StdHashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn at(lon: f64, lat: f64) -> LatLon {
        LatLon { lat, lon }
    }

    fn way(id: u64, pairs: &[(&str, &str)], line: &[(f64, f64)]) -> Element {
        Element {
            kind: "way".into(),
            id,
            tags: tags(pairs),
            lat: None,
            lon: None,
            geometry: Some(line.iter().map(|&(x, y)| at(x, y)).collect()),
            members: None,
        }
    }

    fn node(id: u64, pairs: &[(&str, &str)], (x, y): (f64, f64)) -> Element {
        Element {
            kind: "node".into(),
            id,
            tags: tags(pairs),
            lat: Some(y),
            lon: Some(x),
            geometry: None,
            members: None,
        }
    }

    /// Окно `[0, 1] × [0, 1]` — геометрия тестов в «градусах» для простоты.
    const WINDOW: GeoRect = GeoRect {
        west: 0.0,
        south: 0.0,
        east: 1.0,
        north: 1.0,
    };

    fn ids(response: &OverpassResponse) -> Vec<u64> {
        response.elements.iter().map(|element| element.id).collect()
    }

    fn crop(elements: Vec<Element>) -> OverpassResponse {
        let response = OverpassResponse { elements };
        Cropper::new(&response).crop(WINDOW)
    }

    #[test]
    fn a_way_through_the_window_stays_whole() {
        let street = way(
            1,
            &[("highway", "primary")],
            &[(-5.0, 0.5), (0.5, 0.5), (5.0, 0.5)],
        );
        let cropped = crop(vec![street.clone()]);
        assert_eq!(ids(&cropped), [1]);
        assert_eq!(
            cropped.elements[0].geometry.as_ref().unwrap().len(),
            3,
            "the way is never clipped"
        );
    }

    #[test]
    fn a_way_crossing_without_a_vertex_inside_is_kept() {
        let crossing = way(1, &[("highway", "residential")], &[(-1.0, 0.5), (2.0, 0.5)]);
        let far = way(2, &[("highway", "residential")], &[(3.0, 3.0), (4.0, 4.0)]);
        assert_eq!(ids(&crop(vec![crossing, far])), [1]);
    }

    #[test]
    fn a_closed_block_around_the_window_is_kept() {
        let block = way(
            1,
            &[("landuse", "residential")],
            &[
                (-3.0, -3.0),
                (4.0, -3.0),
                (4.0, 4.0),
                (-3.0, 4.0),
                (-3.0, -3.0),
            ],
        );
        assert_eq!(ids(&crop(vec![block])), [1]);
    }

    #[test]
    fn nodes_are_kept_only_inside() {
        let inside = node(1, &[("highway", "crossing")], (0.5, 0.5));
        let outside = node(2, &[("highway", "crossing")], (1.5, 0.5));
        assert_eq!(ids(&crop(vec![inside, outside])), [1]);
    }

    /// Улица в окне уходит за его край; улица, которую она встречает там,
    /// приходит тоже, а её соседка — уже нет.
    #[test]
    fn roads_meeting_a_kept_road_come_one_level_deep() {
        let kept = way(1, &[("highway", "primary")], &[(0.5, 0.5), (3.0, 0.5)]);
        let joining = way(2, &[("highway", "service")], &[(3.0, 0.5), (3.0, 3.0)]);
        let beyond = way(3, &[("highway", "service")], &[(3.0, 3.0), (6.0, 3.0)]);
        let footway_elsewhere = way(4, &[("highway", "footway")], &[(8.0, 8.0), (9.0, 9.0)]);
        assert_eq!(
            ids(&crop(vec![joining, beyond, kept, footway_elsewhere])),
            [1, 2]
        );
    }

    #[test]
    fn a_river_polygon_is_clipped_to_the_window() {
        let river = Element {
            kind: "relation".into(),
            id: 7,
            tags: tags(&[("natural", "water"), ("type", "multipolygon")]),
            lat: None,
            lon: None,
            geometry: None,
            members: Some(vec![Member {
                kind: "way".into(),
                role: "outer".into(),
                geometry: Some(
                    [
                        (-5.0, 0.25),
                        (5.0, 0.25),
                        (5.0, 0.75),
                        (-5.0, 0.75),
                        (-5.0, 0.25),
                    ]
                    .iter()
                    .map(|&(x, y)| at(x, y))
                    .collect(),
                ),
            }]),
        };
        let cropped = crop(vec![river]);
        let members = cropped.elements[0].members.as_ref().unwrap();
        assert_eq!(members.len(), 1);
        let ring = members[0].geometry.as_ref().unwrap();
        assert!(
            ring.iter().all(|p| WINDOW.contains(point(p))),
            "every vertex inside"
        );
        assert_eq!(ring.first(), ring.last(), "written back closed");
    }

    #[test]
    fn a_boundary_keeps_only_its_driving_side_and_name() {
        let boundary = Element {
            kind: "relation".into(),
            id: 60189,
            tags: tags(&[
                ("name", "Россия"),
                ("name:en", "Russia"),
                ("admin_level", "2"),
                ("driving_side", "right"),
            ]),
            lat: None,
            lon: None,
            geometry: None,
            members: None,
        };
        let cropped = crop(vec![boundary]);
        let kept = &cropped.elements[0].tags;
        assert_eq!(kept.len(), 3);
        assert!(!kept.contains_key("name:en"));
    }

    /// Срез, выгруженный файлом, читается назад как ответ Overpass.
    #[test]
    fn a_dumped_crop_reads_back() {
        let street = way(1, &[("highway", "primary")], &[(0.5, 0.5), (3.0, 0.5)]);
        let json = to_json(&crop(vec![street]));
        let back: OverpassResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(ids(&back), [1]);
    }
}
