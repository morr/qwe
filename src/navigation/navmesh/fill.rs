//! Заливка сетки по карте: что из `MapData` блокирует, что прорезает обратно
//! и в каком порядке ([`Navmesh::fill_from_mapdata`]).
//!
//! Растеризацией сюда занимается [`super::raster`], достижимостью —
//! [`super::reach`]; здесь только правила предметной области.

use std::f32::consts::SQRT_2;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::Navmesh;
use crate::grid::{grid_size, navtile_size};
use crate::map::footprint::{
    FENCE_BAND_WIDTH, FENCE_GATE_WIDTH, FenceGapCache, FenceIndex, JOIN_EPSILON,
    distance_to_polyline,
};
use crate::map::grid::Grid;
use crate::map::osm::model::{MapData, closest_on_segment, distance_to_segment, water_line_caps};

impl Navmesh {
    /// Заполнение из OSM-карты. Порядок важен: мосты прорезают проходимые
    /// коридоры поверх воды (иначе Упа разрезает карту надвое), здания, стены и
    /// ограды (с проёмами, [`Self::fill_fences`]) блокируют уже после, а арки
    /// прорезаются последними — их смысл именно в том, чтобы пробить только что
    /// заблокированный дом.
    ///
    /// Бордюры мостов ([`RoadLine::curb_bands`]) непроходимы: с моста не сходят
    /// вбок через перила. Поверх воды это ничего не меняет (вода уже
    /// заблокирована), а на сухопутных пролётах — подходах и эстакадах —
    /// именно бордюр и мешает срезать путь через край настила. Торцы моста
    /// бордюр не перекрывает: блокируются только две продольные кромки.
    ///
    /// Линейные водотоки блокируют вместе с площадной водой и по той же
    /// причине — русло переходят по мосту, а не вброд. Опасность у них своя:
    /// ручей идёт через весь город непрерывной ниткой, и без переходов
    /// `prune_unreachable` ампутировал бы отрезанный берег (ровно поэтому
    /// рельсы в заливку не попадают вовсе). Держат карту связной две вещи:
    /// прорезка мостов **после** этой заливки и трубы, которые не блокируют
    /// (`WaterLine::tunnel`) — под дорогой ручей чаще убран в культверт, чем
    /// перекрыт мостом.
    pub fn fill_from_mapdata(&mut self, map: &MapData) {
        // Дорожная половина кеша проёмов (индекс отрезков всех дорог города и
        // узлы под висячие торцы) — самые дорогие десятки миллисекунд заливки —
        // строится рядом с `fill_base`: оба шага читают только карту, кеш —
        // чистая функция карты, и его порядок от гонки не зависит. На пустых
        // оградах кеш никому не нужен — и не строится.
        let cache = std::thread::scope(|scope| {
            let building = (!map.fences.is_empty())
                .then(|| scope.spawn(|| FenceGapCache::build(&map.fences, &map.roads)));
            self.fill_base(map);
            building.map(|building| building.join().expect("поток кеша проёмов упал"))
        });
        self.fence_cache = cache;
        // основа до оград едет калиткам (`open_sealed_fences` кладёт ограды
        // на неё в каждом раунде): копия passable на этом месте на два порядка
        // дешевле её повторной растеризации
        self.base_scratch = Some(self.passable.clone());
        self.fill_fences(map);
        self.carve_passages(map);
    }

