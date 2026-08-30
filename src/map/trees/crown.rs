//! Геометрия кроны: мятый многоугольник → «bloat» → облачный контур, кольца
//! штрихов и растянутый силуэт тени. Разбор алгоритма —
//! `.claude/skills/osm-map/references/tree-algo.md`.
//! Стили, спавн и пересборка живут в [`super`].

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use super::{TreeShape, TreeStyle};
use crate::map::SHADOW_DIR;
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::model::signed_ring_area;
use crate::settings::{TREE_DETAIL_STROKE, TREE_OUTLINE_STROKE};

/// Чернила контура и штрихов (watabou `colorInk`).
pub(super) const INK_COLOR: Color = Color::srgb(0.004, 0.008, 0.024);
/// Базовая зелень кроны; пер-дерево умножается на яркость из `TINT_FACTORS`.
pub(super) const CROWN_COLOR: Color = Color::srgb(0.42, 0.60, 0.33);
/// Внутренние кольца штриховки облачной кроны — `BALL_BANDS2`.
const BALL_BANDS: [f32; 2] = [0.8, 0.5];
/// Кольца конической кроны — `CONE_BANDS3`.
pub(super) const CONE_BANDS: [f32; 3] = [0.7, 0.4, 0.1];
/// Кольца пальмовой кроны — `PALM_BANDS2`.
pub(super) const PALM_BANDS: [f32; 2] = [0.7, 0.3];

/// Вектор штриховки: рёбра CCW-колец вдоль него рисуются чаще (теневая сторона).
///
/// Это тот же источник света, что отбрасывает тени на всей карте, записанный
/// иначе: у ребра CCW-кольца нормаль перпендикулярна его направлению, поэтому
/// «ребро вдоль [`SHADE_DIR`]» и «нормаль вдоль [`SHADOW_DIR`]» — одно условие.
/// Выводится, а не пишется числом: солнце, повёрнутое в `map/mod.rs`, иначе
/// развернуло бы тени, оставив штриховку крон на старой стороне.
const SHADE_DIR: Vec2 = Vec2::new(-SHADOW_DIR.y, SHADOW_DIR.x);
/// Предел сдвига вершины базы, которым раскрывается залипший вырез, — доля
/// радиуса кроны. Замерено: на 12 вариантах хватает 0.08.
const NOTCH_NUDGE_LIMIT: f32 = 0.15;
/// Шагов поиска сдвига: берётся наименьший из подходящих, чтобы силуэт менялся
/// как можно меньше.
const NOTCH_NUDGE_STEPS: usize = 50;
/// Сид, с которого начинается нумерация вариантов кроны.
const VARIANT_SEED_BASE: u32 = 0x051E_D2E5;
/// Шаг сида между вариантами — простое число, чтобы соседние варианты не
/// оказались на близких участках потока Лемера.
const VARIANT_SEED_STRIDE: u32 = 7919;
/// Шаг сида между **наборами** ([`CrownParams::seed`]). Обязан не быть кратен
/// [`VARIANT_SEED_STRIDE`], иначе `seed` не пересоздаёт набор, а прокручивает
/// его: при шаге, равном шагу вариантов, `BASE + seed·s + variant·s` — это
/// `BASE + (seed + variant)·s`, то есть тот же ряд крон, сдвинутый на клетку,
/// и кривой вариант от такого «пересева» просто переезжает к соседу. Взято
/// обратное золотое сечение — нечётное и с шагом вариантов общих делителей не
/// имеющее.
const CROWN_SEED_STRIDE: u32 = 0x9E37_79B9;

