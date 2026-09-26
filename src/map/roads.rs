//! Слой дорог, аллей и стен Кремля: по ленте на `RoadLine`/`WallLine`, слитой
//! в merged-меш на класс. Тумблеры слоёв — ресурс [`RoadStyle`], форма —
//! [`RoadShape`] (`roads/shape.rs`); оба переключаются на лету панелью
//! (`ui/roads.rs`, `ui/road_paint.rs`), и правка пересобирает только эти слои
//! ([`rebuild_roads`]). Рельсовые пути — в `map/rail.rs`, трамвай — в
//! `map/tram.rs`: у обоих свой стиль и свой зум-LOD, и пересобираются они по
//! зуму, а не по [`RoadStyle`].
//!
//! Мост (`RoadLine::bridge`) уходит из слоёв своего класса в тройку
//! `bridge_shadows` + `bridge_casings` + `bridges`: серый бордюр по краям
//! настила и заливка цветом
//! класса над `Z_ROAD` — эстакада кроет улицу, которую пересекает, а ровные
//! торцы бордюра читаются как края настила, вид 2ГИС. Под ними —
//! [`push_bridge_shadows`]: единственное на карте, что говорит, что настил
//! поднят, потому что наземные тени считают только дома.
//!
//! Раньше дороги рисовал `MeshBuilder::push_polyline` — свой квад на
//! сегмент, продлённый с обоих концов на полуширины. Стыков у него нет вообще:
//! на изломе продление торчит за внешний угол прямоугольным выступом, между
//! двумя выступами остаётся выемка, а торец пути — квадратный шип. Лента
//! теперь всегда [`RoadJoin::Round`] ([`ROAD_JOIN`]) — дуга на внешней стороне
//! излома и полудиск на
//! торце, то же самое, что `stroke-linejoin: round` + `stroke-linecap: round`
//! у Mapnik, которым нарисован osm-carto: круглые торцы двух ways в общем узле
//! перекрываются и сливаются в скруглённый стык.
//!
//! Осевая (`RoadLine::points`) при этом **не трогается**: на ней стоят навмеш
//! (`bridge`/`passage`-прорезы), арки, посадка деревьев и генератор дверей.
//! Chaikin-сглаживание работает на копии и только ради картинки; само правило
//! живёт в `map/smooth.rs` — его читают ещё пять слоёв, — а здесь остаётся
//! [`centerline`], дорожная обёртка над ним с её двумя закреплениями.
//!
//! Улица — это не одна лента, а три слоя: **тротуар** (`Z_SIDEWALK`, светлая
//! полоса шире проезжей части на [`sidewalk_width`] с каждой стороны), кант и
//! заливка асфальтом. Тротуар лежит под лентами **улицы** по той же логике, что
//! кант: заливка поперечной улицы кроет его на перекрёстке, и тротуар
//! обрывается там, где обрывается в жизни, — но **поверх ленты аллеи**: дорожка,
//! выходящая на улицу, упирается в тротуар, как упирается в бордюр на
//! фотографии, вместо того чтобы перечеркнуть полосу песочной лентой. **Разметка** — линии по границам
//! полос ([`lane_count`]) — отдельный **слой краски** (`roads/paint.rs`):
//! геометрия от оси улицы на той же раскладке полос, по которой шейдер
//! асфальта кладёт колею. Линия **рвётся на перекрёстках** — по общим узлам
//! ways (`roads/junctions.rs`), а не по торцам, так что way, разрезанный посреди
//! квартала, несёт линию сквозь стык, а сквозная улица теряет её ровно на
//! ширину поперечной. Широкие улицы кладутся поверх узких: заливка магистрали
//! кроет торец жилой улицы, и колея въезда гаснет под ней.

use std::borrow::Cow;

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

