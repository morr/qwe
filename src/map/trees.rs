//! Процедурные кроны деревьев в стиле Watabou Village Generator.
//! Алгоритм восстановлен из Village.js, подробный разбор — `TREE_ALGO.md`:
//! мятый 12-угольник → «bloat» (рекурсивное выдавливание середин рёбер) →
//! облачный контур; внутренние кольца-штрихи; тень — растянутый силуэт.

mod conifer;

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

pub use self::conifer::ConiferField;
use crate::loading::AppState;
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::MapData;
use crate::map::osm::model::signed_ring_area;
use crate::map::{SHADOW_COLOR, SHADOW_DIR};
use crate::settings::{
    TREE_DETAIL_STROKE, TREE_OUTLINE_STROKE, TREE_VARIANTS, Z_TREE, Z_TREE_SHADOW,
};

/// Чернила контура и штрихов (watabou `colorInk`).
const INK_COLOR: Color = Color::srgb(0.004, 0.008, 0.024);
/// Базовая зелень кроны; пер-дерево умножается на яркость из `TINT_FACTORS`.
const CROWN_COLOR: Color = Color::srgb(0.42, 0.60, 0.33);
/// Внутренние кольца штриховки облачной кроны — `BALL_BANDS2`.
const BALL_BANDS: [f32; 2] = [0.8, 0.5];
/// Кольца конической кроны — `CONE_BANDS3`.
const CONE_BANDS: [f32; 3] = [0.7, 0.4, 0.1];
/// Кольца пальмовой кроны — `PALM_BANDS2`.
const PALM_BANDS: [f32; 2] = [0.7, 0.3];

/// Вектор штриховки: рёбра CCW-колец вдоль него рисуются чаще (теневая сторона).
const SHADE_DIR: Vec2 = Vec2::new(0.5, 0.866_025_4);
/// Растяжение силуэта тени вдоль её оси (`1 + 0.5·shadowLength·0.8`).
const SHADOW_STRETCH: f32 = 1.4;
/// Обратный сдвиг силуэта тени (`−R·(1 − shadowLength/4)`).
const SHADOW_BACKSHIFT: f32 = -0.75;
/// «Высота» дерева `h` из `drawTree`: `0.4 + 0.8·gauss3` — она решает, длинная
/// тень или короткая, и на сколько радиусов вытянут веер у хвои. У watabou
/// значение на дерево, здесь — на вариант кроны (геометрия кэшируется по ним).
const SHADOW_HEIGHT_BASE: f32 = 0.4;
const SHADOW_HEIGHT_SPREAD: f32 = 0.8;
/// Порог `h`, выше которого крона отбрасывает длинную тень (`drawTree`:
/// `h·shadowLength > 0.5` при `shadowLength = 1`).
const LONG_SHADOW_HEIGHT: f32 = 0.5;
/// Ширина устья выреза, ниже которой обводка съедает просвет целиком: один
/// штрих уходит на чернила, второй — на видимую зелень между стенками.
const NOTCH_MOUTH_MIN: f32 = 2.0 * TREE_OUTLINE_STROKE;
/// Глубина выреза, ниже которой ямку целиком закрывает обводка: два шипа по её
/// краям сливаются в один горб с плоской верхушкой.
const NOTCH_DEPTH_MIN: f32 = TREE_OUTLINE_STROKE;
/// Пол высоты шипа контура. У watabou шип над коротким ребром выходит
/// непропорционально низким (`len^1.5`) — от таких шипов и вырезы мелкие, и
/// острия тупые.
const SPIKE_HEIGHT_MIN: f32 = 2.0 * TREE_OUTLINE_STROKE;
/// Ни один угол контура не должен стать уже этого: сдвиг, раскрывающий вырез,
/// не имеет права схлопнуть соседнее остриё в иглу.
const CORNER_MOUTH_FLOOR: f32 = TREE_OUTLINE_STROKE;
/// Предел сдвига вершины базы, которым раскрывается залипший вырез, — доля
/// радиуса кроны. Замерено: на 12 вариантах хватает 0.08.
const NOTCH_NUDGE_LIMIT: f32 = 0.15;
/// Шагов поиска сдвига: берётся наименьший из подходящих, чтобы силуэт менялся
/// как можно меньше.
const NOTCH_NUDGE_STEPS: usize = 50;

/// ГПСЧ Лемера (Park–Miller), как в Village.js: `seed = 48271·seed mod 2³¹−1`.
struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self((seed % 0x7FFF_FFFF).max(1))
    }

    fn next_f32(&mut self) -> f32 {
        self.0 = ((u64::from(self.0) * 48271) % 0x7FFF_FFFF) as u32;
        self.0 as f32 / 2_147_483_647.0
    }

    /// Среднее трёх uniform — колокол на (0,1) со средним 0.5.
    fn gauss3(&mut self) -> f32 {
        (self.next_f32() + self.next_f32() + self.next_f32()) / 3.0
    }

    /// Сумма четырёх uniform / 2 − 1 — колокол на (−1,1) со средним 0.
    fn bell4(&mut self) -> f32 {
        (self.next_f32() + self.next_f32() + self.next_f32() + self.next_f32()) / 2.0 - 1.0
    }
}

