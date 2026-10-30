//! Стоянки: асфальт с расчерченными местами и машинами на них.
//!
//! На снимке города двор со стоянкой ни с чем не спутать — прямоугольник
//! асфальта, расчерченный белыми полосками, и на половине мест что-то стоит.
//! До сих пор `amenity=parking` не запрашивался вовсе, и все эти площадки
//! были просто землёй.
//!
//! Раскладка держится одного правила: **к каждому месту машина должна
//! доехать**. Откуда она едет, у большой стоянки в OSM нарисовано —
//! `service=parking_aisle`, проезды между рядами ([`RoadLine::parking_aisle`]).
//! Тогда места раскладываются **по карманам между соседними проездами**
//! ([`aisle_rows`]): в кармане пара рядов спинами по его середине, носами
//! наружу, остаток кармана обеим сторонам под проезд — и глубина места
//! подгоняется так, чтобы проезду осталось не меньше [`PAIR_AISLE`]. Сетка
//! проездов при этом **продолжается своим шагом до краёв контура**: в OSM они
//! обрываются, не доходя до края, и без этого вдоль одних сторон площадки
//! оставалась бы широкая полоса голого асфальта, а вдоль других мест не было
//! бы вовсе. Своего поперечного проезда такая раскладка не выдумывает, но тот,
//! что нарисован в OSM, обязана оставить. Рисуют его **разрывом, а не линией**:
//! ход проезда разрезан надвое, между кусками оставлена полоса под проезд, — и
//! если тот же разрыв повторяется на соседней полосе, это поперечный проезд
//! ([`aisles::cross_drives_of`]), под которым ряды рвутся. У ТРЦ «Макси» такой проезд
//! один, и виден он сразу на 21 полосе из 37.
//!
//! Проездов в OSM нет у большинства дворовых площадок — им раскладка
//! **выдумывается** ([`generated_rows`]): ряды вдоль **самой длинной стороны
//! контура** (не вдоль оси описанного прямоугольника: у стоянки, дотянутой до
//! дороги, эта ось уезжает на десяток градусов от той стороны, по которой
//! стоянка читается). Поперёк это `ряд — проезд — пара рядов спинами — проезд
//! — пара рядов` ([`generated::row_bands`]): пристенный ряд выезжает в свой
//! проезд, каждая пара — в проезды по обе стороны от себя. Носом ряд повёрнут
//! туда, где зазор до соседнего ряда шире ([`generated::row_noses`]), — иначе
//! нос у всей площадки один, и первый ряд каждой пары стоит носом в спину
//! второму.
//! Вдоль — что ряд не тянется через всю площадку:
//! каждые [`ROW_BLOCK`] метров его рвёт поперечный проезд
//! ([`generated::row_places`]), и разрывы всех рядов стоят на одной линии, так
//! что площадка читается кварталами мест с проездами между ними, а не сплошной
//! штриховкой.
//!
//! Узкая полоса вдоль улицы — парковочный карман — размечается отдельно
//! ([`generated::pocket_rows`]): одним рядом вдоль сторон контура, носом к улице.
//!
//! О дорогах вокруг раскладка знает три вещи ([`Surroundings`]). Проезды рядов
//! задают направление — у большой площадки их полей бывает несколько, и каждое
//! размечается по своим ([`aisles::fields_of`]). Полотно улицы у кромки — асфальт, с
//! которого на место заезжают ([`reachable`]). А **дорога сквозь большую
//! стоянку** ([`is_through`] на [`is_ground`]) рисуется поверх её асфальта
//! (`roads/lots.rs`), и мест под ней нет. Маленькая стоянка по-прежнему кроет
//! всё, что на неё заходит, и разрыва под спрятанной дорогой не оставляет.
//!
//! Каждое место проверяется на попадание в контур, поэтому Г-образная стоянка
//! не получает мест поверх газона, а площадка, в которую не встаёт ни одно
//! место, — вовсе никаких. Маленький двор места получает, но не разметку:
//! см. [`MIN_AREA`].
//!
//! Разметка и машины делят **один** список мест: раскладка считается один раз
//! на загрузку мира в [`ParkingLayout`] (`map::spawn::spawn_map`), а краска
//! здесь и машины (`map::cars::fill_lots`) её только читают. Две независимые
//! раскладки поставили бы машину мимо её полосы.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use self::aisles::aisle_rows;
use self::generated::generated_rows;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{
    AreaKind, LotKind, PolyArea, RoadClass, RoadLine, TrafficSide, distance_to_segment,
    ring_bounds, signed_ring_area,
};
use crate::map::roads::is_carriageway;

