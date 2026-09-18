//! Слой дорог, аллей и стен Кремля: по ленте на `RoadLine`/`WallLine`, слитой
//! в merged-меш на класс. Стиль ленты — ресурс [`RoadStyle`], переключаемый на
//! лету панелью Roads (`ui/roads.rs`); правка пересобирает только эти слои
//! ([`rebuild_roads`]). Рельсовые пути — в `map/rail.rs`, трамвай — в
//! `map/tram.rs`: у обоих свой стиль и свой зум-LOD, и пересобираются они по
//! зуму, а не по [`RoadStyle`].
//!
//! Мост (`RoadLine::bridge`) уходит из слоёв своего класса в тройку
//! `bridge_shadows` + `bridge_casings` + `bridges`: серый бордюр по краям
//! настила (всегда, вне зависимости от `RoadStyle::casing`) и заливка цветом
//! класса над `Z_ROAD` — эстакада кроет улицу, которую пересекает, а ровные
//! торцы бордюра читаются как края настила, вид 2ГИС. Под ними —
//! [`push_bridge_shadows`]: единственное на карте, что говорит, что настил
//! поднят, потому что наземные тени считают только дома.
//!
//! Раньше дороги рисовал `MeshBuilder::push_polyline` — свой квад на
//! сегмент, продлённый с обоих концов на полуширины. Стыков у него нет вообще:
//! на изломе продление торчит за внешний угол прямоугольным выступом, между
//! двумя выступами остаётся выемка, а торец пути — квадратный шип. Дефолт
//! теперь [`RoadJoin::Round`] — дуга на внешней стороне излома и полудиск на
//! торце, то же самое, что `stroke-linejoin: round` + `stroke-linecap: round`
//! у Mapnik, которым нарисован osm-carto: круглые торцы двух ways в общем узле
//! перекрываются и сливаются в скруглённый стык.
//!
//! Осевая (`RoadLine::points`) при этом **не трогается**: на ней стоят навмеш
//! (`bridge`/`passage`-прорезы), арки, посадка деревьев и генератор дверей.
//! Chaikin-сглаживание работает на копии и только ради картинки.
//!
//! Улица — это не одна лента, а три слоя: **тротуар** (`Z_SIDEWALK`, светлая
//! полоса шире проезжей части на [`sidewalk_width`] с каждой стороны), кант и
//! заливка асфальтом. Тротуар лежит под всеми лентами дорог по той же логике,
//! что кант: заливка поперечной улицы кроет его на перекрёстке, и тротуар
//! обрывается там, где обрывается в жизни. **Разметку** — линии по границам
//! полос ([`lane_count`]: тег `lanes`, иначе дефолт по ширине) — рисует не
//! геометрия, а шейдер поверхностей (`map/surface.rs`) по локальным
//! координатам ленты (`meshing::ATTRIBUTE_RIBBON`): линия сглажена, гаснет при
//! отдалении и **рвётся на перекрёстках** — по общим узлам ways
//! (`roads/junctions.rs`), а не по торцам, так что way, разрезанный посреди
//! квартала, несёт линию сквозь стык, а сквозная улица теряет её ровно на
//! ширину поперечной. Широкие улицы кладутся поверх узких: заливка
//! магистрали кроет торец жилой улицы, и в перекрёстке остаётся разметка
//! магистрали с разрывом под въезд, а не обрубок линии въезда поверх неё.

use std::borrow::Cow;
use std::f32::consts::PI;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use self::network::RoadNodes;
use crate::map::footprint::{JOIN_EPSILON, casing_width};
use crate::map::meshing::{
    Break, Markings, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin, merge_close_points,
    miter_offsets,
};
use crate::map::osm::model::{
    distance_to_segment, point_in_area, point_in_polygon, polyline_length, ring_bounds,
};
use crate::map::osm::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
use crate::map::surface::{
    self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, SurfaceKind, spawn_layers,
};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};
use crate::settings::{
    Z_ALLEY, Z_ALLEY_CASING, Z_BRIDGE, Z_BRIDGE_CASING, Z_BRIDGE_SHADOW, Z_BUILDING, Z_ROAD,
    Z_ROAD_CASING, Z_SIDEWALK,
};

/// Путь тени настила: та же осевая, сдвинутая по свету на высоту моста,
/// **сходящую к нулю у свободных торцов моста** ([`BridgeSpan`]).
///
/// Без схождения тень вылезала на дорогу в месте примыкания: у береговой
/// опоры настил лежит на земле, и тени там нет вовсе, а сдвинутая на полную
/// высоту лента выезжала за торец моста и ложилась тёмной полосой поперёк
/// подходящей улицы. Подъём считается по длине дуги: `RAMP_SHARE` длины с
/// каждого конца (но не больше `RAMP_MAX`) — это и есть насыпь.
///
/// **Расстояние до торца меряется по всему мосту, а не по этому way.** Мост в
/// OSM нарезан: развязка Оружейного моста — три way (424 + 95 + 299 м),
/// сходящиеся в одном узле, и в этом узле настил идёт на полной высоте. Меряя
/// от торцов куска, рампа отрабатывала в узле у каждого из трёх кусков, и тень
/// проваливалась под настил посреди развязки. `from_start`/`from_end` — это путь до
/// ближайшего свободного торца **через соседние ways**, поэтому у внутреннего
/// стыка он велик и подъём там равен единице.
///
/// Осевая для этого **догущается** ([`densify`]): подъём живёт в вершинах, а
/// прямой мост в OSM — это ровно две точки, и обе они торцы. Без догущения
/// `rise` в обеих ноль, тень ложится точь-в-точь под настил и пропадает
/// целиком — так мостики через пруд и не отбрасывали тени вовсе, тогда как
/// изломанный путепровод (вершин много) давал рваную полосу: подъём прыгал от
/// вершины к вершине.
///
/// Подъём вдобавок **зажат остатком пролёта**: если тень едет вдоль моста, то
/// на расстоянии `at` от торца ей позволено уехать не дальше чем на `at`.
/// Гладкая насыпь поднимается быстрее, чем набирается длина, и у короткого
/// моста тень успевала перевалить за торец и лечь тёмным клином на дорогу —
/// тот самый клин, что торчал из-под каждого мостика через канал. Остаток
/// тоже считается по всему мосту.
fn bridge_shadow_path(points: &[Vec2], deck: &BridgeSpan) -> Vec<ShadowPoint> {
    let dense = deck_centerline(points);
    if dense.len() < 2 {
        return Vec::new();
    }
    let mut along = Vec::with_capacity(dense.len());
    let mut travelled = 0.0;
    for (index, point) in dense.iter().enumerate() {
        if index > 0 {
            travelled += point.distance(dense[index - 1]);
        }
        along.push(travelled);
    }
    let length = travelled;
    let offset = shadow_dir() * (bridge_height(deck.span) * shadow_length_scale());
    let ramp = (deck.span * RAMP_SHARE).clamp(f32::EPSILON, RAMP_MAX);
    let last = dense.len() - 1;
    (0..dense.len())
        .map(|index| {
            let (point, at) = (dense[index], along[index]);
            // до свободного торца моста в обе стороны: по этому куску плюс то,
            // что за его стыком
            let behind = deck.from_start + at;
            let ahead = deck.from_end + (length - at);
            let raised = (behind.min(ahead) / ramp).clamp(0.0, 1.0);
            // плавно, а не изломом: у настоящей насыпи профиль сглажен
            let rise = raised * raised * (3.0 - 2.0 * raised);
            // ход тени вдоль моста и сколько его осталось до торца впереди
            let tangent = (dense[(index + 1).min(last)] - dense[index.saturating_sub(1)])
                .normalize_or(Vec2::X);
            let travel = offset.dot(tangent);
            let room = if travel > 0.0 { ahead } else { behind };
            let rise = if travel.abs() > f32::EPSILON {
                rise.min(room / travel.abs())
            } else {
                rise
            };
            ShadowPoint {
                at: point + offset * rise,
                rise,
            }
        })
        .collect()
}

