//! Порядок записи домов в меш экструзии.
//!
//! Слой рисуется painter's algorithm'ом: глубины в меше нет, и ближний дом
//! обязан лечь в буфер позже дальнего. Долго «дальше» считалось одним числом
//! на дом — глубиной центра пятна вдоль подъёма. На таком ключе ломается
//! любая пара, где одна часть дома стоит перед соседом, а другая за ним: у
//! Г-образной пятиэтажки центр утянут длинным крылом на юго-запад, а в стык с
//! девятиэтажкой смотрит северо-восточное — и стена пятиэтажки ложилась
//! поверх соседской кровли. Никакой другой центр (верхняя точка, угол пятна,
//! глубина минус подъём) этого не чинит: одно число на дом такую пару не
//! выражает в принципе.
//!
//! Поэтому порядок строится **по парам**. Подъём — постоянный сдвиг вдоль
//! `Lean::dir()`, так что в системе «`u` поперёк подъёма, `v` вдоль» пятно
//! дома при каждом `u` — это набор отрезков по `v`, и нарисован дом от начала
//! отрезка до его конца плюс подъём. Камера смотрит с юго-запада сверху: в
//! общей точке экрана ближе тот фрагмент, который поднят выше, а поднят он
//! ровно на `v` минус ближайшая кромка пятна снизу — внутри пятна ноль, за
//! его верхней кромкой растёт до подъёма. Значит достаточно взять несколько
//! `u` внутри пересечения, на каждом сравнить эти две высоты и разложить
//! полученный граф топологической сортировкой.
//!
//! Дворы (дыры контура) в пятне не учитываются: двор для дома — та же земля,
//! высота над ней ноль, и меняет он только дальнюю внутреннюю стену, которая
//! ничего, кроме собственного двора, не кроет.
//!
//! Это по-прежнему порядок домов, а не глубина пикселя: в паре, где каждый
//! кроет другого на своём куске (сцепленные Г-образные дома), кто-то всё
//! равно ляжет не так — выбирается тот порядок, при котором таких пикселей
//! меньше по площади. Настоящее лечение — `z` по высоте
//! вершины и depth-тест; до него пары чинят типовой город.

use std::collections::HashMap;
use std::time::Instant;

use bevy::prelude::*;

use super::{BuildingHeightMode, Lean, building_center, extrusion_lift};
use crate::map::osm::PolyArea;
use crate::map::osm::model::ring_bounds;

/// Шаг выборки поперёк подъёма, м. Меньше метра смысла не имеет: столько же
/// стоит и вся неточность контура, а флигель уже метра шириной виден.
const SAMPLE_STEP: f32 = 1.0;
/// Потолок числа выборок на пару — чтобы длинный общий фронт (промышленный
/// корпус вдоль такого же) не стоил сотни проб.
const MAX_SAMPLES: usize = 16;
/// Запас над карнизом, м: конёк и оборудование кровли уезжают выше стены
/// (`ROOF_RISE_MAX` — 5 настоящих метров, то есть 1.75 нарисованных). Запас
/// делает отношение чуть жаднее, и это верная сторона ошибки: лишнее ребро в
/// графе упорядочивает пару, которая иначе досталась бы ключу по центру.
const RIDGE_MARGIN: f32 = 2.0;
/// Сторона ячейки индекса, м. Порядок городского дома с его подъёмом; мельче
/// — дом попадает в десяток ячеек, крупнее — в ячейке набирается лишняя сотня
/// пар на проверку.
const CELL: f32 = 48.0;

/// Порядок записи домов в меш: дальний первым, ближний последним.
pub(super) fn draw_order(buildings: &[PolyArea], lean: Lean) -> Vec<usize> {
    let started = Instant::now();
    let drawn: Vec<Drawn> = buildings.iter().map(|b| Drawn::of(b, lean)).collect();

    // «кого этот дом кроет»: в обходе накрытый выписывается раньше
    let mut covered: Vec<Vec<usize>> = vec![Vec::new(); buildings.len()];
    let mut scratch = Scratch::default();
    let pairs = candidate_pairs(&drawn);
    let mut edges = 0usize;
    for &(a, b) in &pairs {
        match upper(&drawn[a], &drawn[b], &mut scratch) {
            Some(Upper::A) => covered[a].push(b),
            Some(Upper::B) => covered[b].push(a),
            None => continue,
        }
        edges += 1;
    }

    // база — прежний ключ по центру пятна: он решает всё, что пары не
    // связали, а это большинство домов — они друг друга не касаются
    let mut base: Vec<usize> = (0..buildings.len()).collect();
    let depth = |index: usize| lean.depth(building_center(&buildings[index]));
    base.sort_by(|&a, &b| depth(b).total_cmp(&depth(a)));

    let order = topological(&base, &covered);
    // цена порядка — рядом с ценой самой сборки меша: слой пересобирается ещё
    // и на смене ступени зума, то есть посреди кадра
    debug!(
        "building order: {} buildings, {} pairs, {edges} covers in {:?}",
        buildings.len(),
        pairs.len(),
        started.elapsed()
    );
    order
}

