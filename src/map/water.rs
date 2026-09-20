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
//! Резка — одна на оба слоя ([`split_channels`]), и она же решает, кто из них
//! что рисует: кусок русла, отрезанный берегом с **обеих** сторон, — это не
//! русло по суше, а разрыв площадной воды (OSM обрывает `riverbank` у моста), и
//! его полоса уходит в площадную воду, а лентой не рисуется вовсе.
//!
//! Сетку это не трогает: навмеш глушит и полигон, и полосу русла целиком
//! (`Navmesh::fill_from_mapdata`), и то, что лента внутри полигона больше не
//! рисуется, проходимости не меняет.

use bevy::prelude::*;

use crate::map::grid::Grid;
use crate::map::meshing::{Break, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin, miter_offsets};
use crate::map::osm::model::{point_in_area, point_in_polygon, ring_bounds, signed_ring_area};
use crate::map::osm::{PolyArea, WaterLine, water_line_caps};
use crate::map::smooth::{Smoothing, smooth_path};

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
///
/// Вместе с полигонами в союз идут `gaps` — полосы русел, которыми река
/// продолжается там, где полигон оборван (`split_channels`): их служебные
/// рёбра «поперёк реки» тоже перестают быть берегом.
pub fn mesh_water_areas(areas: &[PolyArea], gaps: &[Vec<Vec2>]) -> MeshBuilder {
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
        .chain(gaps.iter().map(|gap| oriented(gap, true)))
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

/// Шаг сетки рёбер водных контуров, м. Контур реки — тысячи рёбер, и русло
/// спрашивает только те, что лежат в клетках его звена.
const CELL: f32 = 32.0;

/// Пересечения ближе этого вдоль звена — одно: общая вершина двух рёбер
/// контура даёт два попадания в одну точку.
const SAME_CROSSING: f32 = 1e-4;

/// Открытые русла, нарезанные по берегам площадной воды: что из них рисует
/// лента ([`mesh_water_lines`]), а что — сама площадная вода
/// ([`mesh_water_areas`]).
///
/// Резка одна на оба слоя не ради экономии: [`OpenChannels::gaps`] — те куски,
/// которые площадная вода обязана взять себе, а лента обязана не рисовать, и
/// считать их дважды значило бы завести два ответа на вопрос «где кончается
/// полигон».
pub struct OpenChannels {
    /// Контуры лент, закрывающих разрывы площадной воды: полосы шириной русла
    /// по его оси, с заходом в оба полигона. Идут в союз площадной воды, и
    /// лентой не рисуются.
    pub gaps: Vec<Vec<Vec2>>,
    drawn: Vec<DrawnRun>,
}

/// Кусок русла, который рисует лента: ось, ширина, торцы и то, какой из концов
/// отрезан берегом (на нём лента заходит в воду, и там гаснет её отмель).
struct DrawnRun {
    points: Vec<Vec2>,
    clipped: [bool; 2],
    width: f32,
    caps: [RibbonCap; 2],
}

/// Нарезать русла по берегам площадной воды.
///
/// **Кусок, отрезанный с обеих сторон, — не русло по суше, а разрыв площадной
/// воды.** OSM режет `riverbank` у моста (Тула, Упа под мостом на 6157, 3397:
/// `relation 19409693` обрывается перед мостом, `relation 19409692` начинается
/// за ним, между ними пятнадцать метров ничьей земли, по которой идёт осевая
/// `way 25857971`). Река там та же, и рисовать её лентой поверх полигонов
/// нельзя дважды: заход за берег клал прямоугольник глубокой воды на отмель
/// полигона, а сам полигон клал отмель вдоль служебного ребра «поперёк реки» —
/// светлую полосу там, где берега нет. Поэтому полоса такого куска уходит в
/// союз площадной воды: служебные рёбра оказываются внутри союза, отмель идёт
/// по настоящим берегам и заворачивает в русло, а лента этот кусок не рисует
/// вовсе. Заход за берег полоса сохраняет — им она и перекрывается с
/// полигонами, вплотную к ребру союз мог бы оставить щель.
///
/// Тула: один такой кусок на весь город. Остров посреди реки даёт тот же
/// ответ, и это верно — вода по обе стороны от него есть.
pub fn split_channels(lines: &[WaterLine], water: &[PolyArea]) -> OpenChannels {
    let index = WaterIndex::new(water);
    let mut channels = OpenChannels {
        gaps: Vec::new(),
        drawn: Vec::new(),
    };

    for line in lines.iter().filter(|line| !line.tunnel) {
        // сглаживание как у дорог: русло в OSM — ломаная по точкам съёмки, и на
        // её изломах лента без сглаживания заметно гранёная. Режется уже
        // сглаженная ось — та, по которой лента и ляжет
        let path = smooth_path(&line.points, line.width, Smoothing::Light);
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
        for run in index.open_runs(&path, WATER_SHORE_WIDTH) {
            if run.clipped == [true, true] {
                channels.gaps.push(band(&run.points, line.width));
            } else {
                channels.drawn.push(DrawnRun {
                    points: run.points,
                    clipped: run.clipped,
                    width: line.width,
                    caps,
                });
            }
        }
    }

    channels
}

/// Контур ленты: осевая, разведённая на полширины в обе стороны теми же
/// `miter_offsets`, по которым лента и рисуется. Торцы прямые — оба конца
/// такого куска лежат в глубине полигона.
fn band(path: &[Vec2], width: f32) -> Vec<Vec2> {
    let offsets = miter_offsets(path, false, width / 2.0);
    let left = path.iter().zip(&offsets).map(|(&at, &out)| at + out);
    let right = path.iter().zip(&offsets).rev().map(|(&at, &out)| at - out);
    left.chain(right).collect()
}

/// Лента открытых русел одним мешем. **Трубы не рисуются вовсе**: под землёй
/// воды не видно, а пунктир вдоль улицы читался как ручей поверх неё. Тем, что
/// человек проходит там, где на карте «ручей», управляет не эта отрисовка, а
/// её отсутствие: русло обрывается на портале культверта и продолжается за ним
/// (`water_line_caps`), и между порталами воды на карте просто нет.
pub fn mesh_water_lines(channels: &OpenChannels) -> MeshBuilder {
    let color = WATER_COLOR.to_linear();
    let mut open = MeshBuilder::with_surface_coords();

    for run in &channels.drawn {
        // отрезанный конец лежит в воде на ширину отмели глубже берега:
        // полудиск там ни к чему, а «до разрыва» от берега до торца идёт от
        // нуля к минус ширине отмели — по нему шейдер гасит кромки ленты
        let mut breaks = Vec::with_capacity(2);
        let mut run_caps = run.caps;
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
            run.width,
            color,
            RibbonJoin::Round,
            run_caps,
            RibbonBreaks::At(&breaks),
        );
    }

    open
}