use self::network::RoadNodes;
pub use self::node_paint::CrossingMode;
use self::shape::{RoadShape, RoadShapeOnMap};
use crate::map::footprint::JOIN_EPSILON;
use crate::map::meshing::{
    Break, LaneFrame, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin, RibbonShape,
    merge_close_points, miter_offsets, to_break_beyond,
};
use crate::map::osm::model::{
    RoadNodeKind, distance_to_segment, point_in_area, point_in_polygon, polyline_length,
    ring_bounds,
};
use crate::map::osm::{AreaKind, MapData, PolyArea, RoadClass, RoadLine, WallLine};
use crate::map::shadow;
use crate::map::shapes::{is_ring, push_shape};
use crate::map::smooth::{Smoothing, smooth_pinned};
use crate::map::spawn::GRASS_COLOR;
use crate::map::surface::{
    self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, SurfaceKind, spawn_layers,
};
use crate::map::{SHADOW_COLOR, SunOnMap};
use crate::prefs::retuned;
use crate::settings::{
    Z_ALLEY, Z_BRIDGE, Z_BRIDGE_CASING, Z_BRIDGE_SHADOW, Z_BUILDING, Z_LOT_LINES, Z_LOT_SIDEWALK,
    Z_ROAD, Z_ROAD_MEDIAN, Z_SIDEWALK,
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
    let offset = shadow::offset(bridge_height(deck.span));
    let ramp = (deck.span * RAMP_SHARE).clamp(f32::EPSILON, RAMP_MAX);
    let last = dense.len() - 1;
    // нормали стыка — по осевой **настила**, до сдвига: см. [`ShadowPoint`]
    let normals = miter_offsets(&dense, false, 1.0);
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
                normal: normals[index],
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
///
/// `normal` — нормаль стыка **осевой настила**, а не съехавшей ленты, и
/// считается она здесь же, до сдвига. Тень плиты это её силуэт, сдвинутый по
/// свету: поперечник ленты обязан стоять поперёк моста, а не поперёк той
/// кривой, в которую лента складывается. Разница видна ровно у торца, где
/// сдвиг только начинается: съехавшая осевая уходит вбок прямо от устоя, её
/// нормаль наклонена, торцевое ребро ленты получается косым — и один его угол
/// уезжает за торец на `полуширину × sin` этого наклона. На карте это тёмный
/// язычок из-под конца бортика (у мостика через Упу — 0.75 м), с той стороны,
/// куда светит солнце; с другой стороны торец на столько же подрезан.
struct ShadowPoint {
    at: Vec2,
    rise: f32,
    normal: Vec2,
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
/// Асфальт над трамвайными путями (`roads/tram_band.rs`): светлее
/// [`ROAD_COLOR`] едва заметно, как на Яндексе, — трамвайная полоса читается
/// цветом, а не разметкой.
pub const TRAM_BAND_COLOR: Color = Color::srgb(0.59, 0.59, 0.595);
const ALLEY_COLOR: Color = Color::srgb(0.914, 0.875, 0.769);
const WALL_COLOR: Color = Color::srgb(0.639, 0.286, 0.235);

/// Белая разметка на асфальте стоянки: двойная сплошная между встречными
/// полотнами (`roads/lots.rs`), слой `lot_lines`. Обводка со штриховкой
/// направляющего островка (`roads/gores.rs`) — не здесь: она в слое краски
/// `road_paint_islands`.
const LOT_LINE_COLOR: Color = Color::srgb(0.88, 0.88, 0.86);

/// Тротуар — светлый бетон между асфальтом и тёплой землёй: светлее проезжей
/// части на четверть, и именно эта ступень яркости читается как бордюр.
const SIDEWALK_COLOR: Color = Color::srgb(0.82, 0.815, 0.80);
/// Доля ширины улицы на тротуар с каждой стороны и её пределы, м: у
/// магистрали в 16 м тротуар в 3 м, у жилой улицы в 8 м — 1.8 м.
const SIDEWALK_SHARE: f32 = 0.22;
const SIDEWALK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 1.2..=3.0;
/// Кусок тротуара короче этого, м, не кладётся ([`push_sidewalk`]): между
/// кусками пары остаются обрезки в сантиметры.
const SIDEWALK_PIECE_MIN: f32 = 0.5;
/// Самый длинный кусок поперечной улицы между половинами одной пары, м: две
/// половины и самый широкий газон между ними. Такой кусок лежит в проёме
/// разделительной, и тротуара у него нет.
const MEDIAN_CROSSING_MAX: f32 = 40.0;
/// На сколько асфальт кармана стоянки заходит под кромку ленты, м: встык
/// между ними светилась бы щель.
const POCKET_OVERLAP: f32 = 0.05;

/// Полоса не у́же этого, м. У разобранной улицы ширина выведена из самих
/// полос (`network::sections`), и зажим ничего не режет; он остался для
/// дорог, собранных руками, где `lanes=6` может прийти на десятиметровую
/// ленту.
const MIN_LANE_WIDTH: f32 = 2.5;
/// Полос по умолчанию, когда тега `lanes` нет: двусторонней улице — по паре
/// на каждые 7 м ширины (8 и 10 м — две полосы, 12 и 16 — четыре),
/// односторонней — по полосе на 4.5 м (8 м — одна, без линий; 16 — три).
const TWOWAY_METERS_PER_LANE_PAIR: f32 = 7.0;
const ONEWAY_METERS_PER_LANE: f32 = 4.5;

/// Стены Кремля поверх зданий.
const Z_WALL: f32 = Z_BUILDING + 0.1;

/// Бордюр моста — светлый бетонный парапет над серым настилом, общий для
/// улиц и пешеходных мостиков. Толщины (и почему их диапазоны не
/// пересекаются) — в `map::footprint`.
const BRIDGE_CURB_COLOR: Color = Color::srgb(0.80, 0.80, 0.79);

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
    /// кто кладёт ленту дороги ([`push_ribbon`], [`ROAD_RIBBON`]) —
    /// разойдясь, они дали бы двум слоям одной улицы разные торцы.
    const fn ribbon_shape(self) -> Option<(RibbonJoin, RibbonCap)> {
        match self {
            Self::Square => None,
            Self::Miter => Some((RibbonJoin::Miter, RibbonCap::Butt)),
            Self::Round => Some((RibbonJoin::Round, RibbonCap::Round)),
        }
    }
}

/// Стык и торец дорожной ленты. Выбора больше нет: углы и стыки узлов строит
/// полигон узла (`roads/corners.rs`), и `Square` держали только для сравнения
/// со старой картинкой. Сам [`RoadJoin`] остаётся ручкой полосы посадки
/// аллей (`TreeRowStyle`).
pub const ROAD_JOIN: RoadJoin = RoadJoin::Round;

/// [`ROAD_JOIN`] как излом и торец ленты `MeshBuilder`, развёрнутый на
/// компиляции: у дорожной ленты стык всегда лента, и ветки «`Square` — это
/// `push_polyline`» у заливки проезжей части нет.
const ROAD_RIBBON: (RibbonJoin, RibbonCap) = match ROAD_JOIN.ribbon_shape() {
    Some(shape) => shape,
    None => panic!("ROAD_JOIN is not a ribbon join"),
};

/// Тумблеры дорожных слоёв; переключаются панелью (секции Roads и Road paint)
/// и BRP, сохраняются в настройках между запусками. Правка пересобирает
/// дорожные слои ([`rebuild_roads`]).
///
/// Прежние `join`, `smoothing` и `casing` ушли: стык строит полигон узла,
/// сглаживание стало допуском в метрах ([`RoadShape::curve_tolerance`]), а
/// тёмный кант — картографический приём, который с бордюром и тротуаром не
/// нужен. Их ключи в `settings.toml` с другой ветки читаются и молча
/// пропускаются: `bevy_settings` применяет только поля, которые у типа есть.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "roads")]
pub struct RoadStyle {
    /// Серая полоса тротуара вдоль улиц (не проездов) отдельным слоем под
    /// всеми лентами.
    pub sidewalks: bool,
    /// Разметка полос на проезжей части улиц — линия на каждой границе полос,
    /// с разрывами на перекрёстках; слой краски (`roads/paint.rs`). Колея
    /// асфальта от неё не зависит — у неё своя ручка (`RoadPaintStyle`).
    pub markings: bool,
    /// Зебры на плечах узлов и на переходах (`roads/node_paint.rs`).
    pub crossings: CrossingMode,
    /// Стоп-линии на плечах, что уступают, и у регулируемых переходов.
    pub stop_lines: bool,
    /// Стрелки на полосах подходов к узлу (`roads/turns.rs`).
    pub arrows: bool,
}

impl Default for RoadStyle {
    fn default() -> Self {
        Self {
            sidewalks: true,
            markings: true,
            crossings: CrossingMode::default(),
            stop_lines: true,
            arrows: true,
        }
    }
}

/// Ширина тротуара с одной стороны проезжей части шириной `road_width`, м —
/// без вопроса, есть ли у дороги тротуар вообще (это [`sidewalk_width`]).
pub fn sidewalk_band(road_width: f32) -> f32 {
    (road_width * SIDEWALK_SHARE).clamp(*SIDEWALK_WIDTH_RANGE.start(), *SIDEWALK_WIDTH_RANGE.end())
}

/// Ширина тротуара у дороги с одной стороны, м; у проезда и дорожки
/// тротуара нет ([`is_carriageway`]).
pub fn sidewalk_width(road: &RoadLine) -> Option<f32> {
    is_carriageway(road).then(|| sidewalk_band(road.width))
}

/// Ширина тротуара, который у дороги **рисуется** при этом стиле: один ответ
/// и для ленты тротуара, и для его скругления в узле (`roads/corners.rs`).
/// Улица с `sidewalk=no|separate` с обеих сторон ленты не несёт; с одной —
/// её кладёт [`push_sidewalk`] по [`RoadLine::sidewalks`].
fn drawn_sidewalk(style: &RoadStyle, road: &RoadLine) -> Option<f32> {
    style
        .sidewalks
        .then(|| sidewalk_width(road))
        .flatten()
        .filter(|_| road.sidewalks.contains(&true))
}

/// Проезжая часть улицы — то, что несёт тротуар и разметку и участвует в
/// перекрёстках: улица по классу ([`Highway::is_street`] — не дворовый
/// проезд), не арка (`passage` идёт сквозь дом). Мост — тоже: улица через
/// реку не теряет полос.
///
/// Решает **класс, а не ширина**: пока ширина шла по классу, порог в 8 м
/// был тем же классом другими словами, но ширина из сечения
/// (`network::sections`) у двухполосной улицы — 7.6 м, у однополосной
/// односторонней — 4.3, и порог по ширине отнял бы у них тротуар.
///
/// Открыт наружу для [`map::cars`](crate::map::cars): «улица, вдоль которой
/// паркуются» — то же самое понятие, что «улица, у которой есть тротуар и
/// разметка», и второй копии предиката у слоя машин быть не должно.
pub fn is_carriageway(road: &RoadLine) -> bool {
    road.class == RoadClass::Street && !road.passage && road.highway.is_street()
}

/// Число полос проезжей части: из сечения ([`RoadLine::lanes`] — после
/// разбора оно есть у каждой улицы, `network::sections`), у дороги без него
/// (собранной тестом руками) — дефолт по ширине, и не больше, чем влезает по
/// [`MIN_LANE_WIDTH`]. Кольцо — как любая улица: разрывы на въездах даёт
/// краска узла, и линия двухполосного кольца идёт между ними.
pub fn lane_count(road: &RoadLine) -> u8 {
    let most = ((road.width / MIN_LANE_WIDTH).floor() as u8).max(1);
    let lanes = match road.lanes {
        Some(lanes) => lanes,
        None if road.oneway => (road.width / ONEWAY_METERS_PER_LANE).floor() as u8,
        None => 2 * (road.width / TWOWAY_METERS_PER_LANE_PAIR).round() as u8,
    };
    lanes.clamp(1, most)
}

/// Граней у круга разворотной площадки.
const TURNING_CIRCLE_SIDES: usize = 32;
/// Радиус разворотной площадки по отношению к полуширине дороги и его
/// пределы, м: легковой машине на развороте нужно метров шесть, мусоровозу —
/// десять, а площадка шире дороги, которая к ней ведёт, раза в два.
const TURNING_CIRCLE_SCALE: f32 = 2.2;
const TURNING_CIRCLE_RADIUS: std::ops::RangeInclusive<f32> = 6.0..=10.0;

/// Радиус разворотной площадки в тупике дороги шириной `width`, м.
fn turning_radius(width: f32) -> f32 {
    (width / 2.0 * TURNING_CIRCLE_SCALE)
        .clamp(*TURNING_CIRCLE_RADIUS.start(), *TURNING_CIRCLE_RADIUS.end())
}

/// Дуги колец ([`rings`]) сечением всего кольца: ширина и полосы —
/// наибольшие по его дугам. У дуг одного кольца в OSM бывает разное `lanes`
/// (3 и 2 на кольце primary в Туле), и лента шла бы ступенями.
fn ring_arcs(roads: &[RoadLine], rings: &rings::Rings) -> Vec<(usize, RoadLine)> {
    rings
        .list
        .iter()
        .flat_map(|ring| {
            let width = ring
                .roads
                .iter()
                .map(|&road| roads[road].width)
                .fold(0.0, f32::max);
            let lanes = ring
                .roads
                .iter()
                .filter_map(|&road| roads[road].lanes)
                .max();
            ring.roads
                .iter()
                .filter(move |&&road| roads[road].width != width || roads[road].lanes != lanes)
                .map(move |&road| {
                    let arc = RoadLine {
                        width,
                        lanes,
                        ..roads[road].clone()
                    };
                    (road, arc)
                })
        })
        .collect()
}

/// Тротуар кольца и бордюр его острова. Тротуар — только снаружи, лентой по
/// всему кольцу сразу, без швов между дугами; внутри вместо тротуарного
/// кольца — бордюр [`medians::MEDIAN_KERB`] по кромке острова, как у газона
/// разделительной.
fn push_ring_edges(
    builder: &mut MeshBuilder,
    ring: &rings::Ring,
    [width, sidewalk]: [f32; 2],
    color: LinearRgba,
) {
    let path = &ring.path[..ring.path.len() - 1];
    if path.len() < 3 {
        return;
    }
    // сдвиг от оси: плюс — наружу
    let shifted = |shift: f32| -> Vec<Vec2> {
        let outward = if ring.ccw { -shift } else { shift };
        path.iter()
            .zip(miter_offsets(path, true, outward))
            .map(|(point, offset)| *point + offset)
            .collect()
    };
    if sidewalk > 0.0 {
        builder.push_ribbon(
            &shifted(sidewalk / 2.0),
            true,
            width + sidewalk,
            color,
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }
    let kerb = medians::MEDIAN_KERB;
    builder.push_ribbon(
        &shifted(-(width + kerb) / 2.0),
        true,
        kerb,
        color,
        RibbonJoin::Miter,
        RibbonCap::Butt,
    );
}

/// Раскладка полос проезжей части ([`paint::lane_frame`]) — одна на колею
/// асфальта и на линии слоя краски. У проезда и дорожки полос нет, и колеи
/// тоже; однополосная улица колею получает, а линий у неё нет.
fn road_lanes(road: &RoadLine) -> Option<LaneFrame> {
    is_carriageway(road).then(|| paint::lane_frame(lane_count(road)))
}

/// Порядок заливки лент: узкие под широкими, ведущие узлов — поверх всех
/// (см. доку модуля), и **в каждом узле его плечи — до его ведущей**. Одного
/// «ведущие последними» мало: примыкание, что само ведёт другой узел дальше,
/// попадало в хвост вместе с главной и, если было шире, ложилось на неё
/// торцом — квадрат без колеи посреди перекрёстка (Тула, 5968, 1582).
/// Топологическая сортировка с приоритетом по прежнему ключу; на цикле
/// (две дороги ведут узлы друг друга) берётся первая по ключу.
fn fill_order(widths: &[f32], leading: &[bool], junctions: &[node_paint::Junction]) -> Vec<usize> {
    use std::collections::BTreeSet;
    let mut by_key: Vec<usize> = (0..widths.len()).collect();
    by_key.sort_by(|&a, &b| {
        leading[a]
            .cmp(&leading[b])
            .then(widths[a].total_cmp(&widths[b]))
    });
    let mut rank = vec![0; widths.len()];
    for (place, &road) in by_key.iter().enumerate() {
        rank[road] = place;
    }
    // ребро «плечо → ведущая»: плечо ложится раньше
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); widths.len()];
    let mut before = vec![0usize; widths.len()];
    for junction in junctions {
        let mut arms: Vec<usize> = junction.arms.iter().map(|arm| arm.road).collect();
        arms.sort_unstable();
        arms.dedup();
        for &lead in &junction.leading {
            for &arm in arms.iter().filter(|&&arm| !junction.leading.contains(&arm)) {
                if !after[arm].contains(&lead) {
                    after[arm].push(lead);
                    before[lead] += 1;
                }
            }
        }
    }
    let mut ready: BTreeSet<usize> = (0..widths.len())
        .filter(|&road| before[road] == 0)
        .map(|road| rank[road])
        .collect();
    let mut left: BTreeSet<usize> = (0..widths.len()).map(|road| rank[road]).collect();
    let mut order = Vec::with_capacity(widths.len());
    while let Some(&first) = left.first() {
        let place = ready.pop_first().unwrap_or(first);
        ready.remove(&place);
        left.remove(&place);
        let road = by_key[place];
        order.push(road);
        for &next in &after[road] {
            before[next] = before[next].saturating_sub(1);
            if before[next] == 0 && left.contains(&rank[next]) {
                ready.insert(rank[next]);
            }
        }
    }
    order
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
    /// Стиль, которым всё это нарисовано: тумблеры из лог-строки — это он.
    pub style: RoadStyle,
    pub junctions: usize,
    /// Куски линий слоя краски (`roads/paint.rs`) и их вершины — отдельно от
    /// общего счёта: краска строится своими мешами и прячется с зумом.
    pub paint_lines: usize,
    pub paint_vertices: usize,
    /// Краска узлов (`roads/node_paint.rs`): зебры (из них по OSM),
    /// стоп-линии, карманы, кластеры сближенных узлов и проходы главной
    /// сквозь узел.
    pub zebras: [usize; 2],
    pub stop_lines: usize,
    pub pockets: usize,
    pub clusters: usize,
    /// Карманы стоянки вдоль улиц (`roads/pockets.rs`) и разворотные
    /// площадки в тупиках.
    pub kerb_pockets: usize,
    pub turning_circles: usize,
    pub through: usize,
    /// Траектории узлов (`roads/turns.rs`) — кривые манёвров; дороги, ведущие
    /// хоть один узел (колея сквозь).
    pub turns: usize,
    pub leading: usize,
    /// Стрелки на полосах подходов (`roads/turns.rs`).
    pub arrows: usize,
    pub kerb_returns: usize,
    pub sidewalk_returns: usize,
    /// Наружные углы узлов (`roads/corners.rs`): асфальт и тротуар.
    pub outer_corners: [usize; 2],
    pub stitches: usize,
    pub crossings: usize,
    /// Кольца, нарисованные гладкой фигурой (`roads/rings.rs`), и щели
    /// между подходом и кольцом, залитые асфальтом.
    pub rings: [usize; 2],
    /// Острова-крошки в треугольниках узлов, залитые асфальтом
    /// (`corners::small_islands`).
    pub islands: usize,
    /// Направляющие островки у колец (`roads/gores.rs`).
    pub gores: usize,
    /// Из данных v15 (`roads/islands.rs`): островков-точек на улицах, контуров
    /// островков и контуров полотна.
    pub road_islands: [usize; 3],
    /// Клинья между сечениями улиц (`roads/tapers.rs`).
    pub tapers: usize,
    /// Слияния разделённой улицы в обычную (`roads/merges.rs`) и кромки,
    /// сведённые на них к кромке продолжения.
    pub merges: [usize; 2],
    /// Разделительные парных половин (`roads/network/pairs.rs`): асфальтом,
    /// газоном и из асфальтовых — трамвайных полотен.
    pub medians: [usize; 3],
    /// Куски светлой полосы над трамвайными путями (`roads/tram_band.rs`).
    pub tram_bands: usize,
    /// Швы ways, пройденные осью улицы одной кривой (`roads/axis.rs`).
    pub seams: usize,
    /// Изломы, на которые звеньев не хватило для радиуса в полуширину.
    pub tight: usize,
    pub vertices: usize,
    pub network: std::time::Duration,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for RoadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            style,
            junctions,
            paint_lines,
            paint_vertices,
            zebras: [zebras, osm_zebras],
            stop_lines,
            pockets,
            clusters,
            kerb_pockets,
            turning_circles,
            through,
            turns,
            leading,
            arrows,
            kerb_returns,
            sidewalk_returns,
            outer_corners: [outer, outer_sidewalks],
            stitches,
            crossings,
            rings: [rings, webs],
            islands,
            gores,
            road_islands: [refuges, island_areas, carriageways],
            tapers,
            merges: [merges, merge_edges],
            medians: [paved, lawns, beds],
            tram_bands,
            seams,
            tight,
            vertices,
            network,
            elapsed,
        } = self;
        write!(
            f,
            "road meshing: {vertices} verts in {elapsed:?} (sidewalks {}, markings {}, paint {paint_lines} lines / {paint_vertices} verts, \
             junctions {junctions} ({clusters} clusters, main through {through}), zebras \
             {zebras} ({osm_zebras} from OSM), stop lines {stop_lines}, pockets {pockets}, \
             turn paths {turns}, arrows {arrows}, leading roads {leading}, kerb returns {kerb_returns} + \
             {sidewalk_returns} on sidewalks, outer corners {outer} + {outer_sidewalks} on \
             sidewalks, stitches {stitches}, kerb pockets {kerb_pockets}, turning circles {turning_circles}, driveway crossings \
             {crossings}, rings {rings} ({webs} webs), small islands {islands}, gores {gores}, safety islands {refuges} + {island_areas} areas, \
             carriageway areas {carriageways}, tapers {tapers}, merges {merges} ({merge_edges} edges), medians {paved} paved + {lawns} \
             lawn (tram beds {beds}), tram bands {tram_bands}, smooth seams {seams}, tight corners {tight}; {network:?} of it before the \
             ribbons)",
            style.sidewalks, style.markings,
        )
    }
}

