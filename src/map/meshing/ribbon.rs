//! Ленты — полосы вдоль ломаной: [`MeshBuilder::push_ribbon`] и её родня
//! (`push_stroke`, клин `push_taper`, рельсы `push_rails`, полилиния
//! `push_polyline`, кайма контура `push_inset_band*`), веера на изломах и
//! торцах, и геометрия пути под ними: miter-офсеты, склейка близких точек и
//! профиль разрывов разметки ([`GapProfile`]) с его разбором для слоя краски.

pub(super) mod geometry;
pub(super) mod profile;

use std::f32::consts::PI;

use bevy::prelude::*;

use self::geometry::{
    butt_normal, insert_vertex_at, join_gap_hidden, merge_close_points, merge_ribbon_points,
    miter_offsets, ribbon_merge_distance,
};
use self::profile::GapProfile;
use super::geometry::{arc_steps, arclengths, ring_perimeter, signed_area};
use super::{LaneFrame, MeshBuilder, RibbonBreaks, RibbonCap, RibbonJoin};

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
    pub(crate) fn push_inset_band_with(
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

/// Пара профиля раскладки ближе этого к вершине ленты вершины не получает, м:
/// пары идут через пару метров, и сдвиг на десять сантиметров незаметен.
const PROFILE_MERGE: f32 = 0.1;
