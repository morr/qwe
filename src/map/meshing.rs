//! Сборка слитых 2D-мешей слоёв карты: тысячи полигонов OSM в один
//! `Mesh2d` с вершинными цветами (стоковый `ColorMaterial` их умножает;
//! `map::surface::SurfaceMaterial` — умножает и кладёт поверх фактуру).

use std::f32::consts::PI;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexAttribute, VertexFormat};
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

/// Локальные координаты ленты для шейдера поверхностей (`map::surface`):
/// `[поперёк, до ближайшего торца, полуширина, флаг разметки]` — метры и
/// 0/1. По ним шейдер улиц кладёт штриховую осевую и гасит её у торцов way.
/// У полигонов и прочей не-ленточной геометрии — нули. Атрибут есть только у
/// мешей, собранных через [`MeshBuilder::with_surface_coords`]: зданиям,
/// кронам и оверлеям он ни к чему, а это 16 байт на вершину.
///
/// Идентификатор — «высокий случайный», как велит документация
/// `MeshVertexAttribute`: он задаёт порядок атрибутов и не должен совпасть со
/// встроенными.
pub const ATTRIBUTE_RIBBON: MeshVertexAttribute =
    MeshVertexAttribute::new("Ribbon", 2_078_446_317, VertexFormat::Float32x4);

/// Координаты не-ленточной вершины.
const NO_RIBBON: [f32; 4] = [0.0; 4];

/// Максимальное удлинение стыка ленты относительно полуширины. Контур кроны
/// полон почти встречных рёбер (впадины между фестонами), и там miter уходит
/// в длинный шип — при 1.5 стык вырождается в срез, шипов не видно.
const MITER_LIMIT: f32 = 1.5;

/// Допуск на стрелку хорды дуги, м. Шаг тесселяции считается от радиуса, а не
/// берётся константой: у аллеи (полуширина 1.75 м) выходит ~28° на хорду, у
/// магистрали (8 м) ~13°, и обе дуги одинаково гладкие на глаз.
///
/// Тем же допуском отсекаются веера на почти прямых изломах — см.
/// [`MeshBuilder::push_join_fan`].
const ARC_TOLERANCE: f32 = 0.05;

/// Потолок числа хорд в дуге — страховка от вырожденного радиуса.
const MAX_ARC_STEPS: usize = 12;

/// Кайма контура ([`MeshBuilder::push_inset_band`]) не шире этой доли его
/// толщины (`площадь / периметр`): у полосы толщина — половина ширины, и кайма
/// в 0.6 её с каждой стороны оставляет посреди полосы просвет заливки.
const RIM_THICKNESS_SHARE: f32 = 0.6;
/// Кайма тоньше не кладётся: не видна, а квадов на контур — столько же.
const MIN_RIM_WIDTH: f32 = 0.2;

/// Стык сегментов ленты на изломе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonJoin {
    /// Сведение по биссектрисе с ограничением [`MITER_LIMIT`].
    Miter,
    /// Дуга радиуса в полуширину на внешней стороне поворота — то же, что
    /// `stroke-linejoin: round` у Mapnik, которым нарисован osm-carto.
    Round,
}

/// Торец разомкнутой ленты.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonCap {
    /// Срез ровно по последней точке пути.
    Butt,
    /// Полудиск радиуса в полуширину за последней точкой. Торцы двух дорог в
    /// общем узле перекрываются и сливаются в скруглённый стык — так узлы
    /// выглядят в OSM, где это `stroke-linecap: round`.
    Round,
}

/// Координаты [`ATTRIBUTE_RIBBON`] для вершин веера.
#[derive(Clone, Copy)]
enum FanCoords {
    /// Стык: весь веер лежит на внешней стороне излома, «до торца» у него одно
    /// на все вершины — то же, что у точки пути.
    Join { side: f32, to_end: f32 },
    /// Торец: поперёк — проекция на нормаль, «до торца» уходит в минус за
    /// точкой пути, так что разметка на полудиске гаснет сама.
    Cap { outward: Vec2 },
}

#[derive(Default)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    skipped_polygons: usize,
    /// `Some` — меш собирается для `SurfaceMaterial` и несёт
    /// [`ATTRIBUTE_RIBBON`] на каждой вершине.
    ribbon: Option<Vec<[f32; 4]>>,
    /// Флаг разметки для лент, которые лягут дальше
    /// ([`Self::set_road_markings`]).
    road_markings: bool,
}

