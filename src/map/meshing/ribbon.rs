//! Ленты — полосы вдоль ломаной: [`MeshBuilder::push_ribbon`] и её родня
//! (`push_stroke`, клин `push_taper`, рельсы `push_rails`, полилиния
//! `push_polyline`, кайма контура `push_inset_band*`), веера на изломах и
//! торцах, и геометрия пути под ними: miter-офсеты, склейка близких точек и
//! профиль разрывов разметки ([`GapProfile`]) с его разбором для слоя краски.

use std::f32::consts::PI;

use bevy::prelude::*;

use super::geometry::{
    ARC_TOLERANCE, arc_steps, arclengths, project_onto_path, ring_perimeter, signed_area,
};
use super::{Break, LaneFrame, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin};

/// Максимальное удлинение стыка ленты относительно полуширины. Контур кроны
/// полон почти встречных рёбер (впадины между фестонами), и там miter уходит
/// в длинный шип — при 1.5 стык вырождается в срез, шипов не видно.
const MITER_LIMIT: f32 = 1.5;

/// Кайма контура ([`MeshBuilder::push_inset_band`]) не шире этой доли его
/// толщины (`площадь / периметр`): у полосы толщина — половина ширины, и кайма
/// в 0.6 её с каждой стороны оставляет посреди полосы просвет заливки.
const RIM_THICKNESS_SHARE: f32 = 0.6;
/// Кайма тоньше не кладётся: не видна, а квадов на контур — столько же.
const MIN_RIM_WIDTH: f32 = 0.2;

/// «До разрыва» у ленты без единого разрыва: длина дуги плюс это. Разметке
/// нужна координата, растущая вдоль ленты (по ней идут штрихи), а гаснуть ей
/// негде — way продолжается с обоих концов.
pub(super) const FAR_FROM_BREAKS: f32 = 1000.0;

/// Координаты [`ATTRIBUTE_RIBBON`](super::ATTRIBUTE_RIBBON) для вершин веера.
#[derive(Clone, Copy)]
enum FanCoords {
    /// Стык: весь веер лежит на внешней стороне излома, «до разрыва» у него
    /// одно на все вершины — то же, что у точки пути.
    Join { side: f32, to_break: f32 },
    /// Торец: поперёк — проекция на `normal` ленты, «до разрыва» продолжается
    /// за точку пути линейно с наклоном `slope` последнего квада: в минус у
    /// тупика и перекрёстка (разметка на полудиске гаснет), в плюс там, где
    /// way продолжается следующим и линия идёт сквозь стык.
    Cap {
        outward: Vec2,
        normal: Vec2,
        at_end: f32,
        slope: f32,
    },
}

impl MeshBuilder {
    /// Полилиния как цепочка квадов; каждый конец сегмента продлён на
    /// полширины, чтобы стыки перекрывались (как у старых дорог-спрайтов).
    pub fn push_polyline(&mut self, points: &[Vec2], width: f32, color: LinearRgba) {
        let half_width = width / 2.0;
        let (along, total) = arclengths(points, false);
        for (index, segment) in points.windows(2).enumerate() {
            let Some(direction) = (segment[1] - segment[0]).try_normalize() else {
                continue;
            };
            let extension = direction * half_width;
            let normal = direction.perp() * half_width;
            let from = segment[0] - extension;
            let to = segment[1] + extension;
            // «до торца» — по продлённым концам; излом этой функции на
            // середине пути тут может попасть внутрь квада, но режим оставлен
            // ради сравнения картинок, а не ради разметки
            let at_from = to_nearest_end(along[index] - half_width, total);
            let at_to = to_nearest_end(along[index + 1] + half_width, total);
            self.push_quad_full(
                [from + normal, from - normal, to - normal, to + normal],
                [color; 4],
                [
                    self.coords(half_width, at_from, half_width),
                    self.coords(-half_width, at_from, half_width),
                    self.coords(-half_width, at_to, half_width),
                    self.coords(half_width, at_to, half_width),
                ],
            );
        }
    }

    /// **Клин** — разомкнутая лента, ширина которой вдоль ломаной меняется
    /// линейно по длине дуги от `widths[0]` до `widths[1]`: переход между
    /// сечениями улицы (`roads::network::sections`), где до этого ширина
    /// менялась ступенькой.
    ///
    /// Торцы срезаны ровно по крайним точкам и поперёк исходных крайних звеньев
    /// ([`merge_ribbon_points`], `butt_normal`) — так же, как прямой торец тела
    /// улицы за швом, и щели между ними нет; изломы — общими вершинами по
    /// биссектрисе ([`miter_offsets`]), как у ленты без веера: клин короток, а
    /// круглый торец выпирал бы из-под более узкой соседней ленты. Координаты
    /// фактуры: раскладка полос — по доле пути между двумя из
    /// [`Self::set_lane_taper`], так что колея плывёт вместе с краями;
    /// «до разрыва» — линейно от `to_break[0]` до `to_break[1]`, продолжением
    /// срезанной ленты ([`to_break_beyond`]), чтобы колея гасла у того же узла.
    pub fn push_taper(
        &mut self,
        points: &[Vec2],
        widths: [f32; 2],
        to_break: [f32; 2],
        color: LinearRgba,
    ) {
        let halves = widths.map(|width| width / 2.0);
        self.push_taper_sided(points, [halves, halves], to_break, color);
    }

