use super::*;
use crate::camera::{MAX_ZOOM, MIN_ZOOM};
use crate::map::meshing::distance_to_path;
use crate::map::osm::fixture;

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
/// балласт со шпалами → пунктирный знак. Ни на одной ступени путь не остаётся
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

/// Штрих дальних ступеней темнее балласта, по которому он идёт, на **каждой**
/// палитре. Белым (как в osm-carto) он выходил лесенкой через весь станционный
/// парк — самым картографическим, что есть в кадре.
///
/// И темнее он ровно на шпалу: штрих — это те же шпалы, которых на этих
/// ступенях уже не рисуют, поэтому на пороге зума метка не светлеет скачком.
///
/// Темнее — мало: на последней ступени штрих шириной ровно в пиксель, и знак
/// держится только на **величине** разницы тона. Мерим контрастом WCAG
/// `(L_светлый + 0.05) / (L_тёмный + 0.05)` по `luminance()` bevy (линейная):
/// разность яркостей для этого слепа — у обеих палитр она ~0.143, хотя у
/// заброшенной штрих на глаз читается вдвое хуже. Сегодня: действующая
/// 0.177 / 0.035 → 2.69, заброшенная 0.255 / 0.110 → 1.90. Порог
/// [`MIN_DASH_CONTRAST`] стоит под худшей из них так, что шпалу заброшенного
/// пути можно осветлить на 0.02 по каналам sRGB (1.76), а на 0.03 (1.69) —
/// уже нет: «поседение» палитры дальше этого стирает однопиксельную метку.
///
/// Равенство `dash == tie` здесь не проверяется: оба поля стоят на одной
/// константе палитры (`*_TIE`), и разойтись им нельзя по построению — если
/// поле когда-нибудь разведут, этот порог и будет сторожем нового цвета.
#[test]
fn the_far_dash_stays_darker_than_its_ballast() {
    for (name, palette) in [("active", &ACTIVE), ("disused", &DISUSED)] {
        let dash = palette.dash.luminance();
        let ballast = palette.ballast.luminance();
        assert!(
            dash < ballast,
            "{name}: dash {dash} is not darker than its ballast {ballast}"
        );
        let contrast = (ballast + 0.05) / (dash + 0.05);
        assert!(
            contrast >= MIN_DASH_CONTRAST,
            "{name}: dash {dash} on ballast {ballast} gives contrast {contrast}, \
             under {MIN_DASH_CONTRAST} — a one-pixel dash stops reading"
        );
    }
}

/// Нижний порог контраста штриха с балластом; обоснование — в доке
/// [`the_far_dash_stays_darker_than_its_ballast`].
const MIN_DASH_CONTRAST: f32 = 1.75;

