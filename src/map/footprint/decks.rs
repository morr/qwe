//! Нарисованные настилы мостов как вырезающий инструмент: капсулы вокруг
//! пролётов, по которым сверху режутся ограда ([`super::fences::fence_pieces`])
//! и её тень. Геометрию самого моста здесь не строят — только то, что он
//! накрывает.

use bevy::prelude::*;

use super::fences::{GAP_CELL, segment_in_circle};
use crate::map::grid::Grid;
use crate::map::osm::model::RoadLine;

/// Нарисованные настилы мостов — то, что закрывает собой ограду сверху.
///
/// Забор лежит на `Z_FENCE` (2.75), мост — на `Z_BRIDGE` (2.2), и ограда,
/// проходящая **под** пролётом, рисовалась поверх него: тёмная нитка с тенью
/// поперёк моста через Упу (сообщено по снимку). Приём тот же, которым слой
/// машин обходит настил (`cars::BridgeDeck`) — всё, что лежит на z **выше**
/// моста, само уступает ему место, — но вопрос здесь другой: машина целиком
/// снимается с места, а ограда режется по кромке, как режется проёмом.
///
/// **Накрыто — это лента, а не пересечение осевых.** Проёмы дорог меряются
/// кругом в точке пересечения именно потому, что улица **вдоль** забора не
/// должна его открывать ([`fence_gaps`](super::fences::fence_gaps)); у моста
/// всё наоборот — он закрывает ровно то, что под ним, хоть вдоль, хоть поперёк.
/// Отсюда капсула: полоса шириной `2 × curb_reach` (настил с бордюром — вся
/// нарисованная ширина моста) со скруглёнными торцами.
///
/// Осевая берётся сырая, как у машин, а рисуется мост по сглаженной копии —
/// расхождение там сантиметры, и бордюр в 0.8–2 м его покрывает.
///
/// **Запас `reserve`** — вторая половина ответа, и без неё вырез виден. Режется
/// геометрия (осевая ограды, контур тени), а нарисована она шире того, что
/// режут: у ленты это её полуширина с круглым торцом, у тени — мягкая кайма
/// (`fences::SHADOW_BLUR`). Вырез строго по кромке настила оставляет и то и
/// другое на мосту — именно это и было видно вторым снимком: линия оборвалась,
/// а серый свип из-под её торца продолжился на полотно. Поэтому капсула шире
/// настила на `reserve`, и нарисованное кончается ровно на кромке.
pub struct BridgeDecks {
    /// Звенья настилов: `(торец, торец, полуширина выреза)`.
    segments: Vec<(Vec2, Vec2, f32)>,
    /// Номера звеньев по ячейкам их коробок, расширенных полушириной выреза.
    cells: Grid<u32>,
}

