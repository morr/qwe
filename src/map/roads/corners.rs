//! Асфальт узла: скругления бордюра между дорогами, сходящимися в общем узле,
//! наружные углы узла и торцы плеч.
//!
//! Ленты дорог лежат внахлёст, и их края встречаются в перекрёстке прямым
//! (или острым) углом — так рисует osm-carto, но так не бывает на месте:
//! бордюр на повороте всегда скруглён, иначе машина не повернёт, не заехав на
//! тротуар. На снимке сверху именно эти дуги и делают перекрёсток
//! перекрёстком, а не крестом из двух полос.
//!
//! Скругление — «вогнутый треугольник» между краями двух соседних по углу
//! лучей узла и дугой, касательной к обоим краям. Кладётся **той же заливкой**
//! в тот же слой, что и сами дороги, и раньше них: любая лента поверх
//! скругления его кроет, так что разметка и чужие ленты остаются целы.
//! Скругляются только лучи одного класса — асфальтовая дуга между улицей и
//! пешеходной дорожкой легла бы серым поверх песочного.
//!
//! **Радиус — по младшему классу пары** ([`kerb_radius`]): между проспектами
//! 10 м, с улицей 6, с проездом 2.5. Раньше он шёл от ширин, и въезд узкой
//! дороги в широкую получал 0.4 её полуширины — у разделённого проспекта, где
//! в узле сходятся половины разной полосности, углы выходили в метр-два.
//!
//! **Плечо, кончающееся в узле, кончается прямым торцом** ([`KerbReturns::
//! butt`]): круглый торец широкой дороги, упёршейся в узкую, выпирал полудиском
//! за дальний край узкой. Прямой торец лежит на оси узла, а наружный угол
//! между плечами без сквозной дороги (гнутый угол двух улиц, развилка)
//! закругляется веером от узла ([`outer_corner`]) — там, где круглые торцы
//! давали это даром. Узел здесь — три плеча и больше или два, сходящиеся
//! углом; два почти соосных торца — продолжение дороги, и торцы у них круглые,
//! как были.
//!
//! Полигон узла объединением (`i_overlay`) не строится: куски лежат под
//! лентами своего слоя, и щели между ними лента закрывает сама, а объединение
//! на каждый из девяти тысяч узлов стоило бы сотни миллисекунд загрузки.
//!
//! **Тротуар поворачивает вместе с бордюром.** Полоса тротуара шире
//! проезжей части, и её собственный угол на перекрёстке оставался прямым:
//! асфальт выкатывался дугой в угол, срезая тротуар до нитки, а за ним торчал
//! прямоугольный уступ светлой полосы — на снимке это и видно. Тот же клин, но
//! в слое тротуаров, кладётся по краям полос (полуширина плюс ширина тротуара)
//! и дугой **того же центра**: радиус меньше ровно на ширину тротуара, и за
//! бордюром идёт постоянная полоса шириной в тротуар — как на месте. Радиус
//! меньше тротуара — угол и на месте прямой (въезд с малым радиусом), дуги
//! нет. Асфальтовый клин при этом всегда лежит на тротуарном: дуги
//! концентричны, и касательные у них общие, так что клин между краями дорог и
//! бордюрной дугой — внутри клина между краями тротуаров и тротуарной.

use std::f32::consts::PI;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::drawn::{Axis, Drawn};
use super::junctions::node_key;
use super::network::pairs::PROBE_STEP;
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::meshing::arc_steps;
use crate::map::osm::model::{closest_on_segment, polyline_length, ring_area};
use crate::map::osm::{Highway, RoadClass, RoadLine};

/// Радиус бордюра по классу дороги, м; у пары берётся меньший. Между
/// проспектами (`trunk`…`secondary` и их съезды) — 10 м, с улицей
/// (`tertiary`, жилая, `unclassified`) — 6, с проездом, жилой зоной или
/// переездом через тротуар — 2.5, между пешеходными дорожками — 2.
const MAJOR_RADIUS: f32 = 10.0;
const STREET_RADIUS: f32 = 6.0;
const DRIVE_RADIUS: f32 = 2.5;
const PATH_RADIUS: f32 = 2.0;
/// Скругление меньше этого не кладётся, м: его всё равно не видно.
const MIN_RADIUS: f32 = 0.5;
/// Угол между лучами, в котором скругление имеет смысл. Острее — дуга
/// вытягивается в длинный клин, и там кладётся нос ([`NOSE_SHARE`]); почти
/// развёрнутый угол — это продолжение дороги, а не поворот.
const MIN_ANGLE: f32 = 25.0 * PI / 180.0;
const MAX_ANGLE: f32 = 155.0 * PI / 180.0;
/// Острее [`MIN_ANGLE`] (и до [`NOSE_MAX_ANGLE`], где скругление не легло)
/// угол между лучами не скругляется, а получает **нос**:
/// дугу малого радиуса там, где кромки разошлись на два таких радиуса, — как
/// бордюр острия островка между расходящимися полотнами. Без него две кромки
/// сходились в математическое остриё, и между полотнами развилки торчал шип
/// тротуара и земли (Тула, витрины 04, 06, 14). Радиус — доля
/// [`NOSE_SHARE`] радиуса пары, не больше [`NOSE_RADIUS`]: улицы 1.5 м,
/// проезд 1, дорожки 0.8.
const NOSE_SHARE: f32 = 0.4;
const NOSE_RADIUS: f32 = 1.5;
/// Острее этого нос не кладётся: почти параллельные полотна — одна дорога
/// из двух way.
const NOSE_MIN_ANGLE: f32 = 2.0 * PI / 180.0;
/// Нос ложится и в угол до этого, когда скругление не легло: прямой край
/// луча короче касательной — проезд под 34° от секундарной с изломом в
/// 10 м от узла (Тула, витрина 06) оставлял тот же шип тротуара.
const NOSE_MAX_ANGLE: f32 = 60.0 * PI / 180.0;
/// Как далеко от узла ищется нос, м: у развилки под 8° (Тула, витрина 14)
/// кромки улиц в 8 м сходятся в 57 м от узла, и нос встаёт ещё на 20 м дальше.
const NOSE_REACH: f32 = 100.0;
/// Шаг, которым нос ищется вдоль первого луча, м.
const NOSE_STEP: f32 = 0.5;
/// В узкой части носа (просвет меньше радиуса) вершиной становится каждая
/// такая проба.
const NOSE_THIN: usize = 4;
/// Окно звеньев второго луча вокруг прошлой находки, в котором ищется
/// ближайшая к кромке первого точка.
const NOSE_WINDOW: usize = 8;
/// Сколько раз центр носа сдвигается к касанию обеих кромок.
const NOSE_SETTLE: usize = 8;
/// Наружный угол узла закругляется, когда просвет между плечами шире
/// развёрнутого хотя бы на столько: у сквозной дороги просветы ровно по
/// 180°, и веер там лёг бы под её же ленту. Порог — на шум округления, не
/// больше: улица из двух way с изломом в узле в 0.7° (Ложевая у Пролетарской,
/// Тула, витрина 08) при пороге в градус оставалась без угла, и между
/// прямыми торцами её плеч светлел клин в семь сантиметров.
const MIN_OUTER: f32 = 0.05 * PI / 180.0;
/// Луч меряет направление по звену не короче этого, м.
const MIN_ARM: f32 = 0.5;
/// Насколько вершина может отойти вбок от прямой луча и всё ещё продолжать
/// его прямой край, м.
const STRAIGHT_TOLERANCE: f32 = 0.15;
/// На сколько прямые стороны скругления заходят под ленты дорог, м.
const OVERLAP: f32 = 0.05;

