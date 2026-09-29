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

use super::VERGE_PAVED_MAX;
use super::drawn::{Axis, Drawn};
use super::junctions::node_key;
use super::network::pairs::PROBE_STEP;
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::meshing::{arc_steps, miter_offsets, ribbon_merge_distance, ribbon_vertices};
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
/// Радиус угла двух грунтовок, м, — не больше этого, какого бы класса они ни
/// были. Уличные 6 м на однополосном проезде частного сектора (Тула, 18-й и
/// 8-й проезды Мясново) ложились чёткой бордюрной дугой шире самого проезда —
/// асфальтовым перекрёстком, отлитым в грунте. Угол грунтовок срезан колёсами
/// невысоко: радиус чуть больше, чем у проезда.
const DIRT_RADIUS: f32 = 3.0;
/// Скругление меньше этого не кладётся, м: его всё равно не видно.
const MIN_RADIUS: f32 = 0.5;
/// Угол между лучами, в котором скругление имеет смысл. Острее — дуга
/// вытягивается в длинный клин, и там кладётся нос ([`NOSE_SHARE`]); почти
/// развёрнутый угол — это продолжение дороги, а не поворот.
const MIN_ANGLE: f32 = 25.0 * PI / 180.0;
const MAX_ANGLE: f32 = 155.0 * PI / 180.0;
/// Тупой угол от [`MAX_ANGLE`] до этого на перекрёстке со сменой ширины
/// получает клин широкой кромки ([`obtuse_corner`]); почти соосные плечи —
/// продолжение дороги, их уступ — дело клина сечений (`roads/tapers.rs`).
const OBTUSE_MAX: f32 = 178.0 * PI / 180.0;
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
/// Как далеко от узла по лучу ищется касание изогнутого скругления
/// ([`bend`]), м: радиус проспекта — десять метров, и касательная у тупого
/// угла втрое длиннее.
const BEND_REACH: f32 = 40.0;
/// Сколько раз центр изогнутого скругления сдвигается к касанию обеих кромок.
const BEND_SETTLE: usize = 8;
/// Насколько дуга изогнутого скругления может разойтись с кромкой в точке
/// касания, м: больше — центр не сошёлся, и дуга легла бы мимо.
const BEND_FIT: f32 = 0.05;
/// На сколько стороны изогнутого скругления заходят под ленты, м: кромка
/// считается по густой осевой, а лента кладёт гнутый край хордами по своим
/// вершинам (`meshing::ribbon_vertices`), и пяти сантиметров [`OVERLAP`] на
/// дуге не хватало — по стороне шла пунктирная щель (Ростов, витрина 06).
const BEND_OVERLAP: f32 = 0.3;
/// Наружный угол узла закругляется, когда просвет между плечами шире
/// развёрнутого хотя бы на столько: у сквозной дороги просветы ровно по
/// 180°, и веер там лёг бы под её же ленту. Порог — на шум округления, не
/// больше: улица из двух way с изломом в узле в 0.7° (Ложевая у Пролетарской,
/// Тула, витрина 08) при пороге в градус оставалась без угла, и между
/// прямыми торцами её плеч светлел клин в семь сантиметров.
const MIN_OUTER: f32 = 0.05 * PI / 180.0;
/// Площадка плитки у бордюрной дуги угла, за которым газон обочины
/// ([`kerb_pad`]): ширина от бордюра, м, и на сколько она продолжается по
/// прямому краю каждой улицы за точкой касания, м. На углу выходят зебры и
/// стоят люди — на месте здесь плитка, а газон начинается за ней.
const KERB_PAD_WIDTH: f32 = 3.0;
const KERB_PAD_RUN: f32 = 4.0;
/// Луч меряет направление по звену не короче этого, м.
const MIN_ARM: f32 = 0.5;
/// Насколько вершина может отойти вбок от прямой луча и всё ещё продолжать
/// его прямой край, м.
const STRAIGHT_TOLERANCE: f32 = 0.15;
/// На сколько прямые стороны скругления заходят под ленты дорог, м.
const OVERLAP: f32 = 0.05;
/// То же у наружного угла ([`outer_corner`]), м: его стороны — по лучам
/// узловой оси, а прямой торец ленты — по её собственному концевому звену,
/// и на почти прямом стыке двух way одной улицы они расходятся на градус.
/// На восьми метрах полосы тротуара это полтора десятка сантиметров, и
/// пяти сантиметров нахлёста не хватало — через тротуар шла нить (Орёл,
/// витрина 04, север).
const OUTER_OVERLAP: f32 = 0.3;

/// Одна дорога, выходящая из узла.
struct Arm<'d> {
    class: RoadClass,
    highway: Highway,
    /// Дуга кольца (`Drawn::on_ring`): у колец клинья штрихуют свои островки
    /// (`roads/gores.rs`), и клина развилки ([`fork_gore`]) им не нужно.
    ring: bool,
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
    /// Полуширина слева и справа по ходу луча. Они разные под клином на
    /// одну сторону (`roads/tapers.rs`): сужаемая кромка в узле стоит между
    /// полуширинами узкого соседа и своей — у шва на соседской, — а
    /// сохранённая на своей.
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
    /// Где луч пересекает другая дорога — ближайший общий узел, м по оси от
    /// узла; нет такого в досягаемости хвоста площадки — бесконечность.
    /// Площадка плитки у бордюра ([`kerb_pad`]) за него не тянется.
    crossed: f32,
    /// Дорога и её торец (`0` — начало, `1` — конец), если луч — торец пути.
    end: Option<(usize, usize)>,
    /// Своя полуширина дороги.
    full: f32,
    /// Полуширина каждой стороны, к которой кромка под клином приходит за
    /// `widening` метров по лучу: своя — от шва, соседская — к шву; дальше
    /// она держится ([`Arm::half_at`]).
    reach: [f32; 2],
    widening: f32,
    /// Наклон кромки каждой стороны в клине: на сколько она отходит от оси
    /// на метр луча. Вне клина — ноль; скругление ставит дугу на эту,
    /// наклонную, кромку ([`fillet_arc`]).
    slope: [f32; 2],
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
        self.half[side] + (self.reach[side] - self.half[side]) * share
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
    /// Изогнутые скругления ([`bend`]) и слой каждого: у кольца и гнутого
    /// подхода кромка кривая, контур не выпукл — триангулируется целиком.
    pub bends: Vec<(Fill, Vec<Vec2>)>,
    /// Штрихуемые клинья перед носами острых развилок улиц ([`fork_gore`]):
    /// контур от острия клина до дуги носа — слою островков
    /// (`roads/gores.rs`).
    pub fork_gores: Vec<Vec<Vec2>>,
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
    /// Те же углы между двумя **широкими** обочинами — газоном
    /// (`road_verge_lawns` или `road_verge_yards`), как и сами обочины: плиткой угол во все
    /// пятнадцать метров до дорожек был площадью посреди двора.
    pub verge_lawns: Vec<Vec<Vec2>>,
    /// Сколько из `roads` и `sidewalks` — наружные углы, а не скругления.
    pub outer: [usize; 2],
    /// Торцы, кончающиеся в узле, по дорогам: `[начало, конец]` — ленты с
    /// таким торцом кладутся с прямым, а не круглым.
    pub butt: Vec<[bool; 2]>,
    /// На сколько метров не доходит до узла заливка торца `[начало, конец]`:
    /// у асфальтовой улицы, упёршейся в грунтовку, — полуширина грунтовки.
    pub setback: Vec<[f32; 2]>,
}

impl KerbReturns {
    /// Торцы дороги `road`; вне узлов (или без скруглений вовсе) — круглые.
    pub fn butt(&self, road: usize) -> [bool; 2] {
        self.butt.get(road).copied().unwrap_or_default()
    }

    /// Недоход заливки дороги `road` до узла у каждого торца, м.
    pub fn setback(&self, road: usize) -> [f32; 2] {
        self.setback.get(road).copied().unwrap_or_default()
    }
}

