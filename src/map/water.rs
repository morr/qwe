//! Вода карты: площадная ([`mesh_water_areas`], слой `water`) и лента открытых
//! русел ([`mesh_water_lines`], слой `waterways`) — отдельным слоем над дорогами,
//! но под мостами, и **с вырезом там, где русло лежит внутри площадной воды**.
//!
//! Русло в OSM — осевая линия, и она не знает о полигоне реки, в который
//! впадает: осевая Упы идёт внутри своего же `riverbank` на всём протяжении, а
//! ручей доходит до пруда и продолжается в нём до узла на осевой. Внутри
//! полигона лента невидима (цвет и фактура у слоёв общие), зато поперёк его
//! отмели она ложилась ровным прямоугольником глубокой воды. Поэтому ось
//! режется по контурам площадной воды, а у каждого отрезанного конца лента
//! заходит за берег ровно на ширину отмели ([`WATER_SHORE_WIDTH`]) — и её
//! собственная отмель (кромки ленты, шейдер `surface.wgsl`) гаснет на этом
//! заходе так же, как гаснет отмель площадной воды вглубь от берега. На стыке у
//! кромки ленты и у воды рядом на одной глубине один и тот же цвет, и отмель
//! заворачивает из реки в русло без шва.
//!
//! Здесь же и сама площадная вода ([`mesh_water_areas`]): её отмель — поле
//! расстояний до берега по всем полигонам сразу, а не кайма каждого по
//! отдельности.
//!
//! Сетку это не трогает: навмеш глушит и полигон, и полосу русла целиком
//! (`Navmesh::fill_from_mapdata`), и то, что лента внутри полигона больше не
//! рисуется, проходимости не меняет.

use bevy::prelude::*;

use crate::map::area_cut::AreaIndex;
use crate::map::meshing::{Break, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin};
use crate::map::osm::model::{point_in_polygon, signed_ring_area};
use crate::map::osm::{PolyArea, WaterLine, water_line_caps};
use crate::map::roads::{self, RoadSmoothing};

/// Цвет глубокой воды — площадной и ленты русла.
pub const WATER_COLOR: Color = Color::srgb(0.655, 0.804, 0.910);
/// Цвет отмели на самом берегу — у площадной воды и у кромок ленты русла
/// (`surface.wgsl`) один: отмель заворачивает из реки в русло, и два разных
/// цвета дали бы шов ровно на устье.
pub const WATER_SHORE_COLOR: Color = Color::srgb(0.78, 0.885, 0.945);
/// Глубина отмели, м: на таком расстоянии от берега цвет доходит до
/// `WATER_COLOR`. Шесть: на снимке отмель у берега шире, чем кажется с земли,
/// и трёхметровая читалась просто кантом полигона, а не мелью. Та же ширина —
/// заход ленты русла за берег площадной воды ([`mesh_water_lines`]): на нём
/// кромки ленты гаснут вместе с отмелью берега.
pub const WATER_SHORE_WIDTH: f32 = 6.0;

/// Шаг поля отмели, м: столько между двумя соседними офсетами берега. Внутри
/// полосы цвет тянется градиентом, так что шаг решает не ступеньку, а то,
/// насколько точно полоса повторяет расстояние до берега у изгиба.
const SHOAL_STEP: f32 = 0.5;

/// Дуга офсета на выступе берега: длина хорды к радиусу (`LineJoin::Round`
/// у `i_overlay`). На радиусе в шесть метров — хорда в полтора.
const SHOAL_ARC: f32 = 0.25;

/// Кольца одной фигуры `i_overlay`: внешнее первым, дальше дырки.
type Shape = Vec<Vec<[f32; 2]>>;

/// Та же фигура в `Vec2`: внешнее кольцо и дырки ([`rings`]).
type Rings = (Vec<Vec2>, Vec<Vec<Vec2>>);