/// Одна дорога, выходящая из узла.
struct Arm<'d> {
    class: RoadClass,
    highway: Highway,
    /// Мощёная дорожка ([`RoadLine::is_paved_path`]): её скругления ложатся
    /// в слой тротуаров, а не тропинок.
    ///
    /// [`RoadLine::is_paved_path`]: crate::map::osm::RoadLine::is_paved_path
    paved: bool,
    /// Грунтовая улица ([`RoadLine::is_unpaved_street`]): угол двух таких
    /// ложится в слой грунтовок, а не асфальта.
    ///
    /// [`RoadLine::is_unpaved_street`]: crate::map::osm::RoadLine::is_unpaved_street
    unpaved: bool,
    /// Полуширина слева и справа по ходу луча. Они разные у торца с клином
    /// на одну сторону (`roads/tapers.rs`): сужаемая кромка в узле стоит на
    /// полуширине узкого соседа, сохранённая — на своей.
    half: [f32; 2],
    /// Тротуар слева и справа по ходу луча: у половины разделённой улицы со
    /// стороны пары его нет.
    sidewalk: [Option<f32>; 2],
    /// Обочина до отдельного тротуара слева и справа по ходу луча
    /// (`RoadLine::verges`, `Drawn::verges_drawn`).
    verge: [Option<f32>; 2],
    direction: Vec2,
    /// Сколько метров край ленты идёт прямо — до следующей вершины.
    run: f32,
    /// Дорога и её торец (`0` — начало, `1` — конец), если луч — торец пути.
    end: Option<(usize, usize)>,
    /// Своя полуширина дороги — та, до которой клин у торца (`half` в узле
    /// — по узкому соседу) расходится за `widening` метров.
    full: f32,
    widening: f32,
    /// Узловая осевая дороги, вершина узла на ней и куда по ней идёт луч —
    /// из них [`Arm::trail`].
    path: &'d [Vec2],
    vertex: usize,
    forward: bool,
}

impl Arm<'_> {
    /// Осевая луча от узла на [`NOSE_REACH`] вперёд — по ней ищется нос: у
    /// острой развилки кромки сходятся за десятки метров от узла, где ленты
    /// давно уже не прямые (съезд с кольца, ветка по дуге). Собирается только
    /// для носа: лучей в городе десятки тысяч, носов — сотни. У замкнутого
    /// кольца — через шов.
    fn trail(&self) -> Vec<Vec2> {
        let path = self.path;
        let last = path.len() - 1;
        let closed = path[0] == path[last];
        let step = |from: usize| {
            if closed {
                Some(if self.forward {
                    (from + 1) % last
                } else {
                    (from + last - 1) % last
                })
            } else if self.forward {
                (from < last).then_some(from + 1)
            } else {
                from.checked_sub(1)
            }
        };
        let mut trail = vec![path[self.vertex]];
        let (mut reach, mut from) = (0.0, self.vertex);
        while reach < NOSE_REACH {
            let Some(index) = step(from).filter(|&index| index != self.vertex) else {
                break;
            };
            reach += path[index].distance(path[from]);
            trail.push(path[index]);
            from = index;
        }
        trail
    }

    /// Полуширина стороны `side` в `along` метрах от узла по лучу: в клине
    /// кромка расходится линейно, как её кладёт лента
    /// (`MeshBuilder::push_taper_sided`).
    fn half_at(&self, side: usize, along: f32) -> f32 {
        if self.widening <= 0.0 {
            return self.half[side];
        }
        let share = (along / self.widening).min(1.0);
        self.half[side] + (self.full - self.half[side]) * share
    }
}

/// Слой, в который ложится нос ([`KerbReturns::noses`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Road(RoadClass),
    Unpaved,
    Sidewalk,
}

/// Асфальт всех узлов.
#[derive(Default)]
pub struct KerbReturns {
    /// Носы острых развилок ([`NOSE_SHARE`]) и слой каждого. Нос не выпукл и
    /// веером не кладётся: его стороны идут по кромкам лент, а они у острой
    /// развилки гнутые, — контур триангулируется целиком.
    pub noses: Vec<(Fill, Vec<Vec2>)>,
    /// Контур и класс дорог, в чей слой заливки он ляжет: скругления и
    /// наружные углы. Каждый — веер из первой вершины.
    pub roads: Vec<(RoadClass, Vec<Vec2>)>,
    /// Скругления между двумя грунтовыми улицами — в их слое, так же.
    pub unpaved: Vec<Vec<Vec2>>,
    /// Контуры в слое тротуаров, так же: углы полос тротуара и скругления
    /// узлов с мощёной дорожкой.
    pub sidewalks: Vec<Vec<Vec2>>,
    /// Контуры в слое обочин (`road_verges`), так же: углы, где с одной
    /// стороны или с обеих вместо полосы тротуара обочина.
    pub verges: Vec<Vec<Vec2>>,
    /// Сколько из `roads` и `sidewalks` — наружные углы, а не скругления.
    pub outer: [usize; 2],
    /// Торцы, кончающиеся в узле, по дорогам: `[начало, конец]` — ленты с
    /// таким торцом кладутся с прямым, а не круглым.
    pub butt: Vec<[bool; 2]>,
}

impl KerbReturns {
    /// Торцы дороги `road`; вне узлов (или без скруглений вовсе) — круглые.
    pub fn butt(&self, road: usize) -> [bool; 2] {
        self.butt.get(road).copied().unwrap_or_default()
    }
}

