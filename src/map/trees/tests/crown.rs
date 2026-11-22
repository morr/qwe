//! Алгоритм кроны (`trees/crown.rs`, watabou): детерминизм контура, bloat,
//! вырезы и шипы ели, штрихи-этажи, листья пальмы, дуги облака, тени и их
//! направление.

use super::super::crown::{
    CONE_BANDS, PALM_BANDS, bloat, chevron_arcs, conifer_shadow, corner_metrics, leaf_arcs,
    shade_dir, shaded_arcs, shadow_ring,
};
use super::super::*;
use super::support::params;
use crate::map::osm::model::signed_ring_area;
use crate::map::seed::Lcg;
use crate::map::shadow_dir;
use crate::settings::TREE_VARIANTS;

#[test]
fn crown_geometry_is_deterministic() {
    let _sun = crate::map::default_sun();
    for shape in TreeShape::CONCRETE {
        let first = crown_geometry(shape, &mut Lcg::new(42), &params());
        let second = crown_geometry(shape, &mut Lcg::new(42), &params());
        assert_eq!(first.outer, second.outer, "{shape:?}");
        assert_eq!(first.bands.len(), second.bands.len(), "{shape:?}");
    }
}

#[test]
fn cloud_crown_stays_near_unit_radius() {
    let _sun = crate::map::default_sun();
    let crown = crown_geometry(TreeShape::Cotton, &mut Lcg::new(7), &params());
    // bloat выдавливает наружу: контур длиннее базового 12-угольника
    assert!(crown.outer.len() > 12 * 4);
    for point in &crown.outer {
        let distance = point.length();
        assert!(
            (0.4..=1.45).contains(&distance),
            "outer point at {distance}"
        );
    }
    // CCW-обход: bloat наружу требует положительной площади
    assert!(signed_ring_area(&crown.outer) > 0.0);
}

#[test]
fn every_shape_has_its_own_outline_and_bands() {
    let _sun = crate::map::default_sun();
    let cotton = crown_geometry(TreeShape::Cotton, &mut Lcg::new(5), &params());
    let conifer = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5), &params());
    let palm = crown_geometry(TreeShape::Palm, &mut Lcg::new(5), &params());
    assert_eq!(conifer.bands.len(), CONE_BANDS.len());
    assert_eq!(palm.bands.len(), PALM_BANDS.len());
    assert_ne!(cotton.outer, conifer.outer);
    assert_ne!(conifer.outer, palm.outer);
    // шипы у хвои торчат заметно дальше базового радиуса, у пальмы тоже
    for (shape, crown) in [(TreeShape::Conifer, &conifer), (TreeShape::Palm, &palm)] {
        let reach = crown
            .outer
            .iter()
            .map(|point| point.length())
            .fold(0.0_f32, f32::max);
        assert!(reach > 1.1, "{shape:?} spikes barely reach {reach}");
        assert!(
            signed_ring_area(&crown.outer) > 0.0,
            "{shape:?} winding flipped"
        );
    }
}

/// Углы контура: `(устье, высота, впадина ли)` по каждой вершине.
fn corners(ring: &[Vec2]) -> Vec<(f32, f32, bool)> {
    (0..ring.len())
        .filter_map(|index| {
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            let next = ring[(index + 1) % ring.len()];
            corner_metrics(previous, ring[index], next)
        })
        .collect()
}

#[test]
fn every_conifer_notch_survives_the_outline_stroke() {
    let _sun = crate::map::default_sun();
    for variant in 0..TREE_VARIANTS {
        let crown = crown_geometry(
            TreeShape::Conifer,
            &mut variant_rng(variant, &params()),
            &params(),
        );
        for (mouth, depth, valley) in corners(&crown.outer) {
            if !valley {
                continue;
            }
            assert!(
                mouth >= params().notch_mouth_min(),
                "variant {variant}: вырез с устьем {mouth} — обводка сомкнётся поперёк"
            );
            assert!(
                depth >= params().notch_depth_min(),
                "variant {variant}: вырез глубиной {depth} — обводка закроет ямку"
            );
        }
    }
}

