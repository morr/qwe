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
//!
//! **Порядок укладки слоёв — инвариант этого модуля** ([`KerbReturns::lay`]),
//! а не вызывающего: углы газонов уходят лоскутам раньше всего мощения,
//! скругления ложатся веером в свои слои — улицы, грунтовые улицы, тротуары
//! с наездами плитки, обочины, газоны широких обочин, — и последними носы и
//! дуги, триангуляцией целиком. Клинья перед носами улиц (`fork_gore`)
//! укладываются отдельно, слоем штриховки островков (`roads/gores.rs`).

use std::f32::consts::PI;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::VERGE_PAVED_MAX;
use super::drawn::{Axis, Drawn};
use super::junctions::node_key;
use super::network::pairs::PROBE_STEP;
use super::scraps::Scraps;
use super::{
    ALLEY_COLOR, Meadows, ROAD_COLOR, SIDEWALK_COLOR, UNPAVED_ROAD_COLOR, VERGE_YARD_COLOR,
};
use crate::map::along::{arclengths, nearest_on_path, place_on_path};
use crate::map::grid::Grid;
use crate::map::meshing::{
    MeshBuilder, arc_steps, butt_direction, miter_offsets, ribbon_merge_distance, ribbon_vertices,
};
use crate::map::osm::model::{
    closest_on_segment, point_in_area, polyline_length, ring_area, ring_bounds,
};
use crate::map::osm::{Highway, PolyArea, RoadClass, RoadLine};
use crate::map::spawn::GRASS_COLOR;

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
/// Тот же угол делает угловой узел узлом краски (`node_paint::is_corner`).
pub(in crate::map::roads) const MIN_ANGLE: f32 = 25.0 * PI / 180.0;
pub(in crate::map::roads) const MAX_ANGLE: f32 = 155.0 * PI / 180.0;
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
/// То же у наружного угла ([`outer_corner`]), м. Сторона по торцевому лучу
/// лежит на кромке ленты-торца (`Arm::butt`), и пяти сантиметров хватило бы;
/// но у сквозного луча торца нет — его сторона идёт по направлению луча, а
/// край ленты у узла на сглаженном изгибе уже повернул; и тротуарный веер
/// считается по тем же лучам, а его полоса шире бордюрной. Градус между
/// стороной и краем на восьми метрах полосы — полтора десятка сантиметров,
/// и пяти сантиметров нахлёста не хватало — через тротуар шла нить (Орёл,
/// витрина 04, север).
pub(super) const OUTER_OVERLAP: f32 = 0.3;

/// Наезд плитки на дворе у слияния мощёных дорожек ([`tile_overlaps`]):
/// ширина полосы за кромкой луча, м.
const TILE_OVERLAP_WIDTH: f32 = 3.0;
/// Тот же наезд: длина полосы вдоль кромки от узла, м.
const TILE_OVERLAP_REACH: f32 = 5.0;
/// Сторона луча занята и наезда на ней нет, если другой луч узла уходит в
/// неё ближе этого угла (рад): такой угол узла закрыт скруглением, носом
/// или веером, и полоса двора вдоль кромки не в щели, а снаружи угла.
const TILE_OVERLAP_OCCUPIED: f32 = 80.0_f32.to_radians();
/// Тот же наезд: расстояние от узла, на котором щель ещё не началась. Наезд
/// закрывает щель, раскрывающуюся **от узла** — за кромкой у самого узла
/// двора нет (голая земля или встречная лента), дальше — двор. Двор,
/// стоящий вплотную к кромке у узла, — не щель, и наезд ложился бы на газон
/// прямоугольной плитой (Тула, Первомайская × проспект Ленина).
const TILE_OVERLAP_MOUTH: f32 = 0.25;

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
    /// Направление исходного крайнего звена оси у торца, от узла в луч
    /// ([`meshing::butt_direction`]): прямой торец ленты идёт поперёк него,
    /// и по нему строится сторона наружного угла ([`outer_corner`]) — по
    /// направлению луча (`direction`, на вершину ленты после слияния) она
    /// отходила от торцевой кромки, и между ними оставалась щель. Только у
    /// торцевого луча; у сквозного торца нет.
    butt: Option<Vec2>,
    /// Клин дальше по лучу ([`FarTaper`]), если он есть.
    far: Option<FarTaper>,
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
    /// (`MeshBuilder::push_taper_sided`); за узлом — сужается дальним клином
    /// ([`FarTaper`], [`Arm::far`]).
    fn half_at(&self, side: usize, along: f32) -> f32 {
        let mut half = if self.widening <= 0.0 {
            self.half[side]
        } else {
            let share = (along / self.widening).min(1.0);
            self.half[side] + (self.reach[side] - self.half[side]) * share
        };
        if let Some(far) = self.far
            && far.sides[side]
            && along > far.start
        {
            let share = ((along - far.start) / far.length).min(1.0);
            half += (far.half - half) * share;
        }
        half
    }
}

