//! Формы подсветки элемента под курсором: чистая геометрия того, что панель
//! инспектора рисует прозрачно-жёлтым при включённой галочке highlight, и меш
//! этих форм одним цветом.

use bevy::prelude::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::style::LineCap;

use crate::map::footprint::FENCE_BAND_WIDTH;
use crate::map::osm::model::{MapData, PolyArea};
use crate::map::shapes::{
    Contour, Shape, contour_bounds, is_ring, oriented, push_shape, ring_of, shape_area, stroke,
};
use crate::map::{MeshBuilder, RoadStyle};

use super::{FeatureRef, InspectIndex, NODE_REACH};

/// Форма одного полигонального куска элемента: внешнее кольцо и дырки.
///
/// Кусков у элемента бывает много — обводка ломаной распадается на кольца,
/// полосы-генераты кладутся квадом на отрезок осевой, — поэтому
/// [`feature_shape`] отвечает списком.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureShape {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
}

/// Форма элемента пробы для подсветки: что рисовать прозрачно-жёлтым, когда
/// в панели инспектора включена галочка highlight (`ui/inspector.rs`). Чистая
/// функция над [`MapData`] и индексом: площади — контуром с дырками, линии —
/// обводкой на своей полной ширине (`shapes::stroke`), полосы-генераты —
/// квадом на отрезок осевой между двумя офсетами, узлы и деревья — дисками,
/// карманы у бордюра — полигоном из индекса ([`InspectIndex::kerb_pockets`],
/// их в [`MapData`] нет).
///
/// Ширины те же, что у зоны срабатывания находки ([`InspectIndex::hits`]):
/// форма накрывает ровно то, на что находка отвечает. Дерево видит ту же
/// плотность, что и находка, — [`InspectIndex::hits`] отвечает деревом, только
/// пока его ранг в префиксе плотности. Две приблизительности здесь — свойства
/// самого индекса, не формы: полоса стыкуется пер-сегментно, без скруглений
/// (в изгибе осевой стык квадов чуть уже зоны), а зум-LOD обочин не виден ни
/// тому, ни другому.
pub fn feature_shape(
    index: &InspectIndex,
    map: &MapData,
    roads: &RoadStyle,
    feature: FeatureRef,
) -> Option<Vec<FeatureShape>> {
    match feature {
        FeatureRef::Building(i) => area_shape(map.buildings.get(i as usize)?),
        FeatureRef::Water(i) => area_shape(map.water.get(i as usize)?),
        FeatureRef::Park(i) => area_shape(map.parks.get(i as usize)?),
        FeatureRef::Wood(i) => area_shape(map.woods.get(i as usize)?),
        FeatureRef::Grass(i) => area_shape(map.grass.get(i as usize)?),
        FeatureRef::Sand(i) => area_shape(map.sand.get(i as usize)?),
        FeatureRef::Landuse(i) => area_shape(map.landuse.get(i as usize)?),
        FeatureRef::Pocket(i) => area_shape(map.pockets.get(i as usize)?),
        FeatureRef::Parking(i) => area_shape(map.parking.get(i as usize)?),
        FeatureRef::Pitch(i) => area_shape(map.pitches.get(i as usize)?),
        FeatureRef::RoadArea(i) => {
            let area = map.road_areas.get(i as usize)?;
            Some(vec![FeatureShape {
                outer: area.outline.clone(),
                holes: vec![],
            }])
        }
        FeatureRef::Road(i) => {
            let road = map.roads.get(i as usize)?;
            Some(stroke_shapes(&road.points, road.width))
        }
        FeatureRef::Rail(i) => {
            let rail = map.rails.get(i as usize)?;
            Some(stroke_shapes(&rail.points, rail.width))
        }
        FeatureRef::Waterway(i) => {
            let line = map.water_lines.get(i as usize)?;
            Some(stroke_shapes(&line.points, line.width))
        }
        FeatureRef::Pipe(i) => {
            let pipe = map.pipes.get(i as usize)?;
            Some(stroke_shapes(&pipe.points, pipe.width))
        }
        FeatureRef::Wall(i) => {
            let wall = map.walls.get(i as usize)?;
            Some(stroke_shapes(&wall.points, wall.width))
        }
        FeatureRef::Fence(i) => {
            let fence = map.fences.get(i as usize)?;
            Some(stroke_shapes(&fence.points, FENCE_BAND_WIDTH))
        }
        FeatureRef::SidewalkBand { road: i, side } => {
            let road = map.roads.get(i as usize)?;
            let profile = road.sidewalk();
            let band = profile.band()?;
            let side = side as usize;
            (roads.sidewalks && profile.on(side).is_some()).then(|| {
                let half = road.width / 2.0;
                band_shapes(&road.points, half, half + band, side as u8)
            })
        }
        FeatureRef::RoadNode(i) => {
            let node = map.road_nodes.get(i as usize)?;
            Some(vec![disk(node.pos, NODE_REACH)])
        }
        FeatureRef::Structure(i) => {
            let structure = map.structures.get(i as usize)?;
            Some(vec![disk(structure.at, structure.radius)])
        }
        FeatureRef::Tree(rank) => {
            let (pos, radius) = map.trees.positions().get(rank as usize)?;
            Some(vec![disk(*pos, *radius)])
        }
        FeatureRef::KerbPocket(i) => {
            let pocket = index.kerb_pockets.get(i as usize)?;
            Some(vec![FeatureShape {
                outer: pocket.outline.clone(),
                holes: vec![],
            }])
        }
        FeatureRef::Piece(i) => {
            let piece = index.pieces.get(i as usize)?;
            (!piece.contour.is_empty()).then(|| {
                vec![FeatureShape {
                    outer: piece.contour.clone(),
                    holes: piece.holes.clone(),
                }]
            })
        }
    }
}

