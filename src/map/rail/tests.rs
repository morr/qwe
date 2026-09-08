use super::*;
use crate::camera::{MAX_ZOOM, MIN_ZOOM};
use crate::map::meshing::distance_to_path;

/// Ширины балласта из OSM (`osm/parse/tags.rs`): магистральный путь,
/// light_rail / метро, заброшенный. Экранные пороги обязаны держаться на
/// **каждой** — колея одна, а шпала и штрих считаются от балласта.
const BEDS: [f32; 3] = [5.0, 4.0, 3.5];

/// Ширина балласта магистрального пути — для тестов на геометрию призмы.
const NOMINAL_BED: f32 = 5.0;

fn half_extent(builder: &MeshBuilder) -> f32 {
    builder
        .positions_for_test()
        .iter()
        .map(|position| position[1].abs())
        .fold(0.0_f32, f32::max)
}

/// Обе границы диапазона зума камеры покрыты ступенями, ступень не убывает с
/// ростом зума, а зум ровно на границе попадает в верхнюю ступень.
#[test]
fn rail_bucket_covers_the_zoom_range() {
    let bucket_for_zoom = |zoom: f32| RailZoomBucket::for_zoom(zoom).index;
    assert_eq!(bucket_for_zoom(MIN_ZOOM), 0);
    assert_eq!(bucket_for_zoom(MAX_ZOOM), RAIL_LODS.len() - 1);

    let mut previous = 0;
    for step in 0..=(MAX_ZOOM * 100.0) as u32 {
        let zoom = step as f32 * 0.01;
        let bucket = bucket_for_zoom(zoom);
        assert!(bucket >= previous, "bucket dropped at zoom {zoom}");
        previous = bucket;
    }

    for (index, lod) in RAIL_LODS.iter().enumerate().take(RAIL_LODS.len() - 1) {
        assert_eq!(bucket_for_zoom(lod.max_zoom), index + 1);
    }
}

#[test]
fn rail_lods_step_up_with_zoom() {
    for pair in RAIL_LODS.windows(2) {
        assert!(pair[0].max_zoom < pair[1].max_zoom);
        // пол ширины балласта только растёт: он и нужен затем, чтобы путь не
        // истончился раньше улиц, которые пересекает
        assert!(pair[0].min_bed <= pair[1].min_bed);
    }
    assert_eq!(RAIL_LODS[RAIL_LODS.len() - 1].max_zoom, f32::INFINITY);
}

/// Три рисунка сменяют друг друга ровно в одну сторону: конструкция целиком →
/// балласт со шпалами → знак osm-carto. Ни на одной ступени путь не остаётся
/// голой лентой и ни на одной шпалы не смешиваются со штриховкой.
#[test]
fn rail_detail_falls_away_with_zoom() {
    let mut ties_gone = false;
    let mut steel_gone = false;
    for (index, lod) in RAIL_LODS.iter().enumerate() {
        assert!(
            lod.tie.is_some() != lod.dash.is_some(),
            "bucket {index} draws ties and dashes at once (or neither)"
        );
        assert!(
            lod.steel.is_none() || lod.tie.is_some(),
            "bucket {index} draws rails without ties under them"
        );

        assert!(
            !steel_gone || lod.steel.is_none(),
            "bucket {index}: rails came back after they were dropped"
        );
        assert!(
            !ties_gone || lod.tie.is_none(),
            "bucket {index}: ties came back after they were dropped"
        );
        steel_gone |= lod.steel.is_none();
        ties_gone |= lod.tie.is_none();
    }
    // на ближней ступени путь нарисован целиком, на дальней — знаком
    assert!(RAIL_LODS[0].tie.is_some() && RAIL_LODS[0].steel.is_some());
    assert!(RAIL_LODS[RAIL_LODS.len() - 1].dash.is_some());
}

/// Экранные размеры на **худшем** краю каждой ступени и на каждой ширине
/// балласта из парсера: путь не должен ни слипаться в сплошную массу, ни
/// истончаться до невидимого волоска.
#[test]
fn rail_marks_stay_legible_on_screen() {
    for (nominal, (index, lod)) in BEDS
        .into_iter()
        .flat_map(|nominal| RAIL_LODS.iter().enumerate().map(move |lod| (nominal, lod)))
    {
        let max_zoom = lod.max_zoom.min(MAX_ZOOM);
        let bed = nominal.max(lod.min_bed);
        assert!(
            bed / max_zoom >= 1.8,
            "bucket {index}, bed {nominal}: ballast {} px",
            bed / max_zoom
        );

        if let Some(tie) = &lod.tie {
            assert!(
                tie.spacing / max_zoom >= 6.0,
                "bucket {index}: ties merge at {} px apart",
                tie.spacing / max_zoom
            );
            assert!(
                tie.thickness / max_zoom >= 1.2,
                "bucket {index}: tie {} px thick",
                tie.thickness / max_zoom
            );
            // шпала лежит поперёк балласта, но не торчит из-под него
            assert!(tie.length_scale > 0.0 && tie.length_scale < 1.0);
            assert!(
                bed * tie.length_scale > tie.thickness,
                "bucket {index}, bed {nominal}: tie shorter than it is thick"
            );
            // доля шпалы в шаге держится около настоящих 40% (0.26 м на 0.65):
            // упадёт — шпалы перестанут быть текстурой, станут редкими метками,
            // и две белые нитки перевесят их в лестницу
            let duty = tie.thickness / tie.spacing;
            assert!(
                (0.35..=0.45).contains(&duty),
                "bucket {index}: ties fill {duty} of the step"
            );
        }

        if let Some(steel) = &lod.steel {
            let width = steel.width / max_zoom;
            assert!(
                (1.0..=3.5).contains(&width),
                "bucket {index}: rail {width} px"
            );
            // две нитки обязаны читаться порознь, иначе это одна полоса —
            // колея абсолютная, поэтому зазор один на любой балласт
            let gauge = steel.gauge;
            assert!(
                (gauge - steel.width) / max_zoom >= 4.0,
                "bucket {index}: rails merge {} px apart",
                (gauge - steel.width) / max_zoom
            );
            assert!(
                gauge < bed,
                "bucket {index}, bed {nominal}: gauge wider than the ballast"
            );
            // нитки лежат на шпалах, а не торчат за их концы — на самом узком
            // балласте это и есть предел абсолютной колеи
            let tie = lod
                .tie
                .as_ref()
                .expect("rails without ties are refused above");
            assert!(
                gauge + steel.width <= bed * tie.length_scale,
                "bucket {index}, bed {nominal}: rails overhang the ties"
            );
        }

        if let Some(dash) = &lod.dash {
            assert!(
                dash.length / max_zoom >= 4.0 && dash.gap / max_zoom >= 4.0,
                "bucket {index}: dashes merge at zoom {max_zoom}"
            );
            // штрих обязан лежать внутри балласта, иначе он читается как
            // вторая линия рядом с путём
            assert!(dash.width_scale > 0.0 && dash.width_scale < 1.0);
            // ...но при этом быть виден: половина балласта, поднятого `min_bed`
            // (4.5 м из девяти на последней ступени), — ровно пиксель на её
            // дальнем краю
            assert!(
                bed * dash.width_scale / max_zoom >= 1.0,
                "bucket {index}, bed {nominal}: dash under a pixel"
            );
        }
    }
}