/// Асфальт всех узлов: скругления проезжей части и тротуаров, наружные углы и
/// прямые торцы плеч.
///
/// Всё — по подготовленным дорогам `drawn` (`roads/drawn.rs`): **узловые**
/// осевые (`Axis::Nodal` — после сглаживания, до стежков; мост и арка не
/// участвуют, [`rounded`]); тротуар по стороне, если он рисуется
/// (`Drawn::sidewalk_on`); лежит ли рядом вторая половина разделённой
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
        // клинья так, как их нарежет лента
        let fitted = super::tapers::fit(total, [head, tail].map(Some));
        let closed = path[0] == path[path.len() - 1];
        let last = path.len() - 1;
        // вершины, которые оставит лента (`meshing::ribbon_vertices`): край
        // ленты прямой между ними, а не между вершинами оси
        let drawn_vertices = if closed || fitted == [None; 2] {
            ribbon_vertices(path, closed, ribbon_merge_distance(road.width))
        } else {
            let ends = [0, 1].map(|end| Some((fitted[end]?, roads[wedges[end]?.narrow].width)));
            wedged_vertices(path, total, ends, road.width)
        };
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
            // Луч меряется по вершинам **ленты**: она сливает точки оси ближе
            // четверти своей ширины, и край идёт прямо до следующей
            // оставленной. Направление на слитую вершину расходилось с краем
            // ленты — у широкой улицы на градусы, и сторона скругления
            // отходила от края светлой щелью (Тула, Сойфера × Лейтейзена);
            // прямой край, продлённый через слитую вершину на изгибе оси,
            // выводил дугу за кромку асфальтовым язычком (Халтурина ×
            // Гоголевская). Узел, который лента сама слила, — по вершинам оси.
            let on_ribbon = |index: usize| drawn_vertices[index] || !drawn_vertices[vertex];
            for forward in [true, false] {
                let mut at = vertex;
                let mut next = None;
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    at = index;
                    if on_ribbon(index) && path[index].distance(node) >= MIN_ARM {
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
                    at = index;
                    if !on_ribbon(index) {
                        continue;
                    }
                    let offset = path[index] - node;
                    let along = offset.dot(direction);
                    if along <= run || direction.perp_dot(offset).abs() > STRAIGHT_TOLERANCE {
                        break;
                    }
                    run = along;
                }
                // первый общий узел по оси, докуда может дотянуться хвост
                // площадки у бордюра: за ним дорогу пересекает дорожка или
                // проезд
                let mut crossed = f32::INFINITY;
                let (mut at, mut reach) = (vertex, 0.0);
                while let Some(index) = step(at, forward) {
                    if index == vertex || reach > run + KERB_PAD_RUN {
                        break;
                    }
                    reach += path[index].distance(path[at]);
                    at = index;
                    if nodes.is_shared(path[index]) {
                        crossed = reach;
                        break;
                    }
                }
                let at_end = !closed && (vertex == 0 || vertex == last);
                // Клин, под которым стоит узел: у торца — свой, даже не
                // легший; дальше — тот, на чьей длине узел лежит. Узел
                // примыкания бывает и внутри клина (Тула, Сойфера у Фёдора
                // Смирнова: в 14 м от шва на клине в 33 м) — там кромка уже
                // не своя и не соседа, а посередине, и скругление, построенное
                // на своей полуширине, стояло за кромкой, открывая клин
                // тротуара в проезжей части. `(торец, клин, длина, от шва)`
                let under = (!closed)
                    .then(|| {
                        [0, 1].into_iter().find_map(|end| {
                            let wedge = wedges[end]?;
                            let from_seam = if end == 0 { along } else { total - along };
                            match fitted[end] {
                                Some(length) if from_seam < length => {
                                    Some((end, wedge, length, from_seam))
                                }
                                None if at_end && vertex == end * last => {
                                    Some((end, wedge, 0.0, 0.0))
                                }
                                _ => None,
                            }
                        })
                    })
                    .flatten();
                // Кромка прямая только до клина: дальше лента сужается, и
                // касательная, заведённая в клин, торчала из-под него шипом
                // асфальта и тротуара (пример 08, улица в 9 м с клином к
                // однополосной). В клине — до его конца или до шва.
                if !closed {
                    let straight = match under {
                        Some((end, _, length, from_seam)) if length > 0.0 => {
                            if forward == (end == 0) {
                                length - from_seam
                            } else {
                                from_seam
                            }
                        }
                        _ if forward => total - tail - along,
                        _ => along - head,
                    };
                    run = run.min(straight.max(0.0));
                }
                // стороны луча: слева по пути — слева по лучу вперёд и справа
                // по лучу назад; тротуар — по тегу со своей стороны пути
                let mut sides =
                    [0, 1].map(|at| drawn.sidewalk_on(index, usize::from((at == 0) != forward)));
                // под клином: с сужаемых сторон кромка в узле — между узким
                // соседом и своей, по месту на клине (у шва — соседа, и
                // тротуар там тоже его), и идёт наклонно — от шва к своей
                // полуширине, к шву — к соседской; с сохранённой — своя
                let own = road.width / 2.0;
                let (mut half, mut reach, mut slope) = ([own; 2], [own; 2], [0.0; 2]);
                let mut widening = 0.0;
                if let Some((end, wedge, length, from_seam)) = under {
                    let away = forward == (end == 0);
                    // клин не лёг — кромка сразу своя
                    widening = match (length > 0.0, away) {
                        (false, _) => f32::MIN_POSITIVE,
                        (true, true) => length - from_seam,
                        (true, false) => from_seam,
                    };
                    let narrow = roads[wedge.narrow];
                    let narrow_half = narrow.width / 2.0;
                    let share = if length > 0.0 {
                        from_seam / length
                    } else {
                        0.0
                    };
                    for (side, tapered) in wedge.sides.into_iter().enumerate() {
                        if !tapered {
                            continue;
                        }
                        let at = usize::from((side == 0) != forward);
                        half[at] = narrow_half + (own - narrow_half) * share;
                        reach[at] = if away { own } else { narrow_half };
                        if length > 0.0 && widening > 0.0 {
                            slope[at] = (reach[at] - half[at]) / widening;
                        }
                        if from_seam == 0.0 {
                            sides[at] = drawn.sidewalk_on(wedge.narrow, side);
                        }
                    }
                }
                // кусок пары кончается там, где пробы перестали её находить:
                // до узла — не дальше двух проб
                if let Some(left) = drawn.pairs().beside(index, along, 2.0 * PROBE_STEP) {
                    sides[usize::from(left != forward)] = None;
                }
                // обочины — слева и справа по пути, как тротуар по тегу; у
                // узла — какая она там по месту (`RoadLine::verge_at`)
                let mut verge = [None; 2];
                let scale = polyline_length(&road.points) / total.max(f32::EPSILON);
                for (side, width) in drawn.verges_drawn(index).into_iter().enumerate() {
                    verge[usize::from((side == 0) != forward)] =
                        (width > 0.0).then(|| road.verge_at(side, along * scale));
                }
                entry.1.push(Arm {
                    class: road.class,
                    highway: road.highway,
                    ring: drawn.on_ring(index),
                    paved: road.is_paved_path(),
                    unpaved: road.is_unpaved_street(),
                    half,
                    sidewalk: sides,
                    verge,
                    direction,
                    run,
                    crossed,
                    end: at_end.then_some((index, usize::from(vertex == last))),
                    full: own,
                    reach,
                    widening,
                    slope,
                    path,
                    vertex,
                    forward,
                });
            }
        }
    }

    let mut returns = KerbReturns {
        butt: vec![[false; 2]; roads.len()],
        setback: vec![[0.0; 2]; roads.len()],
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
            // Асфальт, продолженный грунтовкой, обрывается поперёк: круглый
            // торец ленты лежал на грунте полукругом. Оба торца прямые, а
            // щель с наружной стороны излома закрывает грунт — под асфальтом.
            if let [first, second] = group[..]
                && first.unpaved != second.unpaved
                && !is_junction(&group)
                && let (Some((a, a_end)), Some((b, b_end))) = (first.end, second.end)
                && !is_merged(first)
                && !is_merged(second)
            {
                returns.butt[a][a_end] = true;
                returns.butt[b][b_end] = true;
                for (first, second) in pairs(&group) {
                    let halves = (first.half[0], second.half[1]);
                    if let Some(outline) = outer_corner(node, first, second, halves) {
                        returns.unpaved.push(outline);
                    }
                }
                continue;
            }
            // узел одного слияния — продолжение дороги, а не перекрёсток
            if !is_junction(&group) || group.iter().all(|arm| is_merged(arm)) {
                continue;
            }
            for arm in &group {
                if let Some((road, end)) = arm.end.filter(|_| !is_merged(arm)) {
                    returns.butt[road][end] = true;
                }
            }
            // Асфальт, упёршийся в грунтовку, кончается на её кромке: лента
            // до узла лежала поверх грунта языком до оси грунтовки, с двумя
            // уступами там, где её торец шире скруглений (Тула, 13). Только
            // когда асфальтовое плечо в узле одно — асфальт, пересекающий
            // грунтовку, идёт через неё.
            let paved: Vec<&&Arm> = group.iter().filter(|arm| !arm.unpaved).collect();
            let dirt = group.iter().filter(|arm| arm.unpaved).map(|arm| arm.full);
            if class == RoadClass::Street
                && let [arm] = paved[..]
                && let Some((road, end)) = arm.end.filter(|_| !is_merged(arm))
                && group.len() - paved.len() >= 2
            {
                returns.setback[road][end] = dirt.fold(0.0, f32::max);
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
                // угол двух грунтовок — грунтом; грунтовки с асфальтом сюда
                // доходит только наружным или носом — тоже грунтом
                let fill = if paved {
                    Fill::Sidewalk
                } else if first.unpaved || second.unpaved {
                    Fill::Unpaved
                } else {
                    Fill::Road(class)
                };
                // Грунтовка входит в асфальт без скругления: асфальт идёт
                // прямо, и его кромка — край грунтовки (Калуга, 07). Асфальтовый
                // веер у её устья читался отводом от асфальтовой улицы. Всё,
                // что между ними всё же закрывается, — грунтом, под асфальтом.
                let mixed = first.unpaved != second.unpaved;
                let round = if mixed {
                    None
                } else {
                    fillet(node, first, second, halves, radius)
                };
                // прямая кромка короче касательной — дуга на гнутые кромки
                if round.is_none()
                    && !mixed
                    && let Some(outline) = bend(first, second, halves, radius)
                {
                    returns.bends.push((fill, outline));
                    continue;
                }
                // тупой угол со сменой ширины — клин широкой кромки
                let round = round.or_else(|| {
                    (!mixed)
                        .then(|| obtuse_corner(node, first, second, halves))
                        .flatten()
                });
                let (outline, outer) = match round {
                    Some(outline) => (outline, false),
                    None if is_nose(first, second) => {
                        let radius = nose_radius(radius, scale);
                        // клин штрихуется только на развилке — узел из трёх
                        // лучей, острый угол, а не нос несложившегося
                        // скругления — и между проезжими частями улиц, а не
                        // у проезда, дорожки или грунтовки. На сложном узле
                        // клинья легли островками посреди его асфальта (Орёл,
                        // витрина 04), у развилки под 34° — клочком в горле
                        // узла (Тула, 06)
                        let gored = fill == Fill::Road(RoadClass::Street)
                            && group.len() == 3
                            && ccw_angle(first, second) < MIN_ANGLE
                            && [first, second]
                                .iter()
                                .all(|arm| arm.highway.is_street() && !arm.ring);
                        if let Some(found) = nose(first, second, (0.0, 0.0), radius, gored) {
                            returns.noses.push((fill, found.outline));
                            returns.fork_gores.extend(found.gore);
                        }
                        continue;
                    }
                    None => match outer_corner(node, first, second, halves) {
                        Some(outline) => (outline, true),
                        None => continue,
                    },
                };
                match fill {
                    Fill::Sidewalk => returns.sidewalks.push(outline),
                    Fill::Unpaved => returns.unpaved.push(outline),
                    Fill::Road(class) => returns.roads.push((class, outline)),
                }
                returns.outer[usize::from(paved)] += usize::from(outer);
            }
            // Асфальт, идущий сквозь грунтовку двумя way с изломом: торцы
            // прямые, скруглений к грунту нет, и щель с наружной стороны
            // излома светилась грунтом тонкой чертой поперёк асфальта (Калуга,
            // 06: шов участков с `lane_markings=no` на переезде гравийки).
            // Закрывает её тот же наружный веер, что у продолжения дороги.
            let paved: Vec<&Arm> = group.iter().copied().filter(|arm| !arm.unpaved).collect();
            if paved.len() < group.len()
                && paved.len() == 2
                && !is_junction(&paved)
                && paved.iter().all(|arm| arm.end.is_some() && !is_merged(arm))
            {
                for (first, second) in pairs(&paved) {
                    let halves = (first.half[0], second.half[1]);
                    if let Some(outline) = outer_corner(node, first, second, halves) {
                        returns.roads.push((class, outline));
                        returns.outer[0] += 1;
                    }
                }
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
                    .or_else(|| obtuse_corner(node, first, second, halves))
                    .or_else(|| outer_corner(node, first, second, halves))
                {
                    if a.min(b) > VERGE_PAVED_MAX {
                        returns.verge_lawns.push(outline);
                    } else {
                        returns.verges.push(outline);
                    }
                }
                // у газона угол, куда выходят зебры, — площадкой плитки вдоль
                // бордюрной дуги, иначе переход кончался на траве серпом
                // между двумя газонами (Тула, 01)
                if a.max(b) > VERGE_PAVED_MAX {
                    let road = (first.half[0], second.half[1]);
                    let radius = kerb_radius(first, second) * scale;
                    returns
                        .verges
                        .extend(kerb_pad(node, first, second, road, radius));
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
        // две полосы одной улицы сквозь узел, где к ней примыкает улица без
        // тротуара: сами по себе они продолжение, но торцы у них прямые —
        // узел улиц перекрёсток, — и без угла между ними через тротуар шла
        // нить (Орёл, витрина 04, север)
        let streets: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Street)
            .collect();
        let butted = walked.len() == 2
            && walked.iter().all(|arm| arm.end.is_some())
            && is_junction(&streets);
        if !(is_junction(&walked) || butted) || walked.iter().all(|arm| is_merged(arm)) {
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
            } else if let Some(outline) = bend(first, second, halves, radius) {
                returns.bends.push((Fill::Sidewalk, outline));
            } else if let Some(outline) = obtuse_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
            } else if is_nose(first, second) {
                // У носа концентричная дуга ушла бы в минус — остриё островка
                // всё мощёное, — и нос тротуара свой, того же малого радиуса:
                // иначе между полосами тротуара торчал бы шип земли.
                let radius = nose_radius(kerb_radius(first, second) * scale, scale);
                if let Some(found) = nose(first, second, (a, b), radius, false) {
                    returns.noses.push((Fill::Sidewalk, found.outline));
                }
            } else if let Some(outline) = outer_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
                returns.outer[1] += 1;
            }
        }
    }
    returns
}