/// Меш подсветки: все куски формы одним цветом. Полигон с дырками идёт в
/// `earcut` целиком — двор внутри дома остаётся неподсвеченным.
pub fn mesh_feature(shapes: &[FeatureShape], color: LinearRgba) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    for shape in shapes {
        builder.push_polygon(&shape.outer, &shape.holes, color);
    }
    builder
}

/// Запиненный элемент для [`mesh_pins`]: его формы и его цвет.
pub struct PinnedFill {
    pub shapes: Vec<FeatureShape>,
    pub color: LinearRgba,
}

/// Целевая толщина полосы штриховки перекрытий на экране, логических px: от
/// неё считается ширина полосы из зума ([`pin_stripe`]), поэтому полоса
/// одинакова на глаз на любом зуме.
const PIN_STRIPE_PX: f32 = 14.0;
/// Пределы ширины полосы штриховки, м: тесный пин не рассыпается в волоски,
/// широкий не разбухает до «полосатой страны». Пол — ниже целевых 9 px на
/// рабочих зумах (0.05 м/px даёт 0.45 м), так что цель работает, а не кламп.
const PIN_STRIPE_MIN: f32 = 0.2;
const PIN_STRIPE_MAX: f32 = 50.0;
/// Косинус 45° — он же синус: перекрытия штрихуются ровно под этой углом.
const COS_45: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Штриховка не начинается с мельчайших клякс: перекрытие мельче этого, м²,
/// красится сплошным цветом пары — полоса в такой щепке одна, чередоваться
/// нечему, а рваный край из полосок в вершину её только грязнит.
const MIN_HATCH_AREA: f32 = 1.0;

/// Ширина полосы штриховки для зума `zoom` (метры на экранный пиксель).
pub fn pin_stripe(zoom: f32) -> f32 {
    (zoom * PIN_STRIPE_PX).clamp(PIN_STRIPE_MIN, PIN_STRIPE_MAX)
}

/// Альфа рамки пина: цвет рамки — цвет заливки, но рамка плотнее заливки и
/// насыщеннее на глаз, поэтому читается как контур, а не вторая заливка.
const PIN_BORDER_ALPHA: f32 = 0.85;

