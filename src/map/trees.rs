//! Процедурные кроны деревьев в стиле Watabou Village Generator.
//! Алгоритм восстановлен из Village.js, подробный разбор — `TREE_ALGO.md`:
//! мятый 12-угольник → «bloat» (рекурсивное выдавливание середин рёбер) →
//! облачный контур; внутренние кольца-штрихи; тень — растянутый силуэт.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use crate::map::meshing::MeshBuilder;
use crate::settings::{
    TREE_DETAIL_STROKE, TREE_OUTLINE_STROKE, TREE_VARIANTS, Z_TREE, Z_TREE_SHADOW,
};

/// Чернила контура и штрихов (watabou `colorInk`).
const INK_COLOR: Color = Color::srgb(0.004, 0.008, 0.024);
/// Базовая зелень кроны; пер-дерево умножается на яркость из `TINT_FACTORS`.
const CROWN_COLOR: Color = Color::srgb(0.42, 0.60, 0.33);
/// Тень: watabou рисует #9699AE в multiply; здесь — альфа-эквивалент.
const SHADOW_COLOR: Color = Color::srgba(0.22, 0.24, 0.33, 0.42);

/// Квантованные множители яркости кроны: `2^(0.2·bell)`, bell ∈ (−1, 1).
const TINT_FACTORS: [f32; 5] = [0.87, 0.94, 1.0, 1.07, 1.15];

/// Вершины базового многоугольника кроны.
const CROWN_POINTS: usize = 12;
/// Внутренние кольца штриховки: (масштаб, вес вероятности) — `BALL_BANDS2`.
const CROWN_BANDS: [(f32, f32); 2] = [(0.8, 1.92), (0.5, 0.75)];
/// Сдвиг колец к свету за номер кольца, доля радиуса.
const BAND_LIFT: f32 = 0.15;

/// Направление тени: 30° вниз-вправо (y-вверх), нормировано.
const SHADOW_DIR: Vec2 = Vec2::new(0.866_025_4, -0.5);
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

/// Геометрия кроны единичного радиуса: облачный контур и кольца штриховки.
struct CrownGeometry {
    outer: Vec<Vec2>,
    /// (кольцо, вес вероятности штриха).
    bands: Vec<(Vec<Vec2>, f32)>,
}

