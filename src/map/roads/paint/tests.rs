use super::*;
use crate::map::osm::MapData;
use crate::map::osm::fixture::street;
use crate::map::roads::{RoadStyle, mesh_roads};

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
    let (layers, report) = mesh_roads(&map, RoadStyle::default());
    assert!(paint_layer(&layers, PAINT_LANES).is_empty());
    assert_eq!(line_offsets(&layers, PAINT_AXES, 0.0), vec![0.0]);
    assert_eq!(report.paint_lines, 1);
    let kinds = paint_layer(&layers, PAINT_AXES)
        .ribbon_coords_for_test()
        .unwrap();
    assert!(
        kinds
            .iter()
            .all(|ribbon| ribbon[3] == LineKind::Axis.code())
    );
}

#[test]
fn four_lanes_get_a_double_axis_and_a_lane_line_each_way() {
    let map = map_of(vec![with_lanes(
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.2),
        4,
        false,
    )]);
    let (layers, _) = mesh_roads(&map, RoadStyle::default());
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
    assert!((lanes[0] + STREET_LANE_WIDTH).abs() < 1e-3, "{lanes:?}");
    assert!((lanes[1] - STREET_LANE_WIDTH).abs() < 1e-3, "{lanes:?}");
}

#[test]
fn a_one_way_street_has_no_axis() {
    let map = map_of(vec![with_lanes(
        street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 10.9),
        3,
        true,
    )]);
    let (layers, _) = mesh_roads(&map, RoadStyle::default());
    assert!(paint_layer(&layers, PAINT_AXES).is_empty());
    // три полосы — две линии, по сетке от полполосы
    let lanes = line_offsets(&layers, PAINT_LANES, 0.0);
    assert_eq!(lanes.len(), 2, "{lanes:?}");
    assert!(
        (lanes[0] + STREET_LANE_WIDTH / 2.0).abs() < 1e-3,
        "{lanes:?}"
    );
    assert!(
        (lanes[1] - STREET_LANE_WIDTH / 2.0).abs() < 1e-3,
        "{lanes:?}"
    );
}

#[test]
fn markings_off_paint_nothing() {
    let map = map_of(vec![street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 14.2)]);
    let style = RoadStyle {
        markings: false,
        ..RoadStyle::default()
    };
    let (layers, report) = mesh_roads(&map, style);
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

#[test]
fn a_bridge_paints_into_its_own_layer() {
    let mut road = street(vec![Vec2::ZERO, Vec2::new(200.0, 0.0)], 7.6);
    road.bridge = true;
    let (layers, _) = mesh_roads(&map_of(vec![road]), RoadStyle::default());
    assert!(paint_layer(&layers, PAINT_AXES).is_empty());
    assert!(!paint_layer(&layers, BRIDGE_PAINT_AXES).is_empty());
}

/// Сетка полос раскладки — где на ней лежат границы полос внутри проезжей
/// части, поперёк пути.
fn grid(frame: LaneFrame) -> Vec<f32> {
    let from = ((frame.low - frame.origin) / STREET_LANE_WIDTH).ceil() as i32;
    let to = ((frame.high - frame.origin) / STREET_LANE_WIDTH).floor() as i32;
    let mut lines: Vec<f32> = (from..=to)
        .map(|k| frame.origin + k as f32 * STREET_LANE_WIDTH)
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
        for end in [false, true] {
            let [from, to] = wedge_frames(body, narrow, end);
            let body_frame = lane_frame(body);
            let paint_from = narrow_frame(body_frame, narrow, end);
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
    let seam = narrow_frame(body, 2, false);
    assert_eq!(grid(seam), grid(lane_frame(2)));
    assert_eq!(seam.origin, 0.0, "две полосы в четыре: линии не двигаются");
    let odd = narrow_frame(lane_frame(3), 2, false);
    assert!((lane_frame(3).origin - odd.origin - STREET_LANE_WIDTH / 2.0).abs() < 1e-4);
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
    let width = |lanes: u8| f32::from(lanes) * STREET_LANE_WIDTH + 1.0;
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
    let (layers, report) = mesh_roads(&map_of(roads), RoadStyle::default());
    assert_eq!(report.tapers, 1);
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
        assert!(
            (pair[1] - pair[0] - STREET_LANE_WIDTH).abs() < 1e-3,
            "{lines:?}"
        );
    }
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
    assert!(
        (lines[0] + STREET_LANE_WIDTH / 2.0).abs() < 1e-3,
        "{lines:?}"
    );
    assert!(
        (lines[1] - STREET_LANE_WIDTH / 2.0).abs() < 1e-3,
        "{lines:?}"
    );
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
    let at_seam = |(start, reversed): (f32, bool), length: f32| {
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
    assert_eq!(PaintTag::of("roads"), None);
}

#[test]
fn the_paint_ladder_hides_lane_lines_first() {
    assert_eq!(PaintZoomBucket::for_zoom(0.2).index, 0);
    assert_eq!(PaintZoomBucket::for_zoom(0.5).index, 1);
    assert_eq!(PaintZoomBucket::for_zoom(2.0).index, 2);
}