#[test]
fn opening_notches_keeps_every_spike() {
    let _sun = crate::map::default_sun();
    // вырезы раскрываются полом высоты шипа и сдвигом вершины базы, а не снятием
    // шипа: 16-угольник с шипом на каждом ребре остаётся 32-точечным, вылет
    // держится в прежних границах (снятие шипа увело бы его к 1.9), острия не
    // тупые (снятие шипа с готового контура дотягивало до 107°) и не иглы
    for variant in 0..TREE_VARIANTS {
        let crown = crown_geometry(
            TreeShape::Conifer,
            &mut variant_rng(variant, &params()),
            &params(),
        );
        assert_eq!(crown.outer.len(), 32, "variant {variant}");
        assert!(signed_ring_area(&crown.outer) > 0.0, "variant {variant}");
        let reach = crown
            .outer
            .iter()
            .map(|point| point.length())
            .fold(0.0_f32, f32::max);
        assert!(
            (1.1..1.6).contains(&reach),
            "variant {variant}: шипы дотягивают до {reach}"
        );
        for (mouth, _, _) in corners(&crown.outer) {
            assert!(
                mouth >= params().corner_mouth_floor(),
                "variant {variant}: угол с устьем {mouth} схлопнулся в иглу"
            );
        }
    }
}

#[test]
fn the_cloud_outline_keeps_its_sub_stroke_ripple() {
    let _sun = crate::map::default_sun();
    // облако и пальма идут мимо прохода: их мелкая рябь по замыслу тонет в
    // чернилах, и мерка «вырез шире обводки» к ним неприменима
    for shape in [TreeShape::Cotton, TreeShape::Palm] {
        let crown = crown_geometry(shape, &mut Lcg::new(11), &params());
        assert!(
            corners(&crown.outer)
                .iter()
                .any(|&(mouth, _, valley)| valley && mouth < params().notch_mouth_min()),
            "{shape:?}: рябь контура пропала — проход задел не только хвою"
        );
    }
}

