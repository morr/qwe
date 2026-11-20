//! Тень на карте: куда она уходит, насколько мягок её край и как свипы
//! ложатся в меш.
//!
//! Тени рисуют семь слоёв — дома, ограды, машины, вагоны, промзона, мосты,
//! оборудование кровель, — и три вещи у них общие. До этого модуля ни одна из
//! трёх не была записана в одном месте:
//!
//! - **длина тени** — `shadow_dir() * высота * shadow_length_scale()`,
//!   выражение из десяти мест;
//! - **правило полутени** — `direction.dot(light).max(0)`, написанное трижды:
//!   именованной функцией у зданий и дважды замыканием, у оград и у машин;
//! - **объединить свипы и обвести каймой** — восемнадцать строк, дословно
//!   повторенные у оград и у зданий, различающиеся одной константой размытия.
//!
//! Скилл `osm-map` формулирует цену прямо: «длина тени, написанная без
//! `sun_stretch()`, — заготовка бага: она будет верна на дефолте и неверна на
//! обоих концах ползунка». Здесь это перестаёт быть правилом, которое надо
//! помнить, и становится вызовом.
//!
//! **Свет остаётся процессной глобалью** (`map/sun.rs`), и это решение, а не
//! упущение. Сделать его аргументом значило бы протащить параметр сквозь
//! каждую `mesh_*` — ровно те функции, которые шов кандидата 01 сделал
//! вызываемыми из игры, из теста и из офлайн-бенча **одним и тем же вызовом**.
//! Глобаль — то, что позволяет сборке идти в потоке загрузки, где ECS нет
//! вовсе. Цена известна и записана: тест с освещённой геометрией берёт гард
//! `default_sun()` / `sun_at()` и сериализуется на мьютексе.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;

use crate::map::meshing::MeshBuilder;
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};

/// Насколько далеко тень объекта высоты `height` уходит от него, м.
///
/// Это и есть то самое место, где живёт `shadow_length_scale()` — котангенс
/// высоты солнца. Зажимают длину те, у кого есть свой предел (у домов
/// `SHADOW_LENGTH_RANGE`, у оборудования кровли — край крыши), и зажимают
/// **после** вызова.
pub fn length(height: f32) -> f32 {
    height * shadow_length_scale()
}

/// Сдвиг тени объекта высоты `height`: куда и насколько.
///
/// Тень **начинается под объектом** и вытекает из-под него, так что это
/// смещение дальнего конца свипа, а не место, куда переносится силуэт.
/// Перенос силуэта — та ошибка, которую по очереди совершили машины, забор и
/// мост: при солнце 15° двухметровый забор «уезжал» на 7.4 м и читался второй
/// оградой рядом.
pub fn offset(height: f32) -> Vec2 {
    shadow_dir() * length(height)
}

/// Доля ширины полутени на вершине с этим направлением наружу.
///
/// Тень встречает предмет, который её отбрасывает, **жёстко** — у стены
/// полутени нет, — и размывается по мере удаления. В объединённой фигуре это
/// читается локально: тело всегда лежит со стороны света от контактного ребра,
/// так что проекция направления наружу на свет даёт ноль у контакта, единицу
/// на дальнем краю и рост от нуля к единице вдоль боковой стороны.
///
/// Без этого правила кайма шла и по контактному контуру, и на каждом выпуклом
/// углу ступенчатого фасада оставляла мягкую тёмную кляксу **на освещённой
/// земле** — дом читался обведённым.
pub fn penumbra(direction: Vec2) -> f32 {
    direction.dot(shadow_dir()).max(0.0)
}

