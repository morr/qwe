//! Проёмы оград и индексы заборов: где сквозь ограду ходят, что из неё
//! вырезается на картинке и как её быстро находит калитка.
//!
//! Физическая толщина ограды и её полоса — в родителе ([`FenceLine::band`]);
//! нарисованные настилы мостов, которыми [`fence_pieces`] режет забор
//! сверху, — в [`super::decks`].

use bevy::prelude::*;

use super::JOIN_EPSILON;
use super::decks::BridgeDecks;
use crate::map::grid::Grid;
use crate::map::osm::model::{FenceLine, RoadLine, closest_on_segment};
use crate::map::roads::network::{RoadNodes, STITCH_MAX_GAP, carries, stitchable};

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
pub(super) const GAP_CELL: f32 = 32.0;

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
/// **Мосты не режут**: пролёт идёт над оградой, а не сквозь неё — под ним
/// ограда стоит и блокирует, хоть и не рисуется ([`BridgeDecks`]). Совпадающие
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
    FenceGapCache::build(fences, roads).into_gaps()
}

/// Дорожная половина проёмов оград — то, что от калиток не зависит, — вместе
/// со слитым списком проёмов каждой ограды. Собирается один раз на заливку
/// ([`Navmesh::fill_from_mapdata`]) и переиспользуется раундами калиток
/// ([`Navmesh::open_sealed_fences`]): каждая калитка растит список проёмов
/// своей ограды, а пересборка сетки всех дорог и повторная детекция по всем
/// оградам на каждый раунд стоили гейтам больше половины секунды (запись 26).
///
/// Ответ тот же, что у разовой сборки [`fence_gaps`], байт в байт: дорожные
/// проёмы детектируются тем же циклом один раз ([`Self::detect`]), а слияние
/// повторяет исходный порядок [`push_gap`] — калитки ограды первыми, дорожные
/// проёмы поверх, в порядке детекции.
pub struct FenceGapCache {
    /// Отрезки дорог по ячейкам своих коробок: дорог десятки тысяч, оград
    /// сотни, и перебор пар «забор × дорога» мерил бы расстояния впустую.
    cells: Grid<(u32, u32)>,
    loose: Vec<[bool; 2]>,
    /// Дорожные проёмы каждой ограды — в порядке детекции, без калиток.
    crossings: Vec<Vec<FenceGap>>,
    /// Слитые проёмы: калитки ограды первыми, дорожные поверх.
    merged: Vec<Vec<FenceGap>>,
    /// Длина `gates` каждой ограды на момент слияния: выросла — слить заново.
    gate_lens: Vec<usize>,
}

impl FenceGapCache {
    pub fn build(fences: &[FenceLine], roads: &[RoadLine]) -> Self {
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
        let nodes = RoadNodes::new(roads);
        // висячие торцы — тем же вопросом, что у стежка: в узле нет другой
        // дороги, несущей это полотно. Кольцевой проезд торцов не имеет
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
        let mut cache = Self {
            cells,
            loose,
            crossings: Vec::new(),
            merged: Vec::new(),
            gate_lens: Vec::new(),
        };
        cache.detect(fences, roads);
        cache
    }

    /// Слить заново ограды, у которых вырос список калиток: дорожные проёмы
    /// не пересчитываются, меняется только их слияние с калитками.
    ///
    /// Срез обязан быть тем же, что получил [`FenceGapCache::detect`], — длина
    /// и порядок: слияние идёт позиционно. Расхождение укорачивает `zip` и
    /// молча пропускает хвост; ассерт держит эту дисциплину в отладке.
    pub fn refresh(&mut self, fences: &[FenceLine]) {
        debug_assert_eq!(
            fences.len(),
            self.crossings.len(),
            "FenceGapCache::refresh got a different fence slice than detect"
        );
        for (((fence, crossings), merged), len) in fences
            .iter()
            .zip(&self.crossings)
            .zip(&mut self.merged)
            .zip(&mut self.gate_lens)
        {
            if fence.gates.len() != *len {
                *merged = merge_gaps(&fence.gates, crossings);
                *len = fence.gates.len();
            }
        }
    }

    pub fn gaps(&self) -> &[Vec<FenceGap>] {
        &self.merged
    }

    fn into_gaps(self) -> Vec<Vec<FenceGap>> {
        self.merged
    }

