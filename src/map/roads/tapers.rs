//! **Клинья** — переход между сечениями там, где одна дорога кончается и её
//! соосно продолжает другая, а ширина у них разная: на шве улицы
//! (`network::sections`, число полос сменилось) и на продолжении через
//! границу улиц (`RoadNetwork::continuations` — другой класс, другая
//! односторонность). Более широкий way начинается не с полной ширины, а с
//! ширины соседа, и расходится до своей на длине «ручка `Taper` × разница
//! ширин» (10 м на метр по умолчанию). До этого ширина менялась ступенькой
//! прямо в узле.
//!
//! **Клин — по сторонам.** В чистом шве (в узле никого, кроме двух way)
//! сужаются обе кромки. На перекрёстке — только та, с которой к узлу не
//! подходит другая проезжая часть ([`Taper::sides`]): со стороны примыкания
//! уступ и так закрывают асфальт примыкающей и скругления бордюра
//! (`roads/corners.rs`), а с пустой стороны он торчал бордюрным зубом — широкая
//! улица кончалась, и узкая шла дальше. Закрывает сторону только примыкание,
//! у которого радиус бордюра (`corners::road_radius`) не меньше уступа: по
//! сторонам проезда в 5 м кромка на полосу разной высоты читалась зубом, и
//! такая сторона сужается тоже. И только пока стык соосен: изогнутый стык
//! пары — не «одна дорога продолжается», и когда широкую продолжает под
//! тупым углом (155–180°) другая дорога узла, сужающего клина на паре нет
//! вовсе — вместо него устье растёт к узлу **встречными клиньями**:
//! узкий шва и тупой продолжатель начинают у шва с сечения широкой и
//! расходятся к своим, и кромки устья смыкаются дугами угловых правил
//! (Одоевский путепровод × Демонстрации, R37-04/3); без тупого продолжения
//! изогнутый стык клин берёт как раньше (Крестовоздвиженская в Союзную,
//! витрина 28). С
//! сохранённой стороны кромка идёт
//! полной ширины до самого узла, как раньше; скругления считают тот торец по
//! полуширине его стороны: сужаемой — по узкому соседу. У крестовины, где
//! примыкания с обеих сторон, клина нет вовсе — ступень тонет в узле. Дорожка
//! (`RoadClass::Alley`) стороны не закрывает: её слой уступа не кроет.
//!
//! Только картинка: `RoadLine` не меняется, навмеш и разбор видят ширину
//! участка как есть. Машины — тоже, но ряд у бордюра на длину клина
//! прерывается ([`car_clearings`]): иначе он встал бы на тротуар.

use bevy::prelude::*;

use super::corners::{MAX_ANGLE, road_radius};
use super::drawn::Drawn;
use super::junctions::node_key;
use super::lane_count;
use super::network::{RoadNetwork, RoadNodes, arm_direction};
use super::node_paint::PASS_ALIGN;
use super::paint::{WedgeEnd, wedge_frames};
use super::ribbons::{wedge_halves, wedge_middle};
use crate::map::meshing::{Break, LaneFrame, MeshBuilder, to_break_beyond};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{RoadClass, RoadLine, TrafficSide};

/// Длина клина на метр разницы ширин по умолчанию, м: полоса в 3.3 м
/// появляется за 33 м — как отгон уширения на городской улице. В игре — ручка
/// `Taper` ([`RoadShape::taper`](super::shape::RoadShape)); здесь — для тестов.
#[cfg(test)]
pub const TAPER_PER_METER: f32 = 10.0;
/// Разница ширин, которую клин ещё не выравнивает, м: сантиметры не видны.
const TAPER_MIN_STEP: f32 = 0.1;
/// Самый короткий клин, м: короче — та же ступенька, только скошенная.
const TAPER_MIN_LENGTH: f32 = 1.0;
/// Доля нарисованной длины way, которую может занять один клин: у way бывают
/// клинья с обоих концов, и между ними должна остаться полная ширина.
const TAPER_MAX_SHARE: f32 = 0.45;

/// Клин у одного торца way: длина по разнице ширин (до обрезки по длине
/// нарисованного пути), дорога, с чьего сечения он начинается у шва
/// (`seam`), и стороны `[слева, справа]` по ходу точек way, которые он ведёт
/// от сечения шва к своим полуширинам (хотя бы одна). Сечение у шва двояко:
/// у сужающего клина оно у узкого соседя — широкая дорога сужается к шву; у
/// встречного — у широкого соседа: дорога растёт к узлу до его ширины.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taper {
    pub length: f32,
    pub seam: usize,
    pub sides: [bool; 2],
}