    /// [`Self::push_taper`] с кромками по сторонам: `halves[сторона]` —
    /// полуширина `[в начале, в конце]` слева (`0`) и справа (`1`) по ходу
    /// ломаной. Так сужается одна кромка, пока другая идёт прямо, — клин у
    /// примыкания (`roads/tapers.rs`); симметричный клин — частный случай.
    pub fn push_taper_sided(
        &mut self,
        points: &[Vec2],
        halves: [[f32; 2]; 2],
        to_break: [f32; 2],
        color: LinearRgba,
    ) {
        let narrowest = halves
            .iter()
            .flatten()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let path = merge_ribbon_points(points, false, narrowest / 2.0);
        if path.len() < 2 {
            return;
        }
        let (along, total) = arclengths(&path, false);
        if total <= 0.0 {
            return;
        }
        let mut miters = miter_offsets(&path, false, 1.0);
        for (end, index) in [(false, 0), (true, path.len() - 1)] {
            if let Some(normal) = butt_normal(points, end) {
                miters[index] = normal;
            }
        }
        let rgba = color.to_f32_array();
        let base = self.positions.len() as u32;
        for ((&point, &at), miter) in path.iter().zip(&along).zip(&miters) {
            let share = at / total;
            let [left, right] = halves.map(|[from, to]| from + (to - from) * share);
            let to_break = to_break[0] + (to_break[1] - to_break[0]) * share;
            // раскладка полос плывёт вместе с краями: крайняя полоса
            // рождается из клина, а колея остаётся между линиями краски
            let lanes = match self.lanes {
                [Some(from), Some(to)] => Some(from.lerp(to, share)),
                [from, _] => from,
            };
            self.push_vertex(
                point + *miter * left,
                rgba,
                Self::coords_in(lanes, left, to_break, left),
            );
            self.push_vertex(
                point - *miter * right,
                rgba,
                Self::coords_in(lanes, -right, to_break, right),
            );
        }
        for index in 0..path.len() as u32 - 1 {
            let (left, right) = (base + 2 * index, base + 2 * index + 1);
            let (next_left, next_right) = (left + 2, right + 2);
            self.indices
                .extend([left, right, next_right, left, next_right, next_left]);
        }
    }

    /// Лента постоянной ширины вдоль ломаной со стыками по биссектрисе
    /// (miter с ограничением `MITER_LIMIT`) и торцами по последней точке.
    /// Для тонких контуров `push_polyline` не годится — там каждый сегмент
    /// продлён на полширины, и на ломаной с сегментами короче ширины штриха
    /// (контур кроны) продления соседних квадов торчат наружу шипами.
    pub fn push_stroke(&mut self, points: &[Vec2], closed: bool, width: f32, color: LinearRgba) {
        self.push_ribbon(
            points,
            closed,
            width,
            color,
            RibbonJoin::Miter,
            RibbonCap::Butt,
        );
    }

    /// Две нитки рельсов вдоль ломаной: ленты ширины `width` по обе стороны
    /// осевой, на полколеи от неё. Смещение — тот же miter-офсет, которым
    /// считается край ленты ([`miter_offsets`]), поэтому нитка повторяет
    /// изгиб пути без щелей на изломах и на самом изломе держит колею, а не
    /// уезжает наружу вместе с углом.
    ///
    /// С балластом у нитки общая осевая, но своя склейка близких точек:
    /// `gauge / 4` здесь против `width / 4` у ленты ([`Self::push_ribbon`]),
    /// так что на очень частой ломаной балласт срезает угол хордой там, где
    /// нитка ещё идёт по точкам. Расхождение ограничено порогом склейки
    /// ленты — на радиусах реальных кривых это сантиметры.
    ///
    /// Торцы — `Butt`: нитка кончается там же, где way, а полудиск на
    /// сантиметровой ленте не виден и стоит лишнего веера.
    pub fn push_rails(
        &mut self,
        points: &[Vec2],
        gauge: f32,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
    ) {
        if gauge <= 0.0 || width <= 0.0 {
            return;
        }
        // склейка по колее, а не по ширине нитки: нитка тоньше сантиметров, и
        // по её мерке в путь прошли бы точки, вырождающие нормаль офсета
        let path = merge_close_points(points, false, gauge / 4.0);
        if path.len() < 2 {
            return;
        }

        let offsets = miter_offsets(&path, false, gauge / 2.0);
        let mut line = Vec::with_capacity(path.len());
        for side in [1.0_f32, -1.0] {
            line.clear();
            line.extend(
                path.iter()
                    .zip(&offsets)
                    .map(|(point, offset)| *point + *offset * side),
            );
            self.push_ribbon(&line, false, width, color, join, RibbonCap::Butt);
        }
    }

    /// Лента постоянной ширины вдоль ломаной: `join` — чем закрыт излом,
    /// `cap` — чем закрыты торцы разомкнутой ленты.
    ///
    /// Точки ближе `width / 4` к предыдущей отбрасываются: на такой дистанции
    /// они не видны, но вырождают нормаль стыка.
    pub fn push_ribbon(
        &mut self,
        points: &[Vec2],
        closed: bool,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
        cap: RibbonCap,
    ) {
        self.push_ribbon_capped(points, closed, width, color, join, [cap; 2]);
    }