/// Ручки генерации кроны — всё, что решает, как крона выглядит, до цвета.
/// Дефолт равен константам, на которых нарисован город: `CrownParams::default()`
/// — это ровно тот вид, что в игре, и любая правка поля читается как «на
/// столько отступили от игры».
///
/// Часть ручек — **множители** к величине, которая у каждой формы своя (вершин
/// базы 12 против 16 у хвои, джиттер 1/3 против 1/4, подъём колец 0.15/0.12/0.1):
/// один множитель двигает все три формы, сохраняя разницу между ними, тогда как
/// абсолютное поле стёрло бы её и потребовало бы по ручке на форму.
/// Остальные — абсолютные доли радиуса кроны, у которых своего значения по
/// форме нет.
///
/// Ресурс — ради витрины `tree_gallery`, которая крутит их вживую; игра берёт
/// [`CrownParams::default`] и панели для них не заводит.
#[derive(Resource, Reflect, Clone, Debug)]
#[reflect(Resource, Default)]
pub struct CrownParams {
    /// Множитель числа вершин базового многоугольника (12 у облака и пальмы,
    /// 16 у хвои). Вершины — это лопасти контура: чем их больше, тем мельче
    /// фестоны облака и тем гуще игольчатая кромка ели.
    pub points: f32,
    /// Множитель джиттера радиуса: на какую долю радиуса вершина базы
    /// утапливается внутрь. Ноль — правильный многоугольник, крона выходит
    /// штампованной.
    pub radius_jitter: f32,
    /// Множитель `lobe` — «крупности» выступа над ребром. Управляет и
    /// раздуванием облака (`bloat`), и длиной шипов ели и листьев пальмы:
    /// у обоих выступ растёт как корень из отношения длины ребра к `lobe`,
    /// поэтому **меньший** `lobe` даёт **более** пышный контур.
    pub lobe: f32,
    /// Множитель подъёма колец штриховки к свету.
    pub band_lift: f32,
    /// Множитель масштаба колец штриховки: раздвигает или стягивает их к
    /// контуру.
    pub band_scale: f32,
    /// Множитель веса штриховки — вероятности (у хвои: условия) нарисовать
    /// кусок кольца. Ноль — чистая заливка без штрихов, большие значения
    /// замыкают кольца целиком и с освещённой стороны.
    pub shade_weight: f32,
    /// Толщина чернильной обводки контура, доля радиуса. От неё же считаются
    /// пороги «вырез съеден обводкой» — см. [`cone_outline`].
    pub outline_stroke: f32,
    /// Толщина штрихов внутренних колец, доля радиуса.
    pub detail_stroke: f32,
    /// Множитель пола высоты шипа хвойного контура (сам пол — две обводки).
    pub spike_floor: f32,
    /// Растяжение силуэта длинной тени вдоль её оси.
    pub shadow_stretch: f32,
    /// Обратный сдвиг силуэта длинной тени по той же оси.
    pub shadow_backshift: f32,
    /// «Высота» кроны `h` из `drawTree`: `base + spread·gauss3`. Она решает,
    /// длинная тень или короткая, и на сколько радиусов вытянут веер у хвои.
    /// У watabou значение на дерево, здесь — на вариант кроны (геометрия
    /// кэшируется по вариантам).
    pub shadow_height_base: f32,
    pub shadow_height_spread: f32,
    /// Порог `h`, выше которого крона отбрасывает длинную тень, а не сдвинутый
    /// силуэт. Хвои не касается: у неё тень всегда конус-веер.
    pub long_shadow_height: f32,
    /// Номер набора крон: те же правила, но другие `TREE_VARIANTS` силуэтов
    /// целиком. Отдельный вариант пересеять нельзя — набор задан одним сидом, а
    /// вариант внутри него своим номером, поэтому неудачный силуэт убирается
    /// только сменой всего набора. Этим ползунком в витрине `tree_gallery`
    /// набор города и выбран: см. [`CrownParams::default`].
    pub seed: u32,
}

impl Default for CrownParams {
    fn default() -> Self {
        Self {
            points: 1.0,
            radius_jitter: 1.0,
            lobe: 1.0,
            band_lift: 1.0,
            band_scale: 1.0,
            shade_weight: 1.0,
            outline_stroke: TREE_OUTLINE_STROKE,
            detail_stroke: TREE_DETAIL_STROKE,
            spike_floor: 1.0,
            shadow_stretch: 1.4,
            shadow_backshift: -0.75,
            shadow_height_base: 0.4,
            shadow_height_spread: 0.8,
            long_shadow_height: 0.5,
            // набор крон города, выбран глазами в витрине `tree_gallery`. Не
            // ноль: в нулевом наборе хвойный вариант #6 выходил зализанным с
            // одной стороны — вершины базы легли слишком ровно, шипы над
            // короткими рёбрами вышли низкими (высота растёт как `len^1.5`), и
            // среди одинаковых ёлок он читался как поломка рендера. Отдельный
            // вариант не пересеять, поэтому меняется набор целиком
            seed: 5,
        }
    }
}