impl Taper {
    /// Кромка, которую клин не трогает: `+1` — левая по ходу way, `-1` —
    /// правая, `None` — клин симметричен. Раскладка полос на односторонне
    /// суженном клине прижата к ней (`roads/paint.rs`).
    pub fn kept(&self) -> Option<f32> {
        match self.sides {
            [true, true] => None,
            [false, _] => Some(1.0),
            [true, false] => Some(-1.0),
        }
    }
}

/// Клинья карты: у каждой дороги — по торцам `[начало, конец]`.
#[derive(Debug, Default)]
pub struct Tapers {
    ends: Vec<[Option<Taper>; 2]>,
    pub count: usize,
}

impl Tapers {
    /// Клинья по стыкам улиц сети и по продолжениям через их границы. `drawn`
    /// — дороги так, как они рисуются (переезд через тротуар уже асфальтом),
    /// по тем же индексам, что у сети; `per_meter` — длина клина на метр
    /// разницы ширин.
    pub fn new(
        drawn: &[&RoadLine],
        network: &RoadNetwork,
        nodes: &RoadNodes,
        per_meter: f32,
    ) -> Self {
        let mut tapers = Self {
            ends: vec![[None; 2]; drawn.len()],
            count: 0,
        };
        if drawn.is_empty() || !network.covers(drawn.len()) {
            return tapers;
        }
        for (a, b) in network.joints().chain(network.continuations()) {
            let (first, second) = (drawn[a.road], drawn[b.road]);
            if a.road == b.road || !takes_taper(first) || !takes_taper(second) {
                continue;
            }
            let node = if a.reversed {
                first.points[0]
            } else {
                first.points[first.points.len() - 1]
            };
            let step = (first.width - second.width).abs();
            if step < TAPER_MIN_STEP {
                continue;
            }
            // торец широкого way, которым он стоит в шве
            let (wide, end, narrow) = if first.width > second.width {
                (a.road, !a.reversed, b.road)
            } else {
                (b.road, b.reversed, a.road)
            };
            // Изогнутый стык пары на перекрёстке — не «одна дорога
            // продолжается», и сужающий клин на нём съедал устье: широкая
            // дорога сужалась к перекрёстку на полосу, двоя осевую (Одоевский
            // путепровод × Демонстрации, R37-04). Сужения нет, когда широкую
            // продолжает под тупым углом (155–180°, углы широкого клина
            // кромки, `corners::obtuse_corner`) другая дорога узла: устье не
            // обрывается и не сужается, а растёт к узлу встречными клиньями —
            // узкий шва и тупой продолжатель начинают у шва с сечения широкой
            // и расходятся к своим, и кромки устья смыкаются дугами угловых
            // правил. Без тупого продолжения изогнутый стык клин берёт как
            // раньше (Крестовоздвиженская в Союзную, витрина 28), а соосный —
            // всегда.
            let away_first = arm_direction(&first.points, !a.reversed);
            let away_second = arm_direction(&second.points, b.reversed);
            let away = |road: usize| {
                if road == a.road {
                    away_first
                } else {
                    away_second
                }
            };
            if let (Some(away_wide), Some(away_narrow)) = (away(wide), away(narrow))
                && -away_wide.dot(away_narrow) < PASS_ALIGN
                && let Some((continuer, _)) =
                    obtuse_continuation(drawn, nodes, node, away_wide, a.road, b.road)
            {
                for road in [narrow, continuer] {
                    widen_mouth(&mut tapers, drawn, node, wide, away_wide, road, per_meter);
                }
                continue;
            }
            let sides = free_sides(drawn, nodes, node, wide, end, narrow);
            if sides == [false; 2] {
                continue;
            }
            tapers.ends[wide][usize::from(end)] = Some(Taper {
                length: step * per_meter,
                seam: narrow,
                sides,
            });
            tapers.count += 1;
        }
        tapers
    }

    /// Клинья у торцов дороги `[начало, конец]`.
    pub fn at(&self, road: usize) -> [Option<Taper>; 2] {
        self.ends.get(road).copied().unwrap_or([None; 2])
    }