#[test]
fn bloat_pushes_midpoints_outward() {
    let _sun = crate::map::default_sun();
    let square = [
        Vec2::new(1.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
        Vec2::new(-1.0, -1.0),
    ];
    let bloated = bloat(&square, 1.0);
    assert!(bloated.len() > square.len());
    let max_distance = bloated
        .iter()
        .map(|point| point.length())
        .fold(0.0_f32, f32::max);
    assert!(max_distance > 2.0_f32.sqrt());
}

#[test]
fn shadow_ring_stretches_along_shadow_dir() {
    let _sun = crate::map::default_sun();
    let crown = crown_geometry(TreeShape::Cotton, &mut Lcg::new(3), &params());
    let shadow = shadow_ring(&crown.outer, &params());
    let extent = |ring: &[Vec2]| {
        let projected: Vec<f32> = ring.iter().map(|point| point.dot(shadow_dir())).collect();
        let min = projected.iter().copied().fold(f32::INFINITY, f32::min);
        let max = projected.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (min, max)
    };
    let (crown_min, crown_max) = extent(&crown.outer);
    let (shadow_min, shadow_max) = extent(&shadow);
    // тень длиннее кроны вдоль своей оси и выступает на подветренную сторону
    assert!(shadow_max - shadow_min > (crown_max - crown_min) * 1.3);
    assert!(shadow_max > crown_max + 0.5);
}

#[test]
fn crown_mesh_builds_non_empty() {
    let _sun = crate::map::default_sun();
    let mut rng = Lcg::new(11);
    let style = TreeStyle::default();
    for shape in TreeShape::CONCRETE {
        let geometry = crown_geometry(shape, &mut rng, &params());
        let mesh = crown_mesh(&geometry, &style, &mut rng, &params());
        assert!(mesh.count_vertices() > 0, "{shape:?}");
        assert!(
            shadow_template(&geometry, &mut rng, &params()).vertex_count() > 0,
            "{shape:?}"
        );
    }
}

/// Дальняя крона — в восемь с лишним раз легче полной и не больше 48 вершин, у
/// ели все шипы на месте (32 вершины контура — 16 остриёв и 16 впадин), по
/// площади она — силуэт полной кроны с обводкой, а цвет —
/// между листвой и чернилами: чернила закрывают часть кроны, но не всю.
#[test]
fn the_far_crown_is_a_light_average_of_the_full_one() {
    let _sun = crate::map::default_sun();
    let style = TreeStyle::default();
    let foliage = style.foliage.to_linear();
    for shape in TreeShape::CONCRETE {
        for variant in 0..TREE_VARIANTS {
            let built = crown_variant(shape, variant, &style, &params());
            let far = built.far.vertex_count();
            assert!(far <= 48, "{shape:?} #{variant}: {far} вершин");
            assert!(
                far * 8 < built.crown.count_vertices(),
                "{shape:?} #{variant}"
            );
            if shape == TreeShape::Conifer {
                assert_eq!(far, 32, "у ели прорежены шипы");
            }
            let [r, g, b, a] = built.far.colors_for_test()[0];
            assert_eq!(a, 1.0);
            assert!(
                g < foliage.green && g > foliage.green * 0.5,
                "{shape:?} {r} {g} {b}"
            );
            // площадь дальней кроны — площадь силуэта полной: заливка плюс
            // внешняя половина обводки (периметр × полштриха)
            let geometry = crown_geometry(shape, &mut variant_rng(variant, &params()), &params());
            let ring = &geometry.outer;
            let perimeter: f32 = (0..ring.len())
                .map(|i| ring[i].distance(ring[(i + 1) % ring.len()]))
                .sum();
            let silhouette =
                signed_ring_area(ring).abs() + perimeter * params().outline_stroke / 2.0;
            let far_area = built.far.area_since(0);
            assert!(
                (far_area / silhouette - 1.0).abs() < 0.1,
                "{shape:?} #{variant}: дальняя {far_area} против силуэта {silhouette}"
            );
        }
    }
}

/// Кольцо кроны единичного радиуса — как его строит `crown_geometry`.
fn band_centre(ring: &[Vec2]) -> Vec2 {
    ring.iter().copied().sum::<Vec2>() / ring.len() as f32
}

/// Штрихи ели скапливаются на теневой стороне: центр тяжести чернил каждого
/// кольца смещён по `shadow_dir()`, а самая освещённая вершина кольца в дуги не
/// попадает — светлая сторона у `drawShaded2` чистая, без случайных штрихов.
#[test]
fn conifer_shading_leans_into_the_shadow() {
    let _sun = crate::map::default_sun();
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5), &params());
    for (ring, weight) in &geometry.bands {
        let arcs = chevron_arcs(ring, *weight);
        assert!(!arcs.is_empty(), "кольцо без штрихов вовсе");
        let points: Vec<Vec2> = arcs.iter().flatten().copied().collect();
        let ink = points.iter().copied().sum::<Vec2>() / points.len() as f32;
        assert!(
            (ink - band_centre(ring)).dot(shadow_dir()) > 0.0,
            "чернила кольца не на теневой стороне"
        );
        let lit = ring
            .iter()
            .copied()
            .reduce(|a, b| {
                if b.dot(shadow_dir()) < a.dot(shadow_dir()) {
                    b
                } else {
                    a
                }
            })
            .expect("кольцо непусто");
        assert!(
            !points.contains(&lit),
            "штрих дотянулся до самой освещённой вершины"
        );
    }
}

/// Верхушка ели — внутреннее кольцо `0.1`, поднятое к макушке: у него должны
/// быть штрихи, и лежать они обязаны выше центра кроны. Именно её раньше
/// съедали лотерея `drawShaded1` и фильтр коротких дуг.
#[test]
fn conifer_has_a_tip() {
    let _sun = crate::map::default_sun();
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5), &params());
    let (ring, weight) = geometry.bands.last().expect("три кольца у хвои");
    let arcs = chevron_arcs(ring, *weight);
    assert!(!arcs.is_empty(), "у ели нет верхушки");
    for point in arcs.iter().flatten() {
        assert!(point.y > 0.0, "верхушка ниже центра кроны: {point}");
    }
}

