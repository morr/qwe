//! Поле хвои: квантильный порог под долю ползунка, массивы вместо рассыпа,
//! примесь, привязанная к позиции ствола, и масштаб волны шума.

use super::super::conifer::CONIFER_NOISE_WAVELENGTH;
use super::super::*;

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

fn field_with(share: f32, mix: f32) -> ConiferField {
    let mut field = ConiferField::default();
    field.resample(&test_forest(), &ConiferNoiseStyle::default(), mix);
    field.set_share(share);
    field
}

/// Поле без примеси — на нём меряется сама кластеризация массивов.
fn field_at(share: f32) -> ConiferField {
    field_with(share, 0.0)
}

/// Ползунок доли — точный: порог квантильный и считается по значениям **с
/// примесью**, так что число хвойных совпадает с запрошенной долей при любом
/// mix, а не «примерно похоже» на неё.
#[test]
fn conifer_share_matches_the_slider() {
    let _sun = crate::map::default_sun();
    let total = GRID_SIDE * GRID_SIDE;
    for mix in [0.0, 0.3, 1.0] {
        for share in [0.05, 0.1, 0.25, 0.5] {
            let field = field_with(share, mix);
            let conifers = (0..total).filter(|&index| field.is_conifer(index)).count();
            let expected = (share * total as f32) as usize;
            assert!(
                conifers.abs_diff(expected) <= 1,
                "share {share}, mix {mix}: {conifers} conifers, expected {expected}"
            );
        }
    }
}

/// Соседи дерева в тестовой сетке — до восьми штук.
fn grid_neighbours(index: usize) -> impl Iterator<Item = usize> {
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
}

/// Статистика массивов: сколько хвойных всего, средняя доля хвойных среди
/// соседей хвойного (кластеризация) и сколько хвойных стоят без единого
/// хвойного соседа (одиночки).
fn stand_stats(field: &ConiferField) -> (usize, f32, usize) {
    let mut conifers = 0;
    let mut same = 0.0;
    let mut lonely = 0;
    for index in 0..GRID_SIDE * GRID_SIDE {
        if !field.is_conifer(index) {
            continue;
        }
        conifers += 1;
        let around: Vec<usize> = grid_neighbours(index).collect();
        let kin = around
            .iter()
            .filter(|&&other| field.is_conifer(other))
            .count();
        same += kin as f32 / around.len() as f32;
        if kin == 0 {
            lonely += 1;
        }
    }
    (conifers, same / conifers as f32, lonely)
}

/// Главное свойство поля: хвоя растёт **массивами**. У хвойного дерева почти
/// все соседи тоже хвойные — при доле 0.1 случайный выбор дал бы 0.1. Меряется
/// без примеси: одиночки при mix > 0 появляются нарочно, см.
/// [`mix_scatters_singles_without_moving_the_share`].
#[test]
fn conifers_grow_in_stands() {
    let _sun = crate::map::default_sun();
    let share = 0.1;
    let field = field_at(share);
    let (conifers, clustering, lonely) = stand_stats(&field);
    eprintln!("clustering {clustering:.3}, lonely {lonely}/{conifers}");
    assert!(
        clustering > 0.8,
        "conifer neighbours share {clustering:.2} at global share {share} — hardly a stand"
    );
    // одинокая ель среди лиственных — ровно то, чего без примеси быть не
    // должно; на отлаженных параметрах поля их ноль, порог оставлен с запасом
    assert!(
        lonely * 200 < conifers,
        "{lonely} of {conifers} conifers stand alone"
    );
}

/// Примесь — ровно те вкрапления, которые без неё запрещены: одиночные ели
/// среди лиственных появляются, массивы при этом выживают, а доля хвои не
/// сдвигается ([`conifer_share_matches_the_slider`] гоняет и mix > 0).
#[test]
fn mix_scatters_singles_without_moving_the_share() {
    let _sun = crate::map::default_sun();
    let (_, clustering, lonely) = stand_stats(&field_with(0.1, 0.2));
    let (_, pure_clustering, _) = stand_stats(&field_at(0.1));
    eprintln!("mixed clustering {clustering:.3}, lonely {lonely}");
    assert!(lonely > 0, "mix 0.2 scattered no singles at all");
    assert!(
        clustering < pure_clustering,
        "mix did not loosen the stands: {clustering:.2} vs pure {pure_clustering:.2}"
    );
    // 0.2 — середина полезного диапазона: вкрапления уже есть, массивы ещё
    // есть; на 0.35 кластеризация падает к 0.4 — соль-перец, что тоже законно,
    // но уже не «массивы с вкраплениями»
    assert!(
        clustering > 0.5,
        "stands dissolved into salt-and-pepper: clustering {clustering:.2}"
    );
}

/// Примесь привязана к **позиции** ствола, не к индексу: тот же лес, обойдённый
/// в обратном порядке, получает те же значения — иначе тумблеры состава,
/// пересобирающие `MapData::trees`, меняли бы породу стоящих деревьев.
#[test]
fn jitter_follows_the_position_not_the_index() {
    let _sun = crate::map::default_sun();
    let forest = test_forest();
    let mut reversed = forest.clone();
    reversed.reverse();
    let style = ConiferNoiseStyle::default();
    let mut field = ConiferField::default();
    field.resample(&forest, &style, 0.35);
    let mut flipped = ConiferField::default();
    flipped.resample(&reversed, &style, 0.35);
    let values = field.values_for_test();
    let flipped_values = flipped.values_for_test();
    assert!(
        (0..values.len()).all(|index| values[index] == flipped_values[values.len() - 1 - index]),
        "values depend on the traversal order, not the position"
    );
}

#[test]
fn conifer_share_edges_are_pure() {
    let _sun = crate::map::default_sun();
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
    let _sun = crate::map::default_sun();
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
    let across_stand = mean_step(CONIFER_NOISE_WAVELENGTH);
    assert!(
        across_stand > between_trees * 2.5,
        "field moves {between_trees:.4} between trees vs {across_stand:.4} across a stand"
    );
    assert!(
        across_stand > 0.05,
        "field barely varies across a stand: {across_stand:.4}"
    );
}
