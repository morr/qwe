//! Трамвайные пути, вынесенные из дорожных слоёв: трамвай пересобирается по
//! ступеням зума ([`TRAM_LODS`]) — линия держит почти постоянную экранную
//! толщину («почти gizmo»), а шпалы редеют с отъездом камеры и на общем плане
//! исчезают, иначе они сливаются в сплошную массу. Обычные ж/д пути — в
//! `map/rail.rs`, со своей таблицей ступеней и своими слоями; стиль дорог
//! (`RoadStyle`) не касается ни тех, ни других.
//!
//! Навмеша путь не касается — люди ходят через рельсы как по земле.

use bevy::prelude::*;

use crate::loading::AppState;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::{MapData, RailKind, RailLine};
use crate::map::roads::{RoadJoin, RoadSmoothing, push_ribbon, smooth_path};
use crate::map::zoom::{ZoomBucket, ZoomLods};
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

/// [`TRAM_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`). Пустой enum —
/// тип-маркер, значений у него не бывает.
pub enum TramLods {}

impl ZoomLods for TramLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        TRAM_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`TRAM_LODS`]; пересечение порога пересобирает трамвайный
/// меш ([`rebuild_tram`]).
pub type TramZoomBucket = ZoomBucket<TramLods>;

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
    let lod = &TRAM_LODS[bucket.index];

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
        bucket.index,
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