/// Плечо призмы торчит из-под верха балласта — иначе откоса не видно и путь
/// снова читается плоской лентой.
#[test]
fn ballast_shoulder_sticks_out_from_under_the_bed() {
    let points = [Vec2::ZERO, Vec2::new(100.0, 0.0)];
    let mut shoulder = MeshBuilder::default();
    shoulder.push_ribbon(
        &points,
        false,
        NOMINAL_BED * SHOULDER_SCALE,
        LinearRgba::WHITE,
        RAIL_JOIN,
        RAIL_CAP,
    );
    let mut bed = MeshBuilder::default();
    bed.push_ribbon(
        &points,
        false,
        NOMINAL_BED,
        LinearRgba::WHITE,
        RAIL_JOIN,
        RAIL_CAP,
    );

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
    // нитка тоньше допуска веера сходится на изломе общими вершинами по
    // биссектрисе — удлинение miter у неё и у полуколеи одно
    let outward = (gauge / 2.0 + width / 2.0) / (sharpest / 2.0).cos();
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

// --- слой целиком ------------------------------------------------------
//
// Ниже — тесты на `mesh_rails`, то есть на слой, собранный без мира. До шва
// сборка жила внутри системы Bevy, и достать её из теста было нечем:
// проверять можно было только примитивы под ней.

/// Путь через полкилометра карты — чтобы на любой ступени было что рисовать.
fn straight_track() -> Vec<Vec2> {
    vec![Vec2::new(100.0, 100.0), Vec2::new(600.0, 100.0)]
}

/// Ближняя ступень: и шпалы, и сталь на месте.
fn near_bucket() -> RailZoomBucket {
    RailZoomBucket::for_zoom(MIN_ZOOM)
}

#[test]
fn a_track_builds_its_layers_bottom_up() {
    let rails = [fixture::rail(straight_track(), NOMINAL_BED)];
    let (layers, report) = mesh_rails(near_bucket(), &rails);

    let names: Vec<&str> = layers.iter().map(|layer| layer.name).collect();
    assert_eq!(
        names,
        [
            "rail_ballast",
            "rail_ties",
            "rail_steel",
            "rail_bridge_ballast",
            "rail_bridge_ties",
            "rail_bridge_steel",
        ]
    );

    // три меша, а не один, ровно потому же, почему их три и в мире: шпала
    // обязана лежать выше **любого** балласта, иначе развязка расслаивается
    for pair in layers.windows(2) {
        assert!(
            pair[0].z < pair[1].z,
            "{} лежит не ниже {}",
            pair[0].name,
            pair[1].name
        );
    }
    assert_eq!(report.tracks, 1);
    assert!(report.vertices > 0);
}

#[test]
fn every_rail_layer_is_flat() {
    let rails = [fixture::rail(straight_track(), NOMINAL_BED)];
    let (layers, _) = mesh_rails(near_bucket(), &rails);

    // путь и так весь из щебня, шпал и стали — фактура поверхности ему ни к чему
    for layer in &layers {
        assert_eq!(layer.material, MaterialSpec::Flat, "{}", layer.name);
    }
}

#[test]
fn a_tram_track_is_left_to_its_own_module() {
    let tram = RailLine {
        kind: RailKind::Tram,
        ..fixture::rail(straight_track(), NOMINAL_BED)
    };
    let (layers, report) = mesh_rails(near_bucket(), &[tram]);

    assert_eq!(
        report.tracks, 0,
        "трамвай рисует `map/tram.rs`, а не этот слой"
    );
    assert_eq!(report.vertices, 0);
    // слои описаны, но пусты: пустой меш адаптер пропустит сам
    assert!(layers.iter().all(|layer| layer.builder.is_empty()));
}

#[test]
fn the_far_bucket_drops_the_steel() {
    let rails = [fixture::rail(straight_track(), NOMINAL_BED)];
    let far = RailZoomBucket::for_zoom(MAX_ZOOM);
    assert!(
        RAIL_LODS[far.index].steel.is_none(),
        "дальняя ступень обязана быть без нитей"
    );

    let (layers, _) = mesh_rails(far, &rails);
    let steel = layers
        .iter()
        .find(|layer| layer.name == "rail_steel")
        .expect("слой стали описан на любой ступени");

    // ступень только снимает детали: слой остаётся в списке, но пустой
    assert!(steel.builder.is_empty());
    let ballast = layers
        .iter()
        .find(|layer| layer.name == "rail_ballast")
        .expect("балласт рисуется на каждой ступени");
    assert!(!ballast.builder.is_empty());
}

fn layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a MeshBuilder {
    &layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("слой {name} описан"))
        .builder
}

// --- путепровод и переезд (R35) -------------------------------------------

/// Путь на мосту ложится своими слоями над настилом моста, а не на землю
/// под ним: иначе мост поверх дороги рисовался как путь, проложенный по
/// асфальту (Орёл, 7245, 728).
#[test]
fn a_track_on_a_bridge_is_drawn_above_the_deck() {
    let mut overpass = fixture::rail(straight_track(), NOMINAL_BED);
    overpass.bridge = true;
    let (layers, report) = mesh_rails(near_bucket(), &[overpass]);

    for name in ["rail_ballast", "rail_ties", "rail_steel"] {
        assert!(layer(&layers, name).is_empty(), "{name}: путь не на земле");
    }
    for name in [
        "rail_bridge_ballast",
        "rail_bridge_ties",
        "rail_bridge_steel",
    ] {
        assert!(!layer(&layers, name).is_empty(), "{name}: путь на мосту");
    }
    let bridge_ballast = layers
        .iter()
        .find(|layer| layer.name == "rail_bridge_ballast")
        .unwrap();
    assert!(
        bridge_ballast.z > crate::settings::Z_BRIDGE,
        "путь над плитой"
    );
    // откоса на мосту нет: балласт в корыте не шире своего верха
    assert!(half_extent(&bridge_ballast.builder) - 100.0 <= NOMINAL_BED / 2.0 + 1e-3);
    assert_eq!(report.bridges, 1);
}