    /// То же, но торцы задаются по отдельности — `[начало, конец]`. Русло,
    /// которому это нужно (открытый конец против входа в трубу), кладётся уже
    /// через [`Self::push_ribbon_broken`]: отмели на его кромках нужна своя
    /// координата «до разрыва» (`water::mesh_water_lines`).
    pub fn push_ribbon_capped(
        &mut self,
        points: &[Vec2],
        closed: bool,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
        caps: [RibbonCap; 2],
    ) {
        self.push_ribbon_shaped(
            points,
            width,
            color,
            RibbonShape {
                closed,
                join,
                caps,
                breaks: RibbonBreaks::Ends,
            },
        );
    }

    /// Лента заданной [`RibbonShape`] — вход для тех, кому мало `join`+`cap`:
    /// проезжей части улицы с её разрывами разметки ([`RibbonBreaks`]) и
    /// кольца, которое рисуется замкнутым. «До разрыва» в [`ATTRIBUTE_RIBBON`]
    /// считается до края ближайшего разрыва, а не до торца, и у замкнутой
    /// ленты — **по кругу**.
    pub fn push_ribbon_shaped(
        &mut self,
        points: &[Vec2],
        width: f32,
        color: LinearRgba,
        shape: RibbonShape,
    ) {
        let RibbonShape {
            closed,
            join,
            caps,
            breaks,
        } = shape;
        let mut path = merge_ribbon_points(points, closed, ribbon_merge_distance(width));
        if path.len() < 2 {
            return;
        }

        let half_width = width / 2.0;
        // прямой торец — поперёк исходного крайнего звена (`butt_normal`)
        let butts = [false, true].map(|end| {
            (!closed && caps[usize::from(end)] == RibbonCap::Butt)
                .then(|| butt_normal(points, end))
                .flatten()
                .map(|normal| normal * half_width)
        });
        let (mut along, total) = arclengths(&path, closed);
        // «до разрыва» — по ней шейдер гасит разметку у перекрёстка и
        // фазирует штрихи; у замкнутой ленты разрывов нет, остаётся длина дуги
        let gaps = GapProfile::new(&path, &along, total, breaks, closed.then_some(total));
        if self.ribbon.is_some() {
            gaps.split_path(&mut path, &mut along, width / 4.0);
            // и на каждой паре профиля раскладки: между ними GPU тянет её по
            // прямой, как профиль
            if !closed {
                for &(at, _) in &self.lane_profile {
                    insert_vertex_at(&mut path, &mut along, at, PROFILE_MERGE);
                }
            }
        }
        let ends: Vec<f32> = along.iter().map(|&at| gaps.distance(at)).collect();
        let frames: Vec<Option<LaneFrame>> = along.iter().map(|&at| self.lanes_at(at)).collect();

        let count = path.len();
        let segments = if closed { count } else { count - 1 };

        match join {
            RibbonJoin::Miter => {
                let mut offsets = miter_offsets(&path, closed, half_width);
                for (butt, index) in butts.into_iter().zip([0, count - 1]) {
                    if let Some(butt) = butt {
                        offsets[index] = butt;
                    }
                }

                for index in 0..segments {
                    let next = (index + 1) % count;
                    self.push_quad_full(
                        [
                            path[index] + offsets[index],
                            path[index] - offsets[index],
                            path[next] - offsets[next],
                            path[next] + offsets[next],
                        ],
                        [color; 4],
                        Self::segment_coords(
                            half_width,
                            [frames[index], frames[next]],
                            ends[index],
                            ends[next],
                        ),
                    );
                }
            }
            RibbonJoin::Round => {
                // Сегменты — квады без продлений; на внутренней стороне
                // излома они перекрываются сами, снаружи щель закрывает веер.
                // Излом, которому веер не положен (щель мельче допуска), квады
                // проходят **общими вершинами** по биссектрисе: торцы со своими
                // нормалями там не совпадают ни в одной точке, кроме осевой, и
                // растеризатор оставлял между ними волосяную щель поперёк всей
                // ленты — светлую линию тротуара под проездом.
                // направления до и после каждой вершины: `None` у концов
                // открытой ленты и у вершины с вырожденным звеном
                let bends: Vec<Option<(Vec2, Vec2)>> = (0..count)
                    .map(|index| {
                        if !closed && (index == 0 || index + 1 == count) {
                            return None;
                        }
                        let previous = path[(index + count - 1) % count];
                        let next = path[(index + 1) % count];
                        Some((
                            (path[index] - previous).try_normalize()?,
                            (next - path[index]).try_normalize()?,
                        ))
                    })
                    .collect();
                let miters = miter_offsets(&path, closed, half_width);
                let shared: Vec<bool> = bends
                    .iter()
                    .map(|bend| {
                        bend.is_some_and(|(incoming, outgoing)| {
                            join_gap_hidden(half_width, incoming.angle_to(outgoing))
                        })
                    })
                    .collect();
                for index in 0..segments {
                    let next = (index + 1) % count;
                    let Some(direction) = (path[next] - path[index]).try_normalize() else {
                        continue;
                    };
                    let normal = direction.perp() * half_width;
                    let at_start = match (index, butts[0]) {
                        (0, Some(butt)) => butt,
                        _ if shared[index] => miters[index],
                        _ => normal,
                    };
                    let at_end = match butts[1] {
                        Some(butt) if next == count - 1 => butt,
                        _ if shared[next] => miters[next],
                        _ => normal,
                    };
                    self.push_quad_full(
                        [
                            path[index] + at_start,
                            path[index] - at_start,
                            path[next] - at_end,
                            path[next] + at_end,
                        ],
                        [color; 4],
                        Self::segment_coords(
                            half_width,
                            [frames[index], frames[next]],
                            ends[index],
                            ends[next],
                        ),
                    );
                }
                for (index, bend) in bends.iter().enumerate() {
                    let Some((incoming, outgoing)) = *bend else {
                        continue;
                    };
                    self.push_join_fan(
                        path[index],
                        half_width,
                        incoming,
                        outgoing,
                        color,
                        (ends[index], frames[index]),
                    );
                }
            }
        }

        if !closed {
            // полудиск за начальной точкой: от нормали через −direction,
            // то есть назад по ходу пути
            if caps[0] == RibbonCap::Round
                && let Some(direction) = (path[1] - path[0]).try_normalize()
            {
                self.push_arc_fan(
                    path[0],
                    half_width,
                    direction.perp().to_angle(),
                    PI,
                    color,
                    FanCoords::Cap {
                        outward: -direction,
                        normal: direction.perp(),
                        at_end: ends[0],
                        slope: slope_between(ends[0], ends[1], along[1] - along[0]),
                    },
                    frames[0],
                );
            }
            if caps[1] == RibbonCap::Round
                && let Some(direction) = (path[count - 1] - path[count - 2]).try_normalize()
            {
                self.push_arc_fan(
                    path[count - 1],
                    half_width,
                    (-direction.perp()).to_angle(),
                    PI,
                    color,
                    FanCoords::Cap {
                        outward: direction,
                        normal: direction.perp(),
                        at_end: ends[count - 1],
                        slope: slope_between(
                            ends[count - 1],
                            ends[count - 2],
                            along[count - 1] - along[count - 2],
                        ),
                    },
                    frames[count - 1],
                );
            }
        }
    }

