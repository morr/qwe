//! Футпринт линейной геометрии карты — полосы, которые дороги, водотоки и
//! стены занимают на земле, и политика их ширин (кант, бордюр).
//!
//! Одна конструкция на всех потребителей, каждый берёт полосу своим способом:
//! заливка сетки растеризует осевую (`set_polyline` — гарантия 4-связной
//! цепочки), постройка меша строит контур (`ribbon_outline`), рендер кладёт
//! ленту (`push_ribbon` — по **сглаженной копии** осевой, поэтому геометрию
//! полос он не берёт, только ширины). До этого модуля каждая сторона выводила
//! полосы из `points + width` заново, и «одно правило для двух заполнений»
//! держалось на дисциплине; расходились — тайл за тайлом (см. паритетные
//! тесты `navigation/parity_tests.rs`).
//!
//! Ширины живут здесь, а не в рендере: бордюр не только рисуется — он
//! блокирует проходимость, и нарисованная полоса обязана совпадать с
//! заблокированной по построению.

use std::ops::RangeInclusive;

use bevy::prelude::*;

use super::grid::Grid;
use super::meshing::miter_offsets;
use super::osm::model::{
    FenceLine, RoadLine, WallLine, WaterLine, closest_on_segment, distance_to_segment, ring_bounds,
};
use super::roads::network::{RoadNodes, STITCH_MAX_GAP, carries, stitchable};
use crate::settings::PASSAGE_MAX_WIDTH;

/// Насколько близко точка одной ломаной должна лежать к другой ломаной,
/// чтобы дороги считались примыкающими. Развязка в OSM — это общий узел,
/// то есть буквально одна и та же точка в обеих ways; допуск покрывает лишь
/// потерю точности проекции.
///
/// Тем же допуском склеиваются торцы мостовых ways в один мост
/// (`map::roads::Bridges`) — вопрос там другой (сошлись ли **торцы**, а не
/// «есть ли общая точка вообще»), но число одно и то же и по той же причине:
/// это цена проекции, а не свойство вопроса.
pub const JOIN_EPSILON: f32 = 0.5;

