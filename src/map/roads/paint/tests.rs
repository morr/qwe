use super::*;
use crate::map::osm::fixture::street;
use crate::map::osm::{Highway, MapData};
use crate::map::roads::shape::RoadShape;
use crate::map::roads::{CrossingMode, RoadStyle, mesh_roads};

/// Меш слоя краски по имени.
fn paint_layer<'a>(layers: &'a [LayerMesh], name: &str) -> &'a MeshBuilder {
    &layers
        .iter()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("нет слоя {name}"))
        .builder
}

fn map_of(roads: Vec<RoadLine>) -> MapData {
    MapData {
        network: RoadNetwork::new(&roads),
        roads,
        ..MapData::default()
    }
}

fn with_lanes(mut road: RoadLine, lanes: u8, oneway: bool) -> RoadLine {
    road.lanes = Some(lanes);
    road.oneway = oneway;
    road
}

/// Где в станции `x` лежат центры полос краски слоя `name`, поперёк прямой
/// улицы вдоль оси x: по паре вершин полосы на станцию. Допуск по x — на
/// линию, идущую по клину наискось: её пара вершин стоит по нормали к ней, то
/// есть чуть в стороне от станции.
fn line_offsets(layers: &[LayerMesh], name: &str, x: f32) -> Vec<f32> {
    let builder = paint_layer(layers, name);
    let positions = builder.positions_for_test();
    let mut offsets: Vec<f32> = positions
        .chunks(2)
        .filter(|pair| ((pair[0][0] + pair[1][0]) / 2.0 - x).abs() < 0.05)
        .map(|pair| (pair[0][1] + pair[1][1]) / 2.0)
        .collect();
    offsets.sort_by(f32::total_cmp);
    offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    offsets
}

#[test]
fn a_two_lane_street_gets_one_axis_and_no_lane_lines() {
    let map = map_of(vec![with_lanes(
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 7.6),
        2,
        false,
    )]);
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert!(paint_layer(&layers, PAINT_LANES).is_empty());
    assert_eq!(line_offsets(&layers, PAINT_AXES, 0.0), vec![0.0]);
    // осевая — пунктир на перегоне и сплошная у тупиков по концам: три куска
    assert_eq!(report.paint_lines, 3);
    let kinds = paint_layer(&layers, PAINT_AXES)
        .ribbon_coords_for_test()
        .unwrap();
    let axis = [LineKind::AxisDashed.code(), LineKind::AxisSolid.code()];
    assert!(kinds.iter().all(|ribbon| axis.contains(&ribbon[3])));
}

#[test]
fn four_lanes_get_a_double_axis_and_a_lane_line_each_way() {
    let map = map_of(vec![with_lanes(
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.2),
        4,
        false,
    )]);
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(line_offsets(&layers, PAINT_AXES, 0.0), vec![0.0]);
    let axes = paint_layer(&layers, PAINT_AXES)
        .ribbon_coords_for_test()
        .unwrap();
    assert!(
        axes.iter()
            .all(|ribbon| ribbon[3] == LineKind::Double.code())
    );
    let lanes = line_offsets(&layers, PAINT_LANES, 0.0);
    assert_eq!(lanes.len(), 2);
    assert!((lanes[0] + lane_width()).abs() < 1e-3, "{lanes:?}");
    assert!((lanes[1] - lane_width()).abs() < 1e-3, "{lanes:?}");
}

/// Пять полос в обе стороны (Ростов, Текучёва): осевая есть — двойная
/// сплошная на границе потоков, а не посреди средней полосы. Без
/// `lanes:backward` лишняя полоса — потоку по ходу точек (справа), с ним —
/// как сказано.
#[test]
fn five_two_way_lanes_get_a_double_axis_between_the_flows() {
    let five = |backward: Option<u8>| {
        let mut road = with_lanes(
            street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 17.5),
            5,
            false,
        );
        road.highway = Highway::Primary;
        road.lanes_backward = backward;
        let (layers, _) = mesh_roads(
            &map_of(vec![road]),
            RoadStyle::default(),
            RoadShape::default(),
        );
        let axes = line_offsets(&layers, PAINT_AXES, 100.0);
        let lanes = line_offsets(&layers, PAINT_LANES, 100.0);
        let kinds: Vec<_> = paint_layer(&layers, PAINT_AXES)
            .ribbon_coords_for_test()
            .map(|coords| coords.iter().map(|ribbon| ribbon[3]).collect())
            .unwrap_or_default();
        (axes, lanes, kinds)
    };
    let (axes, lanes, kinds) = five(None);
    assert_eq!(axes.len(), 1, "{axes:?}");
    assert!((axes[0] - lane_width() / 2.0).abs() < 1e-3, "{axes:?}");
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|&kind| kind == LineKind::Double.code()));
    // остальные три границы — линии полос
    assert_eq!(lanes.len(), 3, "{lanes:?}");
    let (axes, _, _) = five(Some(3));
    assert!((axes[0] + lane_width() / 2.0).abs() < 1e-3, "{axes:?}");
}