    /// Заливка до оград: вода, водотоки, бордюры и настилы мостов, здания,
    /// стены. Ограды и арки идут поверх неё, и [`Self::open_sealed_fences`]
    /// перекладывает их на эту же основу, не заливая город заново.
    pub(super) fn fill_base(&mut self, map: &MapData) {
        // новая карта — чужие и дорожный индекс проёмов, и скретч основы
        self.fence_cache = None;
        self.base_scratch = None;
        // сетка переживает смену города: без сброса на новой карте остались
        // бы дома и прунинг старой. Здесь же подхватывается текущий размер
        // навтайла — дефолтная аллокация при `init_resource` сделана до
        // восстановления настроек и права быть не обязана
        self.tile_size = navtile_size();
        self.grid_size = grid_size();
        let len = (self.grid_size.x * self.grid_size.y) as usize;
        if self.passable.len() != len {
            self.passable = vec![true; len];
        } else {
            self.passable.fill(true);
        }
        for area in &map.water {
            self.set_area(area, false);
        }
        for line in &map.water_lines {
            // трубы полосы не имеют — над кульвертом земля
            let Some(band) = line.channel_band() else {
                continue;
            };
            // торец у входа в трубу срезан, а не скруглён: за порталом вода
            // уже под землёй, и капсульный полукруг глушил бы вход в культверт
            // на полуширину русла (`water_line_caps`, то же правило у отрисовки)
            let caps = water_line_caps(line, &map.water_lines);
            self.set_polyline_capped(&band.line, band.width, false, caps);
        }
        // бордюры мостов. Тайл бордюра блокируется не безусловно: OSM режет
        // один физический мост на несколько ways (проезжая часть и тротуар —
        // параллельные ленты, эстакада — цепочка кусков), и бордюр одного way
        // не должен перегораживать ни настил соседнего, ни примыкающую
        // дорогу. Поэтому сначала по каждому бордюрному тайлу собираются
        // владельцы (чей бордюр) и примыкания, а блокировка решается ниже
        // щупом «что снаружи»
        let coverage = crate::map::footprint::CurbCoverage::build(&map.roads);
        let bridge_ways: Vec<(&[Vec2], f32)> = coverage
            .bridges()
            .iter()
            .map(|road| (road.points.as_slice(), road.curb_reach()))
            .collect();
        let bridge_bands = BridgeBands::build(&bridge_ways);
        let mut curb_tiles: HashMap<usize, CurbTile> = HashMap::new();
        for (index, road) in coverage.bridges().iter().enumerate() {
            let id = index as u32 + 1;
            for band in road.curb_bands() {
                self.visit_polyline(&band.line, band.width, &mut |grid, x, y| {
                    if let Some(index) = grid.index(x, y) {
                        let owners = &mut curb_tiles.entry(index).or_default().owners;
                        if !owners.contains(&id) {
                            owners.push(id);
                        }
                    }
                });
            }
        }
        // покрытие примыкающей дорогой: её панель входит на мост, бордюр под
        // ней не блокируется — с запасом в диагональ тайла на блуждание
        // бордюрной цепочки. Прямоугольник без капсульных продлений за торцы:
        // подходы моста коллинеарны ему, и капсульный торец, выступающий за
        // общий узел на полширины, слизывал бы бордюр вдоль короткого моста с
        // обоих концов; проём на настоящем примыкании пробивает тело
        // пересекающей дороги. Примыкание — это общий узел (или конец на
        // осевой моста): береговая тропа, прошедшая в паре метров ПОД
        // пролётом, — не примыкание, открытый ею бордюр был бы сходом с
        // моста в реку
        for road in coverage.joining() {
            let width = road.width + self.tile_size * SQRT_2;
            self.visit_polyline_rect(&road.points, width, &mut |grid, x, y| {
                if let Some(index) = grid.index(x, y)
                    && let Some(tile) = curb_tiles.get_mut(&index)
                {
                    tile.road = true;
                }
            });
        }
        // блокировка — щупом «что снаружи»: тайл держит бордюр, если на шаг
        // наружу от осевой владельца НЕ лежит лента другого bridge-way. Так
        // пара «мост + тротуар» запирается по внешнему краю ленты, которая
        // оказалась крайней, — даже когда номинальная ширина проезжей части
        // (primary 16 м) заглатывает свой тротуар целиком и правило «чужая
        // лента накрыла — открыто» оставило бы пару вовсе без барьера.
        // Внутренние швы при этом открыты: щуп из шва попадает в соседнюю
        // ленту. Блокировка ничего не открывает сама по себе: тайл, который
        // щуп оставил открытым, может всё ещё лежать в воде
        // Ленты берутся из [`BridgeBands`], а не перебором всех мостов:
        // бордюрных тайлов тем больше, чем больше мостов, и перебор был
        // квадратичным по ним
        for (&index, tile) in &curb_tiles {
            if tile.road {
                continue;
            }
            // центр тайла по плоскому индексу и в масштабе снапшота
            // (`Navmesh::index_center`) — не `crate::grid::tile_center`,
            // читающий процессный атомик
            let center = self.index_center(index);
            let holds = tile.owners.iter().any(|&id| {
                let owner = id as usize - 1;
                let closest = closest_point_on_polyline(center, bridge_ways[owner].0);
                let outward = (center - closest).normalize_or(Vec2::X);
                let probe = center + outward * self.tile_size;
                !bridge_bands.covered_by_other(probe, owner)
            });
            if holds {
                self.passable[index] = false;
            }
        }
        for road in map.roads.iter().filter(|road| road.bridge) {
            // прорезка не доходит до осевых бордюров (`width + curb`) на
            // полудиагональ тайла. Тайлы цепочки бордюра метятся по «осевая
            // бордюра проходит через тайл», и на косом мосту центр такого тайла
            // отклоняется от неё до полудиагонали (√2 м) — то есть залезает
            // внутрь настила. Прорезка до самой осевой открывала такие тайлы
            // обратно, и барьер превращался в пунктир.
            // Урезание ровно на этот заход оставляет цепочку бордюра целой при
            // любом угле, а связность настила держит его собственная цепочка по
            // осевой — так же, как у тонких рек в set_polyline.
            let band = road.deck_band();
            let deck = (band.width + road.curb_width() - self.tile_size * SQRT_2).max(0.0);
            self.set_polyline(&band.line, deck, true);
        }
        // после прорезок бордюрный барьер обязан остаться без диагональных
        // щелей. На узком мосту (аллея 3.5 м при тайле 2 м) цепочка настила
        // проходит через те же тайлы, что цепочка его же бордюра, и настил
        // отвоёвывает тайл себе — барьер продолжается со сдвигом в соседнюю
        // колонку, касаясь углом. Свой A* сквозь угол не шагает, но
        // OrdinalGrid из bevy_northstar (HPA*, Theta*) шагает по диагонали
        // между двумя заблокированными тайлами — та же угроза, что у тонких
        // рек (см. [`Self::visit_polyline`]). Латка — со внешней стороны: из
        // двух открытых ортогональных соседей диагональной пары блокируется
        // тот, что дальше от осевой моста-владельца, — настил не трогается,
        // щель закрыта снаружи
        loop {
            let mut seals: Vec<usize> = Vec::new();
            for (&index, tile) in &curb_tiles {
                if self.passable[index] {
                    continue;
                }
                let (x, y) = (
                    index as i32 / self.grid_size.y,
                    index as i32 % self.grid_size.y,
                );
                for (dx, dy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                    let Some(partner) = self.index(x + dx, y + dy) else {
                        continue;
                    };
                    let (Some(side), Some(vertical)) =
                        (self.index(x + dx, y), self.index(x, y + dy))
                    else {
                        continue;
                    };
                    if self.passable[partner] || !self.passable[side] || !self.passable[vertical] {
                        continue;
                    }
                    let way = bridge_ways[tile.owners[0] as usize - 1].0;
                    let outer = if distance_to_polyline(self.index_center(side), way)
                        >= distance_to_polyline(self.index_center(vertical), way)
                    {
                        side
                    } else {
                        vertical
                    };
                    seals.push(outer);
                }
            }
            if seals.is_empty() {
                break;
            }
            for index in seals {
                self.passable[index] = false;
            }
        }
        for area in &map.buildings {
            self.set_area(area, false);
        }
        for wall in &map.walls {
            let band = wall.band();
            self.set_polyline(&band.line, band.width, false);
        }
    }