    /// Координаты четырёх углов квада сегмента: `+нормаль` в начале, `−нормаль`
    /// в начале, `−нормаль` в конце, `+нормаль` в конце — порядок
    /// [`Self::push_quad_full`]; `lanes` — раскладка в начале и в конце.
    fn segment_coords(
        half_width: f32,
        [lanes_start, lanes_end]: [Option<LaneFrame>; 2],
        at_start: f32,
        at_end: f32,
    ) -> [[f32; 4]; 4] {
        [
            Self::coords_in(lanes_start, half_width, at_start, half_width),
            Self::coords_in(lanes_start, -half_width, at_start, half_width),
            Self::coords_in(lanes_end, -half_width, at_end, half_width),
            Self::coords_in(lanes_end, half_width, at_end, half_width),
        ]
    }

    /// Веер, закрывающий щель butt-квадов на **внешней** стороне излома.
    /// При левом повороте (`angle_to > 0`) щель справа по ходу, и наоборот.
    ///
    /// Порог пропуска — по **ширине щели** (`радиус · излом`), а не по углу:
    /// один и тот же излом в 5° у аллеи оставляет 15 см, и на приближении это
    /// хорошо видимая светлая прорезь поперёк дороги. Тот же допуск, что и на
    /// стрелку хорды дуги, — то, чего не видно, одинаково не видно и там, и
    /// тут; изломы медианной для Тулы крутизны (3.4°) веер всё равно получают.
    fn push_join_fan(
        &mut self,
        center: Vec2,
        radius: f32,
        incoming: Vec2,
        outgoing: Vec2,
        color: LinearRgba,
        (to_break, lanes): (f32, Option<LaneFrame>),
    ) {
        let turn = incoming.angle_to(outgoing);
        if join_gap_hidden(radius, turn) {
            return;
        }
        let side = -turn.signum();
        // угол нормали растёт вместе с углом направления, поэтому от нормали
        // входящего сегмента до нормали исходящего ровно `turn` радиан
        let start = (incoming.perp() * side).to_angle();
        self.push_arc_fan(
            center,
            radius,
            start,
            turn,
            color,
            FanCoords::Join { side, to_break },
            lanes,
        );
    }

    /// Веер треугольников по дуге: `sweep` радиан от `start` вокруг `center`,
    /// в раскладке `lanes`.
    #[allow(clippy::too_many_arguments)]
    fn push_arc_fan(
        &mut self,
        center: Vec2,
        radius: f32,
        start: f32,
        sweep: f32,
        color: LinearRgba,
        coords: FanCoords,
        lanes: Option<LaneFrame>,
    ) {
        let steps = arc_steps(radius, sweep.abs());
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        let coords_at =
            |across: f32, to_break: f32| Self::coords_in(lanes, across, to_break, radius);
        let at_center = match coords {
            FanCoords::Join { to_break, .. } => coords_at(0.0, to_break),
            FanCoords::Cap { at_end, .. } => coords_at(0.0, at_end),
        };
        self.push_vertex(center, rgba, at_center);
        for step in 0..=steps {
            let angle = start + sweep * step as f32 / steps as f32;
            let point = center + Vec2::from_angle(angle) * radius;
            let at_rim = match coords {
                FanCoords::Join { side, to_break } => coords_at(side * radius, to_break),
                FanCoords::Cap {
                    outward,
                    normal,
                    at_end,
                    slope,
                } => {
                    let offset = point - center;
                    coords_at(offset.dot(normal), at_end + slope * offset.dot(outward))
                }
            };
            self.push_vertex(point, rgba, at_rim);
        }
        for step in 0..steps as u32 {
            self.indices
                .extend([base, base + 1 + step, base + 2 + step]);
        }
    }

