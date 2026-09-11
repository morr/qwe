//! Спортивные и детские площадки: покрытие своего цвета и разметка на нём.
//!
//! Двор на снимке отличается от двора не домами, а тем, что между ними: зелёное
//! футбольное поле с белой разметкой, синяя коробка баскетбола, рыжий овал
//! беговой дорожки, песочное пятно детской площадки. До сих пор `leisure=*` не
//! запрашивался вовсе, и всё это было ровной землёй квартала.
//!
//! Разметка рисуется **в раме поля** — в его минимальном описанном
//! прямоугольнике ([`min_area_rect`]), — и только если контур эту раму
//! заполняет: у Г-образного или круглого пятна осевая пошла бы поперёк газона.
//! Линии те же 12 см, что у стоянки, и на том же плоском материале: белая
//! краска не несёт фактуры покрытия.

use bevy::prelude::*;

use crate::map::meshing::{MeshBuilder, min_area_rect};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, PitchKind, PolyArea};

/// Цвета покрытий. Не карта, а снимок: газон футбольного поля насыщеннее
/// газона двора, асфальтовая коробка синее и темнее улицы, тартан рыжий.
const SOCCER_COLOR: Color = Color::srgb(0.278, 0.396, 0.235);
const HARD_COLOR: Color = Color::srgb(0.322, 0.361, 0.396);
const TRACK_COLOR: Color = Color::srgb(0.514, 0.278, 0.208);
const PLAYGROUND_COLOR: Color = Color::srgb(0.482, 0.427, 0.353);
const GROUND_COLOR: Color = Color::srgb(0.400, 0.427, 0.361);

/// Ширина полосы разметки, м, и её цвет — та же краска, что на стоянке и на
/// улице.
const LINE_WIDTH: f32 = 0.12;
const LINE_COLOR: Color = Color::srgb(0.82, 0.82, 0.80);

/// Отступ разметки от края покрытия — доля меньшей стороны. У настоящего поля
/// за линией есть полоса безопасности, и без неё разметка читается как кант.
const LINE_INSET: f32 = 0.06;
/// Радиус центрального круга — доля длинной стороны. У футбольного поля это
/// 9.15 м на 105, то есть ровно эта доля; баскетбольный круг мельче, но и
/// площадка короче, так что одна доля работает на обоих.
const CIRCLE_SHARE: f32 = 0.087;
/// Круг рисуется правильным многоугольником: 32 стороны на девятиметровом
/// радиусе — это 1.8 м на сторону, доли пикселя на любом видимом зуме.
const CIRCLE_SIDES: usize = 32;
/// Штрафная площадь — доли длинной и короткой стороны (16.5 × 40.3 на
/// 105 × 68 у настоящего поля).
const PENALTY_LENGTH: f32 = 0.157;
const PENALTY_WIDTH: f32 = 0.593;
/// Поле короче этого штрафных не получает: во дворе их и не размечают.
const PENALTY_MIN_LENGTH: f32 = 45.0;
/// Насколько плотно контур обязан заполнять свою раму, чтобы разметка легла.
/// То же число, по которому [`crate::map::buildings`] решает, класть ли на дом
/// двускатную крышу: один смысл — «это прямоугольник, а не клякса».
const RECT_FILL_MIN: f32 = 0.85;
/// Площадка мельче этого пятна разметки не получает: теннисный стол во дворе
/// разметить нечем.
const MIN_AREA: f32 = 150.0;

/// Цвет покрытия площадки.
pub fn color(kind: PitchKind) -> Color {
    match kind {
        PitchKind::Soccer => SOCCER_COLOR,
        PitchKind::Hard => HARD_COLOR,
        PitchKind::Track => TRACK_COLOR,
        PitchKind::Playground => PLAYGROUND_COLOR,
        PitchKind::Ground => GROUND_COLOR,
    }
}

/// Разметка площадки в меш: периметр, осевая, центральный круг и — у
/// большого футбольного поля — штрафные.
///
/// Детская площадка, дорожка и спорткомплекс разметки не несут: у первой её
/// нет, у второй она вдоль овала (её пришлось бы вести по контуру, а не по
/// раме), у третьего внутри лежат уже размеченные поля.
pub fn push_markings(builder: &mut MeshBuilder, area: &PolyArea) {
    let AreaKind::Pitch(kind) = area.kind else {
        return;
    };
    if !matches!(kind, PitchKind::Soccer | PitchKind::Hard) {
        return;
    }
    let Some(frame) = Frame::of(area) else {
        return;
    };
    let color = LINE_COLOR.to_linear();

    // периметр — четыре полосы по внутреннему прямоугольнику
    let half = Vec2::new(frame.length, frame.width) * 0.5;
    for side in [1.0, -1.0] {
        // боковые: вдоль поля, отступ поперёк
        line(
            builder,
            frame.at + frame.across * (half.y * side),
            frame.axis,
            half.x,
            color,
        );
        // лицевые: поперёк поля, отступ вдоль
        line(
            builder,
            frame.at + frame.axis * (half.x * side),
            frame.across,
            half.y,
            color,
        );
    }
    // осевая — поперёк длинной оси
    line(builder, frame.at, frame.across, half.y, color);
    // центральный круг
    ring(
        builder,
        frame.at,
        frame.length * CIRCLE_SHARE,
        LINE_WIDTH,
        color,
    );

    if kind == PitchKind::Soccer && frame.length >= PENALTY_MIN_LENGTH {
        let depth = frame.length * PENALTY_LENGTH;
        let reach = frame.width * PENALTY_WIDTH * 0.5;
        for side in [1.0, -1.0] {
            let goal = frame.at + frame.axis * (half.x * side);
            let front = goal - frame.axis * (depth * side);
            // фронт штрафной и два её выхода к лицевой линии
            line(builder, front, frame.across, reach, color);
            for edge in [1.0, -1.0] {
                let at = goal - frame.axis * (depth * 0.5 * side) + frame.across * (reach * edge);
                line(builder, at, frame.axis, depth * 0.5, color);
            }
        }
    }
}

