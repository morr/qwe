//! Кровли: скатная когорта, конёк по длинной оси, отказы фронтона, скаты по
//! свету, вальма и крестовой двускат, слуховые окна, высоты частного сектора.

use super::super::classify::classify;
use super::super::fixtures::{building, oblong, square};
use super::super::material::*;
use super::super::roofs::*;
use super::super::*;
use super::support::house;
use crate::map::meshing::min_area_rect;
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, BuildingUse};

#[test]
fn a_pitched_roof_goes_on_houses_and_small_boxes_only() {
    let _sun = crate::map::default_sun();
    let mut house = building(oblong(8.0, 600.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(classify(&house, None).is_pitched(), "a house of any size");
    let small = building(square(), None, AreaKind::Building);
    assert!(classify(&small, None).is_pitched(), "an untagged small box");
    let big = building(oblong(20.0, 20.0), None, AreaKind::Building);
    assert!(!classify(&big, None).is_pitched(), "an untagged big box");
    let mut flats = building(square(), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(!classify(&flats, None).is_pitched());
    let tower = building(square(), None, AreaKind::Kremlin);
    assert!(
        !classify(&tower, None).is_pitched(),
        "the kremlin keeps its flat roof"
    );
    let mut yard = house.clone();
    yard.holes.push(vec![
        Vec2::new(4.0, 2.0),
        Vec2::new(6.0, 2.0),
        Vec2::new(6.0, 4.0),
        Vec2::new(4.0, 4.0),
    ]);
    assert!(
        !classify(&yard, None).is_pitched(),
        "a courtyard has no ridge"
    );
}

#[test]
fn the_ridge_runs_along_the_long_axis_of_the_rotated_footprint() {
    let _sun = crate::map::default_sun();
    // прямоугольник 20 × 6, повёрнутый на 30°, обойдённый по часовой
    let rotate = |p: Vec2| Vec2::from_angle(30f32.to_radians()).rotate(p);
    let mut ring: Vec<Vec2> = oblong(6.0, 20.0).into_iter().map(rotate).collect();
    ring.reverse();
    let rect = min_area_rect(&ring).unwrap();
    assert!(
        (rect[1] - rect[0]).length() > 19.9,
        "long side first: {rect:?}"
    );
    assert!(
        (rect[2] - rect[1]).length() < 6.1,
        "short side second: {rect:?}"
    );
    assert!(signed_ring_area(&rect) > 0.0, "CCW");
    // конёк — вдоль длинной оси
    let mut house = building(ring, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let roof = gable_roof(
        &house,
        &classify(&house, None),
        Vec2::ZERO,
        |_| Vec2::ZERO,
        Srgba::WHITE,
        None,
    )
    .unwrap();
    let ridge = roof.slopes[0].0[2] - roof.slopes[0].0[3];
    let long = rect[1] - rect[0];
    assert!(ridge.normalize().dot(long.normalize()).abs() > 0.999);
}

#[test]
fn an_l_shaped_house_is_pitched_but_takes_no_gable() {
    let _sun = crate::map::default_sun();
    let l_shape = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(12.0, 0.0),
        Vec2::new(12.0, 5.0),
        Vec2::new(5.0, 5.0),
        Vec2::new(5.0, 12.0),
        Vec2::new(0.0, 12.0),
    ];
    let mut house = building(l_shape, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    assert!(classify(&house, None).is_pitched());
    assert!(
        gable_roof(
            &house,
            &classify(&house, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            None
        )
        .is_none()
    );
}

#[test]
fn a_skewed_quad_takes_no_gable_whose_corner_hangs_off_the_wall() {
    let _sun = crate::map::default_sun();
    // Тула, way 968419942: заполняет свой прямоугольник на 0.91, а угол крыши
    // над ним висит в 2.2 м от стены — торец дома читался срезанным
    let skewed = vec![
        Vec2::new(412.56, 3158.32),
        Vec2::new(426.85, 3167.85),
        Vec2::new(420.76, 3178.18),
        Vec2::new(407.79, 3169.97),
    ]
    .into_iter()
    .map(|point| point - Vec2::new(400.0, 3150.0))
    .collect();
    let mut house = building(skewed, None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let facts = shape_facts(&house).unwrap();
    assert!(
        facts.rect_fill > 0.85,
        "fill alone would let it through: {}",
        facts.rect_fill
    );
    assert!(facts.gable_overhang > 2.0, "{}", facts.gable_overhang);
    assert!(
        gable_roof(
            &house,
            &classify(&house, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            None
        )
        .is_none()
    );
    // посев, по которому дому выпадает двускатная (`(seed >> 5) % 10 >= 4`)
    let gabled_seed = 5 << 5;
    assert!(matches!(
        roofing(
            &house,
            &classify(&house, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            gabled_seed,
            None
        ),
        Roofing::Hip(_)
    ));

    // слегка неровная обводка прямоугольника двускатную не теряет
    let mut sloppy = building(
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(12.0, 0.2),
            Vec2::new(12.0, 8.0),
            Vec2::new(0.1, 8.0),
        ],
        None,
        AreaKind::Building,
    );
    sloppy.building_use = BuildingUse::House;
    assert!(
        gable_roof(
            &sloppy,
            &classify(&sloppy, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            None
        )
        .is_some()
    );
}

#[test]
fn gable_roof_itself_refuses_a_building_that_is_not_gabled() {
    let _sun = crate::map::default_sun();
    let mut flats = building(oblong(8.0, 20.0), None, AreaKind::Building);
    flats.building_use = BuildingUse::Apartments;
    assert!(
        gable_roof(
            &flats,
            &classify(&flats, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            None
        )
        .is_none()
    );
}

#[test]
fn the_slope_facing_the_light_is_lighter_and_the_ridge_is_lifted() {
    let _sun = crate::map::default_sun();
    let luminance = |color: LinearRgba| color.red + color.green + color.blue;
    let mut house = building(oblong(8.0, 20.0), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let base = Srgba::rgb(0.5, 0.5, 0.5);
    let roof = gable_roof(
        &house,
        &classify(&house, None),
        Vec2::ZERO,
        |rise| Lean::of().ridge(rise),
        base,
        None,
    )
    .unwrap();
    // скаты: южный (карниз y = 0) отвёрнут от света, северный повёрнут
    let (south, north) = (&roof.slopes[0], &roof.slopes[1]);
    assert_eq!(south.0[0].y, 0.0);
    assert!(luminance(south.1) < luminance(base.into()));
    assert!(luminance(north.1) > luminance(base.into()));
    // конёк поднят по вектору подъёма на масштаб стен
    let ridge = south.0[3] - Vec2::new(0.0, 4.0);
    let expected = Lean::of().ridge(ridge_rise(8.0));
    assert!(ridge.distance(expected) < 1e-4, "{ridge:?} vs {expected:?}");
    assert!(expected.y > 0.0 && expected.x > 0.0);
    // конёк — вдоль длинной оси, оба фронтона стоят на торцах
    for ((a, b), face) in roof.gables {
        assert!((b - a).length() < 8.1, "gable on the short side");
        assert!(face[2].distance((a + b) / 2.0 + expected) < 1e-4);
    }
}

#[test]
fn a_house_without_height_stays_low() {
    let _sun = crate::map::default_sun();
    // квадрат 10 × 10 — мелкое пятно: частный дом это один-два этажа, дом без
    // назначения на таком пятне тоже низкий, но выше. Разбор вывода — в
    // тестах `heights`, здесь достаточно, что дефолт больше не один на всех
    let mut house = building(square(), None, AreaKind::Building);
    house.building_use = BuildingUse::House;
    let other = building(square(), None, AreaKind::Building);
    assert!(height_or_default(&house) <= 8.0);
    assert!(height_or_default(&other) <= 12.0);
}

/// Площадь плоской фигуры по её квадам и полигонам — для проверки, что крыша
/// накрыла пятно целиком и ровно один раз.
fn hip_area(roof: &HipRoof) -> f32 {
    let quad = |corners: &[Vec2; 4]| {
        (corners[1] - corners[0])
            .perp_dot(corners[2] - corners[0])
            .abs()
            / 2.0
            + (corners[2] - corners[0])
                .perp_dot(corners[3] - corners[0])
                .abs()
                / 2.0
    };
    roof.slopes
        .iter()
        .map(|(slope, _)| quad(slope))
        .sum::<f32>()
        + signed_ring_area(&roof.ridge.0).abs()
}

#[test]
fn a_hip_roof_covers_the_footprint_exactly_once() {
    let _sun = crate::map::default_sun();
    // плоский режим: скаты и площадка конька лежат в одной плоскости, и их
    // площади обязаны сложиться в площадь пятна — ни дыр, ни нахлёстов
    let plot = oblong(10.0, 20.0);
    let base = Srgba::WHITE;
    let Roofing::Hip(roof) = hip_only(&house(plot.clone()), base) else {
        panic!("a rectangle must accept a hip roof");
    };
    let footprint = signed_ring_area(&plot).abs();
    assert!(
        (hip_area(&roof) - footprint).abs() < 0.5,
        "{} vs {footprint}",
        hip_area(&roof)
    );
    // скатов ровно по ребру контура
    assert_eq!(roof.slopes.len(), plot.len());
}

#[test]
fn an_l_shaped_house_gets_a_cross_gable_instead_of_a_hip_or_a_flat_one() {
    let _sun = crate::map::default_sun();
    // Г-образный дом одной двускатной не накрыть — прямоугольник торчал бы из
    // него, — и в частном секторе он кроется двумя с ендовой, а не вальмой
    let ell = house(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(14.0, 0.0),
        Vec2::new(14.0, 6.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(6.0, 14.0),
        Vec2::new(0.0, 14.0),
    ]);
    for seed in [0, 1 << 5, 7 << 5, 12345] {
        let Roofing::Gable(roof) = roofing(
            &ell,
            &classify(&ell, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            seed,
            None,
        ) else {
            panic!("an L takes a cross gable");
        };
        assert_eq!(RoofShape::of_gable(&roof), RoofShape::Cross);
        // корпус — два ската и два фронтона, крыло — ещё два и один торец
        assert_eq!(roof.slopes.len(), 4);
        assert_eq!(roof.gables.len(), 3);
    }
    // а плоская кровля так и остаётся у того, кому она положена
    let mut block = building(oblong(30.0, 80.0), Some(15.0), AreaKind::Building);
    block.building_use = BuildingUse::Apartments;
    assert!(matches!(
        roofing(
            &block,
            &classify(&block, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            0,
            None
        ),
        Roofing::Flat
    ));
}

#[test]
fn a_private_house_is_gabled_and_almost_never_hipped_or_flat() {
    let _sun = crate::map::default_sun();
    // частный дом 10 × 8: на снимке частного сектора вальмы и плоской кровли
    // у такого почти не бывает
    let cottage = house(oblong(8.0, 10.0));
    let (mut gabled, mut other) = (0, 0);
    for seed in 0..400u32 {
        match roofing(
            &cottage,
            &classify(&cottage, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            seed.wrapping_mul(2654435761),
            None,
        ) {
            Roofing::Gable(roof) => match roof.form {
                GableForm::Gable => gabled += 1,
                _ => other += 1,
            },
            Roofing::Hip(_) => panic!("a small house took a hip"),
            Roofing::Flat | Roofing::Tent(_) => panic!("a house lost its pitched roof"),
        }
    }
    assert!(
        gabled * 2 > gabled + other,
        "{gabled} gables of {}",
        gabled + other
    );
    assert!(other > 0, "no half-hips or gambrels at all");
}

#[test]
fn every_gable_form_covers_its_rectangle_exactly_once() {
    let _sun = crate::map::default_sun();
    // плоский режим: грани крыши лежат в плане и обязаны сложиться в
    // прямоугольник дома — ни дыр, ни нахлёстов
    let cottage = house(oblong(8.0, 12.0));
    for shape in [
        RoofShape::Gable,
        RoofShape::HalfHip,
        RoofShape::Gambrel,
        RoofShape::LeanTo,
    ] {
        let Roofing::Gable(roof) = roofing_of(
            shape,
            &cottage,
            &classify(&cottage, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            0,
            None,
        ) else {
            panic!("{shape:?} did not build");
        };
        assert_eq!(RoofShape::of_gable(&roof), shape);
        let covered: f32 = roof
            .slopes
            .iter()
            .map(|(face, _)| signed_ring_area(face).abs())
            .sum();
        assert!((covered - 96.0).abs() < 0.1, "{shape:?} covers {covered}");
    }
}

#[test]
fn the_cross_gable_wing_meets_the_main_ridge_no_higher_than_it() {
    let _sun = crate::map::default_sun();
    let ell = house(vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(14.0, 0.0),
        Vec2::new(14.0, 6.0),
        Vec2::new(6.0, 6.0),
        Vec2::new(6.0, 14.0),
        Vec2::new(0.0, 14.0),
    ]);
    // подъём уводит точку на тысячу метров вверх за метр высоты: план (до
    // 14 м) в сдвиге теряется, и высота читается прямо из `y`
    let Roofing::Gable(roof) = roofing_of(
        RoofShape::Cross,
        &ell,
        &classify(&ell, None),
        Vec2::ZERO,
        |rise| Vec2::new(0.0, rise * 1000.0),
        Srgba::WHITE,
        0,
        None,
    ) else {
        panic!("an L takes a cross gable");
    };
    let top = |faces: &[(Vec<Vec2>, LinearRgba)]| {
        faces
            .iter()
            .flat_map(|(face, _)| face.iter())
            .map(|point| point.y / 1000.0)
            .fold(f32::MIN, f32::max)
    };
    let (main, wing) = (top(&roof.slopes[..2]), top(&roof.slopes[2..]));
    assert!((main - ridge_rise(6.0)).abs() < 0.02, "main ridge {main}");
    assert!(
        wing <= main + 0.02,
        "wing ridge {wing} over the main {main}"
    );
    assert!(
        wing > 0.5 * main,
        "wing ridge {wing} sank into the main roof"
    );
}

#[test]
fn dormers_stand_on_a_lifted_roof_only() {
    let _sun = crate::map::default_sun();
    let cottage = house(oblong(9.0, 12.0));
    let lean = Lean::of();
    let Roofing::Gable(lifted) = roofing_of(
        RoofShape::Dormer,
        &cottage,
        &classify(&cottage, None),
        Vec2::ZERO,
        |rise| lean.ridge(rise),
        Srgba::WHITE,
        0,
        None,
    ) else {
        panic!("a dormer roof did not build");
    };
    assert!(!lifted.dormers.is_empty());
    assert_eq!(RoofShape::of_gable(&lifted), RoofShape::Dormer);
    // в плоском режиме окно было бы заплатой на скате
    let Roofing::Gable(flat) = roofing_of(
        RoofShape::Dormer,
        &cottage,
        &classify(&cottage, None),
        Vec2::ZERO,
        |_| Vec2::ZERO,
        Srgba::WHITE,
        0,
        None,
    ) else {
        panic!("a dormer roof did not build flat");
    };
    assert!(flat.dormers.is_empty());
}

#[test]
fn a_yard_shed_is_low_and_mostly_lean_to() {
    let _sun = crate::map::default_sun();
    let (mut lean_to, mut total) = (0, 0);
    for index in 0..60 {
        let at = Vec2::new(index as f32 * 23.0, index as f32 * 17.0);
        let ring: Vec<Vec2> = oblong(4.0, 6.0).into_iter().map(|p| p + at).collect();
        let shed = building(ring, None, AreaKind::Building);
        assert!(height_or_default(&shed) <= 3.0, "a shed is one low box");
        if let Roofing::Gable(roof) = roofing(
            &shed,
            &classify(&shed, None),
            Vec2::ZERO,
            |_| Vec2::ZERO,
            Srgba::WHITE,
            building_seed(&shed),
            None,
        ) {
            total += 1;
            lean_to += usize::from(roof.form == GableForm::LeanTo);
        }
    }
    assert_eq!(total, 60);
    assert!(lean_to * 2 > total, "{lean_to} lean-to of {total}");
}

#[test]
fn a_private_house_is_mostly_one_storey() {
    let _sun = crate::map::default_sun();
    let mut one = 0;
    for index in 0..80 {
        let at = Vec2::new(index as f32 * 31.0, index as f32 * 19.0);
        let ring: Vec<Vec2> = oblong(9.0, 11.0).into_iter().map(|p| p + at).collect();
        let cottage = house(ring.clone());
        let untagged = building(ring, None, AreaKind::Building);
        for building in [cottage, untagged] {
            let height = height_or_default(&building);
            assert!(height <= 6.5, "{height} m is not a private house");
            one += usize::from(height < 4.5);
        }
    }
    assert!(one * 10 >= 160 * 6, "only {one} of 160 are one storey");
}

/// Вальма, заказанная явно: посевом она частному дому почти не выпадает.
fn hip_only(building: &PolyArea, base: Srgba) -> Roofing {
    roofing_of(
        RoofShape::Hip,
        building,
        &classify(building, None),
        Vec2::ZERO,
        |_| Vec2::ZERO,
        base,
        0,
        None,
    )
}