    /// Арки режутся последними — после зданий, стен и оград: весь смысл в том,
    /// чтобы пробить только что залитый квартал.
    pub(super) fn carve_passages(&mut self, map: &MapData) {
        for road in map.roads.iter().filter(|road| road.passage) {
            let band = road.passage_band();
            self.set_polyline(&band.line, band.width, true);
        }
    }

    /// Ограды участков перекрывают сетку, а дороги, проходящие сквозь них,
    /// делают проёмы (`footprint::fence_gaps`).
    ///
    /// Проём режется **в маске заборов**, а не прорезкой по сетке: сначала
    /// собираются тайлы оград, из них вычитаются тайлы проёмов, и только
    /// остаток блокируется. Прорезать дорогой всю сетку нельзя — это сняло бы
    /// блокировку зданий и воды там, где тропа их касается; маска открывает
    /// ровно то, что закрыл бы сам забор (приём маски бордюров выше).
    ///
    /// Радиус проёма в тайлах — полудлина проёма плюс диагональ тайла. Забор
    /// растеризован 4-связной цепочкой, и на косой линии вынутый из неё один
    /// тайл щели не даёт: соседи по лесенке смыкаются через него углами.
    /// Диагональ — тот же запас на блуждание тайловых центров, что у прорезки
    /// настила моста.
    pub(super) fn fill_fences(&mut self, map: &MapData) {
        if map.fences.is_empty() {
            return;
        }
        let mut cache = self
            .fence_cache
            .take()
            .unwrap_or_else(|| FenceGapCache::build(&map.fences, &map.roads));
        // калитки только растут: пересобрать списки проёмов тех оград, у
        // которых они выросли, — дорожная половина не меняется
        cache.refresh(&map.fences);
        let gaps = cache.gaps();
        let margin = self.tile_size * SQRT_2;
        let mut blocked: Vec<usize> = Vec::new();
        for (fence, gaps) in map.fences.iter().zip(gaps) {
            let band = fence.band();
            let tile_size = self.tile_size;
            self.visit_polyline(&band.line, band.width, &mut |grid, x, y| {
                let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
                let in_gap = gaps
                    .iter()
                    .any(|gap| center.distance(gap.at) <= gap.reach + margin);
                if !in_gap && let Some(index) = grid.index(x, y) {
                    blocked.push(index);
                }
            });
        }
        for index in blocked {
            self.passable[index] = false;
        }
        self.fence_cache = Some(cache);
    }