/// Клин дальше по лучу: узел вне его, но кромка за узлом сужается к дальнему
/// шву way. Нос ищет кромки по [`Arm::trail`] за десятки метров от узла, и по
/// номинальной полуширине луча вставал мимо настоящего расхождения кромок —
/// штрихуемый клин ложился на плитку обочины (Тула, устье съезда с Фрунзе,
/// S7).
#[derive(Clone, Copy)]
struct FarTaper {
    /// Сколько метров от узла до начала клина.
    start: f32,
    /// Длина клина — обрезанная, как её положила лента (`tapers::fit`).
    length: f32,
    /// Полуширина узкого соседа.
    half: f32,
    /// Какие стороны луча ([`Arm::half`] — слева, справа) клин сужает.
    sides: [bool; 2],
}

/// Слой, в который ложится нос ([`KerbReturns::noses`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Road(RoadClass),
    Unpaved,
    Sidewalk,
}

/// Асфальт всех узлов. Укладка — методом модуля ([`Self::lay`]): порядок
/// слоёв — инвариант `corners`, а не вызывающего. Читатели геометрии берут
/// представления ([`Self::paving`], [`Self::streets`]), счётчики —
/// [`Self::counts`], клинья развилок — [`Self::fork_gores`]; поля закрыты
/// внутрь модуля и его тестов.
#[derive(Default)]
pub struct KerbReturns {
    /// Носы острых развилок ([`NOSE_SHARE`]) и слой каждого. Нос не выпукл и
    /// веером не кладётся: его стороны идут по кромкам лент, а они у острой
    /// развилки гнутые, — контур триангулируется целиком.
    noses: Vec<(Fill, Vec<Vec2>)>,
    /// Изогнутые скругления ([`bend`]) и слой каждого: у кольца и гнутого
    /// подхода кромка кривая, контур не выпукл — триангулируется целиком.
    bends: Vec<(Fill, Vec<Vec2>)>,
    /// Штрихуемые клинья перед носами острых развилок улиц ([`fork_gore`]):
    /// контур от острия клина до дуги носа — слою островков
    /// (`roads/gores.rs`).
    fork_gores: Vec<Vec<Vec2>>,
    /// Контур и класс дорог, в чей слой заливки он ляжет: скругления и
    /// наружные углы. Каждый — веер из первой вершины.
    roads: Vec<(RoadClass, Vec<Vec2>)>,
    /// Скругления между двумя грунтовыми улицами — в их слое, так же.
    unpaved: Vec<Vec<Vec2>>,
    /// Контуры в слое тротуаров, так же: углы полос тротуара и скругления
    /// узлов с мощёной дорожкой.
    sidewalks: Vec<Vec<Vec2>>,
    /// Наезды плитки на дворе у слияний мощёных дорожек ([`tile_overlaps`]):
    /// полоса вдоль кромки луча, за которой двор. Рисуются в слое тротуаров
    /// рядом с `sidewalks`, но лоскуты газона (`roads/scraps.rs`) ими не
    /// кормятся: наезд — сам починка, а не мощение, замыкающее лоскут.
    overlaps: Vec<Vec<Vec2>>,
    /// Контуры в слое обочин (`road_verges`), так же: углы, где с одной
    /// стороны или с обеих вместо полосы тротуара обочина.
    verges: Vec<Vec<Vec2>>,
    /// Те же углы между двумя **широкими** обочинами — газоном
    /// (`road_verge_lawns` или `road_verge_yards`), как и сами обочины: плиткой угол во все
    /// пятнадцать метров до дорожек был площадью посреди двора.
    verge_lawns: Vec<Vec<Vec2>>,
    /// Сколько из `roads` и `sidewalks` — наружные углы, а не скругления.
    outer: [usize; 2],
    /// Торцы, кончающиеся в узле, по дорогам: `[начало, конец]` — ленты с
    /// таким торцом кладутся с прямым, а не круглым.
    butt: Vec<[bool; 2]>,
    /// На сколько метров не доходит до узла заливка торца `[начало, конец]`:
    /// у асфальтовой улицы, упёршейся в грунтовку, — полуширина грунтовки.
    setback: Vec<[f32; 2]>,
}

