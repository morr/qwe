//! Арки: `tunnel=building_passage` — это дорога, проложенная сквозь дом, и в
//! navmesh она уже прорезана коридором. Здание же рисуется сплошным, и пешки
//! идут сквозь нарисованную стену — карта врёт.
//!
//! Проём лежит **в плоскости стены** и выровнен по грани контура. Ширина —
//! ширина самой дороги, спроецированная углом входа (|sin| между дорогой и
//! гранью: перпендикулярный вход — полная ширина, скользящий — почти ничего)
//! и подрезанная концами грани, чтобы у арки возле угла дома квад не повисал
//! в воздухе. Высота — [`ARCH_HEIGHT`] настоящих метров долей высоты
//! **этого** дома: `band × 6 / height`. Не через `EXTRUDE_SCALE` — подъём
//! обрезан `EXTRUDE_RANGE`, и у сарая или башни нарисованный метр стоит не
//! тех же 0.35 настоящих.
//!
//! Ни то ни другое **не подгоняется под клетки стены**, и это осознанно:
//! навмеш прорезан настоящей шириной дороги, и проём уже неё вернул бы ровно
//! ту ложь, ради которой арки и появились, — пешка идёт там, где нарисована
//! стена. Вместо этого клетки, которые проём задел, отдаются ему целиком:
//! вокруг выреза кладётся **заплата** ([`WallCells`], `WallMark::Solid`) —
//! те же швы и та же кладка, но без окон и балконов. Иначе окно, стоящее по
//! центру клетки, обрезалось бы краем проёма: шейдер о вырезе не знает, он
//! красит ту клетку, которая до него доехала. Ровно так же устроена дверь
//! (`layers::push_doors`) — одна конструкция на оба проёма.
//!
//! Стена ищется не пересечением дороги с контуром, а от **концов** прохода: в
//! OSM арку сплошь и рядом размечают отрезком от вершины контура до вершины
//! контура (арка 485488257 в Туле — ровно такая), то есть дорога лежит внутри
//! дома и стен касается только концами. Пересечения там нет вовсе, зато
//! каждый конец — и есть выход арки наружу.

use std::collections::HashMap;

use bevy::color::Mix;
use bevy::prelude::*;

use super::height_or_default;
use super::layers::{WallSpan, silhouette_edges};
use crate::map::SHADOW_COLOR;
use crate::map::meshing::{MeshBuilder, WallFrame};
use crate::map::osm::model::{
    closest_on_segment, point_at_arc_length, point_in_area, polyline_length, ring_bounds,
};
use crate::map::osm::{PolyArea, RoadLine};
use crate::settings::ARCH_HEIGHT;

/// Цвет проёма арки — та же подложка, что у земли в `spawn.rs`: сквозь арку
/// видно двор, а не стену.
const ARCH_COLOR: Color = Color::srgb(0.878, 0.865, 0.827);
/// Дальше скольких метров конец прохода не считается выходом на стену:
/// конец в OSM — общая вершина контура, так что реально там ноль; запас
/// покрывает шум проекции и слегка неровную разметку.
const ARCH_WALL_REACH: f32 = 6.0;
/// Насколько дальше ближайшей грани всё ещё «та же» стена, м: у общей вершины
/// двух граней обе на нулевом расстоянии, и проём обязан кроиться по обеим.
const ARCH_WALL_TIE: f32 = 0.5;

/// Проём, прорезанный в одной грани контура здания.
pub(super) struct ArchOpening {
    /// Грань, в которой прорезан проём.
    pub(super) a: Vec2,
    pub(super) b: Vec2,
    /// Интервал проёма вдоль грани, м от `a`.
    pub(super) low: f32,
    pub(super) high: f32,
    /// Вертикальный габарит проёма — доля полосы/подъёма режима.
    pub(super) sill: Vec2,
}