/// Обход в глубину: дом выписывается после всех, кого он кроет. Итеративный,
/// потому что цепочка домов вдоль улицы длиной в город — обычное дело, а
/// рекурсия на ней кладёт стек. Обратное ребро (цикл) просто игнорируется:
/// порядок остаётся полным, а в цикле кто-то и так ляжет не так.
fn topological(base: &[usize], covered: &[Vec<usize>]) -> Vec<usize> {
    const NEW: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;

    let mut state = vec![NEW; covered.len()];
    let mut order = Vec::with_capacity(covered.len());
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &root in base {
        if state[root] != NEW {
            continue;
        }
        state[root] = OPEN;
        stack.push((root, 0));
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let step = *next;
            *next += 1;
            match covered[node].get(step) {
                Some(&child) => {
                    if state[child] == NEW {
                        state[child] = OPEN;
                        stack.push((child, 0));
                    }
                }
                None => {
                    state[node] = DONE;
                    order.push(node);
                    stack.pop();
                }
            }
        }
    }
    order
}

/// Дом в системе координат подъёма: `x` — поперёк подъёма, `y` — вдоль него.
/// Подъём в ней — прибавка к `y`, поэтому пятно и всё нарисованное над ним
/// живут в одной полосе.
struct Drawn {
    /// Контур в координатах подъёма.
    ring: Vec<Vec2>,
    /// Длина подъёма вдоль `y` — с запасом на конёк.
    lift: f32,
    min: Vec2,
    max: Vec2,
}

impl Drawn {
    fn of(building: &PolyArea, lean: Lean) -> Self {
        let along = lean.dir();
        let across = along.perp();
        let ring: Vec<Vec2> = building
            .outer
            .iter()
            .map(|point| Vec2::new(across.dot(*point), along.dot(*point)))
            .collect();
        let (min, max) = ring_bounds(&ring);
        // подъём параллелен `dir()`, так что его длина — это и есть прибавка
        // к `y`; режим здесь всегда экструзия, других этот слой не строит
        let lift = extrusion_lift(building, BuildingHeightMode::Extrusion).length() + RIDGE_MARGIN;
        Self {
            ring,
            lift,
            min,
            max,
        }
    }
}

/// Буферы на всю сборку: пар — десятки тысяч, и своя пара `Vec` на каждую
/// стоила бы больше, чем сама проверка.
#[derive(Default)]
struct Scratch {
    crossings: Vec<f32>,
    spans_a: Vec<(f32, f32)>,
    spans_b: Vec<(f32, f32)>,
    events: Vec<f32>,
}

enum Upper {
    A,
    B,
}