mod aisles;
mod generated;

/// Место, м: легковая машина плюс просвет по обе стороны.
const STALL_WIDTH: f32 = 2.6;
const STALL_DEPTH: f32 = 5.2;
/// До скольких метров место ужимается ради проезда. Ниже — легковая машина
/// начинает торчать в проезд, и выигрыш оборачивается тем же затором.
const STALL_DEPTH_MIN: f32 = 4.8;
/// Сколько метров обязано остаться на проезд между парой рядов. Меньше
/// [`AISLE`]: на 14.8 м между проездами (больничная стоянка) полного проезда
/// и пары рядов не бывает вместе, а выбор между «ряд, ряд и 5.2 м» и «один
/// ряд и 9.6 м» решается в пользу первого — так эти площадки и размечены.
const PAIR_AISLE: f32 = 5.0;
/// Проезд между рядами, м, — и продольный, и поперечный: это одна и та же
/// полоса асфальта, по которой машина подъезжает к месту.
const AISLE: f32 = 6.0;
/// Длина ряда между поперечными проездами, м, — правило **выдуманной**
/// раскладки: ряд длиннее читается сплошной штриховкой, тогда как на снимке
/// такая площадка разбита проездами на кварталы мест. Выдуманную раскладку в
/// Туле берут 199 площадок, у 97 из них ряд вышел бы длиннее 50 м, у 33 —
/// длиннее 100, самый длинный 258 м (`w605838911`; кэш 7600 × 5700, v14).
/// Пятьдесят метров — это 19 мест подряд, обычный квартал между проездами.
const ROW_BLOCK: f32 = 50.0;
/// Насколько звено проезда может отклониться от главного направления стоянки
/// и всё ещё считаться рядом, °. Тридцать — это заведомо больше разнобоя
/// самих проездов (у ТРЦ «Макси» 102–105°) и заведомо меньше отворота к
/// соседнему проезду и поперечного проезда (там 59–61°, то есть 40° в сторону).
const AISLE_SPREAD: f32 = 30.0;
/// Насколько далеко конец одного звена может отстоять от начала следующего,
/// чтобы они всё ещё считались одним ходом проезда, м. Звенья приходят из
/// одной ломаной и стыкуются точно; допуск — на счёт в `f32` по мировым
/// координатам в тысячи метров.
const JOIN_SLACK: f32 = 0.01;
/// Допуск на последнее место ряда, м: место считается умножением номера на
/// [`STALL_WIDTH`] по мировым координатам в тысячи метров, и встающее ровно в
/// конец площадки не должно теряться на ошибке в последнем знаке.
const SPAN_SLACK: f32 = 0.01;
/// Два проезда ближе этого друг к другу — один и тот же, м.
const LANE_MERGE: f32 = STALL_DEPTH;
/// Сколько мест подряд обязано быть в куске ряда, чтобы его размечали. У
/// скошенной кромки площадки ряд обрывается, и остаётся одно место: на
/// картинке это полоска на пустом асфальте, к которой никто не подъедет.
const MIN_ROW_RUN: usize = 2;
/// Самая узкая полоса, по которой машина всё-таки протиснется к месту, м.
/// Карман между проездами, где на пару рядов не хватило, размечается одним
/// рядом, только если по обе стороны от него остаётся хотя бы столько.
const MIN_AISLE: f32 = 3.0;
/// Насколько середины разрывов на соседних полосах могут разойтись и всё ещё
/// быть одним поперечным проездом, м. У ТРЦ «Макси» 21 разрыв одного проезда
/// укладывается в 1.5 м; половина проезда — заведомый запас и заведомо меньше
/// шага мест.
const CROSS_DRIVE_SLACK: f32 = AISLE / 2.0;
/// Клетка сетки пересечений, м, — шире диагонали места (5.8 м), чтобы сосед
/// наверняка лежал в 3 × 3 клетках вокруг.
const OVERLAP_CELL: f32 = STALL_WIDTH + STALL_DEPTH;
/// Насколько местам позволено налезть друг на друга, м: соседи по ряду стоят
/// вплотную, и проверка пересечения обязана считать это «не пересекаются».
const OVERLAP_SLACK: f32 = 0.05;
/// Отступ разметки от края площадки, м.
const EDGE_MARGIN: f32 = 1.2;
/// Зазор между местом и краем **парковочного кармана** — полосы в один ряд,
/// в которую место с [`EDGE_MARGIN`] не встаёт, м. Только на счёт: угол места
/// ровно на контуре проверку попадания проходит через раз.
const STRIP_MARGIN: f32 = 0.2;
/// Место **вдоль бордюра** ([`generated::parallel_rows`]), м: длина по ходу машины — тот же
/// шаг, что у ряда машин вдоль улицы (`map::cars` берёт его отсюда как
/// `CAR_PITCH`), с запасом на «Газель» (5.3 м), — и ширина поперёк: фургон с
/// просветом.
pub(crate) const PARALLEL_LENGTH: f32 = 6.0;
const PARALLEL_WIDTH: f32 = 2.2;
/// Ширина полосы разметки, м, и её цвет — та же белая краска, что на улице.
const LINE_WIDTH: f32 = 0.12;
/// Насколько полоса не доходит до спины места, м. Ряды пары стоят спинами
/// вплотную, и без этого зазора их полосы сливаются в одну черту во всю
/// глубину обоих мест.
const LINE_GAP: f32 = 0.5;
const LINE_COLOR: Color = Color::srgb(0.82, 0.82, 0.80);
/// Стоянка мельче этого пятна не получает **разметки**: две машины во дворе
/// никто не расчерчивает. Места на ней остаются — машины на них стоят
/// (`map::cars::fill_lots`), просто по неразмеченному асфальту.
const MIN_AREA: f32 = 120.0;

