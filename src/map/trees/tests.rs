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
        assert!(
            shadow_template(&geometry, &mut rng).vertex_count() > 0,
            "{shape:?}"
        );
    }
}

/// Кольцо кроны единичного радиуса — как его строит `crown_geometry`.
fn band_centre(ring: &[Vec2]) -> Vec2 {
    ring.iter().copied().sum::<Vec2>() / ring.len() as f32
}

/// Штрихи ели скапливаются на теневой стороне: центр тяжести чернил каждого
/// кольца смещён по `SHADOW_DIR`, а самая освещённая вершина кольца в дуги не
/// попадает — светлая сторона у `drawShaded2` чистая, без случайных штрихов.
#[test]
fn conifer_shading_leans_into_the_shadow() {
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5));
    for (ring, weight) in &geometry.bands {
        let arcs = chevron_arcs(ring, *weight);
        assert!(!arcs.is_empty(), "кольцо без штрихов вовсе");
        let points: Vec<Vec2> = arcs.iter().flatten().copied().collect();
        let ink = points.iter().copied().sum::<Vec2>() / points.len() as f32;
        assert!(
            (ink - band_centre(ring)).dot(SHADOW_DIR) > 0.0,
            "чернила кольца не на теневой стороне"
        );
        let lit = ring
            .iter()
            .copied()
            .reduce(|a, b| {
                if b.dot(SHADOW_DIR) < a.dot(SHADOW_DIR) {
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
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(5));
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
    for variant in 0..crate::settings::TREE_VARIANTS as u32 {
        let mut rng = Lcg::new(0x051E_D2E5 + variant * 7919);
        let geometry = crown_geometry(TreeShape::Conifer, &mut rng);
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
    let mut rng = Lcg::new(5);
    for shape in TreeShape::CONCRETE {
        let geometry = crown_geometry(shape, &mut rng);
        for (ring, weight) in &geometry.bands {
            for arc in shape.shade(ring, *weight, &mut rng) {
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
    let mut rng = Lcg::new(5);
    let geometry = crown_geometry(TreeShape::Palm, &mut rng);
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
    let mut rng = Lcg::new(0x051E_D2E5);
    let geometry = crown_geometry(TreeShape::Cotton, &mut rng);
    let (ring, weight) = &geometry.bands[0];
    let arcs = shaded_arcs(ring, *weight, &mut rng);
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

/// Тень ели — конус: длиннее кроны вдоль `SHADOW_DIR` и сужается к дальнему
/// концу, а не растянутая клякса.
#[test]
fn conifer_shadow_tapers_into_a_cone() {
    let geometry = crown_geometry(TreeShape::Conifer, &mut Lcg::new(9));
    let points: Vec<Vec2> = conifer_shadow(&geometry.outer, 0.8)
        .into_iter()
        .flat_map(|(outer, _)| outer)
        .collect();
    let along = |point: &Vec2| point.dot(SHADOW_DIR);
    let reach = points.iter().map(along).fold(f32::MIN, f32::max);
    assert!(reach > 2.0, "веер не дотянулся до 3h: {reach}");
    let width = |range: std::ops::Range<f32>| {
        let across: Vec<f32> = points
            .iter()
            .filter(|point| range.contains(&along(point)))
            .map(|point| point.perp_dot(SHADOW_DIR))
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
    let reach = |variant: u32| {
        let mut rng = Lcg::new(0x051E_D2E5 + variant * 7919);
        let geometry = crown_geometry(TreeShape::Conifer, &mut rng);
        shadow_template(&geometry, &mut rng)
            .positions_for_test()
            .iter()
            .map(|point| Vec2::new(point[0], point[1]).dot(SHADOW_DIR))
            .fold(f32::MIN, f32::max)
    };
    let reaches: Vec<f32> = (0..crate::settings::TREE_VARIANTS as u32)
        .map(reach)
        .collect();
    let spread = reaches.iter().copied().fold(f32::MIN, f32::max)
        - reaches.iter().copied().fold(f32::MAX, f32::min);
    assert!(spread > 0.5, "тени всех вариантов одной длины: {spread}");
}