    /// Клин теста у торца `end` дороги `road`, как если бы его нашёл [`Self::new`].
    #[cfg(test)]
    pub fn set(&mut self, road: usize, end: usize, taper: Taper) {
        if self.ends[road][end].is_none() {
            self.count += 1;
        }
        self.ends[road][end] = Some(taper);
    }

    /// Клинья по дорогам карты как есть, без подмен рисования, — для того, у
    /// кого нет `roads::Drawn`: витрины машин, где нет и карты.
    pub fn of_map(roads: &[RoadLine], network: &RoadNetwork, per_meter: f32) -> Self {
        let nodes = RoadNodes::new(roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        Self::new(&drawn, network, &nodes, per_meter)
    }
}

/// Разрывы ряда припаркованных машин (`map::cars`) на клиньях: ряд стоит на
/// полуширине участка, а в клине бордюр ближе к оси — машина встала бы на
/// тротуар. Разрыв — в узле шва, длиной в клин, по дороге, на которой клин
/// лежит, — `(дорога, разрыв)`. `tapers` — по тем же индексам, что `roads`.
pub fn car_clearings(roads: &[RoadLine], tapers: &Tapers) -> Vec<(usize, Break)> {
    let mut clearings = Vec::with_capacity(tapers.count);
    for (road, ends) in tapers.ends.iter().enumerate() {
        let points = &roads[road].points;
        let share = polyline_length(points) * TAPER_MAX_SHARE;
        for (end, taper) in ends.iter().enumerate() {
            let Some(taper) = taper else { continue };
            let at = if end == 1 {
                points[points.len() - 1]
            } else {
                points[0]
            };
            clearings.push((
                road,
                Break {
                    at,
                    reach: taper.length.min(share),
                },
            ));
        }
    }
    clearings
}

/// Ширину клином выравнивает ровная проезжая лента — и настил моста: на шве
/// земля/мост число полос меняется так же, как на шве улицы, и без клина
/// ширина прыгала ступенькой ровно на голове моста (R30, Красный мост в
/// Орле: земля 2 полосы, мост 3). Настил и его бортик сужаются клином вместе
/// (`mesh_roads`, `Bridges::push_deck`). Арка — нет: она приколота к стене
/// дома.
fn takes_taper(road: &RoadLine) -> bool {
    !road.passage && road.lanes.is_some()
}

/// Стороны `[слева, справа]` по ходу точек way `wide` у его торца `end` в
/// узле `node`, с которых к узлу не подходит другая проезжая часть — там
/// уступ виден, и клин сужает только их. Лучи чужих дорог берутся по всем их
/// вершинам в узле (сквозная дорога даёт два), дорожки (`RoadClass::Alley`)
/// не считаются.
fn free_sides(
    drawn: &[&RoadLine],
    nodes: &RoadNodes,
    node: Vec2,
    wide: usize,
    end: bool,
    narrow: usize,
) -> [bool; 2] {
    let Some(away) = arm_direction(&drawn[wide].points, end) else {
        return [false; 2];
    };
    let key = node_key(node);
    let mut free = [true; 2];
    for &other in nodes.roads_at(node) {
        if other == wide || other == narrow || drawn[other].class != RoadClass::Street {
            continue;
        }
        let points = &drawn[other].points;
        for (vertex, point) in points.iter().enumerate() {
            if node_key(*point) != key {
                continue;
            }
            let arms = [
                arm_direction(&points[vertex..], false),
                arm_direction(&points[..=vertex], true),
            ];
            for direction in arms.into_iter().flatten() {
                let cross = away.perp_dot(direction);
                if cross == 0.0 {
                    continue;
                }
                // Уступ, прикрытый лентой примыкания, читается ступенью, когда
                // он выше радиуса бордюра у примыкания: по сторонам проезда в
                // 5 м кромка Болдина стояла на 3.3 м разной высоты — зуб, а не
                // угол (Тула, витрина 22, 2 → 4 полосы). Улице (6 м) и
                // проспекту (10) такой уступ тонет в скруглениях.
                let step = (drawn[wide].width - drawn[narrow].width) / 2.0;
                let radius = road_radius(drawn[other]).min(road_radius(drawn[wide]));
                if step > radius {
                    continue;
                }
                // слева от луча `away` — слева по ходу way у его начала и
                // справа у конца
                let left_of_way = (cross > 0.0) != end;
                free[usize::from(!left_of_way)] = false;
            }
        }
    }
    free
}

/// Продолжается ли дорога с направлением `away` от узла в другую дорогу узла
/// (не пару стыка `skip`) — и какая: их плечи расходятся под тупым углом, от
/// [`MAX_ANGLE`] — угла широкого клина кромки, который закрывает перепад
/// ширин на таком продолжении (`corners::obtuse_corner`). Лучи чужих дорог —
/// по всем их вершинам в узле: сквозная даёт два. Возвращается дорога и её
/// луч, продолжающий `away`.
fn obtuse_continuation(
    drawn: &[&RoadLine],
    nodes: &RoadNodes,
    node: Vec2,
    away: Vec2,
    skip_a: usize,
    skip_b: usize,
) -> Option<(usize, Vec2)> {
    let key = node_key(node);
    let obtuse = MAX_ANGLE.cos();
    nodes.roads_at(node).iter().find_map(|&other| {
        if other == skip_a || other == skip_b || drawn[other].class != RoadClass::Street {
            return None;
        }
        let points = &drawn[other].points;
        points.iter().enumerate().find_map(|(vertex, point)| {
            if node_key(*point) != key {
                return None;
            }
            [
                arm_direction(&points[vertex..], false),
                arm_direction(&points[..=vertex], true),
            ]
            .into_iter()
            .flatten()
            .find(|direction| away.dot(*direction) <= obtuse)
            .map(|direction| (other, direction))
        })
    })
}

/// Встречный клин устья: дорога `road`, подходящая к `node` торцом, начинает
/// у шва с сечения широкой `wide` (`away_wide` — её луч из узла) и на длине
/// клина расходится к своим полуширинам — со стороны, обращённой к широкой:
/// вторая сторона смотрит от устья, и её кромка — угол чужой пары (нос,
/// дуга примыкания). Дорога, проходящая узел насквозь, клина не берёт: у неё
/// нет торца в этом узле.
fn widen_mouth(
    tapers: &mut Tapers,
    drawn: &[&RoadLine],
    node: Vec2,
    wide: usize,
    away_wide: Vec2,
    road: usize,
    per_meter: f32,
) {
    if !takes_taper(drawn[road]) {
        return;
    }
    let key = node_key(node);
    let points = &drawn[road].points;
    let last = points.len() - 1;
    let Some(end) = [0usize, 1]
        .into_iter()
        .find(|&end| node_key(points[end * last]) == key)
    else {
        return;
    };
    let step = drawn[wide].width - drawn[road].width;
    if step < TAPER_MIN_STEP {
        return;
    }
    let Some(away_road) = arm_direction(points, end == 1) else {
        return;
    };
    // широкая слева от луча дороги — когда их векторное произведение
    // положительно; слева от луча — слева по ходу way у его начала и справа
    // у конца (как стороны в `free_sides`)
    let left_of_way = (away_road.perp_dot(away_wide) > 0.0) != (end == 1);
    let sides = [left_of_way, !left_of_way];
    if tapers.ends[road][end].is_none() {
        tapers.count += 1;
    }
    tapers.ends[road][end] = Some(Taper {
        length: step * per_meter,
        seam: wide,
        sides,
    });
}

/// Нарисованный путь, разрезанный под клинья: `(клин у начала, середина, клин
/// у конца)`. Оба клина смотрят **от узла шва** — от сечения шва к своим
/// полуширинам (у сужающего клина сечение шва уже своего, у встречного —
/// шире).
/// Длина клина обрезается до [`TAPER_MAX_SHARE`] пути; клин короче
/// [`TAPER_MIN_LENGTH`] не кладётся, и его конец остаётся в середине.
pub fn split(path: &[Vec2], lengths: [Option<f32>; 2]) -> [Option<Vec<Vec2>>; 3] {
    let total = polyline_length(path);
    let [head, tail] = fit(total, lengths);
    let from = head.unwrap_or(0.0);
    let to = total - tail.unwrap_or(0.0);
    let head_path = head.map(|length| {
        let mut piece = cut(path, 0.0, length);
        piece.dedup();
        piece
    });
    let tail_path = tail.map(|length| {
        let mut piece = cut(path, total - length, total);
        piece.reverse();
        piece.dedup();
        piece
    });
    [head_path, Some(cut(path, from, to)), tail_path]
}

/// Длины клиньев `[у начала, у конца]` на пути длиной `total` — так, как их
/// нарежет [`split`]: не больше [`TAPER_MAX_SHARE`] пути, короче
/// [`TAPER_MIN_LENGTH`] — клина нет. Слой краски (`roads/paint.rs`) плывёт по
/// тем же длинам.
pub fn fit(total: f32, lengths: [Option<f32>; 2]) -> [Option<f32>; 2] {
    lengths.map(|length| {
        length
            .map(|length| length.min(total * TAPER_MAX_SHARE))
            .filter(|&length| length >= TAPER_MIN_LENGTH)
    })
}

/// Кусок ломаной между длинами дуги `from..to`. Им же режется тротуар у
/// разделённой улицы (`roads::push_sidewalk`).
pub(super) fn cut(path: &[Vec2], from: f32, to: f32) -> Vec<Vec2> {
    let mut piece = Vec::new();
    let mut run = 0.0;
    for segment in path.windows(2) {
        let length = segment[0].distance(segment[1]);
        let (start, end) = (run, run + length);
        run = end;
        if end < from || start > to || length <= 0.0 {
            continue;
        }
        let at =
            |along: f32| segment[0].lerp(segment[1], ((along - start) / length).clamp(0.0, 1.0));
        if piece.is_empty() {
            piece.push(at(from.max(start)));
        }
        piece.push(at(to.min(end)));
    }
    piece
}

/// Клинья улицы: пары «срезанный путь, клин, торец» из головного и хвостового
/// клиньев, у кого они есть
pub(super) fn wedge_run<'a>(
    head: &'a Option<Vec<Vec2>>,
    tail: &'a Option<Vec<Vec2>>,
    ends: [Option<Taper>; 2],
) -> Vec<(&'a [Vec2], Taper, bool)> {
    [(head, ends[0], false), (tail, ends[1], true)]
        .into_iter()
        .filter_map(|(path, taper, end)| Some((path.as_deref()?, taper?, end)))
        .collect()
}