impl CrownParams {
    /// Ширина устья выреза, ниже которой обводка съедает просвет целиком: один
    /// штрих уходит на чернила, второй — на видимую зелень между стенками.
    pub(super) fn notch_mouth_min(&self) -> f32 {
        2.0 * self.outline_stroke
    }

    /// Глубина выреза, ниже которой ямку целиком закрывает обводка: два шипа по
    /// её краям сливаются в один горб с плоской верхушкой.
    pub(super) fn notch_depth_min(&self) -> f32 {
        self.outline_stroke
    }

    /// Пол высоты шипа контура. У watabou шип над коротким ребром выходит
    /// непропорционально низким (`len^1.5`) — от таких шипов и вырезы мелкие, и
    /// острия тупые.
    fn spike_height_min(&self) -> f32 {
        2.0 * self.outline_stroke * self.spike_floor
    }

    /// Ни один угол контура не должен стать уже этого: сдвиг, раскрывающий
    /// вырез, не имеет права схлопнуть соседнее остриё в иглу.
    pub(super) fn corner_mouth_floor(&self) -> f32 {
        self.outline_stroke
    }
}

/// ГПСЧ Лемера (Park–Miller), как в Village.js: `seed = 48271·seed mod 2³¹−1`.
pub(super) struct Lcg(u32);