/// Асфальт всех узлов: скругления проезжей части и тротуаров, наружные углы и
/// прямые торцы плеч.
///
/// Всё — по подготовленным дорогам `drawn` (`roads/drawn.rs`): **узловые**
/// осевые (`Axis::Nodal` — после сглаживания, до стежков; мост и арка не
/// участвуют, [`rounded`]); тротуар, если он рисуется
/// (`Drawn::sidewalk_drawn`); лежит ли рядом вторая половина разделённой
/// улицы и слева ли (`Pairs::beside`, `roads/network/pairs.rs`) — с её
/// стороны тротуара нет, и угол по нему не скругляется; клинья у торцов
/// (`Drawn::taper_ends`, `roads/tapers.rs`) — в клине кромка уже ближе к
/// оси, и прямой пробег луча с другого конца кончается там, где клин
/// начинается, а торец под клином стоит в узле на полуширине узкого соседа с
/// сужаемых сторон — по ней и считаются его углы; торец плечо слияния
/// (`Drawn::is_merged`, `roads/merges.rs`) — плечи одного слияния друг другу
/// не перекрёсток, ни прямых торцов, ни углов между ними. `scale` —
/// множитель радиусов по классам (ручка `Corner radius`).
pub fn kerb_returns(drawn: &Drawn, scale: f32) -> KerbReturns {
    let roads = drawn.roads();
    let nodes = drawn.nodes();
    let mut arms: HashMap<(i32, i32), (Vec2, Vec<Arm>)> = HashMap::new();
    for (index, &road) in roads.iter().enumerate() {
        let Some(path) = rounded(drawn, index) else {
            continue;
        };
        if path.len() < 2 {
            continue;
        }
        let wedges = drawn.taper_ends(index);
        let [head, tail] = wedges.map(|wedge| wedge.map_or(0.0, |wedge| wedge.length));
        let total = polyline_length(path);
        let closed = path[0] == path[path.len() - 1];
        let last = path.len() - 1;
        let mut along = 0.0;
        for (vertex, &node) in path.iter().enumerate() {
            if vertex > 0 {
                along += node.distance(path[vertex - 1]);
            }
            if closed && vertex == last {
                continue;
            }
            if !nodes.is_shared(node) {
                continue;
            }
            let entry = arms
                .entry(node_key(node))
                .or_insert_with(|| (node, Vec::new()));
            // вперёд по пути и назад; у замкнутого кольца — через шов (его
            // последняя вершина повторяет первую и пропущена выше)
            let ring = last;
            let step = |from: usize, forward: bool| {
                if closed {
                    Some(if forward {
                        (from + 1) % ring
                    } else {
                        (from + ring - 1) % ring
                    })
                } else if forward {
                    (from < last).then_some(from + 1)
                } else {
                    from.checked_sub(1)
                }
            };
            for forward in [true, false] {
                let mut at = vertex;
                let mut next = None;
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    at = index;
                    if path[index].distance(node) >= MIN_ARM {
                        next = Some(path[index]);
                        break;
                    }
                }
                let Some(next) = next else {
                    continue;
                };
                let direction = (next - node).normalize();
                // край идёт прямо и через вершины, лежащие на той же прямой: OSM
                // ставит их где угодно — узел пересечения с тротуаром в двух
                // метрах от улицы, излом в полградуса, — и дуга, обрезанная по
                // первой из них, не ложилась совсем
                let mut run = next.distance(node);
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    let offset = path[index] - node;
                    let along = offset.dot(direction);
                    if along <= run || direction.perp_dot(offset).abs() > STRAIGHT_TOLERANCE {
                        break;
                    }
                    run = along;
                    at = index;
                }
                // Кромка прямая только до клина: дальше лента сужается, и
                // касательная, заведённая в клин, торчала из-под него шипом
                // асфальта и тротуара (пример 08, улица в 9 м с клином к
                // однополосной).
                if !closed {
                    let body = if forward {
                        total - tail - along
                    } else {
                        along - head
                    };
                    run = run.min(body.max(0.0));
                }
                let at_end = !closed && (vertex == 0 || vertex == last);
                // стороны луча: слева по пути — слева по лучу вперёд и справа
                // по лучу назад
                let own = drawn.sidewalk_drawn(index);
                let mut sides = [own; 2];
                // тротуар по тегу — слева или справа по пути (`sidewalk=*`)
                for (side, present) in road.sidewalk().sides().into_iter().enumerate() {
                    if !present {
                        sides[usize::from((side == 0) != forward)] = None;
                    }
                }
                // торец под клином: с сужаемых сторон кромка и тротуар в узле
                // — узкого соседа, с сохранённой — свои
                let mut half = [road.width / 2.0; 2];
                let mut widening = 0.0;
                if let Some(wedge) = wedges[usize::from(vertex == last)].filter(|_| at_end) {
                    // клин так, как его нарежет лента; не лёг — кромка сразу своя
                    widening = super::tapers::fit(total, [head, tail].map(Some))
                        [usize::from(vertex == last)]
                    .unwrap_or(f32::MIN_POSITIVE);
                    let narrow = roads[wedge.narrow];
                    for (side, tapered) in wedge.sides.into_iter().enumerate() {
                        if !tapered {
                            continue;
                        }
                        let at = usize::from((side == 0) != forward);
                        half[at] = narrow.width / 2.0;
                        sides[at] = drawn
                            .sidewalk_drawn(wedge.narrow)
                            .filter(|_| narrow.sidewalk().sides()[side]);
                    }
                }
                // кусок пары кончается там, где пробы перестали её находить:
                // до узла — не дальше двух проб
                if let Some(left) = drawn.pairs().beside(index, along, 2.0 * PROBE_STEP) {
                    sides[usize::from(left != forward)] = None;
                }
                // обочины — слева и справа по пути, как тротуар по тегу
                let mut verge = [None; 2];
                for (side, width) in drawn.verges_drawn(index).into_iter().enumerate() {
                    verge[usize::from((side == 0) != forward)] = (width > 0.0).then_some(width);
                }
                entry.1.push(Arm {
                    class: road.class,
                    highway: road.highway,
                    paved: road.is_paved_path(),
                    unpaved: road.is_unpaved_street(),
                    half,
                    sidewalk: sides,
                    verge,
                    direction,
                    run,
                    end: at_end.then_some((index, usize::from(vertex == last))),
                    full: road.width / 2.0,
                    widening,
                    path,
                    vertex,
                    forward,
                });
            }
        }
    }

    let mut returns = KerbReturns {
        butt: vec![[false; 2]; roads.len()],
        ..default()
    };
    let is_merged = |arm: &Arm| {
        arm.end
            .is_some_and(|(road, end)| drawn.is_merged(road, end))
    };
    // между двумя плечами слияния нет ни угла, ни скругления
    let merge_pair = |first: &Arm, second: &Arm| is_merged(first) && is_merged(second);
    for (node, mut found) in arms.into_values() {
        if found.len() < 2 {
            continue;
        }
        found.sort_by(|a, b| a.direction.to_angle().total_cmp(&b.direction.to_angle()));
        for class in [RoadClass::Street, RoadClass::Alley] {
            let group: Vec<&Arm> = found.iter().filter(|arm| arm.class == class).collect();
            // узел одного слияния — продолжение дороги, а не перекрёсток
            if !is_junction(&group) || group.iter().all(|arm| is_merged(arm)) {
                continue;
            }
            for arm in &group {
                if let Some((road, end)) = arm.end.filter(|_| !is_merged(arm)) {
                    returns.butt[road][end] = true;
                }
            }
            for (first, second) in pairs(&group) {
                if merge_pair(first, second) {
                    continue;
                }
                let radius = kerb_radius(first, second) * scale;
                let halves = (first.half[0], second.half[1]);
                // угол у мощёной дорожки — плиткой, в слое тротуаров: песчаное
                // скругление на стыке двух плиточных аллей читалось бы пятном
                let paved = first.paved || second.paved;
                let (outline, outer) = match fillet(node, first, second, halves, radius) {
                    Some(outline) => (outline, false),
                    None if is_nose(first, second) => {
                        let fill = if paved {
                            Fill::Sidewalk
                        } else if first.unpaved && second.unpaved {
                            Fill::Unpaved
                        } else {
                            Fill::Road(class)
                        };
                        let radius = nose_radius(radius, scale);
                        if let Some(outline) = nose(first, second, (0.0, 0.0), radius) {
                            returns.noses.push((fill, outline));
                        }
                        continue;
                    }
                    None => match outer_corner(node, first, second, halves) {
                        Some(outline) => (outline, true),
                        None => continue,
                    },
                };
                let layer = usize::from(paved);
                if paved {
                    returns.sidewalks.push(outline);
                } else if first.unpaved && second.unpaved {
                    // угол двух грунтовок — грунтом; грунтовки с асфальтом —
                    // асфальтом: узел там асфальтовый
                    returns.unpaved.push(outline);
                } else {
                    returns.roads.push((class, outline));
                }
                returns.outer[layer] += usize::from(outer);
            }
        }
        // Угол, где хоть с одной стороны вместо полосы тротуара обочина до
        // отдельной дорожки, — той же дугой, но в слой обочин под зеленью:
        // без него между концом полосы (или обочины) одной улицы, обочиной
        // другой и бордюрной дугой оставался клин голой земли (Тула, 15).
        let verged: Vec<&Arm> = found
            .iter()
            .filter(|arm| (0..2).any(|side| arm.sidewalk[side].or(arm.verge[side]).is_some()))
            .collect();
        if is_junction(&verged) && !verged.iter().all(|arm| is_merged(arm)) {
            for (first, second) in pairs(&verged) {
                if merge_pair(first, second)
                    || (first.verge[0].is_none() && second.verge[1].is_none())
                {
                    continue;
                }
                // обочина тянется дальше полосы — угол по ней, где она есть
                let (Some(a), Some(b)) = (
                    first.verge[0].or(first.sidewalk[0]),
                    second.verge[1].or(second.sidewalk[1]),
                ) else {
                    continue;
                };
                // дуга — по узкой из двух: по широкой (как у полос тротуара)
                // она уходила от бордюра, и вдоль узкой оставался клин земли
                let halves = (first.half[0] + a, second.half[1] + b);
                let radius = kerb_radius(first, second) * scale - a.min(b);
                if let Some(outline) = fillet(node, first, second, halves, radius)
                    .or_else(|| outer_corner(node, first, second, halves))
                {
                    returns.verges.push(outline);
                }
            }
        }
        // Тротуары — свои соседи: проезд без тротуара не рвёт полосу улицы,
        // через которую он выходит, и угол считается между улицами по обе
        // стороны от него.
        let walked: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.sidewalk.iter().any(Option::is_some))
            .collect();
        if !is_junction(&walked) || walked.iter().all(|arm| is_merged(arm)) {
            continue;
        }
        for (first, second) in pairs(&walked) {
            if merge_pair(first, second) {
                continue;
            }
            // угол от левого края первого луча к правому краю второго
            let (Some(a), Some(b)) = (first.sidewalk[0], second.sidewalk[1]) else {
                continue;
            };
            // тот же центр, что у бордюрной дуги: радиус меньше на тротуар,
            // полоса шире на него же. Берётся больший из двух тротуаров — при
            // разной их ширине одной дугой обе полосы не обойти, а меньший
            // радиус оставляет асфальтовый клин внутри тротуарного
            let halves = (first.half[0] + a, second.half[1] + b);
            let radius = kerb_radius(first, second) * scale - a.max(b);
            if let Some(outline) = fillet(node, first, second, halves, radius) {
                returns.sidewalks.push(outline);
            } else if is_nose(first, second) {
                // У носа концентричная дуга ушла бы в минус — остриё островка
                // всё мощёное, — и нос тротуара свой, того же малого радиуса:
                // иначе между полосами тротуара торчал бы шип земли.
                let radius = nose_radius(kerb_radius(first, second) * scale, scale);
                if let Some(outline) = nose(first, second, (a, b), radius) {
                    returns.noses.push((Fill::Sidewalk, outline));
                }
            } else if let Some(outline) = outer_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
                returns.outer[1] += 1;
            }
        }
    }
    returns
}