/// Большая ли это стоянка — [`LotKind::Ground`], который решил разбор по
/// замощённому контуру (`parse/lots.rs`). У неё сквозь площадку видна дорога
/// ([`is_through`]) — в Туле сквозь одну, стоянку ТРЦ «Макси», 8.3 га.
pub fn is_ground(area: &PolyArea) -> bool {
    area.kind == AreaKind::Parking(LotKind::Ground)
}

/// Дорога **сквозь** большую стоянку — та, что рисуется поверх её асфальта, с
/// бордюром, и под которой мест нет.
///
/// По тегам её от проезда между рядами отличает одно: движение по ней
/// организовано. У «Макси» бульвар посреди площадки — `highway=service` с
/// `oneway=yes` и тремя кольцами, а у остальных больших площадок города
/// `service` внутри контура — это те же проезды рядов, только без
/// `service=parking_aisle`, и ни один не односторонний. Улица классом выше
/// проезда (`roads::is_carriageway`) — дорога при любых тегах.
pub fn is_through(road: &RoadLine) -> bool {
    road.class == RoadClass::Street
        && !road.parking_aisle
        && !road.bridge
        && !road.passage
        && (road.oneway || road.roundabout || is_carriageway(road))
}

/// Одно место: центр, направление, в котором машина стоит, и его глубина.
///
/// Глубина — поле, а не константа, потому что её задаёт карман между
/// проездами: чтобы на проезд осталось не меньше [`PAIR_AISLE`], паре рядов в
/// тесном кармане приходится ужаться до [`STALL_DEPTH_MIN`]. Длиннее всех в
/// слое машин «Газель» (5.3 м) — она торчит из ужатого места, легковые (3.9 —
/// 4.6 м) помещаются.
#[derive(Debug, Clone, Copy)]
pub struct Stall {
    pub at: Vec2,
    pub along: Vec2,
    pub depth: f32,
    /// Машина стоит **вдоль** бордюра, а не носом к нему: карман вдоль улицы
    /// тоньше места поперёк ([`generated::parallel_rows`]). `depth` у такого
    /// места — его
    /// длина по ходу машины. Разметки у него нет: полосы по бокам места легли
    /// бы вдоль бордюра и читались бы кромкой полосы движения.
    pub parallel: bool,
}

