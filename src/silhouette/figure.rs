//! Лежащие тела и лужа под ними.
//!
//! Труп — не растянутый диск, а фигура человека сверху: голова, торс, руки и
//! ноги из капсул по скелету, в одной из [`POSES`] поз. Скелет задан в ростах
//! (1 = рост стоя), суставы — углами от оси тела: голова смотрит в +x, ноги
//! в −x, левый бок в +y. Конечности нарочно толще анатомии: рука в честные
//! 0,05 роста на зуме толпы — доли пикселя, а фигура обязана читаться позой;
//! сгущённая в полтора-два раза, она даёт силуэт-отпечаток, а не проволочного
//! человечка.
//!
//! Все позы лежат в одинаковых квадратных ячейках ([`CELL_SPAN`] ростов):
//! спрайт вращается на любой угол, и прямоугольная ячейка ничего бы не
//! выиграла. Габаритный ящик каждой позы центрируется в ячейке; точка, куда
//! ложится лужа, — грудь, — отдаётся наружу в тех же координатах ячейки.

use bevy::prelude::*;

use super::smoothstep;

/// Число поз; глифы `Glyph::corpse(i)` идут в атласе подряд.
pub const POSES: usize = 4;
/// Сторона ячейки в ростах: самая длинная поза (ничком, рука вытянута за
/// голову — 1,29 роста от пальцев до пяток) с запасом на кромку и мипы.
pub const CELL_SPAN: f32 = 1.4;

// --- Скелет, в ростах. Голова и конечности крупнее анатомии — см. выше. ---
const HEAD_R: f32 = 0.085;
const NECK_R: f32 = 0.05;
/// От линии плеч до центра головы: плечо, шея и половина головы.
const NECK_LEN: f32 = 0.165;
const TORSO_LEN: f32 = 0.28;
const HIP_R: f32 = 0.11;
const CHEST_R: f32 = 0.12;
/// Плечи — перекладина поперёк верха торса: сверху они плоские, а не круглые.
const SHOULDER_HALF: f32 = 0.11;
const SHOULDER_R: f32 = 0.075;
const HIP_HALF: f32 = 0.08;
const UPPER_ARM: f32 = 0.16;
const FOREARM: f32 = 0.14;
const ARM_R: f32 = 0.062;
const FOREARM_R: f32 = 0.055;
const HAND_R: f32 = 0.055;
const THIGH: f32 = 0.23;
const CALF: f32 = 0.22;
const THIGH_R: f32 = 0.082;
const CALF_R: f32 = 0.068;
const FOOT: f32 = 0.11;
const FOOT_R: f32 = 0.045;
/// Сужение к дальнему суставу: плечо толще у плеча, бедро — у таза.
const TAPER: f32 = 0.88;
/// Тёмная кайма фигуры, ростов: как ободок диска — тело с контуром, не клякса.
const RIM: f32 = 0.02;
/// Яркость каймы, множитель к цвету спрайта.
const RIM_SHADE: f32 = 0.5;

/// Поза: наклон торса, поворот головы и углы суставов, градусы от +x
/// (голова), +y — левый бок. Руки: плечо и предплечье; ноги: бедро, голень,
/// стопа; в обоих массивах сначала левая.
struct PoseSpec {
    torso: f32,
    head_turn: f32,
    arms: [(f32, f32); 2],
    legs: [(f32, f32, f32); 2],
}

const SPECS: [PoseSpec; POSES] = [
    // навзничь: одна рука за голову, другая в сторону, ноги врозь
    PoseSpec {
        torso: 0.0,
        head_turn: 0.0,
        arms: [(48.0, 22.0), (-108.0, -84.0)],
        legs: [(163.0, 168.0, 132.0), (-163.0, -168.0, -132.0)],
    },
    // ничком: тянется рукой вперёд, другая вдоль тела, нога подогнута
    PoseSpec {
        torso: 3.0,
        head_turn: 0.05,
        arms: [(18.0, 4.0), (-140.0, -172.0)],
        legs: [(176.0, 178.0, 181.0), (-152.0, -112.0, -105.0)],
    },
    // на боку, калачиком: колени к животу, голени назад, руки у груди
    PoseSpec {
        torso: 25.0,
        head_turn: 0.06,
        arms: [(110.0, 30.0), (85.0, 15.0)],
        legs: [(100.0, -172.0, -165.0), (110.0, -160.0, -150.0)],
    },
    // рухнул лицом вниз: одна рука под телом, другая откинута, голова набок
    PoseSpec {
        torso: 0.0,
        head_turn: -0.07,
        arms: [(165.0, -60.0), (-120.0, -95.0)],
        legs: [(174.0, 178.0, 168.0), (-168.0, -158.0, -140.0)],
    },
];