/// Слои, в которые [`KerbReturns::lay`] кладёт асфальт узлов: по билдеру на
/// слой `build_roads`. Именованная связка, а не массив: слоёв семь, и по
/// индексу не прочесть, какой где.
pub(super) struct KerbLayers<'a> {
    /// Асфальт улиц (`roads`).
    pub(super) streets: &'a mut MeshBuilder,
    /// Песок аллей (`alleys`).
    pub(super) alleys: &'a mut MeshBuilder,
    /// Грунтовые улицы (`unpaved_roads`).
    pub(super) unpaved: &'a mut MeshBuilder,
    /// Плитка тротуаров (`sidewalks`).
    pub(super) sidewalks: &'a mut MeshBuilder,
    /// Обочины до отдельных тротуаров (`road_verges`).
    pub(super) verges: &'a mut MeshBuilder,
    /// Газон широких обочин у замапленного газона (`road_verge_lawns`).
    pub(super) lawns: &'a mut MeshBuilder,
    /// Газон широких обочин травой двора (`road_verge_yards`).
    pub(super) yards: &'a mut MeshBuilder,
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

    /// Укладка асфальта узлов в слои `layers` — порядок здесь, в модуле:
    /// сначала углы газонов уходят лоскутам (`scraps`) — мощение вокруг
    /// режет их потом, — затем скругления ложатся веером в свои слои (улицы,
    /// грунтовые улицы, тротуары с наездами плитки, обочины, газоны широких
    /// обочин), и последними носы и дуги, триангуляцией целиком. Ленты дорог
    /// кладутся поверх скруглений (`build_roads`), и этот порядок — половина
    /// той картинки. `meadows` решает, каким газоном ляжет угол широкой
    /// обочины: лугом у замапленного газона, травой двора — везде ещё.
    pub(super) fn lay(&self, layers: KerbLayers<'_>, meadows: &Meadows, scraps: &mut Scraps) {
        let KerbLayers {
            streets,
            alleys,
            unpaved,
            sidewalks,
            verges,
            lawns,
            yards,
        } = layers;
        // углы газонов — лоскутам до всего мощения: лоскут режется по мощению
        for outline in &self.verge_lawns {
            scraps.push_lawn(outline.clone());
        }
        for (class, outline) in &self.roads {
            let (builder, color) = match class {
                RoadClass::Street => (&mut *streets, ROAD_COLOR),
                RoadClass::Alley => (&mut *alleys, ALLEY_COLOR),
            };
            // Скругление не выпукло, но веер из его первой вершины — угла краёв —
            // верен: дуга между точками касания и есть та часть окружности, что
            // видна из угла. `earcutr` на восьми тысячах таких фигур стоил бы
            // больше самой укладки.
            builder.push_convex(outline, color.to_linear());
        }
        for outline in &self.unpaved {
            unpaved.push_convex(outline, UNPAVED_ROAD_COLOR.to_linear());
        }
        // и тот же угол в слое тротуаров: полоса поворачивает за бордюром
        for outline in &self.sidewalks {
            sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        // и наезды плитки на дворе у слияний мощёных дорожек — тем же слоем
        for outline in &self.overlaps {
            sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        // и в слое обочин — где вместо полосы обочина до отдельной дорожки
        for outline in &self.verges {
            verges.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        for outline in &self.verge_lawns {
            if meadows.under(outline) {
                lawns.push_convex(outline, GRASS_COLOR.to_linear());
            } else {
                yards.push_convex(outline, VERGE_YARD_COLOR.to_linear());
            }
        }
        // носы острых развилок идут по гнутым кромкам лент, и веер из острия
        // их не покрыл бы — триангуляция целиком; носов в городе сотни
        for (fill, outline) in self.noses.iter().chain(&self.bends) {
            let (builder, color) = match fill {
                Fill::Road(RoadClass::Street) => (&mut *streets, ROAD_COLOR),
                Fill::Road(RoadClass::Alley) => (&mut *alleys, ALLEY_COLOR),
                Fill::Unpaved => (&mut *unpaved, UNPAVED_ROAD_COLOR),
                Fill::Sidewalk => (&mut *sidewalks, SIDEWALK_COLOR),
            };
            builder.push_polygon(outline, &[], color.to_linear());
        }
    }

    /// Асфальт узлов, которым режутся лоскуты газона (`roads/scraps.rs`):
    /// улицы, тротуары, обочины и мощёные носы с дугами. Наружу — из
    /// [`Self::paving`]. Наезды плитки ([`Self::overlaps`]) сюда не входят:
    /// наезд — сам починка, а не мощение, замыкающее лоскут.
    pub(super) fn paving(&self) -> KerbPaving<'_> {
        KerbPaving {
            roads: &self.roads,
            sidewalks: &self.sidewalks,
            verges: &self.verges,
            noses: &self.noses,
            bends: &self.bends,
        }
    }

    /// Асфальт улиц узлов — скругления и носы в слое улиц: пустота, которую
    /// он уже замостил, — просто асфальт перекрёстка, а не пустое устье
    /// (горла штриховки, `gores::mouths`). Наружу — из [`Self::streets`].
    pub(super) fn streets(&self) -> KerbStreets<'_> {
        KerbStreets {
            roads: &self.roads,
            noses: &self.noses,
            bends: &self.bends,
        }
    }

    /// Что скругления насчитали — для отчёта сборки (`roads::RoadReport`):
    /// группы в счётчиках, наружные углы — отдельно от скруглений.
    pub(super) fn counts(&self) -> KerbCounts {
        KerbCounts {
            kerb_returns: self.roads.len() + self.unpaved.len() - self.outer[0],
            sidewalk_returns: self.sidewalks.len() - self.outer[1],
            tile_overlaps: self.overlaps.len(),
            bends: self.bends.len(),
            outer_corners: self.outer,
            noses: self.noses.len(),
        }
    }

    /// Штрихуемые клинья перед носами острых развилок улиц ([`fork_gore`])
    /// — `gores::Gores::add_forks`: асфальт под клином уже лежит (лента и
    /// нос развилки), штрихуется только контур.
    pub(super) fn fork_gores(&self) -> &[Vec<Vec2>] {
        &self.fork_gores
    }

    /// Что скругления насчитали вокруг точки — диагностике тестов
    /// (`roads/tests.rs`, `QWE_DIAG`): группы как есть, по строке на контур.
    #[cfg(test)]
    pub(super) fn dump_near(&self, at: Vec2, reach: f32) {
        let near = |outline: &Vec<Vec2>| outline.iter().any(|p| p.distance(at) < reach);
        for (class, outline) in self.roads.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr roads {class:?} {outline:?}");
        }
        for (fill, outline) in self.bends.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr bend {fill:?} {outline:?}");
        }
        for (fill, outline) in self.noses.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr nose {fill:?} {outline:?}");
        }
        for outline in self.fork_gores.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr gore {outline:?}");
        }
        for outline in self.sidewalks.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr sidewalk {outline:?}");
        }
        for outline in self.verges.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr verge {outline:?}");
        }
        for outline in self.verge_lawns.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr lawn {outline:?}");
        }
    }
}