    /// Тайлы и боксы, у которых пересбор сетки мог изменить проходимость, —
    /// бокс вокруг слитого проёма каждой добавленной калитки. Пересбор меняет
    /// сетку только на проёмах оград, а у слитого проёма новой калитки меняется
    /// и позиция: калитка, вставшая на дорожный проём ([`JOIN_EPSILON`]), тянет
    /// его на себя, и по краю старого проёма тайлы закрываются обратно —
    /// passable между раундами не монотонна. Радиус бокса — reach слитого
    /// проёма плюс [`JOIN_EPSILON`] на съезд головы слития к калитке и ещё
    /// [`JOIN_EPSILON`] на разлёт членов слияния от головы, максимум — по всем
    /// старым проёмам ограды в радиусе слияния, а не только ширине самой
    /// калитки ([`FENCE_GATE_WIDTH`]): второй проём ограды в том же стыке
    /// бывает шире первого, и его диск съезжает наравне с первым. Вердикт
    /// тайла меняется, только если его центр попадает в диск головы слития
    /// или члена слияния с запасом `margin` — оба лежат в боксе. До пересбора
    /// слитых списков ещё нет — проёмы берутся из старых списков ограды;
    /// промах — калитка стоит вдали от них и открывает ровно свою ширину.
    pub(super) fn gate_gap_boxes(
        &self,
        added: &[(usize, Vec2)],
    ) -> (Vec<(IVec2, IVec2)>, Vec<usize>) {
        let gaps = self
            .fence_cache
            .as_ref()
            .expect("gate rounds refill keeps the gap cache alive")
            .gaps();
        let margin = self.tile_size * SQRT_2;
        let mut boxes: Vec<(IVec2, IVec2)> = Vec::new();
        for &(fence, at) in added {
            let mut reach = FENCE_GATE_WIDTH / 2.0;
            for gap in &gaps[fence] {
                // члены слияния лежат не дальше 2·JOIN_EPSILON от калитки:
                // голова слития — не дальше JOIN_EPSILON (иначе калитка
                // открыла бы свой список с нуля), член — не дальше
                // JOIN_EPSILON от головы
                if gap.at.distance(at) < 2.0 * JOIN_EPSILON {
                    reach = reach.max(gap.reach);
                }
            }
            let reach = reach + margin + 2.0 * JOIN_EPSILON;
            let min = self.to_tile(at - reach);
            let max = self.to_tile(at + reach);
            boxes.push((min, max));
        }
        let mut watched: Vec<usize> = Vec::new();
        for &(min, max) in &boxes {
            for x in min.x..=max.x {
                for y in min.y..=max.y {
                    if let Some(index) = self.index(x, y) {
                        watched.push(index);
                    }
                }
            }
        }
        watched.sort_unstable();
        watched.dedup();
        (boxes, watched)
    }