/// Капсула с двумя радиусами (оболочка двух кругов); `a == b` — круг.
struct Part {
    a: Vec2,
    b: Vec2,
    ra: f32,
    rb: f32,
}

impl Part {
    fn limb(a: Vec2, b: Vec2, ra: f32, rb: f32) -> Self {
        Self { a, b, ra, rb }
    }

    fn dot(centre: Vec2, radius: f32) -> Self {
        Self::limb(centre, centre, radius, radius)
    }

    /// Расстояние со знаком до контура (внутри — отрицательное).
    fn distance(&self, p: Vec2) -> f32 {
        let ab = self.b - self.a;
        let h = ab.length();
        if h < 1e-4 {
            return (p - self.a).length() - self.ra.max(self.rb);
        }
        let along = ab / h;
        let q = p - self.a;
        // в системе капсулы: y вдоль оси, x поперёк, по модулю
        let q = Vec2::new(q.perp_dot(along).abs(), q.dot(along));
        let slope = (self.ra - self.rb) / h;
        let cos = (1.0 - slope * slope).max(0.0).sqrt();
        let k = q.dot(Vec2::new(-slope, cos));
        if k < 0.0 {
            q.length() - self.ra
        } else if k > cos * h {
            (q - Vec2::new(0.0, h)).length() - self.rb
        } else {
            q.dot(Vec2::new(cos, slope)) - self.ra
        }
    }
}

/// Собранная поза: части в ростах, центр габаритного ящика и грудь.
pub struct Figure {
    parts: Vec<Part>,
    centre: Vec2,
    chest: Vec2,
}

fn dir(degrees: f32) -> Vec2 {
    Vec2::from_angle(degrees.to_radians())
}

impl Figure {
    pub fn pose(index: usize) -> Self {
        let spec = &SPECS[index % POSES];
        let axis = dir(spec.torso);
        let side = axis.perp();
        let chest = axis * TORSO_LEN;
        let head = chest + axis * NECK_LEN + side * spec.head_turn;
        let shoulders = [chest + side * SHOULDER_HALF, chest - side * SHOULDER_HALF];
        let hips = [Vec2::new(0.0, HIP_HALF), Vec2::new(0.0, -HIP_HALF)];

        let mut parts = vec![
            Part::limb(Vec2::ZERO, chest, HIP_R, CHEST_R),
            Part::limb(shoulders[0], shoulders[1], SHOULDER_R, SHOULDER_R),
            Part::limb(chest, head, NECK_R, NECK_R),
            Part::dot(head, HEAD_R),
        ];
        for (shoulder, (upper, fore)) in shoulders.into_iter().zip(spec.arms) {
            let elbow = shoulder + dir(upper) * UPPER_ARM;
            let wrist = elbow + dir(fore) * FOREARM;
            parts.push(Part::limb(shoulder, elbow, ARM_R, ARM_R * TAPER));
            parts.push(Part::limb(elbow, wrist, FOREARM_R, FOREARM_R * TAPER));
            parts.push(Part::dot(wrist + dir(fore) * HAND_R, HAND_R));
        }
        for (hip, (thigh, calf, foot)) in hips.into_iter().zip(spec.legs) {
            let knee = hip + dir(thigh) * THIGH;
            let ankle = knee + dir(calf) * CALF;
            let toe = ankle + dir(foot) * FOOT;
            parts.push(Part::limb(hip, knee, THIGH_R, THIGH_R * TAPER));
            parts.push(Part::limb(knee, ankle, CALF_R, CALF_R * TAPER));
            parts.push(Part::limb(ankle, toe, FOOT_R, FOOT_R));
        }

        let (mut min, mut max) = (Vec2::MAX, Vec2::MIN);
        for part in &parts {
            let r = part.ra.max(part.rb);
            min = min.min(part.a.min(part.b) - r);
            max = max.max(part.a.max(part.b) + r);
        }
        Self {
            parts,
            centre: (min + max) / 2.0,
            chest: chest * 0.6,
        }
    }

    /// Точка ячейки (−1…1) в ростах.
    fn to_figure(&self, p: Vec2) -> Vec2 {
        p * (CELL_SPAN / 2.0) + self.centre
    }

    /// Тексель в точке `p` ячейки (−1…1): яркость и альфа; `edge` — полуширина
    /// сглаживания в единицах ячейки.
    pub fn texel(&self, p: Vec2, edge: f32) -> (f32, f32) {
        let f = self.to_figure(p);
        let edge = edge * CELL_SPAN / 2.0;
        let inside = -self
            .parts
            .iter()
            .map(|part| part.distance(f))
            .fold(f32::MAX, f32::min);
        let alpha = smoothstep(-edge, edge, inside);
        let shade = RIM_SHADE + (1.0 - RIM_SHADE) * smoothstep(RIM - edge, RIM + edge, inside);
        (shade, alpha)
    }