#[test]
fn an_odd_axis_sits_on_the_border_of_the_flows() {
    let mut road = with_lanes(street(vec![Vec2::ZERO, Vec2::X], 17.5), 5, false);
    let right = axis_offset(&road, 5, TrafficSide::Right).unwrap();
    assert!((right - lane_width() / 2.0).abs() < 1e-4);
    let left = axis_offset(&road, 5, TrafficSide::Left).unwrap();
    assert!((left + lane_width() / 2.0).abs() < 1e-4);
    assert_eq!(axis_offset(&road, 4, TrafficSide::Right), Some(0.0));
    road.lanes_backward = Some(1);
    let pushed = axis_offset(&road, 4, TrafficSide::Right).unwrap();
    assert!(
        (pushed - lane_width()).abs() < 1e-4,
        "три по ходу, одна назад"
    );
    road.oneway = true;
    assert_eq!(axis_offset(&road, 4, TrafficSide::Right), None);
}

#[test]
fn a_one_way_street_has_no_axis() {
    let map = map_of(vec![with_lanes(
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 10.9),
        3,
        true,
    )]);
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert!(paint_layer(&layers, PAINT_AXES).is_empty());
    // три полосы — две линии, по сетке от полполосы
    let lanes = line_offsets(&layers, PAINT_LANES, 0.0);
    assert_eq!(lanes.len(), 2, "{lanes:?}");
    assert!((lanes[0] + lane_width() / 2.0).abs() < 1e-3, "{lanes:?}");
    assert!((lanes[1] - lane_width() / 2.0).abs() < 1e-3, "{lanes:?}");
}

#[test]
fn markings_off_paint_nothing() {
    let map = map_of(vec![street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.2)]);
    let style = RoadStyle {
        markings: false,
        ..RoadStyle::default()
    };
    let (layers, report) = mesh_roads(&map, style, RoadShape::default());
    for name in [
        PAINT_LANES,
        PAINT_AXES,
        BRIDGE_PAINT_LANES,
        BRIDGE_PAINT_AXES,
    ] {
        assert!(paint_layer(&layers, name).is_empty(), "{name}");
    }
    assert_eq!(report.paint_lines, 0);
}

/// `lane_markings=no` снимает с улицы осевую и границы полос, как бы много
/// полос у неё ни было (Тула, витрина 09: Бухоновский переулок).
#[test]
fn a_street_without_lane_markings_paints_no_lines() {
    let road = RoadLine {
        lane_markings: false,
        ..with_lanes(
            street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.2),
            4,
            false,
        )
    };
    let (layers, report) = mesh_roads(
        &map_of(vec![road]),
        RoadStyle::default(),
        RoadShape::default(),
    );
    assert!(paint_layer(&layers, PAINT_LANES).is_empty());
    assert!(paint_layer(&layers, PAINT_AXES).is_empty());
    assert_eq!(report.paint_lines, 0);
}

#[test]
fn a_bridge_paints_into_its_own_layer() {
    let mut road = street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 7.6);
    road.bridge = true;
    let (layers, _) = mesh_roads(
        &map_of(vec![road]),
        RoadStyle::default(),
        RoadShape::default(),
    );
    assert!(paint_layer(&layers, PAINT_AXES).is_empty());
    assert!(!paint_layer(&layers, BRIDGE_PAINT_AXES).is_empty());
}

/// Сетка полос раскладки — где на ней лежат границы полос внутри проезжей
/// части, поперёк пути.
fn grid(frame: LaneFrame) -> Vec<f32> {
    let from = ((frame.low - frame.origin) / lane_width()).ceil() as i32;
    let to = ((frame.high - frame.origin) / lane_width()).floor() as i32;
    let mut lines: Vec<f32> = (from..=to)
        .map(|k| frame.origin + k as f32 * lane_width())
        .filter(|&at| at > frame.low + 1e-3 && at < frame.high - 1e-3)
        .collect();
    lines.sort_by(f32::total_cmp);
    lines
}

