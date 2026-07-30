//! Посадка деревьев по разобранной карте: где именно вырастет крона, решает
//! этот модуль, а рисует её `map::trees`. Отделено от парсинга — с тегами
//! Overpass посадка не связана ничем, кроме того, что работает по его выходу.

use std::collections::HashMap;

use bevy::math::{IVec2, Vec2};

use crate::map::osm::model::{
    MapData, PolyArea, RoadLine, distance_to_segment, point_in_area, ring_area, ring_bounds,
};
use crate::settings::{MAP_SIZE, TREE_DENSITY_STEP};

/// Плотность деревьев при `TreeStyle::density == 1`: одно дерево на столько м²
/// леса (1600 / 1.3 / 1.5 / 2 — плотность поднималась на 30%, затем ещё на 50%,
/// затем удвоена). Площадь берётся по контуру лесного полигона, а внутри него
/// есть аллеи, газоны и пруды, где сажать нельзя, — поэтому деревьев выходит
/// меньше, чем «площадь / 410», особенно на верхних шагах ползунка.
const TREE_AREA_PER_TREE: f32 = 410.0;

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

/// Предельная доля площади, которую покрывают непересекающиеся диски при
/// случайной посадке с отбрасыванием близких (RSA jamming для дисков, ≈0.547).
/// Гексагональная упаковка даёт 0.907, но её случайной посадкой не получить: к
/// концу свободные места остаются, а попасть в них броском уже нельзя.
const RSA_JAMMING_FRACTION: f32 = 0.547;

/// Насколько близко к насыщению посадка успевает подойти за отведённые попытки
/// (`count * ATTEMPTS_PER_TREE`). Подход асимптотический — у самого насыщения
/// принимается одна точка из сотен, поэтому потолок ползунка берётся с запасом.
/// Проверяется по логу: `trees planted of N asked` должны совпадать.
const TREE_DENSITY_HEADROOM: f32 = 0.8;

/// Попыток на дерево: и страховка от вырожденных/застроенных полигонов, и
/// главный рычаг густоты на верхних шагах ползунка. Ближе к насыщению бросок
/// принимается всё реже, так что бюджет попыток прямо задаёт, сколько из
/// запрошенного лес доберёт (`trees planted of … asked` в логе). 60 против
/// прежних 30 — с индексами близости попытка стоит копейки.
const ATTEMPTS_PER_TREE: usize = 60;

/// По какой плотности засаживается лес — самая густая посадка, какую держит
/// [`TREE_MIN_SPACING`]. Считается, а не подбирается руками: иначе правка
/// минимального зазора молча упирает посадку в потолок, которого не видно.
///
/// Насыщение: диски радиуса `d/2` покрывают `RSA_JAMMING_FRACTION` площади, то
/// есть `0.547 / (π·(d/2)²)` дерева на м². При `d = 6` — одно на ~52 м², против
/// базовых 410 м² это ~7.9x; с запасом на асимптотику — ~6.3x.
const TREE_PLANTING_DENSITY: f32 = {
    let saturation_per_m2 =
        4.0 * RSA_JAMMING_FRACTION / (std::f32::consts::PI * TREE_MIN_SPACING * TREE_MIN_SPACING);
    saturation_per_m2 * TREE_AREA_PER_TREE * TREE_DENSITY_HEADROOM
};

/// Потолок ползунка плотности — та же посадочная плотность, вверх до шага
/// ползунка, чтобы на верхнем шаге лес был виден **весь**.
///
/// Соблазнительно поставить сюда «достижимую» плотность (на Туле сажается 14 616
/// деревьев из 21 155 запрошенных — 69%, остальное не влезает: внутри контура
/// леса аллеи, газоны и пруды). Так нельзя: порог появления считается **по
/// своему лесу**, и лес, который свой запрос добрал, держит деревья до самого
/// верха диапазона. Урезанный потолок просто не показал бы их никогда.
pub const TREE_DENSITY_MAX: f32 = {
    let steps = (TREE_PLANTING_DENSITY / TREE_DENSITY_STEP) as u32 + 1;
    steps as f32 * TREE_DENSITY_STEP
};

/// Сторона ячейки индексов близости, м. Того же порядка, что `FOOTPRINT_CELL`
/// (30) у генератора входов: в ячейке должно лежать несколько кандидатов, а не
/// полквартала и не одна стена.
const NEARBY_CELL: f32 = 32.0;