/// Свипы теней — в меш: объединить и обвести мягким краем.
///
/// **Объединение** (`i_overlay`, NonZero) нужно потому, что слой
/// полупрозрачный: две наложившиеся тени дали бы пятно двойной темноты. У
/// дома это соседние крылья одного здания и соседние дома, у ограды — тень
/// звена и тень излома, а в частном секторе ещё и заборы соседних участков,
/// лежащие друг на друге по всей длине при низком солнце. На городском входе
/// объединение идёт **тайлами** ([`union_of_dense`] — цена одного прохода
/// растёт быстрее контуров, и сто тысяч контуров одним вызовом — секунды;
/// плотная клетка дробится на квадранты — запись 25).
///
/// **Кайма** идёт наружу от внешнего кольца и внутрь просвета от каждой дырки:
/// сторону она выбирает по площади самого кольца, так что дырке нужно
/// `outside: false`, иначе кайма ляжет на уже залитое тело и обведёт просвет
/// двойной темнотой вместо растушёвки. Каймы соседних фигур могут наложиться,
/// но обе сходят в ноль, и удвоение выходит слабее самой тени.
///
/// `blur` — ширина этого края, и она **не физическая полутень**: угловой
/// размер солнца дал бы сантиметры. Это то, чем край тени размыт на снимке —
/// разрешением кадра и светом неба, — поэтому число выбирается видом. У дома
/// метр, у забора и машины треть, **потому что и тень там втрое-вдесятеро
/// короче**.
pub fn push_union(builder: &mut MeshBuilder, contours: &[Vec<[f32; 2]>], blur: f32) {
    use crate::map::parallel::in_parallel;

    let shapes = union_of_dense(contours);
    let color = SHADOW_COLOR.to_linear();
    let fade = LinearRgba {
        alpha: 0.0,
        ..color
    };
    // фигура от фигуры не зависит: заливка и каймы собираются в свои билдеры
    // по ядрам и дописываются в порядке фигур — последовательность вершин та
    // же, что у серийного прохода. На городском входе это вторая половина
    // фазы: earcut и каймы по 13 тыс. фигур (Париж) стоят столько же, сколько
    // само объединение
    let parts: Vec<MeshBuilder> = in_parallel(&shapes, |shape| {
        let mut part = MeshBuilder::default();
        let mut rings = shape.iter().map(|contour| {
            contour
                .iter()
                .map(|point| Vec2::from_array(*point))
                .collect::<Vec<Vec2>>()
        });
        let Some(outer) = rings.next() else {
            return part;
        };
        let holes: Vec<Vec<Vec2>> = rings.collect();
        part.push_polygon(&outer, &holes, color);
        part.push_inset_band_tapered(&outer, blur, true, penumbra, color, fade);
        for hole in &holes {
            part.push_inset_band_tapered(hole, blur, false, penumbra, color, fade);
        }
        part
    });
    for part in &parts {
        builder.append(part);
    }
}

/// Ниже порога — однопроходный union, как раньше: у тестов и витрин вход мал,
/// тайлам там делиться не о что, а порядок вершин у путей разный.
const TILED_MIN: usize = 512;

/// Ячейка тайлового union и тайловой разности, м. Контуры группируются по
/// клетке **первой точки** (клетка — та, кто тень отбрасывает), каждая клетка
/// объединяется в своём потоке, результаты клеток — в одном финальном union.
/// Тень длиннее клетки не мешает: налегания соседних клеток снимает как раз
/// финальный union, а цена его — фигуры клеток, которых на порядки меньше,
/// чем контуров на входе.
const TILE: f32 = 256.0;

/// Порог плотной группы, контуров. Группа с бо́льшим числом делится на
/// квадранты по первой точке — тот же приём, каким весь вход делится на
/// клетки ([`TILE`]), и порог тот же, что у [`TILED_MIN`]: группировка по
/// клеткам существует, потому что цена одного прохода растёт быстрее числа
/// контуров; клетка с контуров больше порога больна той же болезнью, а
/// параллелизм клеток её не лечит — в своём потоке она одна (запись 25:
/// 2053 свипа — 77 мс, 3656 — 1210 мс на ступени 1.30 Парижа, обе клетки —
/// частный сектор).
const DENSE_MIN: usize = 512;

/// Гранулярность решётки, на которой дробление останавливается, м: глубже
/// группы и так малы, а первые точки, всё лёгшие в один квадрант, —
/// вырожденный вход, не стоящий своей решётки.
const DENSE_MIN_STEP: f32 = 32.0;