/// Раскладка всех стоянок карты — по списку мест на контур `MapData::parking`,
/// в том же порядке.
///
/// Кеш здесь окупается там, где у разрывов разметки (`roads::junctions`) не
/// окупился: вход раскладки — только контуры стоянок, а они меняются лишь со
/// сменой мира, тогда как слой машин пересобирается на каждое деление ползунка
/// солнца, зума и стиля дорог. Считать одно и то же по кадру — ровно та работа,
/// которой быть не должно; плата — резидентные 16 байт на место (Тула ~0.17 МБ).
#[derive(Resource, Default)]
pub struct ParkingLayout(pub Vec<Vec<Stall>>);

impl ParkingLayout {
    /// Места всех стоянок. Из дорог читается **единственное** — проезды стоянок
    /// (`service=parking_aisle`): по ним ряды и разворачиваются. Остальная сеть
    /// раскладке по-прежнему безразлична, и это не упущение — стоянка лежит
    /// поверх дорог (`Z_PARKING`) и кроет любую ленту, что заходит на неё или
    /// идёт сквозь неё: асфальт стоянки и есть проезд.
    ///
    /// Второе, что читается, — **дороги сквозь большую стоянку**
    /// ([`is_through`] на [`is_ground`]): они рисуются поверх её асфальта
    /// (`roads::mesh_roads`), и мест под ними нет.
    ///
    /// Третье — **асфальт улиц у кромки**: место, перед носом которого лежит
    /// полотно дороги, доступно с неё ([`reachable`]). Сторона движения
    /// `traffic` решает, куда смотрит машина, вставшая вдоль бордюра
    /// ([`generated::parallel_rows`]).
    pub fn new(lots: &[PolyArea], roads: &[RoadLine], traffic: TrafficSide) -> Self {
        let aisles: Vec<&RoadLine> = roads.iter().filter(|road| road.parking_aisle).collect();
        let through: Vec<&RoadLine> = roads.iter().filter(|road| is_through(road)).collect();
        // улицы с габаритами: на площадку идут только те, что рядом с ней
        let streets: Vec<(&RoadLine, Vec2, Vec2)> = roads
            .iter()
            .filter(|road| road.class == RoadClass::Street && !road.bridge && !road.passage)
            .map(|road| {
                let (low, high) = ring_bounds(&road.points);
                (road, low, high)
            })
            .collect();
        // Соседние площадки дотянуты до одних и тех же дорог и в зазоре между
        // собой **перекрываются** — оба контура честно считают его своим. Места
        // же на нём ставит одна: площадки раскладываются от большой к малой, и
        // место, наехавшее на уже стоящее чужое, снимается. Иначе машины двух
        // раскладок стоят друг на друге под разными углами (у ТРЦ «Макси» —
        // северо-западная стоянка и два кармана вдоль её проезда).
        let mut order: Vec<usize> = (0..lots.len()).collect();
        order.sort_by(|a, b| {
            let area = |index: &usize| signed_ring_area(&lots[*index].outer).abs();
            area(b).total_cmp(&area(a))
        });
        let mut taken = Placed::default();
        let mut layout: Vec<Vec<Stall>> = lots.iter().map(|_| Vec::new()).collect();
        for index in order {
            let lot = &lots[index];
            let (lot_low, lot_high) = ring_bounds(&lot.outer);
            let reach = Vec2::splat(AISLE);
            let ground = is_ground(lot);
            // с дороги за бордюром на место не заехать
            let drives: Vec<&RoadLine> = streets
                .iter()
                .filter(|(road, low, high)| {
                    low.cmple(lot_high + reach).all()
                        && high.cmpge(lot_low - reach).all()
                        && !(ground && is_through(road))
                })
                .map(|(road, _, _)| *road)
                .collect();
            let through: &[&RoadLine] = if ground { &through } else { &[] };
            let mut found = stalls_beside(lot, &aisles, through, &drives, traffic);
            found.retain(|stall| taken.push(*stall));
            layout[index] = found;
        }
        Self(layout)
    }
}