    /// Кайма вдоль замкнутого контура: полоса ширины `width` от контура вглубь
    /// него (при `outside` — наружу, так каймится дырка), цвет от `edge` на
    /// контуре до `inner` на дальнем краю. Мелководье у берега, тёмная кромка
    /// луга. Дальний край строится miter-офсетами, поэтому на острых вогнутых
    /// углах соседние квады накладываются — цвет там один и тот же, и наложение
    /// не видно.
    ///
    /// Ширина зажата толщиной самого контура (`площадь / периметр` — у полосы
    /// это половина её ширины): у газона-разделителя в полтора метра кайма в два
    /// метра вылезла бы за дальний край на дорогу. Тоньше 20 см кайма не кладётся.
    /// Для `outside` толщина дырки ни при чём — ширину зажимает вызывающий по
    /// внешнему контуру. Возвращает положенную ширину, `None` — кайма не легла.
    pub fn push_inset_band(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        edge: LinearRgba,
        inner: LinearRgba,
    ) -> Option<f32> {
        self.push_inset_band_with(ring, width, outside, |_, _| (edge, inner))
    }

    /// Та же кайма, но цвет решается на каждое ребро контура `(a, b)`: одной
    /// парой цветов на весь контур не сказать, что грань, повёрнутая к солнцу,
    /// светлее отвёрнутой. Слои зданий этим сейчас не пользуются — так был
    /// сделан парапет кровли, которого больше нет (`layers.rs`).
    pub fn push_inset_band_with(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        color: impl Fn(Vec2, Vec2) -> (LinearRgba, LinearRgba),
    ) -> Option<f32> {
        self.push_band(ring, width, outside, |_| 1.0, color)
    }

    /// Та же кайма, но ширина решается на каждой вершине — по единичному
    /// направлению, в котором кайма от неё пойдёт. Полутень тени растёт с
    /// расстоянием от того, кто её отбрасывает: у самой стены край жёсткий,
    /// вдали — размытый, и одной ширины на весь контур этого не сказать
    /// ([`crate::map::buildings::shadows::shadow_builder`]).
    ///
    /// `taper` возвращает долю `width` (вне `0..=1` зажимается). Схлопнутая в
    /// ноль вершина вырождает свой квад в треугольник — это и есть жёсткий
    /// край; ребро, у которого схлопнуты обе, не кладётся вовсе.
    pub fn push_inset_band_tapered(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        taper: impl Fn(Vec2) -> f32,
        edge: LinearRgba,
        inner: LinearRgba,
    ) -> Option<f32> {
        self.push_band(ring, width, outside, taper, |_, _| (edge, inner))
    }

    fn push_band(
        &mut self,
        ring: &[Vec2],
        width: f32,
        outside: bool,
        taper: impl Fn(Vec2) -> f32,
        color: impl Fn(Vec2, Vec2) -> (LinearRgba, LinearRgba),
    ) -> Option<f32> {
        let path = merge_close_points(ring, true, width / 4.0);
        if path.len() < 3 {
            return None;
        }
        let area = signed_area(&path);
        let width = if outside {
            width
        } else {
            width.min(RIM_THICKNESS_SHARE * area.abs() / ring_perimeter(&path))
        };
        if width < MIN_RIM_WIDTH {
            return None;
        }
        // у обхода против часовой стрелки внутренняя сторона слева — куда и
        // смотрят miter-офсеты; по часовой — справа
        let side = if (area > 0.0) != outside { 1.0 } else { -1.0 };
        // офсеты единичной ширины, чтобы `taper` домножал уже готовое
        // направление: множитель длину меняет, направление — нет. Сторона
        // тоже вносится здесь, на месте — лишний `Vec` на кольцо стоил бы
        // дороже самого сужения
        let mut offsets = miter_offsets(&path, true, 1.0);
        for offset in &mut offsets {
            let direction = *offset * side;
            *offset = direction * width * taper(direction.normalize_or_zero()).clamp(0.0, 1.0);
        }
        let count = path.len();
        for index in 0..count {
            let next = (index + 1) % count;
            if offsets[index] == Vec2::ZERO && offsets[next] == Vec2::ZERO {
                continue;
            }
            let (edge, inner) = color(path[index], path[next]);
            self.push_quad_gradient(
                [
                    path[index],
                    path[index] + offsets[index],
                    path[next] + offsets[next],
                    path[next],
                ],
                [edge, inner, inner, edge],
            );
        }
        Some(width)
    }
}

/// «До разрыва» разомкнутой ленты [`MeshBuilder::push_ribbon_shaped`] с
/// разрывами `breaks` на её **продолжении** — `beyond` метров за началом
/// (`end = false`) или концом пути. Нужна клину (`MeshBuilder::push_taper`),
/// который продолжает срезанную ленту: с тем же значением на стыке штрихи
/// разметки идут через стык без сдвига фазы.
pub fn to_break_beyond(
    points: &[Vec2],
    width: f32,
    breaks: &[Break],
    end: bool,
    beyond: f32,
) -> f32 {
    let path = merge_ribbon_points(points, false, ribbon_merge_distance(width));
    if path.len() < 2 {
        return FAR_FROM_BREAKS;
    }
    let (along, total) = arclengths(&path, false);
    let gaps = GapProfile::new(&path, &along, total, RibbonBreaks::At(breaks), None);
    gaps.distance(if end { total + beyond } else { -beyond })
}

