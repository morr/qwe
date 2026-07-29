//! Посадка деревьев по разобранной карте: где именно вырастет крона, решает
//! этот модуль, а рисует её `map::trees`. Отделено от парсинга — с тегами
//! Overpass посадка не связана ничем, кроме того, что работает по его выходу.

use bevy::math::Vec2;

use crate::map::osm::model::{
    MapData, PolyArea, distance_to_segment, point_in_area, ring_area, ring_bounds,
};
use crate::settings::MAP_SIZE;

/// Плотность деревьев: одно на столько м² леса (1600 / 1.3 / 1.5 — плотность
/// поднята на 30%, затем ещё на 50% против первоначальной).
const TREE_AREA_PER_TREE: f32 = 820.0;

/// Разброс радиуса кроны, м.
const TREE_MIN_RADIUS: f32 = 2.5;
const TREE_MAX_RADIUS: f32 = 4.0;

/// Во сколько раз контур кроны выступает за номинальный радиус: фестоны
/// `bloat` и шипы хвои уходят за единичную окружность (см. `map::trees`).
pub(super) const TREE_CROWN_REACH: f32 = 1.5;

/// Зазор **кроны** (не ствола) до стены здания, м: крона, наползающая на дом,
/// читается как растущая из крыши, поэтому зазор меряется от края кроны и от
/// рёбер контура, а не только от точки внутри полигона.
pub(super) const TREE_WALL_CLEARANCE: f32 = 1.5;

/// Зазор кроны до кромки дороги, м: заметно меньше стенного — крона, свисающая
/// над тротуаром, выглядит естественно, крона поверх стены — нет.
const TREE_KERB_CLEARANCE: f32 = 0.5;

/// Зазор до берега, м: больше обычного, чтобы крона (радиус до 4) не свисала
/// над прудом — дерево впритык к воде читается как растущее из воды.
pub(super) const TREE_SHORE_CLEARANCE: f32 = 3.0;

/// Минимум между центрами деревьев, м: кроны (радиус до 4) могут чуть
/// касаться, но не сливаться в кляксу.
pub(super) const TREE_MIN_SPACING: f32 = 6.0;

/// Точка ближе `clearance` к любому ребру полигона — внешнему кольцу или
/// кольцу дырки (кольца замкнуты неявно, последнее ребро — от конца к началу).
pub(super) fn near_area_edge(point: Vec2, area: &PolyArea, clearance: f32) -> bool {
    std::iter::once(&area.outer).chain(&area.holes).any(|ring| {
        (0..ring.len()).any(|index| {
            distance_to_segment(point, ring[index], ring[(index + 1) % ring.len()]) <= clearance
        })
    })
}