/// Минимальное расстояние от точки до ломаной — по всем её сегментам.
pub fn distance_to_polyline(point: Vec2, points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| distance_to_segment(point, segment[0], segment[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Примыкают ли две ломаные — у какой-нибудь точки одной есть сосед на другой
/// ближе [`JOIN_EPSILON`].
///
/// Тест симметричен намеренно: общий узел может оказаться серединой одной из
/// ways, и односторонняя проверка его пропустит.
///
/// Живёт у футпринта, а не в каждой заливке: один и тот же вопрос задают и
/// заливка сетки, и постройка полигонального меша, и ответы обязаны совпадать
/// — разойдясь, они открыли бы бордюр моста в одном бэкенде и не открыли в
/// другом. Потребители берут его через [`CurbCoverage`].
pub fn ways_joined(first: &[Vec2], second: &[Vec2]) -> bool {
    first
        .iter()
        .any(|&point| distance_to_polyline(point, second) < JOIN_EPSILON)
        || second
            .iter()
            .any(|&point| distance_to_polyline(point, first) < JOIN_EPSILON)
}

/// Могут ли ломаные с такими AABB примыкать — коробки пересекаются с запасом
/// [`JOIN_EPSILON`].
///
/// Префильтр к [`ways_joined`], а не замена ему: `true` не утверждает ничего,
/// `false` — утверждает. Отрезок лежит внутри своей коробки, значит расстояние
/// до отрезка не меньше расстояния до коробки; разошлись коробки больше чем на
/// эпсилон — разошлись и точки. Запас обязателен: примыкание в OSM — общий
/// узел, но проекция теряет точность, и стык в 0.4 м от торца моста коробок уже
/// не пересекает.
fn boxes_may_join(first: (Vec2, Vec2), second: (Vec2, Vec2)) -> bool {
    (first.0 - JOIN_EPSILON).cmple(second.1).all() && (second.0 - JOIN_EPSILON).cmple(first.1).all()
}

/// Кант — 8% ширины дороги в разумных пределах.
const CASING_SCALE: f32 = 0.08;
const CASING_RANGE: RangeInclusive<f32> = 0.3..=1.0;

/// Бордюр моста — толще и темнее канта (12%), чтобы никогда с ним не
/// сливаться.
const BRIDGE_CURB_SCALE: f32 = 0.12;
const BRIDGE_CURB_RANGE: RangeInclusive<f32> = 0.8..=2.0;

/// Толщина канта для ленты такой ширины. Общая с подложкой аллей
/// (`map::spawn`) и клиренсом посадки деревьев (`planting/index.rs`), чтобы
/// кант везде на карте был одной толщины.
pub fn casing_width(width: f32) -> f32 {
    (width * CASING_SCALE).clamp(*CASING_RANGE.start(), *CASING_RANGE.end())
}

/// Толщина бордюра моста для дороги такой ширины.
pub fn bridge_curb_width(width: f32) -> f32 {
    (width * BRIDGE_CURB_SCALE).clamp(*BRIDGE_CURB_RANGE.start(), *BRIDGE_CURB_RANGE.end())
}

/// Полоса: осевая + ширина. Осевая, а не готовый контур, намеренно —
/// сетке нужна именно осевая, чтобы растеризация осталась 4-связной цепочкой
/// (тонкая косая полоса из одного контура рассыпалась бы в шахматку).
/// Что полоса значит на земле — блокирует или прорезает — решает потребитель
/// порядком заливки, самой полосе роль не нужна.
pub struct Band {
    pub line: Vec<Vec2>,
    pub width: f32,
}

impl RoadLine {
    /// Толщина бордюра этого моста.
    pub fn curb_width(&self) -> f32 {
        bridge_curb_width(self.width)
    }

    /// Полуширина всей мостовой полосы: настил + бордюр. Ею меряют «накрыт ли
    /// сосед лентой моста» и щуп сетки, и разность меша; рендер рисует
    /// бордюрную подложку шириной `2 × curb_reach`.
    pub fn curb_reach(&self) -> f32 {
        self.width / 2.0 + self.curb_width()
    }

    /// Настил моста — ровно проезжая часть, как её рисует рендер. Сеточная
    /// поправка на блуждание тайловых центров (`− tile·√2`) сюда не входит:
    /// она — свойство растеризации, не футпринта.
    pub fn deck_band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width,
        }
    }

    /// Две бордюрные полосы моста: осевая каждой — кромка настила плюс
    /// полбордюра (`miter_offsets`, общие с рендером), ширина — бордюр.
    /// Осмысленно только для `bridge`-дорог; обе заливки фильтруют по флагу
    /// до вызова.
    pub fn curb_bands(&self) -> [Band; 2] {
        let curb = self.curb_width();
        let offsets = miter_offsets(&self.points, false, (self.width + curb) / 2.0);
        [-1.0f32, 1.0].map(|side| Band {
            line: self
                .points
                .iter()
                .zip(&offsets)
                .map(|(&point, &offset)| point + side * offset)
                .collect(),
            width: curb,
        })
    }

    /// Прорезь арки: осевая прохода с шириной, капнутой
    /// [`PASSAGE_MAX_WIDTH`] — way обычно `service` (5 м), а сама арка у́же,
    /// и некапнутый коридор съедал бы фасад с обеих сторон.
    pub fn passage_band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width.min(PASSAGE_MAX_WIDTH),
        }
    }
}

impl WaterLine {
    /// Русло как полоса; `None` — труба: не рисуется и не блокирует, над
    /// кульвертом земля. Капы торцов (плоский срез у портала трубы —
    /// `water_line_caps`) остаются у потребителей: это правило пары линий, а
    /// не одной.
    pub fn channel_band(&self) -> Option<Band> {
        (!self.tunnel).then(|| Band {
            line: self.points.clone(),
            width: self.width,
        })
    }
}

impl WallLine {
    pub fn band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: self.width,
        }
    }
}

/// Физическая толщина ограды, м — то, что перекрывает навмеш. Не ширина
/// отрисовки: та растёт с зумом (`fences::FENCE_LODS`, 0.25 → 1.3 м) ради
/// экранных пикселей, а забор на земле от зума толще не становится. Сетке
/// толщина почти безразлична — непроходимость держит цепочка тайлов по осевой
/// (`Navmesh::set_polyline`), — а полигональному мешу она даёт контур,
/// который потом раздувается на радиус агента.
pub const FENCE_BAND_WIDTH: f32 = 0.3;