/// Ось улицы поперёк пути по x = 300 (путь идёт по y = 100).
const ACROSS: [Vec2; 2] = [Vec2::new(300.0, 0.0), Vec2::new(300.0, 200.0)];

/// Переезд: щебень и шпалы на ширине дороги закрыты настилом, и настил не
/// выходит ни за подошву балласта, ни за дорогу с её тротуарами — у каждой
/// стороны своя полоса.
#[test]
fn a_street_across_a_track_gets_a_crossing_deck() {
    let rails = [fixture::rail(straight_track(), NOMINAL_BED)];
    // слева по ходу оси (к −x) 4 м проезжей части и 2 м тротуара, справа — 4
    let street = CrossedStreet {
        axis: &ACROSS,
        reach: [6.0, 4.0],
    };
    let (deck, count) = mesh_level_crossings(&rails, &[street]);

    assert_eq!(count, 1);
    let half = NOMINAL_BED * SHOULDER_SCALE / 2.0;
    let xs: Vec<f32> = deck.positions_for_test().iter().map(|p| p[0]).collect();
    for position in deck.positions_for_test() {
        assert!((position[1] - 100.0).abs() <= half + 1e-3, "{position:?}");
    }
    let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (min - 294.0).abs() < 1e-3,
        "левая кромка с тротуаром: {min}"
    );
    assert!((max - 304.0).abs() < 1e-3, "правая кромка без него: {max}");
}

// настил переезда лежит между шпалами и нитками: рельсы идут поверх него
const _: () = assert!(
    Z_RAIL_TIE < crate::settings::Z_RAIL_CROSSING
        && crate::settings::Z_RAIL_CROSSING < Z_RAIL_STEEL
);

/// Нет переезда там, где путь не пересекает улицу в одном уровне:
/// путепровод над ней, трамвай (он на асфальте и так), улица вдоль пути и
/// улица, до пути не доходящая. Мост улицы над путём и пешеходную дорожку
/// отсеивает [`crossable`] до этой функции.
#[test]
fn no_crossing_deck_without_an_at_grade_crossing() {
    let track = fixture::rail(straight_track(), NOMINAL_BED);
    let mut overpass = track.clone();
    overpass.bridge = true;
    let mut tram = track.clone();
    tram.kind = RailKind::Tram;
    let alongside = [Vec2::new(150.0, 98.0), Vec2::new(550.0, 102.0)];
    let short = [Vec2::new(300.0, 0.0), Vec2::new(300.0, 90.0)];

    let cases: [(&str, RailLine, &[Vec2]); 4] = [
        ("путепровод", overpass, &ACROSS),
        ("трамвай", tram, &ACROSS),
        ("вдоль пути", track.clone(), &alongside),
        ("не доходит", track, &short),
    ];
    for (case, rail, axis) in cases {
        let street = CrossedStreet {
            axis,
            reach: [4.0; 2],
        };
        let (deck, count) = mesh_level_crossings(&[rail], &[street]);
        assert!(deck.is_empty(), "{case}");
        assert_eq!(count, 0, "{case}");
    }

    let over_the_track = fixture::bridge(ACROSS.to_vec(), 8.0);
    let mut footway = fixture::street(ACROSS.to_vec(), 8.0);
    footway.class = crate::map::osm::RoadClass::Alley;
    assert!(!crossable(&over_the_track), "мост улицы над путём");
    assert!(!crossable(&footway), "дорожка");
    assert!(crossable(&fixture::street(ACROSS.to_vec(), 5.0)), "проезд");
}

/// Косой переезд: торцы настила идут по кромкам дороги, а не поперёк пути —
/// все его углы лежат на кромках.
#[test]
fn an_oblique_crossing_deck_follows_the_road_edges() {
    let rails = [fixture::rail(straight_track(), NOMINAL_BED)];
    let axis = [Vec2::new(250.0, 0.0), Vec2::new(350.0, 200.0)];
    let street = CrossedStreet {
        axis: &axis,
        reach: [4.0; 2],
    };
    let (deck, _) = mesh_level_crossings(&rails, &[street]);

    let along = (axis[1] - axis[0]).normalize();
    assert!(!deck.is_empty());
    for position in deck.positions_for_test() {
        let offset = Vec2::new(position[0], position[1]) - axis[0];
        let across = offset.perp_dot(along).abs();
        assert!((across - 4.0).abs() < 1e-3, "{position:?}");
    }
}