/// Узел ли это для группы лучей одного класса: три плеча и больше или два,
/// сходящиеся углом. Два почти соосных плеча — продолжение дороги (другой
/// класс, другой way), и острые — развилка без третьего плеча: там торцы
/// остаются круглыми.
fn is_junction(group: &[&Arm]) -> bool {
    match group {
        [] | [_] => false,
        [first, second] => {
            let angle = ccw_angle(first, second);
            let turn = |angle: f32| (MIN_ANGLE..=MAX_ANGLE).contains(&angle);
            turn(angle) || turn(2.0 * PI - angle)
        }
        _ => true,
    }
}

/// Угол от луча `first` против часовой стрелки до луча `second`, (0, 2π].
fn ccw_angle(first: &Arm, second: &Arm) -> f32 {
    let angle = second.direction.to_angle() - first.direction.to_angle();
    if angle <= 0.0 {
        angle + 2.0 * PI
    } else {
        angle
    }
}

/// Соседние по углу пары лучей группы — по кругу, чтобы последний луч встретил
/// первый. Меньше двух лучей — пар нет.
fn pairs<'a, 'd>(group: &'a [&'a Arm<'d>]) -> impl Iterator<Item = (&'a Arm<'d>, &'a Arm<'d>)> {
    let count = if group.len() < 2 { 0 } else { group.len() };
    (0..count).map(move |index| (group[index], group[(index + 1) % group.len()]))
}

/// Радиус бордюра между двумя лучами узла — по младшему классу пары (см.
/// [`MAJOR_RADIUS`]).
fn kerb_radius(first: &Arm, second: &Arm) -> f32 {
    class_radius(first).min(class_radius(second))
}

/// Острый ли угол от `first` до `second` для носа ([`NOSE_SHARE`]) — если
/// скругление там не легло.
fn is_nose(first: &Arm, second: &Arm) -> bool {
    (NOSE_MIN_ANGLE..NOSE_MAX_ANGLE).contains(&ccw_angle(first, second))
}

/// Радиус носа при радиусе пары `radius` (уже умноженном на `scale`).
fn nose_radius(radius: f32, scale: f32) -> f32 {
    (radius * NOSE_SHARE).min(NOSE_RADIUS * scale)
}

fn class_radius(arm: &Arm) -> f32 {
    radius_of(arm.class, arm.highway)
}

/// Радиус бордюра дороги по её классу — без ручки `Corner radius`. Им клин
/// у шва решает, прикрывает ли примыкание уступ (`roads/tapers.rs`).
pub(super) fn road_radius(road: &RoadLine) -> f32 {
    radius_of(road.class, road.highway)
}

fn radius_of(class: RoadClass, highway: Highway) -> f32 {
    if class == RoadClass::Alley {
        return PATH_RADIUS;
    }
    match highway {
        Highway::Motorway
        | Highway::Trunk
        | Highway::Primary
        | Highway::Secondary
        | Highway::MotorwayLink
        | Highway::TrunkLink
        | Highway::PrimaryLink
        | Highway::SecondaryLink => MAJOR_RADIUS,
        Highway::Tertiary
        | Highway::TertiaryLink
        | Highway::Residential
        | Highway::Unclassified => STREET_RADIUS,
        // жилая зона, проезд и переезд через тротуар (дорожка, нарисованная
        // асфальтом, — `network::driveway_crossings`)
        Highway::LivingStreet | Highway::Service | Highway::Path => DRIVE_RADIUS,
    }
}

/// Наружный угол от луча `first` против часовой стрелки до луча `second`, если
/// просвет между ними шире развёрнутого: веер от узла с радиусом от одной
/// полуширины к другой — то, что раньше давали круглые торцы лент. Первая
/// вершина — центр веера, чуть позади узла, стороны заходят под торцы лент
/// на [`OVERLAP`].
fn outer_corner(node: Vec2, first: &Arm, second: &Arm, halves: (f32, f32)) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= PI + MIN_OUTER {
        return None;
    }
    let sweep = angle - PI;
    let from = first.direction.perp();
    let steps = arc_steps(halves.0.max(halves.1), sweep).max(1);
    let middle = Vec2::from_angle(sweep / 2.0).rotate(from);
    let mut outline = Vec::with_capacity(steps + 2);
    outline.push(node - middle * OVERLAP);
    outline.push(node + from * halves.0 + first.direction * OVERLAP);
    for step in 1..steps {
        let share = step as f32 / steps as f32;
        let radius = halves.0 + (halves.1 - halves.0) * share;
        outline.push(node + Vec2::from_angle(sweep * share).rotate(from) * radius);
    }
    outline.push(node - second.direction.perp() * halves.1 + second.direction * OVERLAP);
    Some(outline)
}