/// `facing` — куда смотрят видимые стены режима: в 2.5D против вектора
/// подъёма (`-Lean::dir()`), у фасадной полосы строго на юг. Проёмы
/// кроятся ровно по тем граням, что режим рисует, — иначе стена и вырез не
/// совпадут ребро в ребро, а `push_wall_with_openings` ищет проём по
/// точному равенству концов грани.
pub(super) fn arch_openings(
    building: &PolyArea,
    passages: &[&RoadLine],
    band: Vec2,
    facing: Vec2,
) -> Vec<ArchOpening> {
    if passages.is_empty() || band == Vec2::ZERO {
        return Vec::new();
    }
    // доля стены, которую занимает проём; у совсем низкого дома арка не
    // может быть выше него самого
    let sill = band * (ARCH_HEIGHT / height_or_default(building)).min(1.0);

    // видимые стены — те же грани, что рисует `extrusion_builder`
    let walls: Vec<(Vec2, Vec2)> = silhouette_edges(&building.outer, facing)
        .into_iter()
        .chain(
            building
                .holes
                .iter()
                .flat_map(|hole| silhouette_edges(hole, -facing)),
        )
        .collect();

    let mut openings = Vec::new();
    for passage in passages {
        // конец прохода и направление, которым дорога входит в дом
        let ends = [
            passage.points.first().zip(passage.points.get(1)),
            passage
                .points
                .last()
                .zip(passage.points.iter().rev().nth(1)),
        ];
        for &(&point, &neighbour) in ends.iter().flatten() {
            let Some(direction) = (neighbour - point).try_normalize() else {
                continue;
            };
            // конец прохода в OSM — общая вершина контура, то есть точка
            // стыка ДВУХ граней: проём, зажатый в одну из них, обрезался бы
            // до половины ширины дороги. Кроим по всем граням в пределах
            // допуска от ближайшей — на стыке куски продолжают друг друга.
            let nearest = walls
                .iter()
                .map(|&(a, b)| point.distance(closest_on_segment(point, a, b)))
                .fold(f32::MAX, f32::min);
            if nearest > ARCH_WALL_REACH {
                continue;
            }
            for &(a, b) in &walls {
                let at = closest_on_segment(point, a, b);
                if point.distance(at) > nearest + ARCH_WALL_TIE {
                    continue;
                }
                let Some(along) = (b - a).try_normalize() else {
                    continue;
                };
                // дорога под углом к стене дырявит её уже собственной ширины
                let half = passage.width / 2.0 * direction.perp_dot(along).abs();

                let length = (b - a).length();
                let base = (at - a).dot(along);
                let (low, high) = ((base - half).max(0.0), (base + half).min(length));
                if high - low < 0.05 {
                    continue;
                }
                openings.push(ArchOpening {
                    a,
                    b,
                    low,
                    high,
                    sill,
                });
            }
        }
    }
    openings
}

/// Клетки той стены, в которой кроится проём: её рама, та же рама «без
/// проёмов» под заплату и размер клетки по обеим осям (панель вдоль основания,
/// этаж вверх по подъёму).
///
/// Собирает это `layers::wall_cells` — там и живёт вся арифметика панелей и
/// этажей; сюда четвёрка приезжает готовой, потому что резать по клеткам
/// приходится ровно здесь.
pub(super) struct WallCells {
    /// Рама стены — рисунок материала, окна, балконы.
    pub(super) frame: Option<WallFrame>,
    /// Она же, помеченная `WallMark::Solid`: швы и кладка те же, проёмов нет.
    pub(super) patch: Option<WallFrame>,
    /// Ширина панели этой стены, м вдоль основания.
    pub(super) panel: f32,
    /// Этаж стены — вектор вверх по подъёму.
    pub(super) storey: Vec2,
}

