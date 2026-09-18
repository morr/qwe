//! Арки — проходы `building_passage` сквозь нарисованный дом.
//!
//! Солнце — процессная глобаль, а тесты идут параллельными потоками в одном
//! процессе: каждый тест здесь строит освещённую геометрию, поэтому каждый
//! берёт `default_sun()` — гард, который держит солнце на дефолте и не пускает
//! к нему соседа (`map/sun.rs`).

use super::*;
use crate::map::buildings::fixtures::{
    building, detail, extruded_mesh, is_solid, is_wall, oblong, square, whole_cells,
};
use crate::map::buildings::layers::wall_columns;
use crate::map::buildings::material::WallKind;
use crate::map::buildings::{BuildingHeightMode, EXTRUDE_RANGE, Lean, extrusion_lift};
use crate::map::osm::{AreaKind, BuildingUse, fixture};

fn passage(points: Vec<Vec2>, passage: bool) -> RoadLine {
    if passage {
        fixture::passage(points, 5.0)
    } else {
        fixture::street(points, 5.0)
    }
}

/// Проём режется только под дорогой с флагом `passage`, и только если
/// она действительно идёт сквозь дом.
#[test]
fn only_a_building_passage_cuts_an_arch() {
    let _sun = crate::map::default_sun();
    let house = vec![building(square(), Some(15.0), AreaKind::Building)];
    let through = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        true,
    )];
    let alongside = vec![passage(
        vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)],
        false,
    )];
    let elsewhere = vec![passage(
        vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 10.0)],
        true,
    )];

    let solid = extruded_mesh(&house, &[], detail(false)).vertex_count();
    assert!(extruded_mesh(&house, &through, detail(false)).vertex_count() > solid);
    assert_eq!(
        extruded_mesh(&house, &alongside, detail(false)).vertex_count(),
        solid
    );
    assert_eq!(
        extruded_mesh(&house, &elsewhere, detail(false)).vertex_count(),
        solid
    );
}

/// Высота проёма задана в настоящих метрах, а рисуется проекция:
/// трёхметровая арка обязана занять ту же долю нарисованной стены, какую
/// три метра занимают в настоящей высоте дома.
#[test]
fn an_arch_opening_is_three_real_metres_of_the_drawn_wall() {
    let _sun = crate::map::default_sun();
    // 40 м высоты, подъём 14 м: арка обязана занять 14 × 3/40 = 1.05 м
    let tall = building(square(), Some(40.0), AreaKind::Building);
    let lift = extrusion_lift(&tall, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &tall, &[&road], lift);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        let low = values.iter().copied().fold(f32::INFINITY, f32::min);
        let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        high - low
    };

    let expected = lift.y * ARCH_HEIGHT / 40.0;
    assert!(
        (span(|p| p[1]) - expected).abs() < 0.01,
        "opening is {} m tall, expected {expected} m of a {} m wall",
        span(|p| p[1]),
        lift.y
    );
}

/// У низкого дома нарисованный метр стоит других настоящих метров:
/// подъём обрезан `EXTRUDE_RANGE`, и пересчёт через `EXTRUDE_SCALE` дал бы
/// не ту долю. Проверяем, что доля считается от высоты самого дома.
#[test]
fn a_clamped_wall_still_gets_a_proportional_opening() {
    let _sun = crate::map::default_sun();
    // 2 м высоты: подъём 2 × 0.35 = 0.7 обрезается снизу до 1 м
    let low = building(square(), Some(2.0), AreaKind::Building);
    let lift = extrusion_lift(&low, BuildingHeightMode::Extrusion);
    assert_eq!(lift.y, *EXTRUDE_RANGE.start());

    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);
    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &low, &[&road], lift);

    let heights: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .collect();
    let opening = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - heights.iter().copied().fold(f32::INFINITY, f32::min);
    // арка выше самого дома (6 > 2) — проём режется по стене целиком
    assert!(
        (opening - lift.y).abs() < 0.01,
        "opening {opening} m, expected the whole {} m wall",
        lift.y
    );
}

/// Проём лежит в плоскости стены и шириной с проход, а не растянут вдоль
/// дороги: у дороги, подходящей к южной грани под углом, вырез всё равно
/// ровно по грани.
#[test]
fn an_arch_is_cut_along_the_wall_not_along_the_road() {
    let _sun = crate::map::default_sun();
    let house = building(square(), Some(15.0), AreaKind::Building);
    // `push_arches` — фасадный режим: полоса сдвинута строго вниз. Косой
    // подъём 2.5D сюда не годится — его x-составляющая растянула бы проём
    let band = Vec2::new(0.0, -3.0);
    // дорога идёт наискось и коротка: до стены дотягивается один конец
    let slanted = passage(vec![Vec2::new(4.0, -2.0), Vec2::new(9.0, 20.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&slanted], band);

    let span = |pick: fn(&[f32; 3]) -> f32| {
        let values: Vec<f32> = builder.positions_for_test().iter().map(pick).collect();
        values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - values.iter().copied().fold(f32::INFINITY, f32::min)
    };
    // ширина дороги, спроецированная углом входа: наклонная дорога
    // дырявит стену уже собственной ширины
    let entry = (Vec2::new(9.0, 20.0) - Vec2::new(4.0, -2.0)).normalize();
    let expected = slanted.width * entry.perp_dot(Vec2::X).abs();
    assert!(
        (span(|p| p[0]) - expected).abs() < 0.01,
        "opening is {} m wide, expected {expected} m",
        span(|p| p[0])
    );
}

/// Регресс: конец прохода — общая вершина двух граней (как у любой
/// OSM-арки). Зажатый в одну грань проём выходил вдвое уже дороги;
/// теперь куски на обеих гранях продолжают друг друга.
#[test]
fn an_arch_at_a_shared_vertex_keeps_the_road_width() {
    let _sun = crate::map::default_sun();
    // южная сторона из двух граней со стыком в (5, 0)
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ],
        Some(15.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], lift);

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let width = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - xs.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        width >= road.width * 0.9,
        "opening is {width} m wide against a {} m road",
        road.width
    );
}