/// Каждый «этаж» хвойной кроны — **одна** ломаная, на любом варианте. Джиттер
/// угла иногда качает одну хорду за порог, и без [`close_single_gaps`] кольцо
/// распадалось надвое (на 12 вариантах таких колец было пять).
#[test]
fn every_conifer_band_is_a_single_arc() {
    let _sun = crate::map::default_sun();
    for variant in 0..crate::settings::TREE_VARIANTS as u32 {
        let mut rng = variant_rng(variant as usize, &params());
        let geometry = crown_geometry(TreeShape::Conifer, &mut rng, &params());
        for (number, (ring, weight)) in geometry.bands.iter().enumerate() {
            assert_eq!(
                chevron_arcs(ring, *weight).len(),
                1,
                "вариант {variant}, кольцо {number}: «этаж» разорван"
            );
        }
    }
}

/// Дуга — связный кусок кольца: её точки идут подряд по `ring`, без дырок
/// внутри. Так «этаж» кроны рисуется одной ломаной, а не россыпью штрихов.
#[test]
fn shaded_arcs_run_along_the_ring() {
    let _sun = crate::map::default_sun();
    let mut rng = Lcg::new(5);
    for shape in TreeShape::CONCRETE {
        let geometry = crown_geometry(shape, &mut rng, &params());
        for (ring, weight) in &geometry.bands {
            for arc in shape.shade(ring, *weight, &mut rng, &params()) {
                let start = ring
                    .iter()
                    .position(|point| *point == arc[0])
                    .expect("дуга начинается вершиной кольца");
                for (step, point) in arc.iter().enumerate() {
                    assert_eq!(
                        *point,
                        ring[(start + step) % ring.len()],
                        "{shape:?}: дуга рвётся на шаге {step}"
                    );
                }
            }
        }
    }
}

/// Пальма рисуется листьями целиком (`drawShaded4`): 5 точек на лист, а у
/// склеенных соседних листьев — 9, 13, … Всегда `4·n + 1`.
#[test]
fn palm_arcs_cover_whole_leaves() {
    let _sun = crate::map::default_sun();
    let mut rng = Lcg::new(5);
    let geometry = crown_geometry(TreeShape::Palm, &mut rng, &params());
    for (ring, weight) in &geometry.bands {
        for arc in leaf_arcs(ring, *weight, &mut rng) {
            assert_eq!(arc.len() % 4, 1, "лист нарисован кусками: {}", arc.len());
        }
    }
}

/// Облачная крона: у кольца `0.8` остаётся длинная дуга по теневой стороне
/// **и** россыпь коротких штрихов. Раньше порог в 2.5 толщины штриха съедал
/// больше половины дуг, и кольцо читалось как рваное.
#[test]
fn cotton_keeps_its_dashes() {
    let _sun = crate::map::default_sun();
    let mut rng = Lcg::new(0x051E_D2E5);
    let geometry = crown_geometry(TreeShape::Cotton, &mut rng, &params());
    let (ring, weight) = &geometry.bands[0];
    let arcs = shaded_arcs(ring, *weight, &mut rng, &params());
    let length = |arc: &Vec<Vec2>| {
        arc.windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum::<f32>()
    };
    let longest = arcs.iter().map(length).fold(0.0_f32, f32::max);
    assert!(longest > 1.0, "нет длинной дуги по теневой стороне");
    assert!(
        arcs.len() >= 5,
        "коротких штрихов не осталось: {}",
        arcs.len()
    );
}

