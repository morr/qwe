//! Оверлей полигонального меша: заливка непроходимых контуров плюс рёбра
//! полигонов, всё одним merged-мешем. По нему видно и контуры препятствий, и
//! как polyanya разбила проходимое пространство.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy::sprite_render::AlphaMode2d;

use crate::loading::AppState;
use crate::map::MeshBuilder;
use crate::navigation::{PolyNavmesh, PolymeshDebug};
use crate::settings::MAP_SIZE;

/// Над заливкой сеточного navmesh-оверлея (5.2), под юнитами.
const POLYMESH_OVERLAY_Z: f32 = 5.3;
/// Толщина ребра, метры мира: видна на городском зуме, не заливает экран.
const POLYMESH_EDGE_WIDTH: f32 = 0.4;
const POLYMESH_EDGE_COLOR: Color = Color::srgba(0.2, 0.85, 0.95, 0.6);
/// Заливка непроходимого — **тот же** красный, что у сеточного оверлея
/// (`debug.rs::sync_navmesh_overlay`): два слоя показывают одно и то же, и
/// одинаковый цвет — единственное, что делает их точность сравнимой на глаз.
const POLYMESH_BLOCKED_COLOR: Color = Color::srgba(0.9, 0.15, 0.15, 0.35);

/// Границы чанков — верхний уровень иерархии, по которому выбирается коридор
/// (`polymesh::find_path_polymesh`). Тёмные и полупрозрачные: они не часть
/// геометрии, а разбиение поверх неё, и читаться должны как сетка на карте, а
/// не как ещё один слой мира. Жёлтый пробовался и сливался с песком и дорогами.
/// Штрих той же толщины, что и рёбра меша: сетка чанков рисуется всегда, и
/// жирная линия перечёркивала бы геометрию, которую оверлей и показывает.
const POLYMESH_CHUNK_COLOR: Color = Color::srgba(0.05, 0.05, 0.08, 0.7);
const POLYMESH_CHUNK_WIDTH: f32 = 0.4;

/// Что нарисовано: поколение постройки и радиус — пока те же, пересобирать
/// слой незачем (идиома `ConiferNoiseOverlayMarker`). Чанков в ключе нет:
/// их переключение перестраивает меш, то есть двигает поколение.
#[derive(Component)]
pub(super) struct PolymeshOverlayMarker {
    generation: u32,
    radius_bits: u32,
}

/// Оверлей построенного меша: заливка непроходимых контуров плюс рёбра
/// полигонов, всё одним merged-мешем. Ключ кеша — на маркере: пересборка
/// только когда постройка сменилась, а не на каждом тычке ресурса.
pub(super) fn sync_polymesh_overlay(
    mut commands: Commands,
    debug: Res<PolymeshDebug>,
    poly: Res<PolyNavmesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    overlay: Query<(Entity, &PolymeshOverlayMarker)>,
) {
    let generation = poly.generation();
    let radius_bits = poly.built_radius().to_bits();
    let visible = debug.enabled && debug.show;
    if visible
        && overlay
            .iter()
            .any(|(_, drawn)| drawn.generation == generation && drawn.radius_bits == radius_bits)
    {
        return;
    }
    for (entity, _) in &overlay {
        commands.entity(entity).despawn();
    }
    if !visible {
        return;
    }
    let Some(built) = poly.build() else {
        return;
    };

    let mut builder = MeshBuilder::default();
    // сначала заливка — внутри одного меша порядок индексов и есть порядок
    // растеризации, так что рёбра лягут поверх неё
    let blocked = POLYMESH_BLOCKED_COLOR.to_linear();
    for obstacle in &built.obstacles {
        builder.push_polygon(obstacle, &[], blocked);
    }
    let color = POLYMESH_EDGE_COLOR.to_linear();
    for layer in &built.mesh.layers {
        // общее ребро соседних полигонов рисуется один раз — иначе на
        // полупрозрачном штрихе каждый внутренний шов был бы вдвое темнее
        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        for polygon in &layer.polygons {
            let count = polygon.vertices.len();
            for index in 0..count {
                let a = polygon.vertices[index];
                let b = polygon.vertices[(index + 1) % count];
                if !seen.insert((a.min(b), a.max(b))) {
                    continue;
                }
                // polyanya живёт на glam 0.30 — конверсия по полям. Координаты
                // вершин локальные для слоя: чанк триангулирован от своего
                // угла, мировая точка — плюс `offset`
                let origin = Vec2::new(layer.offset.x, layer.offset.y);
                let from = &layer.vertices[a as usize].coords;
                let to = &layer.vertices[b as usize].coords;
                builder.push_stroke(
                    &[
                        origin + Vec2::new(from.x, from.y),
                        origin + Vec2::new(to.x, to.y),
                    ],
                    false,
                    POLYMESH_EDGE_WIDTH,
                    color,
                );
            }
        }
    }

    // границы чанков — последними, чтобы легли поверх рёбер меша. Условия нет:
    // сетка берётся из самой постройки, и у плоского меша она 1x1, то есть ни
    // одной внутренней линии. Рисуется ровно то, по чему ходит поиск
    {
        let (grid, chunk_size) = built.chunks();
        let chunk_color = POLYMESH_CHUNK_COLOR.to_linear();
        for column in 1..grid.x {
            let x = column as f32 * chunk_size.x;
            builder.push_stroke(
                &[Vec2::new(x, 0.0), Vec2::new(x, MAP_SIZE.y)],
                false,
                POLYMESH_CHUNK_WIDTH,
                chunk_color,
            );
        }
        for row in 1..grid.y {
            let y = row as f32 * chunk_size.y;
            builder.push_stroke(
                &[Vec2::new(0.0, y), Vec2::new(MAP_SIZE.x, y)],
                false,
                POLYMESH_CHUNK_WIDTH,
                chunk_color,
            );
        }
    }

    if builder.is_empty() {
        return;
    }
    commands.spawn((
        PolymeshOverlayMarker {
            generation,
            radius_bits,
        },
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(materials.add(ColorMaterial {
            alpha_mode: AlphaMode2d::Blend,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, POLYMESH_OVERLAY_Z),
        DespawnOnExit(AppState::Playing),
        Name::new("polymesh_overlay"),
    ));
}
