//! Посадка деревьев по разобранной карте: где именно вырастет крона, решает
//! этот модуль, а рисует её `map::trees`. Отделено от парсинга — с тегами
//! Overpass посадка не связана ничем, кроме того, что работает по его выходу.

use std::collections::HashMap;

use bevy::math::{IVec2, Vec2};

use crate::map::osm::model::{
    MapData, PlantedTree, PolyArea, RoadLine, RowTrees, TreeRowLayout, TreeRowPlacement,
    distance_to_segment, point_in_area, ring_area, ring_bounds,
};
use crate::map::roads::casing_width;
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

/// Шаг аллеи на такой плотности, м — **типичное расстояние между деревьями в
/// лесу той же плотности**, а не своя подобранная константа.
///
/// Считается, а не подбирается: лес на плотности `d` держит одно дерево на
/// `TREE_AREA_PER_TREE / d` м², то есть соседи в нём стоят примерно через корень
/// из этой площади. Аллея со своим шагом (было 9 м при `d == 1`) выходила вдвое
/// гуще соседнего парка, и на карте это видно сразу: ряд читается сплошной
/// зелёной кишкой рядом с редким лесом, да ещё и упирается в минимальный зазор
/// там, где лес до него и близко не доходит.
///
/// Посадке нужна обратная функция ([`density_for_row_spacing`]) — эта задаёт
/// смысл пары и проверяется тестами.
#[cfg(test)]
fn row_spacing_at(density: f32) -> f32 {
    (TREE_AREA_PER_TREE / density).sqrt()
}

/// Плотность, на которой шаг аллеи становится равен `spacing` — обратная к
/// [`row_spacing_at`]. Это и есть порог появления слота: слот номер `n` от
/// начала нужен, когда ряд просит шаг `длина / n`.
fn density_for_row_spacing(spacing: f32) -> f32 {
    TREE_AREA_PER_TREE / (spacing * spacing)
}

/// Шаг сдвига при [`TreeRowPlacement::Slide`], м. Он же предел: дерево ищет
/// свободное место не дальше одного шага посадки, иначе ряд сползает в кучу у
/// свободного конца вместо того, чтобы просто поредеть на занятом участке.
const TREE_ROW_SLIDE_STEP: f32 = 1.0;

/// Потолок деревьев на один ряд. Страховка от данных, а не тюнинг: `spacing=0.5`
/// на пятикилометровой набережной — это десять тысяч крон из одного way, и
/// заметить такое на глаз в логе уже поздно.
const TREE_ROW_MAX_TREES: usize = 1500;

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

/// Всё, что мешает дереву встать, вместе с индексами близости по этому всему.
///
/// Отдельной структурой, а не замыканием внутри посадки, потому что индексы
/// строятся по семи тысячам домов и всем дорогам карты, а посадок теперь три:
/// лес и по одной на каждую политику аллей ([`TreeRowPlacement`]). Строить одно
/// и то же трижды — почти всё время посадки.
struct Obstacles<'a> {
    map: &'a MapData,
    /// Расширенные AABB зданий: паддинг по максимальной кроне, на конкретный
    /// радиус проверка ужимается уже в [`Obstacles::solid`].
    building_bounds: Vec<(Vec2, Vec2)>,
    water_bounds: Vec<(Vec2, Vec2)>,
    /// Луг и пляж деревьев не несут, но кроне свисать на них не запрещено.
    bare: Vec<&'a PolyArea>,
    bare_bounds: Vec<(Vec2, Vec2)>,
    building_index: NearbyAreas,
    road_index: NearbyRoadSegments,
    water_index: NearbyAreas,
    bare_index: NearbyAreas,
}

fn in_bbox(pos: Vec2, min: Vec2, max: Vec2) -> bool {
    pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
}

impl<'a> Obstacles<'a> {
    fn build(map: &'a MapData) -> Self {
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
        let bare: Vec<&PolyArea> = map.grass.iter().chain(&map.sand).collect();
        let bare_bounds: Vec<(Vec2, Vec2)> =
            bare.iter().map(|area| ring_bounds(&area.outer)).collect();

        Self {
            building_index: NearbyAreas::build(&building_bounds),
            road_index: NearbyRoadSegments::build(&map.roads),
            water_index: NearbyAreas::build(&water_bounds),
            bare_index: NearbyAreas::build(&bare_bounds),
            map,
            building_bounds,
            water_bounds,
            bare,
            bare_bounds,
        }
    }

