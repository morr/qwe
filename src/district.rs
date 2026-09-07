//! Районы: грубое разбиение проходимой земли, по которому считаются скверна,
//! перепись людей и расстояние до сердца. Производная от карты, не состояние
//! прогона: строится в потоке загрузки после прунинга, рестарт её не трогает.
//!
//! Район — **связная компонента проходимых навтайлов внутри одной клетки**
//! сетки [`DISTRICT_GRID`], а не сама клетка: клетка, разрезанная рекой, даёт
//! район на каждый берег, клетка с мостом — один, потому что настил проходим и
//! связывает берега. Так «скверна переходит Упу только по мосту» следует из
//! того же navmesh, по которому ходят пешки, а не из отдельного правила.
//! Осколки мельче [`DISTRICT_MIN_AREA`] приписываются соседу с самой длинной
//! общей границей. Модель и её замеры — скилл `city-siege`.

use std::collections::{HashMap, VecDeque};

use bevy::diagnostic::Diagnostics;
use bevy::math::DVec2;
use bevy::prelude::*;

use crate::determinism::{SimPipeline, SimTick};
use crate::diagnostics::{SIM_CENSUS_MS, measure_ms};
use crate::human::Human;
use crate::movement::SimPosition;
use crate::navigation::Navmesh;
use crate::settings::{
    DISTRICT_CENSUS_TICKS, DISTRICT_GRID, DISTRICT_LABEL_METERS, DISTRICT_MIN_AREA, MAP_SIZE,
};
use crate::spatial::SimSet;

/// Номер района — индекс в [`Districts::districts`].
pub type DistrictId = u16;

/// Метка «не в районе» во внутреннем растре по тайлам.
const UNLABELED: u16 = u16::MAX;

#[derive(Debug, Clone, Reflect)]
pub struct District {
    /// Клетка [`DISTRICT_GRID`], в которой компонента родилась. У района,
    /// поглотившего осколок из соседней клетки, — клетка поглотившего.
    pub cell: IVec2,
    /// Проходимых навтайлов в районе.
    pub tiles: u32,
    /// Центр масс тайлов, метры карты.
    pub centroid: Vec2,
    /// Смежные районы: хотя бы одна пара тайлов соседствует через границу
    /// клеток (4-соседство, как у прунинга и A*).
    pub neighbours: Vec<DistrictId>,
    /// Переходов по графу соседства до района сердца; `None` — сердце ни в
    /// каком районе, либо до него не дойти.
    pub dist_to_heart: Option<u16>,
}

/// Районы карты. Пусто до загрузки мира; `poll_job` вставляет собранный
/// ресурс рядом с `PortalPos` и `HeartPos`.
#[derive(Resource, Debug, Default, Reflect)]
#[reflect(Resource)]
pub struct Districts {
    pub districts: Vec<District>,
    /// Район сердца; `None`, когда снапнутое сердце не попало ни в какой.
    pub heart: Option<DistrictId>,
    pub portal: Option<DistrictId>,
    /// Растр меток с шагом [`DISTRICT_LABEL_METERS`]: метка ячейки — район
    /// навтайла в её центре. Метку на каждый навтайл (миллионы) хранить
    /// незачем — компоненты считаются на полном navmesh, а читают их по
    /// позиции. Индексация `x * label_size.y + y`.
    #[reflect(ignore)]
    labels: Vec<Option<DistrictId>>,
    label_size: IVec2,
}

/// Компонента до дедупликации осколков.
struct Component {
    cell: IVec2,
    tiles: u32,
    /// Сумма координат тайлов — центроид после всех слияний.
    sum: DVec2,
}