/// Вершины разомкнутой оси `path` (длиной `total`), которые оставит лента
/// дороги шириной `width` с клиньями `ends` (`[у начала, у конца]`: длина,
/// как её нарежет лента, и ширина узкого соседа). Лента кладётся кусками
/// (`roads.rs`, `tapers::split`): тело сливает точки на
/// [`ribbon_merge_distance`] своей ширины, клин — на половине узкой
/// полуширины (`MeshBuilder::push_taper_sided`) и от шва; каждый кусок — со
/// своих концов. Вершина на стыке кусков остаётся.
fn wedged_vertices(
    path: &[Vec2],
    total: f32,
    ends: [Option<(f32, f32)>; 2],
    width: f32,
) -> Vec<bool> {
    let (along, _) = arclengths(path);
    let mut kept = vec![false; path.len()];
    let [head, tail] = ends.map(|end| end.map_or(0.0, |(length, _)| length));
    let mut pieces = vec![(head, total - tail, false, ribbon_merge_distance(width))];
    if let Some((length, narrow)) = ends[0] {
        pieces.push((0.0, length, false, narrow / 4.0));
    }
    if let Some((length, narrow)) = ends[1] {
        pieces.push((total - length, total, true, narrow / 4.0));
    }
    for (from, to, reversed, distance) in pieces {
        // точки куска и чьи они: вершина оси или точка разреза
        let mut points: Vec<(Vec2, Option<usize>)> = Vec::new();
        let place = |at: f32| place_on_path(path, &along, at).map(|(point, _)| point);
        let vertex_at = |at: f32| along.iter().position(|&value| value == at);
        match vertex_at(from) {
            Some(index) => points.push((path[index], Some(index))),
            None => points.extend(place(from).map(|point| (point, None))),
        }
        for (index, &at) in along.iter().enumerate() {
            if at > from && at < to {
                points.push((path[index], Some(index)));
            }
        }
        match vertex_at(to) {
            Some(index) => points.push((path[index], Some(index))),
            None => points.extend(place(to).map(|point| (point, None))),
        }
        if reversed {
            points.reverse();
        }
        let line: Vec<Vec2> = points.iter().map(|&(point, _)| point).collect();
        for ((_, index), keep) in points.iter().zip(ribbon_vertices(&line, false, distance)) {
            if let Some(index) = *index {
                kept[index] |= keep;
            }
        }
    }
    kept
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
/// [`MAJOR_RADIUS`]); между двумя грунтовками — не больше [`DIRT_RADIUS`].
fn kerb_radius(first: &Arm, second: &Arm) -> f32 {
    let radius = class_radius(first).min(class_radius(second));
    if first.unpaved && second.unpaved {
        radius.min(DIRT_RADIUS)
    } else {
        radius
    }
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
    outline.push(node - middle * OUTER_OVERLAP);
    outline.push(node + from * halves.0 + first.direction * OUTER_OVERLAP);
    for step in 1..steps {
        let share = step as f32 / steps as f32;
        let radius = halves.0 + (halves.1 - halves.0) * share;
        outline.push(node + Vec2::from_angle(sweep * share).rotate(from) * radius);
    }
    outline.push(node - second.direction.perp() * halves.1 + second.direction * OUTER_OVERLAP);
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
    radius: f32,
) -> Option<Vec<Vec2>> {
    let arc = fillet_arc(node, first, second, halves, radius)?;
    // прямые стороны заходят под ленты на `OVERLAP`: сторона, совпадающая с
    // краем ленты, но не делящая с ней вершин, растеризуется с пропусками —
    // по краю проезда шла пунктирная щель со светлым тротуаром под ней
    let mut outline = Vec::with_capacity(arc.steps + 4);
    outline.push(arc.corner - (arc.side_first + arc.side_second) * OVERLAP);
    outline.push(arc.on_first - arc.side_first * OVERLAP);
    outline.extend(arc.points(0.0));
    outline.push(arc.on_second - arc.side_second * OVERLAP);
    Some(outline)
}

/// **Изогнутое скругление** (bend) угла от луча `first` против часовой стрелки
/// до луча `second` — там, где прямое ([`fillet_arc`]) не легло, потому что
/// кромка луча прямая меньше касательной: дуга кольца (кольцо Ø 29 м идёт
/// хордами по два метра, и прямой пробег его луча — одна хорда), подход,
/// загнутый к кольцу (`rings::bend_approach`). Без него угол въезда в
/// кольцо оставался ступенькой, и из-под неё торчал торец полосы тротуара
/// клином (Белгород, Чапаева у кольца, R26).
///
/// Кромки — **осевые лучей** ([`Arm::trail`]), сдвинутые на полуширины
/// `halves`, как у носа; угол — их первое пересечение от узла, дуга радиуса
/// `radius` садится на обе кромки: центр сдвигается [`BEND_SETTLE`] раз, пока
/// не встанет на радиус от каждой. Касание не нашлось в [`BEND_REACH`] — `None`.
/// Контур — от угла по кромке первого до касания, дуга, по кромке второго
/// назад; стороны заходят под ленты на [`OVERLAP`]. Он не выпукл (кромка
/// кольца выгнута в угол) — триангулируется целиком ([`KerbReturns::bends`]).
/// Клин под лучом ([`Arm::slope`]) — не сюда: у него кромка своя. Угол
/// острее [`NOSE_MAX_ANGLE`] — тоже: там нос, а у кольца подход, влитый под
/// 25° (`rings::ENTRY_ANGLE`), — это клин островка (`roads/gores.rs`), и дуга
/// в шесть метров заливала бы его асфальтом на двадцать метров вглубь.
fn bend(first: &Arm, second: &Arm, halves: (f32, f32), radius: f32) -> Option<Vec<Vec2>> {
    if radius < MIN_RADIUS || first.slope[0] != 0.0 || second.slope[1] != 0.0 {
        return None;
    }
    if !(NOSE_MAX_ANGLE..=MAX_ANGLE).contains(&ccw_angle(first, second)) {
        return None;
    }
    // кромка и она же под лентой на `BEND_OVERLAP` — вершина в вершину
    let edges = |arm: &Arm, half: f32| -> [Vec<Vec2>; 2] {
        let trail = within(&arm.trail(), BEND_REACH);
        [half, half - BEND_OVERLAP * half.signum()].map(|half| {
            trail
                .iter()
                .zip(miter_offsets(&trail, false, half))
                .map(|(point, offset)| *point + offset)
                .collect()
        })
    };
    // слева по ходу первого, справа по ходу второго
    let [first_edge, first_under] = edges(first, halves.0);
    let [second_edge, second_under] = edges(second, -halves.1);
    let (corner, [from_first, from_second]) = first_crossing(&first_edge, &second_edge)?;
    // кромки от угла прочь от узла
    let chain = |edge: &[Vec2], from: usize| -> Vec<Vec2> {
        std::iter::once(corner)
            .chain(edge[from + 1..].iter().copied())
            .collect()
    };
    let (first_chain, second_chain) = (
        chain(&first_edge, from_first),
        chain(&second_edge, from_second),
    );
    let direction = |chain: &[Vec2]| (chain[1] - chain[0]).try_normalize();
    let (along_first, along_second) = (direction(&first_chain)?, direction(&second_chain)?);
    let half_angle = along_first.angle_to(along_second).abs() / 2.0;
    if half_angle < MIN_ANGLE / 2.0 {
        return None;
    }
    let mut centre =
        corner + (along_first + along_second).try_normalize()? * (radius / half_angle.sin());
    for _ in 0..BEND_SETTLE {
        let (first_foot, _) = nearest_on_path(&first_chain, centre)?;
        let (second_foot, _) = nearest_on_path(&second_chain, centre)?;
        let (away_first, away_second) = (centre - first_foot, centre - second_foot);
        centre += away_first.normalize_or_zero() * (radius - away_first.length())
            + away_second.normalize_or_zero() * (radius - away_second.length());
    }
    let (first_foot, first_cut) = nearest_on_path(&first_chain, centre)?;
    let (second_foot, second_cut) = nearest_on_path(&second_chain, centre)?;
    let (_, first_total) = arclengths(&first_chain);
    let (_, second_total) = arclengths(&second_chain);
    // касание — на кромках за углом и до конца поиска, дуга — в радиус
    let touches = |foot: Vec2, cut: f32, total: f32| {
        cut > OVERLAP && cut < total - OVERLAP && (centre.distance(foot) - radius).abs() < BEND_FIT
    };
    if !touches(first_foot, first_cut, first_total)
        || !touches(second_foot, second_cut, second_total)
    {
        return None;
    }
    // стороны — по кромкам под лентами: вершины кромки до касания
    let under = |edge: &[Vec2], under: &[Vec2], from: usize, cut: f32| -> Vec<Vec2> {
        let mut along = corner.distance(edge[from + 1]);
        let mut points = Vec::new();
        for index in from + 1..edge.len() {
            if index > from + 1 {
                along += edge[index].distance(edge[index - 1]);
            }
            if along >= cut {
                break;
            }
            points.push(under[index]);
        }
        points
    };
    let inward = |foot: Vec2| (foot - centre).normalize_or_zero() * BEND_OVERLAP;
    let mut outline =
        vec![corner + (along_first + along_second).normalize_or_zero() * -BEND_OVERLAP];
    outline.extend(under(&first_edge, &first_under, from_first, first_cut));
    outline.push(first_foot + inward(first_foot));
    let (from, to) = (first_foot - centre, second_foot - centre);
    let sweep = from.angle_to(to);
    let steps = arc_steps(radius, sweep.abs()).max(1);
    outline
        .extend((0..=steps).map(|step| {
            centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from)
        }));
    outline.push(second_foot + inward(second_foot));
    let mut back = under(&second_edge, &second_under, from_second, second_cut);
    back.reverse();
    outline.extend(back);
    (outline.len() >= 4 && ring_area(&outline) > 0.01).then_some(outline)
}