/// Скругление угла от луча `first` против часовой стрелки до луча `second`:
/// `halves` — полуширины полос, по краям которых сходится угол, `radius` —
/// радиус дуги в этом углу.
fn fillet(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    mut radius: f32,
) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if !(MIN_ANGLE..=MAX_ANGLE).contains(&angle) {
        return None;
    }
    let (along_first, along_second) = (first.direction, second.direction);
    // края, смотрящие друг на друга: у первого слева, у второго справа
    let (side_first, side_second) = (along_first.perp(), -along_second.perp());
    // угол, где встречаются края: halves.0·n₁ + t·u₁ = halves.1·n₂ + s·u₂
    let rhs = side_second * halves.1 - side_first * halves.0;
    let determinant = -along_first.perp_dot(along_second);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-along_second) / determinant;
    let s = along_first.perp_dot(rhs) / determinant;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    let corner = node + side_first * halves.0 + along_first * t;

    let half_angle = angle / 2.0;
    // касательная не длиннее прямого края ленты: за следующей вершиной край
    // уже повернул, и дуга легла бы мимо
    let tangent = (radius / half_angle.tan())
        .min(first.run - t)
        .min(second.run - s);
    if tangent <= 0.0 {
        return None;
    }
    radius = tangent * half_angle.tan();
    if radius < MIN_RADIUS {
        return None;
    }
    let on_first = corner + along_first * tangent;
    let on_second = corner + along_second * tangent;
    let centre = corner + (along_first + along_second).normalize() * (radius / half_angle.sin());

    let sweep = PI - angle;
    let from = on_first - centre;
    let turn = from.perp_dot(on_second - centre).signum();
    let steps = arc_steps(radius, sweep);
    // прямые стороны заходят под ленты на `OVERLAP`: сторона, совпадающая с
    // краем ленты, но не делящая с ней вершин, растеризуется с пропусками —
    // по краю проезда шла пунктирная щель со светлым тротуаром под ней
    let mut outline = Vec::with_capacity(steps + 4);
    outline.push(corner - (side_first + side_second) * OVERLAP);
    outline.push(on_first - side_first * OVERLAP);
    outline.push(on_first);
    for step in 1..steps {
        let rotation = Vec2::from_angle(turn * sweep * step as f32 / steps as f32);
        outline.push(centre + rotation.rotate(from));
    }
    outline.push(on_second);
    outline.push(on_second - side_second * OVERLAP);
    Some(outline)
}

/// Нос острой развилки от луча `first` против часовой стрелки до луча
/// `second` ([`NOSE_SHARE`]): асфальт (или тротуар) от острия, где сошлись
/// кромки, до дуги радиуса `radius`, касательной к обеим кромкам. Кромка —
/// полуширина луча ([`Arm::half_at`], с клином) плюс `extra` (тротуар:
/// левый у первого, правый у второго).
///
/// Кромки идут по **осевым лучей** ([`Arm::trail`]), а не по их начальным
/// направлениям: остриё лежит в десятках метров от узла, и прямая от узла
/// ушла бы с гнутой ленты — съезд с кольца, ветка по дуге — на метры. Шаг
/// [`NOSE_STEP`] вдоль первого луча: его кромка, ближайшая к ней точка оси
/// второго и просвет между кромками; остриё — где просвет стал положительным,
/// центр носа — середина просвета в два радиуса. Не нашлось (второй луч
/// кончился, просвет не дорос за [`NOSE_REACH`]) — `None`.
fn nose(first: &Arm, second: &Arm, extra: (f32, f32), radius: f32) -> Option<Vec<Vec2>> {
    if radius < MIN_RADIUS {
        return None;
    }
    let (first_trail, second_trail) = (first.trail(), second.trail());
    let (along_first, total_first) = arclengths(&first_trail);
    let (along_second, _) = arclengths(&second_trail);
    let links = second_trail
        .len()
        .checked_sub(1)
        .filter(|&links| links > 0)?;
    // звено второго, у которого нашлась прошлая проба: пробы идут вперёд, и
    // ближайшая точка ищется в окне от него, а не по всей осевой
    let mut hint: usize = 0;
    // кромка первого на `at`, лицом к ней — кромка второго, и просвет между ними
    let mut facing = |at: f32| -> Option<(Vec2, Vec2, f32)> {
        let (point, tangent) = place_on_path(&first_trail, &along_first, at)?;
        let edge = point + tangent.perp() * (first.half_at(0, at) + extra.0);
        let (link, onto) = (hint.saturating_sub(NOSE_WINDOW)..(hint + NOSE_WINDOW).min(links))
            .map(|link| {
                let onto = closest_on_segment(edge, second_trail[link], second_trail[link + 1]);
                (link, onto)
            })
            .min_by(|(_, a), (_, b)| {
                a.distance_squared(edge)
                    .total_cmp(&b.distance_squared(edge))
            })?;
        hint = link;
        // второй кончился: ближайшая точка — его конец, не перпендикуляр
        if link + 1 == links && onto == second_trail[links] {
            return None;
        }
        let reach = along_second[link] + second_trail[link].distance(onto);
        let direction = (second_trail[link + 1] - second_trail[link]).try_normalize()?;
        let out = -direction.perp();
        let half = second.half_at(1, reach) + extra.1;
        let gap = (edge - onto).dot(out) - half;
        Some((edge, onto + out * half, gap))
    };
    let mut tip = None;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut at = 0.0;
    // середина просвета в два радиуса и докуда ещё вести стороны: основания
    // перпендикуляров из центра лягут чуть дальше неё
    let mut found: Option<(Vec2, f32)> = None;
    while at <= total_first {
        let Some((edge, opposite, gap)) = facing(at) else {
            break;
        };
        if gap < 0.0 && found.is_none() {
            // остриё — середина скрещённых кромок последней пробы, под обеими
            // лентами
            tip = Some((edge + opposite) / 2.0);
        } else {
            let tip = tip?;
            if left.is_empty() {
                left.push(tip);
                right.push(tip);
            }
            // стороны заходят под ленты на `OVERLAP`; в узкой части клина —
            // вершина через [`NOSE_THIN`] проб: у развилки под 8° он тянется
            // на двадцать метров, и все его пробы были бы вершинами меша
            if found.is_some()
                || gap >= radius
                || ((at / NOSE_STEP) as usize).is_multiple_of(NOSE_THIN)
            {
                let inward = (opposite - edge).normalize_or_zero() * OVERLAP;
                left.push(edge - inward);
                right.push(opposite + inward);
            }
            match found {
                None if gap >= 2.0 * radius => {
                    found = Some(((edge + opposite) / 2.0, at + 2.0 * radius));
                }
                Some((_, until)) if at >= until => break,
                _ => {}
            }
        }
        at += NOSE_STEP;
    }
    let (mut centre, _) = found?;
    // Дуга касается обеих кромок: центр сдвигается, пока не встанет на радиус
    // от каждой. Середина просвета — только начало: гнутая кромка (кольцо
    // выпукло в сторону островка) ближе к ней, чем прямая, и дуга в радиус до
    // ближней не доставала до дальней — у острия оставалась зазубрина.
    for _ in 0..NOSE_SETTLE {
        let (left_foot, _) = nearest_on_path(&left, centre)?;
        let (right_foot, _) = nearest_on_path(&right, centre)?;
        let (away_left, away_right) = (centre - left_foot, centre - right_foot);
        centre += away_left.normalize_or_zero() * (radius - away_left.length())
            + away_right.normalize_or_zero() * (radius - away_right.length());
    }
    let (foot_left, cut_left) = nearest_on_path(&left, centre)?;
    let (foot_right, cut_right) = nearest_on_path(&right, centre)?;
    let radius = centre.distance(foot_left).min(centre.distance(foot_right));
    if radius < MIN_RADIUS {
        return None;
    }
    let keep = |chain: &[Vec2], cut: f32| -> Vec<Vec2> {
        let (along, _) = arclengths(chain);
        chain
            .iter()
            .zip(&along)
            .take_while(|(_, along)| **along < cut)
            .map(|(point, _)| *point)
            .collect()
    };
    let mut outline = keep(&left, cut_left);
    let from = (foot_left - centre).normalize_or_zero() * radius;
    let to = (foot_right - centre).normalize_or_zero() * radius;
    // короткой стороной — она и смотрит на остриё
    let sweep = from.angle_to(to);
    let steps = arc_steps(radius, sweep.abs()).max(1);
    for step in 0..=steps {
        outline.push(centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from));
    }
    // остриё — уже первая вершина левой стороны
    outline.extend(keep(&right, cut_right).into_iter().skip(1).rev());
    (outline.len() >= 3).then_some(outline)
}

