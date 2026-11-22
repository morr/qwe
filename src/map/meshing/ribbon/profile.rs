//! Профиль разрывов разметки вдоль ленты: «до разрыва» в каждой вершине
//! ([`GapProfile`]) и его разбор для слоя краски — [`to_break_beyond`],
//! [`break_profile`] и [`break_distances`].

use bevy::prelude::*;

use crate::map::meshing::geometry::{arclengths, project_onto_path};
use crate::map::meshing::{Break, RibbonBreaks};

use super::FAR_FROM_BREAKS;
use super::geometry::{
    append_vertex_at, insert_vertex_at, merge_ribbon_points, ribbon_merge_distance,
};

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

/// Разрывы разметки одной ленты в координатах длины дуги — отсортированы и не
/// пересекаются, так что между двумя соседними «до разрыва» — функция с одним
/// изломом.
pub(super) struct GapProfile {
    /// `(центр, полудлина)` каждого разрыва.
    gaps: Vec<(f32, f32)>,
    /// Длина замкнутой ленты: по ней «до разрыва» меряется **по кругу**, в обе
    /// стороны от точки. `None` у разомкнутой.
    period: Option<f32>,
}

impl GapProfile {
    pub(super) fn new(
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
    pub(super) fn distance(&self, at: f32) -> f32 {
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
    pub(super) fn split_path(
        &self,
        path: &mut Vec<Vec2>,
        along: &mut Vec<f32>,
        merge_distance: f32,
    ) {
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