/// Сквозь скошенный проём видна боковая стенка проезда — одна, та, что
/// смотрит против подъёма, и только в пределах дома.
#[test]
fn an_arch_shows_the_side_wall_of_its_passage() {
    let _sun = crate::map::default_sun();
    let house = building(square(), Some(15.0), AreaKind::Building);
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let road = passage(vec![Vec2::new(5.0, -2.0), Vec2::new(5.0, 12.0)], true);

    let walls = tunnel_walls(&house, &[&road], lift, -Lean::of().dir());

    assert_eq!(walls.len(), 1, "exactly one side of the passage is visible");
    let wall = &walls[0];
    // подъём уходит вправо — видна восточная стенка, смотрящая на запад
    let half = road.width / 2.0;
    assert!((wall.a.x - (5.0 + half)).abs() < 1e-3 && (wall.b.x - (5.0 + half)).abs() < 1e-3);
    let (low, high) = (wall.a.y.min(wall.b.y), wall.a.y.max(wall.b.y));
    assert!(
        low.abs() < 1e-3 && (high - 10.0).abs() < 1e-3,
        "wall runs {low}..{high}"
    );
    assert!(wall.sill.length() > 0.0 && wall.sill.length() <= lift.length());
}

/// Арка у самого угла дома: проём подрезается по концу грани, а не
/// повисает половиной квада в воздухе за углом.
#[test]
fn an_arch_near_a_corner_is_trimmed_to_the_wall() {
    let _sun = crate::map::default_sun();
    let house = building(square(), Some(15.0), AreaKind::Building);
    // фасадная полоса, как в `an_arch_is_cut_along_the_wall_not_along_the_road`
    let band = Vec2::new(0.0, -3.0);
    // дорога упирается в южную грань в метре от юго-западного угла
    let road = passage(vec![Vec2::new(1.0, 0.0), Vec2::new(1.0, 12.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&road], band);
    assert!(!builder.is_empty());

    let xs: Vec<f32> = builder
        .positions_for_test()
        .iter()
        .map(|position| position[0])
        .collect();
    let west = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let east = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(west >= -0.01, "opening hangs past the corner: {west}");
    // а восточный край — там, куда дотянулась полуширина
    assert!((east - 3.5).abs() < 0.01, "{east}");
}

/// Регресс на реальную арку 485488257 (Тула): проход размечен отрезком
/// **между двумя вершинами контура**, лежит внутри дома и стен касается
/// только концами. Поиск пересечения дороги с контуром здесь не находит
/// ничего — вырез обязан появиться от концов.
#[test]
fn an_arch_lying_inside_the_outline_still_cuts_an_opening() {
    let _sun = crate::map::default_sun();
    // упрощённая геометрия того дома: южная грань y = 0, арка — отрезок
    // от вершины (5, 0) вглубь до вершины (5.2, 14) северной грани
    let house = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 14.0),
            Vec2::new(5.2, 14.0),
            Vec2::new(0.0, 14.0),
        ],
        Some(42.0),
        AreaKind::Building,
    );
    let lift = extrusion_lift(&house, BuildingHeightMode::Extrusion);
    let inner = passage(vec![Vec2::new(5.0, 0.0), Vec2::new(5.2, 14.0)], true);

    let mut builder = MeshBuilder::default();
    push_arches(&mut builder, &house, &[&inner], lift);
    assert!(
        !builder.is_empty(),
        "an outline-to-outline passage cut nothing"
    );
    // и проём сидит на южной грани — начинается на y = 0
    let bottom = builder
        .positions_for_test()
        .iter()
        .map(|position| position[1])
        .fold(f32::INFINITY, f32::min);
    assert!(bottom.abs() < 0.01, "opening floats at y = {bottom}");
}