/// Колея асфальта клина и линии краски — одна раскладка. Асфальт клина идёт
/// по пути **от шва к телу** (`wedge_frames`), краска — по пути way
/// (`narrow_frame`); у торца конца один путь навстречу другому. В каждой точке
/// клина сетка асфальта, отражённая в раму way, обязана совпасть с сеткой
/// краски.
#[test]
fn the_wedge_asphalt_and_the_wedge_paint_share_one_grid() {
    for (body, narrow) in [(4, 2), (3, 2), (5, 2), (6, 3)] {
        for (end, drift) in [false, true]
            .into_iter()
            .flat_map(|end| [None, Some(1.0), Some(-1.0)].map(|drift| (end, drift)))
        {
            let wedge = WedgeEnd {
                length: 10.0,
                lanes: narrow,
                drift,
                kept: None,
                origin: None,
            };
            let [from, to] = wedge_frames(body, wedge, end);
            let body_frame = lane_frame(body);
            let paint_from = narrow_frame(body_frame, narrow, end, drift);
            for step in 0..=10 {
                let t = step as f32 / 10.0;
                let asphalt = from.lerp(to, t);
                let asphalt = if end { mirrored(asphalt) } else { asphalt };
                let paint = paint_from.lerp(body_frame, t);
                let (a, p) = (grid(asphalt), grid(paint));
                assert_eq!(a.len(), p.len(), "{body}/{narrow} end {end} t {t}");
                for (a, p) in a.iter().zip(&p) {
                    assert!(
                        (a - p).abs() < 1e-3,
                        "{body}/{narrow} end {end} t {t}: {a} vs {p}"
                    );
                }
            }
        }
    }
}

/// У шва клина сетка — узкого сечения, у тела — своя: линии, что есть у
/// обоих, идут без сдвига (чётность та же), а при смене чётности сдвиг — ровно
/// полполосы.
#[test]
fn a_wedge_starts_on_the_narrow_grid() {
    let body = lane_frame(4);
    let seam = narrow_frame(body, 2, false, None);
    assert_eq!(grid(seam), grid(lane_frame(2)));
    assert_eq!(seam.origin, 0.0, "две полосы в четыре: линии не двигаются");
    let odd = narrow_frame(lane_frame(3), 2, false, None);
    assert!((lane_frame(3).origin - odd.origin - lane_width() / 2.0).abs() < 1e-4);
}

/// Профиль колеи у узла слияния — тот же, что у линий краски: в узле раскладка
/// узла, за клином тело; way, начатый в середине клина, получает пару на
/// своём торце.
#[test]
fn a_merge_ramp_hands_the_asphalt_the_paint_frames() {
    let body = lane_frame(2);
    let node = LaneFrame {
        origin: body.origin + 3.0,
        low: body.low + 3.0,
        high: body.high + 3.0,
    };
    let ramp = MergeRamp {
        frame: node,
        length: 20.0,
        start: 30.0,
        away: false,
    };
    // way длиной 30 кончается в узле: клин — его последние 20 м
    let profile = ramp.lane_profile(body, 30.0);
    assert!(profile.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    assert_eq!(profile.first().map(|pair| pair.0), Some(10.0));
    assert_eq!(profile.first().map(|pair| pair.1), Some(body));
    assert_eq!(profile.last().map(|pair| pair.0), Some(30.0));
    // со стороны пары (здесь low — ближе к оси продолжения) асфальт не уже
    // тела: лента половины у узла лежит поверх соседки
    let widened = LaneFrame {
        low: body.low,
        ..node
    };
    assert_eq!(profile.last().map(|pair| pair.1), Some(widened));
    for &(along, frame) in &profile {
        let paint = ramp.frame_at(body, along).unwrap_or(body);
        assert_eq!((frame.origin, frame.high), (paint.origin, paint.high));
        assert_eq!(frame.low, paint.low.min(body.low));
    }
    // клин, начатый на соседнем way: пара на торце у узла
    let farther = MergeRamp {
        start: 5.0,
        away: true,
        ..ramp
    };
    let profile = farther.lane_profile(body, 30.0);
    assert_eq!(profile.first().map(|pair| pair.0), Some(0.0));
    assert_eq!(profile.last().map(|pair| pair.1), Some(body));
}

/// Односторонний клин 2 → 4 (пример 16): общие полосы прижаты к левой кромке,
/// обе новые рождаются у бордюра справа. Каждая линия узкого сечения на теле
/// стоит на разницу полуширин левее, чем у шва.
#[test]
fn a_one_way_wedge_adds_its_lanes_at_the_kerb() {
    let body = lane_frame(4);
    let seam = narrow_frame(body, 2, false, Some(1.0));
    assert_eq!(grid(seam), grid(lane_frame(2)), "у шва — сетка узкого");
    let shift = body.high - lane_frame(2).high;
    // линия между полосами узкого сечения уходит к левой линии тела
    let divider = seam.origin + lane_width() * ((0.0 - seam.origin) / lane_width()).round();
    let moved = divider + (body.origin - seam.origin);
    assert!((moved - shift).abs() < 1e-4, "{moved} vs {shift}");
    assert!(
        (moved - lane_width()).abs() < 1e-4,
        "делитель встаёт на +1 полосу"
    );
    // карман левого поворота — наоборот
    let mut road = street(vec![Vec2::ZERO, Vec2::X * 100.0], 14.0);
    road.oneway = true;
    assert_eq!(wedge_drift(&road, TrafficSide::Right), Some(1.0));
    assert_eq!(wedge_drift(&road, TrafficSide::Left), Some(-1.0));
    let turn = |left, through, right| LaneTurn {
        left,
        through,
        right,
    };
    road.turns[0] = vec![
        turn(true, false, false),
        turn(false, true, false),
        turn(false, true, true),
    ];
    assert_eq!(wedge_drift(&road, TrafficSide::Right), Some(-1.0));
    road.oneway = false;
    assert_eq!(wedge_drift(&road, TrafficSide::Right), None);
}

/// Все линии краски в станции `x` — обоих видов.
fn all_lines(layers: &[LayerMesh], x: f32) -> Vec<f32> {
    let mut all = line_offsets(layers, PAINT_LANES, x);
    all.extend(line_offsets(layers, PAINT_AXES, x));
    all.sort_by(f32::total_cmp);
    all
}

/// Шов двух ways одной улицы у x = 200: узкий в `lanes[0]` полос, широкий в
/// `lanes[1]`. Клин лежит на широком от шва.
fn seam_of(lanes: [u8; 2]) -> (Vec<LayerMesh>, f32) {
    let width = |lanes: u8| f32::from(lanes) * lane_width() + 1.0;
    let roads = vec![
        with_lanes(
            street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], width(lanes[0])),
            lanes[0],
            false,
        ),
        with_lanes(
            street(
                vec![Vec2::new(200.0, 0.0), Vec2::new(400.0, 0.0)],
                width(lanes[1]),
            ),
            lanes[1],
            false,
        ),
    ];
    let (layers, report) = mesh_roads(&map_of(roads), RoadStyle::default(), RoadShape::default());
    assert_eq!(report.drawn.tapers, 1);
    let taper = (width(lanes[1]) - width(lanes[0])) * tapers::TAPER_PER_METER;
    (layers, 200.0 + taper)
}

