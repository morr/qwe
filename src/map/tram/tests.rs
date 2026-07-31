use super::*;

/// Обе границы диапазона зума камеры покрыты ступенями, ступень не убывает с
/// ростом зума, а зум ровно на границе попадает в верхнюю ступень.
#[test]
fn tram_bucket_covers_the_zoom_range() {
    assert_eq!(bucket_for_zoom(0.05), 0);
    assert_eq!(bucket_for_zoom(4.5), TRAM_LODS.len() - 1);

    let mut previous = 0;
    for step in 0..=450 {
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
    let mut min_zoom = 0.05;
    for lod in &TRAM_LODS {
        let max_zoom = lod.max_zoom.min(4.5);
        for zoom in [min_zoom, max_zoom] {
            let px = lod.line_width / zoom;
            assert!((1.0..=3.2).contains(&px), "line {px} px at zoom {zoom}");
        }
        min_zoom = max_zoom;
    }
}

/// Шпалы не сливаются в массу: шаг на экране не меньше ~6 px даже у дальнего
/// края ступени, а сама шпала длиннее и линии, и собственной толщины.
#[test]
fn tram_ties_stay_sparse_on_screen() {
    for lod in &TRAM_LODS {
        let Some(tie) = &lod.tie else { continue };
        let worst_zoom = lod.max_zoom.min(4.5);
        assert!(
            tie.spacing / worst_zoom >= 5.5,
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

#[test]
fn tie_density_orders_spacing() {
    assert!(TieDensity::Sparse.spacing_multiplier() > TieDensity::Normal.spacing_multiplier());
    assert!(TieDensity::Normal.spacing_multiplier() > TieDensity::Dense.spacing_multiplier());
    assert_eq!(TieDensity::Normal.spacing_multiplier(), 1.0);
}

/// Смена ступени и правда меняет геометрию: вблизи линия тоньше (шпалы торчат,
/// но общий размах всё равно меньше дальней ленты), а дальняя ступень — голая
/// лента без единой вершины шпал.
#[test]
fn tram_mesh_narrows_and_sheds_ties_per_bucket() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let style = TramStyle::default();

    let extent = |builder: &MeshBuilder| {
        builder
            .positions_for_test()
            .iter()
            .map(|position| position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    };

    let mut near = MeshBuilder::default();
    push_tram(&mut near, &points, style, &TRAM_LODS[0]);
    let mut far = MeshBuilder::default();
    push_tram(&mut far, &points, style, &TRAM_LODS[TRAM_LODS.len() - 1]);

    assert!(extent(&near) < extent(&far));

    let mut bare_far = MeshBuilder::default();
    push_ribbon(
        &mut bare_far,
        &points,
        TRAM_LODS[TRAM_LODS.len() - 1].line_width,
        TRAM_COLOR.to_linear(),
        style.join,
    );
    assert_eq!(far.vertex_count(), bare_far.vertex_count());

    let mut bare_near = MeshBuilder::default();
    push_ribbon(
        &mut bare_near,
        &points,
        TRAM_LODS[0].line_width,
        TRAM_COLOR.to_linear(),
        style.join,
    );
    assert!(near.vertex_count() > bare_near.vertex_count());
}