/// Площадная вода одним мешем: заливка и **отмель как поле расстояний до
/// берега** — цвет в точке зависит только от того, как далеко до ближайшего
/// берега, от `WATER_SHORE_COLOR` на самом берегу до `WATER_COLOR` на глубине
/// `WATER_SHORE_WIDTH`.
///
/// Кайма по контуру каждого полигона (`spawn::push_area`, как у зелени) этого
/// не умела дважды. **Два полигона одной реки** — рукав Упы (мультиполигон
/// 19415535) упирается в её основной полигон общей границей поперёк устья, и
/// каждый клал вдоль этой границы свою светлую кайму: светлая полоса поперёк
/// воды там, где берега нет. И **узкая вода**: ширина каймы зажата долей
/// толщины полигона, так что рукав в 18 м темнел к середине резче, чем
/// мелела бы настоящая протока, и отмель реки обрывалась на входе в него.
///
/// Поэтому полигоны сначала сливаются (`i_overlay`, NonZero) — общей границы
/// больше нет, — а затем кладутся вложенными офсетами внутрь на каждые
/// [`SHOAL_STEP`]: полоса между офсетами `d` и `d + шаг` — полигон, у
/// которого внешнее кольцо цвета глубины `d`, а дырки — цвета `d + шаг`.
/// Офсет узкого места сходит на нет сам, и середина рукава получает цвет своей
/// настоящей глубины, а на устье изолинии плавно заворачивают из реки в рукав.
pub fn mesh_water_areas(areas: &[PolyArea]) -> MeshBuilder {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;
    use i_overlay::mesh::outline::offset::OutlineOffset;
    use i_overlay::mesh::style::{LineJoin, OutlineStyle};

    let mut builder = MeshBuilder::with_surface_coords();
    // NonZero сливает, только если обход согласован: внешние кольца против
    // часовой, дырки по ней — в OSM порядок точек какой придётся
    let contours: Vec<Vec<[f32; 2]>> = areas
        .iter()
        .flat_map(|area| {
            std::iter::once(oriented(&area.outer, true))
                .chain(area.holes.iter().map(|hole| oriented(hole, false)))
        })
        .collect();
    if contours.is_empty() {
        return builder;
    }
    let water: Vec<Shape> = contours.simplify_shape(FillRule::NonZero);

    let shore = WATER_SHORE_COLOR.to_linear();
    let deep = WATER_COLOR.to_linear();
    let steps = (WATER_SHORE_WIDTH / SHOAL_STEP).round() as usize;
    let color = |level: usize| shore.mix(&deep, (level as f32 / steps as f32).min(1.0));

    let mut levels = vec![water];
    for level in 1..=steps {
        let depth = level as f32 * SHOAL_STEP;
        let style = OutlineStyle::new(-depth).line_join(LineJoin::Round(SHOAL_ARC));
        let inset: Vec<Shape> = levels[0].outline(&style);
        if inset.is_empty() {
            break;
        }
        levels.push(inset);
    }

    for (level, shapes) in levels.iter().enumerate() {
        match levels.get(level + 1) {
            Some(inner) => {
                push_shoal_band(&mut builder, shapes, color(level), inner, color(level + 1))
            }
            // глубже воды нет: на полной глубине это заливка, в узком месте —
            // цвет той глубины, до которой вода дотянулась
            None => {
                for shape in shapes {
                    let (outer, holes) = rings(shape);
                    builder.push_polygon(&outer, &holes, color(level));
                }
            }
        }
    }
    builder
}

/// Полоса между уровнем `shapes` и вложенным в него уровнем `inner`.
///
/// Разность считается не булевой операцией, а раскладкой колец: у разности
/// потерялось бы, какая вершина с какого уровня, а цвет вершины — это ровно
/// её уровень. Внутренняя фигура целиком лежит в какой-то внешней, и её
/// внешнее кольцо — дырка полосы; её дырка обнимает остров, и кольцо между
/// ними — отдельный кусок полосы с островом внутри.
fn push_shoal_band(
    builder: &mut MeshBuilder,
    shapes: &[Shape],
    color: LinearRgba,
    inner: &[Shape],
    inner_color: LinearRgba,
) {
    let inner: Vec<Rings> = inner.iter().map(rings).collect();
    for shape in shapes {
        let (outer, holes) = rings(shape);
        let nested: Vec<&Rings> = inner
            .iter()
            // в дырке внешней фигуры лежит уже чужая вода — озеро на острове
            .filter(|(ring, _)| {
                point_in_polygon(ring[0], &outer)
                    && !holes.iter().any(|hole| point_in_polygon(ring[0], hole))
            })
            .collect();
        let swallowed = |hole: &[Vec2]| {
            nested
                .iter()
                .any(|(ring, _)| point_in_polygon(hole[0], ring))
        };

        let mut band_holes: Vec<(&[Vec2], LinearRgba)> = holes
            .iter()
            .filter(|hole| !swallowed(hole))
            .map(|hole| (hole.as_slice(), color))
            .collect();
        band_holes.extend(
            nested
                .iter()
                .map(|(ring, _)| (ring.as_slice(), inner_color)),
        );
        builder.push_polygon_graded((&outer, color), &band_holes);

        for (_, inner_holes) in &nested {
            for around in inner_holes {
                let islands: Vec<(&[Vec2], LinearRgba)> = holes
                    .iter()
                    .filter(|hole| point_in_polygon(hole[0], around))
                    .map(|hole| (hole.as_slice(), color))
                    .collect();
                builder.push_polygon_graded((around, inner_color), &islands);
            }
        }
    }
}