impl MeshBuilder {
    /// Сборщик с локальными координатами лент ([`ATTRIBUTE_RIBBON`]) — для
    /// слоёв, которые рисует `map::surface::SurfaceMaterial`; без них тот
    /// материал меш не примет.
    pub fn with_surface_coords() -> Self {
        Self {
            ribbon: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// Нести ли лентам, положенным после этого вызова, флаг разметки: шейдер
    /// улиц кладёт осевую только по нему, и узкий проезд его не получает.
    /// Без координат поверхности флаг некуда записать.
    pub fn set_road_markings(&mut self, on: bool) {
        self.road_markings = on;
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Координаты ленты — тест проверяет по ним, что легло в атрибут.
    #[cfg(test)]
    pub fn ribbon_coords_for_test(&self) -> Option<&[[f32; 4]]> {
        self.ribbon.as_deref()
    }

    fn push_vertex(&mut self, position: Vec2, rgba: [f32; 4], ribbon: [f32; 4]) {
        self.positions.push([position.x, position.y, 0.0]);
        self.colors.push(rgba);
        if let Some(coords) = &mut self.ribbon {
            coords.push(ribbon);
        }
    }

    /// Координаты вершины ленты с текущим флагом разметки.
    fn coords(&self, across: f32, to_end: f32, half_width: f32) -> [f32; 4] {
        [
            across,
            to_end,
            half_width,
            if self.road_markings { 1.0 } else { 0.0 },
        ]
    }

    pub fn skipped_polygons(&self) -> usize {
        self.skipped_polygons
    }

    /// Сколько вершин уже накоплено — тестам, чтобы сравнивать объём
    /// геометрии, не разбирая готовый меш.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Накопленные вершины — только для тестов, которым надо проверить, куда
    /// именно легла геометрия (высота проёма арки, например).
    #[cfg(test)]
    pub fn positions_for_test(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Полигон с дырками через earcut. Вырожденный/кривой — пропуск со
    /// счётчиком, один плохой контур OSM не должен ронять всю карту.
    pub fn push_polygon(&mut self, outer: &[Vec2], holes: &[Vec<Vec2>], color: LinearRgba) {
        if outer.len() < 3 {
            self.skipped_polygons += 1;
            return;
        }

        let mut coordinates: Vec<f64> =
            Vec::with_capacity((outer.len() + holes.iter().map(Vec::len).sum::<usize>()) * 2);
        let mut hole_starts = Vec::with_capacity(holes.len());
        for point in outer {
            coordinates.push(point.x as f64);
            coordinates.push(point.y as f64);
        }
        for hole in holes {
            hole_starts.push(coordinates.len() / 2);
            for point in hole {
                coordinates.push(point.x as f64);
                coordinates.push(point.y as f64);
            }
        }

        let Ok(triangles) = earcutr::earcut(&coordinates, &hole_starts, 2) else {
            self.skipped_polygons += 1;
            return;
        };
        if triangles.is_empty() {
            self.skipped_polygons += 1;
            return;
        }

        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        for chunk in coordinates.chunks_exact(2) {
            self.push_vertex(Vec2::new(chunk[0] as f32, chunk[1] as f32), rgba, NO_RIBBON);
        }
        self.indices
            .extend(triangles.into_iter().map(|index| base + index as u32));
    }

    /// Уже собранная геометрия, приложенная со сдвигом и масштабом. Силуэт
    /// тени кроны один на вариант и повторяется под тысячами деревьев —
    /// триангулировать его каждый раз заново незачем.
    pub fn push_template(&mut self, template: &MeshBuilder, offset: Vec2, scale: f32) {
        let base = self.positions.len() as u32;
        self.positions
            .extend(template.positions.iter().map(|point| {
                [
                    point[0] * scale + offset.x,
                    point[1] * scale + offset.y,
                    point[2],
                ]
            }));
        self.colors.extend_from_slice(&template.colors);
        if let Some(coords) = &mut self.ribbon {
            match &template.ribbon {
                Some(source) => {
                    coords.extend(source.iter().map(|[across, to_end, half_width, flag]| {
                        [across * scale, to_end * scale, half_width * scale, *flag]
                    }))
                }
                None => coords.extend(std::iter::repeat_n(NO_RIBBON, template.positions.len())),
            }
        }
        self.indices
            .extend(template.indices.iter().map(|index| base + index));
    }

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

    /// Пунктир вдоль ломаной: лента ширины `width` кусками по `dash` метров
    /// через `gap`. Так Mapnik рисует ж/д путь в osm-carto — белая штриховка
    /// поверх тёмной ленты.
    ///
    /// Один проход по сегментам с курсором по длине дуги: точки текущего штриха
    /// копятся на ходу (концы интерполируются, вершины OSM между ними
    /// сохраняются) и сбрасываются в ленту по завершении штриха. Торцы — `Butt`:
    /// штрих это метка, а не конец дороги.
    ///
    /// Путь короче одного штриха всё равно даёт один штрих: иначе короткие ways
    /// (а их в ж/д развязке большинство) остались бы голой тёмной лентой.
    pub fn push_dashes(
        &mut self,
        points: &[Vec2],
        width: f32,
        dash: f32,
        gap: f32,
        color: LinearRgba,
        join: RibbonJoin,
    ) {
        if dash <= 0.0 || gap <= 0.0 || points.len() < 2 {
            return;
        }

        // остаток текущего интервала и что это за интервал
        let mut left = dash;
        let mut drawing = true;
        let mut current = vec![points[0]];

        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let Some(direction) = (to - from).try_normalize() else {
                continue;
            };
            let mut remaining = from.distance(to);
            let mut cursor = from;

            while remaining > left {
                cursor += direction * left;
                remaining -= left;
                if drawing {
                    current.push(cursor);
                    self.push_ribbon(&current, false, width, color, join, RibbonCap::Butt);
                }
                // буфер один на весь путь: своя `Vec` на каждый штрих ж/д
                // развязки — это аллокация на каждые несколько метров пути
                current.clear();
                current.push(cursor);
                drawing = !drawing;
                left = if drawing { dash } else { gap };
            }

            left -= remaining;
            // в пропуске копить нечего: следующий штрих начнётся с точки,
            // которую поставит переключение внутри цикла выше
            if drawing {
                current.push(to);
            }
        }

        if drawing && current.len() > 1 {
            self.push_ribbon(&current, false, width, color, join, RibbonCap::Butt);
        }
    }

    /// Поперечные шпалы вдоль ломаной: через каждые `spacing` метров — планка
    /// длиной `length` поперёк пути и толщиной `thickness`. Так рисуют трамвай
    /// Яндекс.Карты и 2ГИС — тонкая линия с частой поперечной насечкой.
    ///
    /// Тот же проход по длине дуги, что и у [`Self::push_dashes`], только на
    /// отметке ставится не кусок пути, а перпендикуляр к нему. Первая шпала
    /// отступает на полшага: планка ровно в торце пути выглядит обрубком, а на
    /// стыке двух ways две такие складываются в крест.
    pub fn push_ticks(
        &mut self,
        points: &[Vec2],
        length: f32,
        thickness: f32,
        spacing: f32,
        color: LinearRgba,
    ) {
        if spacing <= 0.0 || length <= 0.0 || thickness <= 0.0 || points.len() < 2 {
            return;
        }

        let half = length / 2.0;
        let mut left = spacing / 2.0;

        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let Some(direction) = (to - from).try_normalize() else {
                continue;
            };
            let mut remaining = from.distance(to);
            let mut cursor = from;

            while remaining > left {
                cursor += direction * left;
                remaining -= left;
                let arm = direction.perp() * half;
                self.push_ribbon(
                    &[cursor - arm, cursor + arm],
                    false,
                    thickness,
                    color,
                    RibbonJoin::Miter,
                    RibbonCap::Butt,
                );
                left = spacing;
            }

            left -= remaining;
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

    /// То же, но торцы задаются по отдельности — `[начало, конец]`. Нужно
    /// руслу: один его конец продолжается открытым руслом (там полудиск
    /// сливает стык), а другой упирается во вход в трубу, где полудиску за
    /// узлом взяться неоткуда (`spawn::mesh_water_lines`).
    pub fn push_ribbon_capped(
        &mut self,
        points: &[Vec2],
        closed: bool,
        width: f32,
        color: LinearRgba,
        join: RibbonJoin,
        caps: [RibbonCap; 2],
    ) {
        let mut path = merge_close_points(points, closed, width / 4.0);
        if path.len() < 2 {
            return;
        }

        let half_width = width / 2.0;
        let (mut along, total) = arclengths(&path, closed);
        if self.ribbon.is_some() && !closed {
            split_at_midpoint(&mut path, &mut along, total, width / 4.0);
        }
        // «до ближайшего торца» — то, по чему шейдер гасит разметку у
        // перекрёстка; у замкнутой ленты торцов нет, и остаётся длина дуги
        let ends: Vec<f32> = along
            .iter()
            .map(|&at| {
                if closed {
                    at
                } else {
                    to_nearest_end(at, total)
                }
            })
            .collect();

        let count = path.len();
        let segments = if closed { count } else { count - 1 };

        match join {
            RibbonJoin::Miter => {
                let offsets = miter_offsets(&path, closed, half_width);

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
                        self.segment_coords(half_width, ends[index], ends[next]),
                    );
                }
            }
            RibbonJoin::Round => {
                // сегменты — квады без продлений; на внутренней стороне
                // излома они перекрываются сами, снаружи щель закрывает веер
                for index in 0..segments {
                    let next = (index + 1) % count;
                    let Some(direction) = (path[next] - path[index]).try_normalize() else {
                        continue;
                    };
                    let normal = direction.perp() * half_width;
                    self.push_quad_full(
                        [
                            path[index] + normal,
                            path[index] - normal,
                            path[next] - normal,
                            path[next] + normal,
                        ],
                        [color; 4],
                        self.segment_coords(half_width, ends[index], ends[next]),
                    );
                }
                for index in 0..count {
                    if !closed && (index == 0 || index + 1 == count) {
                        continue;
                    }
                    let previous = path[(index + count - 1) % count];
                    let next = path[(index + 1) % count];
                    let (Some(incoming), Some(outgoing)) = (
                        (path[index] - previous).try_normalize(),
                        (next - path[index]).try_normalize(),
                    ) else {
                        continue;
                    };
                    self.push_join_fan(
                        path[index],
                        half_width,
                        incoming,
                        outgoing,
                        color,
                        ends[index],
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
                    },
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
                    FanCoords::Cap { outward: direction },
                );
            }
        }
    }

    /// Координаты четырёх углов квада сегмента: `+нормаль` в начале, `−нормаль`
    /// в начале, `−нормаль` в конце, `+нормаль` в конце — порядок
    /// [`Self::push_quad_full`].
    fn segment_coords(&self, half_width: f32, at_start: f32, at_end: f32) -> [[f32; 4]; 4] {
        [
            self.coords(half_width, at_start, half_width),
            self.coords(-half_width, at_start, half_width),
            self.coords(-half_width, at_end, half_width),
            self.coords(half_width, at_end, half_width),
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
        to_end: f32,
    ) {
        let turn = incoming.angle_to(outgoing);
        if radius * turn.abs() < ARC_TOLERANCE {
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
            FanCoords::Join { side, to_end },
        );
    }

    /// Веер треугольников по дуге: `sweep` радиан от `start` вокруг `center`.
    fn push_arc_fan(
        &mut self,
        center: Vec2,
        radius: f32,
        start: f32,
        sweep: f32,
        color: LinearRgba,
        coords: FanCoords,
    ) {
        let steps = arc_steps(radius, sweep.abs());
        let base = self.positions.len() as u32;
        let rgba = color.to_f32_array();
        let at_center = match coords {
            FanCoords::Join { to_end, .. } => self.coords(0.0, to_end, radius),
            FanCoords::Cap { .. } => self.coords(0.0, 0.0, radius),
        };
        self.push_vertex(center, rgba, at_center);
        for step in 0..=steps {
            let angle = start + sweep * step as f32 / steps as f32;
            let point = center + Vec2::from_angle(angle) * radius;
            let at_rim = match coords {
                FanCoords::Join { side, to_end } => self.coords(side * radius, to_end, radius),
                FanCoords::Cap { outward } => {
                    let offset = point - center;
                    self.coords(offset.dot(outward.perp()), -offset.dot(outward), radius)
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
        let path = merge_close_points(ring, true, width / 4.0);
        if path.len() < 3 {
            return None;
        }
        let area = signed_area(&path);
        let width = if outside {
            width
        } else {
            width.min(RIM_THICKNESS_SHARE * area.abs() / perimeter(&path))
        };
        if width < MIN_RIM_WIDTH {
            return None;
        }
        // у обхода против часовой стрелки внутренняя сторона слева — куда и
        // смотрят miter-офсеты; по часовой — справа
        let side = if (area > 0.0) != outside { 1.0 } else { -1.0 };
        let offsets = miter_offsets(&path, true, width);
        let count = path.len();
        for index in 0..count {
            let next = (index + 1) % count;
            self.push_quad_gradient(
                [
                    path[index],
                    path[index] + offsets[index] * side,
                    path[next] + offsets[next] * side,
                    path[next],
                ],
                [edge, inner, inner, edge],
            );
        }
        Some(width)
    }

    /// Прямоугольник по AABB (для тайловых оверлеев).
    pub fn push_rect(&mut self, min: Vec2, max: Vec2, color: LinearRgba) {
        self.push_quad(
            [
                Vec2::new(min.x, min.y),
                Vec2::new(max.x, min.y),
                Vec2::new(max.x, max.y),
                Vec2::new(min.x, max.y),
            ],
            color,
        );
    }

    pub(crate) fn push_quad(&mut self, corners: [Vec2; 4], color: LinearRgba) {
        self.push_quad_gradient(corners, [color; 4]);
    }

    /// Квад с цветом на каждую вершину — для вертикального градиента стен
    /// экструдированных зданий.
    pub(crate) fn push_quad_gradient(&mut self, corners: [Vec2; 4], colors: [LinearRgba; 4]) {
        self.push_quad_full(corners, colors, [NO_RIBBON; 4]);
    }

    /// Квад с цветом и координатами ленты на каждую вершину.
    fn push_quad_full(
        &mut self,
        corners: [Vec2; 4],
        colors: [LinearRgba; 4],
        coords: [[f32; 4]; 4],
    ) {
        let base = self.positions.len() as u32;
        for ((corner, color), ribbon) in corners.into_iter().zip(colors).zip(coords) {
            self.push_vertex(corner, color.to_f32_array(), ribbon);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn build(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        if let Some(ribbon) = self.ribbon {
            mesh.insert_attribute(ATTRIBUTE_RIBBON, ribbon);
        }
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// Удвоенная площадь по формуле шнурков со знаком: положительна при обходе
/// против часовой стрелки.
fn signed_area(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].perp_dot(ring[(index + 1) % count]))
        .sum::<f32>()
        / 2.0
}

/// Периметр замкнутого контура.
fn perimeter(ring: &[Vec2]) -> f32 {
    let count = ring.len();
    (0..count)
        .map(|index| ring[index].distance(ring[(index + 1) % count]))
        .sum()
}

/// Длина дуги в каждой точке ломаной и полная длина; у замкнутой — с
/// замыкающим сегментом.
fn arclengths(path: &[Vec2], closed: bool) -> (Vec<f32>, f32) {
    let mut along = Vec::with_capacity(path.len());
    let mut total = 0.0;
    for (index, point) in path.iter().enumerate() {
        if index > 0 {
            total += point.distance(path[index - 1]);
        }
        along.push(total);
    }
    if closed && path.len() > 1 {
        total += path[path.len() - 1].distance(path[0]);
    }
    (along, total)
}

/// Расстояние до ближайшего торца разомкнутого пути.
fn to_nearest_end(along: f32, total: f32) -> f32 {
    along.min(total - along)
}

/// Вершина на середине пути, чтобы «до ближайшего торца» — функция с изломом
/// на середине — была линейной внутри каждого квада: GPU интерполирует
/// атрибут по прямой, и квад, накрывший середину, получил бы расстояние,
/// растущее там, где оно убывает. Середина ближе `merge_distance` к соседней
/// вершине не вставляется: излом внутри такого коротышки не виден, а
/// вырожденный квад — да.
fn split_at_midpoint(path: &mut Vec<Vec2>, along: &mut Vec<f32>, total: f32, merge_distance: f32) {
    let middle = total / 2.0;
    let Some(index) = along
        .windows(2)
        .position(|pair| pair[0] < middle && middle < pair[1])
    else {
        return;
    };
    if middle - along[index] <= merge_distance || along[index + 1] - middle <= merge_distance {
        return;
    }
    let t = (middle - along[index]) / (along[index + 1] - along[index]);
    let point = path[index].lerp(path[index + 1], t);
    path.insert(index + 1, point);
    along.insert(index + 1, middle);
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

/// Сколько хорд нужно дуге радиуса `radius` на `sweep` радиан, чтобы стрелка
/// хорды осталась в пределах [`ARC_TOLERANCE`].
fn arc_steps(radius: f32, sweep: f32) -> usize {
    let max_step = if radius > ARC_TOLERANCE {
        2.0 * (1.0 - ARC_TOLERANCE / radius).acos()
    } else {
        PI
    };
    ((sweep / max_step).ceil() as usize).clamp(1, MAX_ARC_STEPS)
}

#[cfg(test)]
mod tests;