    /// Куда ложится лужа: грудь, в координатах ячейки (−1…1).
    pub fn pool_anchor(&self) -> Vec2 {
        (self.chest - self.centre) / (CELL_SPAN / 2.0)
    }
}

// --- Лужа ---
/// Средний радиус в долях полуячейки; с гармониками и брызгами край не
/// выходит за 0,9.
const POOL_RADIUS: f32 = 0.62;
/// Гармоники контура: порядок, амплитуда, фаза — пятно, а не круг.
const POOL_WAVES: [(f32, f32, f32); 3] = [(2.0, 0.10, 4.0), (3.0, 0.12, 0.7), (5.0, 0.07, 2.1)];
/// Брызги рядом: центр и радиус.
const POOL_DROPS: [(Vec2, f32); 3] = [
    (Vec2::new(0.78, 0.30), 0.09),
    (Vec2::new(-0.66, -0.50), 0.07),
    (Vec2::new(0.35, -0.80), 0.05),
];
/// Потемнение к середине: где крови больше, там она гуще.
const POOL_CORE_SHADE: f32 = 0.75;

/// Тексель лужи в точке `p` ячейки (−1…1).
pub fn pool_texel(p: Vec2, edge: f32) -> (f32, f32) {
    let r = p.length();
    let theta = p.y.atan2(p.x);
    let ripple: f32 = POOL_WAVES
        .iter()
        .map(|(order, amplitude, phase)| amplitude * (order * theta + phase).sin())
        .sum();
    let radius = POOL_RADIUS * (1.0 + ripple);
    let mut inside = radius - r;
    for (centre, drop) in POOL_DROPS {
        inside = inside.max(drop - (p - centre).length());
    }
    let alpha = smoothstep(-edge, edge, inside);
    let shade = 1.0 - (1.0 - POOL_CORE_SHADE) * (1.0 - r / radius).clamp(0.0, 1.0);
    (shade, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE: f32 = 2.0 / 128.0;

    #[test]
    fn capsule_distance_is_negative_inside_and_positive_outside() {
        let limb = Part::limb(Vec2::ZERO, Vec2::X, 0.2, 0.1);
        assert!(limb.distance(Vec2::new(0.5, 0.0)) < 0.0);
        assert!(limb.distance(Vec2::new(0.5, 0.5)) > 0.0);
        // концы — окружности своих радиусов
        assert!((limb.distance(Vec2::new(-0.2, 0.0))).abs() < 1e-5);
        assert!((limb.distance(Vec2::new(1.1, 0.0))).abs() < 1e-5);
    }

    #[test]
    fn every_pose_fits_its_cell_with_a_margin() {
        for index in 0..POSES {
            let figure = Figure::pose(index);
            for part in &figure.parts {
                for point in [part.a, part.b] {
                    let cell = (point - figure.centre) / (CELL_SPAN / 2.0);
                    let reach = cell.abs().max_element() + part.ra.max(part.rb) / (CELL_SPAN / 2.0);
                    assert!(reach < 0.95, "pose {index} reaches {reach} past its cell");
                }
            }
        }
    }

    #[test]
    fn every_pose_is_solid_at_the_pool_anchor_and_empty_at_the_corner() {
        for index in 0..POSES {
            let figure = Figure::pose(index);
            let (_, at_anchor) = figure.texel(figure.pool_anchor(), EDGE);
            assert_eq!(at_anchor, 1.0, "pose {index} has no body under its pool");
            let (_, at_corner) = figure.texel(Vec2::new(0.97, 0.97), EDGE);
            assert_eq!(at_corner, 0.0, "pose {index} leaks into the corner");
        }
    }

    #[test]
    fn the_figure_is_neither_a_blob_nor_a_thread() {
        // фигура — не пятно и не проволока: от восьмой до половины ячейки
        for index in 0..POSES {
            let figure = Figure::pose(index);
            let n = 64;
            let mut covered = 0;
            for y in 0..n {
                for x in 0..n {
                    let p = (Vec2::new(x as f32, y as f32) + 0.5) / n as f32 * 2.0 - 1.0;
                    if figure.texel(p, EDGE).1 > 0.5 {
                        covered += 1;
                    }
                }
            }
            let share = covered as f32 / (n * n) as f32;
            assert!(
                (0.12..0.5).contains(&share),
                "pose {index} covers {share} of its cell"
            );
        }
    }

    #[test]
    fn pool_is_solid_in_the_middle_darker_there_and_gone_at_the_edge() {
        let (core_shade, core_alpha) = pool_texel(Vec2::ZERO, EDGE);
        assert_eq!(core_alpha, 1.0);
        assert!(core_shade < 1.0);
        let (_, corner) = pool_texel(Vec2::new(0.97, 0.97), EDGE);
        assert_eq!(corner, 0.0);
    }
}