/// Высота настила над тем, что под ним, м: [`SPAN_TO_HEIGHT`] пролёта, но не
/// выше [`BRIDGE_HEIGHT`].
///
/// Высота росла с пролётом всегда — просто раньше об этом не спрашивали, и
/// всякий way с `bridge=yes` поднимался на шесть метров. В OSM этот тег носят
/// не только пролёты: им же размечены сходы с набережной, тротуар вдоль
/// путепровода, четырёхметровый переход через ливнёвку. Шестиметровая тень от
/// двадцатиметровой дорожки, под которой на месте ничего нет, — самое заметное
/// враньё, какое карта может себе позволить, потому что тень читается как
/// высота.
fn bridge_height(span: f32) -> f32 {
    (span * SPAN_TO_HEIGHT).min(BRIDGE_HEIGHT)
}

/// Место одного мостового way в своём мосту.
///
/// `span` — длина **всего** моста, `from_start`/`from_end` — кратчайший путь
/// от торцов этого куска до ближайшего свободного торца моста
/// (бесконечность, если свободного торца нет вовсе — кольцевая эстакада
/// нигде не садится на землю, и подъём у неё везде полный). `casts` —
/// решение целого моста, а не куска.
///
/// Кратчайший путь от дальнего узла может вести **назад по этому же куску** —
/// у первого way цепочки 60 + 30 + 60 он и ведёт, давая 60, а не 90. Двойного
/// счёта из этого не выходит: расстояние до земли берётся как минимум из двух
/// сторон, и тот же самый маршрут уже учтён со стороны `from_start`.
#[derive(Clone, Copy, Debug)]
struct BridgeSpan {
    span: f32,
    from_start: f32,
    from_end: f32,
    casts: bool,
}

/// Мосты карты: **связные цепочки** мостовых ways, а не отдельные ways.
///
/// Мост в OSM нарезан — Тула: 61 мостовой way, из них 8 сцеплены в 3 моста
/// (424 + 95 + 299 = 818 м развязка Оружейного моста, 34 + 129 + 22 = 185 м и
/// 39 + 4 = 43 м), итого 56 мостов. Считая каждый way отдельным мостом, тень
/// врала дважды: рампа отрабатывала на каждом внутреннем стыке (тень
/// проваливалась под настил посреди длинного моста), а [`SHORT_SPAN`]
/// применялся к куску — 22-метровая середина 185-метрового моста проверялась
/// как отдельный мостик и могла остаться без тени вовсе.
///
/// **Склейка — по торцам, а не [`crate::map::footprint::ways_joined`].** Тот
/// предикат отвечает на другой вопрос — «есть ли у этих ломаных общая точка
/// вообще», — и им же меряется примыкание дороги к мосту для бордюра. Здесь
/// он ошибается в обе стороны: на Туле он склеил бы две пары пешеходных
/// мостиков, которые всего лишь пересекаются, а T-образное примыкание торца к
/// середине чужого моста (на Туле таких нет, но данные их не запрещают)
/// превратило бы ветку в продолжение. Сходятся именно **торцы** — с тем же
/// допуском [`JOIN_EPSILON`], потому что это цена проекции.
///
/// **Геометрия при этом не склеивается.** Куски одного моста бывают разной
/// ширины, и одной ломаной их не описать; а главное — в узле сходятся и три
/// конца сразу (на Туле ровно один такой: 424 + 95 + 299 сходятся в одной
/// точке, это съезд развязки, а не цепочка). Поэтому склеивается не путь, а
/// **счёт**: длина моста и расстояние до свободного торца. Развилке это ничего
/// не стоит — у трёхконцевого узла путь до свободного торца просто идёт по
/// самой короткой из трёх веток, и настил на развилке остаётся поднятым, как
/// ему и положено.
struct Bridges {
    /// На индекс дороги; `None` — не мост.
    spans: Vec<Option<BridgeSpan>>,
}