/// Две полосы в четыре: осевая идёт сквозь клин без сдвига, а к его концу
/// рядом с ней — по линии в каждую сторону через шаг полосы.
#[test]
fn a_wedge_adds_lanes_without_moving_the_axis() {
    let (layers, wide) = seam_of([2, 4]);
    let at_seam = all_lines(&layers, 200.0);
    assert!(at_seam.iter().any(|x| x.abs() < 1e-3), "{at_seam:?}");
    let lines = all_lines(&layers, wide);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[1].abs() < 1e-3, "{lines:?}");
    for pair in lines.windows(2) {
        assert!((pair[1] - pair[0] - lane_width()).abs() < 1e-3, "{lines:?}");
    }
}

/// Расширение 2 → 4 (R25, Тула, Болдина): новые полосы на клине не
/// размечаются, пока не наберут ширину, и их линии встают сразу полными — ни
/// одной вершины краски с прозрачностью между нулём и краской; осевая на
/// клине одиночная, как у узкой части, двойная — с конца клина.
#[test]
fn a_widening_paints_no_fading_lines_and_a_single_axis() {
    let (layers, wide) = seam_of([2, 4]);
    for name in [PAINT_LANES, PAINT_AXES] {
        let colors = paint_layer(&layers, name).colors_for_test();
        let full = colors.iter().map(|color| color[3]).fold(0.0, f32::max);
        assert!(full > 0.0, "{name}: нет краски");
        let fading: Vec<f32> = colors
            .iter()
            .map(|color| color[3])
            .filter(|&alpha| alpha > 1e-4 && alpha < full - 1e-4)
            .collect();
        assert!(
            fading.is_empty(),
            "{name}: проявляющиеся вершины {fading:?}"
        );
    }
    let axes = paint_layer(&layers, PAINT_AXES);
    let kinds_between = |from: f32, to: f32| -> Vec<f32> {
        let mut kinds: Vec<f32> = axes
            .positions_for_test()
            .iter()
            .zip(axes.ribbon_for_test())
            .filter(|(at, _)| at[0] > from && at[0] < to)
            .map(|(_, ribbon)| ribbon[3])
            .collect();
        kinds.sort_by(f32::total_cmp);
        kinds.dedup();
        kinds
    };
    let double = LineKind::Double.code();
    assert!(
        !kinds_between(200.5, wide - 0.5).contains(&double),
        "двойная на клине: {:?}",
        kinds_between(200.5, wide - 0.5)
    );
    assert_eq!(kinds_between(wide + 0.5, 390.0), vec![double]);
}

/// Две полосы в три: чётность сменилась, и линия уходит на полполосы плавно
/// по длине клина, а вторая рождается у кромки.
#[test]
fn a_wedge_with_odd_lanes_drifts_the_line_over_its_length() {
    let (layers, wide) = seam_of([2, 3]);
    let at_seam = all_lines(&layers, 200.0);
    assert!(at_seam.iter().any(|x| x.abs() < 1e-3), "{at_seam:?}");
    let lines = all_lines(&layers, wide);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!((lines[0] + lane_width() / 2.0).abs() < 1e-3, "{lines:?}");
    assert!((lines[1] - lane_width() / 2.0).abs() < 1e-3, "{lines:?}");
}

