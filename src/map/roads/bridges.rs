//! Мосты: настил (`RoadLine::bridge`) уходит из слоёв своего класса в тройку
//! `bridge_shadows` + `bridge_casings` + `bridges`. Здесь — то, что у моста
//! своё: цепочки мостовых ways ([`Bridges`]), бордюр настила
//! ([`push_bridge_curb`]) и его тень ([`push_bridge_shadows`]). Заливку
//! настила кладёт `mesh_roads` в порядке заливки улиц — с рамой полос и
//! разрывами асфальта своей улицы.

use bevy::prelude::*;

use super::{ROAD_JOIN, RoadJoin, smoothstep};
use crate::map::SHADOW_COLOR;
use crate::map::along::densify;
use crate::map::footprint::JOIN_EPSILON;
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin, merge_close_points, miter_offsets};
use crate::map::osm::model::{
    distance_to_segment, point_in_area, point_in_polygon, polyline_length, ring_bounds,
};
use crate::map::osm::{MapData, PolyArea, RoadLine};
use crate::map::shadow;
use crate::map::surface::{LayerMesh, MaterialSpec, SurfaceKind};
use crate::settings::{Z_BRIDGE, Z_BRIDGE_CASING, Z_BRIDGE_SHADOW};

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
            // плавно, а не изломом: у настоящей насыпи профиль сглажен
            let rise = smoothstep(behind.min(ahead) / ramp);
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
///
/// Он же и копит мостовые слои: бордюры и теневые ленты кладёт
/// [`Self::push_deck`], а заливку настила — вызывающий в [`Self::fills`],
/// из цикла порядка заливки улиц. Мост обязан остаться в этом порядке: у
/// заливки рама полос и разрывы асфальта своей улицы, а мост над мостом —
/// это порядок пуша.
pub(super) struct Bridges {
    /// На индекс дороги; `None` — не мост.
    spans: Vec<Option<BridgeSpan>>,
    count: BridgeReport,
    /// Настилы — один меш на улицы и пешеходные мостики разом: белая и
    /// песочная заливки соседствуют, и порядок перекрытия моста над мостом —
    /// порядок пуша. Мост над мостом — редкость, четыре слоя ради него не
    /// нужны.
    casings: MeshBuilder,
    fills: MeshBuilder,
    /// Тень моста — на то, над чем он проходит: воду, дорогу, пути. Ленты
    /// копятся и кладутся разом: их ядра объединяются
    /// ([`push_bridge_shadows`]).
    shadows: Vec<ShadowBand>,
}

/// Мосты карты счётом: мостовых ways, мостов из них (связных цепочек — см.
/// [`Bridges`]) и мостов, отбрасывающих тень. Кеш v15: Тула — 91 way, 86
/// мостов, 73 с тенью; Берлин — 429, 325, 278.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BridgeReport {
    pub ways: usize,
    pub bridges: usize,
    pub casting: usize,
}

impl Bridges {
    pub(super) fn new(map: &MapData) -> Self {
        let mut spans = vec![None; map.roads.len()];
        let decks: Vec<usize> = map
            .roads
            .iter()
            .enumerate()
            .filter(|(_, road)| road.bridge && road.points.len() >= 2)
            .map(|(index, _)| index)
            .collect();
        let finish = |spans, count| Self {
            spans,
            count,
            casings: MeshBuilder::default(),
            fills: MeshBuilder::with_surface_coords(),
            shadows: Vec::new(),
        };
        if decks.is_empty() {
            return finish(spans, BridgeReport::default());
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
        let mut chains: Vec<usize> = ends.iter().map(|[first, _]| roots[*first]).collect();
        chains.sort_unstable();
        chains.dedup();
        let count = BridgeReport {
            ways: decks.len(),
            bridges: chains.len(),
            casting: chains.iter().filter(|&&root| casts[root]).count(),
        };
        finish(spans, count)
    }

    fn span(&self, road: usize) -> Option<&BridgeSpan> {
        self.spans[road].as_ref()
    }

    /// Бордюр и тень одного настила. Заливку с рамой полос и разрывами
    /// асфальта своей улицы кладёт вызывающий в [`Self::fills`] — так мост
    /// остаётся в порядке заливки улиц.
    pub(super) fn push_deck(&mut self, road: usize, points: &[Vec2], line: &RoadLine) {
        // бордюр настила — он и есть мост
        push_bridge_curb(
            &mut self.casings,
            points,
            2.0 * line.curb_reach(),
            ROAD_JOIN,
        );
        // Тень настила — тот же настил, сдвинутый по свету на высоту
        // моста. Ни один другой слой её не даёт: наземные тени считают
        // только дома, а мост через Упу — самая заметная вещь на воде.
        if let Some(deck) = self.span(road).copied().filter(|deck| deck.casts) {
            self.shadows.push(ShadowBand {
                path: bridge_shadow_path(points, &deck),
                reach: line.curb_reach(),
                penumbra: bridge_penumbra(deck.span),
            });
        }
    }

    /// Меш заливки настилов.
    pub(super) fn fills(&mut self) -> &mut MeshBuilder {
        &mut self.fills
    }

    pub(super) fn count(&self) -> BridgeReport {
        self.count
    }

    /// Три мостовых слоя снизу вверх: тени (ядра объединены здесь), бордюры,
    /// настилы. Тень полупрозрачна, поэтому у неё `Blend`: непрозрачный
    /// материал съел бы альфу вершинного цвета; настил — фактурный асфальт
    /// улиц, бордюр — плоский.
    pub(super) fn layers(self) -> [LayerMesh; 3] {
        let mut shadows = MeshBuilder::default();
        push_bridge_shadows(&mut shadows, &self.shadows);
        [
            LayerMesh::new(
                shadows,
                Z_BRIDGE_SHADOW,
                "bridge_shadows",
                MaterialSpec::Blend,
            ),
            LayerMesh::new(
                self.casings,
                Z_BRIDGE_CASING,
                "bridge_casings",
                MaterialSpec::Flat,
            ),
            LayerMesh::new(
                self.fills,
                Z_BRIDGE,
                "bridges",
                MaterialSpec::Surface(SurfaceKind::Street),
            ),
        ]
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

/// Бордюр моста — светлый бетонный парапет над серым настилом, общий для
/// улиц и пешеходных мостиков. Толщины (и почему их диапазоны не
/// пересекаются) — в `map::footprint`.
const BRIDGE_CURB_COLOR: Color = Color::srgb(0.80, 0.80, 0.79);

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
/// [`super::push_ribbon`]-обёртки.
fn push_bridge_curb(
    builder: &mut MeshBuilder,
    points: &[Vec2],
    width: f32,
    join: RoadJoin,
) {
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

#[cfg(test)]
mod tests;