impl BridgeDecks {
    pub fn build(roads: &[RoadLine], reserve: f32) -> Self {
        let mut segments: Vec<(Vec2, Vec2, f32)> = Vec::new();
        let mut cells: Grid<u32> = Grid::new(GAP_CELL);
        for road in roads.iter().filter(|road| road.bridge) {
            let reach = road.curb_reach() + reserve;
            for pair in road.points.windows(2) {
                cells.insert_segment(pair[0], pair[1], reach, segments.len() as u32);
                segments.push((pair[0], pair[1], reach));
            }
        }
        Self { segments, cells }
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Дописать к `covered` интервалы параметра звена `a→b`, накрытые
    /// настилами. Дубликаты (звено настила лежит в нескольких ячейках) не
    /// отсеиваются: вызывающий и так сортирует и склеивает перекрытия.
    pub(super) fn cover(&self, a: Vec2, b: Vec2, covered: &mut Vec<(f32, f32)>) {
        for &index in self.cells.near_each(a.min(b), a.max(b)) {
            let (c, d, reach) = self.segments[index as usize];
            covered.extend(segment_in_capsule(a, b, c, d, reach));
        }
    }

    /// Те же капсулы многоугольниками — то, из чего вычитается тень.
    ///
    /// Линию режет интервал параметра, а тень — булева разность: тень это
    /// площадь, а не точки на осевой, и она приходит на настил **сбоку**, из-под
    /// ещё не обрезанного куска ограды рядом с мостом. По капсуле на звено;
    /// объединять их не нужно, `NonZero` внутри разности сделает это сам.
    pub fn outlines(&self) -> Vec<Vec<[f32; 2]>> {
        self.segments
            .iter()
            .map(|&(a, b, reach)| capsule_outline(a, b, reach))
            .collect()
    }
}

/// Сторон у полукруглого торца капсулы.
const CAP_SIDES: usize = 8;

/// Капсула вокруг отрезка многоугольником, против часовой стрелки: прямоугольник
/// и два полукруга.
///
/// Многоугольник **описан** вокруг круга, а не вписан: вписанный оставил бы у
/// каждого торца по серпику невырезанной тени — того самого, ради чего вырез и
/// делается.
fn capsule_outline(a: Vec2, b: Vec2, radius: f32) -> Vec<[f32; 2]> {
    let along = (b - a).try_normalize().unwrap_or(Vec2::X);
    let radius = radius / (std::f32::consts::PI / (2.0 * CAP_SIDES as f32)).cos();
    let mut ring = Vec::with_capacity(2 * (CAP_SIDES + 1));
    for (end, forward) in [(b, along), (a, -along)] {
        ring.extend((0..=CAP_SIDES).map(|side| {
            let angle = std::f32::consts::PI * side as f32 / CAP_SIDES as f32;
            (end + radius * (forward * angle.sin() - forward.perp() * angle.cos())).to_array()
        }));
    }
    ring
}

/// Часть звена `a→b` внутри капсулы вокруг отрезка `c→d` — интервал параметра
/// в `0..=1`; `None`, если звено капсулы не касается.
///
/// Капсула выпукла, значит накрытое ей множество параметра — один интервал, и
/// объединения трёх кусков (полоса и два торцевых круга) хватает: они не могут
/// разойтись на два интервала.
fn segment_in_capsule(a: Vec2, b: Vec2, c: Vec2, d: Vec2, radius: f32) -> Option<(f32, f32)> {
    let (mut from, mut to) = (f32::INFINITY, f32::NEG_INFINITY);
    for (start, end) in [
        segment_in_circle(a, b, c, radius),
        segment_in_circle(a, b, d, radius),
        segment_in_band(a, b, c, d, radius),
    ]
    .into_iter()
    .flatten()
    {
        from = from.min(start);
        to = to.max(end);
    }
    (from < to).then_some((from, to))
}

/// Часть звена `a→b` внутри прямой полосы отрезка `c→d` — без торцевых
/// скруглений: в системе координат отрезка обе координаты точки аффинны по
/// параметру, так что это пересечение двух интервалов.
fn segment_in_band(a: Vec2, b: Vec2, c: Vec2, d: Vec2, radius: f32) -> Option<(f32, f32)> {
    let along = (d - c).try_normalize()?;
    let across = along.perp();
    let length = (d - c).length();
    let (lengthwise_from, lengthwise_to) =
        affine_within((a - c).dot(along), (b - c).dot(along), 0.0, length)?;
    let (crosswise_from, crosswise_to) =
        affine_within((a - c).dot(across), (b - c).dot(across), -radius, radius)?;
    let (from, to) = (
        lengthwise_from.max(crosswise_from),
        lengthwise_to.min(crosswise_to),
    );
    (from < to).then_some((from, to))
}

/// Интервал параметра в `0..=1`, на котором аффинная величина, идущая от
/// `from` к `to`, лежит в `lo..=hi`.
fn affine_within(from: f32, to: f32, lo: f32, hi: f32) -> Option<(f32, f32)> {
    let slope = to - from;
    if slope.abs() <= f32::EPSILON {
        return (lo..=hi).contains(&from).then_some((0.0, 1.0));
    }
    let (first, second) = ((lo - from) / slope, (hi - from) / slope);
    let (start, end) = (first.min(second).max(0.0), first.max(second).min(1.0));
    (start < end).then_some((start, end))
}