/// Заливка запиненных элементов одним слоем: каждый — своим цветом. Там,
/// где элементы накрыли друг друга, заливка идёт штриховкой под 45°, полосы
/// которой чередуют цвета накрывших элементов: перекрытие и читается как
/// «здесь оба», и говорит, какой строке панели какой цвет принадлежит.
/// Сплошные части поэтому вычитаются — каждый элемент рисует только то, чего
/// не касаются другие, — полупрозрачный слой нигде не складывается вдвое.
/// Перекрытия трёх и более штрихует первая пара, до них дотянувшаяся: полосы
/// чередуют два из накрывших цветов, для отладочной подсветки достаточно, а
/// полный расклад регионов по подмножествам дороже самого индекса.
///
/// `stripe` — ширина полосы, м; её считает [`pin_stripe`] из зума, поэтому
/// пересборка слоёв при смене зума ([`sync_pinned_highlights`]) держит полосу
/// одной толщины на глаз. `border` — ширина рамки **каждого** пина (м; ноль —
/// без рамок): у каждой подсвеченной области своя рамка — цвет заливки, но
/// плотнее и насыщеннее на глаз ([`PIN_BORDER_ALPHA`]), — и пересечение
/// областей видно по пересечению их рамок, а не по рамке вокруг перекрытия.
pub fn mesh_pins(fills: &[PinnedFill], stripe: f32, border: f32) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    // наборы контуров каждого элемента: внешние против часовой, дырки по ней —
    // та закрутка, которой NonZero отличает тело от дырки
    let contours: Vec<Vec<Contour>> = fills
        .iter()
        .map(|fill| {
            let mut all = Vec::new();
            for shape in &fill.shapes {
                all.push(oriented(&shape.outer, true));
                all.extend(shape.holes.iter().map(|hole| oriented(hole, false)));
            }
            all
        })
        .collect();

    // сплошные части: свой цвет минус все прочие пины
    for (i, fill) in fills.iter().enumerate() {
        let others: Vec<Contour> = contours
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .flat_map(|(_, set)| set.iter().cloned())
            .collect();
        if others.is_empty() {
            for shape in &fill.shapes {
                builder.push_polygon(&shape.outer, &shape.holes, fill.color);
            }
            continue;
        }
        for shape in
            contours[i]
                .clone()
                .overlay(&others, OverlayRule::Difference, FillRule::NonZero)
        {
            push_shape(&mut builder, shape, fill.color);
        }
    }

    // перекрытия парами: пересечение, минус уже заштрихованное, штрихами
    let bounds: Vec<(Vec2, Vec2)> = contours
        .iter()
        .map(|set| {
            set.iter().map(contour_bounds).fold(
                (Vec2::INFINITY, Vec2::NEG_INFINITY),
                |(a0, a1), (b0, b1)| (a0.min(b0), a1.max(b1)),
            )
        })
        .collect();
    let mut hatched: Vec<Contour> = Vec::new();
    for i in 0..fills.len() {
        for j in (i + 1)..fills.len() {
            let (a_low, a_high) = bounds[i];
            let (b_low, b_high) = bounds[j];
            if a_high.x < b_low.x || b_high.x < a_low.x || a_high.y < b_low.y || b_high.y < a_low.y
            {
                continue;
            }
            let overlap: Vec<Shape> = contours[i].clone().overlay(
                &contours[j],
                OverlayRule::Intersect,
                FillRule::NonZero,
            );
            if overlap.is_empty() {
                continue;
            }
            let overlap: Vec<Shape> = if hatched.is_empty() {
                overlap
            } else {
                overlap
                    .into_iter()
                    .flatten()
                    .collect::<Vec<Contour>>()
                    .overlay(&hatched, OverlayRule::Difference, FillRule::NonZero)
            };
            for shape in &overlap {
                if shape_area(shape) < MIN_HATCH_AREA {
                    // клякса без своей полосы — сплошным цветом пары
                    push_shape(&mut builder, shape.clone(), fills[i].color);
                } else {
                    hatch_shape(&mut builder, shape, stripe, fills[i].color, fills[j].color);
                }
            }
            hatched.extend(overlap.into_iter().flatten());
        }
    }

    // рамки: у каждого пина своя, по его полным контурам — и пересечение
    // областей видно по пересечению рамок, а не по рамке вокруг перекрытия
    if border > 0.0 {
        for (i, fill) in fills.iter().enumerate() {
            let border_color = LinearRgba {
                alpha: PIN_BORDER_ALPHA,
                ..fill.color
            };
            push_border(&mut builder, &contours[i], border, border_color);
        }
    }
    builder
}

/// Рамка пина: обводка всех колец его формы плотным цветом заливки. Полоса
/// обводки вокруг замкнутого кольца — аннулус: `stroke` возвращает внешнее
/// кольцо и дырку, дырка идёт в полигон дыркой, а не второй сплошной
/// заливкой. Кольцо замыкается повтором первой точки: `stroke` с
/// `ring: true` ждёт именно такой путь.
fn push_border(builder: &mut MeshBuilder, contours: &[Contour], border: f32, color: LinearRgba) {
    for contour in contours {
        let mut path = ring_of(contour);
        if !is_ring(&path) {
            path.push(path[0]);
        }
        let mut rings = stroke(&path, border, LineCap::Butt, true).into_iter();
        let Some(first) = rings.next() else {
            continue;
        };
        let holes: Vec<Vec<Vec2>> = rings.map(|ring| ring_of(&ring)).collect();
        builder.push_polygon(&ring_of(&first), &holes, color);
    }
}