/// **Тупой угол со сменой ширины** от луча `first` против часовой стрелки до
/// луча `second`, [`MAX_ANGLE`]..[`OBTUSE_MAX`] (R13): почти продолжение
/// дороги, но в узле перекрёстка, где торцы прямые, а ширины разные. Кромка
/// широкого плеча, продлённая за узел, сходится с кромкой узкого только за
/// узлом по узкому — скругление ([`fillet_arc`]) там не ляжет (угол краёв
/// позади торца широкого), и между прямым торцом широкого и кромкой узкого
/// оставался клин земли, асфальт обрывался срезом (Тула, Одоевский
/// путепровод × Демонстрации, 162°).
///
/// Контур — клин от торца широкого по его продлённой кромке до пологой дуги,
/// касательной к обеим кромкам: касание на широком — не дальше его торца,
/// на узком — не дальше его прямого края ([`Arm::run`]). Первая вершина —
/// в узле чуть под лентой широкого, стороны заходят под ленты на
/// [`OVERLAP`]; веером из первой вершины. Сквозная дорога (оба луча — одна
/// ось), клин под лучом и равные ширины (кромки сходятся перед узлом по
/// обоим лучам) — `None`.
fn obtuse_corner(node: Vec2, first: &Arm, second: &Arm, halves: (f32, f32)) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= MAX_ANGLE || angle >= OBTUSE_MAX {
        return None;
    }
    if std::ptr::eq(first.path, second.path) || first.slope[0] != 0.0 || second.slope[1] != 0.0 {
        return None;
    }
    // только кромка проезжей части и не у кольца: у дорожек ширины разные по
    // классу покрытия, а две дуги кольца — одна замкнутая лента со своим
    // тротуаром (`roads::push_ring_edges`), и клин по тротуарам дуг разной
    // ширины ложился плиткой поперёк газона острова (Тула, кадр R15)
    if first.class != RoadClass::Street
        || second.class != RoadClass::Street
        || first.ring
        || second.ring
    {
        return None;
    }
    // кромки: слева у первого, справа у второго — `node + n·h + u·x`
    let (u1, u2) = (first.direction, second.direction);
    let (n1, n2) = (u1.perp(), -u2.perp());
    let rhs = n2 * halves.1 - n1 * halves.0;
    let determinant = -u1.perp_dot(u2);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-u2) / determinant;
    let s = u1.perp_dot(rhs) / determinant;
    // угол краёв — позади торца широкого и на прямом крае узкого
    let (wide, narrow, behind, on) = match (t >= 0.0, s >= 0.0) {
        (true, false) => (second, first, -s, t),
        (false, true) => (first, second, -t, s),
        _ => return None,
    };
    wide.end?;
    let corner = node + n1 * halves.0 + u1 * t;
    let tangent = behind.min(narrow.run - on);
    if tangent <= OVERLAP {
        return None;
    }
    let half_angle = angle / 2.0;
    let radius = tangent * half_angle.tan();
    let (on_first, on_second) = (corner + u1 * tangent, corner + u2 * tangent);
    let centre = corner + (u1 + u2).normalize() * (radius / half_angle.sin());
    let (from, to) = (on_first - centre, on_second - centre);
    let sweep = from.angle_to(to);
    let steps = arc_steps(radius, sweep.abs()).max(1);
    let (wide_normal, wide_half) = if std::ptr::eq(wide, first) {
        (n1, halves.0)
    } else {
        (n2, halves.1)
    };
    let narrow_normal = if std::ptr::eq(wide, first) { n2 } else { n1 };
    let narrow_half = if std::ptr::eq(wide, first) {
        halves.1
    } else {
        halves.0
    };
    // торец широкого — под его лентой, кромка узкого — под своей
    let wide_end = node + wide_normal * (wide_half - OVERLAP) + wide.direction * OVERLAP;
    let narrow_end = node + narrow_normal * (narrow_half - OVERLAP);
    let arc = (0..=steps)
        .map(|step| centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from));
    let mut outline = Vec::with_capacity(steps + 5);
    outline.push(node + wide.direction * OUTER_OVERLAP);
    if std::ptr::eq(wide, first) {
        outline.push(wide_end);
        outline.extend(arc);
        outline.push(on_second - n2 * OVERLAP);
        outline.push(narrow_end);
    } else {
        outline.push(narrow_end);
        outline.push(on_first - n1 * OVERLAP);
        outline.extend(arc);
        outline.push(wide_end);
    }
    (ring_area(&outline).abs() > 0.01).then_some(outline)
}