/// Места стоянки: по проездам из OSM, а если по ним не встало ни одного места
/// — по самой длинной стороне контура. Пусто, если площадка мелкая или
/// вырожденная.
///
/// Раскладка выдумывается не только там, где проездов нет вовсе: пустой ответ
/// [`aisle_rows`] — это и «полос меньше двух», и «ни одно место не прошло
/// проверок», — и тогда ряды идут по стороне контура, а не по проездам.
#[cfg(test)]
fn stalls(area: &PolyArea, aisles: &[&RoadLine]) -> Vec<Stall> {
    stalls_beside(area, aisles, &[], &[], TrafficSide::Right)
}

/// То же, но с дорогами вокруг. Места, на которые легла бы дорога из `through`
/// вместе со своим бордюром, не ставятся: дорога сквозь стоянку режет её ряды.
/// А полотно улицы из `drives` — асфальт, с которого на место заезжают, наравне
/// с асфальтом самой площадки.
pub fn stalls_beside(
    area: &PolyArea,
    aisles: &[&RoadLine],
    through: &[&RoadLine],
    drives: &[&RoadLine],
    traffic: TrafficSide,
) -> Vec<Stall> {
    let roads = Surroundings::near(area, through, drives);
    let outline = Outline::of(area);
    let rows = aisle_rows(&outline, aisles, &roads);
    if rows.is_empty() {
        generated_rows(&outline, &roads, traffic)
    } else {
        rows
    }
}

/// Высота полосы индекса рёбер, м, — порядка места: проба смотрит рёбра одной
/// полосы, и у площадки в тысячу вершин их там единицы.
const OUTLINE_BAND: f32 = 4.0;

/// Контур площадки с рёбрами, разложенными по горизонтальным полосам, — чтобы
/// вопрос «внутри ли точка» не обходил всё кольцо.
///
/// Раскладка задаёт его десятки тысяч раз на площадку (четыре угла и две пробы
/// перед носом на каждое место-кандидат, плюс три на звено проезда в
/// [`aisle_rows`]), а контур стоянки, дотянутой до дорог, — сотни вершин на
/// скруглениях: у ТРЦ «Макси» 1220, и обход всего кольца на каждую пробу стоил
/// 87 мс на одну эту площадку. Ответ тот же, что у `point_in_area`: чётность
/// пересечений луча по всем кольцам разом (дырка лежит внутри внешнего кольца,
/// и её пересечения чётность гасят).
///
/// **Полосы разложены по габариту внешнего кольца, и этого хватает на всё, что
/// делает мощение.** Ребро попадает в каждую полосу, которую задевает по `y`,
/// так что в полосе точки лежат все рёбра, способные пересечь её луч; дырка же
/// целиком внутри внешнего кольца, а часть, отрезанная отмосткой, — сама себе
/// площадка со своим габаритом. Сверено перебором на трёх таких контурах
/// (`the_outline_index_agrees_with_the_full_ring`).
struct Outline<'a> {
    area: &'a PolyArea,
    low: f32,
    bands: Vec<Vec<(Vec2, Vec2)>>,
}

impl<'a> Outline<'a> {
    fn of(area: &'a PolyArea) -> Self {
        let (low, high) = ring_bounds(&area.outer);
        let count = ((high.y - low.y) / OUTLINE_BAND).floor().max(0.0) as usize + 1;
        let mut bands: Vec<Vec<(Vec2, Vec2)>> = vec![Vec::new(); count];
        for ring in std::iter::once(&area.outer).chain(&area.holes) {
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                let band = |y: f32| {
                    (((y - low.y) / OUTLINE_BAND).floor().max(0.0) as usize).min(count - 1)
                };
                for slot in &mut bands[band(a.y.min(b.y))..=band(a.y.max(b.y))] {
                    slot.push((a, b));
                }
            }
        }
        Self {
            area,
            low: low.y,
            bands,
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        let band = (point.y - self.low) / OUTLINE_BAND;
        if band < 0.0 {
            return false;
        }
        let Some(edges) = self.bands.get(band as usize) else {
            return false;
        };
        let mut inside = false;
        for (a, b) in edges {
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
        inside
    }
}

/// Бордюр дороги, идущей сквозь стоянку, с одной стороны, м, — когда у неё нет
/// своего тротуара (проезд, `roads::is_carriageway`); с тротуаром бордюр —
/// его полоса по классу (`SidewalkProfile::kerb`).
pub const LOT_KERB: f32 = 1.2;
/// Сколько асфальта остаётся между бордюром сквозной дороги и местом, м.
const THROUGH_CLEARANCE: f32 = 0.5;

/// Дороги вокруг площадки — звеньями `(от, до, расстояние от оси)`.
#[derive(Default)]
struct Surroundings {
    /// Дороги сквозь площадку: ближе этого расстояния к оси место не встаёт.
    through: Vec<(Vec2, Vec2, f32)>,
    /// Полотна улиц у площадки: в пределах этого расстояния от оси — асфальт.
    drives: Vec<(Vec2, Vec2, f32)>,
}

impl Surroundings {
    fn near(area: &PolyArea, through: &[&RoadLine], drives: &[&RoadLine]) -> Self {
        let (low, high) = ring_bounds(&area.outer);
        let links = |roads: &[&RoadLine], reach: fn(&RoadLine) -> f32| {
            let mut links = Vec::new();
            for road in roads {
                let reach = reach(road);
                for pair in road.points.windows(2) {
                    let (from, to) = (pair[0], pair[1]);
                    let outside = from.max(to).cmplt(low - reach - AISLE).any()
                        || from.min(to).cmpgt(high + reach + AISLE).any();
                    if !outside {
                        links.push((from, to, reach));
                    }
                }
            }
            links
        };
        Self {
            through: links(through, |road| {
                road.sidewalk().kerb_edge(road.width / 2.0, LOT_KERB) + THROUGH_CLEARANCE
            }),
            drives: links(drives, |road| road.width / 2.0),
        }
    }