/// Путь ленты с вершинами на изломах «до разрыва», длина дуги и «до разрыва»
/// в каждой его вершине — то, что [`MeshBuilder::push_ribbon_shaped`] кладёт в
/// атрибут проезжей части, для слоя краски (`roads/paint.rs`): тот строит линии
/// поверх ленты сам и обязан гаснуть у тех же узлов. Точки ближе
/// `merge_distance` сливаются, как у ленты. Путь короче двух точек — пустые
/// списки.
pub fn break_profile(
    points: &[Vec2],
    closed: bool,
    breaks: &[Break],
    merge_distance: f32,
) -> (Vec<Vec2>, Vec<f32>, Vec<f32>) {
    let mut path = merge_ribbon_points(points, closed, merge_distance);
    if path.len() < 2 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let (mut along, total) = arclengths(&path, closed);
    let gaps = GapProfile::new(
        &path,
        &along,
        total,
        RibbonBreaks::At(breaks),
        closed.then_some(total),
    );
    gaps.split_path(&mut path, &mut along, merge_distance);
    let to_break = along.iter().map(|&at| gaps.distance(at)).collect();
    (path, along, to_break)
}

/// «До разрыва» в вершинах уже нарезанного пути `path` — по другому набору
/// разрывов, чем тот, по которому его резал [`break_profile`]. Изломы новых
/// разрывов должны быть среди вершин пути: набор — подмножество того, по
/// которому путь резался.
pub fn break_distances(path: &[Vec2], closed: bool, breaks: &[Break]) -> Vec<f32> {
    let (along, total) = arclengths(path, closed);
    let gaps = GapProfile::new(
        path,
        &along,
        total,
        RibbonBreaks::At(breaks),
        closed.then_some(total),
    );
    along.iter().map(|&at| gaps.distance(at)).collect()
}

/// Расстояние до ближайшего торца разомкнутого пути.
fn to_nearest_end(along: f32, total: f32) -> f32 {
    along.min(total - along)
}

/// Наклон «до разрыва» на последнем кваде — чтобы продолжить её за торец.
/// Вырожденный квад (нулевой длины) наклона не имеет: за торцом тогда
/// как у тупика, в минус.
fn slope_between(at_end: f32, at_neighbour: f32, distance: f32) -> f32 {
    if distance > 0.0 {
        (at_end - at_neighbour) / distance
    } else {
        -1.0
    }
}

/// Форма ленты: замкнута ли, чем закрыты изломы и торцы, где разрывы разметки.
///
/// У замкнутой (`closed`) торцов нет вовсе — ни полудисков, ни продлений:
/// последняя точка пути повторяет первую, [`merge_close_points`] её снимает, а
/// шов получает такой же веер излома, как всякая другая вершина.
pub struct RibbonShape<'a> {
    pub closed: bool,
    pub join: RibbonJoin,
    pub caps: [RibbonCap; 2],
    pub breaks: RibbonBreaks<'a>,
}

/// Разрывы разметки одной ленты в координатах длины дуги — отсортированы и не
/// пересекаются, так что между двумя соседними «до разрыва» — функция с одним
/// изломом.
struct GapProfile {
    /// `(центр, полудлина)` каждого разрыва.
    gaps: Vec<(f32, f32)>,
    /// Длина замкнутой ленты: по ней «до разрыва» меряется **по кругу**, в обе
    /// стороны от точки. `None` у разомкнутой.
    period: Option<f32>,
}

impl GapProfile {
    fn new(
        path: &[Vec2],
        along: &[f32],
        total: f32,
        breaks: RibbonBreaks,
        period: Option<f32>,
    ) -> Self {
        let mut gaps: Vec<(f32, f32)> = match breaks {
            // у кольца торцов нет: шов — не разрыв, и гасить у него разметку
            // значило бы провести поперёк кольца черту на пустом месте
            RibbonBreaks::Ends if period.is_some() => Vec::new(),
            RibbonBreaks::Ends => vec![(0.0, 0.0), (total, 0.0)],
            RibbonBreaks::At(list) => list
                .iter()
                .map(|gap| {
                    (
                        project_onto_path(path, along, gap.at, period.is_some()),
                        gap.reach,
                    )
                })
                .collect(),
        };
        gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
        // пересекающиеся разрывы — в один: иначе между ними ближайший менялся
        // бы не там, где считает `kinks`
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(gaps.len());
        for (center, reach) in gaps {
            match merged.last_mut() {
                Some(last) if center - reach <= last.0 + last.1 => {
                    let start = (last.0 - last.1).min(center - reach);
                    let end = (last.0 + last.1).max(center + reach);
                    *last = ((start + end) / 2.0, (end - start) / 2.0);
                }
                _ => merged.push((center, reach)),
            }
        }
        Self {
            gaps: merged,
            period,
        }
    }

    /// Дуга от точки до центра разрыва — у замкнутой ленты **короткая из
    /// двух**: по кольцу к разрыву идут в обе стороны.
    fn span(&self, at: f32, center: f32) -> f32 {
        let gap = (at - center).abs();
        match self.period {
            Some(period) => gap.min(period - gap),
            None => gap,
        }
    }

    /// Знаковое расстояние до края ближайшего разрыва: внутри разрыва
    /// отрицательно. Без разрывов — длина дуги за [`FAR_FROM_BREAKS`]: штрихам
    /// нужна растущая координата. У замкнутой ленты без разрывов она на шве
    /// прыгает на всю длину кольца, то есть один штрих там не совпадёт фазой,
    /// — но размеченного кольца не бывает: `roads::lane_count` даёт кольцу
    /// одну полосу, а однополосной разметки нет.
    fn distance(&self, at: f32) -> f32 {
        if self.gaps.is_empty() {
            return at + FAR_FROM_BREAKS;
        }
        self.gaps
            .iter()
            .map(|&(center, reach)| self.span(at, center) - reach)
            .fold(f32::INFINITY, f32::min)
    }