impl Districts {
    /// Обход всех тайлов пропрунённого navmesh: заливка компонент внутри
    /// клеток, границы между ними, поглощение осколков, BFS от сердца, растр
    /// меток. Один проход по тайлам плюс проход по границам — того же
    /// порядка, что BFS прунинга.
    pub fn build(navmesh: &Navmesh, portal: Vec2, heart: Vec2) -> Self {
        let grid = navmesh.grid_size;
        let index = |x: i32, y: i32| (x * grid.y + y) as usize;
        let cell_of =
            |x: i32, y: i32| IVec2::new(x * DISTRICT_GRID.x / grid.x, y * DISTRICT_GRID.y / grid.y);

        // 1. компоненты внутри клеток
        let mut label = vec![UNLABELED; (grid.x * grid.y) as usize];
        let mut components: Vec<Component> = Vec::new();
        let mut stack = Vec::new();
        for x in 0..grid.x {
            for y in 0..grid.y {
                if !navmesh.is_passable(x, y) || label[index(x, y)] != UNLABELED {
                    continue;
                }
                assert!(
                    components.len() < UNLABELED as usize,
                    "districts: more than {UNLABELED} components before folding"
                );
                let id = components.len() as u16;
                let cell = cell_of(x, y);
                let mut component = Component {
                    cell,
                    tiles: 0,
                    sum: DVec2::ZERO,
                };
                label[index(x, y)] = id;
                stack.push(IVec2::new(x, y));
                while let Some(tile) = stack.pop() {
                    component.tiles += 1;
                    component.sum += tile.as_dvec2();
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (tile.x + dx, tile.y + dy);
                        if navmesh.is_passable(nx, ny)
                            && label[index(nx, ny)] == UNLABELED
                            && cell_of(nx, ny) == cell
                        {
                            label[index(nx, ny)] = id;
                            stack.push(IVec2::new(nx, ny));
                        }
                    }
                }
                components.push(component);
            }
        }

