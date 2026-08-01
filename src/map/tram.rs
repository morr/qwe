//! Трамвайные пути, вынесенные из дорожных слоёв: трамвай пересобирается по
//! ступеням зума ([`TRAM_LODS`]) — линия держит почти постоянную экранную
//! толщину («почти gizmo»), а шпалы редеют с отъездом камеры и на общем плане
//! исчезают, иначе они сливаются в сплошную массу. Обычные ж/д пути остаются
//! в `map/roads.rs` под стилем дорог.
//!
//! Навмеша путь не касается — люди ходят через рельсы как по земле.

use bevy::camera_controller::pan_camera::PanCamera;
use bevy::prelude::*;

use crate::loading::AppState;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RailKind, RailLine};
use crate::map::roads::{RoadJoin, RoadSmoothing, push_ribbon, smooth_path};
use crate::settings::Z_TRAM;

/// Трамвай — не лента, а линия с поперечной насечкой, как в Яндекс.Картах и
/// 2ГИС. Причина не в стиле: трамвайный путь лежит **на проезжей части**, и
/// лента в ширину колеи закрыла бы улицу, по которой он идёт.
///
/// Цвет — единственное, чем два этих источника различаются: у Яндекса линия
/// тёмно-красная, у 2ГИС синяя. Геометрия одна и та же; взят вариант 2ГИС —
/// синее на сером асфальте видно лучше, а красным на карте уже размечены
/// стены Кремля.
const TRAM_COLOR: Color = Color::srgb(0.290, 0.451, 0.780);

/// Ширина, зажимающая срез Chaikin у трамвая. Ширина линии меняется от ступени
/// к ступени, но осевая обязана оставаться одной и той же — иначе путь ёрзает
/// при переходе через порог зума.
const TRAM_SMOOTH_WIDTH: f32 = 1.2;

/// Стиль зафиксирован, без ручек панели: на линии в полтора-два экранных
/// пикселя стык излома не читается вовсе, а Strong-сглаживание неотличимо от
/// Light. Осевая всегда слегка сглажена — ломаная OSM на повороте даёт тонкой
/// линии заметный угол.
const TRAM_JOIN: RoadJoin = RoadJoin::Round;
const TRAM_SMOOTHING: RoadSmoothing = RoadSmoothing::Light;

/// Шпала одной ступени: длина поперёк пути, толщина и шаг, м. Насечка обязана
/// быть заметно длиннее толщины самой линии — иначе она сливается с ней в
/// утолщение.
pub struct TramTieLod {
    pub length: f32,
    pub thickness: f32,
    pub spacing: f32,
}

/// Ступень зум-LOD трамвая: до какого зума действует и какой геометрией
/// рисуется. Зум — мировых метров на логический пиксель (`PanCamera`).
pub struct TramLod {
    /// Верхняя (исключающая) граница ступени.
    pub max_zoom: f32,
    pub line_width: f32,
    /// `None` — на этой ступени шпалы не рисуются вовсе.
    pub tie: Option<TramTieLod>,
}

/// Ступени зум-LOD: линия целится в ~1.8 px на середине каждой ступени
/// (экранная толщина гуляет в пределах ~1.1–2.9 px — «почти gizmo»), шаг шпал
/// на экране нигде не падает ниже ~10 px — редкая насечка, а не гребёнка. На
/// последней ступени шпалы исчезают, как в 2ГИС на общем плане города.
pub const TRAM_LODS: [TramLod; 5] = [
    TramLod {
        max_zoom: 0.12,
        line_width: 0.14,
        tie: Some(TramTieLod {
            length: 0.45,
            thickness: 0.09,
            spacing: 1.45,
        }),
    },
    TramLod {
        max_zoom: 0.30,
        line_width: 0.34,
        tie: Some(TramTieLod {
            length: 1.1,
            thickness: 0.20,
            spacing: 3.5,
        }),
    },
    TramLod {
        max_zoom: 0.75,
        line_width: 0.85,
        tie: Some(TramTieLod {
            length: 2.8,
            thickness: 0.50,
            spacing: 8.8,
        }),
    },
    TramLod {
        max_zoom: 1.9,
        line_width: 2.1,
        tie: Some(TramTieLod {
            length: 7.0,
            thickness: 1.2,
            spacing: 27.0,
        }),
    },
    TramLod {
        max_zoom: f32::INFINITY,
        line_width: 5.2,
        tie: None,
    },
];