/// Делит плотную группу на квадранты по первой точке.
///
/// Возвращает листья: каждый несёт не больше [`DENSE_MIN`] элементов или
/// стоит на решётке [`DENSE_MIN_STEP`]. Очередь — FIFO, квадранты внутри
/// группы — по возрастанию ключа решётки, так что порядок листьев
/// детерминирован. Группа, чьи первые точки легли в один квадрант, не
/// делится вовсе — вместо неё мельче становится шаг решётки, так что
/// деление всегда прогрессирует.
fn split_dense<T>(items: Vec<T>, point_of: impl Fn(&T) -> Vec2) -> Vec<Vec<T>> {
    use std::collections::VecDeque;

    use bevy::platform::collections::HashMap;

    let mut leaves = Vec::new();
    if items.len() <= DENSE_MIN {
        leaves.push(items);
        return leaves;
    }
    let mut queue = VecDeque::from([(items, TILE / 2.0)]);
    while let Some((group, step)) = queue.pop_front() {
        if group.len() <= DENSE_MIN || step < DENSE_MIN_STEP {
            leaves.push(group);
            continue;
        }
        let mut by_quad: HashMap<IVec2, Vec<T>> = HashMap::new();
        for item in group {
            let quad = (point_of(&item) / step).floor().as_ivec2();
            by_quad.entry(quad).or_default().push(item);
        }
        let mut quads: Vec<(IVec2, Vec<T>)> = by_quad.into_iter().collect();
        quads.sort_unstable_by_key(|&(quad, _)| (quad.x, quad.y));
        for (_, group) in quads {
            if group.len() > DENSE_MIN {
                queue.push_back((group, step / 2.0));
            } else {
                leaves.push(group);
            }
        }
    }
    leaves
}

/// Объединённые свипы: один проход на малом входе, тайлами на городском.
///
/// Группировка детерминирована дважды: порядок контуров внутри клетки —
/// входной, клетки обходятся по возрастанию ключа, а не по порядку хеша.
/// Геометрия union от группировки не зависит (ассоциативность), порядок
/// вершин результата от однопроходного отличается — контракт картинки, не
/// байтов. Слияние **результатов** клеток в кварталы (4×4) замерялось против
/// этого и выигрыша не показало: у объединённых фигур цена ближе к линейной,
/// и почти всё и так уходит в параллельные клеточные union — сверхлинейна
/// цена **сырых** контуров одной группы, и она лечится дроблением плотной
/// клетки ([`union_of_dense`], запись 25). Объединение по клеткам — общий
/// примитив: полимешу (`navigation/polymesh`) — union надутых препятствий
/// перед CDT, эта функция.
pub(crate) fn union_of(contours: &[Vec<[f32; 2]>]) -> Vec<Vec<Vec<[f32; 2]>>> {
    union_tiled(contours, false)
}

/// Тот же тайловый union, но плотные клетки дробятся на квадранты
/// ([`split_dense`]) — теневой вариант.
///
/// Его результат читает картинка, и перестановка вершин там — тот же класс
/// «перестановки тайловых путей», что у самой группировки по клеткам.
/// Полимеш его не зовёт: его union — вход CDT, и перестановка вершин
/// доезжает до навмеш-геометрии; когда дойдёт руки ускорить его тем же
/// приёмом, сверяться с базой probe-ом бит-в-бит.
pub(crate) fn union_of_dense(contours: &[Vec<[f32; 2]>]) -> Vec<Vec<Vec<[f32; 2]>>> {
    union_tiled(contours, true)
}

fn union_tiled(contours: &[Vec<[f32; 2]>], dense: bool) -> Vec<Vec<Vec<[f32; 2]>>> {
    if contours.len() <= TILED_MIN {
        return contours.simplify_shape(FillRule::NonZero);
    }
    use bevy::platform::collections::HashMap;

    use crate::map::parallel::in_parallel;

    let mut by_cell: HashMap<IVec2, Vec<Vec<[f32; 2]>>> = HashMap::new();
    for contour in contours {
        let cell = (Vec2::from_array(contour[0]) / TILE).floor().as_ivec2();
        by_cell.entry(cell).or_default().push(contour.clone());
    }
    let mut cells: Vec<(IVec2, Vec<Vec<[f32; 2]>>)> = by_cell.into_iter().collect();
    cells.sort_unstable_by_key(|&(cell, _)| (cell.x, cell.y));

    // клетки — по одной группе на клетку; плотные клетки в теневом варианте
    // едут квадрантами, и все группы — в один общий параллельный проход
    let groups: Vec<Vec<Vec<[f32; 2]>>> = cells
        .into_iter()
        .flat_map(|(_, cell_contours)| {
            if dense {
                split_dense(cell_contours, |contour| Vec2::from_array(contour[0]))
            } else {
                vec![cell_contours]
            }
        })
        .collect();
    let tiled: Vec<Vec<Vec<[f32; 2]>>> = in_parallel(&groups, |group| {
        group.simplify_shape(FillRule::NonZero)
    })
    .into_iter()
    .flatten()
    .collect();
    tiled.simplify_shape(FillRule::NonZero)
}