        // 2. границы: длина общей границы (в парах тайлов) между компонентами.
        // Разные метки у соседних тайлов бывают только через границу клеток —
        // внутри клетки соседи залиты одной компонентой
        let mut borders: Vec<HashMap<u16, u32>> = vec![HashMap::new(); components.len()];
        for x in 0..grid.x {
            for y in 0..grid.y {
                let a = label[index(x, y)];
                if a == UNLABELED {
                    continue;
                }
                for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                    if !navmesh.is_passable(nx, ny) {
                        continue;
                    }
                    let b = label[index(nx, ny)];
                    if b != a {
                        *borders[a as usize].entry(b).or_default() += 1;
                        *borders[b as usize].entry(a).or_default() += 1;
                    }
                }
            }
        }

        // 3. осколки: компонента мельче порога уходит к соседу с самой длинной
        // общей границей, от самой мелкой к крупным, до неподвижной точки.
        // Порог задан площадью, а не тайлами, чтобы навтайл 1 м не делал
        // осколком вчетверо меньший двор
        let min_tiles = (DISTRICT_MIN_AREA / (navmesh.tile_size * navmesh.tile_size)).ceil() as u32;
        let mut parent: Vec<u16> = (0..components.len() as u16).collect();
        let smallest_shard =
            |parent: &[u16], components: &[Component], borders: &[HashMap<u16, u32>]| {
                (0..components.len())
                    .filter(|&i| {
                        parent[i] == i as u16
                            && components[i].tiles < min_tiles
                            && !borders[i].is_empty()
                    })
                    .min_by_key(|&i| components[i].tiles)
            };
        while let Some(shard) = smallest_shard(&parent, &components, &borders) {
            // ничья по длине границы решается меньшим номером, а не порядком
            // обхода HashMap: номера районов попадут в отпечаток прогона, и
            // два запуска на одной карте обязаны их совпасть
            let into = borders[shard]
                .iter()
                .max_by_key(|(id, length)| (**length, std::cmp::Reverse(**id)))
                .map(|(id, _)| *id as usize)
                .expect("a shard with a non-empty border map has a neighbour");
            parent[shard] = into as u16;
            let (tiles, sum) = (components[shard].tiles, components[shard].sum);
            components[into].tiles += tiles;
            components[into].sum += sum;
            for (other, length) in std::mem::take(&mut borders[shard]) {
                let other = other as usize;
                if other == into {
                    borders[into].remove(&(shard as u16));
                    continue;
                }
                *borders[into].entry(other as u16).or_default() += length;
                borders[other].remove(&(shard as u16));
                *borders[other].entry(into as u16).or_default() += length;
            }
        }
        // корень поглощённой компоненты: цепочка слияний схлопывается в один
        // переход, потому что осколок всегда сливается с живым корнем
        let root = |mut id: u16| {
            while parent[id as usize] != id {
                id = parent[id as usize];
            }
            id
        };

        // 4. плотные номера районов
        let mut dense = vec![UNLABELED; components.len()];
        let mut districts = Vec::new();
        for (i, component) in components.iter().enumerate() {
            if parent[i] != i as u16 {
                continue;
            }
            dense[i] = districts.len() as u16;
            let mean = component.sum / component.tiles as f64;
            districts.push(District {
                cell: component.cell,
                tiles: component.tiles,
                centroid: (mean.as_vec2() + 0.5) * navmesh.tile_size,
                neighbours: Vec::new(),
                dist_to_heart: None,
            });
        }
        for (i, border) in borders.iter().enumerate() {
            if parent[i] != i as u16 {
                continue;
            }
            let mut neighbours: Vec<DistrictId> =
                border.keys().map(|&id| dense[id as usize]).collect();
            neighbours.sort_unstable();
            neighbours.dedup();
            districts[dense[i] as usize].neighbours = neighbours;
        }

        let district_of_tile = |tile: IVec2| -> Option<DistrictId> {
            if !navmesh.is_passable(tile.x, tile.y) {
                return None;
            }
            let id = label[index(tile.x, tile.y)];
            (id != UNLABELED).then(|| dense[root(id) as usize])
        };

        // 5. расстояние до сердца — BFS по графу соседства
        let heart_district = district_of_tile(navmesh.to_tile(heart));
        let portal_district = district_of_tile(navmesh.to_tile(portal));
        if let Some(start) = heart_district {
            let mut queue = VecDeque::from([start]);
            districts[start as usize].dist_to_heart = Some(0);
            while let Some(id) = queue.pop_front() {
                let next = districts[id as usize].dist_to_heart.unwrap() + 1;
                for k in 0..districts[id as usize].neighbours.len() {
                    let neighbour = districts[id as usize].neighbours[k];
                    if districts[neighbour as usize].dist_to_heart.is_none() {
                        districts[neighbour as usize].dist_to_heart = Some(next);
                        queue.push_back(neighbour);
                    }
                }
            }
        }

        // 6. растр меток по позиции
        let label_size = (MAP_SIZE / DISTRICT_LABEL_METERS).ceil().as_ivec2();
        let mut labels = Vec::with_capacity((label_size.x * label_size.y) as usize);
        for lx in 0..label_size.x {
            for ly in 0..label_size.y {
                let center = (IVec2::new(lx, ly).as_vec2() + 0.5) * DISTRICT_LABEL_METERS;
                labels.push(district_of_tile(navmesh.to_tile(center)));
            }
        }

        Self {
            districts,
            heart: heart_district,
            portal: portal_district,
            labels,
            label_size,
        }
    }

    /// Район в точке карты, O(1) по растру меток; `None` — вода, стена,
    /// недостижимый карман или точка за картой.
    pub fn district_at(&self, position: Vec2) -> Option<DistrictId> {
        self.label((position / DISTRICT_LABEL_METERS).floor().as_ivec2())
    }

    /// Район ближайшей размеченной ячейки растра в пределах `max_meters`.
    /// Растр грубее навтайла: у ячейки в 8 м метка по её центральному тайлу,
    /// и проходимый тайл на тротуаре у стены попадает в ячейку, чей центр —
    /// внутри дома. Для точки, которая заведомо на проходимом тайле, это и
    /// есть её район.
    pub fn district_near(&self, position: Vec2, max_meters: f32) -> Option<DistrictId> {
        let cell = (position / DISTRICT_LABEL_METERS).floor().as_ivec2();
        let radius = (max_meters / DISTRICT_LABEL_METERS).ceil() as i32;
        crate::navigation::nearest_tile_where(cell, radius, |cell| self.label(cell).is_some())
            .and_then(|cell| self.label(cell))
    }

    fn label(&self, cell: IVec2) -> Option<DistrictId> {
        if cell.x < 0 || cell.y < 0 || cell.x >= self.label_size.x || cell.y >= self.label_size.y {
            return None;
        }
        self.labels[(cell.x * self.label_size.y + cell.y) as usize]
    }

    pub fn len(&self) -> usize {
        self.districts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.districts.is_empty()
    }
}

/// Перепись: живых людей в каждом районе, индекс — [`DistrictId`]. Считается
/// раз в [`DISTRICT_CENSUS_TICKS`] проходом по всем людям через
/// [`Districts::district_at`]; читают её скверна и HUD. Не состояние прогона:
/// пересчитывается сама через секунду после любого рестарта.
#[derive(Resource, Debug, Default, Reflect)]
#[reflect(Resource)]
pub struct DistrictCensus {
    pub humans: Vec<u32>,
}

