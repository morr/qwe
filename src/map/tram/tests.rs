use super::*;
use crate::camera::{MAX_ZOOM, MIN_ZOOM};
use crate::map::osm::fixture;

/// Обе границы диапазона зума камеры покрыты ступенями, ступень не убывает с
/// ростом зума, а зум ровно на границе попадает в верхнюю ступень.
#[test]
fn tram_bucket_covers_the_zoom_range() {
    let bucket_for_zoom = |zoom: f32| TramZoomBucket::for_zoom(zoom).index;
    assert_eq!(bucket_for_zoom(MIN_ZOOM), 0);
    assert_eq!(bucket_for_zoom(MAX_ZOOM), TRAM_LODS.len() - 1);

    let mut previous = 0;
    for step in 0..=(MAX_ZOOM * 100.0) as u32 {
        let zoom = step as f32 * 0.01;
        let bucket = bucket_for_zoom(zoom);
        assert!(bucket >= previous, "bucket dropped at zoom {zoom}");
        previous = bucket;
    }

    for (index, lod) in TRAM_LODS.iter().enumerate().take(TRAM_LODS.len() - 1) {
        assert_eq!(bucket_for_zoom(lod.max_zoom), index + 1);
    }
}

#[test]
fn tram_lods_step_up_with_zoom() {
    for pair in TRAM_LODS.windows(2) {
        assert!(pair[0].max_zoom < pair[1].max_zoom);
        assert!(pair[0].line_width < pair[1].line_width);
    }
    assert_eq!(TRAM_LODS[TRAM_LODS.len() - 1].max_zoom, f32::INFINITY);
}

/// «Почти gizmo»: на обоих концах каждой ступени линия остаётся в пределах
/// 1–3.2 экранных пикселей.
#[test]
fn tram_line_stays_near_screen_width() {
    let mut min_zoom = MIN_ZOOM;
    for lod in &TRAM_LODS {
        let max_zoom = lod.max_zoom.min(MAX_ZOOM);
        for zoom in [min_zoom, max_zoom] {
            let px = lod.line_width / zoom;
            assert!((1.0..=3.2).contains(&px), "line {px} px at zoom {zoom}");
        }
        min_zoom = max_zoom;
    }
}

/// Шпалы не сливаются в массу: шаг на экране не меньше ~10 px даже у дальнего
/// края ступени, а сама шпала длиннее и линии, и собственной толщины.
#[test]
fn tram_ties_stay_sparse_on_screen() {
    for lod in &TRAM_LODS {
        let Some(tie) = &lod.tie else { continue };
        let worst_zoom = lod.max_zoom.min(MAX_ZOOM);
        assert!(
            tie.spacing / worst_zoom >= 10.0,
            "ties merge at zoom {worst_zoom}"
        );
        assert!(tie.length > lod.line_width);
        assert!(tie.length > tie.thickness);
    }
}

/// На общем плане города шпалы исчезают, как в 2ГИС.
#[test]
fn far_bucket_drops_ties() {
    assert!(TRAM_LODS[TRAM_LODS.len() - 1].tie.is_none());
}

/// Смена ступени и правда меняет геометрию: вблизи линия тоньше (шпалы торчат,
/// но общий размах всё равно меньше дальней ленты), а дальняя ступень — голая
/// лента без единой вершины шпал.
#[test]
fn tram_mesh_narrows_and_sheds_ties_per_bucket() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];

    let extent = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };

    let mut near = MeshBuilder::default();
    push_tram(&mut near, &points, &TRAM_LODS[0]);
    let mut far = MeshBuilder::default();
    push_tram(&mut far, &points, &TRAM_LODS[TRAM_LODS.len() - 1]);

    assert!(extent(&near) < extent(&far));

    let mut bare_far = MeshBuilder::default();
    push_ribbon(
        &mut bare_far,
        &points,
        TRAM_LODS[TRAM_LODS.len() - 1].line_width,
        TRAM_COLOR.to_linear(),
        TRAM_JOIN,
    );
    assert_eq!(far.vertex_count(), bare_far.vertex_count());

    let mut bare_near = MeshBuilder::default();
    push_ribbon(
        &mut bare_near,
        &points,
        TRAM_LODS[0].line_width,
        TRAM_COLOR.to_linear(),
        TRAM_JOIN,
    );
    assert!(near.vertex_count() > bare_near.vertex_count());
}

// --- слой целиком ------------------------------------------------------
//
// Тесты на `mesh_tram`. До шва слой собирался внутри системы Bevy, и тумблер
// видимости проверить было нечем вовсе: он жил в самой системе, за ранним
// возвратом.

fn tram_line() -> RailLine {
    RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)], 1.2)
    }
}

fn near_bucket() -> TramZoomBucket {
    TramZoomBucket::for_zoom(MIN_ZOOM)
}

#[test]
fn a_tram_line_builds_one_flat_layer() {
    let (layers, report) = mesh_tram(near_bucket(), &TramStyle { visible: true }, &[tram_line()]);

    assert_eq!(layers.len(), 1, "линия и шпалы одного цвета — один меш");
    assert_eq!(layers[0].name, "tram");
    assert_eq!(layers[0].z, Z_TRAM);
    assert_eq!(layers[0].material, MaterialSpec::Flat);
    assert_eq!(report.tracks, 1);
    assert!(report.vertices > 0);
}

#[test]
fn the_toggle_off_draws_nothing() {
    let (layers, report) = mesh_tram(near_bucket(), &TramStyle { visible: false }, &[tram_line()]);

    // не ранний выход у вызывающего: слой описан и пуст, а деспавн в адаптере
    // безусловен — забыть его негде
    assert_eq!(report.tracks, 0);
    assert_eq!(report.vertices, 0);
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
}

#[test]
fn an_ordinary_track_is_left_to_the_rail_module() {
    let heavy = fixture::rail(vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)], 5.0);
    let (_, report) = mesh_tram(near_bucket(), &TramStyle { visible: true }, &[heavy]);

    assert_eq!(report.tracks, 0, "обычный путь рисует `map/rail.rs`");
    assert_eq!(report.vertices, 0);
}