    /// Лежит ли точка на полотне улицы.
    fn paved(&self, point: Vec2) -> bool {
        self.drives
            .iter()
            .any(|(from, to, reach)| distance_to_segment(point, *from, *to) <= *reach)
    }

    /// Проходит ли между двумя точками дорога сквозь площадку.
    fn crossed(&self, from: Vec2, to: Vec2) -> bool {
        (0..=4).any(|step| {
            let at = from.lerp(to, step as f32 / 4.0);
            self.through
                .iter()
                .any(|(start, end, reach)| distance_to_segment(at, *start, *end) < *reach)
        })
    }

    /// Задевает ли место полотно или бордюр. Углов и центра хватает: полоса
    /// запрета (от 3.7 м в сторону от оси) шире полудиагонали места (2.9 м),
    /// так что ось, прошедшая сквозь место, ловится его центром.
    fn cover(&self, at: Vec2, along: Vec2, depth: f32) -> bool {
        if self.through.is_empty() {
            return false;
        }
        let across = Vec2::new(-along.y, along.x);
        let half_depth = along * (depth / 2.0);
        let half_width = across * (STALL_WIDTH / 2.0);
        let probes = [
            at,
            at - half_width - half_depth,
            at + half_width - half_depth,
            at + half_width + half_depth,
            at - half_width + half_depth,
        ];
        // дальше полудиагонали места от полосы запрета — углы не спрашиваются
        let diagonal = Vec2::new(STALL_WIDTH, depth).length() / 2.0;
        self.through.iter().any(|(from, to, reach)| {
            distance_to_segment(at, *from, *to) < *reach + diagonal
                && probes
                    .iter()
                    .any(|probe| distance_to_segment(*probe, *from, *to) < *reach)
        })
    }
}

/// Габариты кольца в осях `along`/`across`: (мин, макс) проекций.
fn axis_bounds(ring: &[Vec2], along: Vec2, across: Vec2) -> (Vec2, Vec2) {
    let mut low = Vec2::INFINITY;
    let mut high = Vec2::NEG_INFINITY;
    for point in ring {
        let projected = Vec2::new(point.dot(along), point.dot(across));
        low = low.min(projected);
        high = high.max(projected);
    }
    (low, high)
}

/// Уже поставленные места с сеткой по ним: место кладётся, только если не
/// наезжает на чужое.
///
/// Без этого поперечный проезд стелил бы свой ряд поперёк чужих мест: в OSM
/// проезды пересекаются, а у ТРЦ «Макси» полсотни проездов на одну площадку,
/// из них шесть — под другим углом.
#[derive(Default)]
struct Placed {
    stalls: Vec<Stall>,
    /// Клетка шире диагонали места, поэтому соседей хватает искать в 3 × 3.
    grid: HashMap<(i32, i32), Vec<usize>>,
}

impl Placed {
    /// Поставить место, если оно не наезжает на уже стоящее; встало ли.
    fn push(&mut self, stall: Stall) -> bool {
        let cell = Self::cell(stall.at);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(near) = self.grid.get(&(cell.0 + dx, cell.1 + dy)) else {
                    continue;
                };
                if near
                    .iter()
                    .any(|index| overlaps(&self.stalls[*index], &stall))
                {
                    return false;
                }
            }
        }
        self.grid.entry(cell).or_default().push(self.stalls.len());
        self.stalls.push(stall);
        true
    }

    fn cell(at: Vec2) -> (i32, i32) {
        (
            (at.x / OVERLAP_CELL).floor() as i32,
            (at.y / OVERLAP_CELL).floor() as i32,
        )
    }
}