impl Bridges {
    fn new(map: &MapData) -> Self {
        let mut spans = vec![None; map.roads.len()];
        let decks: Vec<usize> = map
            .roads
            .iter()
            .enumerate()
            .filter(|(_, road)| road.bridge && road.points.len() >= 2)
            .map(|(index, _)| index)
            .collect();
        if decks.is_empty() {
            return Self { spans };
        }
        // узлы — склеенные торцы; их вдвое больше ways, перебор квадратичен и
        // на шести десятках мостов не стоит ничего
        let mut nodes: Vec<Vec2> = Vec::new();
        let node_at = |nodes: &mut Vec<Vec2>, point: Vec2| {
            if let Some(found) = nodes
                .iter()
                .position(|known| known.distance(point) < JOIN_EPSILON)
            {
                return found;
            }
            nodes.push(point);
            nodes.len() - 1
        };
        let mut ends: Vec<[usize; 2]> = Vec::with_capacity(decks.len());
        let mut lengths: Vec<f32> = Vec::with_capacity(decks.len());
        for &index in &decks {
            let points = &map.roads[index].points;
            let (first, last) = (points[0], points[points.len() - 1]);
            ends.push([node_at(&mut nodes, first), node_at(&mut nodes, last)]);
            lengths.push(polyline_length(points));
        }

        // компоненты связности по узлам — это и есть мосты
        let mut parent: Vec<usize> = (0..nodes.len()).collect();
        for [first, second] in &ends {
            let (a, b) = (
                component_root(&mut parent, *first),
                component_root(&mut parent, *second),
            );
            if a != b {
                parent[a] = b;
            }
        }
        let roots: Vec<usize> = (0..nodes.len())
            .map(|node| component_root(&mut parent, node))
            .collect();

        let mut degree = vec![0_usize; nodes.len()];
        let mut span = vec![0.0_f32; nodes.len()];
        for (deck, [first, second]) in ends.iter().enumerate() {
            degree[*first] += 1;
            degree[*second] += 1;
            span[roots[*first]] += lengths[deck];
        }

        // Путь до ближайшего свободного торца. Граф крошечный (десятки рёбер),
        // поэтому расслабление до сходимости, а не очередь с приоритетом.
        let mut to_free: Vec<f32> = degree
            .iter()
            .map(|&count| if count == 1 { 0.0 } else { f32::INFINITY })
            .collect();
        for _ in 0..=decks.len() {
            let mut moved = false;
            for (deck, [first, second]) in ends.iter().enumerate() {
                for (from, to) in [(*first, *second), (*second, *first)] {
                    let through = to_free[from] + lengths[deck];
                    if through < to_free[to] {
                        to_free[to] = through;
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }

        // Есть ли под мостом разрыв — вопрос всему мосту сразу: кусок над
        // сушей рядом с куском над водой обязан получить ту же тень.
        let underneath = Underneath::new(map);
        let mut casts: Vec<bool> = span.iter().map(|&length| length >= SHORT_SPAN).collect();
        for (deck, &index) in decks.iter().enumerate() {
            let root = roots[ends[deck][0]];
            if casts[root] {
                continue;
            }
            casts[root] = probe_underneath(&map.roads[index].points, &underneath);
        }

        for (deck, &index) in decks.iter().enumerate() {
            let [first, second] = ends[deck];
            let root = roots[first];
            spans[index] = Some(BridgeSpan {
                span: span[root],
                from_start: to_free[first],
                from_end: to_free[second],
                casts: casts[root],
            });
        }
        Self { spans }
    }

    fn span(&self, road: usize) -> Option<&BridgeSpan> {
        self.spans[road].as_ref()
    }
}

/// Корень компоненты со сжатием пути.
fn component_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Есть ли под этим настилом разрыв — проба по точке через каждые
/// [`SHADOW_STEP`] метров.
///
/// Спрашивают только у короткого моста. Пропорциональной высоты мало:
/// западный подход к мосту через Упу на Советской — это четыре way по 23–30 м с
/// `bridge=yes` и `layer=1`, а на месте там ровная земля: насыпь, а не
/// эстакада. Отличить насыпь от пролёта по тегам нельзя, зато можно спросить,
/// есть ли под ней разрыв. Дороги в этот список не входят намеренно — именно
/// вдоль дорог и лежат подходы, — а вода и путь под коротким настилом
/// сомнений не оставляют.
///
/// Длинному мосту вопрос не задаётся: на сотне метров насыпи не бывает, а
/// перебирать контуры воды под каждым из них незачем.
fn probe_underneath(points: &[Vec2], underneath: &Underneath) -> bool {
    deck_centerline(points)
        .iter()
        .any(|point| underneath.covers(*point))
}

/// Осевая настила, по которой идут и тень, и проба: слипшиеся точки OSM
/// склеены (они вырождают нормаль стыка — то же, что делает лента), остальное
/// догущено до [`SHADOW_STEP`].
fn deck_centerline(points: &[Vec2]) -> Vec<Vec2> {
    densify(
        &merge_close_points(points, false, SHADOW_STEP / 4.0),
        SHADOW_STEP,
    )
}

/// Что лежит под настилом: контуры воды, русла водотоков и рельсовые пути,
/// каждый со своим габаритом.
///
/// Габарит считается один раз на сборку слоёв, и это не оптимизация впрок:
/// проба идёт по точке через каждые два метра короткого моста, а контуров воды
/// в городе бывает под тысячу.
struct Underneath<'a> {
    /// Контуры воды — пруд, река, затон.
    areas: Vec<(Rect, &'a PolyArea)>,
    /// Русла водотоков и пути: осевая и полуширина.
    lines: Vec<(Rect, &'a [Vec2], f32)>,
}

impl<'a> Underneath<'a> {
    fn new(map: &'a MapData) -> Self {
        let bounds = |points: &[Vec2], margin: f32| {
            let (min, max) = points.iter().fold(
                (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                |(min, max), point| (min.min(*point), max.max(*point)),
            );
            Rect::from_corners(min - margin, max + margin)
        };
        let channels = map
            .water_lines
            .iter()
            // труба не разрыв: вода идёт под землёй, поверху проходят пешком
            .filter(|line| !line.tunnel)
            .map(|line| (line.points.as_slice(), line.width));
        let rails = map
            .rails
            .iter()
            .map(|rail| (rail.points.as_slice(), rail.width));
        Self {
            areas: map
                .water
                .iter()
                .map(|area| (bounds(&area.outer, 0.0), area))
                .collect(),
            lines: channels
                .chain(rails)
                .map(|(points, width)| (bounds(points, width / 2.0), points, width / 2.0))
                .collect(),
        }
    }

    fn covers(&self, point: Vec2) -> bool {
        self.areas
            .iter()
            .any(|(bounds, area)| bounds.contains(point) && point_in_area(point, area))
            || self.lines.iter().any(|(bounds, path, half)| {
                bounds.contains(point)
                    && path
                        .windows(2)
                        .any(|span| distance_to_segment(point, span[0], span[1]) <= *half)
            })
    }
}

/// Точка теневой ленты: куда съехал настил и насколько он в этом месте поднят
/// (0 у береговой опоры, 1 на полной высоте). Подъём нужен и после сдвига —
/// им же сходит на нет и уширение, и полутень ([`push_bridge_shadows`]).
struct ShadowPoint {
    at: Vec2,
    rise: f32,
}

/// Ломаная, догущённая до шага не крупнее `step`: исходные вершины остаются на
/// месте, между ними встают промежуточные. Нужна там, где вдоль ленты меняется
/// не только направление, но и величина — здесь высота настила.
fn densify(points: &[Vec2], step: f32) -> Vec<Vec2> {
    let Some((last, rest)) = points.split_last() else {
        return Vec::new();
    };
    let mut dense = Vec::with_capacity(rest.len() + 1);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        dense.push(from);
        let parts = (from.distance(to) / step).ceil().max(1.0);
        for part in 1..parts as usize {
            dense.push(from.lerp(to, part as f32 / parts));
        }
    }
    dense.push(*last);
    dense
}

/// Доля длины моста, на которой настил поднимается от земли до полной высоты,
/// потолок этой длины в метрах и шаг, которым осевая догущается под подъём.
const RAMP_SHARE: f32 = 0.25;
const RAMP_MAX: f32 = 25.0;
const SHADOW_STEP: f32 = 2.0;

/// Пролёт, короче которого мост ([`Bridges`] — связные мостовые ways, а не
/// один way) считается мостом только над водой или путями — см.
/// [`probe_underneath`]. Тридцать пять метров: подходы к мосту через Упу на
/// Советской это 23–30 м, мостик через канал в парке — 39.
const SHORT_SPAN: f32 = 35.0;

/// Насколько тень настила шире самого настила с каждой стороны, м. Метр — это
/// тень перил, толщина плиты и полоса воды, которой настил закрыл небо; от
/// высоты моста, в отличие от сдвига, эта кайма почти не зависит (перила у
/// пешеходного мостика те же, что у моста через Упу), но вместе с настилом
/// садится на землю у торцов. См. [`push_bridge_shadows`].
const SHADOW_SPREAD: f32 = 1.0;

/// Потолок высоты настила над тем, что под ним, м, и высота на метр пролёта.
/// Длину тени высота даёт тем же котангенсом высоты солнца, что у домов и
/// вагонов: путепровод над улицей поднят метров на шесть, и тень от него —
/// самое заметное, что бывает на воде под мостом. Полную высоту набирает
/// пролёт от 48 м — мост через Упу на Советской (137 м) и Оружейный мост
/// (571 м) на потолке,
/// мостик через пруд (14 м) поднят на метр восемьдесят.
const BRIDGE_HEIGHT: f32 = 6.0;
const SPAN_TO_HEIGHT: f32 = 1.0 / 8.0;

/// Проезжая часть — асфальт: серый, заметно темнее тротуара и земли. Белой
/// (osm-carto) она была, пока не появилась разметка: белую линию на белом не
/// видно. Потом была светло-голубовато-серой (0.655, как на детальных картах
/// 2ГИС), и это картографический тон, а не снимок: у выветренного асфальта на
/// аэрофото нейтральный серый около середины шкалы, и ступень до светлого
/// бетонного тротуара там заметно больше. Тот же тон у стоянок
/// (`spawn::PARKING_COLOR`) — они лежат поверх улиц одним полотном с ними.
/// Открыт наружу витрине машин: ряд обязан стоять на том же асфальте, что в
/// городе, — на своём сером ступень яркости между кузовом и покрытием была бы
/// не та.
pub const ROAD_COLOR: Color = Color::srgb(0.545, 0.545, 0.55);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Тротуар — светлый бетон между асфальтом и тёплой землёй: светлее проезжей
/// части на четверть, и именно эта ступень яркости читается как бордюр.
const SIDEWALK_COLOR: Color = Color::srgb(0.82, 0.815, 0.80);
/// Доля ширины улицы на тротуар с каждой стороны и её пределы, м: у
/// магистрали в 16 м тротуар в 3 м, у жилой улицы в 8 м — 1.8 м.
const SIDEWALK_SHARE: f32 = 0.22;
const SIDEWALK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 1.2..=3.0;
/// Улицы у́же этого — проезды (`service`, 5 м): ни тротуара, ни разметки, ни
/// разрыва в разметке улицы, к которой проезд примыкает.
const STREET_MIN_WIDTH: f32 = 8.0;

/// Полоса не у́же этого, м: `lanes=6` на десятиметровой ленте — данные о
/// настоящей улице, а лента у нас по классу, и лишние полосы отбрасываются.
const MIN_LANE_WIDTH: f32 = 2.5;
/// Полос по умолчанию, когда тега `lanes` нет: двусторонней улице — по паре
/// на каждые 7 м ширины (8 и 10 м — две полосы, 12 и 16 — четыре),
/// односторонней — по полосе на 4.5 м (8 м — одна, без линий; 16 — три).
const TWOWAY_METERS_PER_LANE_PAIR: f32 = 7.0;
const ONEWAY_METERS_PER_LANE: f32 = 4.5;

/// Кант дороги — затемнённая заливка, как у osm-carto (улица в тёмном канте);
/// темнее асфальта. Отдельным слоем под заливкой: заливки всех дорог кроют
/// канты всех дорог, поэтому кант никогда не режет перекрёсток пополам.
const ROAD_CASING_COLOR: Color = Color::srgb(0.40, 0.40, 0.41);
const ALLEY_CASING_COLOR: Color = Color::srgb(0.729, 0.678, 0.549);

/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

/// Бордюр моста — светлый бетонный парапет над серым настилом, общий для
/// улиц и пешеходных мостиков. Толщины (и почему их диапазоны не
/// пересекаются) — в `map::footprint`.
const BRIDGE_CURB_COLOR: Color = Color::srgb(0.80, 0.80, 0.79);

/// Изломы мельче Chaikin не срезает: прямые участки обязаны остаться точками
/// OSM, иначе сглаживание съедает и без того редкую геометрию длинных улиц.
const MIN_SMOOTH_ANGLE: f32 = 10.0 * PI / 180.0;
/// Доля сегмента, отрезаемая с каждой стороны излома (классический Chaikin).
const CHAIKIN_CUT: f32 = 0.25;

/// Чем закрыт излом ленты дороги.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadJoin {
    /// Статус-кво до сглаживания: свой квад на сегмент, оба конца продлены на
    /// полуширины. Оставлен, чтобы можно было сравнить с прежней картинкой.
    Square,
    /// Сведение по биссектрисе с ограничением длины стыка.
    Miter,
    /// Дуга на внешней стороне излома + полудиск на торце — вид osm-carto.
    #[default]
    Round,
}

impl RoadJoin {
    pub const ALL: [Self; 3] = [Self::Square, Self::Miter, Self::Round];

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Miter => "Miter",
            Self::Round => "Round",
        }
    }

    /// Излом и торец ленты `MeshBuilder`. `None` — `Square`: ленты у него нет
    /// вовсе, это `push_polyline` с продлёнными торцами. Одна таблица на всех,
    /// кто кладёт ленту дороги ([`push_ribbon`], [`push_street_fill`]) —
    /// разойдясь, они дали бы двум слоям одной улицы разные торцы.
    fn ribbon_shape(self) -> Option<(RibbonJoin, RibbonCap)> {
        match self {
            Self::Square => None,
            Self::Miter => Some((RibbonJoin::Miter, RibbonCap::Butt)),
            Self::Round => Some((RibbonJoin::Round, RibbonCap::Round)),
        }
    }
}

/// Сколько раз осевая прогоняется через Chaikin перед построением ленты.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoadSmoothing {
    /// Осевая ровно по данным OSM — как в самом OSM, где углы остаются острыми.
    Off,
    /// Один проход: улица на повороте перестаёт ломаться под углом, а рисунок
    /// сети ещё держится там, где OSM ставил узлы.
    #[default]
    Light,
    Strong,
}