/// Восемнадцать дорожных слоёв в выбранном стиле и форме: заливка аллей,
/// тротуары, газон разделительных, заливка улиц, два слоя большой стоянки,
/// три мостовых слоя, лента крепостной стены и восемь слоёв краски
/// (`roads/paint.rs`) над своим асфальтом.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_roads`] и `spawn_map`), и тест. Это самый
/// крупный модуль шва, и он же самый показательный: восемнадцать слоёв, четыре
/// вида материала и вся телеметрия области — всё уезжает через один возврат.
pub fn mesh_roads(
    map: &MapData,
    style: RoadStyle,
    shape: RoadShape,
) -> (Vec<LayerMesh>, RoadReport) {
    let started = std::time::Instant::now();
    let (roads, walls): (&[RoadLine], &[WallLine]) = (&map.roads, &map.walls);
    let mut painter = paint::Painter::new(map.traffic_side);

    let mut sidewalks = MeshBuilder::with_surface_coords();
    let mut alleys = MeshBuilder::with_surface_coords();
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
    // улицы на больших стоянках — бордюром и разметкой поверх их асфальта
    // (`roads/lots.rs`)
    let mut grounds = lots::Grounds::of(map);

    let nodes = RoadNodes::new(roads);
    // ось по улице целиком, не по way (`roads/axis.rs`); у переезда та же
    // ось, что у его дороги, — он отличается шириной и классом
    let axes = axis::street_axes(roads, &map.rails, &map.network, &nodes, &shape);
    let paths = &axes.paths;
    // Дороги так, как они рисуются: переезд через тротуар — асфальтом
    // проезда, а не песочной дорожкой (`network::driveway_crossings`), дуга
    // кольца — сечением всего кольца (`ring_arcs`).
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
        .chain(ring_arcs(roads, &axes.rings))
        .collect();
    let mut drawn: Vec<&RoadLine> = roads.iter().collect();
    for (index, crossing) in &crossings {
        drawn[*index] = crossing;
    }
    let stitches = network::stitches(&drawn, map, &nodes, |road| drawn_sidewalk(&style, road));
    // перекрёстки, стежки среди них: по ним рвётся краска и гаснет колея
    // асфальта — колея есть и с выключенной разметкой, так что считаются они
    // всегда
    let junctions = junctions::marking_breaks(roads, is_carriageway, &stitches.targets);
    // клинья между сечениями улиц
    let tapers = tapers::Tapers::new(&drawn, &map.network, &nodes, shape.taper());
    // Кусок поперечной улицы в проёме разделительной — между половинами одной
    // пары — тротуара не несёт: его полоса светлым пятном лежала посреди
    // перекрёстка. Торцы узлов — точки OSM, и ось их не двигает.
    let across_median: Vec<bool> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
                return false;
            };
            polyline_length(path) < MEDIAN_CROSSING_MAX
                && nodes.roads_at(start).iter().any(|&half| {
                    half != index
                        && axes.pairs.runs[half].iter().any(|run| {
                            run.partner != index && nodes.roads_at(end).contains(&run.partner)
                        })
                })
        })
        .collect();
    let sidewalks_of =
        |index: usize| drawn_sidewalk(&style, drawn[index]).filter(|_| !across_median[index]);
    // длина улицы у начала каждого way — по ней идут штрихи краски
    let stations = paint::street_stations(&map.network, paths);
    // разделённая улица, сходящаяся в обычную: узел не перекрёсток
    let merges = merges::merges(&drawn, paths, &nodes, &axes.pairs.runs);
    // Скругления кладутся раньше всех лент своего слоя: лента поверх кроет
    // скругление, а не наоборот, и разметка остаётся целой.
    let (kerb_returns, islands) = {
        let rounded: Vec<Option<&[Vec2]>> = drawn
            .iter()
            .zip(paths)
            .map(|(road, path)| (!road.carves_navmesh()).then_some(path.as_ref()))
            .collect();
        // со стороны второй половины тротуара нет — угла по нему тоже; кусок
        // пары может кончиться на пробу раньше узла
        let slack = 2.0 * network::pairs::PROBE_STEP;
        let paired = |road: usize, at: f32| {
            axes.pairs.runs[road]
                .iter()
                .find(|run| run.from - slack <= at && at <= run.to + slack)
                .map(|run| run.left)
        };
        (
            corners::kerb_returns(
                &drawn,
                &rounded,
                &nodes,
                sidewalks_of,
                paired,
                |road| {
                    tapers
                        .at(road)
                        .map(|end| end.map_or(0.0, |taper| taper.length))
                },
                |road, end| merges.is_merged(road, end),
                shape.corner_radius(),
            ),
            corners::small_islands(&drawn, &rounded, &nodes),
        )
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
    // кромки половин, сходящиеся к кромкам продолжения, — тоже до лент
    let mut merge_edges = 0;
    for merge in &merges.list {
        let bands = merges::merge_bands(
            merge,
            &drawn,
            paths,
            |half, partner| {
                axes.pairs.runs[half]
                    .iter()
                    .find(|run| run.partner == partner)
                    .map(|run| run.left)
            },
            sidewalks_of,
            shape.taper(),
        );
        for band in bands {
            streets.push_polygon(&band.asphalt, &[], ROAD_COLOR.to_linear());
            if let Some(outline) = &band.sidewalk {
                sidewalks.push_polygon(outline, &[], SIDEWALK_COLOR.to_linear());
            }
            merge_edges += 1;
        }
    }
    // карманы — по тому же ответу и тем же разрывам, что ряд машин
    // (`map::cars`): асфальт за кромкой и тротуар, отодвинутый за него
    let row_breaks = pockets::row_breaks(roads, &map.network, &map.road_nodes, shape.taper());
    let mut kerb_pockets = 0;
    for (index, road) in roads.iter().enumerate() {
        if !pockets::parkable(road) {
            continue;
        }
        let half = road.width / 2.0;
        let sidewalk = sidewalks_of(index);
        for kerbside in pockets::kerbsides(
            road,
            &paths[index],
            &row_breaks.breaks[index],
            map.traffic_side,
        ) {
            let sidewalk = sidewalk.filter(|_| road.sidewalks[usize::from(kerbside.side < 0.0)]);
            for pocket in &kerbside.pockets {
                let outline = |outer: f32| {
                    pockets::outline(
                        &paths[index],
                        pocket,
                        kerbside.side,
                        [half - POCKET_OVERLAP, outer],
                    )
                };
                let edge = half + pockets::POCKET_WIDTH;
                streets.push_polygon(&outline(edge), &[], ROAD_COLOR.to_linear());
                if let Some(sidewalk) = sidewalk {
                    sidewalks.push_polygon(
                        &outline(edge + sidewalk),
                        &[],
                        SIDEWALK_COLOR.to_linear(),
                    );
                }
                kerb_pockets += 1;
            }
        }
    }
    // разворотные площадки в тупиках (`highway=turning_circle`); тупик без
    // тега кончается круглым торцом ленты и так
    let mut turning_circles = 0;
    for node in &map.road_nodes {
        if node.kind != RoadNodeKind::TurningCircle || nodes.is_shared(node.pos) {
            continue;
        }
        let Some(index) = roads.iter().position(|road| {
            road.class == RoadClass::Street
                && !road.carves_navmesh()
                && [road.points.first(), road.points.last()]
                    .into_iter()
                    .flatten()
                    .any(|end| end.distance(node.pos) < JOIN_EPSILON)
        }) else {
            continue;
        };
        let road = drawn[index];
        let radius = turning_radius(road.width);
        let disc = |radius: f32| -> Vec<Vec2> {
            (0..TURNING_CIRCLE_SIDES)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / TURNING_CIRCLE_SIDES as f32;
                    node.pos + Vec2::from_angle(angle) * radius
                })
                .collect()
        };
        streets.push_convex(&disc(radius), ROAD_COLOR.to_linear());
        if let Some(sidewalk) = sidewalks_of(index) {
            sidewalks.push_convex(&disc(radius + sidewalk), SIDEWALK_COLOR.to_linear());
        }
        turning_circles += 1;
    }
    // стежок до дороги, до которой OSM торец не довёл (`roads/network.rs`)
    let stitched: Vec<Cow<[Vec2]>> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            if stitches.touches(index) {
                let mut points = path.to_vec();
                stitches.apply(index, &mut points);
                Cow::Owned(points)
            } else {
                Cow::Borrowed(path.as_ref())
            }
        })
        .collect();
    // краска узлов (`roads/node_paint.rs`): где линии рвутся, а где главная
    // проходит узел, зебры, стоп-линии, карманы — по той же оси, что и линии.
    // Строится и без разметки: ведущая дорога узла и плечи для траекторий —
    // это колея асфальта, а не краска
    let mut node_paint = node_paint::NodePaint::new(
        &drawn,
        &stitched,
        &junctions.breaks,
        &stitches.targets,
        map,
        &islands,
        node_paint::NodePaintStyle {
            crossings: if style.markings {
                style.crossings
            } else {
                CrossingMode::Off
            },
            stop_lines: style.markings && style.stop_lines,
        },
        // тротуар по тегу (`sidewalk=*`), а не по ручке «Sidewalks»: ручка
        // прячет ленту, а зебра по правилу — вопрос модели, как карман у
        // `pockets::kerb_parking`; кусок поперечной в проёме пары — нет
        |index| {
            sidewalk_width(drawn[index]).is_some()
                && drawn[index].sidewalks.contains(&true)
                && !across_median[index]
        },
        |index| {
            axes.pairs.runs[index]
                .iter()
                .map(|run| node_paint::Partner {
                    road: run.partner,
                    paved: run.paved,
                })
                .collect()
        },
        |road| axes.rings.of(road).is_some(),
    );
    // траектории манёвров (`roads/turns.rs`) — колея в узле
    let turns = turns::Turns::new(
        &drawn,
        &stitched,
        &node_paint.junctions,
        map.traffic_side,
        |road| axes.rings.of(road).is_some(),
    );
    // Широкие улицы поверх узких — см. доку модуля; ведущая узла — поверх
    // всех: её колея идёт через узел, и примыкание шире неё не должно её
    // закрыть.
    let mut leading = vec![false; roads.len()];
    for junction in &node_paint.junctions {
        for &road in &junction.leading {
            leading[road] = true;
        }
    }
    let widths: Vec<f32> = drawn.iter().map(|road| road.width).collect();
    let order = fill_order(&widths, &leading, &node_paint.junctions);
    // направляющие островки у колец (`roads/gores.rs`) — до лент: к ним
    // дотягиваются двойные сплошные разделительных
    let gore_roads: Vec<gores::GoreRoad> = order
        .iter()
        .filter(|&&index| {
            let road = drawn[index];
            road.class == RoadClass::Street && !road.carves_navmesh()
        })
        .map(|&index| gores::GoreRoad::new(drawn[index], &stitched[index]))
        .collect();
    let mut gores = gores::Gores::of(&gore_roads);
    // островки по правилу — на двусторонних подходах, где веера из въезда и
    // съезда в OSM нет: краска и колея подхода рвутся на их длину
    let splitters = gores::splitters(&drawn, &stitched, &axes.rings);
    for splitter in &splitters {
        node_paint.breaks[splitter.road].push(splitter.gap);
        node_paint.asphalt[splitter.road].push(splitter.gap);
    }
    gores.add_splitters(&splitters);
    // разделительные парных половин (`roads/medians.rs`): асфальт — до лент
    // половин, под ними; газон с бордюром — в свой слой над тротуарами
    let mut median_grass = MeshBuilder::with_surface_coords();
    let mut paved: Vec<network::pairs::Median> = Vec::new();
    let mut lawn_kerbs = Vec::new();
    // торцы трамвайных полотен — разрывы для газона рядом: полотно и газон
    // одной пары улиц встречаются торец в торец
    let bed_ends: Vec<Break> = axes
        .pairs
        .medians
        .iter()
        .filter(|median| median.carries_tram())
        .flat_map(medians::bed_ends)
        .flatten()
        .collect();
    for median in &axes.pairs.medians {
        let [first, second] = median.roads;
        let breaks = medians::crossing_breaks(
            median,
            [&junctions.breaks[first], &junctions.breaks[second]],
        );
        // до перекрёстка — как линии полос, а не там, где кончились пробы
        let mut median = median.clone();
        medians::reach_breaks(&mut median, &breaks);
        if median.is_paved() {
            // полотно — внутренние полосы половин до середины; узкая
            // разделительная — полосой асфальта во всё расстояние между осями
            if median.carries_tram() {
                medians::push_bed(&mut streets, &median, ROAD_COLOR.to_linear());
            } else {
                medians::push_paved(&mut streets, &median, ROAD_COLOR.to_linear(), ROAD_JOIN);
            }
            if style.markings {
                let mut midline = median.midline.clone();
                gores.reach(&mut midline);
                // и там, где обе половины рвёт краска узла — зебра поперёк
                // обеих, стоп-линии
                let mut painted = breaks.clone();
                painted.extend(medians::crossing_breaks(
                    &median,
                    [&node_paint.breaks[first], &node_paint.breaks[second]],
                ));
                painter.paint_median(&midline, &painted);
            }
            paved.push(median);
        } else {
            let mut breaks = breaks;
            breaks.extend(bed_ends.iter().copied());
            lawn_kerbs.extend(medians::push_lawn(
                &mut sidewalks,
                &mut median_grass,
                &median,
                &breaks,
                SIDEWALK_COLOR.to_linear(),
                GRASS_COLOR.to_linear(),
            ));
        }
    }
    // асфальт от торца полотна до носа газона рядом
    if !lawn_kerbs.is_empty() {
        streets.set_lanes(None);
        for bed in paved.iter().filter(|median| median.carries_tram()) {
            for cap in medians::bed_caps(bed, &lawn_kerbs) {
                push_shape(&mut streets, cap, ROAD_COLOR.to_linear());
            }
        }
    }
    let network_time = started.elapsed();

    // щель между подходом и кольцом — асфальтом, под лентами
    for web in &axes.rings.webs {
        streets.push_polygon(web, &[], ROAD_COLOR.to_linear());
    }
    // остров-крошка в треугольнике узлов — тоже
    for island in &islands {
        streets.push_polygon(island, &[], ROAD_COLOR.to_linear());
    }
    if style.sidewalks {
        for ring in &axes.rings.list {
            let width = drawn[ring.roads[0]].width;
            let sidewalk = ring
                .roads
                .iter()
                .filter_map(|&road| sidewalks_of(road))
                .fold(0.0, f32::max);
            push_ring_edges(
                &mut sidewalks,
                ring,
                [width, sidewalk],
                SIDEWALK_COLOR.to_linear(),
            );
        }
    }
    for index in order {
        let road = drawn[index];
        let color = match road.class {
            RoadClass::Street => ROAD_COLOR,
            RoadClass::Alley => ALLEY_COLOR,
        };
        let points: &[Vec2] = &stitched[index];
        // колея гаснет по разрывам асфальта; у ведущей узла их там нет
        let breaks = node_paint.asphalt[index].as_slice();
        let lanes = road_lanes(road);
        // линии краски — по той же оси, разрывам и клиньям, что и асфальт
        if style.markings {
            let wedges = if road.bridge {
                [None; 2]
            } else {
                paint::wedge_ends(points, &tapers, &drawn, index, map.traffic_side)
            };
            painter.paint(
                road,
                points,
                paint::LineBreaks {
                    cut: &node_paint.breaks[index],
                    solid: &node_paint.solid[index],
                },
                wedges,
                node_paint.pockets[index],
                stations[index],
            );
        }
        if road.bridge {
            // бордюр настила — он и есть мост
            push_bridge_curb(
                &mut bridge_casings,
                points,
                2.0 * road.curb_reach(),
                ROAD_JOIN,
            );
            // Тень настила — тот же настил, сдвинутый по свету на высоту
            // моста. Ни один другой слой её не даёт: наземные тени считают
            // только дома, а мост через Упу — самая заметная вещь на воде.
            if let Some(deck) = bridges.span(index).filter(|deck| deck.casts) {
                shadow_bands.push(ShadowBand {
                    path: bridge_shadow_path(points, deck),
                    reach: road.curb_reach(),
                    penumbra: bridge_penumbra(deck.span),
                });
            }
            bridge_fills.set_lanes(lanes);
            push_street_fill(
                &mut bridge_fills,
                points,
                road.width,
                color.to_linear(),
                breaks,
                [false; 2],
            );
            continue;
        }
        let fill = match road.class {
            RoadClass::Street => &mut streets,
            RoadClass::Alley => &mut alleys,
        };
        // клинья у швов со сменой сечения: торцы, срезанные под них, и сами
        // клинья от ширины узкого соседа (`roads/tapers.rs`)
        let ends = tapers.at(index);
        let [head, body, tail] = if ends == [None; 2] {
            [None, None, None]
        } else {
            tapers::split(points, ends.map(|end| end.map(|taper| taper.length)))
        };
        let body: &[Vec2] = body.as_deref().unwrap_or(points);
        // торец под клин и торец плеча, кончающегося в узле, — прямые: узел
        // закрывают скругления и наружные углы (`roads/corners.rs`); торец
        // со стежком уже не в узле
        let butt = kerb_returns.butt(index);
        let stitched_end = stitches.ends[index].map(|end| end.is_some());
        let trimmed = [
            head.is_some() || (butt[0] && !stitched_end[0]),
            tail.is_some() || (butt[1] && !stitched_end[1]),
        ];
        let wedges: Vec<(&[Vec2], &RoadLine, bool)> =
            [(&head, ends[0], false), (&tail, ends[1], true)]
                .into_iter()
                .filter_map(|(path, taper, end)| {
                    Some((path.as_deref()?, drawn[taper?.narrow], end))
                })
                .collect();
        // «до разрыва» клина — продолжение срезанной ленты за её торцом: от
        // узла шва до стыка с лентой, чтобы штрихи шли через стык без сдвига
        let continued = |path: &[Vec2], width: f32, breaks: &[Break], end: bool| {
            let length = polyline_length(path);
            [
                to_break_beyond(body, width, breaks, end, length),
                to_break_beyond(body, width, breaks, end, 0.0),
            ]
        };

        // тротуар кольца — одной лентой на всё кольцо (`push_ring_edges`)
        let ring = axes.rings.of(index);
        if let Some(sidewalk) = sidewalks_of(index).filter(|_| ring.is_none()) {
            let band = |road: &RoadLine, sidewalk: f32| road.width + 2.0 * sidewalk;
            // у половины разделённой улицы тротуара со стороны пары нет; на
            // клине куски пары не пересчитываются — там тротуар как был
            let runs = if wedges.is_empty() {
                axes.pairs.runs[index].as_slice()
            } else {
                &[]
            };
            let stitch =
                stitches.ends[index][0].map_or(0.0, |start| start.distance(paths[index][0]));
            push_sidewalk(
                &mut sidewalks,
                body,
                [road.width, sidewalk],
                runs,
                stitch,
                road.sidewalks,
                SIDEWALK_COLOR.to_linear(),
                trimmed,
            );
            // клин тротуара симметричен; у одностороннего тротуара его нет
            let wedges = if road.sidewalks == [true; 2] {
                wedges.as_slice()
            } else {
                &[]
            };
            for &(path, narrow, end) in wedges {
                let from =
                    drawn_sidewalk(&style, narrow).map_or(narrow.width, |own| band(narrow, own));
                let to = band(road, sidewalk);
                sidewalks.push_taper(
                    path,
                    [from, to],
                    continued(path, to, &[], end),
                    SIDEWALK_COLOR.to_linear(),
                );
            }
        }
        // Клин половины разделённой улицы сужается и со стороны пары, а
        // разделительная считана по полной ширине: в щели между ними лежал
        // полный тротуар половины — светлая полоса с тёмной кромкой вдоль
        // всего клина (пример 16). Со стороны пары под клин кладётся асфальт
        // полной полуширины — кромка там идёт прямо, как у тела.
        let length = polyline_length(points);
        for &(path, _, end) in &wedges {
            let middle = if end {
                length - polyline_length(path) / 2.0
            } else {
                polyline_length(path) / 2.0
            };
            let Some(run) = axes.pairs.runs[index]
                .iter()
                .find(|run| (run.from..=run.to).contains(&middle))
            else {
                continue;
            };
            let side = if run.left { 1.0 } else { -1.0 };
            let inner: Vec<Vec2> = path
                .iter()
                .zip(miter_offsets(path, false, road.width / 4.0))
                .map(|(&point, offset)| point + offset * side)
                .collect();
            fill.set_lanes(None);
            push_ribbon_trimmed(
                fill,
                &inner,
                road.width / 2.0,
                color.to_linear(),
                ROAD_JOIN,
                [true; 2],
            );
        }
        fill.set_lanes(lanes);
        push_street_fill(fill, body, road.width, color.to_linear(), breaks, trimmed);
        for &(path, narrow, end) in &wedges {
            let to_break = continued(path, road.width, breaks, end);
            // раскладка плывёт от сечения соседа к своему — та же, что у
            // линий краски на этом клине
            match lanes {
                Some(_) => {
                    let [from, to] = paint::wedge_frames(
                        lane_count(road),
                        lane_count(narrow),
                        end,
                        paint::wedge_drift(road, map.traffic_side),
                    );
                    fill.set_lane_taper(Some(from), Some(to));
                }
                None => fill.set_lanes(None),
            }
            fill.push_taper(
                path,
                [narrow.width, road.width],
                to_break,
                color.to_linear(),
            );
        }
        if road.class == RoadClass::Street && !road.passage {
            grounds.push(road, points);
        }
    }
    for zebra in &node_paint.zebras {
        painter.paint_zebra(zebra);
    }
    for line in &node_paint.stop_lines {
        painter.paint_stop_line(line);
    }
    // колея траекторий — всегда, как колея полос
    painter.paint_turn_wear(&turns.wear);
    // стрелки на полосах подходов — краска, своим тумблером
    if style.arrows {
        let marks = paint::ArrowMarks::new(&node_paint.zebras, &node_paint.stop_lines);
        for arrow in &turns.arrows {
            let setback = paint::Painter::arrow_setback(arrow, &marks);
            painter.paint_arrow(arrow, setback);
            // и второй ряд дальше от узла, где полоса это позволяет
            let breaks = &node_paint.breaks[arrow.road];
            if let Some(repeat) = paint::Painter::repeat_setback(arrow, setback, &marks, breaks) {
                painter.paint_arrow(arrow, repeat);
            }
        }
    }
    // направляющие островки у колец: асфальт — в слой улиц, поверх тротуаров,
    // разметка — в слой краски, своим мешем выше асфальта стоянок
    // (`roads/gores.rs`)
    gores.push_asphalt(&mut streets, ROAD_COLOR.to_linear());
    // островки безопасности и площади полотна из данных (`roads/islands.rs`):
    // площадь — асфальтом улиц, островок — бордюром поверх асфальта и краски
    let road_islands = islands::RoadIslands::new(map, &drawn, &stitched);
    streets.set_lanes(None);
    for shape in &road_islands.carriageways {
        push_shape(&mut streets, shape.clone(), ROAD_COLOR.to_linear());
    }
    // светлая полоса над рельсами (`roads/tram_band.rs`) — поверх всего
    // асфальта улиц: порядок пуша в слое — порядок отрисовки, а краска лежит
    // своим слоем выше
    let tram_bands = tram_band::tram_bands(&map.rails, &drawn, paths, &paved);
    for band in &tram_bands {
        push_ribbon_trimmed(
            &mut streets,
            band,
            tram_band::TRAM_BAND_WIDTH,
            TRAM_BAND_COLOR.to_linear(),
            ROAD_JOIN,
            [true; 2],
        );
    }
    let mut lot_layers = grounds.layers(&style, &gores, &paved);
    for shape in &road_islands.kerbs {
        push_shape(
            &mut lot_layers.sidewalks,
            shape.clone(),
            SIDEWALK_COLOR.to_linear(),
        );
    }
    if style.markings {
        for (island, across) in gores.islands() {
            painter.paint_island(island, across);
        }
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
                ROAD_JOIN,
            );
        }
    }

    // тень моста полупрозрачна, поэтому у неё `Blend`: непрозрачный материал
    // съел бы альфу вершинного цвета. Асфальт, тротуар и дорожка — фактурные,
    // канты и лента стены — плоские
    let mut layers: Vec<LayerMesh> = [
        (
            alleys,
            Z_ALLEY,
            "alleys",
            MaterialSpec::Surface(SurfaceKind::Alley),
        ),
        (
            sidewalks,
            Z_SIDEWALK,
            "sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            median_grass,
            Z_ROAD_MEDIAN,
            "road_medians",
            MaterialSpec::Surface(SurfaceKind::Grass),
        ),
        (
            streets,
            Z_ROAD,
            "roads",
            MaterialSpec::Surface(SurfaceKind::Street),
        ),
        (
            lot_layers.sidewalks,
            Z_LOT_SIDEWALK,
            "lot_sidewalks",
            MaterialSpec::Surface(SurfaceKind::Sidewalk),
        ),
        (
            lot_layers.lines,
            Z_LOT_LINES,
            "lot_lines",
            MaterialSpec::Flat,
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
    let paint_lines = painter.lines;
    let paint_layers = painter.layers();
    let paint_vertices = paint_layers
        .iter()
        .map(|layer| layer.builder.vertex_count())
        .sum();
    // краска — сразу над своим асфальтом: улиц над улицами, мостов над
    // настилом; сортировка устойчива, прочие слои порядка не меняют
    layers.extend(paint_layers);
    layers.sort_by(|a, b| a.z.total_cmp(&b.z));

    let report = RoadReport {
        style,
        junctions: junctions.junctions,
        paint_lines,
        paint_vertices,
        zebras: [
            node_paint.zebras.len(),
            node_paint.zebras.iter().filter(|zebra| zebra.osm).count(),
        ],
        stop_lines: node_paint.stop_lines.len(),
        pockets: node_paint.pockets.iter().flatten().flatten().count(),
        clusters: node_paint.clusters,
        kerb_pockets,
        turning_circles,
        through: node_paint.through,
        turns: turns.maneuvers,
        arrows: if style.arrows { turns.arrows.len() } else { 0 },
        leading: leading.iter().filter(|&&lead| lead).count(),
        kerb_returns: kerb_returns.roads.len() - kerb_returns.outer[0],
        sidewalk_returns: kerb_returns.sidewalks.len() - kerb_returns.outer[1],
        outer_corners: kerb_returns.outer,
        stitches: stitches.count,
        crossings: crossings.len(),
        gores: gores.count(),
        road_islands: [
            road_islands.refuges,
            road_islands.kerbs.len() - road_islands.refuges,
            road_islands.carriageways.len(),
        ],
        tapers: tapers.count,
        merges: [merges.list.len(), merge_edges],
        rings: [axes.rings.list.len(), axes.rings.webs.len()],
        islands: islands.len(),
        medians: axes.pairs.count(),
        tram_bands: tram_bands.len(),
        seams: axes.seams,
        tight: axes.tight,
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
    let (layers, report) = mesh_roads(map, RoadStyle::default(), RoadShape::default());
    // счётчики сети (стыки, скругления, островки) в строках слоёв не видны —
    // та же строка, что пишет в лог игра
    eprintln!("{report}");
    surface::layer_costs(&layers, report.elapsed)
}

/// Когда пересобирать дорожные слои — **условие живёт рядом со слоем**, а не у
/// того, кто ставит систему в расписание: причина тут дорожная, и узнать её
/// надо, правя `roads.rs`, а не `map/mod.rs`.
///
/// `SunOnMap` в списке потому, что **в дорожный меш запечена тень моста**:
/// настил, сдвинутый по `shadow_dir()` на высоту пролёта через
/// `shadow_length_scale()`. Без этого условия она осталась бы от солнца, с
/// которым грузился город, пока все остальные тени карты едут за осевшим.
/// Осевшим (`SunOnMap`), а не ползунком (`SunStyle`): на шкале семьдесят
/// делений, и каждое стоило бы полной пересборки девяти слоёв.
///
/// **Условие одно, регистрация одна.** Две копии одной системы в одном
/// расписании могут сработать в одном кадре обе, и слой заспавнится дважды:
/// деспавн второй копии идёт по данным, снятым до применения команд первой.
/// Поэтому условия складываются через `or_else`, а не разносятся по
/// регистрациям.
pub fn rebuilds_on() -> impl SystemCondition<()> {
    retuned::<RoadStyle>
        .or_else(retuned::<RoadShapeOnMap>)
        .or_else(retuned::<SunOnMap>)
}

/// Пересборка дорожных слоёв после переключения стиля из UI или BRP: деспавн
/// старых слоёв и повторный спавн из той же `MapData`.
pub fn rebuild_roads(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    style: Res<RoadStyle>,
    shape: Res<RoadShapeOnMap>,
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
        mesh_roads(&map, *style, shape.0),
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
    // меши краски несут ещё и свой вид — по нему их прячет ступень зума
    // (`paint::show_paint`)
    let (paint, rest): (Vec<LayerMesh>, Vec<LayerMesh>) = layers
        .into_iter()
        .partition(|layer| paint::PaintTag::of(layer.name).is_some());
    spawn_layers(commands, meshes, materials, rest, RoadLayerTag);
    for layer in paint {
        let Some(tag) = paint::PaintTag::of(layer.name) else {
            continue;
        };
        spawn_layers(commands, meshes, materials, [layer], (RoadLayerTag, tag));
    }
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
/// (`buildings::shadows::PENUMBRA_WIDTH`, мягче не бывает вовсе). Между ними
/// доля и работает: пролёт 16 м — 0.36 м каймы, 40 м — 0.9, от 44 м и выше —
/// потолок. Концы у солнца не едут (это константы соседних слоёв), а сама
/// доля едет: на низком солнце тень длиннее, и край у неё мягче.
const PENUMBRA_SHARE: f32 = 0.3;
const PENUMBRA_MIN: f32 = 0.35;
const PENUMBRA_MAX: f32 = 1.0;

/// Ширина полутени для моста с таким пролётом — см. [`PENUMBRA_SHARE`].
fn bridge_penumbra(span: f32) -> f32 {
    let length = shadow::length(bridge_height(span));
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

    // Ядра лент для проверки погружения каймы — с габаритом на каждое.
    // Проба идёт на каждый квад каймы (лента уплотнена до `SHADOW_STEP`, так
    // что у моста через Упу их под сотню) и без габарита обходила бы кольца
    // **всех** мостов города: пересекаются же считанные пары, соседи по
    // одному настилу. Отсечка — та же, что у `Underneath` в `probe_underneath`
    // и у `Fortresses`. Пустое ядро даёт `(INFINITY, NEG_INFINITY)`, то есть
    // габарит, в который не попадает ничто, — как и надо.
    let cores: Vec<(Vec2, Vec2, Vec<Vec2>)> = edges
        .iter()
        .map(|band| {
            let ring: Vec<Vec2> = if band.len() < 2 {
                Vec::new()
            } else {
                band.iter()
                    .map(|edge| edge.left)
                    .chain(band.iter().rev().map(|edge| edge.right))
                    .collect()
            };
            let (low, high) = ring_bounds(&ring);
            (low, high, ring)
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
                let buried = cores.iter().enumerate().any(|(other, (low, high, ring))| {
                    other != own
                        && lip.cmpge(*low).all()
                        && lip.cmple(*high).all()
                        && point_in_polygon(lip, ring)
                });
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
///
/// Нормаль берётся готовой из [`ShadowPoint`] — она поперёк **настила**, а не
/// поперёк съехавшей ленты; почему это не одно и то же, написано там же.
fn shadow_edges(band: &ShadowBand) -> Vec<ShadowEdge> {
    if band.path.len() < 2 {
        return Vec::new();
    }
    band.path
        .iter()
        .map(|point| {
            // единичную нормаль стыка растягивает своя полуширина
            let half = band.reach + SHADOW_SPREAD * point.rise;
            ShadowEdge {
                left: point.at + point.normal * half,
                right: point.at - point.normal * half,
                normal: point.normal,
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

/// Лента выбранного стиля — `RoadJoin` через [`RoadJoin::ribbon_shape`]. Общая
/// с подложкой аллей (`map::spawn`): у неё те же три настройки, что у дорог, и
/// мапиться на `MeshBuilder` они обязаны одинаково.
///
/// Слои с **жёстко заданным** стыком (ограда, рельсы, трамвай) зовут
/// `MeshBuilder::push_ribbon` напрямую: обёртка им говорила бы только «переведи
/// `RoadJoin::Round` в `RibbonJoin::Round`», то есть выдавала бы стиль дорог за
/// их собственный.
///
/// **Замкнутый путь рисуется замкнутой лентой** — признак берётся из самого
/// пути ([`is_ring`]), а не аргументом: у кольца торцов нет, и решать это за
/// ленту нечем, кроме её же формы. Иначе на шве ложились два торцевых
/// полудиска поверх собственного асфальта кольца, и внутри такого диска
/// разметка с износом считались в **замороженной** раме торца — круглое пятно
/// со смещёнными штрихами (отчёт автора по кольцу ТРЦ «Макси»).
pub fn push_ribbon(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
) {
    push_ribbon_trimmed(builder, points, width, color, join, [false; 2]);
}

/// Лента дороги, у которой торец `[начало, конец]` может быть **срезан под
/// клин** (`roads/tapers.rs`): такой торец кончается ровно на своей точке.
/// Круглый торец полной ширины выпер бы из-под клина, который в этом месте
/// начинается с той же ширины и дальше только сужается.
fn push_ribbon_trimmed(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    join: RoadJoin,
    trimmed: [bool; 2],
) {
    let Some((join, cap)) = join.ribbon_shape() else {
        return builder.push_polyline(points, width, color);
    };
    let caps = trimmed.map(|trimmed| if trimmed { RibbonCap::Butt } else { cap });
    builder.push_ribbon_capped(points, is_ring(points), width, color, join, caps);
}

/// Тротуар дороги шириной `widths[0]` с полосой `widths[1]` — с тех сторон
/// `[слева, справа]`, где он есть по тегу (`sides`, [`RoadLine::sidewalks`]),
/// и кроме кусков `runs`, где рядом идёт вторая половина разделённой улицы
/// (`roads/network/pairs.rs`): со стороны пары его нет. Длины кусков меряны
/// по оси без стежка; `stitch` — длина стежка перед её началом.
#[allow(clippy::too_many_arguments)]
fn push_sidewalk(
    builder: &mut MeshBuilder,
    body: &[Vec2],
    [width, sidewalk]: [f32; 2],
    runs: &[network::pairs::PairRun],
    stitch: f32,
    sides: [bool; 2],
    color: LinearRgba,
    trimmed: [bool; 2],
) {
    if runs.is_empty() && sides == [true; 2] {
        return push_ribbon_trimmed(
            builder,
            body,
            width + 2.0 * sidewalk,
            color,
            ROAD_JOIN,
            trimmed,
        );
    }
    let total = polyline_length(body);
    let mut cursor = 0.0;
    // кусок `from..to` с тротуаром по сторонам `[слева, справа]`
    let mut piece = |from: f32, to: f32, [left, right]: [bool; 2]| {
        if to - from < SIDEWALK_PIECE_MIN {
            return;
        }
        let points = tapers::cut(body, from, to);
        let trims = [from > 0.0 || trimmed[0], to < total || trimmed[1]];
        // полоса с одной стороны — лента на полтротуара в её сторону:
        // `miter_offsets` плюсом сдвигает влево
        let shift = match (left, right) {
            (true, true) => {
                return push_ribbon_trimmed(
                    builder,
                    &points,
                    width + 2.0 * sidewalk,
                    color,
                    ROAD_JOIN,
                    trims,
                );
            }
            (true, false) => sidewalk / 2.0,
            (false, true) => -sidewalk / 2.0,
            (false, false) => return,
        };
        let shifted: Vec<Vec2> = points
            .iter()
            .zip(miter_offsets(&points, false, shift))
            .map(|(point, offset)| *point + offset)
            .collect();
        push_ribbon_trimmed(builder, &shifted, width + sidewalk, color, ROAD_JOIN, trims);
    };
    let mut previous: Option<bool> = None;
    for run in runs {
        let from = (run.from + stitch).clamp(cursor, total);
        let to = (run.to + stitch).clamp(from, total);
        // со стороны пары тротуара нет
        let mut paired = sides;
        paired[usize::from(!run.left)] = false;
        // и в щели между двумя кусками с той же стороны — полотном и газоном
        // одной пары, — которые разделительные сводят торец в торец
        // (`Pairs::join_ends`): светлое пятно тротуара лежало между ними
        let bridged = previous == Some(run.left) && from - cursor < network::pairs::JOIN_GAP;
        piece(cursor, from, if bridged { paired } else { sides });
        piece(from, to, paired);
        cursor = to;
        previous = Some(run.left);
    }
    piece(cursor, total, sides);
}

/// Заливка проезжей части — лента [`ROAD_RIBBON`] с разрывами разметки по
/// перекрёсткам. Срезанный под клин торец — как у [`push_ribbon_trimmed`].
fn push_street_fill(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    color: LinearRgba,
    breaks: &[Break],
    trimmed: [bool; 2],
) {
    let (join, cap) = ROAD_RIBBON;
    builder.push_ribbon_shaped(
        points,
        width,
        color,
        RibbonShape {
            closed: is_ring(points),
            join,
            caps: trimmed.map(|trimmed| if trimmed { RibbonCap::Butt } else { cap }),
            breaks: RibbonBreaks::At(breaks),
        },
    );
}

/// Осевая дороги вне улицы: моста, арки, дорожки (улицы идут по
/// [`axis::street_axes`]). Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
///
/// **Общие узлы с другими дорогами тоже не сглаживаются** ([`RoadNodes`]): на
/// узле кончается поперечная улица и сходятся лучи скругления бордюра
/// (`roads/corners.rs`). Сдвинь хорда сквозную дорогу с узла — торец
/// поперечной повис бы в метре от её асфальта или вылез за дальний край.
///
/// **Замкнутый way сглаживается по циклу**, так что нарисованная ось кольца
/// остаётся кольцом: её потом и рисуют замкнутой лентой ([`push_ribbon`],
/// [`push_street_fill`]).
fn centerline<'a>(road: &'a RoadLine, smoothing: Smoothing, nodes: &RoadNodes) -> Cow<'a, [Vec2]> {
    if road.passage {
        return Cow::Borrowed(&road.points);
    }
    smooth_pinned(
        &road.points,
        road.width,
        smoothing,
        is_ring(&road.points),
        |point| nodes.is_shared(point),
    )
}

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин обязан
/// рваться на тех же перекрёстках, на которых рвётся разметка, и второго
/// восстановления узлов по общим нодам заводить незачем.
pub(super) mod junctions;

/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин стоит на той
/// же оси, что и лента.
pub(super) mod axis;
mod corners;
mod gores;
mod islands;
mod lots;
mod medians;
mod merges;
/// Открыт наружу для [`map::footprint`](crate::map::footprint): проём в ограде
/// у брошенного торца — тот же вопрос «висячий ли он», что у стежка, и второго
/// ответа на него быть не должно. И для разбора: улицы и сечения
/// (`network::sections`) собираются там, потому что ширину дороги читают
/// следующие проходы разбора, — а сеть потом лежит в `MapData::network`.
pub mod network;
pub mod node_paint;
pub mod paint;
/// Открыт наружу для [`map::cars`](crate::map::cars): машина встаёт в тот же
/// карман, что кладёт лента.
pub(super) mod pockets;
mod rings;
/// Открыт наружу для панели и витрины: ресурс ручек формы и глобаль ширины
/// полосы.
pub mod shape;
/// Открыт наружу для [`map::cars`](crate::map::cars): ряд машин прерывается
/// на клине, где бордюр ближе к оси, чем полуширина участка.
pub(super) mod tapers;
mod tram_band;
mod turns;

#[cfg(test)]
mod tests;