/// Пересекаются ли два места — по теореме о разделяющей оси, осей четыре.
/// Допуск в [`OVERLAP_SLACK`] обязателен: соседи по ряду стоят ровно вплотную,
/// и без него ошибка в последнем знаке снимала бы каждое второе место.
fn overlaps(a: &Stall, b: &Stall) -> bool {
    let axes = [
        a.along,
        Vec2::new(-a.along.y, a.along.x),
        b.along,
        Vec2::new(-b.along.y, b.along.x),
    ];
    axes.iter().all(|axis| {
        let reach = |stall: &Stall| {
            let across = Vec2::new(-stall.along.y, stall.along.x);
            (stall.along.dot(*axis) * stall.depth / 2.0).abs()
                + (across.dot(*axis) * STALL_WIDTH / 2.0).abs()
        };
        (b.at - a.at).dot(*axis).abs() < reach(a) + reach(b) - OVERLAP_SLACK
    })
}

/// Есть ли перед носом места асфальт, с которого на него заезжают.
///
/// Влезать в контур мало: сетка полос продолжается за крайние проезды
/// ([`aisles::lanes_of`]), и у самой кромки ряд встаёт так, что его проезд остался
/// снаружи площадки. Места там размечались, хотя заехать на них неоткуда —
/// на снимке это одинокие полосы вдоль кромки, отчёт автора. Проба идёт по
/// обеим передним кромкам, а не по одной осевой: у скошенного угла площадки
/// середина ещё на асфальте, когда половина выезда уже за ним.
///
/// **Асфальт — это и полотно улицы у кромки** ([`Surroundings::paved`]), не
/// только сама площадка: полоса мест вдоль проезда (Тула, way 619292977,
/// 62 × 6 м) вся стоит носом в него, а в контур площадки проезд не входит.
fn reachable(area: &Outline, roads: &Surroundings, at: Vec2, along: Vec2, depth: f32) -> bool {
    let across = Vec2::new(-along.y, along.x);
    let ahead = at + along * (depth / 2.0 + PAIR_AISLE / 2.0);
    [-1.0f32, 1.0].iter().all(|side| {
        let probe = ahead + across * (side * STALL_WIDTH / 2.0);
        area.contains(probe) || roads.paved(probe)
    })
}

/// Место целиком внутри контура — по четырём углам, как и коробки на кровле.
fn fits(area: &Outline, at: Vec2, along: Vec2, across: Vec2, depth: f32) -> bool {
    fits_with(area, at, along, across, depth, 0.0)
}

/// То же, но место раздуто на `margin` со всех сторон: так у площадки требуют
/// **запаса до кромки**, а не попадания впритык.
///
/// Кромка стоянки идёт наискось к рядам, и место, чей угол лежит ровно на ней,
/// на картинке читается половинкой: машины на нём не видно, а полосы по его
/// краям торчат из ряда в никуда — отчёт автора. Запас в [`EDGE_MARGIN`]
/// убирает ровно такой торец, оставляя ряд на метр короче.
fn fits_with(area: &Outline, at: Vec2, along: Vec2, across: Vec2, depth: f32, margin: f32) -> bool {
    fits_within(area, at, along, across, depth, (margin, margin))
}

/// То же, с запасом порознь: `margins.0` вдоль ряда, `margins.1` — по глубине.
fn fits_within(
    area: &Outline,
    at: Vec2,
    along: Vec2,
    across: Vec2,
    depth: f32,
    margins: (f32, f32),
) -> bool {
    let half_width = along * (STALL_WIDTH / 2.0 + margins.0);
    let half_depth = across * (depth / 2.0 + margins.1);
    fits_box(area, at, half_width, half_depth)
}

