//! Калитки по умолчанию: ограда, отрезавшая от портала участок с дверями или
//! просто крупный, получает одну калитку ([`Navmesh::open_sealed_fences`]).
//! Шаг между заливкой ([`super::fill`]) и прунингом ([`super::reach`]).

use std::f32::consts::SQRT_2;

use bevy::prelude::*;

use super::Navmesh;
use super::fill::closest_point_on_polyline;
use crate::map::footprint::{FENCE_GATE_WIDTH, StreetEdges};
use crate::map::osm::model::MapData;

/// Отрезанный оградой карман без дверей получает калитку, только если он не
/// меньше этой площади, м²: щель между забором и глухой стеной дома калитки не
/// стоит. В метрах, а не в тайлах, — чтобы переключатель навтайла не менял,
/// какие дворы открыты.
const SEALED_POCKET_MIN_AREA: f32 = 400.0;

/// Потолок раундов [`Navmesh::open_sealed_fences`]: каждая вложенная ограда
/// стоит раунда, а город, где их больше, — это уже не калитки.
const GATE_ROUNDS: usize = 6;

/// Ближе этого, м, две калитки одного раунда не ставятся — см.
/// [`Navmesh::open_sealed_fences`].
const GATE_SPACING: f32 = 10.0;

impl Navmesh {
    /// Калитки по умолчанию: ограда, которая отрезала от портала участок с
    /// дверями или просто крупный, получает одну калитку — там, где до
    /// достижимой стороны ближе всего. Возвращает число открытых калиток.
    ///
    /// Калитку в OSM размечают редко, а огораживают целиком школы, церкви и
    /// промзоны. Только по проёмам дорог Тула теряла в прунинге 35 тыс. тайлов
    /// (~14 га) и 50 домов оставались без единой достижимой двери — к ним не
    /// ходила ни одна пешка. Щель между забором и стеной дома без дверей
    /// остаётся отрезанной: калитка там ничего бы не дала.
    ///
    /// Калитка записывается **в саму ограду** (`FenceLine::gates`), а не в
    /// сетку: проёмы читает [`fence_gaps`](crate::map::footprint::fence_gaps),
    /// и полигональный меш, который строится из той же `MapData` позже,
    /// получает те же калитки — два заполнения говорят об одном заборе.
    ///
    /// Вызывается после заливки и до прунинга, со снапнутым порталом. Раунды —
    /// ради двойных оград: калитка во внутренней сливает карман с полосой между
    /// заборами, и следующий раунд открывает внешнюю.
    pub fn open_sealed_fences(&mut self, map: &mut MapData, portal: Vec2) -> usize {
        let Some(start) = self.index_of(self.to_tile(portal)) else {
            return 0;
        };
        if map.fences.is_empty() {
            return 0;
        }
        let mut base = self.clone();
        base.fill_base(map);
        let mut bare = base.clone();
        bare.carve_passages(map);
        let pocket_min_tiles =
            (SEALED_POCKET_MIN_AREA / (self.tile_size * self.tile_size)) as usize;
        let doors = self.door_groups(map);
        let mut door_tiles = vec![false; self.passable.len()];
        for &tile in doors.iter().flatten() {
            door_tiles[tile] = true;
        }
        // тайл, закрытый только оградой: без оград он проходим
        let fenced = |grid: &Self, index: usize| !grid.passable[index] && bare.passable[index];
        let limit = self.tile_size * SQRT_2;
        let streets = StreetEdges::build(&map.roads);

        let mut reachable = vec![false; self.passable.len()];
        self.flood(&self.passable, &mut reachable, vec![start]);
        // достижимое без оград — то же достижимое, доросшее сквозь тайлы оград:
        // второй полный обход сетки стоил бы столько же, сколько первый
        let mut reachable_bare = reachable.clone();
        let seeds: Vec<usize> = (0..self.passable.len())
            .filter(|&index| {
                fenced(self, index) && self.neighbours(index).any(|next| reachable[next])
            })
            .collect();
        self.flood(&bare.passable, &mut reachable_bare, seeds);
        let mut opened = 0;
        for _ in 0..GATE_ROUNDS {
            // тайл ограды, в котором открыть калитку: из кандидатов — тот, что
            // уже касается достижимого; при двойной ограде такого нет, и
            // открывается внутренняя, а внешнюю откроет следующий раунд
            // из касающихся — ближайший к кромке проезжей части: вход в
            // огороженную школу делают с улицы, а не с тропинки на задах
            // (дециметры — чтобы ключ был целым и порядок не зависел от
            // сравнения float); равные — по номеру тайла
            let pick = |candidates: &mut dyn Iterator<Item = usize>| {
                candidates.min_by_key(|&tile| {
                    let touches = self.neighbours(tile).any(|next| reachable[next]);
                    let street = (streets.distance(self.index_center(tile)) * 10.0) as u32;
                    (!touches, street, tile)
                })
            };
            let mut gate_tiles: Vec<usize> = Vec::new();
            // карманы: проходимо и достижимо без оград, но не с ними
            let cut =
                |index: usize| self.passable[index] && reachable_bare[index] && !reachable[index];
            let mut seen = vec![false; self.passable.len()];
            for index in 0..self.passable.len() {
                if seen[index] || !cut(index) {
                    continue;
                }
                let pocket = self.component(index, &cut, &mut seen);
                let holds_door = pocket.iter().any(|&tile| door_tiles[tile]);
                if !holds_door && pocket.len() < pocket_min_tiles {
                    continue;
                }
                let mut candidates = pocket
                    .iter()
                    .flat_map(|&tile| self.neighbours(tile))
                    .filter(|&tile| fenced(self, tile));
                gate_tiles.extend(pick(&mut candidates));
            }
            // дверь, которую ограда закрыла вплотную: ни один тайл у двери не
            // проходим, но без оград она была достижима — карманов тут нет,
            // забор лёг прямо на её тайлы
            for group in &doors {
                let reached = group.iter().any(|&tile| reachable[tile]);
                let was_reached = group.iter().any(|&tile| reachable_bare[tile]);
                if reached || !was_reached || group.iter().any(|&tile| self.passable[tile]) {
                    continue;
                }
                let mut candidates = group.iter().copied().filter(|&tile| fenced(self, tile));
                gate_tiles.extend(pick(&mut candidates));
            }

            let mut added: Vec<Vec2> = Vec::new();
            for tile in gate_tiles {
                let Some((fence, at)) = nearest_fence_point(map, self.index_center(tile), limit)
                else {
                    continue;
                };
                // соседние карманы одного раунда — обычно куски одного двора,
                // разрезанного дверью или изломом, и тайлы для калиток они
                // выбирают рядом: без разноса пять калиток легли на Туле через
                // 2–3 м в один десятиметровый пролом. Лишний карман, который
                // соседняя калитка не открыла, подберёт следующий раунд
                if added.iter().any(|gate| gate.distance(at) < GATE_SPACING) {
                    continue;
                }
                let known = &mut map.fences[fence].gates;
                if known
                    .iter()
                    .all(|gate| gate.distance(at) > FENCE_GATE_WIDTH / 2.0)
                {
                    known.push(at);
                    added.push(at);
                }
            }
            if added.is_empty() {
                break;
            }
            opened += added.len();
            // калитки только открывают: заливка оград поверх той же основы
            // даёт сетку, где проходимого стало больше и ничего не закрылось,
            // поэтому достижимость дорастает от калиток, а не считается заново
            *self = base.clone();
            self.fill_fences(map);
            self.carve_passages(map);
            self.extend_reachable(&mut reachable, &added, FENCE_GATE_WIDTH / 2.0 + limit);
        }
        opened
    }