impl Lcg {
    pub(super) fn new(seed: u32) -> Self {
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

impl TreeShape {
    /// Формы, для которых надо собрать меши под этот стиль.
    pub(super) fn crown_shapes(self) -> &'static [Self] {
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
    pub(super) fn resolve(self, conifer: bool) -> Self {
        match self {
            Self::Mixed if conifer => Self::Conifer,
            Self::Mixed => Self::Cotton,
            other => other,
        }
    }

    /// Вершин в базовом многоугольнике: у хвойной кроны их 16, у прочих 12,
    /// умножено на [`CrownParams::points`]. Треугольник — низ: на меньшем числе
    /// вершин многоугольника нет.
    fn base_points(self, params: &CrownParams) -> usize {
        let base = match self {
            Self::Conifer => 16.0,
            Self::Cotton | Self::Palm => 12.0,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        };
        ((base * params.points).round() as usize).max(3)
    }

    /// Доля радиуса, на которую джиттер утапливает вершину внутрь.
    fn radius_jitter(self, params: &CrownParams) -> f32 {
        let base = match self {
            Self::Conifer => 0.25,
            Self::Cotton | Self::Palm => 4.0 / 12.0,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        };
        base * params.radius_jitter
    }

    /// Сдвиг колец к свету за номер кольца, доля радиуса. Константа **формы**,
    /// а не номера кольца: `-(n+1)·R·0.15` у облака, `·0.12` у хвои, `·0.1` у
    /// пальмы.
    fn band_lift(self, params: &CrownParams) -> f32 {
        let base = match self {
            Self::Cotton => 0.15,
            Self::Conifer => 0.12,
            Self::Palm => 0.1,
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        };
        base * params.band_lift
    }

    /// Масштабы внутренних колец и вес вероятности штриха для каждого. Вес
    /// считается по **исходному** масштабу кольца: множитель `band_scale`
    /// двигает кольцо, а не густоту его штриховки, — та своя ручка.
    fn bands(self, params: &CrownParams) -> Vec<(f32, f32)> {
        let scaled =
            |scale: f32, weight: f32| (scale * params.band_scale, weight * params.shade_weight);
        match self {
            Self::Cotton => BALL_BANDS.iter().map(|&s| scaled(s, 3.0 * s * s)).collect(),
            Self::Conifer => CONE_BANDS.iter().map(|&s| scaled(s, 0.5 + s)).collect(),
            Self::Palm => PALM_BANDS.iter().map(|&s| scaled(s, 3.0 * s)).collect(),
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
    pub(super) fn shade(
        self,
        ring: &[Vec2],
        weight: f32,
        rng: &mut Lcg,
        params: &CrownParams,
    ) -> Vec<Vec<Vec2>> {
        match self {
            Self::Cotton => shaded_arcs(ring, weight, rng, params),
            Self::Conifer => chevron_arcs(ring, weight),
            Self::Palm => leaf_arcs(ring, weight, rng),
            Self::Mixed => unreachable!("{MIXED_HAS_NO_GEOMETRY}"),
        }
    }
}

/// Геометрия кроны единичного радиуса: контур и кольца штриховки.
pub(super) struct CrownGeometry {
    /// Форма, которой кольца штрихуются и по которой ложится тень.
    pub(super) shape: TreeShape,
    pub(super) outer: Vec<Vec2>,
    /// (кольцо, вес вероятности штриха).
    pub(super) bands: Vec<(Vec<Vec2>, f32)>,
}

/// `getCloudCrown` / `getPineCrown` / `getPalmCrown`: базовый многоугольник с
/// джиттером угла и радиуса, затем контур и уменьшенные кольца деталей.
pub(super) fn crown_geometry(
    shape: TreeShape,
    rng: &mut Lcg,
    params: &CrownParams,
) -> CrownGeometry {
    let points = shape.base_points(params);
    let mut base = Vec::with_capacity(points);
    for index in 0..points {
        let angle = TAU * (index as f32 + rng.gauss3()) / points as f32;
        let radius = 1.0 - shape.radius_jitter(params) * rng.bell4().abs();
        base.push(Vec2::from_angle(angle) * radius);
    }
    let lobe = (3.0 * PI / points as f32).sin() * params.lobe;
    // правка залипших вырезов идёт только по контуру: кольца штриховки строятся
    // ниже из нетронутой базы. Облаку она не нужна — все его вырезы мельче
    // `CrownParams::notch_depth_min`; пальме вредна — её листья тонкие по замыслу
    let outer = if shape == TreeShape::Conifer {
        cone_outline(&base, lobe, params)
    } else {
        shape.outline(&base, lobe)
    };
    let bands = shape
        .bands(params)
        .into_iter()
        .enumerate()
        .map(|(number, (scale, weight))| {
            let lift = Vec2::new(0.0, (number as f32 + 1.0) * shape.band_lift(params));
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
/// подряд, так что вариант полностью задан своим номером и сидом набора.
pub(super) fn variant_rng(variant: usize, params: &CrownParams) -> Lcg {
    Lcg::new(
        VARIANT_SEED_BASE
            .wrapping_add(params.seed.wrapping_mul(CROWN_SEED_STRIDE))
            .wrapping_add((variant as u32).wrapping_mul(VARIANT_SEED_STRIDE)),
    )
}

/// Контур хвойной кроны, у которого **каждый вырез читается под обводкой**.
/// Обводка шириной `CrownParams::outline_stroke` (12% радиуса по умолчанию)
/// съедает вырез двумя
/// способами, и оба дают на глаз один и тот же «горб» вместо двух шипов:
///
/// - вырез **уже** `notch_mouth_min` — чернила смыкаются поперёк, и он читается
///   иглой внутрь кроны;
/// - вырез **мельче** `notch_depth_min` — ямку закрывает сама линия, и остаётся
///   плоская верхушка между двумя тупыми остриями.
///
/// Мелкие вырезы лечит `spike_height_min`: у watabou высота шипа растёт как
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
fn cone_outline(base: &[Vec2], lobe: f32, params: &CrownParams) -> Vec<Vec2> {
    let mut base = base.to_vec();
    loop {
        let ring = spike_simple(&base, lobe, params.spike_height_min());
        let swallowed = swallowed_notches(&ring, params);
        if swallowed.is_empty() {
            return ring;
        }
        // за круг раскрывается один вырез: их число строго убывает, поэтому
        // цикл конечен
        if !swallowed
            .iter()
            .any(|&vertex| nudge_notch_open(&mut base, vertex, lobe, swallowed.len(), params))
        {
            return spike_simple(&base, lobe, params.spike_height_min());
        }
    }
}

/// Вершины базы, вырез над которыми обводка съедает — поперёк или по глубине.
fn swallowed_notches(ring: &[Vec2], params: &CrownParams) -> Vec<usize> {
    let count = ring.len() / 2;
    // впадины — чётные вершины контура: `spike_simple` кладёт вершину базы,
    // затем шип над ребром, которое из неё выходит
    (0..ring.len())
        .step_by(2)
        .filter(|&index| {
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            corner_metrics(previous, ring[index], ring[index + 1]).is_some_and(
                |(mouth, depth, valley)| {
                    valley && (mouth < params.notch_mouth_min() || depth < params.notch_depth_min())
                },
            )
        })
        .map(|index| (index / 2 + count - 1) % count)
        .collect()
}

/// Сдвигает вершину базы поперёк хорды её соседей на наименьшее смещение, при
/// котором съеденных вырезов становится меньше, а ни один угол контура не
/// сужается за `corner_mouth_floor`; пробуются оба направления. Без такого
/// смещения вершина остаётся на месте.
fn nudge_notch_open(
    base: &mut [Vec2],
    vertex: usize,
    lobe: f32,
    swallowed: usize,
    params: &CrownParams,
) -> bool {
    let count = base.len();
    let chord = base[(vertex + 1) % count] - base[(vertex + count - 1) % count];
    let step = chord.perp().normalize_or_zero() * (NOTCH_NUDGE_LIMIT / NOTCH_NUDGE_STEPS as f32);
    let origin = base[vertex];
    for offset in 1..=NOTCH_NUDGE_STEPS {
        for direction in [1.0_f32, -1.0] {
            base[vertex] = origin + step * (offset as f32 * direction);
            let ring = spike_simple(base, lobe, params.spike_height_min());
            if swallowed_notches(&ring, params).len() < swallowed
                && narrowest_corner(&ring) >= params.corner_mouth_floor()
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
pub(super) fn corner_metrics(previous: Vec2, vertex: Vec2, next: Vec2) -> Option<(f32, f32, bool)> {
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
pub(super) fn bloat(ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
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
pub(super) fn shadow_ring(outer: &[Vec2], params: &CrownParams) -> Vec<Vec2> {
    outer
        .iter()
        .map(|&point| {
            let local = Vec2::new(point.dot(SHADOW_DIR), point.perp_dot(SHADOW_DIR));
            let x = (local.x + 1.0) * params.shadow_stretch + params.shadow_backshift;
            SHADOW_DIR * x + SHADOW_DIR.perp() * -local.y
        })
        .collect()
}

/// Меш кроны: заливка + чернильный контур + штрихованные кольца (процедура
/// штриховки своя у каждой формы, см. [`TreeShape::shade`]).
/// Вершинные цвета настоящие; материал дерева умножает их на серый множитель,
/// так зелень варьируется, а чернила остаются чернилами.
pub(super) fn crown_mesh(
    geometry: &CrownGeometry,
    style: &TreeStyle,
    rng: &mut Lcg,
    params: &CrownParams,
) -> Mesh {
    let mut builder = MeshBuilder::default();
    let ink = style.details.to_linear();
    builder.push_polygon(&geometry.outer, &[], style.foliage.to_linear());
    builder.push_stroke(&geometry.outer, true, params.outline_stroke, ink);

    for (ring, weight) in &geometry.bands {
        for arc in geometry.shape.shade(ring, *weight, rng, params) {
            // круглые стыки, а не miter: «этаж» разворачивается на кончике
            // каждого шипа почти на 180°, и срезанный miter оставлял бы там
            // клин пустоты — ломаная читалась бы рваной. Контур кроны рисуется
            // прежним miter: у него излом наружу, и острия должны быть острыми
            builder.push_ribbon(
                &arc,
                false,
                params.detail_stroke,
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
pub(super) fn shaded_arcs(
    ring: &[Vec2],
    weight: f32,
    rng: &mut Lcg,
    params: &CrownParams,
) -> Vec<Vec<Vec2>> {
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
            >= params.detail_stroke
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
pub(super) fn chevron_arcs(ring: &[Vec2], weight: f32) -> Vec<Vec<Vec2>> {
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
pub(super) fn leaf_arcs(ring: &[Vec2], weight: f32, rng: &mut Lcg) -> Vec<Vec<Vec2>> {
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
pub(super) fn shadow_template(
    geometry: &CrownGeometry,
    rng: &mut Lcg,
    params: &CrownParams,
) -> MeshBuilder {
    let height = params.shadow_height_base + params.shadow_height_spread * rng.gauss3();
    let mut builder = MeshBuilder::default();
    match geometry.shape {
        TreeShape::Conifer => {
            for (outer, holes) in conifer_shadow(&geometry.outer, height) {
                builder.push_polygon(&outer, &holes, LinearRgba::WHITE);
            }
        }
        _ if height > params.long_shadow_height => {
            builder.push_polygon(
                &shadow_ring(&geometry.outer, params),
                &[],
                LinearRgba::WHITE,
            );
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
pub(super) fn conifer_shadow(outer: &[Vec2], height: f32) -> Vec<(Vec<Vec2>, Vec<Vec<Vec2>>)> {
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