/// Начало ломаной `path` длиной до `reach` м (последнее звено — целиком).
fn within(path: &[Vec2], reach: f32) -> Vec<Vec2> {
    let mut along = 0.0;
    let mut kept = Vec::with_capacity(path.len());
    for (index, &point) in path.iter().enumerate() {
        if index > 0 {
            along += point.distance(path[index - 1]);
        }
        kept.push(point);
        if along >= reach {
            break;
        }
    }
    kept
}

/// Первое пересечение ломаных `a` и `b` по ходу `a` (а при равенстве — `b`):
/// точка и номера звеньев, на которых оно лежит.
fn first_crossing(a: &[Vec2], b: &[Vec2]) -> Option<(Vec2, [usize; 2])> {
    for i in 0..a.len().saturating_sub(1) {
        let (p, r) = (a[i], a[i + 1] - a[i]);
        let mut best: Option<(f32, f32, usize)> = None;
        for j in 0..b.len().saturating_sub(1) {
            let (q, s) = (b[j], b[j + 1] - b[j]);
            let denominator = r.perp_dot(s);
            if denominator.abs() < 1e-9 {
                continue;
            }
            let t = (q - p).perp_dot(s) / denominator;
            let u = (q - p).perp_dot(r) / denominator;
            if (0.0..=1.0).contains(&t)
                && (0.0..=1.0).contains(&u)
                && best.is_none_or(|(bt, ..)| t < bt)
            {
                best = Some((t, u, j));
            }
        }
        if let Some((t, _, j)) = best {
            return Some((p + r * t, [i, j]));
        }
    }
    None
}

/// Площадка плитки у бордюрной дуги угла между обочинами с газоном
/// ([`KERB_PAD_WIDTH`]): кольцевой сектор за дугой скругления дорог
/// (`halves`, `radius` — те же, что у него; дуга уже площадки — весь угол на
/// её ширину от обеих кромок) и по прямому хвосту [`KERB_PAD_RUN`] вдоль
/// края каждой дороги — не дальше дороги, которая её пересекает
/// ([`Arm::crossed`]). Куски — выпуклые, каждый веером из первой вершины.
fn kerb_pad(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    radius: f32,
) -> Vec<Vec<Vec2>> {
    let Some(arc) = fillet_arc(node, first, second, halves, radius) else {
        return Vec::new();
    };
    // Ширина площадки у каждого луча — не шире его полосы тротуара, если
    // она есть: площадка в 3 м за тротуаром в 2.5 выходила в газон уступом
    // в полметра у конца хвоста и у начала дуги (Тула, Фёдора Смирнова у
    // Красноармейского, R1). Где тротуара нет, газон подходит к бордюру, и
    // площадка — во всю [`KERB_PAD_WIDTH`].
    let width = |sidewalk: Option<f32>| sidewalk.map_or(KERB_PAD_WIDTH, |s| s.min(KERB_PAD_WIDTH));
    let widths = [width(first.sidewalk[0]), width(second.sidewalk[1])];
    // кромка заходит под асфальт на `OVERLAP`, внутренняя дуга — того же
    // центра, на ширину площадки ближе к нему (от ширины у первого луча к
    // ширине у второго)
    let outer: Vec<Vec2> = arc.points(OVERLAP).collect();
    let sides = [arc.side_first, arc.side_second];
    let alongs = [arc.along_first, arc.along_second];
    let mut pieces: Vec<Vec<Vec2>> = if arc.radius > widths[0].max(widths[1]) {
        let inner: Vec<Vec2> = arc.points_between(-widths[0], -widths[1]).collect();
        outer
            .windows(2)
            .zip(inner.windows(2))
            .map(|(out, inn)| vec![out[0], out[1], inn[1], inn[0]])
            .collect()
    } else {
        // Дуга уже площадки: внутренняя сходилась в центр, и за ним, где у
        // луча нет хвоста (его прямой край кончился у касания), в угол
        // площадки проглядывал газон зубцом (Тула, Халтурина у Гоголевской).
        // Здесь площадка — весь угол на её ширину от обеих кромок: от дуги
        // по кромкам до задних краёв и до угла, где те сходятся.
        let determinant = sides[0].perp_dot(sides[1]);
        if determinant.abs() < 1e-6 {
            return Vec::new();
        }
        // на ширине площадки за каждой кромкой: `side_i · (p − угол) = W_i`
        let back = arc.corner
            + Vec2::new(
                widths[0] * sides[1].y - widths[1] * sides[0].y,
                widths[1] * sides[0].x - widths[0] * sides[1].x,
            ) / determinant;
        // по кромке — докуда она на ширину площадки от другой кромки
        let edge = |index: usize| {
            let reach = widths[1 - index] / alongs[index].dot(sides[1 - index]).max(1e-3);
            arc.corner + alongs[index] * reach.max(arc.tangent) - sides[index] * OVERLAP
        };
        let mut outline = vec![back, edge(0)];
        outline.extend(&outer);
        outline.push(edge(1));
        vec![outline]
    };
    // хвост — не дальше прямого края и не за дорожку, пересекающую улицу:
    // за ней снова газон, и хвост торчал за её лентой квадратом плитки в
    // газоне (Тула, Гоголевская у Халтурина, R10)
    for (on, along, side, room, width) in [
        (
            arc.on_first,
            arc.along_first,
            arc.side_first,
            first.run.min(first.crossed) - arc.t_first,
            widths[0],
        ),
        (
            arc.on_second,
            arc.along_second,
            arc.side_second,
            second.run.min(second.crossed) - arc.t_second,
            widths[1],
        ),
    ] {
        let run = KERB_PAD_RUN.min(room - arc.tangent);
        if run <= 0.0 {
            continue;
        }
        let kerb = on - side * OVERLAP;
        let back = on + side * width;
        pieces.push(vec![kerb, kerb + along * run, back + along * run, back]);
    }
    pieces
}

/// Дуга скругления ([`fillet`]): вершина угла краёв, точки касания, центр и
/// радиус — зажатый так, чтобы касательная не выходила за прямой край лучей.
struct FilletArc {
    corner: Vec2,
    on_first: Vec2,
    on_second: Vec2,
    centre: Vec2,
    radius: f32,
    tangent: f32,
    /// Где от узла вдоль лучей край сошёлся с краем, м.
    t_first: f32,
    t_second: f32,
    /// Направления кромок от угла и нормали к ним наружу, в угол.
    along_first: Vec2,
    along_second: Vec2,
    side_first: Vec2,
    side_second: Vec2,
    from: Vec2,
    turn: f32,
    sweep: f32,
    steps: usize,
}

impl FilletArc {
    /// Точки дуги того же центра, на `extra` дальше от него, чем дуга
    /// скругления, — от касания с первым лучом до касания со вторым, концы
    /// включительно. При `extra == 0` это сама дуга, точка в точку.
    fn points(&self, extra: f32) -> impl Iterator<Item = Vec2> + '_ {
        let scale = (self.radius + extra) / self.radius;
        let ends = [self.on_first, self.on_second]
            .map(|on| on + (on - self.centre).normalize_or_zero() * extra);
        (0..=self.steps).map(move |step| match step {
            0 => ends[0],
            step if step == self.steps => ends[1],
            step => {
                let rotation =
                    Vec2::from_angle(self.turn * self.sweep * step as f32 / self.steps as f32);
                self.centre + rotation.rotate(self.from) * scale
            }
        })
    }

    /// Как [`Self::points`], но отступ от дуги плывёт линейно от `from` у
    /// касания с первым лучом до `to` у касания со вторым.
    fn points_between(&self, from: f32, to: f32) -> impl Iterator<Item = Vec2> + '_ {
        (0..=self.steps).map(move |step| {
            let share = step as f32 / self.steps as f32;
            let extra = from + (to - from) * share;
            let rotation = Vec2::from_angle(self.turn * self.sweep * share);
            self.centre + rotation.rotate(self.from) * ((self.radius + extra) / self.radius)
        })
    }
}