    /// Тайлы арок — проездов сквозь здания — одним списком. Набор не зависит
    /// ни от оград, ни от раундов калиток: арка кладётся поверх основы тем же
    /// обходом, что и в [`Self::carve_passages`], поэтому собирается один раз
    /// и возвращается на место локальным refill'ом вместо растеризации всех
    /// проездов на каждый раунд.
    pub(super) fn passage_tiles(&mut self, map: &MapData) -> Vec<usize> {
        let mut tiles: Vec<usize> = Vec::new();
        for road in map.roads.iter().filter(|road| road.passage) {
            let band = road.passage_band();
            self.visit_polyline(&band.line, band.width, &mut |grid, x, y| {
                if let Some(index) = grid.index(x, y) {
                    tiles.push(index);
                }
            });
        }
        tiles.sort_unstable();
        tiles.dedup();
        tiles
    }

    /// Локальный refill раундов калиток — то же, что сброс к основе,
    /// [`Self::fill_fences`] и [`Self::carve_passages`], но заново кладётся
    /// только то, что пересбор мог изменить: основа восстанавливается в
    /// боксах проёмов свежих калиток ([`Self::gate_gap_boxes`]), ограды —
    /// отрезками, чьи коробки пересекают боксы (тот же обход, что у полной
    /// заливки, байт в байт), арки — своим списком тайлов. За пределами
    /// боксов вердикты проёмов не менялись: повторная блокировка там ничего
    /// не меняет, маска режет проём, а не прорезает сетку.
    pub(super) fn refill_gate_boxes(
        &mut self,
        map: &MapData,
        fences: &FenceIndex,
        base: &[bool],
        boxes: &[(IVec2, IVec2)],
        watched: &[usize],
        passages: &[usize],
    ) {
        for &index in watched {
            self.passable[index] = base[index];
        }
        let mut cache = self
            .fence_cache
            .take()
            .unwrap_or_else(|| FenceGapCache::build(&map.fences, &map.roads));
        // калитки только растут: пересобрать списки проёмов тех оград, у
        // которых они выросли, — дорожная половина не меняется
        cache.refresh(&map.fences);
        let gaps = cache.gaps();
        let margin = self.tile_size * SQRT_2;
        let tile_size = self.tile_size;
        let band = FENCE_BAND_WIDTH;
        let mut blocked: Vec<usize> = Vec::new();
        for &(min_tile, max_tile) in boxes {
            let min = Vec2::new(min_tile.x as f32, min_tile.y as f32) * tile_size;
            let max = Vec2::new((max_tile.x + 1) as f32, (max_tile.y + 1) as f32) * tile_size;
            // коробка запроса растёт на полширины ленты: тайл бокса с центром
            // в ленте отрезка сажает отрезок не дальше полуширины от бокса, а
            // тайл цепочки отрезка и вовсе лежит в боксе
            for &(fence, segment) in &fences.segments_in_box(min - band / 2.0, max + band / 2.0) {
                let points = &map.fences[fence as usize].points;
                let gaps = &gaps[fence as usize];
                self.visit_segment_band(
                    points[segment as usize],
                    points[segment as usize + 1],
                    band,
                    [false; 2],
                    &mut |grid, x, y| {
                        if let Some(index) = grid.index(x, y) {
                            let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
                            let in_gap = gaps
                                .iter()
                                .any(|gap| center.distance(gap.at) <= gap.reach + margin);
                            if !in_gap {
                                blocked.push(index);
                            }
                        }
                    },
                );
            }
        }
        for index in blocked {
            self.passable[index] = false;
        }
        for &index in passages {
            self.passable[index] = true;
        }
        self.fence_cache = Some(cache);
    }
}