/// Текущая ступень [`TRAM_LODS`] — индекс. Меняется только при пересечении
/// порога зума ([`update_tram_zoom_bucket`]), на что [`rebuild_tram`] отвечает
/// пересборкой одного трамвайного меша. Не сохраняется: зум сбрасывается к
/// `START_ZOOM` на каждом входе в мир.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct TramZoomBucket(pub usize);

impl Default for TramZoomBucket {
    fn default() -> Self {
        Self(bucket_for_zoom(crate::camera::START_ZOOM))
    }
}

/// Первая ступень, чья граница выше зума. Зум на самой границе попадает в
/// верхнюю ступень.
pub fn bucket_for_zoom(zoom: f32) -> usize {
    TRAM_LODS
        .iter()
        .position(|lod| zoom < lod.max_zoom)
        .unwrap_or(TRAM_LODS.len() - 1)
}

/// Трамвайный меш — чтобы пересборка знала, что деспавнить.
#[derive(Component)]
pub struct TramLayerTag;

/// Трамвайный меш текущей ступени зума. Единственный вызов — из
/// [`rebuild_tram`]: и вход в мир, и смена ступени зума идут через пересборку
/// (в свежем мире деспавнить ей нечего). Линия и шпалы — один цвет, поэтому
/// лежат в одном меше: накладываться сами на себя они могут без всякого
/// z-файтинга.
fn spawn_tram(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    bucket: TramZoomBucket,
    rails: &[RailLine],
) {
    let started = std::time::Instant::now();
    let lod = &TRAM_LODS[bucket.0];

    let mut builder = MeshBuilder::default();
    for rail in rails {
        if rail.kind != RailKind::Tram {
            continue;
        }
        let points = smooth_path(&rail.points, TRAM_SMOOTH_WIDTH, TRAM_SMOOTHING);
        push_tram(&mut builder, &points, lod);
    }
    if builder.is_empty() {
        return;
    }

    let vertices = builder.vertex_count();
    commands.spawn((
        TramLayerTag,
        Mesh2d(meshes.add(builder.build())),
        // вершинные цвета — материал белый, как у остальных слоёв карты
        MeshMaterial2d(materials.add(Color::WHITE)),
        Transform::from_xyz(0.0, 0.0, Z_TRAM),
        DespawnOnExit(AppState::Playing),
        Name::new("tram"),
    ));

    info!(
        "tram meshing: {vertices} verts in {:?} (bucket {})",
        started.elapsed(),
        bucket.0,
    );
}

/// Линия и шпалы одного пути на одной ступени LOD — отдельно от спавна ради
/// тестов на геометрию.
pub(crate) fn push_tram(builder: &mut MeshBuilder, points: &[Vec2], lod: &TramLod) {
    let color = TRAM_COLOR.to_linear();
    push_ribbon(builder, points, lod.line_width, color, TRAM_JOIN);
    if let Some(tie) = &lod.tie {
        builder.push_ticks(points, tie.length, tie.thickness, tie.spacing, color);
    }
}

/// Ступень зума по фактическому масштабу камеры. `set_if_neq` — чтобы
/// `resource_changed` срабатывал только на пересечении порога, а не каждый
/// кадр.
pub fn update_tram_zoom_bucket(
    camera: Single<&PanCamera, With<Camera2d>>,
    mut bucket: ResMut<TramZoomBucket>,
) {
    bucket.set_if_neq(TramZoomBucket(bucket_for_zoom(camera.zoom_factor)));
}

/// Пересборка трамвайного меша при смене ступени зума — дорожные и рельсовые
/// слои не трогаются.
pub fn rebuild_tram(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bucket: Res<TramZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<TramLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    spawn_tram(
        &mut commands,
        &mut meshes,
        &mut materials,
        *bucket,
        &map.rails,
    );
}

#[cfg(test)]
mod tests;