impl WallCells {
    /// Клетки, которые задел проём `low..high`, — целые панели вокруг него, за
    /// концы стены длиной `length` не выходящие. Клетку стена отдаёт проёму
    /// целиком: окно стоит по её центру, шейдер про вырез не знает, и край
    /// проёма резал бы ряд окон пополам. По этой же границе кроится заплата
    /// двери (`layers::push_doors`) — одна конструкция на оба проёма, и счёт
    /// у неё поэтому один.
    pub(super) fn block(&self, low: f32, high: f32, length: f32) -> (f32, f32) {
        (
            ((low / self.panel).floor() * self.panel).max(0.0),
            ((high / self.panel).ceil() * self.panel).min(length),
        )
    }
}

/// Стена с проёмами: боковые куски во всю высоту, перемычка над каждой аркой и
/// **заплата** вокруг выреза. Это настоящий вырез — в дыру просвечивают нижние
/// слои (дорога, проложенная сквозь дом, тень), а не закраска цветом земли.
///
/// Заплата — те клетки, которые проём задел не целиком: простенок сбоку от
/// арки шириной меньше панели и перемычка высотой меньше этажа. Окно стоит по
/// центру клетки, шейдер про вырез не знает, и без заплаты край проёма резал
/// бы ряд окон пополам. Клетки **внутри** проёма выброшены вместе с ним, а
/// выше и по бокам от заплаты стена целая — там окна полные, и гасить их
/// незачем.
///
/// Два проезда, выходящие в одну грань ближе панели друг от друга, кроятся
/// **одной** заплатой на обоих: у каждого своя дыра, между ними простенок по
/// настоящим краям вырезов. Своя заплата у каждого замуровала бы соседа —
/// простенок первого лёг бы поперёк второго проёма, а навмеш прорезан обоими.
pub(super) fn push_wall_with_openings(
    builder: &mut MeshBuilder,
    span: &WallSpan,
    cells: &WallCells,
    openings: &[ArchOpening],
    bottom: LinearRgba,
    top: LinearRgba,
) {
    let (a, b, lift) = (span.a, span.b, span.lift);
    let mut cuts: Vec<&ArchOpening> = openings
        .iter()
        .filter(|opening| opening.a == a && opening.b == b)
        .collect();
    builder.set_wall(cells.frame);
    if cuts.is_empty() {
        builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
        return;
    }
    cuts.sort_by(|first, second| first.low.total_cmp(&second.low));

    let Some(along) = (b - a).try_normalize() else {
        return;
    };
    let length = (b - a).length();
    let height = lift.length();
    // цвет стены на этой высоте — тот же градиент, что у целой стены
    let shade = |up: Vec2| match height > 0.0 {
        true => bottom.mix(&top, (up.length() / height).clamp(0.0, 1.0)),
        false => bottom,
    };
    let piece = |builder: &mut MeshBuilder, from: f32, to: f32, low: Vec2, high: Vec2| {
        if to - from < 0.01 || (high - low).length() < 0.01 {
            return;
        }
        let (p0, p1) = (a + along * from, a + along * to);
        let (under, over) = (shade(low), shade(high));
        builder.push_quad_gradient(
            [p0 + low, p1 + low, p1 + high, p0 + high],
            [under, under, over, over],
        );
    };

    let mut cursor: f32 = 0.0;
    let mut rest: &[&ArchOpening] = &cuts;
    while !rest.is_empty() {
        // клетки, которые проём задел: целые панели вокруг него и целые этажи
        // под перемычкой — за эту границу заплата не выходит. Соседний проезд,
        // попавший в те же панели, идёт с ним одной группой под одной заплатой:
        // отдать блок первому вырезу и пропустить второй значило бы замуровать
        // его простенком, а навмеш прорезан обоими
        let mut block = cells.block(rest[0].low, rest[0].high, length);
        let mut group = 1;
        while let Some(next) = rest.get(group) {
            let reach = cells.block(next.low, next.high, length);
            if reach.0 >= block.1 {
                break;
            }
            block.1 = block.1.max(reach.1);
            group += 1;
        }
        let (group_cuts, tail) = rest.split_at(group);
        rest = tail;

        let start = block.0.max(cursor);
        // перемычка у группы общая, поэтому этажи — по самому высокому вырезу
        let storeys = group_cuts
            .iter()
            .map(|cut| (cut.sill.length() / cells.storey.length()).ceil())
            .fold(0.0, f32::max);
        let over = match cells.storey * storeys {
            up if up.length() >= height => lift,
            up => up,
        };

        // стена слева от заплаты и над ней — обычная, с окнами
        builder.set_wall(cells.frame);
        piece(builder, cursor, start, Vec2::ZERO, lift);
        piece(builder, start, block.1, over, lift);
        // и заплата: простенки по краям группы и между её вырезами, перемычка
        // над каждым вырезом
        builder.set_wall(cells.patch);
        let mut edge = start;
        for cut in group_cuts {
            piece(builder, edge, cut.low, Vec2::ZERO, over);
            piece(builder, cut.low.max(edge), cut.high, cut.sill, over);
            edge = cut.high.max(edge);
        }
        piece(builder, edge, block.1, Vec2::ZERO, over);

        cursor = block.1.max(cursor);
    }
    builder.set_wall(cells.frame);
    piece(builder, cursor, length, Vec2::ZERO, lift);
}