/// Вычитание `clips` из `subjects` — тем же разложением, что union.
///
/// Subjects — плоские контуры без дыр (свипы); фигурам с дырами —
/// [`difference_of_shapes`].
/// Разность, стоящая **до** union, в тайлы `push_union` не попадает: у
/// заборов (Париж) один `Difference` по всем свипам города — 48 тыс.
/// контуров — стоит 2.1 с из 2.2 стадии теней, дороже самого union.
/// Тождество `(A−B)∪(C−B) = (A∪C)−B` режет её теми же клетками: клетка
/// делает `(⋃клетки) − клипы` — разность с теми клипами, чья рамка задевает
/// рамку её свипов (разность с непересекающейся капсулой тождественна), а
/// без задетых клипов это просто union клетки, как у клеток [`union_of`].
/// Пропускать контуры клетки без слияния нельзя: налегания свипов, которые
/// раньше снимал один большой `Difference`, переехали бы в union и съели
/// выигрыш. Рамки заведомо шире геометрии (свип вытекает из своей клетки),
/// ложный «задел» только запускает лишний оверлей, а не портит результат.
/// Слитые результаты клеток едут во вход union как раньше. Плотная клетка
/// делится на квадранты ([`split_dense`]) — та же сверхлинейность сырых
/// контуров, что у union, и клипы фильтруются по рамке листа, уже рамки
/// клетки: разность с непересекающимся клипом тождественна, ложный задел
/// столько же стоит, сколько стоял. Малый вход (тесты, витрины) — прежний
/// однопроходный путь, как у [`union_of`].
pub(crate) fn difference_of(
    subjects: &[Vec<[f32; 2]>],
    clips: &[Vec<[f32; 2]>],
) -> Vec<Vec<[f32; 2]>> {
    if clips.is_empty() {
        return subjects.to_vec();
    }
    if subjects.len() <= TILED_MIN {
        let clips = clips.to_vec();
        return subjects
            .to_vec()
            .overlay(&clips, OverlayRule::Difference, FillRule::NonZero)
            .into_iter()
            .flatten()
            .collect();
    }
    use bevy::platform::collections::HashMap;

    use crate::map::parallel::in_parallel;

    let clip_boxes: Vec<[f32; 4]> = clips.iter().map(|ring| ring_box(ring)).collect();
    // клетка свипов и рамка их общей геометрии: накапливается по ходу группировки
    let mut by_cell: HashMap<IVec2, (Vec<usize>, [f32; 4])> = HashMap::new();
    for (index, contour) in subjects.iter().enumerate() {
        let cell = (Vec2::from_array(contour[0]) / TILE).floor().as_ivec2();
        let (indices, bounds) = by_cell.entry(cell).or_insert_with(|| {
            (
                Vec::new(),
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
            )
        });
        indices.push(index);
        for &[x, y] in contour {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        }
    }
    let mut cells: Vec<(IVec2, Vec<usize>, [f32; 4])> = by_cell
        .into_iter()
        .map(|(cell, (indices, bounds))| (cell, indices, bounds))
        .collect();
    cells.sort_unstable_by_key(|&(cell, _, _)| (cell.x, cell.y));

    // клетка — одна группа с рамкой всей клетки; плотная клетка едет
    // квадрантами, каждый со своей рамкой — фильтр клипов по ней уже, и
    // разности с непересекающимися клипами меньше
    let mut groups: Vec<(Vec<usize>, [f32; 4])> = Vec::new();
    for (_, indices, bounds) in cells {
        if indices.len() <= DENSE_MIN {
            groups.push((indices, bounds));
            continue;
        }
        for leaf in split_dense(indices, |&i| Vec2::from_array(subjects[i][0])) {
            let mut leaf_bounds = [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ];
            for &i in &leaf {
                for &[x, y] in &subjects[i] {
                    leaf_bounds[0] = leaf_bounds[0].min(x);
                    leaf_bounds[1] = leaf_bounds[1].min(y);
                    leaf_bounds[2] = leaf_bounds[2].max(x);
                    leaf_bounds[3] = leaf_bounds[3].max(y);
                }
            }
            groups.push((leaf, leaf_bounds));
        }
    }

    let tiled: Vec<Vec<Vec<[f32; 2]>>> = in_parallel(&groups, |(indices, bounds)| {
        let subject: Vec<Vec<[f32; 2]>> = indices.iter().map(|&i| subjects[i].clone()).collect();
        let clips: Vec<Vec<[f32; 2]>> = clips
            .iter()
            .zip(&clip_boxes)
            .filter(|&(_, &box_bounds)| boxes_touch(box_bounds, *bounds))
            .map(|(ring, _)| ring.clone())
            .collect();
        if clips.is_empty() {
            return subject
                .simplify_shape(FillRule::NonZero)
                .into_iter()
                .flatten()
                .collect();
        }
        subject
            .overlay(&clips, OverlayRule::Difference, FillRule::NonZero)
            .into_iter()
            .flatten()
            .collect()
    });
    tiled.into_iter().flatten().collect()
}