impl RoadSmoothing {
    pub const ALL: [Self; 3] = [Self::Off, Self::Light, Self::Strong];

    fn iterations(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Light => 1,
            Self::Strong => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Light => "Light",
            Self::Strong => "Strong",
        }
    }
}

/// Стиль дорожных лент; переключается панелью Roads и BRP, сохраняется в
/// настройках между запусками. Правка пересобирает дорожные слои
/// ([`rebuild_roads`]).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roads")]
pub struct RoadStyle {
    pub join: RoadJoin,
    pub smoothing: RoadSmoothing,
    /// Тёмный кант по краю дороги отдельным слоем под заливкой.
    pub casing: bool,
    /// Серая полоса тротуара вдоль улиц (не проездов) отдельным слоем под
    /// всеми лентами.
    pub sidewalks: bool,
    /// Разметка полос на проезжей части улиц — линия на каждой границе полос,
    /// с разрывами на перекрёстках; рисует шейдер поверхностей.
    pub markings: bool,
}

impl Default for RoadStyle {
    fn default() -> Self {
        Self {
            join: RoadJoin::default(),
            smoothing: RoadSmoothing::default(),
            casing: false,
            sidewalks: true,
            markings: true,
        }
    }
}

/// Ширина тротуара с одной стороны улицы, м; проезд тротуара не получает.
pub fn sidewalk_width(road_width: f32) -> Option<f32> {
    (road_width >= STREET_MIN_WIDTH).then(|| {
        (road_width * SIDEWALK_SHARE)
            .clamp(*SIDEWALK_WIDTH_RANGE.start(), *SIDEWALK_WIDTH_RANGE.end())
    })
}

/// Ширина тротуара, который у дороги **рисуется** при этом стиле: один ответ
/// и для ленты тротуара, и для зажима радиуса скругления (`roads/corners.rs`).
fn drawn_sidewalk(style: &RoadStyle, road: &RoadLine) -> Option<f32> {
    (style.sidewalks && is_carriageway(road))
        .then(|| sidewalk_width(road.width))
        .flatten()
}

/// Проезжая часть улицы — то, что несёт тротуар и разметку и участвует в
/// перекрёстках: класс `Street`, не арка (`passage` идёт сквозь дом), не у́же
/// [`STREET_MIN_WIDTH`]. Мост — тоже: улица через реку не теряет полос.
///
/// Открыт наружу для [`map::cars`](crate::map::cars): «улица, вдоль которой
/// паркуются» — то же самое понятие, что «улица, у которой есть тротуар и
/// разметка», и второй копии предиката у слоя машин быть не должно.
pub fn is_carriageway(road: &RoadLine) -> bool {
    road.class == RoadClass::Street && !road.passage && road.width >= STREET_MIN_WIDTH
}

/// Число полос проезжей части: тег `lanes`, иначе дефолт по ширине, и не
/// больше, чем влезает по [`MIN_LANE_WIDTH`]. Кольцо — всегда одна полоса:
/// на однополосном кольце линий нет, а рвать линию двухполосного на каждом
/// въезде хуже, чем не рисовать её вовсе.
pub fn lane_count(road: &RoadLine) -> u8 {
    if road.roundabout {
        return 1;
    }
    let most = ((road.width / MIN_LANE_WIDTH).floor() as u8).max(1);
    let lanes = match road.lanes {
        Some(lanes) => lanes,
        None if road.oneway => (road.width / ONEWAY_METERS_PER_LANE).floor() as u8,
        None => 2 * (road.width / TWOWAY_METERS_PER_LANE_PAIR).round() as u8,
    };
    lanes.clamp(1, most)
}