/// Кольца фигуры `i_overlay` в `Vec2`: внешнее и дырки.
fn rings(shape: &Shape) -> Rings {
    let mut contours = shape.iter().map(|contour| {
        contour
            .iter()
            .copied()
            .map(Vec2::from_array)
            .collect::<Vec<_>>()
    });
    let outer = contours.next().unwrap_or_default();
    (outer, contours.collect())
}

/// Кольцо в массивах `i_overlay`, против часовой стрелки или по ней.
fn oriented(ring: &[Vec2], counterclockwise: bool) -> Vec<[f32; 2]> {
    let mut points: Vec<[f32; 2]> = ring.iter().map(Vec2::to_array).collect();
    if (signed_ring_area(ring) > 0.0) != counterclockwise {
        points.reverse();
    }
    points
}

/// Лента открытых русел одним мешем. **Трубы не рисуются вовсе**: под землёй
/// воды не видно, а пунктир вдоль улицы читался как ручей поверх неё. Тем, что
/// человек проходит там, где на карте «ручей», управляет не эта отрисовка, а
/// её отсутствие: русло обрывается на портале культверта и продолжается за ним
/// (`water_line_caps`), и между порталами воды на карте просто нет.
pub fn mesh_water_lines(lines: &[WaterLine], water: &[PolyArea]) -> MeshBuilder {
    let color = WATER_COLOR.to_linear();
    let index = AreaIndex::new(water);
    let mut open = MeshBuilder::with_surface_coords();

    for line in lines.iter().filter(|line| !line.tunnel) {
        // сглаживание как у дорог: русло в OSM — ломаная по точкам съёмки, и на
        // её изломах лента без сглаживания заметно гранёная. Режется уже
        // сглаженная ось — та, по которой лента и ляжет
        let path = roads::smooth_path(&line.points, line.width, RoadSmoothing::Light);
        // круглые торцы там, где вода продолжается: два way одного русла
        // встречаются в общем узле, и полудиски сливаются в непрерывную реку.
        // Портал культверта — исключение: за ним воды нет, и полудиск торчал бы
        // на полуширину русла в сухую землю
        let caps = water_line_caps(line, lines).map(|round| {
            if round {
                RibbonCap::Round
            } else {
                RibbonCap::Butt
            }
        });
        for run in index.outside_runs(&path, WATER_SHORE_WIDTH) {
            // отрезанный конец лежит в воде на ширину отмели глубже берега:
            // полудиск там ни к чему, а «до разрыва» от берега до торца идёт от
            // нуля к минус ширине отмели — по нему шейдер гасит кромки ленты
            let mut breaks = Vec::with_capacity(2);
            let mut run_caps = caps;
            for (end, point) in [(0, run.points[0]), (1, run.points[run.points.len() - 1])] {
                if run.clipped[end] {
                    run_caps[end] = RibbonCap::Butt;
                    breaks.push(Break {
                        at: point,
                        reach: WATER_SHORE_WIDTH,
                    });
                }
            }
            // `At` даже без разрывов: у `Ends` «до разрыва» считается до торца, и
            // отмель гасла бы у каждого конца русла, в том числе на суше
            open.push_ribbon_broken(
                &run.points,
                line.width,
                color,
                RibbonJoin::Round,
                run_caps,
                RibbonBreaks::At(&breaks),
            );
        }
    }

    open
}

#[cfg(test)]
mod tests;