/// Один регион перекрытия — штрихами: полосы под 45° **экранно** — из
/// левого нижнего края в правый верхний (карта рисуется y вверх, так что это
/// линии `y − x = const`), ширина полосы одинаковая, каждая — пересечение с
/// регионом и своим цветом из пары. И полосы, и регион режутся в
/// повёрнутом на +45° пространстве; куски возвращаются в координаты карты до
/// укладки в меш. Фаза привязана к нулю карты, а не к региону: куски одного
/// перекрытия сходятся в одну штриховку.
fn hatch_shape(
    builder: &mut MeshBuilder,
    shape: &Shape,
    stripe: f32,
    even: LinearRgba,
    odd: LinearRgba,
) {
    // поворот на +45°: вдоль штриха идёт диагональ (1,1), поперёк — (1,−1)
    let rotate = |p: [f32; 2]| [(p[1] - p[0]) * COS_45, (p[0] + p[1]) * COS_45];
    let unrotate = |p: [f32; 2]| [(p[1] - p[0]) * COS_45, (p[1] + p[0]) * COS_45];
    let rotated: Vec<Contour> = shape
        .iter()
        .map(|contour| contour.iter().copied().map(rotate).collect())
        .collect();
    let (low, high) = rotated.iter().map(contour_bounds).fold(
        (Vec2::INFINITY, Vec2::NEG_INFINITY),
        |(a0, a1), (b0, b1)| (a0.min(b0), a1.max(b1)),
    );
    // запас по штриху: прямоугольник полосы обязан накрыть регион целиком
    let (v0, v1) = (low.y - stripe, high.y + stripe);
    let first = (low.x / stripe).floor() as i64;
    let last = (high.x / stripe).floor() as i64;
    for k in first..=last {
        let (a, b) = (k as f32 * stripe, (k + 1) as f32 * stripe);
        let color = if k.rem_euclid(2) == 0 { even } else { odd };
        let band: Contour = [[a, v0], [b, v0], [b, v1], [a, v1]].into_iter().collect();
        for piece in vec![band].overlay(&rotated, OverlayRule::Intersect, FillRule::NonZero) {
            let piece: Shape = piece
                .iter()
                .map(|contour| contour.iter().copied().map(unrotate).collect())
                .collect();
            push_shape(builder, piece, color);
        }
    }
}

/// Площадь — своя геометрия как есть.
fn area_shape(area: &PolyArea) -> Option<Vec<FeatureShape>> {
    (!area.outer.is_empty()).then(|| {
        vec![FeatureShape {
            outer: area.outer.clone(),
            holes: area.holes.clone(),
        }]
    })
}

/// Обводка ломаной на полной ширине: список колец, какие дала обводка
/// (самопересечения и кольца распадаются на куски).
fn stroke_shapes(points: &[Vec2], width: f32) -> Vec<FeatureShape> {
    stroke(points, width, LineCap::Butt, is_ring(points))
        .into_iter()
        .map(|contour| FeatureShape {
            outer: ring_of(&contour),
            holes: vec![],
        })
        .collect()
}

/// Полоса вдоль оси на одной стороне: квад на каждый отрезок, без сглаживания
/// и скруглений — та же зона, по которой находка мерит расстояние до осевой.
fn band_shapes(points: &[Vec2], inner: f32, outer: f32, side: u8) -> Vec<FeatureShape> {
    points
        .windows(2)
        .filter_map(|segment| {
            let direction = (segment[1] - segment[0]).normalize_or_zero();
            if direction == Vec2::ZERO {
                return None;
            }
            let perp = direction.perp();
            let perp = if side == 0 { perp } else { -perp };
            Some(FeatureShape {
                outer: vec![
                    segment[0] + perp * inner,
                    segment[1] + perp * inner,
                    segment[1] + perp * outer,
                    segment[0] + perp * outer,
                ],
                holes: vec![],
            })
        })
        .collect()
}