/// Асфальт клиньев: от сечения шва к своему (у сужающего сечение шва —
/// узкого соседа, у встречного — широкого), с раскладкой полос,
/// плывущей так же, как линии краски на клине
#[allow(clippy::too_many_arguments)]
pub(super) fn push_asphalt(
    fill: &mut MeshBuilder,
    wedges: &[(&[Vec2], Taper, bool)],
    drawn: &[&RoadLine],
    road: &RoadLine,
    body: &[Vec2],
    breaks: &[Break],
    lanes: Option<LaneFrame>,
    color: LinearRgba,
    traffic_side: TrafficSide,
) {
    for &(path, taper, end) in wedges {
        let seam = drawn[taper.seam];
        // «до разрыва» клина — продолжение срезанной ленты за её торцом: от
        // узла шва до стыка с лентой, чтобы штрихи шли через стык без сдвига
        let length = polyline_length(path);
        let to_break = [
            to_break_beyond(body, road.width, breaks, end, length),
            to_break_beyond(body, road.width, breaks, end, 0.0),
        ];
        match lanes {
            Some(_) => {
                let wedge = WedgeEnd::new(road, seam, taper, end, length, traffic_side);
                let [from, to] = wedge_frames(lane_count(road), wedge, end);
                fill.set_lane_taper(Some(from), Some(to));
            }
            None => fill.set_lanes(None),
        }
        fill.push_taper_sided(
            path,
            wedge_halves(taper.sides, end, |_| [seam.width / 2.0, road.width / 2.0]),
            to_break,
            color,
        );
    }
}