/// Вырез арки шейдеру не виден: окна и балконы он ставит по центру клетки
/// стены, ничего не зная о дыре, и край проёма резал бы их пополам — и на
/// простенке сбоку от арки, и на перемычке над ней. Поэтому клетки, которые
/// проём задел, стена отдаёт ему целиком — заплатой «без проёмов», ровно как
/// под дверью.
///
/// Проверяется это тем же, чем держится вся рама стены: у **всякого** куска
/// стены, на котором проёмы рисуются, границы обязаны лежать на швах клеток —
/// целое число панелей вдоль основания и целое число этажей вверх (или самый
/// верх стены, где над последним этажом лежит запас под карниз).
#[test]
fn the_wall_around_an_arch_wears_no_half_windows() {
    let _sun = crate::map::default_sun();
    let mut block = building(oblong(20.0, 40.0), Some(30.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    // проезд сквозь дом упирается в южную грань на x = 12, между швами
    let road = passage(vec![Vec2::new(12.0, -2.0), Vec2::new(12.0, 25.0)], true);
    let lift = extrusion_lift(&block, BuildingHeightMode::Extrusion);

    let builder = extruded_mesh(std::slice::from_ref(&block), &[road], detail(false));
    let frames = builder.roof_coords_for_test().expect("roof coords");
    // клетки этой стены: 40 м на целое число панелей, подъём на этажи с
    // запасом под карниз
    // облицовка тут любая, кроме гаражной: боксом ячейку меряет только
    // `WallKind::GarageDoors`, у всех остальных она панельная
    let panel = 40.0 / wall_columns(40.0, WallKind::Panel);
    // этажей столько же, сколько насчитал бы `storeys_of`, — целое число
    let storeys = (30.0 / crate::settings::STOREY_HEIGHT).round().max(1.0);
    let storey = lift.y / (storeys + crate::map::meshing::PARAPET_CELLS);

    let mut patched = 0;
    for (point, frame) in builder.positions_for_test().iter().zip(frames) {
        if !is_wall(frame[2]) {
            continue;
        }
        // южная стена: основание на y = 0, верх уехал по подъёму, поэтому
        // место вдоль стены считается с поправкой на косину
        let (x, y) = (point[0], point[1]);
        let along = x - y / lift.y * lift.x;
        if !(-0.01..=lift.y + 0.01).contains(&y) || !(-0.01..=40.01).contains(&along) {
            continue;
        }
        if is_solid(frame[3]) {
            patched += 1;
            continue;
        }
        assert!(
            whole_cells(along / panel),
            "окно разрезано вдоль: стена с проёмами кончается на {along} м, \
             панель — {panel} м"
        );
        assert!(
            whole_cells(y / storey) || (y - lift.y).abs() < 0.01,
            "окно разрезано поперёк: стена с проёмами кончается на {y} м, \
             этаж — {storey} м"
        );
    }
    assert!(patched > 0, "вокруг арки должна лечь заплата без проёмов");
}

/// Два проезда, выходящие в одну грань в пределах одних панелей, — это два
/// выреза, а не один. Заплата кроится по целым панелям, и соседний проём,
/// попавший в тот же блок, отбрасывался целиком: простенок первой арки заливал
/// его сплошной стеной, хотя навмеш прорезан обоими проездами.
#[test]
fn two_arches_in_one_panel_block_each_keep_their_opening() {
    let _sun = crate::map::default_sun();
    let (a, b) = (Vec2::ZERO, Vec2::new(40.0, 0.0));
    let lift = Vec2::new(0.0, 12.0);
    let cells = WallCells {
        frame: None,
        patch: None,
        panel: 3.2,
        storey: Vec2::new(0.0, 3.0),
    };
    let sill = Vec2::new(0.0, 2.0);
    // второй проезд целиком внутри блока панелей первого: 6.2 < ceil(4.0 / 3.2) * 3.2
    let openings = vec![
        ArchOpening {
            a,
            b,
            low: 0.5,
            high: 4.0,
            sill,
        },
        ArchOpening {
            a,
            b,
            low: 5.0,
            high: 6.2,
            sill,
        },
    ];
    let span = WallSpan::new(a, b, lift, 4.0, None);

    let mut builder = MeshBuilder::default();
    push_wall_with_openings(
        &mut builder,
        &span,
        &cells,
        &openings,
        LinearRgba::WHITE,
        LinearRgba::WHITE,
    );

    // ни один кусок стены не лежит поперёк проёма ниже его перемычки
    for quad in builder.positions_for_test().chunks(4) {
        let (from, to) = quad
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), point| {
                (low.min(point[0]), high.max(point[0]))
            });
        let base = quad.iter().fold(f32::MAX, |low, point| low.min(point[1]));
        if base >= sill.y - 0.01 {
            continue;
        }
        for opening in &openings {
            assert!(
                from >= opening.high - 0.01 || to <= opening.low + 0.01,
                "стена {from}..{to} лежит поперёк проёма {}..{}",
                opening.low,
                opening.high
            );
        }
    }
}

/// Середина прохода берётся по длине, а не по числу точек: у ломаной с
/// одним длинным и одним коротким сегментом это разные точки.
#[test]
fn the_passage_middle_is_measured_along_its_length() {
    let _sun = crate::map::default_sun();
    let road = passage(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(90.0, 0.0),
            Vec2::new(100.0, 0.0),
        ],
        true,
    );
    let middle = passage_middle(&road).unwrap();
    assert!((middle.x - 50.0).abs() < 0.01, "{middle:?}");
}
