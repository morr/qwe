//! Штрихи вдоль ломаной и полосы слоя краски: пунктир
//! ([`MeshBuilder::push_dashes`], ж/д путь) и поперечные шпалы
//! ([`MeshBuilder::push_ticks`], трамвай), полоса под линию разметки
//! ([`MeshBuilder::push_paint_strip`]) и площадная краска
//! ([`MeshBuilder::push_paint_area`]) — у последних в атрибут ленты
//! ([`ATTRIBUTE_RIBBON`](super::ATTRIBUTE_RIBBON)) своя система координат.

use bevy::prelude::*;

use super::{MeshBuilder, PaintStation, RibbonCap, RibbonJoin, miter_offsets};

impl MeshBuilder {
    /// Полоса под одну линию слоя краски (`roads/paint.rs`): лента полуширины
    /// `half_width` вдоль `points`, стыки — общими вершинами по биссектрисе,
    /// торцы — по крайним точкам. Полоса **шире самой линии**: линию с полом
    /// в пиксели, штрихами и сглаженным краем рисует шейдер краски по
    /// координатам полосы, а полоса лишь оставляет ему место на самом дальнем
    /// зуме, где линия ещё видна.
    ///
    /// В [`ATTRIBUTE_RIBBON`](super::ATTRIBUTE_RIBBON) — `[поперёк от линии,
    /// длина улицы, до разрыва,
    /// kind]` из `stations` (по одной на точку), в альфу цвета — видимость
    /// станции.
    pub fn push_paint_strip(
        &mut self,
        points: &[Vec2],
        closed: bool,
        half_width: f32,
        stations: &[PaintStation],
        kind: f32,
        color: LinearRgba,
    ) {
        let count = points.len();
        if count < 2 || stations.len() != count {
            return;
        }
        let miters = miter_offsets(points, closed, half_width);
        let base = self.positions.len() as u32;
        for ((&point, miter), station) in points.iter().zip(&miters).zip(stations) {
            let rgba = color.with_alpha(color.alpha * station.alpha).to_f32_array();
            for side in [1.0, -1.0] {
                self.push_vertex(
                    point + *miter * side,
                    rgba,
                    [side * half_width, station.along, station.to_break, kind],
                );
            }
        }
        let segments = if closed { count } else { count - 1 };
        for index in 0..segments as u32 {
            let next = (index + 1) % count as u32;
            let (left, right) = (base + 2 * index, base + 2 * index + 1);
            let (next_left, next_right) = (base + 2 * next, base + 2 * next + 1);
            self.indices
                .extend([left, right, next_right, left, next_right, next_left]);
        }
    }

    /// Площадь слоя краски — многоугольник с дырками, у каждой вершины в
    /// [`ATTRIBUTE_RIBBON`](super::ATTRIBUTE_RIBBON) — `[0, координата поперёк
    /// полос, to_break,
    /// kind]`, где координата — проекция вершины на `across`, а `to_break` и
    /// `kind` те же, что у [`Self::push_paint_strip`]. Так штриховку рисует
    /// шейдер краски: полосы идут по мировой координате и не зависят от того,
    /// как лёг треугольник.
    pub fn push_paint_area(
        &mut self,
        outer: &[Vec2],
        holes: &[Vec<Vec2>],
        across: Vec2,
        to_break: f32,
        kind: f32,
        color: LinearRgba,
    ) {
        let base = self.positions.len() as u32;
        self.push_polygon(outer, holes, color);
        let pushed = self.positions.len() as u32;
        if let Some(coords) = &mut self.ribbon {
            for index in base..pushed {
                let [x, y, _] = self.positions[index as usize];
                coords[index as usize] = [0.0, Vec2::new(x, y).dot(across), to_break, kind];
            }
        }
    }

    /// Пунктир вдоль ломаной: лента ширины `width` кусками по `dash` метров
    /// через `gap`. Так Mapnik рисует ж/д путь в osm-carto — рисунок оттуда,
    /// цвет штриха выбирает вызывающий.
    ///
    /// Один проход по сегментам с курсором по длине дуги: точки текущего штриха
    /// копятся на ходу (концы интерполируются, вершины OSM между ними
    /// сохраняются) и сбрасываются в ленту по завершении штриха. Торцы — `Butt`:
    /// штрих это метка, а не конец дороги.
    ///
    /// Путь короче одного штриха всё равно даёт один штрих: иначе короткие ways
    /// (а их в ж/д развязке большинство) остались бы голой лентой без метки.
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
}