/// Кусок оси русла вне площадной воды, уже с заходом за берег, и какие из двух
/// его концов отрезаны контуром (`[начало, конец]`), а не пришли из OSM.
#[derive(Debug)]
struct OpenRun {
    points: Vec<Vec2>,
    clipped: [bool; 2],
}

/// Площадная вода для резки русел: сами полигоны с их AABB и сетка рёбер всех
/// их колец.
struct WaterIndex<'a> {
    areas: &'a [PolyArea],
    bounds: Vec<(Vec2, Vec2)>,
    edges: Grid<(Vec2, Vec2)>,
}

impl<'a> WaterIndex<'a> {
    fn new(areas: &'a [PolyArea]) -> Self {
        let mut edges = Grid::new(CELL);
        for area in areas {
            for ring in std::iter::once(&area.outer).chain(&area.holes) {
                for (index, &from) in ring.iter().enumerate() {
                    let to = ring[(index + 1) % ring.len()];
                    // радиус здесь знает запрос, а не ребро — рамка не раздувается
                    edges.insert_segment(from, to, 0.0, (from, to));
                }
            }
        }
        Self {
            areas,
            bounds: areas.iter().map(|area| ring_bounds(&area.outer)).collect(),
            edges,
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        self.areas
            .iter()
            .zip(&self.bounds)
            .any(|(area, &(min, max))| {
                point.cmpge(min).all() && point.cmple(max).all() && point_in_area(point, area)
            })
    }

    /// Доли звена `from → to`, где оно пересекает ребро водного контура, по
    /// возрастанию и без повторов.
    fn crossings(&self, from: Vec2, to: Vec2) -> Vec<f32> {
        let (min, max) = (from.min(to), from.max(to));
        let span = to - from;
        let mut hits = Vec::new();
        // `near_each`, а не `near`: ребро — пара `Vec2`, сравнивать их нечем,
        // да и повторы здесь безразличны — доли всё равно сортируются и
        // склеиваются ниже
        for &(a, b) in self.edges.near_each(min, max) {
            let edge = b - a;
            let denominator = span.perp_dot(edge);
            if denominator == 0.0 {
                continue;
            }
            let t = (a - from).perp_dot(edge) / denominator;
            let u = (a - from).perp_dot(span) / denominator;
            // начало звена считается, конец — нет: берег, пришедший ровно в
            // вершину оси, обязан сбросить «внутри/снаружи» один раз, а не
            // ноль (он же конец предыдущего звена)
            if (0.0..1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                hits.push(t);
            }
        }
        hits.sort_by(f32::total_cmp);
        hits.dedup_by(|next, kept| *next - *kept < SAME_CROSSING);
        hits
    }

    /// Куски оси вне воды. Каждый отрезанный конец продолжен в воду на `reach`
    /// вдоль той же оси, а за её концом — по прямой последнего звена.
    ///
    /// Вода между двумя кусками, что уже `2 · reach`, не режет русло вовсе:
    /// заходы с обеих сторон всё равно легли бы друг на друга, а две ленты
    /// внахлёст с отмелью, гаснущей навстречу, дают шов посреди протоки.
    fn open_runs(&self, path: &[Vec2], reach: f32) -> Vec<OpenRun> {
        if path.len() < 2 {
            return Vec::new();
        }

        // ось с вершинами в каждой точке пересечения; у каждого звена между
        // соседними вершинами — внутри оно воды или снаружи. Внутри/снаружи
        // меняется только на пересечении, так что точку в полигоне спрашиваем
        // одну на промежуток между ними, а не на каждое звено
        let mut points = vec![path[0]];
        let mut inside = Vec::with_capacity(path.len());
        let mut state = None;
        for segment in path.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let hits = self.crossings(from, to);
            let stops = hits
                .iter()
                .map(|&t| (from.lerp(to, t), true))
                .chain(std::iter::once((to, false)));
            for (point, crossing) in stops {
                let last = points[points.len() - 1];
                if point != last {
                    let wet = *state.get_or_insert_with(|| self.contains(last.midpoint(point)));
                    inside.push(wet);
                    points.push(point);
                }
                if crossing {
                    state = None;
                }
            }
        }
        if inside.is_empty() {
            return Vec::new();
        }

        // узкая вода посреди русла — не разрез
        let lengths: Vec<f32> = points
            .windows(2)
            .map(|piece| piece[0].distance(piece[1]))
            .collect();
        let runs: Vec<(usize, usize)> = runs_of(&inside).collect();
        for (start, end) in runs {
            let bounded = start > 0 && end + 1 < inside.len();
            if inside[start] && bounded && lengths[start..=end].iter().sum::<f32>() < 2.0 * reach {
                inside[start..=end].fill(false);
            }
        }

        runs_of(&inside)
            .filter(|&(start, _)| !inside[start])
            .map(|(start, end)| {
                let last = points.len() - 1;
                let clipped = [start > 0, end + 1 < last];
                let mut run = Vec::new();
                if clipped[0] {
                    let mut back = walk(points[..=start].iter().rev().copied(), reach);
                    back.reverse();
                    run.extend(back);
                }
                run.extend_from_slice(&points[start..=end + 1]);
                if clipped[1] {
                    run.extend(walk(points[end + 1..].iter().copied(), reach));
                }
                OpenRun {
                    points: run,
                    clipped,
                }
            })
            .collect()
    }
}

/// Промежутки `[start, end]` подряд одинаковых значений.
fn runs_of(flags: &[bool]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = 0;
    (0..flags.len()).filter_map(move |index| {
        let closes = index + 1 == flags.len() || flags[index + 1] != flags[index];
        closes.then(|| {
            let run = (start, index);
            start = index + 1;
            run
        })
    })
}

/// Точки ломаной, пройденной на `reach` от её первой точки (сама первая не
/// входит), с последней ровно на `reach`. Ломаная кончилась раньше — дальше по
/// прямой её последнего звена.
fn walk(chain: impl Iterator<Item = Vec2>, reach: f32) -> Vec<Vec2> {
    let mut chain = chain;
    let mut out = Vec::new();
    let Some(mut previous) = chain.next() else {
        return out;
    };
    let mut left = reach;
    let mut direction = Vec2::ZERO;
    for next in chain {
        let length = previous.distance(next);
        if length <= 0.0 {
            continue;
        }
        direction = (next - previous) / length;
        if length >= left {
            out.push(previous + direction * left);
            return out;
        }
        out.push(next);
        left -= length;
        previous = next;
    }
    if direction != Vec2::ZERO {
        out.push(previous + direction * left);
    }
    out
}

#[cfg(test)]
mod tests;