/// Разметка проезжей части: линии лежат на границах полос, так что
/// однополосной рисовать нечего.
fn road_markings(road: &RoadLine) -> Option<Markings> {
    if !is_carriageway(road) {
        return None;
    }
    let lanes = lane_count(road);
    (lanes >= 2).then_some(Markings {
        lanes,
        oneway: road.oneway,
    })
}

/// Шаг, с которым осевая крепостной стены проверяется на «стоит ли тут
/// здание стены», м.
const WALL_PROBE_STEP: f32 = 2.0;

/// Крепостные сооружения карты (`AreaKind::Kremlin`) с их AABB — чтобы лента
/// `barrier=city_wall` не рисовалась поверх стены, которая уже нарисована
/// зданием.
///
/// Лента — единственный рисунок стены там, где мапер провёл только линию. Но
/// у Тульского кремля есть и `building=wall`, и башни, и красная лента
/// ложилась по ним сверху: в 2.5D — тёмно-оранжевой обводкой рядом с поднятой
/// стеной, у башен — кругами поверх шатров. Поэтому лента режется на куски, и
/// рисуются только те, что идут **мимо** крепостных зданий. Навмеша это не
/// касается: он по-прежнему блокирует всю ленту.
struct Fortresses<'a> {
    areas: Vec<(&'a PolyArea, (Vec2, Vec2))>,
}

impl<'a> Fortresses<'a> {
    fn of(buildings: &'a [PolyArea]) -> Self {
        Self {
            areas: buildings
                .iter()
                .filter(|building| building.kind == AreaKind::Kremlin)
                .map(|building| (building, ring_bounds(&building.outer)))
                .collect(),
        }
    }

    fn covers(&self, point: Vec2) -> bool {
        self.areas.iter().any(|(area, (min, max))| {
            point.cmpge(*min).all() && point.cmple(*max).all() && point_in_area(point, area)
        })
    }

    /// Куски осевой, не накрытые крепостными зданиями. Каждый отрезок
    /// проверяется точками через [`WALL_PROBE_STEP`]; кусок начинается и
    /// кончается на такой точке.
    fn bare_runs(&self, points: &[Vec2]) -> Vec<Vec<Vec2>> {
        if self.areas.is_empty() {
            return vec![points.to_vec()];
        }
        let mut runs = Vec::new();
        let mut current: Vec<Vec2> = Vec::new();
        let mut cut = false;
        let mut visit = |point: Vec2, runs: &mut Vec<Vec<Vec2>>| {
            if self.covers(point) {
                cut = true;
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                }
                current.clear();
            } else {
                current.push(point);
            }
        };
        for (index, pair) in points.windows(2).enumerate() {
            let steps = (pair[0].distance(pair[1]) / WALL_PROBE_STEP)
                .ceil()
                .max(1.0) as usize;
            // первая точка отрезка — только у первого, дальше она уже была
            // концом предыдущего
            let from = usize::from(index > 0);
            for step in from..=steps {
                visit(pair[0].lerp(pair[1], step as f32 / steps as f32), &mut runs);
            }
        }
        if current.len() >= 2 {
            runs.push(current);
        }
        // Огрызок между двумя зданиями — не стена, а зазор разметки: осевая
        // `city_wall` и контур башни в OSM расходятся на метр-другой, и на
        // Тульском кремле у угловой башни от ленты оставался красный язычок.
        // Режется только там, где лента вообще резалась: неразрезанная линия
        // любой длины — единственный рисунок своей стены.
        if cut {
            runs.retain(|run| polyline_length(run) >= WALL_STUB_MAX);
        }
        runs
    }
}

/// Кусок крепостной ленты короче этого между крепостными зданиями не рисуется, м.
/// Двенадцати не хватило: у северо-восточных башен Тульского кремля осевая
/// расходится с контурами на куски в пятнадцать–тридцать метров, и от ленты
/// оставались красные крюки у каждого угла. Прясло стены между башнями длиннее
/// сорока метров, так что настоящий неразмеченный кусок стены под порог не
/// попадает.
const WALL_STUB_MAX: f32 = 40.0;

/// Дорожный слой карты — чтобы пересборка стиля знала, что деспавнить.
///
/// `Copy` — метку получает каждый из девяти слоёв, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct RoadLayerTag;

/// Что вышло из сборки дорожных слоёв — значением, а не только строкой в логе.
///
/// Здесь живут числа, которыми тюнилась вся эта область и которые до шва
/// нельзя было ни на чём закрепить: 8710 закруглений кербов на Туле, 903 из
/// них в полосе тротуара, 39 стежков, 8 переездов. `network` — сколько из
/// общего времени ушло **до первой ленты**, то есть на сеть, стежки и углы.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RoadReport {
    /// Стиль, которым всё это нарисовано: пять ручек из лог-строки — это он.
    pub style: RoadStyle,
    pub junctions: usize,
    pub kerb_returns: usize,
    pub sidewalk_returns: usize,
    pub stitches: usize,
    pub crossings: usize,
    pub vertices: usize,
    pub network: std::time::Duration,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for RoadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            style,
            junctions,
            kerb_returns,
            sidewalk_returns,
            stitches,
            crossings,
            vertices,
            network,
            elapsed,
        } = self;
        write!(
            f,
            "road meshing: {vertices} verts in {elapsed:?} ({:?}, smoothing {:?}, casing {}, \
             sidewalks {}, markings {}, junctions {junctions}, kerb returns {kerb_returns} + \
             {sidewalk_returns} on sidewalks, stitches {stitches}, driveway crossings \
             {crossings}; {network:?} of it before the ribbons)",
            style.join, style.smoothing, style.casing, style.sidewalks, style.markings,
        )
    }
}