/// Пять полос (3 + 2) в шесть (3 + 3): осевая пяти — на границе потоков, в
/// полполосы от середины, у шести — посередине. На шве осевая широкого
/// стоит там же, где у узкого, и уходит на середину по длине клина, без
/// скачка (Ростов, Текучёва). Узкий, нарисованный навстречу, — зеркально.
#[test]
fn the_axis_runs_through_a_seam_of_different_splits() {
    let width = |lanes: u8| f32::from(lanes) * lane_width() + 1.0;
    for reversed in [false, true] {
        let mut narrow = with_lanes(
            street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], width(5)),
            5,
            false,
        );
        narrow.lanes_backward = Some(2);
        if reversed {
            narrow.points.reverse();
            narrow.lanes_backward = Some(3);
        }
        let wide = with_lanes(
            street(vec![Vec2::new(200.0, 0.0), Vec2::new(400.0, 0.0)], width(6)),
            6,
            false,
        );
        // плюс — влево по ходу широкого; поток по ходу — справа, в три
        // полосы, и осевая пяти — в полполосы левее середины
        let seam = lane_width() / 2.0;
        // у шва узел сетки широкого — на осевой узкого: осевая шести (k = 0)
        // начинается там же, где кончилась осевая пяти
        let taper = Taper {
            length: 33.0,
            narrow: 0,
            sides: [true; 2],
        };
        let wedge = WedgeEnd::new(&wide, &narrow, taper, false, 33.0, TrafficSide::Right);
        let frame = wedge_frame(lane_frame(6), wedge, false);
        assert!(
            (frame.origin - seam).abs() < 1e-6,
            "reversed {reversed}: {frame:?}"
        );
        let (layers, report) = mesh_roads(
            &map_of(vec![narrow, wide]),
            RoadStyle::default(),
            RoadShape::default(),
        );
        assert_eq!(report.drawn.tapers, 1);
        let at_seam = line_offsets(&layers, PAINT_AXES, 200.0);
        assert!(
            at_seam.iter().all(|&y| (y - seam).abs() < 1e-3) && !at_seam.is_empty(),
            "reversed {reversed}: {at_seam:?}"
        );
        let taper = (width(6) - width(5)) * tapers::TAPER_PER_METER;
        let after = line_offsets(&layers, PAINT_AXES, 200.0 + taper);
        assert!(
            after.iter().all(|&y| y.abs() < 1e-3) && !after.is_empty(),
            "reversed {reversed}: {after:?}"
        );
    }
}

#[test]
fn the_dash_phase_runs_along_the_street() {
    let seam = Vec2::new(100.0, 0.0);
    let roads = vec![
        street(vec![Vec2::ZERO, seam], 7.6),
        // второй way нарисован навстречу улице
        street(vec![Vec2::new(250.0, 0.0), seam], 7.6),
    ];
    let network = RoadNetwork::new(&roads);
    let paths: Vec<Vec<Vec2>> = roads.iter().map(|road| road.points.clone()).collect();
    let stations = street_stations(&network, &paths);
    let (first, second) = (stations[0], stations[1]);
    // улица может идти в любую сторону — важно, что длина на шве одна
    let at_seam = |Station { start, reversed }: Station, length: f32| {
        if reversed {
            start - length
        } else {
            start + length
        }
    };
    let seam_first = at_seam(first, 100.0);
    let seam_second = at_seam(second, 150.0);
    assert!(
        (seam_first - seam_second).abs() < 1e-3,
        "{stations:?}: {seam_first} vs {seam_second}"
    );
}

#[test]
fn paint_tags_name_their_layers() {
    assert_eq!(PaintTag::of(PAINT_LANES), Some(PaintTag::Lanes));
    assert_eq!(PaintTag::of(BRIDGE_PAINT_AXES), Some(PaintTag::Axes));
    assert_eq!(PaintTag::of(PAINT_ZEBRAS), Some(PaintTag::Zebras));
    assert_eq!(PaintTag::of("roads"), None);
}

#[test]
fn the_paint_ladder_hides_lane_lines_first() {
    assert_eq!(PaintZoomBucket::for_zoom(0.2).index, 0);
    assert_eq!(PaintZoomBucket::for_zoom(0.5).index, 1);
    assert_eq!(PaintZoomBucket::for_zoom(0.7).index, 2);
    assert_eq!(PaintZoomBucket::for_zoom(2.0).index, 3);
}

