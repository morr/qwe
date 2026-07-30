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
use crate::map::meshing::MeshBuilder;
use crate::map::osm::MapData;
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
/// Сдвиг колец к свету за номер кольца, доля радиуса (`0.15/0.12/0.1`).
const BAND_LIFT: [f32; 3] = [0.15, 0.12, 0.1];
/// Минимальная длина дуги-штриха, доля радиуса.
const MIN_ARC_LENGTH: f32 = 0.15;

/// Вектор штриховки: рёбра CCW-колец вдоль него рисуются чаще (теневая сторона).
const SHADE_DIR: Vec2 = Vec2::new(0.5, 0.866_025_4);
/// Растяжение силуэта тени вдоль её оси (`1 + 0.5·shadowLength·0.8`).
const SHADOW_STRETCH: f32 = 1.4;
/// Обратный сдвиг силуэта тени (`−R·(1 − shadowLength/4)`).
const SHADOW_BACKSHIFT: f32 = -0.75;

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
            _ => 12,
        }
    }

    /// Доля радиуса, на которую джиттер утапливает вершину внутрь.
    fn radius_jitter(self) -> f32 {
        match self {
            Self::Conifer => 0.25,
            _ => 4.0 / 12.0,
        }
    }

    /// Масштабы внутренних колец и вес вероятности штриха для каждого.
    fn bands(self) -> Vec<(f32, f32)> {
        match self {
            Self::Cotton | Self::Mixed => BALL_BANDS.iter().map(|&s| (s, 3.0 * s * s)).collect(),
            Self::Conifer => CONE_BANDS.iter().map(|&s| (s, 0.5 + s)).collect(),
            Self::Palm => PALM_BANDS.iter().map(|&s| (s, 3.0 * s)).collect(),
        }
    }

    /// Контур из базового многоугольника: `Bloater::bloat` или `Spiker`.
    fn outline(self, ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
        match self {
            Self::Cotton | Self::Mixed => bloat(ring, lobe),
            Self::Conifer => spike_simple(ring, lobe),
            Self::Palm => spike_bent(ring, lobe),
        }
    }

    /// Множитель `lobe` для внутренних колец: у облака фестоны кольца
    /// крупнее самого кольца (`k/scale`), у шипастых форм — как у контура.
    fn band_lobe(self, lobe: f32, scale: f32) -> f32 {
        match self {
            Self::Cotton | Self::Mixed => lobe / scale,
            _ => lobe,
        }
    }
}

/// Геометрия кроны единичного радиуса: контур и кольца штриховки.
struct CrownGeometry {
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
    let outer = shape.outline(&base, lobe);
    let bands = shape
        .bands()
        .into_iter()
        .enumerate()
        .map(|(number, (scale, weight))| {
            let lift = Vec2::new(0.0, (number as f32 + 1.0) * BAND_LIFT[number.min(2)]);
            let ring: Vec<Vec2> = base.iter().map(|&point| point * scale + lift).collect();
            (shape.outline(&ring, shape.band_lobe(lobe, scale)), weight)
        })
        .collect();
    CrownGeometry { outer, bands }
}

/// `Spiker::simple`: между соседними вершинами вставлен один шип наружу
/// длиной `sqrt(len/lobe)·len` — хвойная «ёлочная» кромка.
fn spike_simple(ring: &[Vec2], lobe: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(ring.len() * 2);
    let mut previous = ring[ring.len() - 1];
    for &point in ring {
        out.push(previous);
        out.push(previous.midpoint(point) + spike_vector(previous, point, lobe));
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

/// Меш кроны: заливка + чернильный контур + пунктирные кольца (`drawShaded1`).
/// Вершинные цвета настоящие; материал дерева умножает их на серый множитель,
/// так зелень варьируется, а чернила остаются чернилами.
fn crown_mesh(geometry: &CrownGeometry, style: &TreeStyle, rng: &mut Lcg) -> Mesh {
    let mut builder = MeshBuilder::default();
    let ink = style.details.to_linear();
    builder.push_polygon(&geometry.outer, &[], style.foliage.to_linear());
    builder.push_stroke(&geometry.outer, true, TREE_OUTLINE_STROKE, ink);

    for (ring, weight) in &geometry.bands {
        // рёбра отбираются по watabou (`drawShaded1`), но соседние выбранные
        // склеиваются в одну дугу — иначе каждое ребро рисуется своим штрихом
        // с собственными торцами, и кольцо распадается на зубцы
        for arc in shaded_arcs(ring, *weight, rng) {
            builder.push_stroke(&arc, false, TREE_DETAIL_STROKE, ink);
        }
    }
    builder.build()
}

/// `drawShaded1`: вероятность нарисовать ребро тем выше, чем ближе его
/// направление к `SHADE_DIR` — штрихи скапливаются на теневой стороне кольца.
/// Возвращает связные цепочки выбранных рёбер длиннее `MIN_ARC_LENGTH` —
/// более короткие при зуме читаются как мусорные квадратики, а не как штрих.
fn shaded_arcs(ring: &[Vec2], weight: f32, rng: &mut Lcg) -> Vec<Vec<Vec2>> {
    let mut arcs: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for index in 0..ring.len() {
        let from = ring[index];
        let to = ring[(index + 1) % ring.len()];
        let drawn = (to - from).try_normalize().is_some_and(|direction| {
            rng.next_f32() < weight * (0.5 + 0.5 * SHADE_DIR.dot(direction))
        });
        if drawn {
            if current.is_empty() {
                current.push(from);
            }
            current.push(to);
        } else if !current.is_empty() {
            arcs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        arcs.push(current);
    }
    arcs.retain(|arc| {
        arc.windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum::<f32>()
            >= MIN_ARC_LENGTH
    });
    arcs
}

/// Силуэт тени единичного радиуса — шаблон, который `spawn_trees` кладёт в
/// общий меш теней под каждое дерево этого варианта.
fn shadow_template(geometry: &CrownGeometry) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    builder.push_polygon(&shadow_ring(&geometry.outer), &[], LinearRgba::WHITE);
    builder
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
                    let mut rng = Lcg::new(0x051E_D2E5 + variant as u32 * 7919);
                    let geometry = crown_geometry(shape, &mut rng);
                    (
                        meshes.add(crown_mesh(&geometry, style, &mut rng)),
                        shadow_template(&geometry),
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