fn fillet_arc(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    mut radius: f32,
) -> Option<FilletArc> {
    // Края, смотрящие друг на друга: у первого слева, у второго справа, — по
    // оси `u` с нормалью `n` и в клине наклонно: `e = u + n·наклон` на метр
    // оси (`Arm::slope`). Без наклона дуга под клином садилась на прямую
    // своей полуширины, мимо сужающейся кромки.
    let (axis_first, axis_second) = (first.direction, second.direction);
    let (normal_first, normal_second) = (axis_first.perp(), -axis_second.perp());
    let edge_first = axis_first + normal_first * first.slope[0];
    let edge_second = axis_second + normal_second * second.slope[1];
    let (along_first, along_second) = (edge_first.normalize(), edge_second.normalize());
    let angle = {
        let angle = along_second.to_angle() - along_first.to_angle();
        if angle <= 0.0 {
            angle + 2.0 * PI
        } else {
            angle
        }
    };
    if !(MIN_ANGLE..=MAX_ANGLE).contains(&angle) {
        return None;
    }
    let (side_first, side_second) = (along_first.perp(), -along_second.perp());
    // угол, где встречаются края: halves.0·n₁ + t·e₁ = halves.1·n₂ + s·e₂, t и
    // s — метры по оси
    let rhs = normal_second * halves.1 - normal_first * halves.0;
    let determinant = -edge_first.perp_dot(edge_second);
    if determinant.abs() < 1e-6 {
        return None;
    }
    let t = rhs.perp_dot(-edge_second) / determinant;
    let s = edge_first.perp_dot(rhs) / determinant;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    let corner = node + normal_first * halves.0 + edge_first * t;

    let half_angle = angle / 2.0;
    // касательная не длиннее прямого края ленты: за следующей вершиной край
    // уже повернул, и дуга легла бы мимо
    let tangent = (radius / half_angle.tan())
        .min((first.run - t) * edge_first.length())
        .min((second.run - s) * edge_second.length());
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
    let steps = arc_steps(radius, sweep).max(1);
    Some(FilletArc {
        corner,
        on_first,
        on_second,
        centre,
        radius,
        tangent,
        t_first: t,
        t_second: s,
        along_first,
        along_second,
        side_first,
        side_second,
        from,
        turn,
        sweep,
        steps,
    })
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
///
/// `gored` — перед носом ещё и штрихуемый клин развилки ([`fork_gore`]) по тем
/// же пробам.
fn nose(first: &Arm, second: &Arm, extra: (f32, f32), radius: f32, gored: bool) -> Option<Nose> {
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
    // кромка первого на `at`, лицом к ней — кромка второго, и просвет между
    // ними; в пробе — ещё полуширины обоих и нормали к кромкам внутрь развилки
    let mut facing = |at: f32| -> Option<(Vec2, Vec2, f32, Station)> {
        let (point, tangent) = place_on_path(&first_trail, &along_first, at)?;
        let near_half = first.half_at(0, at) + extra.0;
        let edge = point + tangent.perp() * near_half;
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
        let opposite = onto + out * half;
        let station = Station {
            at,
            edges: [edge, opposite],
            gap,
            halves: [near_half, half],
            inward: [tangent.perp(), out],
        };
        Some((edge, opposite, gap, station))
    };
    let mut tip = None;
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut stations = Vec::new();
    let mut at = 0.0;
    // середина просвета в два радиуса и докуда ещё вести стороны: основания
    // перпендикуляров из центра лягут чуть дальше неё
    let mut found: Option<(Vec2, f32)> = None;
    // проба, на которой просвет дорос до двух радиусов, — там кончается клин
    let mut widest = None;
    while at <= total_first {
        let Some((edge, opposite, gap, station)) = facing(at) else {
            break;
        };
        if gored {
            stations.push(station);
        }
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
                    widest = stations.len().checked_sub(1);
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
    let arc: Vec<Vec2> = (0..=steps)
        .map(|step| centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from))
        .collect();
    outline.extend(&arc);
    // остриё — уже первая вершина левой стороны
    outline.extend(keep(&right, cut_right).into_iter().skip(1).rev());
    if outline.len() < 3 {
        return None;
    }
    let gore = widest.and_then(|widest| fork_gore(&stations[..=widest], &arc));
    Some(Nose { outline, gore })
}

/// Нос развилки ([`nose`]) и штрихуемый клин перед ним ([`fork_gore`]).
struct Nose {
    outline: Vec<Vec2>,
    gore: Option<Vec<Vec2>>,
}

/// Проба носа ([`nose`]) в `at` метрах по первому лучу: кромки обоих лучей
/// лицом друг к другу, просвет между ними (меньше нуля — ленты
/// перекрываются), полуширины (с тротуаром, если он в кромке) и нормали к
/// кромкам внутрь развилки.
struct Station {
    at: f32,
    edges: [Vec2; 2],
    gap: f32,
    halves: [f32; 2],
    inward: [Vec2; 2],
}

/// Клин развилки короче этого, м, не штрихуется. У развилки острее 25° он
/// не короче 12 м (полоса и два радиуса носа на синус угла); короче выходит
/// между гнутыми лучами внутри сложного узла — треугольнички в 3–5 м
/// островками посреди его асфальта (Орёл, витрина 04).
const FORK_GORE_MIN: f32 = 8.0;

/// Штрихуемый клин перед носом острой развилки. Ленты двух улиц под 8–25°
/// перекрываются на десятки метров, и нос встаёт только там, где их кромки
/// разошлись, — до него между полотнами тянулся ровный асфальтовый язык
/// (Тула, витрина 14: 34 м от узла). На земле полосы расходятся раньше, а
/// между ними — штриховка. Клин начинается, где ось узкой улицы вышла из
/// полотна широкой (перекрытие кромок сошло до меньшей полуширины), и
/// расходится до ширины просвета у носа (`stations` кончаются пробой, где
/// просвет дорос до двух радиусов носа), кончаясь поперёк у вершины дуги
/// носа `arc`. Стороны
/// заходят в полотна на недостающую ширину — каждая пропорционально своей
/// полуширине, так что обе полосы сужаются заодно. `None` — клин короче
/// [`FORK_GORE_MIN`] или ось узкой не выходит из широкой до носа.
fn fork_gore(stations: &[Station], arc: &[Vec2]) -> Option<Vec<Vec2>> {
    let (widest, _) = stations.split_last()?;
    let start = stations
        .iter()
        .position(|station| station.gap >= -station.halves[0].min(station.halves[1]))?;
    let (from, to) = (stations[start].at, widest.at);
    if to - from < FORK_GORE_MIN {
        return None;
    }
    let mut sides = [Vec::new(), Vec::new()];
    for station in &stations[start..] {
        let width = widest.gap * (station.at - from) / (to - from);
        let cut = (width - station.gap).max(0.0);
        let total = station.halves[0] + station.halves[1];
        for (side, points) in sides.iter_mut().enumerate() {
            let into = cut * station.halves[side] / total;
            points.push(station.edges[side] - station.inward[side] * into);
        }
    }
    // Кончается клин поперёк, по касательной к вершине дуги носа: вдоль самой
    // дуги по бордюру обводка легла бы белой скобой с крючками у её концов.
    let tip = sides[0][0].midpoint(sides[1][0]);
    let apex = arc
        .iter()
        .copied()
        .min_by(|a, b| a.distance(tip).total_cmp(&b.distance(tip)))?;
    let along = (apex - tip).try_normalize()?;
    let reach = apex.distance(tip);
    let side = |points: &[Vec2]| -> Vec<Vec2> {
        let depth = |point: Vec2| (point - tip).dot(along);
        let mut kept = Vec::new();
        for (index, pair) in points.windows(2).enumerate() {
            // прямым сторонам вершина через [`NOSE_THIN`] проб не нужна
            if index % NOSE_THIN == 0 {
                kept.push(pair[0]);
            }
            let [near, far] = [depth(pair[0]), depth(pair[1])];
            if far >= reach {
                let share = ((reach - near) / (far - near)).clamp(0.0, 1.0);
                kept.push(pair[0].lerp(pair[1], share));
                return kept;
            }
        }
        kept.extend(points.last());
        kept
    };
    let mut outline = side(&sides[0]);
    // остриё — уже первая вершина первой стороны
    let back = side(&sides[1]);
    outline.extend(back.into_iter().skip(1).rev());
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

    /// Широкая улица сливает вершину оси ближе четверти своей ширины, и её
    /// край идёт прямо на следующую оставленную: дуга скругления кончается на
    /// этом крае, а не на прямой к слитой вершине — та уводила её конец за
    /// кромку в тротуар (Тула, Халтурина × Гоголевская, Сойфера × Лейтейзена).
    #[test]
    fn the_corner_ends_on_the_edge_the_ribbon_draws() {
        let wide = street(
            vec![
                Vec2::new(0.0, 60.0),
                Vec2::ZERO,
                Vec2::new(-0.45, -4.5),
                Vec2::new(0.0, -40.0),
            ],
            20.0,
        );
        let drive = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 5.0);
        let found = returns_of(&[wide, drive]);
        let (_, south_west) = found
            .iter()
            .find(|(_, outline)| outline[0].x < 0.0 && outline[0].y < 0.0)
            .expect("скругление юго-западного угла");
        // лента идёт от узла прямо на (0, −40): её западный край — x = −10
        let on_edge = south_west[south_west.len() - 2];
        assert!((on_edge.x + 10.0).abs() < 1e-3, "{on_edge:?}");
        assert!(
            south_west
                .iter()
                .all(|point| point.x <= -10.0 + OVERLAP + 1e-3),
            "{south_west:?}"
        );
    }

    /// Плитка у бордюра угла с газоном обочины кончается на дорожке,
    /// пересекающей улицу сразу за углом: за её лентой снова газон, и хвост
    /// площадки торчал в него квадратом плитки (Тула, Гоголевская у Халтурина).
    #[test]
    fn the_kerb_pad_stops_at_a_path_crossing_the_street() {
        use crate::map::osm::model::SidewalkSide;
        let verged = |points: Vec<Vec2>| RoadLine {
            sidewalks: [SidewalkSide::None; 2],
            verges: [12.0; 2],
            ..street(points, 12.0)
        };
        let path = RoadLine {
            class: RoadClass::Alley,
            ..street(
                vec![
                    Vec2::new(14.0, -30.0),
                    Vec2::new(14.0, 0.0),
                    Vec2::new(14.0, 30.0),
                ],
                3.0,
            )
        };
        let roads = [
            verged(vec![
                Vec2::new(-50.0, 0.0),
                Vec2::ZERO,
                Vec2::new(14.0, 0.0),
                Vec2::new(50.0, 0.0),
            ]),
            verged(vec![
                Vec2::new(0.0, -50.0),
                Vec2::ZERO,
                Vec2::new(0.0, 50.0),
            ]),
            path,
        ];
        let found = walked_returns_of(&roads, true);
        let north_east: Vec<Vec2> = found
            .verges
            .iter()
            .flatten()
            .copied()
            .filter(|point| point.x > 0.0 && point.y > 0.0)
            .collect();
        assert!(!north_east.is_empty(), "у угла нет площадки");
        // касание дуги в (12, 6), дорожка — на x = 14
        let reach = north_east.iter().map(|point| point.x).fold(0.0, f32::max);
        assert!(reach <= 14.0 + 1e-3, "{reach}");
    }

    /// Площадка у угла, где за бордюром полоса тротуара уже площадки, а за ней
    /// газон обочины: площадка — не шире тротуара, иначе у конца хвоста и у
    /// начала дуги кромка газона шла уступом (Тула, Фёдора Смирнова, R1).
    #[test]
    fn a_kerb_pad_is_no_wider_than_the_sidewalk_band() {
        let verged = |points: Vec<Vec2>| RoadLine {
            highway: Highway::Residential,
            verges: [12.0; 2],
            ..street(points, 12.0)
        };
        let roads = [
            verged(vec![
                Vec2::new(-50.0, 0.0),
                Vec2::ZERO,
                Vec2::new(50.0, 0.0),
            ]),
            verged(vec![
                Vec2::new(0.0, -50.0),
                Vec2::ZERO,
                Vec2::new(0.0, 50.0),
            ]),
        ];
        let map = map_of(&roads);
        let drawn = Drawn::for_test(&map).with_sidewalks(true);
        let sidewalk = drawn.sidewalk_on(0, 0).expect("у улицы тротуар");
        assert!(sidewalk < KERB_PAD_WIDTH - 0.1, "{sidewalk}");
        let found = kerb_returns(&drawn, 1.0);
        let pad: Vec<Vec2> = found
            .verges
            .iter()
            .flatten()
            .copied()
            .filter(|point| point.x > 0.0 && point.y > 0.0)
            .collect();
        assert!(!pad.is_empty(), "у угла нет площадки");
        // от кромки — бордюрной дуги радиуса 6 м или прямых краёв — не дальше
        // полосы тротуара
        let centre = Vec2::splat(6.0 + STREET_RADIUS);
        for point in pad {
            let off = if point.x >= centre.x {
                point.y - 6.0
            } else if point.y >= centre.y {
                point.x - 6.0
            } else {
                STREET_RADIUS - point.distance(centre)
            };
            assert!(off <= sidewalk + 0.01, "{point:?} за кромкой на {off}");
        }
    }

    /// Площадка за дугой уже своей ширины лежит на всю ширину от обеих
    /// кромок: внутренняя дуга сходилась в центр, и за ним в угол площадки
    /// проглядывал газон зубцом (Тула, Халтурина у Гоголевской).
    #[test]
    fn a_kerb_pad_behind_a_tight_arc_keeps_its_depth() {
        use crate::map::osm::model::SidewalkSide;
        let verged = |road: RoadLine| RoadLine {
            sidewalks: [SidewalkSide::None; 2],
            verges: [12.0; 2],
            ..road
        };
        // оба прямых края кончаются у касаний дуги — хвостов у площадки нет
        let roads = [
            verged(street(
                vec![
                    Vec2::new(-50.0, 0.0),
                    Vec2::ZERO,
                    Vec2::new(5.0, 0.0),
                    Vec2::new(20.0, -20.0),
                ],
                12.0,
            )),
            verged(with_highway(
                street(
                    vec![Vec2::ZERO, Vec2::new(0.0, 8.5), Vec2::new(20.0, 30.0)],
                    5.0,
                ),
                Highway::Service,
            )),
        ];
        let found = walked_returns_of(&roads, true);
        // угол кромок (2.5, 6), дуга проезда в 2.5 м — центр (5, 8.5); за ним
        // площадка идёт до 3 м от обеих кромок
        let behind = Vec2::new(5.25, 8.75);
        assert!(
            found
                .verges
                .iter()
                .any(|outline| point_in_polygon(behind, outline)),
            "{:?}",
            found.verges
        );
    }

    /// Асфальт двумя way с изломом в узле, где его пересекает грунтовка:
    /// торцы прямые, скруглений к грунту нет — щель с наружной стороны
    /// излома закрыта асфальтом, а не светится грунтом (Калуга, 06).
    #[test]
    fn asphalt_kinked_across_a_dirt_road_leaves_no_gap_at_the_seam() {
        let mut dirt = street(
            vec![Vec2::new(0.0, -50.0), Vec2::ZERO, Vec2::new(0.0, 50.0)],
            6.0,
        );
        dirt.pavement = Some(crate::map::osm::model::Pavement::Unpaved);
        let roads = [
            street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0),
            street(vec![Vec2::ZERO, Vec2::new(50.0, 5.0)], 8.0),
            dirt,
        ];
        let found = walked_returns_of(&roads, false);
        assert!(found.butt(0)[1] && found.butt(1)[0], "торцы в узле прямые");
        // между торцами с южной стороны: левее торца восточного и правее
        // западного — ни одна лента сюда не доходит
        let gap = Vec2::new(0.15, -3.5);
        assert!(
            found
                .roads
                .iter()
                .any(|(_, outline)| point_in_polygon(gap, outline)),
            "щель у излома открыта: {:?}",
            found.roads
        );
        // и только она: к грунтовке асфальт не скругляется
        assert_eq!(found.roads.len(), 1, "{:?}", found.roads);
    }

    /// Угол двух грунтовок — грунтом и малым радиусом: уличные 6 м ложились
    /// бордюрной дугой шире самого проезда (Тула, проезды Мясново). Та же
    /// крестовина в асфальте свой радиус улиц сохраняет.
    #[test]
    fn two_dirt_roads_meet_with_a_small_corner() {
        let mut dirt = crossing();
        for road in &mut dirt {
            road.pavement = Some(crate::map::osm::model::Pavement::Unpaved);
        }
        let found = walked_returns_of(&dirt, false);
        assert!(found.roads.is_empty(), "асфальта нет: {:?}", found.roads);
        assert_eq!(found.unpaved.len(), 4);
        for outline in &found.unpaved {
            let radius = right_angle_radius(outline);
            assert!((radius - DIRT_RADIUS).abs() < 1e-2, "{radius}");
        }
        let paved = returns_of(&crossing());
        assert!((right_angle_radius(&paved[0].1) - STREET_RADIUS).abs() < 1e-2);
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

    /// Узкая улица 8 м переходит в широкую 11 м клином в 30 м, и в 20 м от шва
    /// в широкую примыкает поперечная: кромка там — на 5.0 м от оси, между
    /// соседской 4 и своей 5.5, и идёт наклонно (Тула, Сойфера у Фёдора
    /// Смирнова).
    fn junction_inside_a_taper() -> KerbReturns {
        let map = map_of(&[
            street(vec![Vec2::new(0.0, 60.0), Vec2::ZERO], 8.0),
            street(
                vec![Vec2::ZERO, Vec2::new(0.0, -20.0), Vec2::new(0.0, -80.0)],
                11.0,
            ),
            street(vec![Vec2::new(0.0, -20.0), Vec2::new(40.0, -20.0)], 8.0),
        ]);
        let drawn = Drawn::for_test(&map).with_sidewalks(false).with_taper(
            1,
            0,
            Taper {
                length: 30.0,
                narrow: 0,
                sides: [true; 2],
            },
        );
        kerb_returns(&drawn, 1.0)
    }

    /// Кромка клина широкой улицы: полуширина на `y` (от шва в 0 вниз).
    fn wedge_edge(y: f32) -> f32 {
        4.0 + 1.5 * (-y / 30.0).clamp(0.0, 1.0)
    }

    #[test]
    fn a_junction_inside_a_taper_wedge_stands_on_the_wedge_edge() {
        let found = junction_inside_a_taper();
        let east: Vec<&Vec<Vec2>> = found
            .roads
            .iter()
            .map(|(_, outline)| outline)
            .filter(|outline| outline[0].x > 0.0)
            .collect();
        assert!(!east.is_empty());
        for outline in east {
            // угол — на наклонной кромке клина, чуть под лентой
            let corner = outline[0];
            assert!(
                (corner.x - (wedge_edge(corner.y) - OVERLAP)).abs() < 0.05,
                "{corner:?}"
            );
            // и касание дуги с широкой улицей — на ней же: из двух концов
            // дуги тот, что дальше от оси поперечной
            let ends = [outline[2], outline[outline.len() - 2]];
            let on_wide = if (ends[0].y + 20.0).abs() > (ends[1].y + 20.0).abs() {
                ends[0]
            } else {
                ends[1]
            };
            assert!(
                (on_wide.x - wedge_edge(on_wide.y)).abs() < 0.02,
                "{on_wide:?}"
            );
        }
    }

    #[test]
    fn an_arm_toward_the_seam_still_gets_its_corner() {
        let found = junction_inside_a_taper();
        // северо-восточный угол — между поперечной и лучом к шву
        assert!(
            found
                .roads
                .iter()
                .any(|(_, outline)| outline[0].x > 0.0 && outline[0].y > -20.0),
            "{:?}",
            found.roads
        );
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

    /// Улица из двух way с изломом в узле, где к ней примыкает проезд без
    /// тротуара: торцы её полос прямые (узел улиц — перекрёсток), и наружный
    /// угол между ними закрывает щель; без него через тротуар шла нить
    /// (Орёл, витрина 04, север).
    #[test]
    fn a_band_butted_at_a_drive_gets_its_outer_corner() {
        let west = street(vec![Vec2::new(-50.0, 0.0), Vec2::ZERO], 8.0);
        let east = street(vec![Vec2::ZERO, Vec2::new(50.0, -1.5)], 8.0);
        let drive = street(vec![Vec2::new(0.0, -50.0), Vec2::ZERO], 5.0);
        let found = walked_returns_of(&[west, east, drive], true);
        assert!(found.butt(0)[1] && found.butt(1)[0], "торцы прямые");
        assert_eq!(found.outer[1], 1, "{:?}", found.sidewalks);
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
        let drawn = Drawn::for_test(&map)
            .with_pairs(0, vec![PairRun::for_test(0.0, 100.0, 1, true, 0.0, true)]);
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
    fn a_sharp_street_fork_hatches_a_gore_ahead_of_its_nose() {
        // та же развилка под 15°: ленты по 8 м перекрываются до острия носа
        // в 30.6 м по биссектрисе. Клин начинается, где ось ветки вышла из
        // полотна сквозной (оси разошлись на полуширину, 4 м: ~15.5 м от
        // узла), и кончается у вершины дуги носа (Тула, витрина 14)
        let angle = 15_f32.to_radians();
        let branch = street(
            vec![Vec2::ZERO, Vec2::from_angle(angle).rotate(Vec2::X) * 80.0],
            8.0,
        );
        let through = street(
            vec![Vec2::new(-50.0, 0.0), Vec2::ZERO, Vec2::new(100.0, 0.0)],
            8.0,
        );
        let found = walked_returns_of(&[through.clone(), branch.clone()], false);
        assert_eq!(found.fork_gores.len(), 1, "{:?}", found.fork_gores);
        let gore = &found.fork_gores[0];
        let bisector = Vec2::from_angle(angle / 2.0).rotate(Vec2::X);
        let half_angle = angle / 2.0;
        let apex = 4.0 / half_angle.sin() + NOSE_RADIUS / half_angle.sin() - NOSE_RADIUS;
        let start = 4.0 / angle.sin();
        let inside = |at: f32| point_in_polygon(bisector * at, gore);
        assert!(inside(start + 3.0), "клин начался у выхода оси ветки");
        assert!(inside(apex - 1.0), "и дошёл до носа");
        assert!(!inside(start - 3.0), "у самого узла клина нет");
        assert!(
            !inside(apex + 1.0),
            "за вершиной дуги — остров, не штриховка"
        );
        // клин лежит на асфальте лент, не на земле
        for point in gore {
            let under = [&through, &branch].iter().any(|road| {
                crate::map::footprint::distance_to_polyline(*point, &road.points) <= 4.0 + 0.05
            });
            assert!(under, "{point:?} вне лент");
        }
        // у развилки с проездом клина нет: штриховка — между полотнами улиц
        let drive = street(branch.points.clone(), 5.0);
        assert!(
            walked_returns_of(&[through, drive], false)
                .fork_gores
                .is_empty()
        );
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

    /// Кольцо R = 15 м хордами по 2.4 м и улица, входящая в него по радиусу:
    /// прямой пробег луча кольца — одна хорда, короче касательной, и прямое
    /// скругление не ложилось — угол въезда оставался ступенькой, а из-под
    /// неё торчал торец полосы тротуара (Белгород, Чапаева у кольца, R26).
    #[test]
    fn a_ring_entry_gets_a_kerb_return_on_the_ring_kerb() {
        let ring_points: Vec<Vec2> = (0..=40)
            .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 40.0) * 15.0)
            .collect();
        let ring = RoadLine {
            oneway: true,
            roundabout: true,
            ..street(ring_points, 8.0)
        };
        let entry = street(vec![Vec2::new(60.0, 0.0), Vec2::new(15.0, 0.0)], 8.0);
        for sidewalks in [false, true] {
            let found = walked_returns_of(&[ring.clone(), entry.clone()], sidewalks);
            // за углом кромок въезда (y = ±4) и кольца (r = 19) — асфальт
            // скругления, по обе стороны въезда
            for y in [4.3, -4.3] {
                let outside = Vec2::new(18.8, y);
                assert!(
                    found
                        .roads
                        .iter()
                        .map(|(_, outline)| outline)
                        .chain(found.bends.iter().map(|(_, outline)| outline))
                        .any(|outline| point_in_polygon(outside, outline)),
                    "угол въезда у {outside} открыт ({sidewalks})"
                );
            }
            // и дуга не заходит на полотна глубже нахлёста
            for (_, outline) in &found.bends {
                for point in outline {
                    let from_ring = point.length() - 15.0;
                    let from_entry = point.y.abs();
                    assert!(
                        from_ring.abs() >= 4.0 - BEND_OVERLAP - 0.05
                            || from_entry >= 4.0 - BEND_OVERLAP - 0.05,
                        "{point} под обеими лентами"
                    );
                }
            }
            if sidewalks {
                // тротуар поворачивает вместе с бордюром — той же гнутой дугой
                assert!(
                    found
                        .bends
                        .iter()
                        .filter(|(fill, _)| *fill == Fill::Sidewalk)
                        .count()
                        >= 2,
                    "{:?}",
                    found.bends.iter().map(|(fill, _)| fill).collect::<Vec<_>>()
                );
            }
        }
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

    /// Узел R13 (Тула, Одоевский путепровод): широкая 14.2 м уходит на 150°,
    /// улица 7.6 м — на −12° (между ними 162°), третья 7.6 м — на юг под −54°.
    /// Направление узкой на `kink` градусов от −12°.
    fn obtuse_junction(kink: f32) -> [RoadLine; 3] {
        let ray = |degrees: f32, length: f32| Vec2::from_angle(degrees.to_radians()) * length;
        [
            street(vec![Vec2::ZERO, ray(150.0, 40.0)], 14.2),
            street(vec![Vec2::ZERO, ray(-12.0 + kink, 50.0)], 7.6),
            street(vec![Vec2::ZERO, ray(-54.0, 40.0)], 7.6),
        ]
    }

    #[test]
    fn an_obtuse_junction_pair_gets_its_corner() {
        // кромка широкой, продлённая за узел, сходится с кромкой узкой в
        // десяти метрах по узкой — между прямым торцом широкой и кромкой узкой
        // клин земли, пока его не закроет клин широкой кромки
        let found = walked_returns_of(&obtuse_junction(0.0), false);
        let along = Vec2::from_angle((-12.0_f32).to_radians());
        let wedge = along * 6.0 + along.perp() * 4.3;
        assert!(
            found
                .roads
                .iter()
                .any(|(_, outline)| point_in_polygon(wedge, outline)),
            "клин у торца широкой открыт: {:?}",
            found.roads
        );
        // и за кромкой широкой, продлённой за узел, асфальта нет
        let wide = Vec2::from_angle(150.0_f32.to_radians());
        let beyond = -wide.perp() * 7.6 - wide * 3.0;
        assert!(
            !found
                .roads
                .iter()
                .any(|(_, outline)| point_in_polygon(beyond, outline))
        );
    }

    #[test]
    fn a_street_split_at_a_junction_with_a_slight_kink_gets_no_obtuse_corner() {
        // 179° между широкой и узкой — продолжение, уступ там — дело клина
        // сечений, а не угла узла
        let found = walked_returns_of(&obtuse_junction(-17.0), false);
        let along = Vec2::from_angle((-29.0_f32).to_radians());
        let wedge = along * 6.0 + along.perp() * 4.3;
        assert!(
            !found
                .roads
                .iter()
                .any(|(_, outline)| point_in_polygon(wedge, outline))
        );
    }
}