/// Диск многоугольником из 24 граней — подсветка узла, цилиндра, кроны.
fn disk(center: Vec2, radius: f32) -> FeatureShape {
    let sides = 24;
    FeatureShape {
        outer: (0..sides)
            .map(|i| {
                center + Vec2::from_angle(i as f32 / sides as f32 * std::f32::consts::TAU) * radius
            })
            .collect(),
        holes: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min: Vec2, max: Vec2) -> FeatureShape {
        FeatureShape {
            outer: vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)],
            holes: vec![],
        }
    }

    /// Цвет треугольника, накрывшего точку; `None` — точка ничем не накрыта.
    /// Меш индексированный: треугольники — тройки индексов, не тройки позиций.
    fn color_at(builder: &MeshBuilder, point: Vec2) -> Option<LinearRgba> {
        builder
            .colored_triangles_for_test()
            .find_map(|(tri, color)| {
                let sides = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])]
                    .map(|(from, to)| (to - from).perp_dot(point - from));
                (sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0))
                    .then_some(color)
            })
    }

    #[test]
    fn two_overlapping_pins_hatch_their_overlap() {
        let red = LinearRgba::RED;
        let green = LinearRgba::rgb(0.0, 1.0, 0.0);
        let builder = mesh_pins(
            &[
                PinnedFill {
                    shapes: vec![square(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0))],
                    color: red,
                },
                PinnedFill {
                    shapes: vec![square(Vec2::new(10.0, 0.0), Vec2::new(30.0, 20.0))],
                    color: green,
                },
            ],
            4.0,
            0.0,
        );
        // сплошные части: свой цвет минус чужой пин
        assert_eq!(color_at(&builder, Vec2::new(5.0, 10.0)), Some(red));
        assert_eq!(color_at(&builder, Vec2::new(25.0, 10.0)), Some(green));
        // перекрытие 10..20 — штриховка: в обе стороны от точки полосы
        // чередуются, оба цвета в перекрытии присутствуют
        let mut reds = 0;
        let mut greens = 0;
        for x in [10.5, 12.0, 13.5, 15.0, 16.5, 18.0, 19.5] {
            match color_at(&builder, Vec2::new(x, 10.0)) {
                Some(color) if color == red => reds += 1,
                Some(color) if color == green => greens += 1,
                other => panic!("overlap point x={x} uncovered or wrong color: {other:?}"),
            }
        }
        assert!(
            reds > 0 && greens > 0,
            "the overlap must carry both stripe colors"
        );
    }

    #[test]
    fn two_identical_fills_hatch_entirely() {
        // hover над запиненным элементом: две одинаковые заливки (жёлтая и
        // цвет пина) — соло пусты, вся форма ушла в перекрытие и штрихуется
        let red = LinearRgba::RED;
        let green = LinearRgba::rgb(0.0, 1.0, 0.0);
        let shape = square(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0));
        let builder = mesh_pins(
            &[
                PinnedFill {
                    shapes: vec![shape.clone()],
                    color: red,
                },
                PinnedFill {
                    shapes: vec![shape],
                    color: green,
                },
            ],
            4.0,
            0.0,
        );
        let mut reds = 0;
        let mut greens = 0;
        for x in [2.0, 6.0, 10.0, 14.0, 18.0] {
            match color_at(&builder, Vec2::new(x, 10.0)) {
                Some(color) if color == red => reds += 1,
                Some(color) if color == green => greens += 1,
                other => panic!("point x={x} uncovered or wrong color: {other:?}"),
            }
        }
        assert!(
            reds > 0 && greens > 0,
            "the whole shape must carry both stripe colors"
        );
    }

    #[test]
    fn each_pin_carries_its_own_border() {
        // у каждого пина своя рамка по его полному контуру — рамки идут и
        // внутри перекрытия, и пересекаются там: точка на краю квадрата A
        // (x=20) внутри перекрытия накрыта красной рамкой A, точка на краю
        // квадрата B (x=10) — зелёной рамкой B
        let red = LinearRgba::new(1.0, 0.0, 0.0, 0.3);
        let green = LinearRgba::new(0.0, 1.0, 0.0, 0.3);
        let builder = mesh_pins(
            &[
                PinnedFill {
                    shapes: vec![square(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0))],
                    color: red,
                },
                PinnedFill {
                    shapes: vec![square(Vec2::new(10.0, 0.0), Vec2::new(30.0, 20.0))],
                    color: green,
                },
            ],
            4.0,
            1.0,
        );
        let border_colors = |point: Vec2| {
            builder
                .colored_triangles_for_test()
                .filter_map(|(tri, color)| {
                    let sides = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])]
                        .map(|(from, to)| (to - from).perp_dot(point - from));
                    (sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0))
                        .then_some(color)
                })
                .filter(|color| color.alpha > 0.5)
                .collect::<Vec<_>>()
        };
        // рамка A на его правом краю: красная плотная внутри перекрытия
        let on_a_edge = border_colors(Vec2::new(20.0, 10.0));
        assert!(
            on_a_edge.iter().any(|c| c.red > 0.9 && c.green < 0.1),
            "pin A border must cross the overlap at x=20: {on_a_edge:?}"
        );
        // рамка B на его левом краю: зелёная плотная внутри перекрытия
        let on_b_edge = border_colors(Vec2::new(10.0, 10.0));
        assert!(
            on_b_edge.iter().any(|c| c.green > 0.9 && c.red < 0.1),
            "pin B border must cross the overlap at x=10: {on_b_edge:?}"
        );
        // глубина штриховки рамок не касается
        assert!(border_colors(Vec2::new(15.0, 10.0)).is_empty());
    }
}