    /// Дорожные проёмы каждой ограды и их слияние с её калитками — тем же
    /// циклом, что у разовой сборки, в том же порядке.
    fn detect(&mut self, fences: &[FenceLine], roads: &[RoadLine]) {
        let crossings: Vec<Vec<FenceGap>> = fences
            .iter()
            .map(|fence| {
                let mut crossings: Vec<FenceGap> = Vec::new();
                let mut seen: Vec<(u32, u32)> = Vec::new();
                for pair in fence.points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let Some(along) = (b - a).try_normalize() else {
                        continue;
                    };
                    seen.clear();
                    for &key in self.cells.near_each(a.min(b), a.max(b)) {
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
                            crossings.push(FenceGap { at, reach });
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
                            let aimed =
                                self.loose[key.0 as usize][side] && (at - end).dot(outward) > 0.0;
                            let limit = if aimed {
                                (road.width / 2.0).max(GAP_END_REACH)
                            } else {
                                road.width / 2.0
                            };
                            if at.distance(end) <= limit {
                                crossings.push(FenceGap { at, reach });
                            }
                        }
                    }
                }
                crossings
            })
            .collect();
        self.merged = fences
            .iter()
            .zip(&crossings)
            .map(|(fence, crossings)| merge_gaps(&fence.gates, crossings))
            .collect();
        self.crossings = crossings;
        self.gate_lens = fences.iter().map(|fence| fence.gates.len()).collect();
    }
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

/// Слитые проёмы одной ограды: калитки ([`FENCE_GATE_WIDTH`]/2) первыми,
/// дорожные поверх — тот же порядок [`push_gap`], что у разовой сборки
/// [`fence_gaps`], поэтому итог совпадает с ней байт в байт.
fn merge_gaps(gates: &[Vec2], crossings: &[FenceGap]) -> Vec<FenceGap> {
    let mut gaps: Vec<FenceGap> = Vec::new();
    for &at in gates {
        push_gap(
            &mut gaps,
            FenceGap {
                at,
                reach: FENCE_GATE_WIDTH / 2.0,
            },
        );
    }
    for gap in crossings {
        push_gap(&mut gaps, *gap);
    }
    gaps
}