    /// Изломы функции «до разрыва»: центр каждого разрыва и точка между
    /// соседними, где ближайший из них меняется. У замкнутой ленты соседи
    /// считаются по кругу, так что последний разрыв граничит с первым **через
    /// шов**; единственный разрыв кольца граничит сам с собой в противоположной
    /// точке, где расстояние перестаёт расти, — квад, накрывший её без вершины,
    /// увёл бы разметку за разрыв, которого там нет.
    fn kinks(&self) -> Vec<f32> {
        let mut kinks = Vec::with_capacity(self.gaps.len() * 2);
        for (index, &(center, reach)) in self.gaps.iter().enumerate() {
            kinks.push(center);
            if let Some(&(next_center, next_reach)) = self.gaps.get(index + 1) {
                kinks.push((center + next_center + reach - next_reach) / 2.0);
            }
        }
        if let (Some(period), Some(&(first, first_reach)), Some(&(last, last_reach))) =
            (self.period, self.gaps.first(), self.gaps.last())
        {
            let crossover = (last + first + period + last_reach - first_reach) / 2.0;
            kinks.push(crossover.rem_euclid(period));
        }
        kinks
    }

    /// Вершина на каждом изломе, чтобы «до разрыва» была линейной внутри
    /// каждого квада: GPU интерполирует атрибут по прямой, и квад, накрывший
    /// излом, получил бы расстояние, растущее там, где оно убывает. Излом ближе
    /// `merge_distance` к соседней вершине не вставляется: внутри такого
    /// коротышки его не видно, а вырожденный квад — да.
    fn split_path(&self, path: &mut Vec<Vec2>, along: &mut Vec<f32>, merge_distance: f32) {
        for kink in self.kinks() {
            if insert_vertex_at(path, along, kink, merge_distance) {
                continue;
            }
            // излом на замыкающем звене: вершин по обе стороны от него нет,
            // оно идёт от последней точки к первой — такая вершина
            // дописывается в хвост
            if let Some(period) = self.period {
                append_vertex_at(path, along, kink, merge_distance, period);
            }
        }
    }
}

/// Пара профиля раскладки ближе этого к вершине ленты вершины не получает, м:
/// пары идут через пару метров, и сдвиг на десять сантиметров незаметен.
const PROFILE_MERGE: f32 = 0.1;

/// Вершина на длине дуги `at`, если та внутри какого-то сегмента и не ближе
/// `merge_distance` к его концам. Отвечает, нашёлся ли такой сегмент, — чтобы
/// звонящий знал, что остаётся замыкающее звено кольца ([`append_vertex_at`]).
fn insert_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
) -> bool {
    let Some(index) = along
        .windows(2)
        .position(|pair| pair[0] < at && at < pair[1])
    else {
        return false;
    };
    if at - along[index] <= merge_distance || along[index + 1] - at <= merge_distance {
        return true;
    }
    let t = (at - along[index]) / (along[index + 1] - along[index]);
    let point = path[index].lerp(path[index + 1], t);
    path.insert(index + 1, point);
    along.insert(index + 1, at);
    true
}

/// То же на **замыкающем** звене кольца — от последней точки к первой: вершина
/// там не вставляется между соседями, а дописывается в хвост.
fn append_vertex_at(
    path: &mut Vec<Vec2>,
    along: &mut Vec<f32>,
    at: f32,
    merge_distance: f32,
    period: f32,
) {
    let (Some(&last), Some(&first)) = (along.last(), path.first()) else {
        return;
    };
    if at <= last || at >= period {
        return;
    }
    if at - last <= merge_distance || period - at <= merge_distance {
        return;
    }
    let t = (at - last) / (period - last);
    let tail = path[path.len() - 1];
    path.push(tail.lerp(first, t));
    along.push(at);
}

/// Miter-офсет вершины ломаной: вектор от точки пути до края ленты полуширины
/// `half_width`, по биссектрисе излома, с ограничением [`MITER_LIMIT`] на
/// острых стыках. Край ленты — `path[i] ± offsets[i]`. Общий и для отрисовки
/// ([`MeshBuilder::push_ribbon`]), и для навмеша (бордюры моста): полосы,
/// заблокированные в сетке, совпадают с нарисованными по построению.
pub fn miter_offsets(path: &[Vec2], closed: bool, half_width: f32) -> Vec<Vec2> {
    let count = path.len();
    (0..count)
        .map(|index| {
            let incoming = (index > 0 || closed).then(|| {
                let previous = path[(index + count - 1) % count];
                (path[index] - previous).normalize_or(Vec2::X).perp()
            });
            let outgoing = (index + 1 < count || closed).then(|| {
                let next = path[(index + 1) % count];
                (next - path[index]).normalize_or(Vec2::X).perp()
            });
            match (incoming, outgoing) {
                (Some(before), Some(after)) => {
                    let bisector = (before + after).normalize_or(before);
                    // на острых стыках длина miter уходит в бесконечность — режем
                    let cosine = bisector.dot(before).max(1.0 / MITER_LIMIT);
                    bisector * (half_width / cosine)
                }
                (Some(normal), None) | (None, Some(normal)) => normal * half_width,
                (None, None) => Vec2::ZERO,
            }
        })
        .collect()
}