/// Рама поля: центр, оси и размеры описанного прямоугольника. `None`, если
/// пятно мелкое или на прямоугольник не похоже.
struct Frame {
    at: Vec2,
    axis: Vec2,
    across: Vec2,
    length: f32,
    width: f32,
}

impl Frame {
    fn of(area: &PolyArea) -> Option<Self> {
        let rect = min_area_rect(&area.outer)?;
        let axis = (rect[1] - rect[0]).try_normalize()?;
        let length = (rect[1] - rect[0]).length();
        let width = (rect[2] - rect[1]).length();
        let footprint = signed_ring_area(&area.outer).abs();
        if footprint < MIN_AREA || footprint < length * width * RECT_FILL_MIN {
            return None;
        }
        let inset = width * LINE_INSET;
        if length <= 2.0 * inset || width <= 2.0 * inset {
            return None;
        }
        Some(Self {
            at: (rect[0] + rect[2]) * 0.5,
            axis,
            across: Vec2::new(-axis.y, axis.x),
            length: length - 2.0 * inset,
            width: width - 2.0 * inset,
        })
    }
}

/// Полоса разметки: отрезок `at ± along * span` шириной [`LINE_WIDTH`].
fn line(builder: &mut MeshBuilder, at: Vec2, along: Vec2, span: f32, color: LinearRgba) {
    let half = Vec2::new(-along.y, along.x) * (LINE_WIDTH / 2.0);
    let reach = along * span;
    builder.push_quad(
        [
            at - reach - half,
            at + reach - half,
            at + reach + half,
            at - reach + half,
        ],
        color,
    );
}

/// Кольцо разметки: правильный многоугольник, каждая сторона — своя полоса.
fn ring(builder: &mut MeshBuilder, at: Vec2, radius: f32, width: f32, color: LinearRgba) {
    if radius <= width {
        return;
    }
    let step = std::f32::consts::TAU / CIRCLE_SIDES as f32;
    // радиус полосы берётся по середине стороны, чтобы кольцо не выходило
    // наружу расчётного радиуса
    let outer = radius + width / 2.0;
    let inner = radius - width / 2.0;
    for side in 0..CIRCLE_SIDES {
        let (a, b) = (side as f32 * step, (side + 1) as f32 * step);
        let (dir_a, dir_b) = (Vec2::from_angle(a), Vec2::from_angle(b));
        builder.push_quad(
            [
                at + dir_a * inner,
                at + dir_b * inner,
                at + dir_b * outer,
                at + dir_a * outer,
            ],
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::BuildingUse;

    fn pitch(kind: PitchKind, size: Vec2) -> PolyArea {
        PolyArea {
            outer: vec![
                Vec2::ZERO,
                Vec2::new(size.x, 0.0),
                size,
                Vec2::new(0.0, size.y),
            ],
            holes: Vec::new(),
            kind: AreaKind::Pitch(kind),
            building_use: BuildingUse::Other,
            height: None,
            entrances: Vec::new(),
        }
    }

    fn marked(area: &PolyArea) -> usize {
        let mut builder = MeshBuilder::default();
        push_markings(&mut builder, area);
        builder.vertex_count()
    }

    /// Большое поле получает периметр, осевую, круг и штрафные.
    #[test]
    fn a_full_size_pitch_gets_the_whole_marking() {
        let big = marked(&pitch(PitchKind::Soccer, Vec2::new(100.0, 64.0)));
        let small = marked(&pitch(PitchKind::Soccer, Vec2::new(40.0, 25.0)));
        assert!(big > small, "{big} vs {small}");
        // периметр (4) + осевая (1) + круг (32) — минимум у любого поля
        assert!(small >= 37 * 4, "{small}");
    }

    /// Детская площадка и дорожка разметки не несут.
    #[test]
    fn a_playground_and_a_track_stay_bare() {
        assert_eq!(
            marked(&pitch(PitchKind::Playground, Vec2::new(40.0, 30.0))),
            0
        );
        assert_eq!(marked(&pitch(PitchKind::Track, Vec2::new(90.0, 40.0))), 0);
    }

    /// Г-образное пятно разметки не получает: рама говорит не о его форме, и
    /// осевая легла бы по газону за пределами площадки.
    #[test]
    fn an_l_shape_gets_nothing() {
        let mut ell = pitch(PitchKind::Hard, Vec2::new(40.0, 30.0));
        // 40 × 30 без угла 20 × 15: 900 м² против 1200 м² рамы
        ell.outer = vec![
            Vec2::ZERO,
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 15.0),
            Vec2::new(20.0, 15.0),
            Vec2::new(20.0, 30.0),
            Vec2::new(0.0, 30.0),
        ];
        assert_eq!(marked(&ell), 0);
    }

    /// И мелкая площадка тоже.
    #[test]
    fn a_table_tennis_corner_gets_nothing() {
        assert_eq!(marked(&pitch(PitchKind::Hard, Vec2::new(10.0, 8.0))), 0);
    }
}
