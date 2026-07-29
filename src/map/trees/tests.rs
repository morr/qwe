use super::*;

fn ring_area(ring: &[Vec2]) -> f32 {
    ring.windows(2)
        .map(|pair| pair[0].perp_dot(pair[1]))
        .sum::<f32>()
        / 2.0
        + ring[ring.len() - 1].perp_dot(ring[0]) / 2.0
}

#[test]
fn crown_geometry_is_deterministic() {
    for shape in TreeShape::CONCRETE {
        let first = crown_geometry(shape, &mut Lcg::new(42));
        let second = crown_geometry(shape, &mut Lcg::new(42));
        assert_eq!(first.outer, second.outer, "{shape:?}");
        assert_eq!(first.bands.len(), second.bands.len(), "{shape:?}");
    }
}

#[test]
fn mixed_shape_is_one_tenth_conifer() {
    let conifers = (0..10_000)
        .filter(|&index| TreeShape::Mixed.resolve(index) == TreeShape::Conifer)
        .count();
    assert!(
        (900..=1100).contains(&conifers),
        "{conifers} conifers per 10000 trees"
    );
    // остальные — облачные, и конкретные формы разрешаются в самих себя
    assert_eq!(TreeShape::Mixed.resolve(1), TreeShape::Cotton);
    for shape in TreeShape::CONCRETE {
        assert_eq!(shape.resolve(3), shape);
    }
}

/// Хвоя в `Mixed` не должна попадать всегда в одни и те же варианты меша.
#[test]
fn mixed_conifers_spread_over_every_variant() {
    let mut hit = [false; TREE_VARIANTS];
    for index in 0..10_000 {
        if TreeShape::Mixed.resolve(index) == TreeShape::Conifer {
            hit[index % TREE_VARIANTS] = true;
        }
    }
    assert!(hit.iter().all(|&seen| seen), "{hit:?}");
}

#[test]
fn cloud_crown_stays_near_unit_radius() {
    let crown = crown_geometry(TreeShape::Cotton, &mut Lcg::new(7));
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
    assert!(ring_area(&crown.outer) > 0.0);
}

#[test]
fn every_shape_has_its_own_outline_and_bands() {
    let cotton = crown_geometry(TreeShape::Cotton, &mut Lcg::new(5));
    let conifer = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5));
    let palm = crown_geometry(TreeShape::Palm, &mut Lcg::new(5));
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
        assert!(ring_area(&crown.outer) > 0.0, "{shape:?} winding flipped");
    }
}

#[test]
fn bloat_pushes_midpoints_outward() {
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
    let crown = crown_geometry(TreeShape::Cotton, &mut Lcg::new(3));
    let shadow = shadow_ring(&crown.outer);
    let extent = |ring: &[Vec2]| {
        let projected: Vec<f32> = ring.iter().map(|point| point.dot(SHADOW_DIR)).collect();
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
    let mut rng = Lcg::new(11);
    let style = TreeStyle::default();
    for shape in TreeShape::CONCRETE {
        let geometry = crown_geometry(shape, &mut rng);
        let mesh = crown_mesh(&geometry, &style, &mut rng);
        assert!(mesh.count_vertices() > 0, "{shape:?}");
        assert!(shadow_template(&geometry).vertex_count() > 0, "{shape:?}");
    }
}