#[test]
fn a_crossing_paints_a_zebra_and_stop_lines_across_the_arms() {
    // крестовина двух `tertiary`: четыре плеча, на каждом зебра и стоп-линия
    let map = map_of(vec![
        with_lanes(
            RoadLine {
                highway: Highway::Tertiary,
                ..street(
                    vec![Vec2::ZERO, Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0)],
                    7.6,
                )
            },
            2,
            false,
        ),
        with_lanes(
            RoadLine {
                highway: Highway::Tertiary,
                ..street(
                    vec![
                        Vec2::new(100.0, -100.0),
                        Vec2::new(100.0, 0.0),
                        Vec2::new(100.0, 100.0),
                    ],
                    7.6,
                )
            },
            2,
            false,
        ),
    ]);
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.junctions.zebras, [4, 0]);
    assert_eq!(report.junctions.stop_lines, 4);
    let zebras = paint_layer(&layers, PAINT_ZEBRAS);
    assert_eq!(zebras.vertex_count(), 4 * 4);
    let kinds = zebras.ribbon_coords_for_test().unwrap();
    assert!(
        kinds
            .iter()
            .all(|coords| coords[3] == LineKind::Zebra.code())
    );
    // стоп-линии — в меше линий полос: их видно до того же зума
    let stops = paint_layer(&layers, PAINT_LANES)
        .ribbon_coords_for_test()
        .unwrap();
    assert_eq!(
        stops
            .iter()
            .filter(|coords| coords[3] == LineKind::Stop.code())
            .count(),
        4 * 4
    );

    let (layers, report) = mesh_roads(
        &map,
        RoadStyle {
            crossings: CrossingMode::Off,
            stop_lines: false,
            ..RoadStyle::default()
        },
        RoadShape::default(),
    );
    assert_eq!(report.junctions.zebras, [0, 0]);
    assert_eq!(report.junctions.stop_lines, 0);
    assert!(paint_layer(&layers, PAINT_ZEBRAS).is_empty());
}

/// Звено зебры — только целиком, и звенья по середине проезжей части
/// (roads list R2): на улице в 8 м планка в 7.4 м кончалась на 0.4 периода,
/// и крайнее звено выходило клином в 0.15 м у кромки (Тула, Фёдора Смирнова).
#[test]
fn a_zebra_is_whole_bars_centred_between_the_kerbs() {
    use crate::map::osm::model::{RoadNode, RoadNodeKind};
    let at = Vec2::new(100.0, 0.0);
    let mut map = map_of(vec![with_lanes(
        RoadLine {
            highway: Highway::Tertiary,
            ..street(vec![Vec2::ZERO, at, Vec2::new(200.0, 0.0)], 8.0)
        },
        2,
        false,
    )]);
    map.road_nodes.push(RoadNode {
        pos: at,
        kind: RoadNodeKind::Crossing {
            signals: false,
            island: false,
            marked: true,
        },
    });
    let (layers, report) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    assert_eq!(report.junctions.zebras, [1, 1]);
    let zebras = paint_layer(&layers, PAINT_ZEBRAS);
    let coords = zebras.ribbon_coords_for_test().unwrap();
    let (first, last) = coords.iter().fold((f32::MAX, f32::MIN), |(low, high), c| {
        (low.min(c[1]), high.max(c[1]))
    });
    // первое звено — от самого торца, последнее кончается торцом
    assert!((first - ZEBRA_FIRST_BAR).abs() < 1e-3, "{first}");
    let bars = (last - first + ZEBRA_PERIOD - ZEBRA_BAR) / ZEBRA_PERIOD;
    assert!((bars - bars.round()).abs() < 1e-3, "{bars} bars");
    // поля до кромок поровну
    let across: Vec<f32> = zebras.positions_for_test().iter().map(|p| p[1]).collect();
    let (low, high) = across.iter().fold((f32::MAX, f32::MIN), |(low, high), &y| {
        (low.min(y), high.max(y))
    });
    assert!((low + high).abs() < 1e-3, "{low}..{high}");
    assert!(high <= 4.0 && high > 3.0, "{high}");
}

#[test]
fn a_pocket_line_ends_at_the_junction_and_the_rest_run_through() {
    // четыре полосы переходят в две на примыкании жилой: крайние линии широкой
    // — в карман, осевая идёт сквозь узел
    let node = Vec2::new(100.0, 0.0);
    let mut wide = with_lanes(street(vec![Vec2::ZERO, node], 14.2), 4, false);
    wide.highway = crate::map::osm::Highway::Tertiary;
    let mut narrow = with_lanes(street(vec![node, Vec2::new(200.0, 0.0)], 7.6), 2, false);
    narrow.highway = crate::map::osm::Highway::Tertiary;
    let side = with_lanes(street(vec![Vec2::new(100.0, -80.0), node], 7.6), 2, false);
    let (layers, report) = mesh_roads(
        &map_of(vec![wide, narrow, side]),
        RoadStyle::default(),
        RoadShape::default(),
    );
    assert_eq!(report.junctions.pockets, 1);
    // у самого узла: линия полос в кармане уже погашена, осевая — нет
    let near = |name: &str| -> Vec<f32> {
        paint_layer(&layers, name)
            .ribbon_coords_for_test()
            .unwrap()
            .iter()
            .zip(paint_layer(&layers, name).positions_for_test())
            .filter(|(_, position)| (position[0] - 99.0).abs() < 1.5 && position[1].abs() < 8.0)
            .map(|(coords, _)| coords[2])
            .collect()
    };
    assert!(near(PAINT_LANES).iter().all(|&to_break| to_break < 0.0));
    assert!(near(PAINT_AXES).iter().any(|&to_break| to_break > 5.0));
}