/// Остров внутри треугольника узлов заливается, когда от него за вычетом
/// полотен остаётся полоса уже этого, м: вписанный радиус треугольника минус
/// наибольшая полуширина его улиц. Развилка в Туле (витрина 06) — треугольник
/// в 17–31 м со сторон, вписанный радиус 5.2 м при полуширине 3.8: от острова
/// оставалась линза в метр с небольшим, и в ней серпом светлела земля.
const ISLAND_FILL: f32 = 2.0;
/// Периметр треугольника, выше которого остров не трогается, м: большой
/// треугольник развилки — настоящий остров, со своим газоном или домом.
const ISLAND_PERIMETER_MAX: f32 = 120.0;

/// Острова-крошки: треугольник из трёх общих узлов, попарно соединённых
/// кусками улиц, от которого за полотнами почти ничего не остаётся
/// ([`ISLAND_FILL`]). Контур — по нарисованным осям `paths` (`None` — дорога
/// не участвует); кладётся асфальтом под ленты, как перепонки колец.
pub fn small_islands(drawn: &Drawn) -> Vec<Vec<Vec2>> {
    let nodes = drawn.nodes();
    // рёбра: куски улиц между соседними общими узлами
    struct Edge<'a> {
        ends: [(i32, i32); 2],
        path: &'a [Vec2],
        half: f32,
    }
    let mut edges: Vec<Edge> = Vec::new();
    for index in 0..drawn.len() {
        let Some(path) = rounded(drawn, index) else {
            continue;
        };
        let road = drawn.road(index);
        if road.class != RoadClass::Street || path.len() < 2 {
            continue;
        }
        let shared: Vec<usize> = (0..path.len())
            .filter(|&index| nodes.is_shared(path[index]))
            .collect();
        for pair in shared.windows(2) {
            edges.push(Edge {
                ends: [node_key(path[pair[0]]), node_key(path[pair[1]])],
                path: &path[pair[0]..=pair[1]],
                half: road.width / 2.0,
            });
        }
    }
    let mut at: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if edge.ends[0] != edge.ends[1] {
            at.entry(edge.ends[0]).or_default().push(index);
            at.entry(edge.ends[1]).or_default().push(index);
        }
    }
    let other = |edge: usize, end: (i32, i32)| {
        let [a, b] = edges[edge].ends;
        if a == end { b } else { a }
    };
    // ребро, пройденное от узла `from`
    let walk = |edge: usize, from: (i32, i32)| -> Vec<Vec2> {
        let mut points = edges[edge].path.to_vec();
        if edges[edge].ends[0] != from {
            points.reverse();
        }
        points
    };
    let mut islands = Vec::new();
    for (first, edge) in edges.iter().enumerate() {
        let [a, b] = edge.ends;
        for &second in at.get(&b).into_iter().flatten() {
            let c = other(second, b);
            if second <= first || c == a {
                continue;
            }
            for &third in at.get(&c).into_iter().flatten() {
                if third <= first || third == second || other(third, c) != a {
                    continue;
                }
                let mut outline = walk(first, a);
                for (edge, from) in [(second, b), (third, c)] {
                    outline.pop();
                    outline.extend(walk(edge, from));
                }
                outline.pop();
                let perimeter: f32 = outline
                    .iter()
                    .zip(outline.iter().cycle().skip(1))
                    .map(|(p, q)| p.distance(*q))
                    .sum();
                let area = ring_area(&outline);
                let half = [first, second, third]
                    .map(|edge| edges[edge].half)
                    .into_iter()
                    .fold(0.0, f32::max);
                if perimeter < ISLAND_PERIMETER_MAX && 2.0 * area / perimeter - half < ISLAND_FILL {
                    islands.push(outline);
                }
            }
        }
    }
    islands
}

/// Узловая осевая дороги, если она участвует в узлах: мост и арка — нет, их
/// торцы стоят ровным срезом бордюра и у стен дома.
fn rounded<'d>(drawn: &'d Drawn, index: usize) -> Option<&'d [Vec2]> {
    (!drawn.road(index).carves_navmesh()).then(|| drawn.axis(index, Axis::Nodal))
}

#[cfg(test)]
mod tests {
    use super::super::network::pairs::PairRun;
    use super::super::tapers::Taper;
    use super::*;
    use crate::map::osm::fixture::street;
    use crate::map::osm::model::point_in_polygon;
    use crate::map::osm::{MapData, RoadLine};

    fn returns_of(roads: &[RoadLine]) -> Vec<(RoadClass, Vec<Vec2>)> {
        walked_returns_of(roads, false).roads
    }

    /// Карта из одних дорог — вход [`Drawn::for_test`].
    fn map_of(roads: &[RoadLine]) -> MapData {
        MapData {
            roads: roads.to_vec(),
            ..default()
        }
    }

    /// Скругления с тротуарами по карте (`sidewalks`) или без них.
    fn walked_returns_of(roads: &[RoadLine], sidewalks: bool) -> KerbReturns {
        let map = map_of(roads);
        kerb_returns(&Drawn::for_test(&map).with_sidewalks(sidewalks), 1.0)
    }

    /// Тротуар улицы 8 м — как его считает `osm::model::sidewalk_band`.
    const SIDEWALK: f32 = 1.76;

    /// Перекрёсток двух улиц 8 м в начале координат.
    fn crossing() -> [RoadLine; 2] {
        [
            east_west(),
            street(
                vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
                8.0,
            ),
        ]
    }