/// Перепись по тику симуляции, а не по своему счётчику: [`SimTick`]
/// сбрасывается на `WorldStarted`, так что фаза переписи в повторе прогона та
/// же, что в первом, — иначе скверна, которая её читает, разошлась бы с
/// отпечатком. 20 000 чтений позиции плюс столько же выборок из растра раз в
/// секунду симуляции; цена — `sim/census_ms`.
fn census_districts(
    tick: Res<SimTick>,
    districts: Res<Districts>,
    mut census: ResMut<DistrictCensus>,
    humans: Query<&SimPosition, With<Human>>,
    mut diagnostics: Diagnostics,
) {
    if !tick.0.is_multiple_of(DISTRICT_CENSUS_TICKS) {
        return;
    }
    let started = std::time::Instant::now();
    census.humans.clear();
    census.humans.resize(districts.len(), 0);
    for position in &humans {
        if let Some(id) = districts.district_at(position.0) {
            census.humans[id as usize] += 1;
        }
    }
    measure_ms(&mut diagnostics, &SIM_CENSUS_MS, started);
}

pub struct DistrictPlugin;

impl Plugin for DistrictPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Districts>()
            .register_type::<DistrictCensus>()
            .init_resource::<Districts>()
            .init_resource::<DistrictCensus>()
            .add_systems(
                FixedUpdate,
                census_districts
                    .in_set(SimSet::SpatialRebuild)
                    .in_set(SimPipeline::BothModes),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{district_city, tiny_city};
    use crate::map::osm::model::MapData;

    fn districts_of(map: &MapData, portal: Vec2, heart: Vec2) -> Districts {
        let mut navmesh = Navmesh::default();
        navmesh.fill_from_mapdata(map);
        navmesh.prune_unreachable(navmesh.to_tile(portal));
        Districts::build(&navmesh, portal, heart)
    }

    /// Цепочка дворов от портала к сердцу через один мост: по району на
    /// двор и на берег, осколок поглощён, вода — ни в каком районе.
    #[test]
    fn district_city_is_a_chain_of_yards_over_one_bridge() {
        let city = district_city();
        let districts = districts_of(&city.map, city.portal, city.heart);

        assert_eq!(districts.len(), 8, "{:?}", districts.districts);
        assert_eq!(districts.portal, districts.district_at(city.portal));
        assert_eq!(districts.heart, districts.district_at(city.heart));
        assert_eq!(districts.district_at(city.water), None);

        // портал в первом дворе, сердце в последнем: семь переходов
        let portal = districts.portal.expect("portal district");
        let heart = districts.heart.expect("heart district");
        assert_eq!(districts.districts[heart as usize].dist_to_heart, Some(0));
        assert_eq!(districts.districts[portal as usize].dist_to_heart, Some(7));

        // осколок — двор мельче порога в соседней клетке — ушёл к первому двору
        assert_eq!(
            districts.district_at(city.shard),
            districts.district_at(city.yards[0])
        );
        // берега разрезаны водой, но смежны через настил моста
        let south = districts.district_at(city.south_bank).expect("south bank");
        let north = districts.district_at(city.north_bank).expect("north bank");
        assert_ne!(south, north);
        assert!(
            districts.districts[south as usize]
                .neighbours
                .contains(&north)
        );
        // каждый двор — свой район, все разные
        let mut ids: Vec<DistrictId> = city
            .yards
            .iter()
            .map(|&yard| districts.district_at(yard).expect("yard district"))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), city.yards.len());
    }

    /// Клетка, разрезанная рекой без моста, — два района, и они не соседи;
    /// клетка с мостом — один район, потому что настил связывает берега.
    #[test]
    fn a_river_splits_a_cell_unless_a_bridge_joins_the_banks() {
        let city = tiny_city();
        let districts = districts_of(&city.map, city.portal, city.portal);

        assert_eq!(districts.district_at(city.river_water), None);

        // клетка моста: берега по обе стороны реки — один район
        let by_bridge_south = districts
            .district_at(Vec2::new(1000.0, 450.0))
            .expect("south of the bridge");
        let by_bridge_north = districts
            .district_at(Vec2::new(1000.0, 700.0))
            .expect("north of the bridge");
        assert_eq!(by_bridge_south, by_bridge_north);

        // клетка без моста: берега — разные районы и не смежны
        let far_south = districts
            .district_at(Vec2::new(2500.0, 450.0))
            .expect("far south bank");
        let far_north = districts
            .district_at(Vec2::new(2500.0, 700.0))
            .expect("far north bank");
        assert_ne!(far_south, far_north);
        assert!(
            !districts.districts[far_south as usize]
                .neighbours
                .contains(&far_north)
        );
    }
}