fn nearby_cell(pos: Vec2) -> IVec2 {
    (pos / NEARBY_CELL).floor().as_ivec2()
}

/// Равномерная сетка «что может накрывать точку»: номера элементов, чей
/// **расширенный** AABB пересекает ячейку. Посадка перебирает десятки тысяч
/// точек, и линейный проход по семи тысячам домов и всем дорогам на каждую
/// попытку — почти всё время посадки. Идиома та же, что в
/// `entrances/index.rs`; сам AABB после выборки всё равно проверяется —
/// ячейка крупнее его.
struct NearbyAreas(HashMap<IVec2, Vec<usize>>);

impl NearbyAreas {
    fn build(bounds: &[(Vec2, Vec2)]) -> Self {
        let mut cells: HashMap<IVec2, Vec<usize>> = HashMap::new();
        for (index, &(min, max)) in bounds.iter().enumerate() {
            let (lo, hi) = (nearby_cell(min), nearby_cell(max));
            for x in lo.x..=hi.x {
                for y in lo.y..=hi.y {
                    cells.entry(IVec2::new(x, y)).or_default().push(index);
                }
            }
        }
        Self(cells)
    }

    fn near(&self, pos: Vec2) -> &[usize] {
        self.0.get(&nearby_cell(pos)).map_or(&[], Vec::as_slice)
    }
}

/// Та же сетка для дорог, но по **отрезкам**, а не по дорогам: AABB реки или
/// проспекта накрывает пол-карты, и индекс по дорогам вернул бы в кандидаты все
/// её сотни отрезков. В ячейку кладётся `(номер дороги, начало, конец)`.
struct NearbyRoadSegments(HashMap<IVec2, Vec<(usize, Vec2, Vec2)>>);