/// Бордюрный тайл на этапе заливки.
#[derive(Default)]
struct CurbTile {
    /// Bridge-ways, чей бордюр проходит через тайл, в порядке обхода
    /// `coverage.bridges()` — все, сколько есть, без предела.
    ///
    /// Предел был: два слота, «проезжая часть и её тротуар». Пара — не
    /// максимум, а типичный случай; на узле, где сходятся несколько мостовых
    /// way (пешеходная развязка), в один тайл приходят бордюры трёх и более
    /// (счёт по городам — в скилле `navigation-deep`). Терять их нельзя:
    /// решение ниже — `any` по владельцам, то есть выброшенный владелец может
    /// только **снять** барьер, никогда не поставить, и терялся именно тот
    /// единственный, чей щуп уходил наружу.
    owners: Vec<u32>,
    /// Накрыт панелью примыкающей обычной дороги.
    road: bool,
}

/// Ближайшая к `point` точка ломаной. `pub(super)` — ту же ломаную щупают
/// калитки ([`super::gates`]).
pub(super) fn closest_point_on_polyline(point: Vec2, points: &[Vec2]) -> Vec2 {
    let mut best = points[0];
    let mut best_distance = f32::INFINITY;
    for segment in points.windows(2) {
        let candidate = closest_on_segment(point, segment[0], segment[1]);
        let distance = point.distance_squared(candidate);
        if distance < best_distance {
            best_distance = distance;
            best = candidate;
        }
    }
    best
}

/// Сторона ячейки пространственного хеша лент мостов, м. Того же порядка, что
/// `NEARBY_CELL` у посадки деревьев (`map/osm/planting/index.rs`): в ячейке
/// должно лежать несколько сегментов, а не весь мост и не полкарты. В
/// `settings.rs` ей не место — от неё зависит только скорость щупа, ответ не
/// зависит по построению (см. [`BridgeBands`]).
///
/// Своя, а не общая с посадкой, и это правило самой сетки: шаг —
/// решение потребителя, потому что число про предметную область, а не про
/// арифметику ячеек (`map/grid.rs`). Общей стала арифметика.
const BRIDGE_BAND_CELL: f32 = 32.0;

/// Отрезок осевой bridge-way вместе с полушириной его ленты
/// (`RoadLine::curb_reach`) и номером владельца в `bridge_ways`.
#[derive(Clone, Copy)]
struct BridgeBand {
    owner: u32,
    reach: f32,
    from: Vec2,
    to: Vec2,
}

/// Ленты всех bridge-ways в равномерной сетке — индекс под щуп «что снаружи».
///
/// Щуп спрашивает одно: накрыт ли пробник лентой ЧУЖОГО way. Линейный перебор
/// стоил «бордюрные тайлы × сегменты всех мостов», а бордюрных тайлов тем
/// больше, чем больше мостов: на Лондоне (499 bridge-ways против 61 у Тулы) это
/// уже квадратичный кусок заливки, и идёт он в потоке загрузки.
///
/// Ответ индекса **точен**, а не приближён, поэтому запрашивается ровно одна
/// ячейка: отрезок кладётся во все ячейки своего AABB, расширенного на `reach`
/// собственного way, значит любая точка ближе `reach` к отрезку лежит внутри
/// этого AABB — её ячейка одна из тех, куда отрезок положен. Ни допуска, ни
/// обхода соседних ячеек не нужно.
struct BridgeBands(Grid<BridgeBand>);

impl BridgeBands {
    /// `ways` — те же пары `(осевая, curb_reach)`, что перебирал щуп; номер в
    /// срезе и есть владелец.
    fn build(ways: &[(&[Vec2], f32)]) -> Self {
        let mut cells = Grid::new(BRIDGE_BAND_CELL);
        for (owner, &(points, reach)) in ways.iter().enumerate() {
            for segment in points.windows(2) {
                let (from, to) = (segment[0], segment[1]);
                let band = BridgeBand {
                    owner: owner as u32,
                    reach,
                    from,
                    to,
                };
                cells.insert_segment(from, to, reach, band);
            }
        }
        Self(cells)
    }

    /// Лежит ли `probe` в ленте какого-нибудь way, кроме `owner`. Предикат
    /// дословно тот же, что у перебора: `distance_to_polyline` — минимум по
    /// отрезкам, а «минимум ≤ порога» и есть «нашёлся отрезок ≤ порога».
    fn covered_by_other(&self, probe: Vec2, owner: usize) -> bool {
        self.0.at(probe).iter().any(|band| {
            band.owner as usize != owner
                && distance_to_segment(probe, band.from, band.to) <= band.reach
        })
    }
}

#[cfg(test)]
mod tests;