/// Клин тротуара — по сторонам, как у асфальта: с сужаемой стороны
/// от полосы узкого соседа (или его голой кромки, если тротуара у
/// него нет) к своей, с сохранённой — своя на всём клине; сторона
/// без тротуара по тегу — голая кромка (`Drawn::band_half`); и
/// сторона пары у клина половины: там асфальт до разделительной, а
/// тротуар светился за сужаемой кромкой узким языком (пример 16)
#[allow(clippy::too_many_arguments)]
pub(super) fn push_sidewalks(
    fill: &mut MeshBuilder,
    wedges: &[(&[Vec2], Taper, bool)],
    drawn: &[&RoadLine],
    prepared: &Drawn,
    index: usize,
    points: &[Vec2],
    road: &RoadLine,
    body: &[Vec2],
    sidewalk: f32,
    color: LinearRgba,
) {
    let total_length = polyline_length(points);
    for &(path, taper, end) in wedges {
        let middle = wedge_middle(total_length, polyline_length(path), end);
        let paired = prepared
            .pairs()
            .beside(index, middle, 0.0)
            .map(|left| usize::from(!left));
        let halves = wedge_halves(taper.sides, end, |side| {
            if paired == Some(side) {
                [drawn[taper.seam].width / 2.0, road.width / 2.0]
            } else {
                [
                    prepared.band_half(taper.seam, side),
                    prepared.band_half(index, side),
                ]
            }
        });
        // «до разрыва» клина — продолжение срезанной полосы за её торцом
        let width = road.width + 2.0 * sidewalk;
        let length = polyline_length(path);
        let to_break = [
            to_break_beyond(body, width, &[], end, length),
            to_break_beyond(body, width, &[], end, 0.0),
        ];
        fill.push_taper_sided(path, halves, to_break, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::street;

    fn seam(widths: [f32; 2], lanes: [u8; 2]) -> (Vec<RoadLine>, Tapers) {
        let mut roads = vec![
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], widths[0]),
            street(
                vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)],
                widths[1],
            ),
        ];
        for (road, lanes) in roads.iter_mut().zip(lanes) {
            road.lanes = Some(lanes);
        }
        let network = RoadNetwork::new(&roads);
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        let tapers = Tapers::new(&drawn, &network, &nodes, TAPER_PER_METER);
        (roads, tapers)
    }

    #[test]
    fn the_wider_way_takes_the_taper_at_the_seam() {
        let (_, tapers) = seam([8.0, 14.2], [2, 4]);
        assert_eq!(tapers.count, 1);
        assert_eq!(tapers.at(0), [None, None]);
        let [start, end] = tapers.at(1);
        assert_eq!(end, None);
        let start = start.unwrap();
        assert_eq!(start.seam, 0);
        assert!((start.length - 62.0).abs() < 1e-3);
    }

    #[test]
    fn equal_sections_need_no_taper() {
        let (_, tapers) = seam([8.0, 8.0], [2, 2]);
        assert_eq!(tapers.count, 0);
    }

    /// Шов 8 → 14.2 м в узле (100, 0) с третьей дорогой `side`; клин — у
    /// начала широкого way 1, если он есть.
    fn seam_with(side: RoadLine) -> Tapers {
        let mut roads = vec![
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            street(vec![Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)], 14.2),
            side,
        ];
        for road in &mut roads {
            road.lanes = Some(2);
        }
        let network = RoadNetwork::new(&roads);
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        Tapers::new(&drawn, &network, &nodes, TAPER_PER_METER)
    }

    /// Шов 8 → 14.2 м в узле (100, 0) с примыканием сверху и, когда
    /// `continues`, — тупым (160°) продолжателем широкой другого класса.
    fn bent_mouth(continues: bool) -> Tapers {
        let mut roads = vec![
            street(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            street(
                vec![
                    Vec2::new(100.0, 0.0),
                    Vec2::new(100.0 + 95.0 * f32::cos(0.436), 95.0 * f32::sin(0.436)),
                ],
                14.2,
            ),
            street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 80.0)], 8.0),
        ];
        let lanes = [2u8, 4, 2, 2];
        if continues {
            // другой класс — стык не крадёт, одна дорога под 160° к широкой
            let mut obtuse = street(vec![Vec2::new(100.0, 0.0), Vec2::new(5.4, -8.3)], 8.0);
            obtuse.highway = crate::map::osm::Highway::Tertiary;
            roads.push(obtuse);
        }
        for (road, lanes) in roads.iter_mut().zip(lanes) {
            road.lanes = Some(lanes);
        }
        let network = RoadNetwork::new(&roads);
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        Tapers::new(&drawn, &network, &nodes, TAPER_PER_METER)
    }

    #[test]
    fn a_bent_joint_at_a_junction_keeps_the_wide_width() {
        // без тупого продолжения изогнутый стык клин берёт как раньше
        // (Крестовоздвиженская в Союзную, витрина 28)
        assert_eq!(bent_mouth(false).count, 1);
        // с тупым продолжением клина на широкой нет: она доходит до устья
        // своей шириной (Одоевский путепровод × Демонстрации, R37-04), а устье
        // закрывают встречные клинья соседей
        assert_eq!(bent_mouth(true).at(1), [None, None]);
    }

    #[test]
    fn an_obtuse_mouth_widens_its_neighbours_to_the_wide_road() {
        let tapers = bent_mouth(true);
        // узкий шва растёт к узлу: у шва — сечение широкой, к телу — своё,
        // со стороны, обращённой к широкой
        let link = tapers.at(0)[1].unwrap();
        assert_eq!(link.seam, 1);
        assert!((link.length - 62.0).abs() < 1e-3);
        assert_eq!(link.sides, [true, false]);
        // тупой продолжатель — тем же встречным клином со своей стороны
        let mouth = tapers.at(3)[0].unwrap();
        assert_eq!(mouth.seam, 1);
        assert_eq!(mouth.sides, [false, true]);
    }

    #[test]
    fn a_side_street_keeps_its_side_of_the_seam() {
        // примыкание слева по ходу широкого way (вверх): сужается только
        // правая кромка, где уступ ничем не закрыт
        let tapers = seam_with(street(
            vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 80.0)],
            8.0,
        ));
        assert_eq!(tapers.count, 1);
        let start = tapers.at(1)[0].unwrap();
        assert_eq!(start.sides, [false, true]);
        assert_eq!(start.kept(), Some(1.0));
        assert_eq!(start.seam, 0);
    }

    #[test]
    fn a_drive_does_not_hide_a_step_over_its_kerb_radius() {
        // проезд в 5 м поперёк кроет стенку, но уступ в 3.1 м выше его радиуса
        // бордюра (2.5): по сторонам проезда кромка стояла бы зубом (Тула,
        // витрина 22, Болдина 2 → 4 полосы)
        let mut drive = street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 60.0)], 5.0);
        drive.highway = crate::map::osm::Highway::Service;
        assert_eq!(seam_with(drive).at(1)[0].unwrap().sides, [true, true]);
    }

    #[test]
    fn a_crossing_street_covers_both_sides() {
        // узел — вершина сквозной дороги, как в OSM
        let tapers = seam_with(street(
            vec![
                Vec2::new(100.0, -80.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(100.0, 80.0),
            ],
            8.0,
        ));
        assert_eq!(tapers.count, 0);
    }

    #[test]
    fn a_footway_at_the_seam_covers_nothing() {
        let tapers = seam_with(crate::map::osm::fixture::footway(vec![
            Vec2::new(100.0, -80.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 80.0),
        ]));
        assert_eq!(tapers.count, 1);
        let start = tapers.at(1)[0].unwrap();
        assert_eq!(start.sides, [true; 2]);
        assert_eq!(start.kept(), None);
    }

    #[test]
    fn another_class_going_on_collinearly_takes_the_taper_too() {
        let (mut roads, _) = seam([8.0, 14.2], [2, 4]);
        roads[1].highway = crate::map::osm::Highway::Secondary;
        let network = RoadNetwork::new(&roads);
        assert_eq!(network.glued(), 0, "разный класс — разные улицы");
        let nodes = RoadNodes::new(&roads);
        let drawn: Vec<&RoadLine> = roads.iter().collect();
        let tapers = Tapers::new(&drawn, &network, &nodes, TAPER_PER_METER);
        assert_eq!(tapers.count, 1);
        let start = tapers.at(1)[0].unwrap();
        assert_eq!(start.seam, 0);
        assert_eq!(start.sides, [true; 2]);
    }

    #[test]
    fn split_hands_out_both_tapers_from_the_seam_inwards() {
        let path = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
        let [head, middle, tail] = split(&path, [Some(20.0), Some(30.0)]);
        assert_eq!(head.unwrap(), vec![Vec2::ZERO, Vec2::new(20.0, 0.0)]);
        assert_eq!(
            middle.unwrap(),
            vec![Vec2::new(20.0, 0.0), Vec2::new(70.0, 0.0)]
        );
        assert_eq!(
            tail.unwrap(),
            vec![Vec2::new(100.0, 0.0), Vec2::new(70.0, 0.0)]
        );
    }

    #[test]
    fn a_taper_never_takes_more_than_its_share_of_a_short_way() {
        let path = [Vec2::ZERO, Vec2::new(40.0, 0.0), Vec2::new(40.0, 20.0)];
        let [head, _, tail] = split(&path, [Some(66.0), None]);
        assert!(tail.is_none());
        let head = head.unwrap();
        let length = crate::map::osm::model::polyline_length(&head);
        assert!((length - 60.0 * TAPER_MAX_SHARE).abs() < 1e-3, "{length}");
    }
}