    /// Место, куда дерево не должно попасть **никогда**, откуда бы ни взялась
    /// его позиция: внутрь дома (или впритык к его стене) и в воду. Дерево на
    /// асфальте — спорная картинка, дерево из крыши или из пруда — баг рендера.
    fn solid(&self, pos: Vec2, radius: f32) -> bool {
        // до стены — весь вылет кроны: крона, наползающая на дом, читается как
        // растущая из крыши, поэтому зазор меряется от края кроны
        let wall_gap = radius * TREE_CROWN_REACH + TREE_WALL_CLEARANCE;
        self.building_index.near(pos).iter().any(|&index| {
            let (min, max) = self.building_bounds[index];
            let building = &self.map.buildings[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, building) || near_area_edge(pos, building, wall_gap))
        }) || self.water_index.near(pos).iter().any(|&index| {
            let (min, max) = self.water_bounds[index];
            let area = &self.map.water[index];
            in_bbox(pos, min, max)
                && (point_in_area(pos, area) || near_area_edge(pos, area, TREE_SHORE_CLEARANCE))
        })
    }

    /// Полная проверка: [`Obstacles::solid`] плюс дороги (парковые аллеи — тоже
    /// дороги) и голые полигоны — луга и песок. Ею отбирается лес, где позиция
    /// разыгрывается и промах ничего не стоит.
    fn blocked(&self, pos: Vec2, radius: f32) -> bool {
        // до кромки дороги — только ствол с запасом: свисающая над дорожкой
        // ветка выглядит естественно, крона поверх стены — нет
        let kerb_gap = radius + TREE_KERB_CLEARANCE;
        self.solid(pos, radius)
            || self.road_index.near(pos).iter().any(|&(index, from, to)| {
                distance_to_segment(pos, from, to) <= self.map.roads[index].width / 2.0 + kerb_gap
            })
            || self.bare_index.near(pos).iter().any(|&index| {
                let (min, max) = self.bare_bounds[index];
                in_bbox(pos, min, max) && point_in_area(pos, self.bare[index])
            })
    }

    /// Ствол стоит на полотне дороги или на её канте. Мягче дорожной части
    /// [`Obstacles::blocked`]: без зазора кроны — дерево у кромки с нависшей
    /// над полотном кроной легально, дерево, растущее из асфальта, нет.
    fn on_road(&self, pos: Vec2) -> bool {
        self.road_index.near(pos).iter().any(|&(index, from, to)| {
            let width = self.map.roads[index].width;
            distance_to_segment(pos, from, to) <= width / 2.0 + casing_width(width)
        })
    }
}

/// Сетка занятых мест со стороной ячейки [`TREE_MIN_SPACING`]: единственная
/// проверка, растущая с числом посаженных деревьев, и линейным перебором она
/// делала посадку квадратичной (при потолке плотности деревьев уже десятки
/// тысяч). Результат не приблизительный: при такой стороне любое дерево ближе
/// минимума лежит в одной из девяти соседних ячеек.
#[derive(Clone, Default)]
struct Occupied(HashMap<IVec2, Vec<Vec2>>);

impl Occupied {
    fn cell_of(pos: Vec2) -> IVec2 {
        (pos / TREE_MIN_SPACING).floor().as_ivec2()
    }

    fn crowded(&self, pos: Vec2) -> bool {
        let spacing_sq = TREE_MIN_SPACING * TREE_MIN_SPACING;
        let cell = Self::cell_of(pos);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dy| {
                self.0
                    .get(&(cell + IVec2::new(dx, dy)))
                    .is_some_and(|others| {
                        others
                            .iter()
                            .any(|&other| pos.distance_squared(other) < spacing_sq)
                    })
            })
        })
    }

    fn insert(&mut self, pos: Vec2) {
        self.0.entry(Self::cell_of(pos)).or_default().push(pos);
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

/// Лесные деревья: сажаются **только в лесных полигонах** (`natural=wood` /
/// `landuse=forest`) — в OSM именно они, а не парк целиком, несут деревья;
/// открытая часть парка обязана остаться полем. Аллеи вдоль улиц приходят
/// отдельным тегом и сажаются отдельно ([`plant_rows`]). Детерминированный LCG по
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
/// Возвращает деревья (по возрастанию порога появления) и **сколько деревьев
/// было запрошено** по площади лесов — если посажено заметно меньше, потолок
/// плотности стоит выше насыщения (см. [`TREE_MIN_SPACING`] и
/// [`TREE_DENSITY_HEADROOM`]). Занятые места пишутся в переданную сетку, чтобы
/// аллеи не вставали поверх лесных деревьев, а лес — поверх одиночных.
fn plant_woods(
    map: &MapData,
    obstacles: &Obstacles,
    occupied: &mut Occupied,
) -> (Vec<PlantedTree>, usize) {
    // (центр, радиус, плотность появления). Порог считается по номеру дерева
    // внутри своего массива: `n`-е посаженное дерево нужно уже на плотности
    // `n · TREE_AREA_PER_TREE / площадь`. Порядок посадки случайный, так что
    // «первые n» — честная случайная выборка, а доля каждого леса точна и
    // тогда, когда лес упёрся в насыщение и не добрал запрошенного.
    let mut planted_trees: Vec<PlantedTree> = Vec::new();
    // сколько деревьев запрошено по площади лесов: если посажено заметно
    // меньше, лес уперся в насыщение (см. `TREE_DENSITY_HEADROOM`)
    let mut asked = 0usize;
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
            if obstacles.blocked(pos, radius) || occupied.crowded(pos) {
                continue;
            }
            planted += 1;
            planted_trees.push((pos, radius, planted as f32 * TREE_AREA_PER_TREE / area));
            occupied.insert(pos);
        }
    }

    // по возрастанию порога: ползунок отдаёт префикс, а не фильтрует весь
    // массив. Сортировка устойчивая, так что порядок посадки внутри леса
    // сохраняется и набор остаётся детерминированным.
    planted_trees.sort_by(|left, right| left.2.total_cmp(&right.2));
    (planted_trees, asked)
}