/// Доли `t` и `u`, в которых прямые `a→b` и `c→d` пересекаются, — каждая вдоль
/// своего отрезка. `None` — отрезки параллельны или совпадают: пересечения у
/// них нет, и допуск на концах тут ничего не решает.
///
/// Где проходит граница «внутри отрезка», решает уже спрашивающий: у
/// [`segment_crossing`] концы с допуском, у [`segments_cross`] — строгие
/// `[0, 1]`.
fn crossing_params(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<(f32, f32)> {
    let (r, s) = (b - a, d - c);
    let denominator = r.perp_dot(s);
    if denominator.abs() <= f32::EPSILON * r.length() * s.length() {
        return None;
    }
    Some((
        (c - a).perp_dot(s) / denominator,
        (c - a).perp_dot(r) / denominator,
    ))
}

/// Точка пересечения отрезков `a→b` и `c→d`, концы включительно. Параллельные
/// и совпадающие отрезки пересечения не имеют.
fn segment_crossing(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    const SLACK: f32 = 1e-4;
    let (t, u) = crossing_params(a, b, c, d)?;
    ((-SLACK..=1.0 + SLACK).contains(&t) && (-SLACK..=1.0 + SLACK).contains(&u))
        .then(|| a + (b - a) * t)
}

/// Пересекаются ли отрезки `a→b` и `c→d`, концы включительно. Параллельные и
/// совпадающие пересечения не имеют — для забора это то, что нужно: сдвиг
/// вдоль ограды ею не остановлен.
///
/// Концы строгие, без допуска [`segment_crossing`]: там точку ищут у ломаной,
/// собранной из общих узлов, и вопрос «а не прошло ли ровно через узел» стоит
/// в полный рост; здесь спрашивают, упёрся ли сдвиг в забор, и касание концом
/// на десятую долю миллиметра его не останавливает.
pub fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    crossing_params(a, b, c, d)
        .is_some_and(|(t, u)| (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
}

/// Отрезки оград в равномерной сетке — индекс «ближайшая ограда к точке» для
/// калиток (`Navmesh::open_sealed_fences`): тайл калитки лежит на заборе по
/// построению, и линейный перебор всех оград города на каждый такой тайл
/// стоил гейтам 183 мс на первом раунде (запись 26). Тот же приём, что у
/// [`StreetEdges`](super::StreetEdges) рядом и у лент мостов заливки.
pub struct FenceIndex {
    segments: Grid<(u32, u32)>,
}

impl FenceIndex {
    pub fn build(fences: &[FenceLine]) -> Self {
        let mut segments: Grid<(u32, u32)> = Grid::new(GAP_CELL);
        for (index, fence) in fences.iter().enumerate() {
            if fence.points.len() < 2 {
                continue;
            }
            for (segment, pair) in fence.points.windows(2).enumerate() {
                segments.insert_segment(pair[0], pair[1], 0.0, (index as u32, segment as u32));
            }
        }
        Self { segments }
    }

    /// Отрезки оград, чьи коробки пересекают прямоугольник `min..max`, по
    /// возрастанию `(ограда, отрезок)`, без дубликатов (отрезок ложится в
    /// несколько ячеек, прямоугольник — в несколько ячеек того же размера).
    /// Локальный refill раундов калиток растеризует в боксах проёмов только
    /// их: отрезок, чья лента доходит до тайла бокса, своей коробкой бокс
    /// задевает.
    pub fn segments_in_box(&self, min: Vec2, max: Vec2) -> Vec<(u32, u32)> {
        let mut found: Vec<(u32, u32)> = self.segments.near_each(min, max).copied().collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Ближайшая к `point` точка ограды и номер ограды — не дальше `limit`
    /// (диагональ навтайла: тайл, для которого ищется калитка, лежит на
    /// заборе по построению).
    ///
    /// Ответ тот же, что у линейного перебора оград, байт в байт: кандидаты —
    /// те же ограды (у ограды, чья ломаная ближе `limit`, отрезок-минимум
    /// заведомо в окне запроса, а отрезок в окне ближе `limit` бывает только
    /// у такой ограды); минимум по ломаной — минимум по её отрезкам, порядок
    /// `(ограда, отрезок)` воспроизводит обход ломаной для равных дистанций;
    /// равные дистанции разных оград выигрывает младший номер, как у перебора.
    pub fn nearest(&self, fences: &[FenceLine], point: Vec2, limit: f32) -> Option<(usize, Vec2)> {
        let mut candidates: Vec<(u32, u32)> = self
            .segments
            .near_each(point - limit, point + limit)
            .copied()
            .collect();
        candidates.sort_unstable();
        // минимум по ломаной каждой ограды-кандидата: строго меньшая дистанция
        // сменяет прежнюю, равная — нет (тот же порядок, что у перебора отрезков
        // ломаной, теперь в порядке (ограда, отрезок))
        let mut best: Option<(usize, Vec2, f32)> = None; // (ограда, точка, дистанция²)
        let mut run: Option<(usize, Vec2, f32)> = None;
        for (key, segment) in candidates {
            let fence = key as usize;
            match &run {
                Some((owner, _, _)) if *owner == fence => {}
                _ => {
                    if let Some(found) = run.take() {
                        best = fold_fence(best, found, point, limit);
                    }
                    run = Some((fence, Vec2::ZERO, f32::INFINITY));
                }
            }
            let points = &fences[fence].points;
            let at = closest_on_segment(
                point,
                points[segment as usize],
                points[segment as usize + 1],
            );
            let distance = point.distance_squared(at);
            if distance < run.as_ref().unwrap().2 {
                run = Some((fence, at, distance));
            }
        }
        if let Some(found) = run {
            best = fold_fence(best, found, point, limit);
        }
        best.map(|(fence, at, _)| (fence, at))
    }
}

/// Свернуть минимум по ломаной одной ограды в общий минимум: дистанция через
/// `total_cmp`, равные дистанции выигрывает младший номер ограды — как у
/// `min_by` линейного перебора в порядке оград.
fn fold_fence(
    best: Option<(usize, Vec2, f32)>,
    found: (usize, Vec2, f32),
    point: Vec2,
    limit: f32,
) -> Option<(usize, Vec2, f32)> {
    let (fence, at, distance) = found;
    if at.distance(point) > limit {
        return best;
    }
    let distance = distance.sqrt();
    match best {
        Some((owner, _, best_distance)) => match best_distance.total_cmp(&distance) {
            std::cmp::Ordering::Greater => Some((fence, at, distance)),
            std::cmp::Ordering::Equal if owner > fence => Some((fence, at, distance)),
            _ => best,
        },
        None => Some((fence, at, distance)),
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
///
/// **И минус то, что накрыто мостом** ([`BridgeDecks`]) — вырез только
/// отрисовочный: проходимость ограда под пролётом сохраняет, `fence_gaps`
/// мостами не режет вовсе («пролёт идёт над оградой, а не сквозь неё»).
/// Отсюда и второй аргумент рядом с проёмами: это две разные причины не
/// рисовать забор, и только первая из них — ещё и дырка в сетке.
pub fn fence_pieces(fence: &FenceLine, gaps: &[FenceGap], decks: &BridgeDecks) -> Vec<Vec<Vec2>> {
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
        // каждым кругом, плюс то, что закрыто настилом
        let mut covered: Vec<(f32, f32)> = gaps
            .iter()
            .filter_map(|gap| segment_in_circle(a, b, gap.at, gap.reach))
            .collect();
        decks.cover(a, b, &mut covered);
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
pub(super) fn segment_in_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<(f32, f32)> {
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

#[cfg(test)]
mod tests;