/// Что скругления насчитали — поля `RoadReport` о них (`roads.rs`):
/// скругления без наружных углов, наезды плитки, носы, дуги, наружные углы.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct KerbCounts {
    /// Скругления асфальта улиц и грунтовых — без наружных углов.
    pub(super) kerb_returns: usize,
    /// Скругления тротуаров — без наружных углов.
    pub(super) sidewalk_returns: usize,
    /// Наезды плитки на дворе у слияний мощёных дорожек.
    pub(super) tile_overlaps: usize,
    /// Изогнутые скругления — у колец и гнутых подходов.
    pub(super) bends: usize,
    /// Наружные углы — асфальт улиц и тротуар.
    pub(super) outer_corners: [usize; 2],
    /// Носы острых развилок, всех слоёв.
    pub(super) noses: usize,
}

/// Мощение вокруг лоскутов газона — представление [`KerbReturns`] для
/// `roads/scraps.rs`: контуры, коими лоскут режется. Плоские группы и носы с
/// дугами раздельны — между ними вызывающий кладёт своё мощение (горки), и
/// порядок вклада в `Scraps` сохраняется.
#[derive(Clone, Copy)]
pub(super) struct KerbPaving<'a> {
    /// Скругления улиц и аллей; мощение — только улицы.
    roads: &'a [(RoadClass, Vec<Vec2>)],
    sidewalks: &'a [Vec<Vec2>],
    verges: &'a [Vec<Vec2>],
    noses: &'a [(Fill, Vec<Vec2>)],
    bends: &'a [(Fill, Vec<Vec2>)],
}

impl<'a> KerbPaving<'a> {
    /// Плоское мощение: скругления улиц, тротуары, обочины — веером.
    pub(super) fn flat(self) -> impl Iterator<Item = &'a [Vec2]> {
        self.roads
            .iter()
            .filter(|(class, _)| *class == RoadClass::Street)
            .map(|(_, outline)| outline.as_slice())
            .chain(self.sidewalks.iter().map(Vec::as_slice))
            .chain(self.verges.iter().map(Vec::as_slice))
    }

    /// Мощёные носы и дуги — улиц и тротуаров: грунт и аллеи лоскут не режут.
    pub(super) fn curved(self) -> impl Iterator<Item = &'a [Vec2]> {
        [self.noses, self.bends]
            .into_iter()
            .flatten()
            .filter(|(fill, _)| matches!(fill, Fill::Road(RoadClass::Street) | Fill::Sidewalk))
            .map(|(_, outline)| outline.as_slice())
    }
}

/// Асфальт улиц узлов — представление [`KerbReturns`] для горл штриховки
/// (`gores::mouths`): только слой улиц, без тротуаров и грунтовых.
#[derive(Clone, Copy)]
pub(super) struct KerbStreets<'a> {
    roads: &'a [(RoadClass, Vec<Vec2>)],
    noses: &'a [(Fill, Vec<Vec2>)],
    bends: &'a [(Fill, Vec<Vec2>)],
}