impl FenceLine {
    pub fn band(&self) -> Band {
        Band {
            line: self.points.clone(),
            width: FENCE_BAND_WIDTH,
        }
    }
}

/// Проём в ограде: точка на осевой забора, где его пересекает дорога, и
/// полудлина проёма вдоль забора, м.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FenceGap {
    pub at: Vec2,
    pub reach: f32,
}

/// Ширина калитки по умолчанию (`FenceLine::gates`), м — ширина тропинки
/// `footway`: калитка, через которую в OSM никто не провёл дорогу, — это
/// калитка для пешехода.
pub const FENCE_GATE_WIDTH: f32 = 3.5;

/// Во сколько раз косое пересечение может удлинить проём против поперечного.
/// Дорога под углом θ занимает на заборе `ширина / sin θ`, и на почти
/// параллельном пересечении это число уходит в бесконечность — а проём длиной
/// в квартал уже не калитка.
const GAP_OBLIQUITY_MAX: f32 = 2.0;

/// Сторона ячейки индекса отрезков дорог для поиска проёмов, м. Ответ от неё не
/// зависит — отрезок регистрируется во всех ячейках своей коробки, — только
/// скорость.
const GAP_CELL: f32 = 32.0;

/// Насколько далеко от ограды OSM бросает торец дороги, которая на месте
/// проходит сквозь неё, м. Число то же и по той же причине, что у стежка
/// ([`STITCH_MAX_GAP`]): столько картограф не доводит проезд до того, во что
/// тот упирается. Одна константа, а не две одинаковых, потому что факт один —
/// разъедься они, стежок дотянул бы асфальт сквозь несрезанный забор.
const GAP_END_REACH: f32 = STITCH_MAX_GAP;