/// Геометрия строится только по конкретной форме: `Mixed` разрешается в
/// `Cotton`/`Conifer` ещё до неё — пул крон собирается по
/// [`TreeShape::crown_shapes`], форму дерева выдаёт [`TreeShape::resolve`].
const MIXED_HAS_NO_GEOMETRY: &str = "Mixed разрешается в конкретную форму до сборки кроны";

/// Форма кроны — `w.TREE_SHAPE` у watabou.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
pub enum TreeShape {
    /// `zb.cloud`: облачный контур (`Bloater`), кольца `BALL_BANDS2`.
    #[default]
    Cotton,
    /// `zb.pine`: колючий контур (`Spiker::simple`), кольца `CONE_BANDS3`.
    Conifer,
    /// `zb.palm`: изогнутые листья (`Spiker::bent`), кольца `PALM_BANDS2`.
    Palm,
    /// Смешанный лес: хвойные массивы среди облачных крон. Собственной
    /// геометрии не имеет — форму каждого дерева разрешает `resolve` по полю
    /// хвои (`conifer::ConiferField`).
    Mixed,
}

impl TreeShape {
    pub const ALL: [Self; 4] = [Self::Cotton, Self::Conifer, Self::Palm, Self::Mixed];
    /// Формы с собственной геометрией кроны — всё, кроме `Mixed`.
    #[cfg(test)]
    const CONCRETE: [Self; 3] = [Self::Cotton, Self::Conifer, Self::Palm];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cotton => "Cotton",
            Self::Conifer => "Conifer",
            Self::Palm => "Palm",
            Self::Mixed => "Mixed",
        }
    }

    /// Формы, для которых надо собрать меши под этот стиль.
    fn crown_shapes(self) -> &'static [Self] {
        match self {
            Self::Cotton => &[Self::Cotton],
            Self::Conifer => &[Self::Conifer],
            Self::Palm => &[Self::Palm],
            Self::Mixed => &[Self::Cotton, Self::Conifer],
        }
    }

    /// Конкретная форма кроны: у `Mixed` её решает поле хвои
    /// (`ConiferField::is_conifer` по мировым координатам ствола), остальные
    /// формы разрешаются в себя.
    fn resolve(self, conifer: bool) -> Self {
        match self {
            Self::Mixed if conifer => Self::Conifer,
            Self::Mixed => Self::Cotton,
            other => other,
        }
    }

    /// Вершин в базовом многоугольнике: у хвойной кроны их 16, у прочих 12.
    fn base_points(self) -> usize {
        match self {
            Self::Conifer => 16,
            Self::Cotton | Self::Palm => 12,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Доля радиуса, на которую джиттер утапливает вершину внутрь.
    fn radius_jitter(self) -> f32 {
        match self {
            Self::Conifer => 0.25,
            Self::Cotton | Self::Palm => 4.0 / 12.0,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Сдвиг колец к свету за номер кольца, доля радиуса. Константа **формы**,
    /// а не номера кольца: `-(n+1)·R·0.15` у облака, `·0.12` у хвои, `·0.1` у
    /// пальмы.
    fn band_lift(self) -> f32 {
        match self {
            Self::Cotton => 0.15,
            Self::Conifer => 0.12,
            Self::Palm => 0.1,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Масштабы внутренних колец и вес вероятности штриха для каждого.
    fn bands(self) -> Vec<(f32, f32)> {
        match self {
            Self::Cotton => BALL_BANDS.iter().map(|&s| (s, 3.0 * s * s)).collect(),
            Self::Conifer => CONE_BANDS.iter().map(|&s| (s, 0.5 + s)).collect(),
            Self::Palm => PALM_BANDS.iter().map(|&s| (s, 3.0 * s)).collect(),
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Контур из базового многоугольника: `Bloater::bloat` или `Spiker`.
    fn outline(self, ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
        match self {
            Self::Cotton => bloat(ring, lobe),
            Self::Conifer => spike_simple(ring, lobe, 0.0),
            Self::Palm => spike_bent(ring, lobe),
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Множитель `lobe` для внутренних колец: у облака фестоны кольца
    /// крупнее самого кольца (`k/scale`), у шипастых форм — как у контура.
    fn band_lobe(self, lobe: f32, scale: f32) -> f32 {
        match self {
            Self::Cotton => lobe / scale,
            Self::Conifer | Self::Palm => lobe,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }

    /// Штриховка кольца — у каждой формы своя процедура (`drawShaded1/2/4`).
    fn shade(self, ring: &[Vec2], weight: f32, rng: &mut Lcg) -> Vec<Vec<Vec2>> {
        match self {
            Self::Cotton => shaded_arcs(ring, weight, rng),
            Self::Conifer => chevron_arcs(ring, weight),
            Self::Palm => leaf_arcs(ring, weight, rng),
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }
}

/// Геометрия кроны единичного радиуса: контур и кольца штриховки.
struct CrownGeometry {
    /// Форма, которой кольца штрихуются и по которой ложится тень.
    shape: TreeShape,
    outer: Vec<Vec2>,
    /// (кольцо, вес вероятности штриха).
    bands: Vec<(Vec<Vec2>, f32)>,
}

/// `getCloudCrown` / `getPineCrown` / `getPalmCrown`: базовый многоугольник с
/// джиттером угла и радиуса, затем контур и уменьшенные кольца деталей.
fn crown_geometry(shape: TreeShape, rng: &mut Lcg) -> CrownGeometry {
    let points = shape.base_points();
    let mut base = Vec::with_capacity(points);
    for index in 0..points {
        let angle = TAU * (index as f32 + rng.gauss3()) / points as f32;
        let radius = 1.0 - shape.radius_jitter() * rng.bell4().abs();
        base.push(Vec2::from_angle(angle) * radius);
    }
    let lobe = (3.0 * PI / points as f32).sin();
    // правка залипших вырезов идёт только по контуру: кольца штриховки строятся
    // ниже из нетронутой базы. Облаку она не нужна — все его вырезы мельче
    // `NOTCH_DEPTH_MIN`; пальме вредна — её листья тонкие по замыслу
    let outer = if shape == TreeShape::Conifer {
        cone_outline(&base, lobe)
    } else {
        shape.outline(&base, lobe)
    };
    let bands = shape
        .bands()
        .into_iter()
        .enumerate()
        .map(|(number, (scale, weight))| {
            let lift = Vec2::new(0.0, (number as f32 + 1.0) * shape.band_lift());
            let ring: Vec<Vec2> = base.iter().map(|&point| point * scale + lift).collect();
            (shape.outline(&ring, shape.band_lobe(lobe, scale)), weight)
        })
        .collect();
    CrownGeometry {
        shape,
        outer,
        bands,
    }
}

/// `Spiker::simple`: между соседними вершинами вставлен один шип наружу
/// длиной `sqrt(len/lobe)·len` — хвойная «ёлочная» кромка.
///
/// `min_height` — пол высоты шипа; кольцам штриховки он не нужен (0.0), контуру
/// нужен, см. [`cone_outline`].
fn spike_simple(ring: &[Vec2], lobe: f32, min_height: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(ring.len() * 2);
    let mut previous = ring[ring.len() - 1];
    for &point in ring {
        let mut spike = spike_vector(previous, point, lobe);
        let height = spike.length();
        if height > f32::EPSILON && height < min_height {
            spike *= min_height / height;
        }
        out.push(previous);
        out.push(previous.midpoint(point) + spike);
        previous = point;
    }
    out
}

/// `Spiker::bent`: шип как у `simple`, но с двумя опорными точками — лист
/// изгибается и приподнят к свету, а не торчит прямой иглой.
fn spike_bent(ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(ring.len() * 4);
    let mut previous = ring[ring.len() - 1];
    for &point in ring {
        let spike = spike_vector(previous, point, lobe);
        let tip = previous.midpoint(point) + spike;
        // watabou смещает опорные точки на 0.1·|шип| «вверх по экрану»;
        // здесь y растёт вверх, поэтому знак положительный
        let lift = 0.1 * spike.length();
        out.push(previous);
        out.push(bend_control(previous, tip, lift));
        out.push(tip);
        out.push(bend_control(tip, point, lift));
        previous = point;
    }
    out
}

/// Вектор шипа над ребром: наружная нормаль, удлинённая как корень из
/// отношения длины ребра к `lobe`.
fn spike_vector(from: Vec2, to: Vec2, lobe: f32) -> Vec2 {
    let edge = to - from;
    let scaled = edge * (edge.length() / lobe).sqrt();
    Vec2::new(scaled.y, -scaled.x)
}

/// Опорная точка изгиба листа: середина отрезка, сдвинутая по нормали и вверх.
fn bend_control(from: Vec2, to: Vec2, lift: f32) -> Vec2 {
    let normal = Vec2::new(from.y - to.y, to.x - from.x);
    from.midpoint(to) - normal * 0.1 + Vec2::new(0.0, lift)
}

/// Поток случайных чисел варианта кроны: крона и её тень разыгрываются из него
/// подряд, так что вариант полностью задан своим номером.
fn variant_rng(variant: usize) -> Lcg {
    Lcg::new(0x051E_D2E5 + variant as u32 * 7919)
}

/// Контур хвойной кроны, у которого **каждый вырез читается под обводкой**.
/// Обводка шириной `TREE_OUTLINE_STROKE` (12% радиуса) съедает вырез двумя
/// способами, и оба дают на глаз один и тот же «горб» вместо двух шипов:
///
/// - вырез **уже** `NOTCH_MOUTH_MIN` — чернила смыкаются поперёк, и он читается
///   иглой внутрь кроны;
/// - вырез **мельче** `NOTCH_DEPTH_MIN` — ямку закрывает сама линия, и остаётся
///   плоская верхушка между двумя тупыми остриями.
///
/// Мелкие вырезы лечит `SPIKE_HEIGHT_MIN`: у watabou высота шипа растёт как
/// `len^1.5`, поэтому над коротким ребром он выходит непропорционально низким —
/// пол высоты и поднимает такие шипы, и заодно заостряет их (острия 51–88° →
/// 35–67°). Узкие лечит сдвиг виновной вершины базы поперёк хорды её соседей:
/// шипы над двумя соседними рёбрами тем сильнее расходятся, чем круче излом
/// базы между ними. Сдвиг берётся наименьший из подходящих — замерено, хватает
/// восьми сотых радиуса, и вылет шипов (1.31–1.55) от него не меняется.
///
/// Убирать лишний шип нельзя ни с контура (у соседа база расширяется вдвое —
/// остриё тупеет до 107°), ни с базы (объединённое ребро вдвое длиннее, шип
/// растёт как `len^1.5`, вылет уходит к 1.9 радиуса).
///
/// Отступление от watabou: у него обводка фиксированной ширины в мировых
/// единицах и город смотрят издалека, так что вырождение не видно. Кроны
/// облака и пальмы мимо: их мелкая рябь по замыслу тонет в чернилах.
fn cone_outline(base: &[Vec2], lobe: f32) -> Vec<Vec2> {
    let mut base = base.to_vec();
    loop {
        let ring = spike_simple(&base, lobe, SPIKE_HEIGHT_MIN);
        let swallowed = swallowed_notches(&ring);
        if swallowed.is_empty() {
            return ring;
        }
        // за круг раскрывается один вырез: их число строго убывает, поэтому
        // цикл конечен
        if !swallowed
            .iter()
            .any(|&vertex| nudge_notch_open(&mut base, vertex, lobe, swallowed.len()))
        {
            return spike_simple(&base, lobe, SPIKE_HEIGHT_MIN);
        }
    }
}

/// Вершины базы, вырез над которыми обводка съедает — поперёк или по глубине.
fn swallowed_notches(ring: &[Vec2]) -> Vec<usize> {
    let count = ring.len() / 2;
    // впадины — чётные вершины контура: `spike_simple` кладёт вершину базы,
    // затем шип над ребром, которое из неё выходит
    (0..ring.len())
        .step_by(2)
        .filter(|&index| {
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            corner_metrics(previous, ring[index], ring[index + 1]).is_some_and(
                |(mouth, depth, valley)| {
                    valley && (mouth < NOTCH_MOUTH_MIN || depth < NOTCH_DEPTH_MIN)
                },
            )
        })
        .map(|index| (index / 2 + count - 1) % count)
        .collect()
}

/// Сдвигает вершину базы поперёк хорды её соседей на наименьшее смещение, при
/// котором съеденных вырезов становится меньше, а ни один угол контура не
/// сужается за `CORNER_MOUTH_FLOOR`; пробуются оба направления. Без такого
/// смещения вершина остаётся на месте.
fn nudge_notch_open(base: &mut [Vec2], vertex: usize, lobe: f32, swallowed: usize) -> bool {
    let count = base.len();
    let chord = base[(vertex + 1) % count] - base[(vertex + count - 1) % count];
    let step = chord.perp().normalize_or_zero() * (NOTCH_NUDGE_LIMIT / NOTCH_NUDGE_STEPS as f32);
    let origin = base[vertex];
    for offset in 1..=NOTCH_NUDGE_STEPS {
        for direction in [1.0_f32, -1.0] {
            base[vertex] = origin + step * (offset as f32 * direction);
            let ring = spike_simple(base, lobe, SPIKE_HEIGHT_MIN);
            if swallowed_notches(&ring).len() < swallowed
                && narrowest_corner(&ring) >= CORNER_MOUTH_FLOOR
            {
                return true;
            }
        }
    }
    base[vertex] = origin;
    false
}

/// Самый узкий угол контура — и впадина, и остриё.
fn narrowest_corner(ring: &[Vec2]) -> f32 {
    (0..ring.len())
        .filter_map(|index| {
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            let next = ring[(index + 1) % ring.len()];
            corner_metrics(previous, ring[index], next).map(|(mouth, _, _)| mouth)
        })
        .fold(f32::INFINITY, f32::min)
}

/// Угол контура в вершине: ширина устья (просвет между плечами на высоте
/// ближнего из них), высота (отступ вершины от хорды между соседями) и признак
/// впадины. Обход контура CCW, поэтому впадина та, где плечи разворачиваются
/// против часовой. `None` — вырожденная вершина.
fn corner_metrics(previous: Vec2, vertex: Vec2, next: Vec2) -> Option<(f32, f32, bool)> {
    let (to_previous, to_next) = (previous - vertex, next - vertex);
    let chord = next - previous;
    if chord.length() < f32::EPSILON
        || to_previous.length() < f32::EPSILON
        || to_next.length() < f32::EPSILON
    {
        return None;
    }
    let angle = to_previous.angle_to(to_next).abs();
    let arm = to_previous.length().min(to_next.length());
    let depth = chord.perp_dot(vertex - previous).abs() / chord.length();
    let valley = to_previous.perp_dot(to_next) > 0.0;
    Some((2.0 * arm * (angle / 2.0).sin(), depth, valley))
}

/// `Bloater.bloat`: каждое ребро кольца — в цепочку наружных горбов.
fn bloat(ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    for index in 0..ring.len() {
        extrude_edge(ring[index], ring[(index + 1) % ring.len()], lobe, &mut out);
    }
    out
}

/// `Bloater.extrudeEx`: середина ребра выталкивается наружу по перпендикуляру
/// на `0.5·min(len/lobe, 1)·len`, рекурсивно до сегментов короче `0.1·lobe`.
fn extrude_edge(a: Vec2, b: Vec2, lobe: f32, out: &mut Vec<Vec2>) {
    let d = a - b;
    let ratio = d.length() / lobe;
    if ratio <= 0.1 {
        out.push(a);
        return;
    }
    let mid = a.midpoint(b) + Vec2::new(-d.y, d.x) * (0.5 * ratio.min(1.0));
    extrude_edge(a, mid, lobe, out);
    extrude_edge(mid, b, lobe, out);
}

/// `drawLongShadow`: силуэт кроны, растянутый вдоль оси тени и сдвинутый по ней.
fn shadow_ring(outer: &[Vec2]) -> Vec<Vec2> {
    outer
        .iter()
        .map(|&point| {
            let local = Vec2::new(point.dot(SHADOW_DIR), point.perp_dot(SHADOW_DIR));
            let x = (local.x + 1.0) * SHADOW_STRETCH + SHADOW_BACKSHIFT;
            SHADOW_DIR * x + SHADOW_DIR.perp() * -local.y
        })
        .collect()
}

/// Меш кроны: заливка + чернильный контур + штрихованные кольца (процедура
/// штриховки своя у каждой формы, см. [`TreeShape::shade`]).
/// Вершинные цвета настоящие; материал дерева умножает их на серый множитель,
/// так зелень варьируется, а чернила остаются чернилами.
fn crown_mesh(geometry: &CrownGeometry, style: &TreeStyle, rng: &mut Lcg) -> Mesh {
    let mut builder = MeshBuilder::default();
    let ink = style.details.to_linear();
    builder.push_polygon(&geometry.outer, &[], style.foliage.to_linear());
    builder.push_stroke(&geometry.outer, true, TREE_OUTLINE_STROKE, ink);

    for (ring, weight) in &geometry.bands {
        for arc in geometry.shape.shade(ring, *weight, rng) {
            // круглые стыки, а не miter: «этаж» разворачивается на кончике
            // каждого шипа почти на 180°, и срезанный miter оставлял бы там
            // клин пустоты — ломаная читалась бы рваной. Контур кроны рисуется
            // прежним miter: у него излом наружу, и острия должны быть острыми
            builder.push_ribbon(
                &arc,
                false,
                TREE_DETAIL_STROKE,
                ink,
                RibbonJoin::Round,
                RibbonCap::Butt,
            );
        }
    }
    builder.build()
}

/// Сборка дуг из отбора: `drawn[i]` — рисуется ли кусок кольца из `step`
/// рёбер, начинающийся в `ring[i·step]`. Подряд идущие куски склеиваются в
/// одну ломаную (соседние делят вершину) — иначе каждый рисуется своим
/// штрихом с собственными торцами, и кольцо распадается на зубцы. Обход
/// стартует с пропущенного куска, поэтому дуга, пересекающая начало кольца,
/// остаётся целой, а не разваливается на два огрызка.
fn chain_arcs(ring: &[Vec2], step: usize, drawn: &[bool]) -> Vec<Vec<Vec2>> {
    let Some(gap) = drawn.iter().position(|&on| !on) else {
        // нарисовано всё кольцо — одна замкнутая ломаная
        let mut arc = ring.to_vec();
        arc.push(ring[0]);
        return vec![arc];
    };
    let mut arcs: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for offset in 0..drawn.len() {
        let chunk = (gap + offset) % drawn.len();
        if drawn[chunk] {
            if current.is_empty() {
                current.push(ring[chunk * step]);
            }
            current.extend((1..=step).map(|edge| ring[(chunk * step + edge) % ring.len()]));
        } else if !current.is_empty() {
            arcs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        arcs.push(current);
    }
    arcs
}

/// `drawShaded1` — штриховка облачной кроны. Вероятность нарисовать ребро тем
/// выше, чем ближе его направление к `SHADE_DIR`: штрихи скапливаются на
/// теневой стороне кольца. Кольцо `0.8` (вес 1.92) выходит одной длинной дугой
/// по теневой стороне плюс россыпь коротких штрихов по краям — их **надо**
/// рисовать, иначе кольцо читается как рваное. Отбрасываются только дуги
/// короче собственной толщины: такая при зуме уже не штрих, а квадратик.
fn shaded_arcs(ring: &[Vec2], weight: f32, rng: &mut Lcg) -> Vec<Vec<Vec2>> {
    let drawn: Vec<bool> = (0..ring.len())
        .map(|index| {
            let edge = ring[(index + 1) % ring.len()] - ring[index];
            edge.try_normalize().is_some_and(|direction| {
                rng.next_f32() < weight * (0.5 + 0.5 * SHADE_DIR.dot(direction))
            })
        })
        .collect();
    let mut arcs = chain_arcs(ring, 1, &drawn);
    arcs.retain(|arc| {
        arc.windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum::<f32>()
            >= TREE_DETAIL_STROKE
    });
    arcs
}

/// `drawShaded2` — штриховка хвойной кроны. Кольцо после [`spike_simple`] идёт
/// «вершина базы, кончик шипа, вершина базы, …», и рисуется оно **шевронами**:
/// шаг 2, кусок база→шип→база целиком. Направление берётся по хорде между
/// вершинами базы, а не по ребру шипа, и **лотереи нет** — условие
/// `w·(0.5 + 0.5·cos) > 0.5` либо выполнено, либо нет. Поэтому соседние
/// шевроны смыкаются в сплошную ломаную-«этаж» на теневой стороне, а светлая
/// остаётся чистой. Веса `0.5 + scale` дают дуги примерно в 190°, 167° и 96°;
/// последняя, у кольца `0.1`, поднятого к макушке, и есть верхушка ели.
fn chevron_arcs(ring: &[Vec2], weight: f32) -> Vec<Vec<Vec2>> {
    let drawn: Vec<bool> = (0..ring.len() / 2)
        .map(|chevron| {
            let chord = ring[(chevron * 2 + 2) % ring.len()] - ring[chevron * 2];
            chord
                .try_normalize()
                .is_some_and(|direction| weight * (0.5 + 0.5 * SHADE_DIR.dot(direction)) > 0.5)
        })
        .collect();
    chain_arcs(ring, 2, &close_single_gaps(&drawn))
}

/// Дырка ровно в один шеврон посреди прогона: джиттер угла качнул одну хорду
/// за порог, и «этаж» распадается надвое. У watabou она так и остаётся, но у
/// него дерево — сорок пикселей, а здесь на зуме это читается ровно как тот
/// разрыв, ради которого всё и затевалось. Оба соседа нарисованы — рисуем и её.
fn close_single_gaps(drawn: &[bool]) -> Vec<bool> {
    let count = drawn.len();
    (0..count)
        .map(|index| {
            drawn[index] || (drawn[(index + count - 1) % count] && drawn[(index + 1) % count])
        })
        .collect()
}

/// `drawShaded4` — штриховка пальмы. Кольцо после [`spike_bent`] тратит по
/// четыре точки на лист, и лист рисуется целиком: шаг 4, направление по хорде
/// между основаниями соседних листьев, вероятность как у `drawShaded1`.
fn leaf_arcs(ring: &[Vec2], weight: f32, rng: &mut Lcg) -> Vec<Vec<Vec2>> {
    let drawn: Vec<bool> = (0..ring.len() / 4)
        .map(|leaf| {
            let chord = ring[(leaf * 4 + 4) % ring.len()] - ring[leaf * 4];
            chord.try_normalize().is_some_and(|direction| {
                rng.next_f32() < weight * (0.5 + 0.5 * SHADE_DIR.dot(direction))
            })
        })
        .collect();
    chain_arcs(ring, 4, &drawn)
}

/// Силуэт тени единичного радиуса — шаблон, который `spawn_trees` кладёт в
/// общий меш теней под каждое дерево этого варианта. Тип тени выбирает
/// «высота» `h` (`drawTree`): у облака и пальмы высокая крона даёт длинную
/// тень, низкая — простой сдвинутый силуэт; ель вместо этого отбрасывает
/// веер-конус. `h` разыгрывается на вариант, так что соседние деревья стоят с
/// тенями разной длины.
fn shadow_template(geometry: &CrownGeometry, rng: &mut Lcg) -> MeshBuilder {
    let height = SHADOW_HEIGHT_BASE + SHADOW_HEIGHT_SPREAD * rng.gauss3();
    let mut builder = MeshBuilder::default();
    match geometry.shape {
        TreeShape::Conifer => {
            for (outer, holes) in conifer_shadow(&geometry.outer, height) {
                builder.push_polygon(&outer, &holes, LinearRgba::WHITE);
            }
        }
        _ if height > LONG_SHADOW_HEIGHT => {
            builder.push_polygon(&shadow_ring(&geometry.outer), &[], LinearRgba::WHITE);
        }
        // `drawSimpleShadow`: тот же силуэт, просто сдвинутый по тени
        _ => {
            let offset = SHADOW_DIR * height;
            let ring: Vec<Vec2> = geometry.outer.iter().map(|&p| p + offset).collect();
            builder.push_polygon(&ring, &[], LinearRgba::WHITE);
        }
    }
    builder
}

/// `drawConiferShadow`: тень ели — не растянутый силуэт, а **конус**.
/// Треугольник от ствола (± радиус поперёк тени) к дальнему концу `3h`
/// задаёт ствол конуса, а поверх вдоль тени ложатся копии кроны убывающего
/// масштаба — получается ярусная «ёлка» вместо кляксы.
///
/// Фигуры объединяются булевым union: слой теней полупрозрачный, и наложение
/// копий внутри одного дерева читалось бы пятнами двойной темноты — та же
/// причина, по которой union стоит у теней зданий
/// (`buildings::layers::shadow_builder`). Считается один раз на вариант.
fn conifer_shadow(outer: &[Vec2], height: f32) -> Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let tip = SHADOW_DIR * 3.0 * height;
    let across = SHADOW_DIR.perp();
    let steps = (3.0 * height).ceil().max(1.0);
    let mut parts: Vec<Vec<Vec2>> = vec![vec![across, -across, tip]];
    for step in 0..steps as usize {
        let scale = 1.0 - step as f32 / steps;
        let offset = tip * (step as f32 + 1.0) / (steps + 1.0);
        parts.push(outer.iter().map(|&point| point * scale + offset).collect());
    }

    let parts: Vec<Vec<[f32; 2]>> = parts
        .into_iter()
        .map(|mut ring| {
            // NonZero гасит контуры противоположного обхода — все части
            // обязаны быть закручены одинаково
            if signed_ring_area(&ring) < 0.0 {
                ring.reverse();
            }
            ring.into_iter().map(|point| [point.x, point.y]).collect()
        })
        .collect();

    parts
        .simplify_shape(FillRule::NonZero)
        .into_iter()
        .filter_map(|shape| {
            let mut rings = shape.into_iter().map(|contour| {
                contour
                    .into_iter()
                    .map(Vec2::from_array)
                    .collect::<Vec<Vec2>>()
            });
            let outer = rings.next()?;
            Some((outer, rings.collect()))
        })
        .collect()
}

/// Стиль деревьев — вкладка Trees из «Style settings» watabou. Меняется на
/// лету из UI-панели; каждое изменение пересобирает кроны (`rebuild_trees`).
#[derive(Resource, Reflect, SettingsGroup, Clone, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "trees")]
pub struct TreeStyle {
    /// Цвет листвы (`colorTree` / «Foliage»).
    pub foliage: Color,
    /// Цвет контура и штрихов (`colorTreeDetails` / «Crown details»).
    pub details: Color,
    /// Разброс яркости листвы (`treeVariance`): множитель `2^(variance·bell)`.
    pub variance: f32,
    pub shape: TreeShape,
    /// Доля хвои при форме `Mixed`, 0..1. Доля точная: порог поля хвои —
    /// квантиль его значений в деревьях (см. [`ConiferField::set_share`]).
    /// На прочих формах не используется.
    pub conifer_share: f32,
    /// Плотность посадки, множитель к базовой (`TREE_DENSITY_MIN..MAX`):
    /// `1` — одно дерево на `TREE_AREA_PER_TREE` (410 м²) леса.
    /// `map::osm::planting` засаживает лес сразу по `TREE_DENSITY_MAX`, а спавн
    /// показывает префикс набора (см. [`visible_count`]) — деревья при движении
    /// ползунка не пересаживаются, а появляются и исчезают.
    pub density: f32,
}

impl Default for TreeStyle {
    fn default() -> Self {
        Self {
            foliage: CROWN_COLOR,
            details: INK_COLOR,
            variance: 0.2,
            shape: TreeShape::default(),
            conifer_share: 0.1,
            density: 1.0,
        }
    }
}

impl TreeStyle {
    /// Квантованные множители яркости: `2^(variance·bell)` для bell от −1 до 1.
    fn tint_factors(&self) -> [f32; 5] {
        [-1.0, -0.5, 0.0, 0.5, 1.0].map(|bell: f32| 2.0_f32.powf(self.variance * bell))
    }
}

/// Сколько деревьев показать при такой плотности: `MapData::trees`
/// отсортированы по плотности появления, так что нужен префикс, а не фильтр.
/// Доля каждого леса при этом точна — порог посчитан от его площади, — и
/// прореживание монотонно: шаг ползунка вверх только добавляет деревья, уже
/// стоящие не переезжают.
///
/// Породе прореживание ортогонально: её решает поле хвои по координатам, так
/// что доля хвои в прореженном наборе та же, а дерево при движении ползунка
/// плотности породу не меняет.
fn visible_count(appears_at: &[f32], density: f32) -> usize {
    appears_at.partition_point(|&at| at <= density)
}

/// Крона или её тень — чтобы пересборка стиля знала, что деспавнить.
#[derive(Component)]
pub struct TreeTag;

/// Спавн деревьев: `TREE_VARIANTS` крон единичного радиуса, каждому дереву —
/// вариант, оттенок и масштаб детерминированно по индексу; ползунок плотности
/// отдаёт префикс набора (см. [`visible_count`]). Кроны — сущность на
/// дерево (свой оттенок и свой z), тени — **один слитый меш на все деревья**:
/// полупрозрачная сущность попадает в сортируемую фазу `Transparent2d`, а
/// тысяча таких сущностей в ней вместе с двадцатью тысячами спрайтов пешеходов
/// теряется по одной-две на кадр (тень мигает). Слой из одного меша — как
/// `building_shadows` — этой фазе не по зубам и рисуется одним вызовом.
pub fn spawn_trees(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    style: &TreeStyle,
    trees: &[(Vec2, f32)],
    appears_at: &[f32],
    field: &ConiferField,
) {
    // по пулу вариантов на каждую конкретную форму — у `Mixed` их два
    let pools: Vec<(TreeShape, Vec<(Handle<Mesh>, MeshBuilder)>)> = style
        .shape
        .crown_shapes()
        .iter()
        .map(|&shape| {
            let variants = (0..TREE_VARIANTS)
                .map(|variant| {
                    let mut rng = variant_rng(variant);
                    let geometry = crown_geometry(shape, &mut rng);
                    (
                        meshes.add(crown_mesh(&geometry, style, &mut rng)),
                        shadow_template(&geometry, &mut rng),
                    )
                })
                .collect();
            (shape, variants)
        })
        .collect();
    let tints: Vec<Handle<ColorMaterial>> = style
        .tint_factors()
        .iter()
        .map(|&factor| materials.add(Color::srgb(factor, factor, factor)))
        .collect();

    let mut shadows = MeshBuilder::default();
    let visible = visible_count(appears_at, style.density);
    for (index, &(position, radius)) in trees.iter().take(visible).enumerate() {
        let shape = style.shape.resolve(field.is_conifer(index));
        let variants = &pools
            .iter()
            .find(|(pooled, _)| *pooled == shape)
            .expect("crown_shapes covers every shape resolve can return")
            .1;
        let (crown, shadow) = &variants[index % variants.len()];
        // микрошаг по z: пересекающиеся кроны рисуются в стабильном порядке
        let z = Z_TREE + (index % 512) as f32 * 1e-3;
        commands.spawn((
            TreeTag,
            Mesh2d(crown.clone()),
            MeshMaterial2d(tints[(index * 7) % tints.len()].clone()),
            Transform::from_translation(position.extend(z)).with_scale(Vec3::splat(radius)),
            DespawnOnExit(AppState::Playing),
            Name::new("tree"),
        ));
        shadows.push_template(shadow, position, radius);
    }

    if !shadows.is_empty() {
        // слой теней — один меш на весь лес, и веер хвои весит вчетверо против
        // одиночного силуэта: при разборе просадок смотреть в первую очередь сюда
        debug!(
            "tree shadows: {} vertices for {visible} trees ({:?})",
            shadows.vertex_count(),
            style.shape
        );
        commands.spawn((
            TreeTag,
            Mesh2d(meshes.add(shadows.build())),
            MeshMaterial2d(materials.add(SHADOW_COLOR)),
            Transform::from_xyz(0.0, 0.0, Z_TREE_SHADOW),
            DespawnOnExit(AppState::Playing),
            Name::new("tree_shadows"),
        ));
    }
}

/// Значение поля хвои в каждом посаженном дереве — до спавна крон, потому что
/// именно по нему `resolve` выбирает форму. Считается один раз на город: у
/// нового города свой набор деревьев, а старые значения к нему не относятся.
pub fn build_conifer_field(
    mut field: ResMut<ConiferField>,
    map: Res<MapData>,
    style: Res<TreeStyle>,
) {
    let started = std::time::Instant::now();
    field.resample(&map.trees);
    field.set_share(style.conifer_share);
    debug!(
        "conifer field: {} trees sampled in {:.1?}",
        map.trees.len(),
        started.elapsed()
    );
}

/// Пересборка крон после правки стиля из UI: деспавн старых сущностей и
/// повторный спавн из тех же позиций (`MapData::trees` не трогается).
pub fn rebuild_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    style: Res<TreeStyle>,
    map: Res<MapData>,
    mut field: ResMut<ConiferField>,
    existing: Query<Entity, With<TreeTag>>,
) {
    // порог поля пересчитывается только если поехала сама доля — правка цвета
    // листвы не должна платить за сортировку значений
    field.set_share(style.conifer_share);
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_trees(
        &mut commands,
        &mut meshes,
        &mut materials,
        &style,
        &map.trees,
        &map.tree_appears_at,
        &field,
    );
}

#[cfg(test)]
mod tests;