/// Девять дорожных слоёв в выбранном стиле: тротуары, канты и заливки аллей и
/// улиц, три мостовых слоя и лента крепостной стены.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_roads`] и `spawn_map`), и тест. Это самый
/// крупный модуль шва, и он же самый показательный: девять слоёв, три вида
/// материала и вся телеметрия области — всё уезжает через один возврат.
pub fn mesh_roads(map: &MapData, style: RoadStyle) -> (Vec<LayerMesh>, RoadReport) {
    let started = std::time::Instant::now();
    let (roads, walls): (&[RoadLine], &[WallLine]) = (&map.roads, &map.walls);
    // перекрёстки нужны только разметке: без неё и рвать нечего
    let junctions = style
        .markings
        .then(|| junctions::marking_breaks(roads, is_carriageway));

    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut alley_casings = MeshBuilder::default();
    let mut alleys = MeshBuilder::with_surface_coords();
    let mut street_casings = MeshBuilder::default();
    let mut streets = MeshBuilder::with_surface_coords();
    // Настилы мостов — один меш на улицы и пешеходные мостики разом: белая и
    // песочная заливки соседствуют, и порядок перекрытия моста над мостом —
    // порядок пуша. Мост над мостом — редкость, четыре слоя ради него не нужны.
    let mut bridge_casings = MeshBuilder::default();
    let mut bridge_fills = MeshBuilder::with_surface_coords();
    // тень моста — на то, над чем он проходит: воду, дорогу, пути. Ленты
    // копятся и кладутся разом: их ядра объединяются (`push_bridge_shadows`)
    let mut bridge_shadows = MeshBuilder::default();
    let mut shadow_bands: Vec<ShadowBand> = Vec::new();
    // мост — цепочка ways, и тень считается по всей цепочке
    let bridges = Bridges::new(map);
    let mut wall_ribbons = MeshBuilder::default();

    let nodes = RoadNodes::new(roads);
    // Дороги так, как они рисуются: переезд через тротуар — асфальтом
    // проезда, а не песочной дорожкой (`network::driveway_crossings`).
    let crossings: Vec<(usize, RoadLine)> = network::driveway_crossings(roads, &nodes)
        .into_iter()
        .map(|(index, width)| {
            let crossing = RoadLine {
                class: RoadClass::Street,
                width,
                ..roads[index].clone()
            };
            (index, crossing)
        })
        .collect();
    let mut drawn: Vec<&RoadLine> = roads.iter().collect();
    for (index, crossing) in &crossings {
        drawn[*index] = crossing;
    }
    let stitches = network::stitches(&drawn, map, &nodes, |road| drawn_sidewalk(&style, road));
    let paths: Vec<Cow<[Vec2]>> = drawn
        .iter()
        .map(|road| centerline(road, style.smoothing, &nodes))
        .collect();
    // широкие улицы поверх узких — см. доку модуля
    let mut order: Vec<usize> = (0..roads.len()).collect();
    order.sort_by(|&a, &b| drawn[a].width.total_cmp(&drawn[b].width));
    // Скругления кладутся раньше всех лент своего слоя: лента поверх кроет
    // скругление, а не наоборот, и разметка остаётся целой. `Square` оставлен
    // ради сравнения с прежней картинкой — скруглений у него нет.
    let kerb_returns = if style.join == RoadJoin::Square {
        corners::KerbReturns::default()
    } else {
        let rounded: Vec<Option<&[Vec2]>> = drawn
            .iter()
            .zip(&paths)
            .map(|(road, path)| (!road.bridge && !road.passage).then_some(path.as_ref()))
            .collect();
        corners::kerb_returns(&drawn, &rounded, &nodes, |road| {
            drawn_sidewalk(&style, road)
        })
    };
    for (class, outline) in &kerb_returns.roads {
        let (builder, color) = match class {
            RoadClass::Street => (&mut streets, ROAD_COLOR),
            RoadClass::Alley => (&mut alleys, ALLEY_COLOR),
        };
        // Скругление не выпукло, но веер из его первой вершины — угла краёв —
        // верен: дуга между точками касания и есть та часть окружности, что
        // видна из угла. `earcutr` на восьми тысячах таких фигур стоил бы
        // больше самой укладки.
        builder.push_convex(outline, color.to_linear());
    }
    // и тот же угол в слое тротуаров: полоса поворачивает за бордюром
    for outline in &kerb_returns.sidewalks {
        sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
    }
    let network_time = started.elapsed();

    for index in order {
        let road = drawn[index];
        let (casing_color, color) = match road.class {
            RoadClass::Street => (ROAD_CASING_COLOR, ROAD_COLOR),
            RoadClass::Alley => (ALLEY_CASING_COLOR, ALLEY_COLOR),
        };
        // стежок до дороги, до которой OSM торец не довёл (`roads/network.rs`)
        let points: Cow<[Vec2]> = if stitches.touches(index) {
            let mut stitched = paths[index].to_vec();
            stitches.apply(index, &mut stitched);
            Cow::Owned(stitched)
        } else {
            Cow::Borrowed(paths[index].as_ref())
        };
        // разметка и её разрывы — только пока она включена
        let (markings, breaks) = match &junctions {
            Some(found) => (road_markings(road), found.breaks[index].as_slice()),
            None => (None, &[][..]),
        };
        if road.bridge {
            // бордюр — всегда, независимо от style.casing: он и есть мост
            push_bridge_curb(
                &mut bridge_casings,
                &points,
                2.0 * road.curb_reach(),
                style.join,
            );
            // Тень настила — тот же настил, сдвинутый по свету на высоту
            // моста. Ни один другой слой её не даёт: наземные тени считают
            // только дома, а мост через Упу — самая заметная вещь на воде.
            if let Some(deck) = bridges.span(index).filter(|deck| deck.casts) {
                shadow_bands.push(ShadowBand {
                    path: bridge_shadow_path(&points, deck),
                    reach: road.curb_reach(),
                    penumbra: bridge_penumbra(deck.span),
                });
            }
            bridge_fills.set_markings(markings);
            push_street_fill(
                &mut bridge_fills,
                &points,
                road.width,
                color.to_linear(),
                style.join,
                breaks,
            );
            continue;
        }
        let (casing, fill) = match road.class {
            RoadClass::Street => (&mut street_casings, &mut streets),
            RoadClass::Alley => (&mut alley_casings, &mut alleys),
        };
        if let Some(sidewalk) = drawn_sidewalk(&style, road) {
            push_ribbon(
                &mut sidewalks,
                &points,
                road.width + 2.0 * sidewalk,
                SIDEWALK_COLOR.to_linear(),
                style.join,
            );
        }
        if style.casing {
            let width = road.width + 2.0 * casing_width(road.width);
            push_ribbon(casing, &points, width, casing_color.to_linear(), style.join);
        }
        fill.set_markings(markings);
        push_street_fill(
            fill,
            &points,
            road.width,
            color.to_linear(),
            style.join,
            breaks,
        );
    }

    push_bridge_shadows(&mut bridge_shadows, &shadow_bands);

    let fortresses = Fortresses::of(&map.buildings);
    for wall in walls {
        for run in fortresses.bare_runs(&wall.points) {
            push_ribbon(
                &mut wall_ribbons,
                &run,
                wall.width,
                WALL_COLOR.to_linear(),
                style.join,
            );
        }
    }

    // тень моста полупрозрачна, поэтому у неё `Blend`: непрозрачный материал
    // съел бы альфу вершинного цвета. Асфальт, тротуар и дорожка — фактурные,
    // канты и лента стены — плоские
    let layers: Vec<LayerMesh> = [
        (
            sidewalks,
            Z_SIDEWALK,
            "sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            alley_casings,
            Z_ALLEY_CASING,
            "alley_casings",
            MaterialSpec::Flat,
        ),
        (
            alleys,
            Z_ALLEY,
            "alleys",
            MaterialSpec::Surface(SurfaceKind::Alley),
        ),
        (
            street_casings,
            Z_ROAD_CASING,
            "road_casings",
            MaterialSpec::Flat,
        ),
        (
            streets,
            Z_ROAD,
            "roads",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (
            bridge_shadows,
            Z_BRIDGE_SHADOW,
            "bridge_shadows",
            MaterialSpec::Blend,
        ),
        (
            bridge_casings,
            Z_BRIDGE_CASING,
            "bridge_casings",
            MaterialSpec::Flat,
        ),
        (
            bridge_fills,
            Z_BRIDGE,
            "bridges",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (wall_ribbons, Z_WALL, "walls", MaterialSpec::Flat),
    ]
    .into_iter()
    .map(|(builder, z, name, material)| LayerMesh::new(builder, z, name, material))
    .collect();

    let report = RoadReport {
        style,
        junctions: junctions.as_ref().map_or(0, |found| found.junctions),
        kerb_returns: kerb_returns.roads.len(),
        sidewalk_returns: kerb_returns.sidewalks.len(),
        stitches: stitches.count,
        crossings: crossings.len(),
        vertices: layers.iter().map(|l| l.builder.vertex_count()).sum(),
        network: network_time,
        elapsed: started.elapsed(),
    };
    (layers, report)
}

/// Офлайн-замер дорожных слоёв — строками `LayerCost`, как у зданий и машин.
///
/// Своей сборки у него нет: он зовёт тот же [`mesh_roads`], что и игра. До шва
/// дорожные слои мерились только строкой `road meshing:` из живого приложения,
/// то есть ровно тем способом, который на macOS врёт (App Nap).
pub fn measure_roads(map: &MapData) -> Vec<LayerCost> {
    let (layers, report) = mesh_roads(map, RoadStyle::default());
    surface::layer_costs(&layers, report.elapsed)
}

/// Пересборка дорожных слоёв после переключения стиля из UI или BRP: деспавн
/// старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    style: Res<RoadStyle>,
    map: Res<MapData>,
    existing: Query<Entity, With<RoadLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_road_meshes(
        &mut commands,
        &mut meshes,
        &materials,
        mesh_roads(&map, *style),
    );
}

/// Положить в мир то, что собрал [`mesh_roads`]: слои под `RoadLayerTag`, плюс
/// отчёт в лог. Одна дверь для `rebuild_roads` и `spawn_map` — форма
/// `buildings::spawn_building_meshes`: дверь нужна не по числу меток, а по
/// числу вызывающих.
pub fn spawn_road_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &LayerMaterials,
    (layers, report): (Vec<LayerMesh>, RoadReport),
) {
    spawn_layers(commands, meshes, materials, layers, RoadLayerTag);
    info!("{report}");
}