/// Одиночные деревья из OSM-нод (`natural=tree`). Ноды в лесу и ближе
/// [`TREE_MIN_SPACING`] к оси аллеи пропускаются — там дерево уже сажает
/// процедурная посадка ([`plant_woods`] / [`plant_rows`]); в домах и в воде
/// ([`Obstacles::solid`]) тоже, как и стволом на полотне или канте дороги
/// ([`Obstacles::on_road`]): наши дороги шире настоящих, и дерево с тротуара
/// регулярно попадает «в асфальт», где крона без подложки повисает на белом.
/// Газоны преградой **не** считаются, а у дороги — в отличие от полной
/// `blocked` — нет зазора кроны: дерево вплотную к кромке легально
/// (ср. [`TreeRowPlacement::Keep`]).
///
/// Порог появления — 0: дерево из данных видно на любой плотности, как и ряд
/// с шагом из OSM. Сажаются раньше леса и аллей и занимают клетки `occupied`,
/// так что процедурная посадка держит от них [`TREE_MIN_SPACING`]; заодно та
/// же сетка схлопывает продублированные ноды.
fn plant_standalone(
    map: &MapData,
    obstacles: &Obstacles,
    occupied: &mut Occupied,
) -> Vec<PlantedTree> {
    let wood_bounds: Vec<(Vec2, Vec2)> = map
        .woods
        .iter()
        .map(|wood| ring_bounds(&wood.outer))
        .collect();
    let row_bounds: Vec<(Vec2, Vec2)> = map
        .tree_rows
        .iter()
        .map(|row| {
            let (min, max) = ring_bounds(&row.points);
            (min - TREE_MIN_SPACING, max + TREE_MIN_SPACING)
        })
        .collect();

    let mut planted: Vec<PlantedTree> = Vec::new();
    for node in &map.tree_nodes {
        let pos = node.pos;
        if pos.x < 0.0 || pos.y < 0.0 || pos.x > MAP_SIZE.x || pos.y > MAP_SIZE.y {
            continue;
        }
        let in_wood = map
            .woods
            .iter()
            .zip(&wood_bounds)
            .any(|(wood, &(min, max))| in_bbox(pos, min, max) && point_in_area(pos, wood));
        if in_wood {
            continue;
        }
        let near_row = map
            .tree_rows
            .iter()
            .zip(&row_bounds)
            .any(|(row, &(min, max))| {
                in_bbox(pos, min, max)
                    && row.points.windows(2).any(|segment| {
                        distance_to_segment(pos, segment[0], segment[1]) <= TREE_MIN_SPACING
                    })
            });
        if near_row {
            continue;
        }
        // радиус из `diameter_crown`, иначе тот же LCG, что в лесу, но с
        // затравкой от координат самой ноды — дерево детерминировано само по
        // себе, а не порядком нод в выгрузке
        let radius = node.radius.unwrap_or_else(|| {
            let mut state: u64 =
                0x9E37_79B9_7F4A_7C15 ^ (pos.x.to_bits() as u64) ^ ((pos.y.to_bits() as u64) << 32);
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let roll = ((state >> 33) as f32) / (u32::MAX >> 1) as f32;
            TREE_MIN_RADIUS + roll * (TREE_MAX_RADIUS - TREE_MIN_RADIUS)
        });
        if obstacles.solid(pos, radius) || obstacles.on_road(pos) || occupied.crowded(pos) {
            continue;
        }
        planted.push((pos, radius, 0.0));
        occupied.insert(pos);
    }
    planted
}

/// Длина полилинии, м.
fn polyline_length(points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum()
}

/// Точка на полилинии в `distance` метрах от её начала. За концом — последняя
/// точка: расставлять деревья дальше ряда нечем.
fn point_at_arc_length(points: &[Vec2], distance: f32) -> Vec2 {
    let mut walked = 0.0;
    for segment in points.windows(2) {
        let (from, to) = (segment[0], segment[1]);
        let length = from.distance(to);
        if length <= 0.0 {
            continue;
        }
        if walked + length >= distance {
            return from.lerp(to, (distance - walked) / length);
        }
        walked += length;
    }
    *points
        .last()
        .expect("вызывается только для points.len() >= 2")
}