/// Колея траекторий — одни и те же полосы в маске и в наложении (иначе
/// наложение светлило бы по маске чужой колеи), сила кривой целая, у хвоста
/// сходит в ноль вглубь полосы.
#[test]
fn turn_wear_lies_in_both_passes_and_its_tails_fade() {
    let junction = JunctionWear {
        curves: vec![
            vec![Vec2::new(-15.0, 0.0), Vec2::new(15.0, 0.0)],
            vec![Vec2::new(0.0, -15.0), Vec2::new(0.0, 15.0)],
        ],
        tails: vec![[Vec2::new(15.0, 0.0), Vec2::new(20.0, 0.0)]],
    };
    let mut painter = Painter::default();
    painter.paint_turn_wear(&[junction]);
    let (mask, wear) = (&painter.wear_mask, &painter.wear);
    assert!(!wear.is_empty());
    assert_eq!(mask.positions_for_test(), wear.positions_for_test());
    let ribbon = wear.ribbon_coords_for_test().expect("координаты ленты");
    assert!(
        ribbon
            .iter()
            .all(|coords| coords[3] == LineKind::Wear.code())
    );
    let alphas: Vec<f32> = wear
        .colors_for_test()
        .iter()
        .map(|color| color[3])
        .collect();
    // две кривые по четыре вершины и хвост: целая у кромки, ноль в глубине
    assert!(alphas[..8].iter().all(|&alpha| alpha == 1.0));
    assert_eq!(&alphas[8..], &[1.0, 1.0, 0.0, 0.0]);
}

#[test]
fn the_wear_passes_have_their_own_materials_under_the_lines() {
    let layers = Painter::default().layers();
    let at = |name: &str| {
        layers
            .iter()
            .find(|layer| layer.name == name)
            .map(|layer| (layer.z, layer.material))
            .expect(name)
    };
    let (mask_z, mask) = at(PAINT_WEAR_MASK);
    let (wear_z, wear) = at(PAINT_WEAR);
    let (lines_z, lines) = at(PAINT_LANES);
    assert_eq!(mask, MaterialSpec::Paint(PaintPass::WearMask));
    assert_eq!(wear, MaterialSpec::Paint(PaintPass::Wear));
    assert_eq!(lines, MaterialSpec::Paint(PaintPass::Lines));
    assert!(
        mask_z < wear_z && wear_z < lines_z,
        "маска, наложение, линии"
    );
}

#[test]
fn a_lane_line_is_solid_only_on_the_approach() {
    let along = [0.0, 10.0, 20.0, 30.0];
    // разрыв узла — у конца пути, с 25 м
    let ahead = [25.0, 15.0, 5.0, -5.0];
    assert_eq!(approach_spans(&along, &ahead, true), vec![(0.0, 25.0)]);
    assert!(
        approach_spans(&along, &ahead, false).is_empty(),
        "против хода — это выезд из узла: пунктир сразу"
    );
    // разрыв у начала, до 5 м: подход — против хода точек
    let behind = [-5.0, 5.0, 15.0, 25.0];
    assert_eq!(approach_spans(&along, &behind, false), vec![(5.0, 30.0)]);
    assert!(approach_spans(&along, &behind, true).is_empty());
}

#[test]
fn the_approach_splits_the_line_into_dashed_and_solid_links() {
    let along = [0.0, 40.0];
    let line = vec![Vec2::ZERO, Vec2::new(40.0, 0.0)];
    let stations = vec![
        PaintStation {
            along: 0.0,
            to_break: 40.0,
            alpha: 1.0,
        },
        PaintStation {
            along: 40.0,
            to_break: 0.0,
            alpha: 1.0,
        },
    ];
    let (line, stations, solid) = split_at_spans(line, stations, &along, &[(15.0, 40.0)]);
    assert_eq!(
        line,
        vec![Vec2::ZERO, Vec2::new(15.0, 0.0), Vec2::new(40.0, 0.0)]
    );
    assert_eq!(stations[1].to_break, 25.0);
    assert_eq!(solid, vec![false, true]);
}