/// Тень ели — конус: длиннее кроны вдоль `shadow_dir()` и сужается к дальнему
/// концу, а не растянутая клякса.
#[test]
fn conifer_shadow_tapers_into_a_cone() {
    let _sun = crate::map::default_sun();
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(9), &params());
    let points: Vec<Vec2> = conifer_shadow(&geometry.outer, 0.8)
        .into_iter()
        .flat_map(|(outer, _)| outer)
        .collect();
    let along = |point: &Vec2| point.dot(shadow_dir());
    let reach = points.iter().map(along).fold(f32::MIN, f32::max);
    assert!(reach > 2.0, "веер не дотянулся до 3h: {reach}");
    let width = |range: std::ops::Range<f32>| {
        let across: Vec<f32> = points
            .iter()
            .filter(|point| range.contains(&along(point)))
            .map(|point| point.perp_dot(shadow_dir()))
            .collect();
        across.iter().copied().fold(f32::MIN, f32::max)
            - across.iter().copied().fold(f32::MAX, f32::min)
    };
    assert!(
        width(1.6..2.4) < width(0.0..0.8),
        "дальний конец тени не уже ближнего"
    );
}

/// «Высота» дерева разыгрывается на вариант, поэтому у соседних деревьев тени
/// разной длины — у watabou это `h` из `drawTree`.
#[test]
fn shadow_length_varies_between_variants() {
    let _sun = crate::map::default_sun();
    let reach = |variant: u32| {
        let mut rng = variant_rng(variant as usize, &params());
        let geometry = crown_geometry(TreeShape::Conifer, &mut rng, &params());
        shadow_template(&geometry, &mut rng, &params())
            .positions_for_test()
            .iter()
            .map(|point| Vec2::new(point[0], point[1]).dot(shadow_dir()))
            .fold(f32::MIN, f32::max)
    };
    let reaches: Vec<f32> = (0..crate::settings::TREE_VARIANTS as u32)
        .map(reach)
        .collect();
    let spread = reaches.iter().copied().fold(f32::MIN, f32::max)
        - reaches.iter().copied().fold(f32::MAX, f32::min);
    assert!(spread > 0.5, "тени всех вариантов одной длины: {spread}");
}

/// Штриховка кроны обязана быть в точности перпендикуляром тени: переход с
/// написанного руками `Vec2::new(-y, x)` на `.perp()` ничем, кроме этого, не
/// запинен, а перепутанный знак развернул бы теневую сторону всех крон.
#[test]
fn the_shade_direction_is_the_shadow_turned_left() {
    let _sun = crate::map::default_sun();
    let shadow = shadow_dir();
    assert!(
        shade_dir().distance(Vec2::new(-shadow.y, shadow.x)) < 1e-6,
        "{:?} vs {shadow:?}",
        shade_dir()
    );
}

/// Докуда конус тени ели дотягивается вдоль тени.
fn conifer_reach(outer: &[Vec2], height: f32) -> f32 {
    conifer_shadow(outer, height)
        .iter()
        .flat_map(|(ring, _)| ring.iter())
        .map(|point| point.dot(shadow_dir()))
        .fold(f32::MIN, f32::max)
}

/// Высота солнца тянет тень кроны так же, как тень дома: до этого длина тени
/// дерева бралась только из разыгранной «высоты» варианта, и на 15° дома
/// получали сорокапятиметровые тени, а деревья рядом — прежние короткие.
#[test]
fn a_lower_sun_lengthens_the_tree_shadow() {
    let outer: Vec<Vec2> = (0..8)
        .map(|step| Vec2::from_angle(step as f32 * std::f32::consts::TAU / 8.0))
        .collect();
    let high = {
        let _sun = crate::map::sun_at(300.0, 59.0);
        conifer_reach(&outer, 1.0)
    };
    let low = {
        let _sun = crate::map::sun_at(300.0, 15.0);
        conifer_reach(&outer, 1.0)
    };
    // `cot 15° / cot 59°` = 6.2; крона у ближнего конца конуса даёт запас в обе
    // стороны, поэтому границы широкие
    assert!(low > high * 5.0 && low < high * 7.0, "{high} -> {low}");
}