/// Кто в паре ложится поверх. `None` — на экране они не встречаются или
/// встречаются вничью (общая кромка сросшихся домов, общая земля дворов).
fn upper(a: &Drawn, b: &Drawn, scratch: &mut Scratch) -> Option<Upper> {
    let from = a.min.x.max(b.min.x);
    let to = a.max.x.min(b.max.x);
    if to <= from {
        return None;
    }
    // полосы отрисовки вдоль подъёма тоже обязаны пересечься
    if a.max.y + a.lift <= b.min.y || b.max.y + b.lift <= a.min.y {
        return None;
    }

    let steps = (((to - from) / SAMPLE_STEP).ceil().max(1.0) as usize).min(MAX_SAMPLES);
    let mut a_over = 0.0f32;
    let mut b_over = 0.0f32;
    for step in 0..steps {
        // по серединам долей, а не по краям: край пересечения — это край
        // контура, и проба там садится ровно на его вершину
        let u = from + (to - from) * (step as f32 + 0.5) / steps as f32;
        spans_at(&a.ring, u, &mut scratch.crossings, &mut scratch.spans_a);
        spans_at(&b.ring, u, &mut scratch.crossings, &mut scratch.spans_b);
        if scratch.spans_a.is_empty() || scratch.spans_b.is_empty() {
            continue;
        }

        // между соседними кромками обе высоты растут с одним наклоном, так
        // что их разность там постоянна: хватит одной пробы на промежуток
        scratch.events.clear();
        for spans in [&scratch.spans_a, &scratch.spans_b] {
            for &(low, high) in spans.iter() {
                scratch.events.push(low);
                scratch.events.push(high);
            }
        }
        for (spans, lift) in [(&scratch.spans_a, a.lift), (&scratch.spans_b, b.lift)] {
            for &(_, high) in spans.iter() {
                scratch.events.push(high + lift);
            }
        }
        scratch.events.sort_by(f32::total_cmp);
        for pair in scratch.events.windows(2) {
            let v = (pair[0] + pair[1]) * 0.5;
            if v <= pair[0] {
                continue;
            }
            let (Some(a_height), Some(b_height)) = (
                height_at(&scratch.spans_a, v, a.lift),
                height_at(&scratch.spans_b, v, b.lift),
            ) else {
                continue;
            };
            // вес пробы — длина промежутка: пробы равномерны по `u`, значит
            // сумма весов и есть площадь, на которой один кроет другого
            let weight = pair[1] - pair[0];
            if a_height > b_height {
                a_over += weight;
            } else if b_height > a_height {
                b_over += weight;
            }
        }
    }

    // пробы спорят там, где дома сцеплены: у каждого свой кусок, который он
    // кроет. Выигрывает большая площадь — тот порядок, при котором неверно
    // нарисованных пикселей меньше
    if a_over > b_over {
        Some(Upper::A)
    } else if b_over > a_over {
        Some(Upper::B)
    } else {
        None
    }
}

/// Отрезки пятна на линии `u`, по возрастанию: пересечения контура с линией,
/// сложенные попарно (правило чётности).
fn spans_at(ring: &[Vec2], u: f32, crossings: &mut Vec<f32>, spans: &mut Vec<(f32, f32)>) {
    crossings.clear();
    spans.clear();
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        // полуоткрытое сравнение: вертикальное ребро отсекается им же, так
        // что делить на ноль не на чем, а вершина считается один раз
        if (a.x <= u) == (b.x <= u) {
            continue;
        }
        crossings.push(a.y + (b.y - a.y) * (u - a.x) / (b.x - a.x));
    }
    crossings.sort_by(|a, b| a.total_cmp(b));
    for pair in crossings.chunks_exact(2) {
        spans.push((pair[0], pair[1]));
    }
}

/// На сколько поднят фрагмент дома в точке `v`: внутри пятна ноль, выше его
/// кромки — расстояние до неё. `None` — дом сюда не достаёт (ниже пятна или
/// выше подъёма).
fn height_at(spans: &[(f32, f32)], v: f32, lift: f32) -> Option<f32> {
    let mut top = f32::NEG_INFINITY;
    for &(low, high) in spans {
        if low > v {
            break;
        }
        top = high.min(v);
    }
    // без пятна снизу разность бесконечна и проверку подъёма не проходит
    let height = v - top;
    (height <= lift).then_some(height)
}

/// Пары домов, чьи нарисованные пятна могут пересечься, — по равномерной
/// сетке. Порядок пар фиксирован сортировкой: обход `HashMap` в него попасть
/// не должен, иначе меш поедет от запуска к запуску.
fn candidate_pairs(drawn: &[Drawn]) -> Vec<(usize, usize)> {
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (index, item) in drawn.iter().enumerate() {
        // вырожденный контур (меньше трёх точек) даёт бесконечные границы —
        // такому в сетке делать нечего
        if !item.min.is_finite() || !item.max.is_finite() {
            continue;
        }
        let from = (item.min / CELL).floor();
        let to = (Vec2::new(item.max.x, item.max.y + item.lift) / CELL).floor();
        for x in from.x as i32..=to.x as i32 {
            for y in from.y as i32..=to.y as i32 {
                cells.entry((x, y)).or_default().push(index);
            }
        }
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for bucket in cells.values() {
        for (offset, &a) in bucket.iter().enumerate() {
            for &b in &bucket[offset + 1..] {
                pairs.push((a, b));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}