/// Ломаная без точек ближе `merge_distance` к предыдущей: на такой дистанции
/// они не видны, но вырождают нормаль стыка. У замкнутой ленты так же
/// подрезается хвост, сошедшийся с началом.
///
/// Эпсилон выбирает потребитель, и они разные: рендеру важно «не видно»
/// (`width / 4`, [`MeshBuilder::push_ribbon`]), полигональному навмешу — только
/// вырожденность (`polymesh::build::DEGENERATE_SPAN`), потому что схлопывание
/// по ширине сдвинуло бы его футпринт относительно заливки сетки.
pub fn merge_close_points(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<Vec2> {
    let merge_distance_sq = merge_distance.powi(2);
    let mut path: Vec<Vec2> = Vec::with_capacity(points.len());
    for &point in points {
        if path
            .last()
            .is_none_or(|last| last.distance_squared(point) > merge_distance_sq)
        {
            path.push(point);
        }
    }
    if closed {
        while path.len() > 1 && path[0].distance_squared(path[path.len() - 1]) <= merge_distance_sq
        {
            path.pop();
        }
    }
    path
}

/// [`merge_close_points`] для ленты: у разомкнутой ломаной конец остаётся на
/// месте, а сливается с ним предпоследняя точка. Иначе лента, срезанная в
/// сантиметрах за вершиной, кончалась бы на вершине и не доходила до шва с
/// соседним куском (клин смены сечения и тело улицы, `roads/tapers.rs`) — щель
/// поперёк проезжей части.
pub fn merge_ribbon_points(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<Vec2> {
    ribbon_vertices(points, closed, merge_distance)
        .into_iter()
        .zip(points)
        .filter_map(|(kept, &point)| kept.then_some(point))
        .collect()
}

/// Какие точки `points` лента оставит вершинами ([`merge_ribbon_points`] —
/// ровно они, по порядку). По ним скругления бордюра в узле
/// (`roads/corners.rs`) меряют прямой край луча: край ленты идёт по этим
/// вершинам, а не по слитым, и дуга по слитой вершине садилась мимо края —
/// у широкой улицы, где слияние берёт четверть ширины, на десятки сантиметров.
pub fn ribbon_vertices(points: &[Vec2], closed: bool, merge_distance: f32) -> Vec<bool> {
    let merge_distance_sq = merge_distance.powi(2);
    let mut kept = vec![false; points.len()];
    let mut last: Option<usize> = None;
    let mut count = 0;
    for (index, &point) in points.iter().enumerate() {
        if last.is_none_or(|last| points[last].distance_squared(point) > merge_distance_sq) {
            kept[index] = true;
            last = Some(index);
            count += 1;
        }
    }
    let Some(mut last) = last else {
        return kept;
    };
    let first = kept.iter().position(|&kept| kept).unwrap_or_default();
    if closed {
        // хвост, сошедшийся с началом
        while count > 1 && points[first].distance_squared(points[last]) <= merge_distance_sq {
            kept[last] = false;
            count -= 1;
            last = kept[..last].iter().rposition(|&kept| kept).unwrap_or(first);
        }
    } else if count > 1 && points[last] != points[points.len() - 1] {
        // конец остаётся на месте, сливается с ним предпоследняя точка; путь,
        // схлопнутый в точку, так и остаётся точкой
        kept[last] = false;
        kept[points.len() - 1] = true;
    }
    kept
}

/// Расстояние, на котором лента шириной `width` сливает соседние точки оси
/// ([`merge_ribbon_points`]): на четверти ширины их не видно.
pub fn ribbon_merge_distance(width: f32) -> f32 {
    width / 4.0
}

/// Направление **исходного** крайнего звена `points` у торца `end` — а не
/// звена после слияния точек: у начала — от него вглубь ломаной, у конца —
/// на него (перпендикуляр к нему, [`butt_normal`], в обоих случаях смотрит
/// влево по ходу). Два куска, разрезанные на одном звене, так встают торцами
/// друг к другу без клиновидной щели, даже если у каждого слияние выпрямило
/// конец по-своему. Звено короче сантиметра не в счёт.
pub(crate) fn butt_direction(points: &[Vec2], end: bool) -> Option<Vec2> {
    let away = |tip: Vec2, point: &&Vec2| point.distance(tip) > 0.01;
    let direction = if end {
        let tip = *points.last()?;
        tip - *points.iter().rev().find(|point| away(tip, point))?
    } else {
        let tip = *points.first()?;
        *points.iter().find(|point| away(tip, point))? - tip
    };
    direction.try_normalize()
}

/// Нормаль прямого торца: поперёк исходного крайнего звена
/// ([`butt_direction`]).
fn butt_normal(points: &[Vec2], end: bool) -> Option<Vec2> {
    butt_direction(points, end).map(Vec2::perp)
}

/// Не видна ли щель butt-квадов на изломе в `turn` радиан у ленты полуширины
/// `radius`: её ширина `radius · |turn|` мельче [`ARC_TOLERANCE`]. Одно правило
/// на обе стороны излома круглой ленты — такому веер не кладётся
/// ([`MeshBuilder::push_join_fan`]), и такой же квады проходят общими
/// вершинами; разойдись они, вернулась бы либо волосяная щель, либо веер поверх
/// общих вершин.
fn join_gap_hidden(radius: f32, turn: f32) -> bool {
    radius * turn.abs() < ARC_TOLERANCE
}