impl<'a> KerbStreets<'a> {
    /// Контуры асфальта улиц: скругления, носы и дуги.
    pub(super) fn outlines(self) -> impl Iterator<Item = &'a [Vec2]> {
        self.roads
            .iter()
            .filter(|(class, _)| *class == RoadClass::Street)
            .map(|(_, outline)| outline.as_slice())
            .chain(
                [self.noses, self.bends]
                    .into_iter()
                    .flatten()
                    .filter(|(fill, _)| matches!(fill, Fill::Road(RoadClass::Street)))
                    .map(|(_, outline)| outline.as_slice()),
            )
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
            let ends = [0, 1].map(|end| Some((fitted[end]?, roads[wedges[end]?.seam].width)));
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
                // однополосной). В клине — до его конца или до шва. Длина
                // клина — подогнанная: сырая бывает длиннее самой дороги, и
                // капа по ней обнуляла прямой пробег всех лучей дороги —
                // углам у узлов не хватало кромки, и асфальтовый угол устья
                // торчал шипом (Тула, съезд с Фрунзе в стоянку, N2).
                let [head_fitted, tail_fitted] = fitted.map(|length| length.unwrap_or(0.0));
                if !closed {
                    let straight = match under {
                        Some((end, _, length, from_seam)) if length > 0.0 => {
                            if forward == (end == 0) {
                                length - from_seam
                            } else {
                                from_seam
                            }
                        }
                        _ if forward => total - tail_fitted - along,
                        _ => along - head_fitted,
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
                    let seam = roads[wedge.seam];
                    let seam_half = seam.width / 2.0;
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
                        half[at] = seam_half + (own - seam_half) * share;
                        reach[at] = if away { own } else { seam_half };
                        if length > 0.0 && widening > 0.0 {
                            slope[at] = (reach[at] - half[at]) / widening;
                        }
                        if from_seam == 0.0 {
                            sides[at] = drawn.sidewalk_on(wedge.seam, side);
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
                // клин дальше по лучу: узел вне его (под узлом — `under`, и
                // клин перед узлом она же), кромка за узлом сужается к
                // дальнему шву way. Стороны клина — по ходу точек way, лучу —
                // по его ходу
                let far = (!closed).then(|| {
                    let end = usize::from(forward);
                    let wedge = wedges[end]?;
                    let length = fitted[end]?;
                    let to_seam = if forward { total - along } else { along };
                    (to_seam > length).then(|| FarTaper {
                        start: to_seam - length,
                        length,
                        half: roads[wedge.seam].width / 2.0,
                        sides: [
                            wedge.sides[usize::from(!forward)],
                            wedge.sides[usize::from(forward)],
                        ],
                    })
                });
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
                    butt: at_end
                        .then(|| {
                            // у конца ось кончается в узле: звено смотрит на
                            // узел, лучу нужно от него
                            butt_direction(path, vertex == last)
                                .map(|dir| if vertex == last { -dir } else { dir })
                        })
                        .flatten(),
                    far: far.flatten(),
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
    // рамки дворов — для проб наезда плитки у каждого узла ниже
    let yards = YardBounds::of(drawn.yards());
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
            // Прямой срез плеча аллеи оправдан только там, где рядом ляжет
            // уголковый кусок — то есть у самого плеча нет встречного и хоть
            // один сосед тоже кончается (пару проходных лучей кусок не
            // получает, см. пропуск ниже). Иначе срезу ничем не прикрыться, и
            // у слияния дорожек он торчал клином и ступенькой на стыке лент
            // разной ширины (Тула, Фрунзе × Красноармейский). Круглая шапка
            // ложится на соседние ленты — тот же слой, тот же цвет, — и
            // слияние читается пересечением полос. Исключение — широкий
            // торец: полудиск шапки радиусом в полуширину луча ложится на
            // узел поверх чужих лент, и там, где чужая лента уже, он
            // выступает за её кромку — клин на газоне и светлая плита
            // первого штриха зебры на соседней ленте (Тула, витрина 02:
            // зебра шире тротуаров в узле; Первомайская × проспект Ленина,
            // юг: зебра шире изгибаемого тротуара). Срез кладёт торец в
            // плоскость узла — выступать нечему, а узкие соседи закрыты
            // своими лентами.
            let squared = |arm: &Arm| {
                arm.end.is_some()
                    && group.iter().any(|other| {
                        !std::ptr::eq(*other, arm)
                            && -arm.direction.dot(other.direction) > 0.0
                            && other.half[0].min(other.half[1]) < arm.half[0].max(arm.half[1])
                    })
                    || (!continues(&group, arm)
                        && group
                            .iter()
                            .any(|other| !std::ptr::eq(*other, arm) && !continues(&group, other)))
            };
            for arm in &group {
                if class == RoadClass::Alley && !squared(arm) {
                    continue;
                }
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
                // Дорожка проходит узел насквозь — у луча есть встречный: крест
                // или устье примыкания, и ленты аллей на узлах не режутся, так
                // что угол и так кроется самим стыком. Кусок был бы заплаткой
                // поверх стыка — ступени, клинья, швы (Тула, Первомайская ×
                // проспект Ленина). Остаётся угол двух лучей, кончающихся в
                // узле, — L-поворот, развилка, — там правда есть вырез.
                if class == RoadClass::Alley
                    && (continues(&group, first) || continues(&group, second))
                {
                    continue;
                }
                let radius = kerb_radius(first, second) * scale;
                let halves = (first.half[0], second.half[1]);
                // угол двух мощёных дорожек — плиткой, в слое тротуаров:
                // песчаное скругление на стыке двух плиточных аллей читалось бы
                // пятном. Одна мощёная, вторая грунтовая — песком под мощёной
                // лентой, как у улиц грунтовка под асфальтом
                let paved = first.paved && second.paved;
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
                    // у обычного перекрёстка кромка дуги — сама линия бордюра
                    // между прямыми краями двух улиц: ленты заканчиваются у узла
                    // прямыми торцами, и кроме куска угол не кроет никто. Общий
                    // шаг читался гранями (Тула, Фёдора Смирнова ×
                    // Красноармейский, 3399 3835)
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
        // Наезд плитки на дворе у слияния мощёных дорожек (R37-01, пример 15):
        // две плиточные полосы (например, переход через улицу и дорожка двора)
        // сходятся в узле, а клин двора квартала входит остриём в щель между
        // их кромками — секторы между лучами закрывают скругления, но полосу
        // двора вдоль кромки снаружи не закрывает ничто. Наезд — полоса плитки
        // вдоль кромки каждого луча, за которой вплотную двор: кромки
        // «наезжают» на него у стыка, и шов смыкается. Сторона, где другой
        // луч ближе [`TILE_OVERLAP_OCCUPIED`], занята угловыми правилами и не
        // трогается; без двора вплотную наезда нет — полосы не раздулись.
        let tiled: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Alley && arm.paved)
            .collect();
        // проходной узел — крест дорожек — наездов не просит: полосы легли бы
        // поверх стыка кромок прямоугольниками, а не поверх щели слияния
        // (Тула, Первомайская × проспект Ленина). У примыкания или слияния
        // хоть один луч в узле кончается — там щель у двора настоящая
        let alleys: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Alley)
            .collect();
        if tiled.len() >= 2
            && !tiled.iter().all(|arm| is_merged(arm))
            && !alleys.iter().all(|arm| continues(&alleys, arm))
        {
            returns.overlaps.extend(tile_overlaps(node, &tiled, &yards));
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

/// Луч проходит узел насквозь: в группе есть встречный ему луч — направления
/// расходятся от прямого не дальше [`MAX_ANGLE`]. Не по [`Arm::end`]: OSM режет
/// дорожку на два way прямо в узле креста, и оба «кончаются», хотя дорога идёт
/// насквозь. Соосный шов двух way одной дорожки — тоже продолжение.
fn continues(group: &[&Arm], arm: &Arm) -> bool {
    group.iter().any(|other| {
        !std::ptr::eq(*other, arm) && arm.direction.angle_to(other.direction).abs() >= MAX_ANGLE
    })
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
    ccw_between(first.direction, second.direction)
}

/// Угол от направления `from` против часовой стрелки до `to`, (0, 2π].
fn ccw_between(from: Vec2, to: Vec2) -> f32 {
    let angle = to.to_angle() - from.to_angle();
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
/// на [`OUTER_OVERLAP`]. Стороны — по **торцевым кромкам лент**
/// ([`Arm::butt`]): прямой торец идёт поперёк исходного крайнего звена оси,
/// а направление луча смотрит на вершину ленты после слияния, — на стыке
/// двух way одной улицы с изломом те расходятся, и веер по лучу оставлял
/// между собой и торцами клиновидную щель поперёк асфальта (Тула, Фрунзе у
/// съезда, N1). Просвет меряется по-прежнему по лучам узловой оси: порог
/// [`MIN_OUTER`] стоит на шуме округления оси, а не торцов.
fn outer_corner(node: Vec2, first: &Arm, second: &Arm, halves: (f32, f32)) -> Option<Vec<Vec2>> {
    let angle = ccw_angle(first, second);
    if angle <= PI + MIN_OUTER {
        return None;
    }
    let sweep = angle - PI;
    let from_dir = first.butt.unwrap_or(first.direction);
    let to_dir = second.butt.unwrap_or(second.direction);
    let from = from_dir.perp();
    let steps = arc_steps(halves.0.max(halves.1), sweep).max(1);
    let middle = Vec2::from_angle(sweep / 2.0).rotate(from);
    let mut outline = Vec::with_capacity(steps + 2);
    outline.push(node - middle * OUTER_OVERLAP);
    outline.push(node + from * halves.0 + from_dir * OUTER_OVERLAP);
    for step in 1..steps {
        let share = step as f32 / steps as f32;
        let radius = halves.0 + (halves.1 - halves.0) * share;
        outline.push(node + Vec2::from_angle(sweep * share).rotate(from) * radius);
    }
    outline.push(node - to_dir.perp() * halves.1 + to_dir * OUTER_OVERLAP);
    Some(outline)
}

/// Дворы с рамками по сетке — префильтр точной пробы [`point_in_area`]:
/// узлов слияний в городе тысячи, и пробирать каждый полигон двора — даже
/// рамкой — не нужно: точка спрашивает свою ячейку, и только дворы, чья
/// рамка её касается, идут в точный тест.
struct YardBounds<'a> {
    /// Квадрат двора в метрах: проба уходит от узла не дальше пары шагов
    /// наезда, и ячейка в пару десятков метров держит списки короткими.
    grid: Grid<usize>,
    yards: Vec<&'a PolyArea>,
}

impl<'a> YardBounds<'a> {
    fn of(yards: impl IntoIterator<Item = &'a PolyArea>) -> Self {
        let yards: Vec<&PolyArea> = yards.into_iter().collect();
        let boxes: Vec<(Vec2, Vec2)> = yards.iter().map(|yard| ring_bounds(&yard.outer)).collect();
        let mut grid = Grid::new(YARD_CELL);
        for (index, (min, max)) in boxes.iter().enumerate() {
            grid.insert(*min, *max, index);
        }
        Self { grid, yards }
    }

    /// Двор под точкой — по ячейке сетки, с рамкой как префильтром точного
    /// теста.
    fn any_yard(&self, at: Vec2) -> bool {
        self.grid
            .at(at)
            .iter()
            .any(|&index| point_in_area(at, self.yards[index]))
    }
}

/// Ячейка сетки дворов [`YardBounds`], м.
const YARD_CELL: f32 = 32.0;

/// Наезд плитки на дворе у слияния мощёных дорожек ([`KerbReturns`], вызов в
/// [`kerb_returns`]): для каждой стороны каждого луча — полоса плитки
/// [`TILE_OVERLAP_WIDTH`] на [`TILE_OVERLAP_REACH`] вдоль кромки от узла,
/// если за кромкой двор (`Drawn::yards`) — у устья полосы его нет, а у
/// ближнего конца он уходит от кромки вглубь на ширину полосы, не остриём
/// клина, — и сторона не занята другим лучом ([`TILE_OVERLAP_OCCUPIED`]).
/// Внутренняя сторона заходит под ленту на [`OUTER_OVERLAP`], как у
/// наружного угла; контур — выпуклый четырёхугольник, как и прочие куски
/// слоя тротуаров. Проходной узел — крест, где каждый луч со встречным
/// ([`continues`]) — отсечён вызывающим: там нет щели, которую полоса
/// закрывала бы.
fn tile_overlaps(node: Vec2, group: &[&Arm], bounds: &YardBounds) -> Vec<Vec<Vec2>> {
    let mut found = Vec::new();
    for (own, arm) in group.iter().enumerate() {
        for (side, normal) in [arm.direction.perp(), -arm.direction.perp()]
            .into_iter()
            .enumerate()
        {
            // сторона занята: в неё уходит другой луч узла — угол здесь
            // закрыт скруглением, носом или веером
            let occupied = group.iter().enumerate().any(|(other, rest)| {
                other != own && normal.dot(rest.direction) >= TILE_OVERLAP_OCCUPIED.cos()
            });
            if occupied {
                continue;
            }
            // двор где-то в полосе наезда за кромкой — не только вплотную:
            // между кромкой и двором бывает голая земля, и наезд закрывает её
            // вместе с краем двора. Пробы — сеткой (вдоль кромки × от кромки),
            // по рамкам дворов из [`YardBounds`]
            let yard_in_band = |along: f32, off: f32| {
                let at = node + arm.direction * along + normal * (arm.half[side] + off);
                bounds.any_yard(at)
            };
            // у устья полосы двора быть не должно: щель раскрывается от узла
            // (голая земля, дальше двор). Двор вплотную к кромке у узла —
            // не щель, и полоса ложилась бы плитой на газон
            if [TILE_OVERLAP_WIDTH * 0.5, TILE_OVERLAP_WIDTH]
                .into_iter()
                .any(|off| yard_in_band(TILE_OVERLAP_MOUTH, off))
            {
                continue;
            }
            // двор у ближнего конца наезда должен уходить от кромки на ширину
            // полосы: клин, вошедший в кромку остриём глубиной меньше полосы,
            // раскрывается дальше — и ближняя половина полосы ложилась бы на
            // газонный угол слияния плитой (Тула, Первомайская × проспект
            // Ленина, западный угол)
            if ![(0.4, 0.5), (0.4, 1.0)]
                .into_iter()
                .all(|(share_along, share_off)| {
                    yard_in_band(
                        TILE_OVERLAP_REACH * share_along,
                        TILE_OVERLAP_WIDTH * share_off,
                    )
                })
            {
                continue;
            }
            let far = arm.direction * TILE_OVERLAP_REACH;
            found.push(vec![
                node + normal * (arm.half[side] - OUTER_OVERLAP),
                node + normal * (arm.half[side] + TILE_OVERLAP_WIDTH),
                node + far + normal * (arm.half[side] + TILE_OVERLAP_WIDTH),
                node + far + normal * (arm.half[side] - OUTER_OVERLAP),
            ]);
        }
    }
    found
}

/// Во сколько раз дуга скругления режется мельче общего шага
/// ([`arc_steps`]) у всех угловых кусков: стыки кусков общего шага
/// (12–17° на угол) читались гранями «ломаной» кромки тротуара и газона
/// (Тула, Фёдора Смирнова × Красноармейский, 3399 3835). Втрое — излом
/// 4–6°. Это верно и для дорожной группы: у обычного перекрёстка её дуга —
/// сама линия бордюра (ленты заканчиваются у узла прямыми торцами), и её
/// стыки видны так же.
const KERB_ARC_SUBDIVISION: usize = 3;

/// Скругление угла от луча `first` против часовой стрелки до луча `second`:
/// `halves` — полуширины полос, по краям которых сходится угол, `radius` —
/// радиус дуги в этом углу. Дуга режется в [`KERB_ARC_SUBDIVISION`] раз
/// мельче общего шага.
fn fillet(
    node: Vec2,
    first: &Arm,
    second: &Arm,
    halves: (f32, f32),
    radius: f32,
) -> Option<Vec<Vec2>> {
    let mut arc = fillet_arc(node, first, second, halves, radius)?;
    arc.steps *= KERB_ARC_SUBDIVISION;
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
    // касание — на кромках за углом и до конца поиска, дуга — в радиус
    let touches = |centre: Vec2, foot: Vec2, cut: f32, total: f32, radius: f32| {
        cut > OVERLAP && cut < total - OVERLAP && (centre.distance(foot) - radius).abs() < BEND_FIT
    };
    // Класс-радиус может не лечь в карман: за углом кромка скоро кончается, и
    // касательная садится за её конец (Путейская × Демонстрации, Тула: прямой
    // край плеча кончается дугой сквозного отрезка за 3.2 м от узла, и до
    // конца кромки за углом краёв — 7 м, касательной шестиметровой дуги — 7.6).
    // Радиус сжимается, пока обе касательные не сядут на кромки, — как в
    // [`fillet_arc`] сжимается касательная под прямым краем.
    let bisector = (along_first + along_second).try_normalize()?;
    let (_, first_total) = arclengths(&first_chain);
    let (_, second_total) = arclengths(&second_chain);
    let mut radius = radius;
    let (centre, first_foot, first_cut, second_foot, second_cut) = loop {
        let mut centre = corner + bisector * (radius / half_angle.sin());
        for _ in 0..BEND_SETTLE {
            let (first_foot, _) = nearest_on_path(&first_chain, centre)?;
            let (second_foot, _) = nearest_on_path(&second_chain, centre)?;
            let (away_first, away_second) = (centre - first_foot, centre - second_foot);
            centre += away_first.normalize_or_zero() * (radius - away_first.length())
                + away_second.normalize_or_zero() * (radius - away_second.length());
        }
        let (first_foot, first_cut) = nearest_on_path(&first_chain, centre)?;
        let (second_foot, second_cut) = nearest_on_path(&second_chain, centre)?;
        let inside = |cut: f32, total: f32| cut > OVERLAP && cut < total - OVERLAP;
        if (inside(first_cut, first_total) && inside(second_cut, second_total))
            || radius <= MIN_RADIUS
        {
            break (centre, first_foot, first_cut, second_foot, second_cut);
        }
        radius *= 0.8;
    };
    if !touches(centre, first_foot, first_cut, first_total, radius)
        || !touches(centre, second_foot, second_cut, second_total, radius)
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
    // тот же мелкий шаг, что у прямого скругления: кромка гнутого куска —
    // бордюр или кромка дорожки, стыки общего шага читаются гранями
    let steps = arc_steps(radius, sweep.abs()).max(1) * KERB_ARC_SUBDIVISION;
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
    let wide_first = std::ptr::eq(wide, first);
    let ((wide_normal, wide_half), (narrow_normal, narrow_half)) = if wide_first {
        ((n1, halves.0), (n2, halves.1))
    } else {
        ((n2, halves.1), (n1, halves.0))
    };
    // торец широкого — под его лентой, кромка узкого — под своей
    let wide_end = node + wide_normal * (wide_half - OVERLAP) + wide.direction * OVERLAP;
    let narrow_end = node + narrow_normal * (narrow_half - OVERLAP);
    let arc = (0..=steps)
        .map(|step| centre + Vec2::from_angle(sweep * step as f32 / steps as f32).rotate(from));
    let mut outline = Vec::with_capacity(steps + 5);
    outline.push(node + wide.direction * OUTER_OVERLAP);
    if wide_first {
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
    let Some(mut arc) = fillet_arc(node, first, second, halves, radius) else {
        return Vec::new();
    };
    // обе кромки кольца идут по газону во весь рост полосы — дуга тоже
    // режется мельче общего шага ([`KERB_ARC_SUBDIVISION`])
    arc.steps *= KERB_ARC_SUBDIVISION;
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
    // ворота — по углу между осями, а не между краями: он и есть угол
    // перекрёстка, и его держат все правила узла (`bend`, `obtuse_corner`).
    // У кромки под клином край уходит от оси на atan(slope), и на устье
    // встречного клина — кромки равной ширины у узла, оси под 147°, — край
    // показывал 155°: ворота тупого клина, который сам откажет по наклону, и
    // угол оставался без куска (Одоевский путепровод, R37-04/3). Форма дуги —
    // по краям, как и раньше.
    if !(MIN_ANGLE..=MAX_ANGLE).contains(&ccw_angle(first, second)) {
        return None;
    }
    let angle = ccw_between(along_first, along_second);
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
mod tests;