/// Теневая лента одного моста, готовая к укладке: путь, полуширина настила и
/// ширина полутени на полном подъёме.
struct ShadowBand {
    path: Vec<ShadowPoint>,
    reach: f32,
    penumbra: f32,
}

/// Край теневой ленты в одной точке пути.
struct ShadowEdge {
    left: Vec2,
    right: Vec2,
    /// единичная нормаль стыка, наружу слева
    normal: Vec2,
    /// ширина полутени здесь — ноль у устоя, полная на пролёте
    blur: f32,
}

/// Ширина мягкого края тени настила, м.
///
/// Не физическая полутень — угловой размер солнца дал бы на такой длине
/// сантиметры, — а то, чем край тени размыт на снимке: разрешением кадра и
/// светом неба. Правило у карты уже есть, и оно сформулировано в машинном
/// `cars/body.rs::SHADOW_BLUR`: «у зданий метр, и он втрое больше, **потому
/// что и тень там втрое-вдесятеро длиннее**». Значит кайма — доля **длины
/// собственной тени**, то есть высоты настила через `shadow_length_scale()`,
/// а не константа, как у машины и забора, у которых высота одна на всех.
///
/// Мост тут самый высокий из всех, кто отбрасывает тень на этой карте, и
/// самый разный: от двухметрового мостика через пруд до шестиметрового
/// путепровода. Поэтому доля, а концы — два числа, которые на карте уже
/// стоят: [`PENUMBRA_MIN`] — кайма машины и забора (мягче на этой карте не
/// размыт никакой край), [`PENUMBRA_MAX`] — кайма дома
/// (`buildings::layers::PENUMBRA_WIDTH`, мягче не бывает вовсе). Между ними
/// доля и работает: пролёт 16 м — 0.36 м каймы, 40 м — 0.9, от 44 м и выше —
/// потолок. Концы у солнца не едут (это константы соседних слоёв), а сама
/// доля едет: на низком солнце тень длиннее, и край у неё мягче.
const PENUMBRA_SHARE: f32 = 0.3;
const PENUMBRA_MIN: f32 = 0.35;
const PENUMBRA_MAX: f32 = 1.0;

/// Ширина полутени для моста с таким пролётом — см. [`PENUMBRA_SHARE`].
fn bridge_penumbra(span: f32) -> f32 {
    let length = bridge_height(span) * shadow_length_scale();
    (length * PENUMBRA_SHARE).clamp(PENUMBRA_MIN, PENUMBRA_MAX)
}

/// Тени всех мостов разом: **объединённые** ядра ([`ShadowBand`]) плюс мягкая
/// кайма по краю каждой ленты.
///
/// Ядро ленты — переменной ширины по [`bridge_shadow_path`], **шире самого
/// настила** на [`SHADOW_SPREAD`] с каждой стороны, и торцы у неё прямые.
///
/// Уширение — не украшение, а единственное, что даёт мосту тень **вдоль
/// света**. Тень плиты это её силуэт, сдвинутый по солнцу; у ленты сдвиг
/// раскладывается на поперечную часть (видимая полоса сбоку) и продольную
/// (лента съезжает сама по себе и остаётся под настилом). Мостики через пруд
/// на Упе идут ровно по азимуту солнца — поперечной части у них нет, и честная
/// тень-сдвиг у них невидима до последнего пикселя. На снимке они, однако,
/// обведены тёмным: вода под настилом не освещена небом, к этому добавляется
/// тень перил и толщина самой плиты. Это и есть `SHADOW_SPREAD` — кайма,
/// сходящая к нулю там же, где садится на землю настил.
///
/// Торцы прямые (лента режется по вершинам, полудиска нет ни на одном конце):
/// круглый торец и был тем артефактом, из-за которого тень «оставалась у
/// дороги» — полудиск радиусом в полширины настила вылезал за прямой срез
/// моста и ложился на улицу, к которой мост примыкает.
///
/// **Ядра объединяются булевым union** (`i_overlay`, NonZero — приём теней
/// зданий и оград), и это не про запас: на Туле **28 пар разных мостов**
/// накрывают друг друга тенями. Так OSM размечает мост с тротуаром — отдельным
/// параллельным way с тем же `bridge=yes`, — и в полупрозрачном слое такая пара
/// читается полосой двойной темноты вдоль всего моста. Одна лента — это один
/// контур (левый рельс вперёд, правый назад), и её собственное самокасание на
/// крутом повороте NonZero закрашивает один раз, так что союз нужен именно
/// ради соседей.
///
/// **Кайма при этом кладётся от каждой ленты своей**, до объединения, и вот
/// почему её нельзя считать от контура союза: её ширина берётся из `rise`,
/// то есть из того, насколько настил в этой точке поднят над землёй, а союз
/// эту величину теряет. Отказаться от `rise` тоже нельзя — у устоя настил
/// лежит на земле, и метр мягкой тени вокруг его торца это ровно та «грязная
/// обводка», ради избавления от которой у зданий убирали контактную юбку.
/// Перекрытие двух кайм друг с другом карта уже разрешает явно (тени зданий:
/// «каймы соседних фигур могут перекрываться, но обе гаснут в ноль»). Кайма,
/// чья внешняя кромка лежит внутри ядра соседней ленты, не кладётся.
fn push_bridge_shadows(builder: &mut MeshBuilder, bands: &[ShadowBand]) {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let color = SHADOW_COLOR.to_linear();
    let fade = LinearRgba {
        alpha: 0.0,
        ..color
    };
    let edges: Vec<Vec<ShadowEdge>> = bands.iter().map(shadow_edges).collect();

    // ядра лент для проверки погружения каймы
    let cores: Vec<Vec<Vec2>> = edges
        .iter()
        .map(|band| {
            if band.len() < 2 {
                Vec::new()
            } else {
                band.iter()
                    .map(|edge| edge.left)
                    .chain(band.iter().rev().map(|edge| edge.right))
                    .collect()
            }
        })
        .collect();

    // контур ленты — левый рельс вперёд, правый назад
    let contours: Vec<Vec<[f32; 2]>> = edges
        .iter()
        .filter(|band| band.len() >= 2)
        .map(|band| {
            band.iter()
                .map(|edge| edge.left.to_array())
                .chain(band.iter().rev().map(|edge| edge.right.to_array()))
                .collect()
        })
        .collect();
    for shape in contours.simplify_shape(FillRule::NonZero) {
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

    for (own, band) in edges.iter().enumerate() {
        for pair in band.windows(2) {
            let (near, far) = (&pair[0], &pair[1]);
            // у устоя кайма схлопнута с обеих сторон — квада там нет вовсе
            if near.blur <= 0.0 && far.blur <= 0.0 {
                continue;
            }
            for side in [1.0_f32, -1.0] {
                let (from, to) = if side > 0.0 {
                    (near.left, far.left)
                } else {
                    (near.right, far.right)
                };
                let lip =
                    (from + near.normal * (side * near.blur) + to + far.normal * (side * far.blur))
                        / 2.0;
                // кайма, чья внешняя кромка лежит в ядре соседа, легла бы
                // поверх уже закрашенного союза двойной темнотой
                let buried = cores
                    .iter()
                    .enumerate()
                    .any(|(other, ring)| other != own && point_in_polygon(lip, ring));
                if buried {
                    continue;
                }
                builder.push_quad_gradient(
                    [
                        from,
                        from + near.normal * (side * near.blur),
                        to + far.normal * (side * far.blur),
                        to,
                    ],
                    [color, fade, fade, color],
                );
            }
        }
    }
}

/// Края ленты: рельсы, нормаль стыка и ширина полутени в каждой точке пути.
/// Общие у контура и у каймы, чтобы кайма садилась ровно на край ядра.
fn shadow_edges(band: &ShadowBand) -> Vec<ShadowEdge> {
    if band.path.len() < 2 {
        return Vec::new();
    }
    let centers: Vec<Vec2> = band.path.iter().map(|point| point.at).collect();
    // единичные нормали стыка: длину каждой задаёт своя полуширина
    let normals = miter_offsets(&centers, false, 1.0);
    band.path
        .iter()
        .zip(&normals)
        .map(|(point, normal)| {
            let half = band.reach + SHADOW_SPREAD * point.rise;
            ShadowEdge {
                left: point.at + *normal * half,
                right: point.at - *normal * half,
                normal: *normal,
                blur: band.penumbra * point.rise,
            }
        })
        .collect()
}

/// Бордюр моста: торцы всегда [`RibbonCap::Butt`] — настил кончается ровным
/// срезом, как на 2ГИС. Полудиск `Round` или продление `push_polyline` при
/// `Square` торчали бы бордюрным языком за конец моста, поэтому мимо
/// [`push_ribbon`]-обёртки.
fn push_bridge_curb(builder: &mut MeshBuilder, points: &[Vec2], width: f32, join: RoadJoin) {
    // `Square` — это `push_polyline` с продлёнными торцами, а торцы здесь
    // решены выше; его излом сводится к `Miter`, как у любой метки, а не дороги
    let join = match join {
        RoadJoin::Square | RoadJoin::Miter => RibbonJoin::Miter,
        RoadJoin::Round => RibbonJoin::Round,
    };
    builder.push_ribbon(
        points,
        false,
        width,
        BRIDGE_CURB_COLOR.to_linear(),
        join,
        RibbonCap::Butt,
    );
}

/// Лента выбранного стиля. Общая с подложкой аллей (`map::spawn`): у неё те же
/// три настройки, что у дорог, и мапиться на `MeshBuilder` они обязаны одинаково.
pub fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    builder.push_ribbon(points, false, width, color, join, cap);
}