/// Все четыре угла прямоугольника с центром `at` и полуосями `half_a`,
/// `half_b` — в контуре.
fn fits_box(area: &Outline, at: Vec2, half_a: Vec2, half_b: Vec2) -> bool {
    [
        at - half_a - half_b,
        at + half_a - half_b,
        at + half_a + half_b,
        at - half_a + half_b,
    ]
    .iter()
    .all(|corner| area.contains(*corner))
}

/// Разметка мест в меш: по полоске между соседними местами. Полоса, а не
/// прямоугольник места: расчерчивают именно границы.
///
/// Полоса кладётся слева от каждого места (у соседей они совпадают — это
/// дешевле, чем искать соседа) плюс **закрывающая** справа там, где кусок ряда
/// начинается: у поперечного проезда и у края контура ряд обязан быть
/// закрыт, иначе крайнее место квартала выглядит распахнутым в проезд.
///
/// «Сосед справа» — это требование к **порядку** списка: места ряда обязаны
/// идти в направлении `-perp(Stall::along)`. У выдуманной раскладки так выходит
/// само, у рядов по проезду ([`aisle_rows`]) дальняя от проезда сторона за этим
/// идёт задом наперёд — иначе полосы легли бы по две в одно место.
pub fn push_markings(builder: &mut MeshBuilder, area: &PolyArea, stalls: &[Stall]) {
    if signed_ring_area(&area.outer).abs() < MIN_AREA {
        return;
    }
    let color = LINE_COLOR.to_linear();
    let area = &Outline::of(area);
    // места одного ряда идут по порядку, так что сосед справа — предыдущее
    // место списка; у первого места куска его там нет
    let mut previous: Option<Vec2> = None;
    for stall in stalls {
        if stall.parallel {
            previous = None;
            continue;
        }
        let along = stall.along;
        let across = Vec2::new(-along.y, along.x);
        let nose = along * (stall.depth / 2.0);
        // со стороны спины полоса не доходит до конца места: у пары рядов
        // спинами полосы встречаются там встык и читаются одной длинной
        // чертой через оба ряда, а не границей мест
        let tail = along * (stall.depth / 2.0 - LINE_GAP);
        let half_line = across * (LINE_WIDTH / 2.0);
        // полоса рисуется, только если по ту её сторону **поместилось бы
        // место**: у кромки площадки она иначе читается чёрточкой в никуда —
        // отчёт автора. Так ряд и размечают: крайнее место открыто в бордюр, а
        // у поперечного проезда полоса остаётся, потому что асфальт за ней
        // есть. Мерить наличием асфальта в полуместе от грани было мало:
        // место и так стоит с запасом [`EDGE_MARGIN`], и проба попадала на
        // него же
        // запас по глубине — тот, с каким стоит само место: в парковочном
        // кармане ([`STRIP_MARGIN`]) его нет и у места, и требовать его от
        // соседа значило бы оставить карман без единой полосы
        let deep = if fits_within(
            area,
            stall.at,
            across,
            along,
            stall.depth,
            (0.0, EDGE_MARGIN),
        ) {
            EDGE_MARGIN
        } else {
            0.0
        };
        let mut bar = |edge: Vec2, outward: Vec2| {
            let beyond = edge + outward * (STALL_WIDTH / 2.0);
            if !fits_within(
                area,
                beyond,
                across,
                along,
                stall.depth,
                (EDGE_MARGIN, deep),
            ) {
                return;
            }
            builder.push_quad(
                [
                    edge - tail - half_line,
                    edge + nose - half_line,
                    edge + nose + half_line,
                    edge - tail + half_line,
                ],
                color,
            );
        };
        bar(stall.at - across * (STALL_WIDTH / 2.0), -across);
        let neighbour = stall.at + across * STALL_WIDTH;
        if previous.is_none_or(|at| at.distance(neighbour) > LINE_WIDTH) {
            bar(stall.at + across * (STALL_WIDTH / 2.0), across);
        }
        previous = Some(stall.at);
    }
}

#[cfg(test)]
mod tests;