/// Вычитание `clips` из `subjects`-**фигур** (внешнее кольцо, за ним дыры) —
/// то же разложение по клеткам, что у [`difference_of`], но группировка по
/// клетке — целой фигурой: дыра обязана ехать в клетке своего наружного
/// кольца. Одинокая дыра в чужой клетке под NonZero — заполненная область,
/// то есть препятствие там, где был вырез (остров в реке — дыра внешнего
/// кольца воды, и в клетке самой реки его не было бы). У [`difference_of`]
/// такой проблемы нет: его subjects — плоские свипы без дыр.
///
/// Возвращает фигуры: результаты клеток — своими фигурами (оверлей и
/// `simplify_shape` группируют сами), порядок клеток — по возрастанию ключа.
pub(crate) fn difference_of_shapes(
    subjects: &[Vec<Vec<[f32; 2]>>],
    clips: &[Vec<[f32; 2]>],
) -> Vec<Vec<Vec<[f32; 2]>>> {
    if clips.is_empty() {
        return subjects.to_vec();
    }
    if subjects.len() <= TILED_MIN {
        let clips = clips.to_vec();
        let flat: Vec<Vec<[f32; 2]>> = subjects.iter().flatten().cloned().collect();
        return flat.overlay(&clips, OverlayRule::Difference, FillRule::NonZero);
    }
    use bevy::platform::collections::HashMap;

    use crate::map::parallel::in_parallel;

    let clip_boxes: Vec<[f32; 4]> = clips.iter().map(|ring| ring_box(ring)).collect();
    // клетка фигуры и рамка всей её геометрии: рамка — по всем контурам
    // фигуры, наружным и дырам, — вырез может задевать только дыру
    let mut by_cell: HashMap<IVec2, (Vec<usize>, [f32; 4])> = HashMap::new();
    for (index, shape) in subjects.iter().enumerate() {
        let cell = (Vec2::from_array(shape[0][0]) / TILE).floor().as_ivec2();
        let (indices, bounds) = by_cell.entry(cell).or_insert_with(|| {
            (
                Vec::new(),
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
            )
        });
        indices.push(index);
        for contour in shape {
            for &[x, y] in contour {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
        }
    }
    let mut cells: Vec<(IVec2, Vec<usize>, [f32; 4])> = by_cell
        .into_iter()
        .map(|(cell, (indices, bounds))| (cell, indices, bounds))
        .collect();
    cells.sort_unstable_by_key(|&(cell, _, _)| (cell.x, cell.y));

    let tiled: Vec<Vec<Vec<Vec<[f32; 2]>>>> = in_parallel(&cells, |(_, indices, bounds)| {
        let subject: Vec<Vec<[f32; 2]>> = indices
            .iter()
            .flat_map(|&i| subjects[i].iter().cloned())
            .collect();
        let clips: Vec<Vec<[f32; 2]>> = clips
            .iter()
            .zip(&clip_boxes)
            .filter(|&(_, &box_bounds)| boxes_touch(box_bounds, *bounds))
            .map(|(ring, _)| ring.clone())
            .collect();
        if clips.is_empty() {
            subject.simplify_shape(FillRule::NonZero)
        } else {
            subject.overlay(&clips, OverlayRule::Difference, FillRule::NonZero)
        }
    });
    tiled.into_iter().flatten().collect()
}

/// Рамка контура: `[min_x, min_y, max_x, max_y]`.
fn ring_box(ring: &[[f32; 2]]) -> [f32; 4] {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for &[x, y] in ring {
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    bounds
}

/// Пересекаются ли рамки — прикосновение тоже «да»: лишний оверлей дешевле
/// пропущенного выреза.
fn boxes_touch(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

#[cfg(test)]
mod tests;