/// Заливка проезжей части — лента с разрывами разметки по перекрёсткам. При
/// `Square` разрывы деть некуда: `push_polyline` знает только торцы, а режим
/// оставлен ради сравнения картинок, не ради разметки.
fn push_street_fill(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
    breaks: &[Break],
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    builder.push_ribbon_broken(
        points,
        width,
        color,
        join,
        [cap; 2],
        RibbonBreaks::At(breaks),
    );
}

/// Осевая, по которой строится лента. Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
///
/// **Общие узлы с другими дорогами тоже не сглаживаются** ([`RoadNodes`]): на
/// узле кончается поперечная улица и сходятся лучи скругления бордюра
/// (`roads/corners.rs`). Сдвинь хорда сквозную дорогу с узла — торец
/// поперечной повис бы в метре от её асфальта или вылез за дальний край.
fn centerline<'a>(
    road: &'a RoadLine,
    smoothing: RoadSmoothing,
    nodes: &RoadNodes,
) -> Cow<'a, [Vec2]> {
    if road.passage {
        return Cow::Borrowed(&road.points);
    }
    smooth_pinned(&road.points, road.width, smoothing, |point| {
        nodes.is_shared(point)
    })
}

/// Сглаживание осевой на копии — общее для дорог, рельсов и зелёной полосы под
/// аллеей (`map::spawn`). Длина среза зажата шириной ленты, поэтому ширина
/// здесь параметр, а не константа.
pub fn smooth_path(points: &[Vec2], width: f32, smoothing: RoadSmoothing) -> Cow<'_, [Vec2]> {
    smooth_pinned(points, width, smoothing, |_| false)
}

/// [`smooth_path`], не трогающее вершины, для которых `pinned` — да.
fn smooth_pinned(
    points: &[Vec2],
    width: f32,
    smoothing: RoadSmoothing,
    pinned: impl Fn(Vec2) -> bool + Copy,
) -> Cow<'_, [Vec2]> {
    let iterations = smoothing.iterations();
    if iterations == 0 || points.len() < 3 {
        return Cow::Borrowed(points);
    }
    let mut path = points.to_vec();
    for _ in 0..iterations {
        path = chaikin(&path, width, pinned);
    }
    Cow::Owned(path)
}

/// Срезание углов по Chaikin: излом заменяется парой точек на прилежащих
/// сегментах. Срезаются только изломы круче [`MIN_SMOOTH_ANGLE`], а длина
/// среза зажата шириной дороги — иначе на длинных сегментах осевая уезжает от
/// данных OSM на десятки метров и дорога перестаёт совпадать с домами.
/// Концы пути и вершины, для которых `pinned` — да, закреплены.
fn chaikin(points: &[Vec2], width: f32, pinned: impl Fn(Vec2) -> bool) -> Vec<Vec2> {
    let mut path = Vec::with_capacity(points.len() * 2);
    path.push(points[0]);
    for index in 1..points.len() - 1 {
        let (previous, corner, next) = (points[index - 1], points[index], points[index + 1]);
        if pinned(corner) {
            path.push(corner);
            continue;
        }
        let (Some(incoming), Some(outgoing)) = (
            (corner - previous).try_normalize(),
            (next - corner).try_normalize(),
        ) else {
            path.push(corner);
            continue;
        };
        if incoming.angle_to(outgoing).abs() < MIN_SMOOTH_ANGLE {
            path.push(corner);
            continue;
        }
        let back = (corner.distance(previous) * CHAIKIN_CUT).min(width);
        let forward = (next.distance(corner) * CHAIKIN_CUT).min(width);
        path.push(corner - incoming * back);
        path.push(corner + outgoing * forward);
    }
    path.push(points[points.len() - 1]);
    path
}

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин обязан
/// рваться на тех же перекрёстках, на которых рвётся разметка, и второго
/// восстановления узлов по общим нодам заводить незачем.
pub(super) mod junctions;

mod corners;
/// Открыт наружу для [`map::footprint`](crate::map::footprint): проём в ограде
/// у брошенного торца — тот же вопрос «висячий ли он», что у стежка, и второго
/// ответа на него быть не должно.
pub(super) mod network;

#[cfg(test)]
mod tests;