/// Второй ряд стрелок — в 20 м за первым, если полоса длинная и чистая; не
/// встаёт на соседний узел и за переход.
#[test]
fn a_long_approach_gets_a_second_row_of_arrows() {
    // полоса едет по +x к кромке узла в x = 0, ось назад — 60 м
    let arrow = |back: f32| LaneArrow {
        road: 0,
        at: Vec2::ZERO,
        travel: Vec2::X,
        turn: LaneTurn {
            left: true,
            through: true,
            right: false,
        },
        back: vec![Vec2::ZERO, Vec2::new(-back, 0.0)],
    };
    let clear = ArrowMarks::new(&[], &[]);
    assert_eq!(
        Painter::repeat_setback(&arrow(60.0), 4.0, &clear, &[]),
        Some(24.0)
    );
    // короткий перегон — второго ряда нет
    assert_eq!(
        Painter::repeat_setback(&arrow(30.0), 4.0, &clear, &[]),
        None
    );
    // разрыв соседнего узла под вторым рядом
    let next = Break {
        at: Vec2::new(-30.0, 0.0),
        reach: 6.0,
    };
    assert_eq!(
        Painter::repeat_setback(&arrow(60.0), 4.0, &clear, &[next]),
        None
    );
    // переход между рядами
    let zebra = Zebra {
        from: Vec2::new(-15.0, -5.0),
        to: Vec2::new(-15.0, 5.0),
        osm: true,
    };
    let marked = ArrowMarks::new(&[zebra], &[]);
    assert_eq!(
        Painter::repeat_setback(&arrow(60.0), 4.0, &marked, &[]),
        None
    );
}

#[test]
fn the_axis_is_solid_on_both_sides_of_a_break() {
    let along = [0.0, 10.0, 50.0, 90.0, 100.0];
    // разрыв посередине: до его края 40 м с обеих сторон
    let to_break = [40.0, 30.0, -10.0, 30.0, 40.0];
    let spans = near_spans(&along, &to_break, APPROACH);
    assert_eq!(spans.len(), 1, "{spans:?}");
    let (from, to) = spans[0];
    assert!(
        (from - 15.0).abs() < 1e-3 && (to - 85.0).abs() < 1e-3,
        "{spans:?}"
    );
    // путь начинается внутри зоны и кончается в ней
    assert_eq!(
        near_spans(&[0.0, 100.0], &[0.0, 0.0], APPROACH),
        vec![(0.0, 100.0)]
    );
    assert!(near_spans(&[0.0, 100.0], &[f32::INFINITY; 2], APPROACH).is_empty());
}

/// Жилая примыкает к третичной сбоку: третичная проходит узел насквозь, её
/// осевая не рвётся — и у примыкания она сплошная, а на перегоне пунктир.
#[test]
fn the_axis_of_a_through_street_is_solid_at_a_side_street() {
    let mut main = with_lanes(
        street(
            vec![Vec2::ZERO, Vec2::new(150.0, 0.0), Vec2::new(300.0, 0.0)],
            7.6,
        ),
        2,
        false,
    );
    main.highway = Highway::Tertiary;
    let side = with_lanes(
        street(vec![Vec2::new(150.0, -80.0), Vec2::new(150.0, 0.0)], 7.6),
        2,
        false,
    );
    let map = map_of(vec![main, side]);
    let (layers, _) = mesh_roads(&map, RoadStyle::default(), RoadShape::default());
    let builder = paint_layer(&layers, PAINT_AXES);
    let positions = builder.positions_for_test();
    let ribbons = builder.ribbon_coords_for_test().unwrap();
    // x вершин осевой главной (на оси y = 0) одного вида: вид у куска
    // один, вершины у него только на концах и изломах
    let xs_of = |kind: LineKind| {
        positions
            .iter()
            .zip(ribbons.iter())
            .filter(|(at, ribbon)| at[1].abs() < 2.0 && ribbon[3] == kind.code())
            .map(|(at, _)| at[0])
            .collect::<Vec<f32>>()
    };
    let solid = xs_of(LineKind::AxisSolid);
    let dashed = xs_of(LineKind::AxisDashed);
    // сплошная — от ~25 м до зоны узла до ~25 м после
    let low = solid
        .iter()
        .copied()
        .filter(|&x| x > 100.0)
        .fold(f32::INFINITY, f32::min);
    let high = solid
        .iter()
        .copied()
        .filter(|&x| x < 200.0)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(low < 125.0 && high > 175.0, "{solid:?}");
    // пунктир — на перегонах с обеих сторон и не у узла
    assert!(dashed.iter().any(|&x| x < 100.0), "{dashed:?}");
    assert!(dashed.iter().any(|&x| x > 200.0), "{dashed:?}");
    assert!(
        dashed
            .iter()
            .all(|&x| !(low + 0.5..high - 0.5).contains(&x)),
        "{dashed:?}"
    );
}
