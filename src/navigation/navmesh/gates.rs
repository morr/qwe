//! Калитки по умолчанию: ограда, отрезавшая от портала участок с дверями или
//! просто крупный, получает одну калитку ([`Navmesh::open_sealed_fences`]).
//! Шаг между заливкой ([`super::fill`]) и прунингом ([`super::reach`]).

use std::f32::consts::SQRT_2;

use bevy::prelude::*;

use super::Navmesh;
use crate::map::footprint::{FENCE_GATE_WIDTH, FenceIndex, StreetEdges};
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
        // основа до оград уже посчитана заливкой: копия её `passable` на два
        // порядка дешевле повторной растеризации воды, зданий и стен. Ручные
        // сборки (аудиты, тесты), миновавшие `fill_from_mapdata`, считают
        // основу сами — прежний путь
        if let Some(passable) = self.base_scratch.take() {
            base.passable = passable;
        } else {
            base.fill_base(map);
        }
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
        let fences = FenceIndex::build(&map.fences);
        // арки не зависят от оград — их тайлы собираются один раз на все
        // раунды (refill возвращает их на место вместо растеризации)
        let passages = self.passage_tiles(map);

        let mut reachable = vec![false; self.passable.len()];
        let mut reachable_bare = vec![false; self.passable.len()];
        // достижимое без оград — bare-компонента портала: bare.passable ⊇
        // passable, так что ограждённая компонента лежит в ней целиком, а тайл
        // bare-компоненты вне её примыкает к ней через тайл, закрытый только
        // оградой, — тот же набор, который раньше растили из ограждённой
        // сквозь тайлы оград; скан касающихся seed-ов и клон достижимости
        // ушли вместе с зависимостью между обходами. Обходы друг от друга не
        // зависят — идут в двух потоках. Гард повторяет прежнее поведение на
        // непроходимом старте (снап вернул сырую подсказку): оба набора пусты
        let bare_start = if self.passable[start] {
            vec![start]
        } else {
            Vec::new()
        };
        std::thread::scope(|scope| {
            scope.spawn(|| self.flood(&self.passable, &mut reachable, vec![start]));
            scope.spawn(|| self.flood(&bare.passable, &mut reachable_bare, bare_start));
        });
        let mut opened = 0;
        // Тайлы, когда-либо бывшие отрезанным карманом, и раунд, когда тайл
        // последний раз входил в карман (0 — никогда). За раунды `reachable`
        // только растёт, а `reachable_bare` постоянен; `passable` меняется
        // только в боксах проёмов свежих калиток (`gate_gap_boxes`), так что
        // тайл может стать отрезанным, лишь войдя в такой бокс или уже быв
        // карманным. Отсюда раунды 2+ обходят не всю сетку, а накопленный
        // набор с дельтой свежих проёмов: карманы те же, что нашёл бы полный
        // проход.
        let mut cut_tiles: Vec<usize> = Vec::new();
        let mut fresh: Vec<usize> = Vec::new();
        let mut pocket_round = vec![0u8; self.passable.len()];
        for round in 1..=GATE_ROUNDS as u8 {
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
            // карманы: проходимо и достижимо без оград, но не с ними. Раунд 1 —
            // полный проход; дальше — накопленный набор с дельтой. Карманы
            // собираются все, потом обрабатываются в порядке их первого тайла —
            // это порядок, в котором полный проход нашёл бы их сам; от него
            // зависит разнос калиток (`GATE_SPACING`), поэтому порядок обязан
            // совпадать с полным проходом бит в бит.
            let cut =
                |index: usize| self.passable[index] && reachable_bare[index] && !reachable[index];
            let mut pockets: Vec<Vec<usize>> = Vec::new();
            if round == 1 {
                for index in 0..self.passable.len() {
                    if pocket_round[index] == round || !cut(index) {
                        continue;
                    }
                    pockets.push(self.component(index, &cut, &mut pocket_round, round));
                }
            } else {
                for index in cut_tiles.iter().chain(fresh.iter()).copied() {
                    if pocket_round[index] == round || !cut(index) {
                        continue;
                    }
                    pockets.push(self.component(index, &cut, &mut pocket_round, round));
                }
            }
            pockets.sort_unstable_by_key(|pocket| pocket.iter().min().copied());
            for pocket in &pockets {
                // карман идёт в накопление целиком, повторами включительно:
                // `component` метит текущим раундом каждый обойдённый тайл,
                // поэтому фильтр ниже не отсекает ничего — тайл, остающийся
                // отрезанным несколько раундов подряд, входит в список по
                // разу за раунд. Повторы безвредны: сеятель следующего раунда
                // пропускает их проверкой `pocket_round[index] == round` выше
                cut_tiles.extend(
                    pocket
                        .iter()
                        .filter(|&&tile| pocket_round[tile] == round)
                        .copied(),
                );
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

            let mut added: Vec<(usize, Vec2)> = Vec::new();
            for tile in gate_tiles {
                let Some((fence, at)) = fences.nearest(&map.fences, self.index_center(tile), limit)
                else {
                    continue;
                };
                // соседние карманы одного раунда — обычно куски одного двора,
                // разрезанного дверью или изломом, и тайлы для калиток они
                // выбирают рядом: без разноса пять калиток легли на Туле через
                // 2–3 м в один десятиметровый пролом. Лишний карман, который
                // соседняя калитка не открыла, подберёт следующий раунд
                if added
                    .iter()
                    .any(|(_, gate)| gate.distance(at) < GATE_SPACING)
                {
                    continue;
                }
                let known = &mut map.fences[fence].gates;
                if known
                    .iter()
                    .all(|gate| gate.distance(at) > FENCE_GATE_WIDTH / 2.0)
                {
                    known.push(at);
                    added.push((fence, at));
                }
            }
            if added.is_empty() {
                break;
            }
            opened += added.len();
            let added_at: Vec<Vec2> = added.iter().map(|&(_, at)| at).collect();
            // калитки только открывают: сетка раунда — та же, что дал бы
            // полный пересбор (основа + `fill_fences` + `carve_passages`),
            // но пересобирается только то, что пересбор мог изменить — боксы
            // проёмов свежих калиток (`refill_gate_boxes`); за их пределами
            // сетка прошлого раунда уже равна пересбору, вердикты проёмов там
            // не менялись. Открывшиеся тайлы боксов идут следующему раунду
            // карманов свежими кандидатами
            let (boxes, watched) = self.gate_gap_boxes(&added);
            let before: Vec<bool> = watched.iter().map(|&index| self.passable[index]).collect();
            self.refill_gate_boxes(map, &fences, &base.passable, &boxes, &watched, &passages);
            fresh = watched
                .iter()
                .zip(before)
                .filter_map(|(&index, was)| (!was && self.passable[index]).then_some(index))
                .collect();
            self.extend_reachable(&mut reachable, &added_at, FENCE_GATE_WIDTH / 2.0 + limit);
            // отрезанные, которых достижимость догнала, не вернутся никогда —
            // reachable только растёт. Держатся и ещё не открытые: passable
            // между раундами не монотонна (слитый проём съезжает), и тайл,
            // закрытый пересбором обратно, может открыться следующим раундом.
            // Предикат не ссылка на `cut` выше: замыкание, дожившее до этой
            // строки, растянуло бы его immutable borrow через мутабельные
            // вызовы выше
            cut_tiles.retain(|&index| !reachable[index]);
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

#[cfg(test)]
mod tests;