/// Фасадные режимы: полоса фасада — один earcut-полигон на всё здание, и
/// честный паз в нём потребовал бы булевой операции над полигонами. Здесь
/// проём **закрашивается** цветом подложки — компромисс; настоящий вырез
/// живёт в 2.5D (`push_wall_with_openings`).
pub(super) fn push_arches(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    passages: &[&RoadLine],
    band: Vec2,
) {
    // проём затенён перемычкой над ним — подложка мешается с тоном тени
    let color = ARCH_COLOR
        .mix(&Color::srgb(0.22, 0.24, 0.33), SHADOW_COLOR.alpha())
        .to_linear();
    // полоса фасада сдвинута строго вниз — видны только южные грани
    for opening in arch_openings(building, passages, band, Vec2::NEG_Y) {
        let Some(along) = (opening.b - opening.a).try_normalize() else {
            continue;
        };
        push_swept_quad(
            builder,
            [
                opening.a + along * opening.low,
                opening.a + along * opening.high,
            ],
            opening.sill,
            color,
        );
    }
}

/// Проходы, разложенные по домам, которые они прорезают: `building_passage`
/// размечают ровно тем куском дороги, что лежит под домом, поэтому дом
/// ищется по середине прохода.
pub(super) fn arches_by_building<'a>(
    buildings: &[PolyArea],
    passages: &'a [RoadLine],
) -> HashMap<usize, Vec<&'a RoadLine>> {
    let mut by_building: HashMap<usize, Vec<&RoadLine>> = HashMap::new();
    for passage in passages.iter().filter(|road| road.passage) {
        let Some(middle) = passage_middle(passage) else {
            continue;
        };
        let pierced = buildings.iter().position(|building| {
            let (min, max) = ring_bounds(&building.outer);
            middle.x >= min.x
                && middle.x <= max.x
                && middle.y >= min.y
                && middle.y <= max.y
                && point_in_area(middle, building)
        });
        if let Some(index) = pierced {
            by_building.entry(index).or_default().push(passage);
        }
    }
    by_building
}

/// Середина ломаной по длине — устойчивее к неравномерным сегментам, чем
/// средняя точка списка.
pub(super) fn passage_middle(passage: &RoadLine) -> Option<Vec2> {
    let total = polyline_length(&passage.points);
    if total <= 0.0 {
        return passage.points.first().copied();
    }
    Some(point_at_arc_length(&passage.points, total / 2.0))
}

/// Отрезок, протянутый вектором `sweep`, — прямоугольник проёма в стене.
fn push_swept_quad(builder: &mut MeshBuilder, edge: [Vec2; 2], sweep: Vec2, color: LinearRgba) {
    builder.push_polygon(
        &[edge[0], edge[1], edge[1] + sweep, edge[0] + sweep],
        &[],
        color,
    );
}