/// `getCloudCrown`: 12 вершин с джиттером угла и радиуса, раздутые в облако.
fn cloud_crown(rng: &mut Lcg) -> CrownGeometry {
    let mut base = Vec::with_capacity(CROWN_POINTS);
    for index in 0..CROWN_POINTS {
        let angle = TAU * (index as f32 + rng.gauss3()) / CROWN_POINTS as f32;
        let radius = 1.0 - (4.0 / CROWN_POINTS as f32) * rng.bell4().abs();
        base.push(Vec2::from_angle(angle) * radius);
    }
    let lobe = (3.0 * PI / CROWN_POINTS as f32).sin();
    let outer = bloat(&base, lobe);
    let bands = CROWN_BANDS
        .iter()
        .enumerate()
        .map(|(number, &(scale, weight))| {
            let lift = Vec2::new(0.0, (number as f32 + 1.0) * BAND_LIFT);
            let ring: Vec<Vec2> = base.iter().map(|&point| point * scale + lift).collect();
            (bloat(&ring, lobe / scale), weight)
        })
        .collect();
    CrownGeometry { outer, bands }
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
fn crown_mesh(geometry: &CrownGeometry, rng: &mut Lcg) -> Mesh {
    let mut builder = MeshBuilder::default();
    builder.push_polygon(&geometry.outer, &[], CROWN_COLOR.to_linear());

    let mut outline = geometry.outer.clone();
    outline.push(geometry.outer[0]);
    builder.push_polyline(&outline, TREE_OUTLINE_STROKE, INK_COLOR.to_linear());

    for (ring, weight) in &geometry.bands {
        for index in 0..ring.len() {
            let from = ring[index];
            let to = ring[(index + 1) % ring.len()];
            let Some(direction) = (to - from).try_normalize() else {
                continue;
            };
            let probability = weight * (0.5 + 0.5 * SHADE_DIR.dot(direction));
            if rng.next_f32() < probability {
                builder.push_polyline(&[from, to], TREE_DETAIL_STROKE, INK_COLOR.to_linear());
            }
        }
    }
    builder.build()
}

fn shadow_mesh(geometry: &CrownGeometry) -> Mesh {
    let mut builder = MeshBuilder::default();
    builder.push_polygon(&shadow_ring(&geometry.outer), &[], LinearRgba::WHITE);
    builder.build()
}

/// Спавн деревьев: `TREE_VARIANTS` пар мешей (крона+тень) единичного радиуса,
/// каждому дереву — вариант, оттенок и масштаб детерминированно по индексу.
pub fn spawn_trees(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    trees: &[(Vec2, f32)],
) {
    let variants: Vec<(Handle<Mesh>, Handle<Mesh>)> = (0..TREE_VARIANTS)
        .map(|variant| {
            let mut rng = Lcg::new(0x051E_D2E5 + variant as u32 * 7919);
            let geometry = cloud_crown(&mut rng);
            (
                meshes.add(crown_mesh(&geometry, &mut rng)),
                meshes.add(shadow_mesh(&geometry)),
            )
        })
        .collect();
    let tints: Vec<Handle<ColorMaterial>> = TINT_FACTORS
        .iter()
        .map(|&factor| materials.add(Color::srgb(factor, factor, factor)))
        .collect();
    let shadow_material = materials.add(SHADOW_COLOR);

    for (index, &(position, radius)) in trees.iter().enumerate() {
        let (crown, shadow) = &variants[index % variants.len()];
        // микрошаг по z: пересекающиеся кроны рисуются в стабильном порядке
        let z = Z_TREE + (index % 512) as f32 * 1e-3;
        commands.spawn((
            Mesh2d(crown.clone()),
            MeshMaterial2d(tints[(index * 7) % tints.len()].clone()),
            Transform::from_translation(position.extend(z)).with_scale(Vec3::splat(radius)),
            Name::new("tree"),
        ));
        commands.spawn((
            Mesh2d(shadow.clone()),
            MeshMaterial2d(shadow_material.clone()),
            Transform::from_translation(position.extend(Z_TREE_SHADOW))
                .with_scale(Vec3::splat(radius)),
            Name::new("tree_shadow"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_area(ring: &[Vec2]) -> f32 {
        ring.windows(2)
            .map(|pair| pair[0].perp_dot(pair[1]))
            .sum::<f32>()
            / 2.0
            + ring[ring.len() - 1].perp_dot(ring[0]) / 2.0
    }

    #[test]
    fn cloud_crown_is_deterministic() {
        let first = cloud_crown(&mut Lcg::new(42));
        let second = cloud_crown(&mut Lcg::new(42));
        assert_eq!(first.outer, second.outer);
        assert_eq!(first.bands.len(), second.bands.len());
    }

    #[test]
    fn cloud_crown_stays_near_unit_radius() {
        let crown = cloud_crown(&mut Lcg::new(7));
        // bloat выдавливает наружу: контур длиннее базового 12-угольника
        assert!(crown.outer.len() > CROWN_POINTS * 4);
        for point in &crown.outer {
            let distance = point.length();
            assert!(
                (0.4..=1.45).contains(&distance),
                "outer point at {distance}"
            );
        }
        // CCW-обход: bloat наружу требует положительной площади
        assert!(ring_area(&crown.outer) > 0.0);
    }

    #[test]
    fn bloat_pushes_midpoints_outward() {
        let square = [
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
            Vec2::new(-1.0, -1.0),
        ];
        let bloated = bloat(&square, 1.0);
        assert!(bloated.len() > square.len());
        let max_distance = bloated
            .iter()
            .map(|point| point.length())
            .fold(0.0_f32, f32::max);
        assert!(max_distance > 2.0_f32.sqrt());
    }

    #[test]
    fn shadow_ring_stretches_along_shadow_dir() {
        let crown = cloud_crown(&mut Lcg::new(3));
        let shadow = shadow_ring(&crown.outer);
        let extent = |ring: &[Vec2]| {
            let projected: Vec<f32> = ring.iter().map(|point| point.dot(SHADOW_DIR)).collect();
            let min = projected.iter().copied().fold(f32::INFINITY, f32::min);
            let max = projected.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            (min, max)
        };
        let (crown_min, crown_max) = extent(&crown.outer);
        let (shadow_min, shadow_max) = extent(&shadow);
        // тень длиннее кроны вдоль своей оси и выступает на подветренную сторону
        assert!(shadow_max - shadow_min > (crown_max - crown_min) * 1.3);
        assert!(shadow_max > crown_max + 0.5);
    }

    #[test]
    fn crown_mesh_builds_non_empty() {
        let mut rng = Lcg::new(11);
        let geometry = cloud_crown(&mut rng);
        let mesh = crown_mesh(&geometry, &mut rng);
        assert!(mesh.count_vertices() > 0);
        assert!(shadow_mesh(&geometry).count_vertices() > 0);
    }
}