/// Плечо призмы торчит из-под верха балласта — иначе откоса не видно и путь
/// снова читается плоской лентой.
#[test]
fn ballast_shoulder_sticks_out_from_under_the_bed() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut shoulder = MeshBuilder::default();
    push_ribbon(
        &mut shoulder,
        &points,
        NOMINAL_BED * SHOULDER_SCALE,
        LinearRgba::WHITE,
        RAIL_JOIN,
    );
    let mut bed = MeshBuilder::default();
    push_ribbon(&mut bed, &points, NOMINAL_BED, LinearRgba::WHITE, RAIL_JOIN);

    assert!(half_extent(&shoulder) > half_extent(&bed));
    // но не превращается в самостоятельную полосу: откос уже половины пути
    assert!(half_extent(&shoulder) < 1.5 * half_extent(&bed));
}

/// Нитки идут по колее: между ними пусто, наружу за балласт они не выходят.
#[test]
fn rails_run_along_the_gauge() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let (gauge, width) = (1.5, 0.12);
    let mut steel = MeshBuilder::default();
    steel.push_rails(&points, gauge, width, LinearRgba::WHITE, RibbonJoin::Round);

    assert!(!steel.is_empty());
    for position in steel.positions_for_test() {
        let offset = position[1].abs();
        assert!(
            (offset - gauge / 2.0).abs() <= width / 2.0 + 1e-4,
            "rail vertex {offset} m off the centerline"
        );
    }
}

/// На изломе нитка остаётся на своей колее: смещение по биссектрисе держит
/// вершину стыка равноудалённой от **обоих** сегментов осевой, то есть ровно
/// на полуколее от каждого, вместо того чтобы уехать наружу вместе с углом.
///
/// От самой вершины излома точка стыка при этом отстоит на
/// `полколеи / cos(излом/2)` — то самое удлинение miter, которое есть у края
/// любой ленты, и по нему считается граница снаружи.
#[test]
fn rails_hold_the_gauge_through_a_bend() {
    let points = [
        Vec2::ZERO,
        Vec2::new(60.0, 0.0),
        Vec2::new(110.0, 30.0),
        Vec2::new(160.0, 30.0),
    ];
    let (gauge, width) = (1.5, 0.12);
    let mut steel = MeshBuilder::default();
    steel.push_rails(&points, gauge, width, LinearRgba::WHITE, RibbonJoin::Round);

    let sharpest = points
        .windows(3)
        .map(|corner| {
            let incoming = (corner[1] - corner[0]).normalize();
            let outgoing = (corner[2] - corner[1]).normalize();
            incoming.angle_to(outgoing).abs()
        })
        .fold(0.0_f32, f32::max);
    let outward = gauge / 2.0 / (sharpest / 2.0).cos() + width / 2.0;
    let inward = gauge / 2.0 - width / 2.0;

    assert!(!steel.is_empty());
    for position in steel.positions_for_test() {
        let distance = distance_to_path(Vec2::new(position[0], position[1]), &points);
        assert!(
            (inward - 1e-3..=outward + 1e-3).contains(&distance),
            "rail vertex {distance} m off the centerline at a bend"
        );
    }
}

/// Вырожденный вход не роняет и не рисует: путь из одной точки, нулевая колея,
/// нулевая ширина нитки.
#[test]
fn degenerate_rails_draw_nothing() {
    for (points, gauge, width) in [
        (vec![Vec2::ZERO], 1.5, 0.12),
        (vec![Vec2::ZERO, Vec2::new(10.0, 0.0)], 0.0, 0.12),
        (vec![Vec2::ZERO, Vec2::new(10.0, 0.0)], 1.5, 0.0),
        (vec![Vec2::ZERO, Vec2::new(0.01, 0.0)], 1.5, 0.12),
    ] {
        let mut steel = MeshBuilder::default();
        steel.push_rails(&points, gauge, width, LinearRgba::WHITE, RibbonJoin::Round);
        assert!(steel.is_empty());
    }
}