    /// Тайлы у каждой двери: тайл двери и восемь соседей — тот же круг, в
    /// котором цель ищет `find_passable_tile_near`.
    fn door_groups(&self, map: &MapData) -> Vec<Vec<usize>> {
        map.buildings
            .iter()
            .flat_map(|building| &building.entrances)
            .map(|&door| {
                let tile = self.to_tile(door);
                (-1..=1)
                    .flat_map(|dx| (-1..=1).map(move |dy| (dx, dy)))
                    .filter_map(|(dx, dy)| self.index(tile.x + dx, tile.y + dy))
                    .collect()
            })
            .collect()
    }
}

/// Ближайшая к `point` точка ограды и номер ограды — не дальше диагонали
/// навтайла (`limit`): тайл, для которого ищется калитка, лежит на заборе по
/// построению.
fn nearest_fence_point(map: &MapData, point: Vec2, limit: f32) -> Option<(usize, Vec2)> {
    map.fences
        .iter()
        .enumerate()
        .filter(|(_, fence)| fence.points.len() >= 2)
        .map(|(index, fence)| (index, closest_point_on_polyline(point, &fence.points)))
        .filter(|(_, at)| at.distance(point) <= limit)
        .min_by(|a, b| a.1.distance(point).total_cmp(&b.1.distance(point)))
}

#[cfg(test)]
mod tests;
