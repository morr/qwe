//! Стены и кровля гаражного прогона — **по куску за раз**: ворота стоят под
//! своим же швом кровли, торец ворот не несёт, кривой обрезок гребёнки не
//! получает. Клетки здесь считаются шагом куска, а не общей меркой
//! [`crate::map::buildings::garages::BAY`].

use bevy::prelude::*;

use super::silhouette_edges;
use super::walls::{WallSpan, wall_cells};
use crate::map::buildings::arches::WallCells;
use crate::map::buildings::garages::{FACADE_COS, GarageRect, GarageRun};
use crate::map::buildings::material::{RoofKind, RoofLook, WallKind, WallLook, run_kind};
use crate::map::meshing::{PARAPET_CELLS, Roof, WallFrame, WallMark};
use crate::map::osm::PolyArea;
use crate::map::osm::model::distance_to_segment;

/// Видимые стены гаражного контура — **по куску за раз**, а не по целому
/// кольцу.
///
/// Разрез идёт хордой, и она рассекает ребро контура: западная стена буквы Г
/// длиной в 52 м лежит на двух кусках сразу, по 8 и 44 м. Целой ей достаётся
/// сетка одного из них, и на второй половине ворота уезжают от гребёнки над
/// ними — та самая ошибка, ради которой стена и села на сетку куска. Кусок
/// же уже несёт своё кольцо, разрезанное там, где надо: его силуэт и есть
/// список стен, каждая со своим куском.
///
/// Хорда стеной не считается: она внутри дома, и снаружи её нет. Отличается
/// она ровно тем, что не лежит на исходном контуре.
pub(super) fn garage_walls(run: &GarageRun, outer: &[Vec2], facing: Vec2) -> Vec<(Vec2, Vec2)> {
    run.rects
        .iter()
        .flat_map(|rect| {
            silhouette_edges(&rect.ring, facing)
                .into_iter()
                .filter(|(a, b)| on_ring(outer, (*a + *b) / 2.0))
        })
        .collect()
}

/// Лежит ли точка на кольце — этим настоящая стена и отличается от хорды
/// разреза.
fn on_ring(ring: &[Vec2], point: Vec2) -> bool {
    let count = ring.len();
    (0..count).any(|at| distance_to_segment(point, ring[at], ring[(at + 1) % count]) < 1e-3)
}

/// Клетки гаражной стены — **шагом своего куска**, а не общей меркой
/// [`crate::map::buildings::garages::BAY`].
///
/// Ворота обязаны стоять под своим же швом кровли, и до сих пор это держалось
/// на совпадении: кровля считается шагом куска (`bay`), стена — шагом
/// `длина / round(длина / BAY)`. Две сетки сходятся только там, где длина
/// стены равна длине куска; у разрезанного контура стена вдвое короче, и
/// гребёнка на кровле разъезжалась с воротами под ней тем сильнее, чем дальше
/// от начала.
///
/// Шаг берётся у куска, **фаза — нет**: клеток на стене целое число и
/// кончаются они ровно на её углах, как у всякой другой стены
/// ([`super::walls::wall_frame`]).
/// Посаженная на фазу куска стена начинается и кончается посреди клетки, и
/// крайние ворота выходят обрезанными — по половине створки на каждом зубе
/// гребёнки. Длинному фасаду фаза и не нужна: его длина и есть длина куска,
/// поэтому тот же шаг от того же угла даёт те же границы.
///
/// Поперечная стена — **торец**: ворот на ней нет ([`WallMark::Solid`]), и
/// меряется она рядами, а не боксами. У ленты ряд один на всю ширину, так что
/// торец — одна клетка без единого шва внутри; у кооператива на нём видны те
/// же границы рядов, что идут по кровле.
///
/// `None` — стена не гаражная, или кусок к ней не повёрнут (косая грань
/// контура): тогда считает [`wall_cells`], как считал.
pub(super) fn garage_cells(
    building: &PolyArea,
    look: &RoofLook,
    wall: &WallLook,
    span: &WallSpan,
) -> Option<WallCells> {
    if wall.kind != WallKind::GarageDoors {
        return None;
    }
    let run = look.run.as_ref()?;
    let along = (span.b - span.a).try_normalize()?;
    let rect = run.holder((span.a + span.b) / 2.0);
    // кривой обрезок: ворот на нём не рисуется вовсе — ни целых, ни половинок
    if rect.plain {
        let cells = wall_cells(building, wall, span);
        return Some(WallCells {
            frame: cells.patch,
            ..cells
        });
    }
    let facade = rect.faces_the_drive(along);
    let (direction, pitch) = match facade {
        true => (rect.axis, rect.bay),
        false => (Vec2::new(-rect.axis.y, rect.axis.x), rect.row),
    };
    // косая грань не лежит ни вдоль, ни поперёк — её сетке неоткуда взяться
    let turn = along.dot(direction);
    if turn.abs() < FACADE_COS || pitch <= 0.0 {
        return None;
    }
    // Ячеек на стене — целое число, и кончаются они ровно на её углах. Фаза
    // куска сюда не едет намеренно: посаженная на неё стена начинается и
    // кончается посреди ячейки, и крайние ворота выходили обрезанными с обоих
    // концов — по половинке створки на каждом зубе гребёнки. Целое число, как
    // у всякой другой стены (`wall_frame`); от куска берётся **шаг**, и этого
    // достаточно: у длинного фасада его длина и есть длина куска, так что
    // гребёнка над ним идёт тем же шагом от того же угла.
    let columns = (span.length() / pitch).round().max(1.0);
    let frame = WallFrame::new(
        span.a,
        span.b,
        span.lift,
        columns,
        span.storeys,
        wall.kind.code(),
        span.seed(),
    )?;
    // балконов на гараже не бывает ни на фасаде, ни на торце; торцу вдобавок
    // не полагается ни одного проёма
    let patch = frame.marked(WallMark::Solid);
    Some(WallCells {
        frame: Some(match facade {
            true => frame.marked(WallMark::Blank),
            false => patch,
        }),
        patch: Some(patch),
        panel: span.length() / columns,
        storey: span.lift / (span.storeys + PARAPET_CELLS),
    })
}

/// Кровля куска, на котором гаража **не рисуется**: тот же профлист, каким
/// крыт гараж, в том же цвете и с той же фазой — но обычной рамкой по мировой
/// точке, без единой ячейки бокса. Так и выглядит зуб гребёнки в четыре
/// метра: кусок кровли, а не отдельный гараж поперёк соседей.
pub(super) fn plain_garage_roof(look: &RoofLook) -> Roof {
    Roof {
        axis: look.frame.axis,
        material: RoofKind::Corrugated.code(),
        seed: look.frame.seed,
    }
}

/// Рама **гаражного куска** для шейдера: номер бокса вдоль него и номер ряда
/// поперёк, оба целые на торцах ([`WallFrame::run`]).
///
/// `lift` — сдвиг, с которым кровля этого дома уже легла в меш (в плоских
/// режимах ноль, в 2.5D подъём стен): рама обязана считаться в той же
/// системе, в какой лежат вершины, иначе швы уедут от дома.
pub(super) fn garage_frame(rect: &GarageRect, seed: f32, lift: Vec2) -> Option<WallFrame> {
    WallFrame::run(
        rect.origin + lift,
        rect.axis,
        rect.bay,
        rect.row,
        run_kind(rect.block).code(),
        seed,
    )
}