/// Проёмы всех оград: где сквозь забор проходит дорога.
///
/// **Проём даёт только пересечение осевых**, не близость лент. Дорога вдоль
/// забора в метре от него — обычная улица частного сектора — своей номинальной
/// лентой в 8–16 м накрывает забор на всём протяжении, и правило «лента
/// накрыла — открыто» сняло бы все заборы вдоль улиц, оставив только те, что в
/// глубине кварталов. Осевая же идёт по середине проезда, и на заборе она
/// оказывается только там, где через него действительно ходят: калитка
/// (тропинка `footway`), въезд (`service`), а в OSM и общий узел дороги с
/// забором — тоже пересечение.
///
/// Второй случай — **торец дороги у забора**: тропа, доведённая до калитки и
/// там оборванная, осевой забор не пересекает, но упирается в него. Её конец
/// ближе полуширины к осевой ограды — проём в ближайшей точке.
///
/// И тот же торец, **не доведённый** до ограды: въезд во двор OSM размечает
/// «до тротуара» и бросает в нескольких метрах от ворот, за которыми он на
/// месте продолжается. Такой торец — висячий в смысле стежка
/// (`roads::network::stitches`: в узле нет другой дороги, несущей его
/// полотно), смотрящий вперёд на ограду — открывает проём не дальше
/// [`GAP_END_REACH`]. Это та же небрежность разметки, которую с отрисовочной
/// стороны закрывает стежок, и закрывать её надо здесь тоже: стежок дотягивает
/// проезд до улицы за оградой, и нарисованный асфальт шёл сквозь несрезанный
/// забор (Тула, way 205998518 у улицы Мосина — въезд к Свято-Никольскому).
/// На Туле правило открывает 50 проёмов (318 → 368, `fence_prune_audit`), и
/// **оба его условия несут вес**: по тому же кешу оно срабатывает 52 раза с
/// ними обоими, 197 раз без проверки на висячесть (улица вдоль забора,
/// разрезанная картографом на ways, даёт торец у ограды на каждом стыке) и 91
/// без «вперёд» (торец в паре метров **сбоку** от ограды — это дорога вдоль
/// неё, а не в неё).
///
/// **Мосты не режут**: пролёт идёт над оградой, а не сквозь неё. Совпадающие
/// осевые (забор, нанесённый на тот же way, что и тропа) пересечением не
/// считаются — у параллельных отрезков его нет.
///
/// Третий — **калитки по умолчанию** (`FenceLine::gates`), шириной
/// [`FENCE_GATE_WIDTH`]: их находит сетка при загрузке, а проёмами они
/// становятся здесь, для обоих заполнений сразу.
///
/// Индекс — `[номер ограды] → проёмы`, в порядке `fences`.
pub fn fence_gaps(fences: &[FenceLine], roads: &[RoadLine]) -> Vec<Vec<FenceGap>> {
    if fences.is_empty() {
        return Vec::new();
    }
    // отрезки дорог по ячейкам своих коробок: дорог десятки тысяч, оград сотни,
    // и перебор пар «забор × дорога» мерил бы расстояния впустую
    let mut cells: Grid<(u32, u32)> = Grid::new(GAP_CELL);
    for (road_index, road) in roads.iter().enumerate() {
        if road.bridge {
            continue;
        }
        // коробка растёт на столько, на сколько отрезок вообще может открыть
        // проём: полуширина у пересечения, [`GAP_END_REACH`] у торца
        let grown = road.width.max(GAP_END_REACH);
        for (segment, pair) in road.points.windows(2).enumerate() {
            cells.insert_segment(pair[0], pair[1], grown, (road_index as u32, segment as u32));
        }
    }
    // висячие торцы — тем же вопросом, что у стежка: в узле нет другой дороги,
    // несущей это полотно. Кольцевой проезд торцов не имеет
    let nodes = RoadNodes::new(roads);
    let loose: Vec<[bool; 2]> = roads
        .iter()
        .enumerate()
        .map(|(index, road)| {
            if !stitchable(road) {
                return [false; 2];
            }
            let last = road.points.len() - 1;
            if road.points[0] == road.points[last] {
                return [false; 2];
            }
            let free = |end: Vec2| {
                !nodes
                    .roads_at(end)
                    .iter()
                    .any(|&other| other != index && carries(road, &roads[other]))
            };
            [free(road.points[0]), free(road.points[last])]
        })
        .collect();
    fences
        .iter()
        .map(|fence| {
            let mut gaps: Vec<FenceGap> = Vec::new();
            for &at in &fence.gates {
                push_gap(
                    &mut gaps,
                    FenceGap {
                        at,
                        reach: FENCE_GATE_WIDTH / 2.0,
                    },
                );
            }
            let mut seen: Vec<(u32, u32)> = Vec::new();
            for pair in fence.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let Some(along) = (b - a).try_normalize() else {
                    continue;
                };
                seen.clear();
                for &key in cells.near_each(a.min(b), a.max(b)) {
                    if seen.contains(&key) {
                        continue;
                    }
                    seen.push(key);
                    let road = &roads[key.0 as usize];
                    let (c, d) = (road.points[key.1 as usize], road.points[key.1 as usize + 1]);
                    let Some(direction) = (d - c).try_normalize() else {
                        continue;
                    };
                    let sin = along.perp_dot(direction).abs();
                    let reach = road.width / 2.0 / sin.max(1.0 / GAP_OBLIQUITY_MAX);
                    if let Some(at) = segment_crossing(a, b, c, d) {
                        push_gap(&mut gaps, FenceGap { at, reach });
                    }
                    let last = road.points.len() - 2;
                    for (side, (end, is_end, outward)) in [
                        (c, key.1 == 0, -direction),
                        (d, key.1 as usize == last, direction),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if !is_end {
                            continue;
                        }
                        let at = closest_on_segment(end, a, b);
                        // висячий торец, смотрящий на ограду, достаёт
                        // до неё через зазор небрежной разметки
                        let aimed = loose[key.0 as usize][side] && (at - end).dot(outward) > 0.0;
                        let limit = if aimed {
                            (road.width / 2.0).max(GAP_END_REACH)
                        } else {
                            road.width / 2.0
                        };
                        if at.distance(end) <= limit {
                            push_gap(&mut gaps, FenceGap { at, reach });
                        }
                    }
                }
            }
            gaps
        })
        .collect()
}

/// Проём, совпавший с уже найденным (общий узел двух отрезков забора даёт
/// одно и то же пересечение дважды), не дублируется — остаётся шире из двух.
fn push_gap(gaps: &mut Vec<FenceGap>, gap: FenceGap) {
    if let Some(same) = gaps
        .iter_mut()
        .find(|known| known.at.distance(gap.at) < JOIN_EPSILON)
    {
        same.reach = same.reach.max(gap.reach);
    } else {
        gaps.push(gap);
    }
}