/// Деревья: сажаются **только в лесных полигонах** (`natural=wood` /
/// `landuse=forest`) — в OSM именно они, а не парк целиком, несут деревья;
/// открытая часть парка обязана остаться полем. Детерминированный LCG по
/// геометрии массива, плотность ∝ площади, rejection-sampling внутри полигона,
/// только в границах карты и не на зданиях/дорогах (парковые аллеи — тоже
/// дороги), не в воде и не на лугах и песке. Зазоры до стен и кромок дорог
/// считаются от края кроны, поэтому радиус разыгрывается до проверок.
pub(super) fn plant_trees(map: &MapData) -> Vec<(Vec2, f32)> {
    // AABB-прекомпьют, чтобы не гонять point-in-polygon по всем 3к зданий;
    // паддинг берётся по максимальной кроне — на конкретный радиус проверка
    // ужимается уже внутри `blocked`
    let building_bounds: Vec<(Vec2, Vec2)> = map
        .buildings
        .iter()
        .map(|building| {
            let (min, max) = ring_bounds(&building.outer);
            let pad = TREE_MAX_RADIUS * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
            (min - pad, max + pad)
        })
        .collect();
    let road_bounds: Vec<(Vec2, Vec2)> = map
        .roads
        .iter()
        .map(|road| {
            let (min, max) = ring_bounds(&road.points);
            let pad = road.width / 2.0 + TREE_MAX_RADIUS + TREE_KERB_CLEARANCE;
            (min - pad, max + pad)
        })
        .collect();

    let water_bounds: Vec<(Vec2, Vec2)> = map
        .water
        .iter()
        .map(|area| {
            let (min, max) = ring_bounds(&area.outer);
            (min - TREE_SHORE_CLEARANCE, max + TREE_SHORE_CLEARANCE)
        })
        .collect();

    // луг и пляж деревьев не несут, но кроне свисать на них не запрещено
    let bare: Vec<&PolyArea> = map.grass.iter().chain(&map.sand).collect();
    let bare_bounds: Vec<(Vec2, Vec2)> = bare.iter().map(|area| ring_bounds(&area.outer)).collect();

    let in_bbox = |pos: Vec2, min: Vec2, max: Vec2| {
        pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
    };
    let blocked = |pos: Vec2, radius: f32| {
        // до стены — весь вылет кроны, до кромки дороги — только ствол с
        // запасом: свисающая над дорожкой ветка выглядит естественно
        let wall_gap = radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
        let kerb_gap = radius + TREE_KERB_CLEARANCE;
        map.buildings
            .iter()
            .zip(&building_bounds)
            .any(|(building, &(min, max))| {
                in_bbox(pos, min, max)
                    && (point_in_area(pos, building) || near_area_edge(pos, building, wall_gap))
            })
            || map
                .roads
                .iter()
                .zip(&road_bounds)
                .any(|(road, &(min, max))| {
                    in_bbox(pos, min, max)
                        && road.points.windows(2).any(|segment| {
                            distance_to_segment(pos, segment[0], segment[1])
                                <= road.width / 2.0 + kerb_gap
                        })
                })
            || map
                .water
                .iter()
                .zip(&water_bounds)
                .any(|(area, &(min, max))| {
                    in_bbox(pos, min, max)
                        && (point_in_area(pos, area)
                            || near_area_edge(pos, area, TREE_SHORE_CLEARANCE))
                })
            || bare
                .iter()
                .zip(&bare_bounds)
                .any(|(area, &(min, max))| in_bbox(pos, min, max) && point_in_area(pos, area))
    };

    let mut trees = Vec::new();
    for wood in &map.woods {
        let area = ring_area(&wood.outer);
        let count = ((area / TREE_AREA_PER_TREE) as usize).max(3);
        let (min, max) = ring_bounds(&wood.outer);
        let size = max - min;
        if size.x <= 0.0 || size.y <= 0.0 {
            continue;
        }

        let first = wood.outer[0];
        let mut state: u64 =
            0x9E37_79B9_7F4A_7C15 ^ (first.x.to_bits() as u64) ^ ((first.y.to_bits() as u64) << 32);
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (u32::MAX >> 1) as f32
        };

        let mut planted = 0;
        // лимит попыток — страховка от вырожденных/застроенных полигонов
        let mut attempts = count * 30;
        while planted < count && attempts > 0 {
            attempts -= 1;
            let pos = min + Vec2::new(next() * size.x, next() * size.y);
            if !point_in_area(pos, wood) {
                continue;
            }
            if pos.x < 0.0 || pos.y < 0.0 || pos.x > MAP_SIZE.x || pos.y > MAP_SIZE.y {
                continue;
            }
            let radius = TREE_MIN_RADIUS + next() * (TREE_MAX_RADIUS - TREE_MIN_RADIUS);
            if blocked(pos, radius) {
                continue;
            }
            let spacing_sq = TREE_MIN_SPACING * TREE_MIN_SPACING;
            if trees
                .iter()
                .any(|&(other, _)| pos.distance_squared(other) < spacing_sq)
            {
                continue;
            }
            trees.push((pos, radius));
            planted += 1;
        }
    }
    trees
}