/// Ранг каждого слота в порядке «обратных битов» (van der Corput).
///
/// Нужен ровно затем, зачем лесу случайный порядок посадки: порог появления
/// считается по рангу, а ползунок плотности показывает **префикс**. На линии
/// «первые n по порядку» — это её начало, так что натуральный порядок дал бы
/// половину аллеи целиком и половину пустой. При обратных битах любой префикс
/// рассыпан по всей длине ряда, а прореживание остаётся монотонным: шаг
/// ползунка вверх только добавляет деревья, уже стоящие не переезжают.
fn scattered_ranks(count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&index| ((index as u32).reverse_bits(), index));
    let mut ranks = vec![0usize; count];
    for (rank, &index) in order.iter().enumerate() {
        ranks[index] = rank;
    }
    ranks
}

/// Аллеи (`natural=tree_row`): деревья вдоль полилинии из OSM.
///
/// Шаг посадки берётся **из данных**, если они его знают (`TreeRow::spacing`) и
/// это разрешено раскладкой; тогда порог появления у всего ряда равен нулю —
/// плотность задана картой, и ползунок такой ряд не прореживает.
///
/// Иначе ряд засаживается по [`TREE_MIN_SPACING`] (тот же физический пол, что у
/// леса), а порог считается так, чтобы на каждой плотности между **видимыми**
/// деревьями было [`row_spacing_at`] — столько же, сколько между соседями в лесу
/// той же плотности. Из `длина / √(410/d) = ранг + 1` выходит
/// `d = (ранг + 1)² · TREE_AREA_PER_TREE / длина²`; квадрат тут именно потому,
/// что лес — двумерный, а ряд — одномерный, и линейная формула делала аллею
/// вдвое гуще соседнего парка.
///
/// `occupied` приходит из [`plant_standalone`] и [`plant_woods`] и
/// **клонируется** вызывающим на каждую политику: все посадки должны видеть
/// одни и те же одиночные и лесные деревья и не видеть друг друга.
fn plant_rows(
    map: &MapData,
    obstacles: &Obstacles,
    mut occupied: Occupied,
    layout: TreeRowLayout,
) -> Vec<PlantedTree> {
    let mut planted: Vec<PlantedTree> = Vec::new();

    for row in &map.tree_rows {
        if row.points.len() < 2 {
            continue;
        }
        let length = polyline_length(&row.points);
        if length < TREE_MIN_SPACING {
            continue;
        }

        // шаг из данных слушаем только если это разрешено настройкой: без неё
        // ряд живёт по ползунку плотности наравне с лесом
        let data_step = layout.osm_spacing.then_some(row.spacing).flatten();
        let step = data_step.unwrap_or(TREE_MIN_SPACING);
        let count = ((length / step) as usize + 1).clamp(2, TREE_ROW_MAX_TREES);
        let ranks = scattered_ranks(count);

        // тот же LCG, что в лесу, с посевом по началу ряда: радиусы обязаны
        // совпадать между политиками, иначе переключение тумблера перетряхивает
        // весь ряд вместо того, чтобы подвинуть застрявшие деревья
        let first = row.points[0];
        let mut state: u64 =
            0x9E37_79B9_7F4A_7C15 ^ (first.x.to_bits() as u64) ^ ((first.y.to_bits() as u64) << 32);
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (u32::MAX >> 1) as f32
        };

        for (slot, &rank) in ranks.iter().enumerate() {
            let radius = row
                .radius
                .unwrap_or_else(|| TREE_MIN_RADIUS + next() * (TREE_MAX_RADIUS - TREE_MIN_RADIUS));
            let at = length * slot as f32 / (count - 1) as f32;
            let Some(pos) = free_spot(
                &row.points,
                at,
                step,
                radius,
                obstacles,
                &occupied,
                layout.placement,
            ) else {
                continue;
            };
            if pos.x < 0.0 || pos.y < 0.0 || pos.x > MAP_SIZE.x || pos.y > MAP_SIZE.y {
                continue;
            }
            // шаг из данных — плотность задана картой, ползунок её не трогает
            let appears_at = if data_step.is_some() {
                0.0
            } else {
                density_for_row_spacing(length / (rank + 1) as f32)
            };
            planted.push((pos, radius, appears_at));
            occupied.insert(pos);
        }
    }

    planted.sort_by(|left, right| left.2.total_cmp(&right.2));
    planted
}

/// Куда встанет дерево слота, и встанет ли вообще. `Keep` проверяет только то,
/// во что дерево не должно попасть никогда ([`Obstacles::solid`]); `Slide` гонит
/// полную проверку и на занятом месте шагает вперёд по ряду — но не дальше
/// одного шага посадки, иначе ряд сползает в кучу у свободного конца.
fn free_spot(
    points: &[Vec2],
    at: f32,
    step: f32,
    radius: f32,
    obstacles: &Obstacles,
    occupied: &Occupied,
    placement: TreeRowPlacement,
) -> Option<Vec2> {
    let mut offset = 0.0;
    loop {
        let pos = point_at_arc_length(points, at + offset);
        let taken = match placement {
            TreeRowPlacement::Keep => obstacles.solid(pos, radius),
            TreeRowPlacement::Slide => obstacles.blocked(pos, radius),
        };
        if !taken && !occupied.crowded(pos) {
            return Some(pos);
        }
        if placement == TreeRowPlacement::Keep || offset >= step {
            return None;
        }
        offset += TREE_ROW_SLIDE_STEP;
    }
}