/// Точка пересечения отрезков `a→b` и `c→d`, концы включительно. Параллельные
/// и совпадающие отрезки пересечения не имеют.
fn segment_crossing(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let (r, s) = (b - a, d - c);
    let denominator = r.perp_dot(s);
    if denominator.abs() <= f32::EPSILON * r.length() * s.length() {
        return None;
    }
    let t = (c - a).perp_dot(s) / denominator;
    let u = (c - a).perp_dot(r) / denominator;
    const SLACK: f32 = 1e-4;
    ((-SLACK..=1.0 + SLACK).contains(&t) && (-SLACK..=1.0 + SLACK).contains(&u)).then(|| a + r * t)
}

/// Как далеко искать проезжую часть от кандидата в калитку, м. Дальше — «улицы
/// рядом нет», и все такие кандидаты равны.
const STREET_REACH: f32 = 96.0;

/// Кромки проезжих частей (`roads::is_carriageway`) — для вопроса «где у
/// огороженного участка сторона к улице». Калитка по умолчанию встаёт туда:
/// вход в школу или на завод делают с большой дороги, а не с тропинки на
/// задах, даже если тропинка ближе к достижимому.
pub struct StreetEdges<'a> {
    roads: &'a [RoadLine],
    segments: Grid<(u32, u32)>,
}

impl<'a> StreetEdges<'a> {
    pub fn build(roads: &'a [RoadLine]) -> Self {
        let mut segments: Grid<(u32, u32)> = Grid::new(GAP_CELL);
        for (index, road) in roads.iter().enumerate() {
            if !super::roads::is_carriageway(road) {
                continue;
            }
            for (segment, pair) in road.points.windows(2).enumerate() {
                segments.insert_segment(pair[0], pair[1], 0.0, (index as u32, segment as u32));
            }
        }
        Self { roads, segments }
    }

    /// Расстояние от точки до кромки ближайшей проезжей части (осевая минус
    /// полширины, не меньше нуля), не дальше [`STREET_REACH`] — иначе
    /// `STREET_REACH`. Кромка, а не осевая: широкая улица притягивает сильнее.
    pub fn distance(&self, point: Vec2) -> f32 {
        let mut best = STREET_REACH;
        for &(road, segment) in self.segments.near_each(point - STREET_REACH, point + STREET_REACH) {
            let road = &self.roads[road as usize];
            let (a, b) = (road.points[segment as usize], road.points[segment as usize + 1]);
            let edge = (distance_to_segment(point, a, b) - road.width / 2.0).max(0.0);
            best = best.min(edge);
        }
        best
    }
}

/// Ограда, как она стоит: осевая, из которой вынуто всё, что лежит внутри
/// кругов проёмов, — куски между проёмами, каждый своей ломаной.
///
/// Круг тот же, что вычитает полигональный меш (`gap_outline`), так что
/// нарисованный проём и проём, через который ходят, — одно место. Сетка режет
/// шире на диагональ тайла, но это поправка растеризации, а не футпринта (как
/// `− tile·√2` у настила моста). Кусок короче [`MIN_FENCE_PIECE`] не
/// рисуется: огрызок забора в полметра у края калитки — это шум, а не столб.
pub fn fence_pieces(fence: &FenceLine, gaps: &[FenceGap]) -> Vec<Vec<Vec2>> {
    fn close(current: &mut Vec<Vec2>, pieces: &mut Vec<Vec<Vec2>>) {
        let length: f32 = current
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum();
        if current.len() >= 2 && length >= MIN_FENCE_PIECE {
            pieces.push(std::mem::take(current));
        } else {
            current.clear();
        }
    }
    let mut pieces: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for pair in fence.points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        // интервалы параметра звена, накрытые проёмами, — пересечение звена с
        // каждым кругом
        let mut covered: Vec<(f32, f32)> = gaps
            .iter()
            .filter_map(|gap| segment_in_circle(a, b, gap.at, gap.reach))
            .collect();
        covered.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut t = 0.0;
        for (from, to) in covered {
            if from > t {
                if current.is_empty() {
                    current.push(a.lerp(b, t));
                }
                current.push(a.lerp(b, from));
                close(&mut current, &mut pieces);
            } else if !current.is_empty() {
                close(&mut current, &mut pieces);
            }
            t = t.max(to);
        }
        if t < 1.0 {
            if current.is_empty() {
                current.push(a.lerp(b, t));
            }
            current.push(b);
        } else {
            close(&mut current, &mut pieces);
        }
    }
    close(&mut current, &mut pieces);
    pieces
}