    /// Сквозная улица 8 м по оси x через узел в начале координат.
    fn east_west() -> RoadLine {
        street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            8.0,
        )
    }

    #[test]
    fn a_crossing_gets_four_rounded_corners_outside_both_ribbons() {
        let found = returns_of(&crossing());
        assert_eq!(found.len(), 4);
        for (_, outline) in &found {
            // угол — на пересечении краёв, чуть под лентами
            assert!(
                (outline[0].abs() - Vec2::splat(4.0 - OVERLAP)).length() < 1e-3,
                "{:?}",
                outline[0]
            );
            let corner = Vec2::splat(4.0) * outline[0].signum();
            // и сама дуга за краями обеих лент
            for point in &outline[1..] {
                assert!(
                    point.x.abs() >= 4.0 - OVERLAP - 1e-3 && point.y.abs() >= 4.0 - OVERLAP - 1e-3
                );
            }
            // радиус двух жилых улиц: все точки дуги на нём от центра
            let centre = corner + Vec2::splat(STREET_RADIUS) * outline[0].signum();
            for point in &outline[2..outline.len() - 1] {
                let radius = point.distance(centre);
                assert!((radius - STREET_RADIUS).abs() < 1e-2, "{radius}");
            }
        }
    }

    /// Радиус скругления прямого угла: касательная от угла краёв до дуги
    /// равна ему. Третья вершина контура — точка касания на первом луче.
    fn right_angle_radius(outline: &[Vec2]) -> f32 {
        let corner = outline[0] + (outline[0].signum() * OVERLAP);
        outline[2].distance(corner)
    }

    #[test]
    fn a_t_junction_rounds_only_the_two_turning_corners() {
        let through = east_west();
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = returns_of(&[through, side]);
        assert_eq!(found.len(), 2);
        // обе дуги со стороны примыкания
        for (_, outline) in &found {
            assert!(outline.iter().all(|point| point.y >= 4.0 - OVERLAP - 1e-3));
        }
        let corner = &found[0].1;
        assert!(!point_in_polygon(Vec2::new(0.0, 10.0), corner));
    }

    #[test]
    fn a_street_and_a_footway_are_not_rounded_together() {
        let through = east_west();
        let path = RoadLine {
            class: RoadClass::Alley,
            ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 3.5)
        };
        assert!(returns_of(&[through, path]).is_empty());
    }

    #[test]
    fn a_vertex_on_a_straight_arm_does_not_cut_the_corner() {
        // проезд пересекает тротуар в двух метрах от улицы — узел на прямой,
        // но скругление по-прежнему ложится полным радиусом
        let through = east_west();
        let drive = street(
            vec![
                Vec2::new(0.0, 40.0),
                Vec2::new(0.05, 6.0),
                Vec2::new(0.0, 2.0),
                Vec2::ZERO,
            ],
            5.0,
        );
        let found = returns_of(&[through, drive]);
        assert_eq!(found.len(), 2);
        for (_, outline) in &found {
            // с проездом радиус проезда: дуга уходит вдоль проезда на него от
            // края улицы
            let reach = outline.iter().map(|point| point.y).fold(0.0, f32::max);
            assert!((reach - (4.0 + DRIVE_RADIUS)).abs() < 0.1, "{reach}");
        }
    }

    #[test]
    fn a_short_arm_limits_the_radius() {
        // вторая вершина поперечной улицы в двух метрах за краем — дуга не
        // длиннее этого
        let through = east_west();
        let side = street(
            vec![Vec2::new(30.0, 30.0), Vec2::new(0.0, 6.0), Vec2::ZERO],
            8.0,
        );
        for (_, outline) in returns_of(&[through, side]) {
            for point in &outline {
                assert!(point.y <= 6.0 + 1e-3, "{point:?}");
            }
        }
    }

    /// Поперечная в 20 м с клином на дальнем конце в 14 м: кромка прямая
    /// только 6 м от узла, и дуга дальше не заходит — в клине лента уже, и
    /// касательная торчала из-под него шипом (витрина 08).
    #[test]
    fn a_taper_on_the_arm_limits_the_radius() {
        let map = map_of(&[
            east_west(),
            street(vec![Vec2::ZERO, Vec2::new(0.0, 20.0)], 8.0),
        ]);
        let drawn = Drawn::for_test(&map).with_sidewalks(false).with_taper(
            1,
            1,
            Taper {
                length: 14.0,
                narrow: 0,
                sides: [true; 2],
            },
        );
        let found = kerb_returns(&drawn, 1.0);
        assert!(!found.roads.is_empty());
        for (_, outline) in &found.roads {
            for point in outline {
                assert!(point.y <= 6.0 + 1e-3, "{point:?}");
            }
        }
    }

    #[test]
    fn the_sidewalk_turns_the_corner_on_the_kerb_arc() {
        let found = walked_returns_of(&crossing(), true);
        assert_eq!(found.sidewalks.len(), 4);
        // центр бордюрной дуги в северо-восточном углу: угол краёв (4, 4) плюс
        // биссектриса на r/sin 45°
        let centre = Vec2::splat(4.0 + STREET_RADIUS);
        for outline in &found.sidewalks {
            let corner = outline[0].signum() * Vec2::splat(4.0 + SIDEWALK - OVERLAP);
            assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
            // дуга — того же центра, радиусом на тротуар меньше бордюрной, так
            // что за бордюром идёт ровная полоса в ширину тротуара
            let quadrant = outline[0].signum() * centre;
            for point in &outline[2..outline.len() - 1] {
                let radius = point.distance(quadrant);
                assert!(
                    (radius - (STREET_RADIUS - SIDEWALK)).abs() < 1e-2,
                    "{radius}"
                );
            }
        }
        // и асфальтовый клин по-прежнему целиком на тротуаре: где не на полосе,
        // там на её скруглении
        for (_, outline) in &found.roads {
            let quadrant = outline[0].signum() * centre;
            for point in outline {
                let band = 4.0 + SIDEWALK + 1e-3;
                let on_band = point.x.abs() <= band || point.y.abs() <= band;
                assert!(
                    on_band || point.distance(quadrant) >= STREET_RADIUS - SIDEWALK - 1e-3,
                    "{point:?}"
                );
            }
        }
    }

    #[test]
    fn a_kerb_radius_under_the_sidewalk_width_leaves_the_corner_square() {
        // жилая зона с тротуаром выходит в магистраль: радиус жилой зоны
        // 2.5 м меньше трёхметрового тротуара (16 м — `sidewalk_band` в
        // потолке), и внешний угол полосы на месте такой же прямой
        let avenue = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(50.0, 0.0)],
            16.0,
        );
        let side = RoadLine {
            highway: Highway::LivingStreet,
            ..street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 16.0)
        };
        let found = walked_returns_of(&[avenue, side], true);
        assert_eq!(found.roads.len(), 2);
        assert!(found.sidewalks.is_empty());
    }

    #[test]
    fn a_drive_without_a_sidewalk_does_not_break_the_band() {
        // проезд 5 м выходит в улицу: асфальт скругляется с обеих сторон,
        // а полоса тротуара идёт мимо него насквозь — скруглять нечего
        let through = east_west();
        let drive = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 5.0);
        let found = walked_returns_of(&[through, drive], true);
        assert_eq!(found.roads.len(), 2);
        assert!(found.sidewalks.is_empty());
    }

    #[test]
    fn a_drive_between_two_streets_leaves_their_sidewalk_corner_alone() {
        // тот же проезд, но улицы сходятся углом: тротуарный угол считается
        // между ними, поверх устья проезда — его асфальт ляжет сверху
        let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
        let drive = street(vec![Vec2::ZERO, Vec2::new(35.0, 35.0)], 5.0);
        let found = walked_returns_of(&[east, north, drive], true);
        // скругление внутри угла и наружный угол по другую сторону узла
        assert_eq!(found.sidewalks.len(), 2);
        assert_eq!(found.outer[1], 1);
        let outline = &found.sidewalks[0];
        let corner = Vec2::splat(4.0 + SIDEWALK - OVERLAP);
        assert!((outline[0] - corner).length() < 1e-3, "{:?}", outline[0]);
    }

    #[test]
    fn no_sidewalk_corner_on_the_side_of_the_paired_half() {
        // сквозная — половина разделённой улицы, вторая половина слева (к
        // северу): с той стороны тротуара нет, и углов по нему тоже
        let map = map_of(&crossing());
        let drawn = Drawn::for_test(&map).with_pairs(
            0,
            vec![PairRun {
                from: 0.0,
                to: 100.0,
                partner: 1,
                left: true,
                gap: 0.0,
                paved: true,
                tram: false,
            }],
        );
        let found = kerb_returns(&drawn, 1.0);
        assert_eq!(found.roads.len(), 4, "асфальт скругляется, как был");
        assert_eq!(found.sidewalks.len(), 2);
        for outline in &found.sidewalks {
            assert!(outline[0].y < 0.0, "{:?}", outline[0]);
        }
    }

    fn with_highway(road: RoadLine, highway: Highway) -> RoadLine {
        RoadLine { highway, ..road }
    }

    #[test]
    fn the_radius_follows_the_minor_class_of_the_pair() {
        let nearest_to_corner = |found: &[(RoadClass, Vec<Vec2>)]| right_angle_radius(&found[0].1);
        let [through, side] = crossing();
        let avenues = returns_of(&[
            with_highway(through.clone(), Highway::Primary),
            with_highway(side.clone(), Highway::Secondary),
        ]);
        assert!((nearest_to_corner(&avenues) - MAJOR_RADIUS).abs() < 0.1);
        let street_into_avenue = returns_of(&[
            with_highway(through.clone(), Highway::Primary),
            with_highway(side.clone(), Highway::Residential),
        ]);
        assert!((nearest_to_corner(&street_into_avenue) - STREET_RADIUS).abs() < 0.1);
        let drive = returns_of(&[
            with_highway(through, Highway::Primary),
            with_highway(side, Highway::Service),
        ]);
        assert!((nearest_to_corner(&drive) - DRIVE_RADIUS).abs() < 0.1);
    }

    #[test]
    fn an_arm_ending_in_a_junction_ends_square_and_a_seam_stays_round() {
        // Т-образный узел: торец примыкающей улицы в узле — прямой, у
        // сквозной там не торец
        let through = east_west();
        let side = street(vec![Vec2::new(0.0, 50.0), Vec2::ZERO], 8.0);
        let found = walked_returns_of(&[through, side], false);
        assert_eq!(found.butt, vec![[false; 2], [false, true]]);
        // шов двух ways одной улицы — не узел, торцы круглые
        let first = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
        let second = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let found = walked_returns_of(&[first, second], false);
        assert_eq!(found.butt, vec![[false; 2]; 2]);
        assert!(found.roads.is_empty());
    }

    #[test]
    fn two_streets_meeting_at_a_corner_get_a_rounded_outer_side() {
        // угол двух улиц без сквозной: скругление внутри, веер снаружи — тот
        // же, что прежде давали круглые торцы
        let east = street(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], 8.0);
        let north = street(vec![Vec2::ZERO, Vec2::new(0.0, 50.0)], 8.0);
        let found = walked_returns_of(&[east, north], false);
        assert_eq!(found.roads.len(), 2);
        assert_eq!(found.outer[0], 1);
        assert_eq!(found.butt, vec![[true, false]; 2]);
        let outer = &found.roads[1].1;
        // веер — в третьем квадранте, радиус полуширины
        for point in &outer[2..outer.len() - 1] {
            assert!(point.x < 0.0 && point.y < 0.0, "{point:?}");
            assert!((point.length() - 4.0).abs() < 1e-3, "{point:?}");
        }
        assert!(point_in_polygon(Vec2::new(-2.0, -2.0), outer));
    }

    #[test]
    fn a_small_triangle_of_streets_is_paved_and_a_large_one_is_not() {
        // развилка с островом (Тула, витрина 06): три улицы по 7.6 м между
        // тремя узлами; вписанный радиус 5.2 м — острова за полотнами почти нет
        let triangle = |scale: f32| {
            let [a, b, c] = [Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(10.0, 12.0)]
                .map(|point| point * scale);
            [(a, b), (b, c), (c, a)].map(|(from, to)| street(vec![from, to], 7.6))
        };
        let islands = |roads: &[RoadLine]| small_islands(&Drawn::for_test(&map_of(roads)));
        let small = islands(&triangle(1.0));
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].len(), 3, "контур — три узла: {:?}", small[0]);
        assert!(point_in_polygon(Vec2::new(13.0, 4.0), &small[0]));
        assert!(
            islands(&triangle(2.0)).is_empty(),
            "большой остров — настоящий"
        );
    }

    #[test]
    fn a_sharp_fork_gets_a_small_nose_instead_of_a_spike() {
        // ветка уходит от сквозной улицы под 15°: кромки сходятся в 30.6 м от
        // узла по биссектрисе, и нос радиуса NOSE_RADIUS закрывает остриё до
        // дуги в 40.6 м (Тула, витрины 04, 06, 14)
        let angle = 15_f32.to_radians();
        let branch = street(
            vec![Vec2::ZERO, Vec2::from_angle(angle).rotate(Vec2::X) * 80.0],
            8.0,
        );
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
            8.0,
        );
        let found = walked_returns_of(&[through, branch], true);
        let bisector = Vec2::from_angle(angle / 2.0).rotate(Vec2::X);
        let half_angle = angle / 2.0;
        let apex =
            |half: f32| half / half_angle.sin() + NOSE_RADIUS / half_angle.sin() - NOSE_RADIUS;
        let covered = |fill: Fill, at: f32| {
            found
                .noses
                .iter()
                .any(|(layer, outline)| *layer == fill && point_in_polygon(bisector * at, outline))
        };
        let road = Fill::Road(RoadClass::Street);
        // шаг поиска 0.5 м — центр носа может уйти на четверть метра
        assert!(covered(road, apex(4.0) - 1.0), "остриё до дуги — асфальт");
        assert!(!covered(road, apex(4.0) + 0.5), "за дугой асфальта нет");
        // дуга носа — на радиусе от центра и не ближе к нему
        let (_, outline) = found.noses.iter().find(|(fill, _)| *fill == road).unwrap();
        let centre = bisector * (apex(4.0) + NOSE_RADIUS);
        let on_arc = outline
            .iter()
            .filter(|point| (point.distance(centre) - NOSE_RADIUS).abs() < 0.3)
            .count();
        assert!(on_arc >= 3, "{outline:?}");
        assert!(
            outline
                .iter()
                .all(|point| point.distance(centre) > NOSE_RADIUS - 0.3)
        );
        // тротуар — своим носом по краям полос, дальше асфальтового
        let walked = apex(4.0 + SIDEWALK);
        assert!(
            covered(Fill::Sidewalk, walked - 1.0),
            "остриё тротуара закрыто"
        );
        assert!(!covered(Fill::Sidewalk, walked + 0.5));
    }

    #[test]
    fn a_nose_follows_a_bent_branch() {
        // ветка уходит под 15° и через 20 м загибается до 30°: прямая от
        // узла ушла бы с её кромки, нос же лежит на кромках лент — ни одна его
        // вершина, кроме острия, не заходит под ленты глубже OVERLAP
        let first = Vec2::from_angle(15_f32.to_radians()).rotate(Vec2::X) * 20.0;
        let branch = street(
            vec![
                Vec2::ZERO,
                first,
                first + Vec2::from_angle(30_f32.to_radians()).rotate(Vec2::X) * 60.0,
            ],
            8.0,
        );
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
            8.0,
        );
        let found = walked_returns_of(&[through.clone(), branch.clone()], false);
        assert_eq!(found.noses.len(), 1);
        let (_, outline) = &found.noses[0];
        for point in &outline[1..] {
            for road in [&through, &branch] {
                let inside =
                    4.0 - crate::map::footprint::distance_to_polyline(*point, &road.points);
                assert!(inside <= OVERLAP + 0.02, "{point:?} под лентой на {inside}");
            }
        }
    }

    #[test]
    fn nearly_parallel_arms_get_no_nose() {
        // почти параллельные полотна — одна дорога из двух way
        let branch = street(
            vec![
                Vec2::ZERO,
                Vec2::from_angle(1_f32.to_radians()).rotate(Vec2::X) * 80.0,
            ],
            8.0,
        );
        assert!(
            walked_returns_of(&[east_west(), branch], false)
                .noses
                .is_empty()
        );
    }

    #[test]
    fn a_slight_kink_between_two_ways_at_a_junction_is_closed() {
        // улица из двух ways, излом в узле 0.7° наружу от примыкания (Тула,
        // витрина 08): торцы обоих плеч прямые, и клин между ними со стороны
        // без примыкания закрывает наружный угол
        let kink = (-0.7_f32).to_radians();
        let west = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
        let east = street(
            vec![Vec2::ZERO, Vec2::from_angle(kink).rotate(Vec2::X) * 50.0],
            8.0,
        );
        let side = street(vec![Vec2::new(0.0, -50.0), Vec2::ZERO], 8.0);
        let found = walked_returns_of(&[west, east, side], false);
        assert_eq!(found.outer[0], 1, "{:?}", found.roads);
        // клин — у кромки над узлом, между торцами
        let wedge = Vec2::new(0.02, 3.9);
        assert!(
            found
                .roads
                .iter()
                .any(|(_, outline)| point_in_polygon(wedge, outline)),
            "клин открыт"
        );
    }
}