/// Все деревья карты: одиночные ноды, лес и аллеи под **каждую** раскладку
/// [`TreeRowLayout`].
///
/// Все раскладки считаются здесь, а не по требованию из UI, потому что ручки в
/// панели переключаются на лету, а построение индексов близости
/// ([`Obstacles::build`]) идёт по семи тысячам домов и всем дорогам карты —
/// платить этим за клик нельзя. Сами аллеи — сотни объектов, четыре прохода по
/// ним стоят копейки.
///
/// Возвращает одиночные деревья, лес, аллеи под все раскладки и сколько лесных
/// деревьев было запрошено по площади (см. [`plant_woods`]). Источники лежат
/// отдельными массивами — панели включают и выключают каждый сам по себе, а
/// сводит их уже `MapData::compose_trees`.
pub(super) fn plant_trees(map: &MapData) -> (Vec<PlantedTree>, Vec<PlantedTree>, RowTrees, usize) {
    let obstacles = Obstacles::build(map);
    let mut occupied = Occupied::default();
    // одиночные — первыми: лес и аллеи держат от них TREE_MIN_SPACING
    let standalone = plant_standalone(map, &obstacles, &mut occupied);
    let (woods, asked) = plant_woods(map, &obstacles, &mut occupied);

    let mut rows = RowTrees::default();
    for layout in TreeRowLayout::ALL {
        // свой клон сетки на каждую раскладку: все они должны видеть один и тот
        // же лес и не видеть друг друга
        rows.set(
            layout,
            plant_rows(map, &obstacles, occupied.clone(), layout),
        );
    }
    (standalone, woods, rows, asked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::model::{AreaKind, RoadClass, TreeCompose, TreeNode, TreeRow};
    use crate::settings::TREE_DENSITY_MIN;

    /// Прямой ряд длиной `length` вдоль оси X, на высоте 1000 — подальше от
    /// края карты и от чего бы то ни было ещё.
    fn straight_row(length: f32, spacing: Option<f32>) -> TreeRow {
        TreeRow {
            points: vec![
                Vec2::new(1000.0, 1000.0),
                Vec2::new(1000.0 + length, 1000.0),
            ],
            spacing,
            radius: None,
        }
    }

    fn map_with_row(row: TreeRow) -> MapData {
        MapData {
            tree_rows: vec![row],
            ..MapData::default()
        }
    }

    /// Посадка рядов при раскладке «шаг из OSM, если он есть» — то, что и
    /// работает по умолчанию.
    fn plant(map: &MapData, placement: TreeRowPlacement) -> Vec<PlantedTree> {
        plant_as(
            map,
            TreeRowLayout {
                placement,
                osm_spacing: true,
            },
        )
    }

    fn plant_as(map: &MapData, layout: TreeRowLayout) -> Vec<PlantedTree> {
        let obstacles = Obstacles::build(map);
        plant_rows(map, &obstacles, Occupied::default(), layout)
    }

    /// Сколько деревьев видно на такой плотности — то же, что делает
    /// `map::trees::visible_count`, но без ползунка.
    fn visible(planted: &[PlantedTree], density: f32) -> Vec<Vec2> {
        planted
            .iter()
            .filter(|&&(.., at)| at <= density)
            .map(|&(pos, ..)| pos)
            .collect()
    }

    #[test]
    fn polyline_length_sums_segments() {
        let points = [Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 4.0)];
        assert!((polyline_length(&points) - 7.0).abs() < 1e-3);
    }

    #[test]
    fn point_at_arc_length_walks_the_corner() {
        let points = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        assert!(point_at_arc_length(&points, 4.0).distance(Vec2::new(4.0, 0.0)) < 1e-3);
        // за угол: 6 м по первому сегменту и ещё 4 по второму
        assert!(point_at_arc_length(&points, 14.0).distance(Vec2::new(10.0, 4.0)) < 1e-3);
        // за концом ряда — последняя точка, а не экстраполяция
        assert!(point_at_arc_length(&points, 99.0).distance(Vec2::new(10.0, 10.0)) < 1e-3);
    }

    /// Смысл обратных битов: префикс рангов рассыпан по всему ряду. Натуральный
    /// порядок дал бы четыре первых слота подряд — то есть четверть аллеи и три
    /// четверти пустоты.
    #[test]
    fn scattered_ranks_spread_the_prefix() {
        let ranks = scattered_ranks(16);
        let mut prefix: Vec<usize> = (0..16).filter(|&slot| ranks[slot] < 4).collect();
        prefix.sort_unstable();
        assert_eq!(prefix.len(), 4);
        // равномерно — значит крайний слот префикса лежит во второй половине
        assert!(
            *prefix.last().unwrap() >= 8,
            "префикс сгружен в начало ряда: {prefix:?}"
        );
        // ранги — перестановка, иначе пороги начнут совпадать или дублироваться
        let mut all = ranks.clone();
        all.sort_unstable();
        assert_eq!(all, (0..16).collect::<Vec<_>>());
    }

    /// Аллея на любой плотности стоит так же редко, как лес той же плотности.
    /// Ради этого порог и квадратичный: с линейным ряд выходил вдвое гуще
    /// соседнего парка, и на карте это било в глаза.
    #[test]
    fn row_is_as_dense_as_the_forest_beside_it() {
        let length = 300.0;
        let planted = plant(
            &map_with_row(straight_row(length, None)),
            TreeRowPlacement::Keep,
        );
        for density in [0.5, 1.0, 3.0, 6.0] {
            let seen = visible(&planted, density);
            let expected = (length / row_spacing_at(density)) as usize;
            assert!(
                seen.len().abs_diff(expected) <= 1,
                "плотность {density}: видно {}, ожидалось ~{expected}",
                seen.len()
            );
        }
    }

    /// Тот же пол минимального зазора, что у леса, и он **не** должен упираться
    /// на рабочих плотностях: ряд, стоящий ровно по минимуму, и читается как
    /// более густой, чем лес.
    #[test]
    fn visible_row_neighbours_keep_the_forest_gap() {
        let planted = plant(
            &map_with_row(straight_row(300.0, None)),
            TreeRowPlacement::Keep,
        );
        for density in [1.0, 3.0] {
            let mut xs: Vec<f32> = visible(&planted, density).iter().map(|p| p.x).collect();
            xs.sort_by(f32::total_cmp);
            let closest = xs
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .fold(f32::INFINITY, f32::min);
            assert!(
                closest > TREE_MIN_SPACING,
                "плотность {density}: соседи через {closest} м, это пол зазора"
            );
        }
    }

    /// Прореживание монотонно: шаг ползунка вверх только добавляет деревья, уже
    /// стоящие не переезжают.
    #[test]
    fn thinning_is_monotone() {
        let planted = plant(
            &map_with_row(straight_row(200.0, None)),
            TreeRowPlacement::Keep,
        );
        let sparse = visible(&planted, 0.5);
        let dense = visible(&planted, 2.0);
        assert!(sparse.len() < dense.len());
        for position in &sparse {
            assert!(
                dense.contains(position),
                "дерево {position:?} исчезло с ростом плотности"
            );
        }
    }

    /// Шаг из данных OSM — ползунок такой ряд не прореживает: пороги нулевые,
    /// а расстояние между стволами равно тегу.
    #[test]
    fn data_spacing_ignores_the_density_slider() {
        let planted = plant(
            &map_with_row(straight_row(100.0, Some(20.0))),
            TreeRowPlacement::Keep,
        );
        assert!(planted.iter().all(|&(.., at)| at == 0.0));
        assert_eq!(planted.len(), 6);
        // на минимуме ползунка ряд стоит целиком
        assert_eq!(visible(&planted, TREE_DENSITY_MIN).len(), planted.len());

        let mut xs: Vec<f32> = planted.iter().map(|&(pos, ..)| pos.x).collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.windows(2) {
            assert!((pair[1] - pair[0] - 20.0).abs() < 1e-2);
        }
    }

    fn building_on_the_row() -> PolyArea {
        PolyArea {
            outer: vec![
                Vec2::new(1040.0, 990.0),
                Vec2::new(1060.0, 990.0),
                Vec2::new(1060.0, 1010.0),
                Vec2::new(1040.0, 1010.0),
            ],
            holes: Vec::new(),
            kind: AreaKind::Building,
            height: None,
            entrances: Vec::new(),
        }
    }

    /// `Keep` доверяет позиции из OSM, но не настолько, чтобы сажать дерево в
    /// доме: крона из крыши читается как баг рендера.
    #[test]
    fn keep_still_drops_trees_inside_buildings() {
        let mut map = map_with_row(straight_row(100.0, None));
        map.buildings.push(building_on_the_row());
        let planted = plant(&map, TreeRowPlacement::Keep);

        assert!(!planted.is_empty());
        for &(pos, ..) in &planted {
            assert!(
                !point_in_area(pos, &map.buildings[0]),
                "дерево {pos:?} стоит внутри дома"
            );
        }
    }

    fn road_across_the_row() -> RoadLine {
        RoadLine {
            points: vec![Vec2::new(1050.0, 900.0), Vec2::new(1050.0, 1100.0)],
            width: 12.0,
            class: RoadClass::Street,
            bridge: false,
            passage: false,
        }
    }

    /// Разница двух политик: `Keep` оставляет дерево на асфальте, `Slide`
    /// уводит его с дороги. Ради этого тумблер и заведён.
    #[test]
    fn slide_moves_trees_off_a_road_where_keep_leaves_them() {
        let mut map = map_with_row(straight_row(100.0, None));
        map.roads.push(road_across_the_row());
        let road = &map.roads[0];

        let on_road = |planted: &[PlantedTree]| {
            planted.iter().any(|&(pos, radius, _)| {
                distance_to_segment(pos, road.points[0], road.points[1])
                    <= road.width / 2.0 + radius + TREE_KERB_CLEARANCE
            })
        };

        assert!(on_road(&plant(&map, TreeRowPlacement::Keep)));
        let slid = plant(&map, TreeRowPlacement::Slide);
        assert!(!on_road(&slid));
        // ряд не должен при этом развалиться: дорога съедает пару слотов, а не всё
        assert!(
            slid.len() >= 10,
            "после сдвига осталось всего {}",
            slid.len()
        );
    }

    /// Обе политики видят один и тот же лес и не видят друг друга — иначе
    /// переключение тумблера меняло бы ряд целиком, а не пару деревьев.
    #[test]
    fn both_policies_start_from_the_same_wood() {
        let map = map_with_row(straight_row(100.0, None));
        let obstacles = Obstacles::build(&map);
        let mut occupied = Occupied::default();
        occupied.insert(Vec2::new(1000.0, 1000.0));

        for layout in TreeRowLayout::ALL {
            let planted = plant_rows(&map, &obstacles, occupied.clone(), layout);
            assert!(
                planted
                    .iter()
                    .all(|&(pos, ..)| pos.distance(Vec2::new(1000.0, 1000.0)) >= TREE_MIN_SPACING),
                "{layout:?} поставил дерево поверх лесного"
            );
        }
    }

    /// Настройка `osm_spacing = false` выключает шаг из тегов: ряд с
    /// `spacing` начинает жить по ползунку плотности наравне с лесом.
    #[test]
    fn disabling_osm_spacing_hands_the_row_back_to_the_slider() {
        let map = map_with_row(straight_row(100.0, Some(20.0)));
        let from_data = plant_as(
            &map,
            TreeRowLayout {
                placement: TreeRowPlacement::Keep,
                osm_spacing: true,
            },
        );
        let from_slider = plant_as(
            &map,
            TreeRowLayout {
                placement: TreeRowPlacement::Keep,
                osm_spacing: false,
            },
        );

        assert!(from_data.iter().all(|&(.., at)| at == 0.0));
        assert!(from_slider.iter().all(|&(.., at)| at > 0.0));
        // теги игнорируются — ряд засажен по своему полу, а не через 20 м
        assert!(from_slider.len() > from_data.len());
        assert_eq!(
            visible(&from_slider, 1.0).len(),
            (100.0 / row_spacing_at(1.0)) as usize
        );
    }

    /// Ряд длиннее карты не должен превращаться в бесконечную посадку.
    #[test]
    fn a_dense_tag_cannot_blow_the_row_up() {
        let planted = plant(
            &map_with_row(straight_row(4000.0, Some(2.0))),
            TreeRowPlacement::Keep,
        );
        assert!(planted.len() <= TREE_ROW_MAX_TREES);
    }

    fn node_at(x: f32, y: f32) -> TreeNode {
        TreeNode {
            pos: Vec2::new(x, y),
            radius: None,
        }
    }

    fn wood_square() -> PolyArea {
        PolyArea {
            outer: vec![
                Vec2::new(1000.0, 1000.0),
                Vec2::new(1100.0, 1000.0),
                Vec2::new(1100.0, 1100.0),
                Vec2::new(1000.0, 1100.0),
            ],
            holes: Vec::new(),
            kind: AreaKind::Wood,
            height: None,
            entrances: Vec::new(),
        }
    }

    fn plant_nodes(map: &MapData) -> Vec<PlantedTree> {
        let obstacles = Obstacles::build(map);
        plant_standalone(map, &obstacles, &mut Occupied::default())
    }

    /// Нода в лесном полигоне пропускается: там дерево уже сажает
    /// [`plant_woods`], и своё сверху дало бы двойную крону.
    #[test]
    fn standalone_skips_nodes_inside_woods() {
        let mut map = MapData {
            woods: vec![wood_square()],
            ..MapData::default()
        };
        map.tree_nodes.push(node_at(1050.0, 1050.0)); // в лесу
        map.tree_nodes.push(node_at(1200.0, 1050.0)); // в чистом поле
        let planted = plant_nodes(&map);
        assert_eq!(planted.len(), 1);
        assert_eq!(planted[0].0, Vec2::new(1200.0, 1050.0));
    }

    /// Нода на аллее пропускается: ряд по этому way уже посажен, а нода ближе
    /// минимального зазора к оси — то же дерево, размеченное дважды.
    #[test]
    fn standalone_skips_nodes_hugging_a_tree_row() {
        let mut map = map_with_row(straight_row(100.0, None));
        map.tree_nodes.push(node_at(1050.0, 1004.0)); // 4 м от оси — это сама аллея
        map.tree_nodes.push(node_at(1050.0, 1020.0)); // 20 м — отдельное дерево
        let planted = plant_nodes(&map);
        assert_eq!(planted.len(), 1);
        assert_eq!(planted[0].0, Vec2::new(1050.0, 1020.0));
    }

    /// В доме и на полотне (или канте) дороги дерево не сажается даже по
    /// данным, а у самой кромки — живёт: зазора кроны у одиночных нет
    /// (та же логика, что у [`TreeRowPlacement::Keep`]).
    #[test]
    fn standalone_avoids_buildings_and_roadbed_but_keeps_kerbside() {
        let mut map = MapData::default();
        map.buildings.push(building_on_the_row());
        map.roads.push(road_across_the_row());
        map.tree_nodes.push(node_at(1050.0, 1000.0)); // внутри дома
        map.tree_nodes.push(node_at(1050.0, 950.0)); // на полотне (ось дороги)
        map.tree_nodes.push(node_at(1058.0, 950.0)); // за кантом, на тротуаре
        let planted = plant_nodes(&map);
        assert_eq!(planted.len(), 1);
        assert_eq!(planted[0].0, Vec2::new(1058.0, 950.0));
    }

    /// Две ноды ближе минимального зазора — одно дерево: в OSM встречаются
    /// продублированные ноды, и обе кроны рисовать незачем.
    #[test]
    fn duplicate_standalone_nodes_collapse() {
        let mut map = MapData::default();
        map.tree_nodes.push(node_at(1000.0, 1000.0));
        map.tree_nodes.push(node_at(1003.0, 1000.0));
        assert_eq!(plant_nodes(&map).len(), 1);
    }

    /// Порог одиночных — 0 (дерево из данных видно на любой плотности), радиус
    /// из `diameter_crown` доезжает как есть, без тега — разыгрывается в
    /// лесной вилке.
    #[test]
    fn standalone_trees_are_always_visible_and_keep_tagged_radius() {
        let mut map = MapData::default();
        map.tree_nodes.push(TreeNode {
            pos: Vec2::new(1000.0, 1000.0),
            radius: Some(5.0),
        });
        map.tree_nodes.push(node_at(1000.0, 1050.0));
        let planted = plant_nodes(&map);
        assert_eq!(planted.len(), 2);
        assert!(planted.iter().all(|&(.., at)| at == 0.0));
        assert_eq!(visible(&planted, TREE_DENSITY_MIN).len(), 2);
        assert_eq!(planted[0].1, 5.0);
        assert!((TREE_MIN_RADIUS..=TREE_MAX_RADIUS).contains(&planted[1].1));
    }

    /// Лес держит минимальный зазор от уже занятых мест — так одиночные
    /// деревья, посаженные первыми, не получают лесного соседа впритык.
    #[test]
    fn woods_keep_clear_of_preoccupied_spots() {
        let map = MapData {
            woods: vec![wood_square()],
            ..MapData::default()
        };
        let obstacles = Obstacles::build(&map);
        let mut occupied = Occupied::default();
        let spot = Vec2::new(1050.0, 1050.0);
        occupied.insert(spot);
        let (woods, _) = plant_woods(&map, &obstacles, &mut occupied);
        assert!(!woods.is_empty());
        assert!(
            woods
                .iter()
                .all(|&(pos, ..)| pos.distance(spot) >= TREE_MIN_SPACING),
            "лесное дерево встало поверх занятого места"
        );
    }

    /// Одиночные деревья идут в общем массиве первыми, и массив остаётся
    /// отсортированным по порогу — на этом стоит `compose_trees`.
    #[test]
    fn standalone_trees_lead_the_planted_array() {
        let mut map = MapData {
            woods: vec![wood_square()],
            ..MapData::default()
        };
        map.tree_nodes.push(node_at(2000.0, 2000.0));
        let (standalone, woods, rows, _) = plant_trees(&map);
        assert_eq!(standalone.len(), 1);
        assert!(
            !woods.is_empty(),
            "лес не посадился рядом с одиночным деревом"
        );
        map.standalone_trees = standalone;
        map.wood_trees = woods;
        map.row_trees = rows;

        map.compose_trees(TreeCompose::default());
        assert_eq!(map.trees[0].0, Vec2::new(2000.0, 2000.0));
        assert_eq!(map.tree_appears_at[0], 0.0);
        assert!(
            map.tree_appears_at
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
    }

    /// Тумблер источника выключает своё слагаемое и не трогает чужие.
    #[test]
    fn compose_toggles_exclude_sources() {
        let mut map = MapData {
            woods: vec![wood_square()],
            ..MapData::default()
        };
        map.tree_nodes.push(node_at(2000.0, 2000.0));
        let (standalone, woods, rows, _) = plant_trees(&map);
        map.standalone_trees = standalone;
        map.wood_trees = woods;
        map.row_trees = rows;

        map.compose_trees(TreeCompose {
            standalone: false,
            ..TreeCompose::default()
        });
        assert_eq!(map.trees.len(), map.wood_trees.len());

        map.compose_trees(TreeCompose {
            woods: false,
            ..TreeCompose::default()
        });
        assert_eq!(map.trees.len(), map.standalone_trees.len());

        map.compose_trees(TreeCompose {
            woods: false,
            standalone: false,
            ..TreeCompose::default()
        });
        assert!(map.trees.is_empty());
    }
}