/// Кусок ограды короче этого, м, после разрезки проёмами не рисуется.
const MIN_FENCE_PIECE: f32 = 0.5;

/// Часть звена `a→b` внутри круга — интервал параметра `[from, to]` в
/// `0..=1`; `None`, если звено круга не касается.
fn segment_in_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<(f32, f32)> {
    let d = b - a;
    let f = a - center;
    let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - radius * radius);
    if qa <= f32::EPSILON {
        return None;
    }
    let discriminant = qb * qb - 4.0 * qa * qc;
    if discriminant <= 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let from = ((-qb - root) / (2.0 * qa)).max(0.0);
    let to = ((-qb + root) / (2.0 * qa)).min(1.0);
    (from < to).then_some((from, to))
}

/// Входы решения «какая часть бордюра составного моста открыта»: мосты и
/// примыкающие к ним не-мосты, отобранные одним предикатом ([`ways_joined`])
/// для обеих заливок.
///
/// Общие здесь именно **входы**. Само решение у заливок осознанно разное, и
/// это не долг, а два ответа на один случай-ловушку (номинальная ширина
/// primary 16 м заглатывает свой параллельный тротуар целиком): сетка решает
/// **направленным щупом** «есть ли лента снаружи от меня» (см.
/// `navmesh::fill_from_mapdata`), меш — **полигональной разностью**, у которой
/// от заглатывания выживают тонкие внешние обрезки-барьеры (см.
/// `polymesh/build.rs`). Точечный тест покрытия, общий для обеих, не
/// воспроизводит ни то, ни другое: с допуском он открывает внешний барьер, без
/// допуска — крошит бордюрную цепочку в пунктир. Проверено анализом при
/// попытке унификации; менять любую из стратегий — только через пин-тесты
/// бордюров (`navmesh/tests.rs`) и паритетные (`navigation/parity_tests.rs`).
pub struct CurbCoverage<'a> {
    bridges: Vec<&'a RoadLine>,
    joining: Vec<&'a RoadLine>,
}

impl<'a> CurbCoverage<'a> {
    pub fn build(roads: &'a [RoadLine]) -> Self {
        let bridges: Vec<&RoadLine> = roads.iter().filter(|road| road.bridge).collect();
        // AABB-прекомпьют по мостам — тем же приёмом, что у
        // `parse::drop_buildings_in_water`. Дорог на карте десятки тысяч
        // (Лондон 43 000), мостов сотни, а примыкает ~1%, и короткое замыкание
        // `.any()` срабатывает ровно у этого процента: остальные честно мерили
        // расстояние до каждого моста, точка за отрезком. Лондон — 1.00 с на
        // вызов против 19 мс, и вызова два, оба на загрузке (заливка сетки и
        // постройка полигонального меша)
        let bridge_boxes: Vec<(Vec2, Vec2)> = bridges
            .iter()
            .map(|bridge| ring_bounds(&bridge.points))
            .collect();
        let joining = roads
            .iter()
            .filter(|road| !road.bridge)
            .filter(|road| {
                let road_bounds = ring_bounds(&road.points);
                bridges
                    .iter()
                    .zip(&bridge_boxes)
                    .any(|(bridge, &bridge_bounds)| {
                        boxes_may_join(road_bounds, bridge_bounds)
                            && ways_joined(&road.points, &bridge.points)
                    })
            })
            .collect();
        Self { bridges, joining }
    }

    /// Мосты в порядке обхода `roads` — обе заливки нумеруют владельцев
    /// бордюров этим же порядком.
    pub fn bridges(&self) -> &[&'a RoadLine] {
        &self.bridges
    }