impl NearbyRoadSegments {
    fn build(roads: &[RoadLine]) -> Self {
        let mut cells: HashMap<IVec2, Vec<(usize, Vec2, Vec2)>> = HashMap::new();
        for (index, road) in roads.iter().enumerate() {
            // паддинг — тот же, что у AABB дороги: отрезок, до которого дерево
            // может не дотянуть зазор, обязан лежать в ячейке точки
            let pad = road.width / 2.0 + TREE_MAX_RADIUS + TREE_KERB_CLEARANCE;
            for segment in road.points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                let lo = nearby_cell(from.min(to) - pad);
                let hi = nearby_cell(from.max(to) + pad);
                for x in lo.x..=hi.x {
                    for y in lo.y..=hi.y {
                        cells
                            .entry(IVec2::new(x, y))
                            .or_default()
                            .push((index, from, to));
                    }
                }
            }
        }
        Self(cells)
    }

    fn near(&self, pos: Vec2) -> &[(usize, Vec2, Vec2)] {
        self.0.get(&nearby_cell(pos)).map_or(&[], Vec::as_slice)
    }
}

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
///
/// Сажается сразу **самый густой** лес — по [`TREE_PLANTING_DENSITY`]; ползунок
/// плотности показывает префикс этого набора (`map::trees::visible_count`), а не
/// пересаживает его. Так ползунок отвечает мгновенно и уже стоящие деревья не
/// прыгают с места на место при каждом его шаге.
///
/// Возвращает деревья, плотность появления каждого (`MapData::tree_appears_at`,
/// по возрастанию) и **сколько деревьев было запрошено** по площади лесов: если
/// посажено заметно меньше, потолок плотности стоит выше насыщения (см.
/// [`TREE_MIN_SPACING`] и [`TREE_DENSITY_HEADROOM`]).
pub(super) fn plant_trees(map: &MapData) -> (Vec<(Vec2, f32)>, Vec<f32>, usize) {
    // AABB-прекомпьют: по нему строится индекс близости и им же отсекаются
    // кандидаты из ячейки, чтобы не гонять point-in-polygon зря. Паддинг
    // берётся по максимальной кроне — на конкретный радиус проверка ужимается
    // уже внутри `blocked`
    let building_bounds: Vec<(Vec2, Vec2)> = map
        .buildings
        .iter()
        .map(|building| {
            let (min, max) = ring_bounds(&building.outer);
            let pad = TREE_MAX_RADIUS * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
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

    // индексы по тем же расширенным AABB: без них каждая попытка посадки
    // перебирала все дома и все дороги карты
    let building_index = NearbyAreas::build(&building_bounds);
    let road_index = NearbyRoadSegments::build(&map.roads);
    let water_index = NearbyAreas::build(&water_bounds);
    let bare_index = NearbyAreas::build(&bare_bounds);

    let in_bbox = |pos: Vec2, min: Vec2, max: Vec2| {
        pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
    };
    let blocked = |pos: Vec2, radius: f32| {
        // до стены — весь вылет кроны, до кромки дороги — только ствол с
        // запасом: свисающая над дорожкой ветка выглядит естественно
        let wall_gap = radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
        let kerb_gap = radius + TREE_KERB_CLEARANCE;
        building_index.near(pos).iter().any(|&index| {
            let (min, max) = building_bounds[index];
            let building = &map.buildings[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, building) || near_area_edge(pos, building, wall_gap))
        }) || road_index.near(pos).iter().any(|&(index, from, to)| {
            distance_to_segment(pos, from, to) <= map.roads[index].width / 2.0 + kerb_gap
        }) || water_index.near(pos).iter().any(|&index| {
            let (min, max) = water_bounds[index];
            let area = &map.water[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, area) || near_area_edge(pos, area, TREE_SHORE_CLEARANCE))
        }) || bare_index.near(pos).iter().any(|&index| {
            let (min, max) = bare_bounds[index];
            in_bbox(pos, min, max) && point_in_area(pos, bare[index])
        })
    };

    // (центр, радиус, плотность появления). Порог считается по номеру дерева
    // внутри своего массива: `n`-е посаженное дерево нужно уже на плотности
    // `n · TREE_AREA_PER_TREE / площадь`. Порядок посадки случайный, так что
    // «первые n» — честная случайная выборка, а доля каждого леса точна и
    // тогда, когда лес упёрся в насыщение и не добрал запрошенного.
    let mut planted_trees: Vec<(Vec2, f32, f32)> = Vec::new();
    // сколько деревьев запрошено по площади лесов: если посажено заметно
    // меньше, лес уперся в насыщение (см. `TREE_DENSITY_HEADROOM`)
    let mut asked = 0usize;
    // сетка занятых мест со стороной ячейки `TREE_MIN_SPACING`: проверка
    // разреженности — единственная работа, растущая с числом посаженных
    // деревьев, и линейным перебором она делала посадку квадратичной (при
    // потолке плотности деревьев уже десятки тысяч). Результат не
    // приблизительный: в стороне `TREE_MIN_SPACING` любое дерево ближе
    // минимума лежит в одной из девяти соседних ячеек.
    let mut occupied: HashMap<IVec2, Vec<Vec2>> = HashMap::new();
    let cell_of = |pos: Vec2| (pos / TREE_MIN_SPACING).floor().as_ivec2();
    for wood in &map.woods {
        let (min, max) = ring_bounds(&wood.outer);
        let size = max - min;
        if size.x <= 0.0 || size.y <= 0.0 {
            continue;
        }
        let area = ring_area(&wood.outer);
        let count = ((area * TREE_PLANTING_DENSITY / TREE_AREA_PER_TREE) as usize).max(3);

        let first = wood.outer[0];
        let mut state: u64 =
            0x9E37_79B9_7F4A_7C15 ^ (first.x.to_bits() as u64) ^ ((first.y.to_bits() as u64) << 32);
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (u32::MAX >> 1) as f32
        };

        asked += count;
        let mut planted = 0;
        let mut attempts = count * ATTEMPTS_PER_TREE;
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
            let cell = cell_of(pos);
            let crowded = (-1..=1).any(|dx| {
                (-1..=1).any(|dy| {
                    occupied
                        .get(&(cell + IVec2::new(dx, dy)))
                        .is_some_and(|others| {
                            others
                                .iter()
                                .any(|&other| pos.distance_squared(other) < spacing_sq)
                        })
                })
            });
            if crowded {
                continue;
            }
            planted += 1;
            planted_trees.push((pos, radius, planted as f32 * TREE_AREA_PER_TREE / area));
            occupied.entry(cell).or_default().push(pos);
        }
    }

    // по возрастанию порога: ползунок отдаёт префикс, а не фильтрует весь
    // массив. Сортировка устойчивая, так что порядок посадки внутри леса
    // сохраняется и набор остаётся детерминированным.
    planted_trees.sort_by(|left, right| left.2.total_cmp(&right.2));
    let appears_at = planted_trees.iter().map(|&(.., at)| at).collect();
    let trees = planted_trees
        .iter()
        .map(|&(pos, radius, _)| (pos, radius))
        .collect();
    (trees, appears_at, asked)
}
