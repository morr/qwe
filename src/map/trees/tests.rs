use super::*;
use crate::settings::CONIFER_NOISE_FREQUENCY;

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
fn only_mixed_listens_to_the_conifer_field() {
    assert_eq!(TreeShape::Mixed.resolve(true), TreeShape::Conifer);
    assert_eq!(TreeShape::Mixed.resolve(false), TreeShape::Cotton);
    for shape in TreeShape::CONCRETE {
        assert_eq!(shape.resolve(true), shape);
        assert_eq!(shape.resolve(false), shape);
    }
}

/// Тестовый лес: сетка 200×200 с шагом 8 м — примерно как настоящая посадка
/// (`TREE_MIN_SPACING` = 6), но правильная, чтобы соседей можно было считать
/// по индексам. Сторона в 1600 м берёт несколько длин волны поля, иначе на
/// выборку попадала бы одна вершина и статистика массивов ничего не значила.
const GRID_SIDE: usize = 200;
const GRID_STEP: f32 = 8.0;

fn test_forest() -> Vec<(Vec2, f32)> {
    (0..GRID_SIDE * GRID_SIDE)
        .map(|index| {
            let position = Vec2::new(
                (index % GRID_SIDE) as f32 * GRID_STEP,
                (index / GRID_SIDE) as f32 * GRID_STEP,
            );
            (position, 3.0)
        })
        .collect()
}

fn field_at(share: f32) -> ConiferField {
    let mut field = ConiferField::default();
    field.resample(&test_forest());
    field.set_share(share);
    field
}

/// Ползунок доли — точный: порог квантильный, так что число хвойных совпадает
/// с запрошенной долей, а не «примерно похоже» на неё.
#[test]
fn conifer_share_matches_the_slider() {
    let total = GRID_SIDE * GRID_SIDE;
    for share in [0.05, 0.1, 0.25, 0.5] {
        let field = field_at(share);
        let conifers = (0..total).filter(|&index| field.is_conifer(index)).count();
        let expected = (share * total as f32) as usize;
        assert!(
            conifers.abs_diff(expected) <= 1,
            "share {share}: {conifers} conifers, expected {expected}"
        );
    }
}

/// Главное свойство поля: хвоя растёт **массивами**. У хвойного дерева почти
/// все соседи тоже хвойные — при доле 0.1 случайный выбор дал бы 0.1.
#[test]
fn conifers_grow_in_stands() {
    let share = 0.1;
    let field = field_at(share);
    let neighbours = |index: usize| {
        let (x, y) = (index % GRID_SIDE, index / GRID_SIDE);
        [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ]
        .into_iter()
        .filter_map(move |(dx, dy): (isize, isize)| {
            let nx = x.checked_add_signed(dx)?;
            let ny = y.checked_add_signed(dy)?;
            (nx < GRID_SIDE && ny < GRID_SIDE).then_some(ny * GRID_SIDE + nx)
        })
    };

    let mut conifers = 0;
    let mut same = 0.0;
    let mut lonely = 0;
    for index in 0..GRID_SIDE * GRID_SIDE {
        if !field.is_conifer(index) {
            continue;
        }
        conifers += 1;
        let around: Vec<usize> = neighbours(index).collect();
        let kin = around
            .iter()
            .filter(|&&other| field.is_conifer(other))
            .count();
        same += kin as f32 / around.len() as f32;
        if kin == 0 {
            lonely += 1;
        }
    }
    let clustering = same / conifers as f32;
    eprintln!("clustering {clustering:.3}, lonely {lonely}/{conifers}");
    assert!(
        clustering > 0.8,
        "conifer neighbours share {clustering:.2} at global share {share} — hardly a stand"
    );
    // одинокая ель среди лиственных — ровно то, чего быть не должно; на
    // отлаженных параметрах поля их ноль, порог оставлен с запасом
    assert!(
        lonely * 200 < conifers,
        "{lonely} of {conifers} conifers stand alone"
    );
}

#[test]
fn conifer_share_edges_are_pure() {
    let total = GRID_SIDE * GRID_SIDE;
    let none = field_at(0.0);
    let all = field_at(1.0);
    assert!((0..total).all(|index| !none.is_conifer(index)));
    assert!((0..total).all(|index| all.is_conifer(index)));
}

/// Масштаб поля — массив, а не крона: между соседними деревьями значение
/// сдвигается на порядок меньше, чем на длине массива. Сравнение относительное:
/// абсолютный порог зависел бы от параметров шума, а важна именно разница
/// масштабов.
#[test]
fn conifer_field_varies_on_the_scale_of_a_stand() {
    let field = ConiferField::default();
    let mean_step = |step: f32| {
        let samples = 400;
        (0..samples)
            .map(|index| {
                let base = Vec2::splat(index as f32 * 37.0);
                (field.sample(base + Vec2::new(step, 0.0)) - field.sample(base)).abs()
            })
            .sum::<f32>()
            / samples as f32
    };
    let between_trees = mean_step(GRID_STEP);
    let across_stand = mean_step(1.0 / CONIFER_NOISE_FREQUENCY as f32);
    assert!(
        across_stand > between_trees * 2.5,
        "field moves {between_trees:.4} between trees vs {across_stand:.4} across a stand"
    );
    assert!(
        across_stand > 0.05,
        "field barely varies across a stand: {across_stand:.4}"
    );
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