    /// Не-мосты, примыкающие хотя бы к одному мосту (общий узел, не близость
    /// в плане): их полотно открывает бордюр, который накрывает.
    pub fn joining(&self) -> &[&'a RoadLine] {
        &self.joining
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture;

    fn length(piece: &[Vec2]) -> f32 {
        piece.windows(2).map(|pair| pair[0].distance(pair[1])).sum()
    }

    /// Тропинка поперёк забора: проём на пересечении осевых, забор рисуется
    /// двумя кусками, обрезанными ровно по кругу проёма.
    #[test]
    fn a_footway_splits_the_drawn_fence_at_its_gap() {
        let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
        let road = fixture::footway(vec![Vec2::new(20.0, -10.0), Vec2::new(20.0, 10.0)]);
        let gaps = &fence_gaps(std::slice::from_ref(&fence), &[road])[0];
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].at.distance(Vec2::new(20.0, 0.0)) < 1e-3);
        let pieces = fence_pieces(&fence, gaps);
        assert_eq!(pieces.len(), 2);
        let reach = gaps[0].reach;
        assert!((pieces[0].last().unwrap().x - (20.0 - reach)).abs() < 1e-3);
        assert!((pieces[1].first().unwrap().x - (20.0 + reach)).abs() < 1e-3);
    }

    /// Улица вдоль забора его не режет — ни в навмеше, ни на картинке.
    #[test]
    fn a_street_along_the_fence_leaves_no_gap() {
        let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
        let road = fixture::street(vec![Vec2::new(-10.0, 4.0), Vec2::new(50.0, 4.0)], 16.0);
        assert!(fence_gaps(std::slice::from_ref(&fence), &[road])[0].is_empty());
    }

    /// Въезд, брошенный OSM в нескольких метрах от ограды и смотрящий на неё,
    /// открывает ворота: на месте он идёт сквозь них, и отрисовка дотягивает
    /// его туда стежком. Дальше [`GAP_END_REACH`] — уже задуманный тупик.
    #[test]
    fn a_drive_dropped_short_of_the_fence_opens_a_gate() {
        let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
        let short =
            |gap: f32| fixture::street(vec![Vec2::new(20.0, -20.0), Vec2::new(20.0, -gap)], 5.0);
        let gaps = &fence_gaps(std::slice::from_ref(&fence), &[short(4.0)])[0];
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].at.distance(Vec2::new(20.0, 0.0)) < 1e-3);
        assert!((gaps[0].reach - 2.5).abs() < 1e-3, "{}", gaps[0].reach);
        assert!(
            fence_gaps(std::slice::from_ref(&fence), &[short(GAP_END_REACH + 1.0)])[0].is_empty()
        );
    }

    /// Но только висячий торец и только смотрящий вперёд: улица, идущая вдоль
    /// забора, и way, разрезанная картографом посреди проезда, ворот не
    /// открывают.
    #[test]
    fn a_way_split_beside_the_fence_opens_nothing() {
        let fence = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0)]);
        // торец в четырёх метрах сбоку от ограды: дорога идёт вдоль неё
        let along = fixture::street(vec![Vec2::new(0.0, -4.0), Vec2::new(20.0, -4.0)], 5.0);
        assert!(fence_gaps(std::slice::from_ref(&fence), &[along])[0].is_empty());

        // тот же торец, но полотно продолжает вторая way — торец не висячий
        let first = fixture::street(vec![Vec2::new(20.0, -20.0), Vec2::new(20.0, -4.0)], 5.0);
        let second = fixture::street(vec![Vec2::new(20.0, -4.0), Vec2::new(32.0, -8.0)], 5.0);
        assert!(fence_gaps(std::slice::from_ref(&fence), &[first, second])[0].is_empty());
    }

    /// Проём на изломе режет оба звена, а кольцо ограды с калиткой остаётся
    /// ломаной без калитки: длина кусков — периметр минус проём. Огрызок короче
    /// полуметра у края не рисуется.
    #[test]
    fn a_gate_at_a_corner_cuts_both_links_and_drops_the_stub() {
        let fence = fixture::fence(fixture::closed(fixture::square(Vec2::ZERO, 10.0)));
        let gate = FenceGap {
            at: Vec2::new(-10.0, -10.0),
            reach: 1.75,
        };
        let pieces = fence_pieces(&fence, &[gate]);
        let total: f32 = pieces.iter().map(|piece| length(piece)).sum();
        assert!((total - (80.0 - 2.0 * 1.75)).abs() < 1e-3, "{total}");

        // за проёмом остаётся 0.3 м забора — не рисуется
        let stub = FenceGap {
            at: Vec2::new(19.0, 0.0),
            reach: 0.7,
        };
        let line = fixture::fence(vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0)]);
        let pieces = fence_pieces(&line, &[stub]);
        assert_eq!(pieces.len(), 1);
        assert!((pieces[0][1].x - 18.3).abs() < 1e-3);
    }

    /// Общий узел посреди одной из ways ловится с любой стороны — ровно ради
    /// этого случая предикат симметричен.
    #[test]
    fn a_way_ending_in_the_middle_of_another_still_joins_it() {
        let through = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let stub = [Vec2::new(50.0, 0.0), Vec2::new(50.0, 40.0)];
        assert!(ways_joined(&through, &stub));
        assert!(ways_joined(&stub, &through));
    }

    /// Тропа, прошедшая под пролётом, узла не делит: примыкание — это общая
    /// точка, а не близость в плане.
    #[test]
    fn a_way_passing_by_does_not_join() {
        let bridge = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let under = [Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)];
        assert!(!ways_joined(&bridge, &under));
    }

    /// Отбор примыкающих: общий узел — да; проход ПОД пролётом — нет, хотя
    /// коробки у него и у моста совпадают (решает по-прежнему `ways_joined`,
    /// а не префильтр); дальняя дорога — нет.
    #[test]
    fn coverage_takes_only_ways_sharing_a_node_with_a_bridge() {
        let roads = vec![
            fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            fixture::street(vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 60.0)], 8.0),
            fixture::street(vec![Vec2::new(0.0, 2.0), Vec2::new(100.0, 2.0)], 3.5),
            fixture::street(vec![Vec2::new(500.0, 500.0), Vec2::new(600.0, 500.0)], 8.0),
        ];
        let coverage = CurbCoverage::build(&roads);
        assert_eq!(coverage.bridges().len(), 1);
        assert_eq!(coverage.joining().len(), 1);
        assert_eq!(coverage.joining()[0].points[0], Vec2::new(100.0, 0.0));
    }

    /// Отбор идёт с запасом [`JOIN_EPSILON`] на обеих сторонах: дорога,
    /// начинающаяся в 0.4 м от торца моста, примыкает — хотя её коробка
    /// коробку моста не пересекает, и любой отбор по голому AABB её потеряет.
    #[test]
    fn a_way_ending_just_short_of_a_bridge_still_joins_it() {
        let roads = vec![
            fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0),
            fixture::street(vec![Vec2::new(100.4, 0.0), Vec2::new(200.0, 0.0)], 8.0),
        ];
        assert_eq!(CurbCoverage::build(&roads).joining().len(), 1);
    }

    #[test]
    fn casing_stays_within_its_range_on_every_road_class() {
        for width in [3.5_f32, 5.0, 8.0, 16.0] {
            assert!(casing_width(width) >= *CASING_RANGE.start());
            assert!(casing_width(width) <= *CASING_RANGE.end());
        }
    }

    /// Бордюр обязан торчать из-под канта на любом классе — иначе при
    /// включённом канте мост неотличим от окантованной дороги.
    #[test]
    fn bridge_curb_is_thicker_than_a_casing() {
        for width in [3.5_f32, 5.0, 8.0, 16.0] {
            assert!(bridge_curb_width(width) > casing_width(width));
            assert!(bridge_curb_width(width) >= *BRIDGE_CURB_RANGE.start());
            assert!(bridge_curb_width(width) <= *BRIDGE_CURB_RANGE.end());
        }
    }

    /// Кромки бордюрных полос отстоят от осевой ровно на полширины настила
    /// плюс полбордюра — та самая конструкция, которой пользуются обе заливки
    /// и рендер.
    #[test]
    fn curb_bands_sit_on_both_deck_edges() {
        let road = fixture::bridge(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 8.0);
        let [left, right] = road.curb_bands();
        let offset = (road.width + road.curb_width()) / 2.0;
        assert_eq!(left.width, road.curb_width());
        assert_eq!(left.line[0].y, -offset);
        assert_eq!(right.line[0].y, offset);
        assert_eq!(
            road.curb_reach(),
            road.width / 2.0 + road.curb_width(),
            "щуп сетки и разность меша меряют одну и ту же полуширину"
        );
    }
}
