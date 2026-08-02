use std::collections::HashMap;
use std::f32::consts::SQRT_2;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use bevy::prelude::*;

use crate::grid::world_to_tile;
use crate::map::osm::model::{MapData, PolyArea, distance_to_segment, ring_bounds};
use crate::map::{bridge_curb_width, miter_offsets};
use crate::settings::{PASSAGE_MAX_WIDTH, navtile_size};

/// Стоимость шага между тайлами (для A*): прямой и диагональный.
pub const COST_STRAIGHT: i32 = 100;
pub const COST_DIAGONAL: i32 = 141;
/// Множитель эвристики — та же шкала, что и стоимость шага.
pub const COST_MULTIPLIER: f32 = 100.0;

/// Тайловая сетка проходимости. Индексация — `x * grid_size.y + y`,
/// тайлы за границей карты непроходимы.
///
/// Размеры — поля, а не глобалы: заливка снимает их с текущего
/// [`navtile_size`], и всё, что работает по снапшоту (постройка northstar,
/// отменённая при смене размера), ходит по размерам **своего** снапшота,
/// а не по уже переключённому атомику.
///
/// `Clone` — для снапшота под постройку иерархии northstar: копия сетки
/// стоит один memcpy, а чтение оригинала под локом заняло бы все ~10 с
/// постройки (см. `northstar::start_northstar_build`).
#[derive(Clone)]
pub struct Navmesh {
    passable: Vec<bool>,
    /// Размер сетки в тайлах на момент заливки.
    pub grid_size: IVec2,
    /// Размер тайла в метрах на момент заливки.
    pub tile_size: f32,
}

impl Default for Navmesh {
    fn default() -> Self {
        let grid_size = crate::settings::grid_size();
        Self {
            passable: vec![true; (grid_size.x * grid_size.y) as usize],
            grid_size,
            tile_size: navtile_size(),
        }
    }
}

impl Navmesh {
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.grid_size.x && y < self.grid_size.y)
            .then_some((x * self.grid_size.y + y) as usize)
    }

    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|index| self.passable[index])
    }

    pub fn set_passable(&mut self, x: i32, y: i32, value: bool) {
        if let Some(index) = self.index(x, y) {
            self.passable[index] = value;
        }
    }

    /// Соседи тайла для A*: 8 направлений, диагональ только когда оба смежных
    /// прямых тайла проходимы (чтобы путь не резал углы зданий).
    pub fn successors(&self, x: i32, y: i32) -> Vec<(IVec2, i32)> {
        let mut result = Vec::with_capacity(8);
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if !self.is_passable(nx, ny) {
                continue;
            }
            let is_diagonal = dx != 0 && dy != 0;
            if is_diagonal && !(self.is_passable(x, ny) && self.is_passable(nx, y)) {
                continue;
            }
            result.push((
                IVec2::new(nx, ny),
                if is_diagonal {
                    COST_DIAGONAL
                } else {
                    COST_STRAIGHT
                },
            ));
        }
        result
    }

    /// Заполнение из OSM-карты. Порядок важен: мосты прорезают проходимые
    /// коридоры поверх воды (иначе Упа разрезает карту надвое), здания и стены
    /// блокируют уже после, а арки прорезаются последними — их смысл именно в
    /// том, чтобы пробить только что заблокированный дом.
    ///
    /// Бордюры мостов ([`bridge_curb_width`]) непроходимы: с моста не сходят
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
        // сетка переживает смену города: без сброса на новой карте остались
        // бы дома и прунинг старой. Здесь же подхватывается текущий размер
        // навтайла — дефолтная аллокация при `init_resource` сделана до
        // восстановления настроек и права быть не обязана
        self.tile_size = navtile_size();
        self.grid_size = crate::settings::grid_size();
        let len = (self.grid_size.x * self.grid_size.y) as usize;
        if self.passable.len() != len {
            self.passable = vec![true; len];
        } else {
            self.passable.fill(true);
        }
        for area in &map.water {
            self.set_area(area, false);
        }
        for line in map.water_lines.iter().filter(|line| !line.tunnel) {
            self.set_polyline(&line.points, line.width, false);
        }
        // бордюры мостов. Тайл бордюра блокируется не безусловно: OSM режет
        // один физический мост на несколько ways (проезжая часть и тротуар —
        // параллельные ленты, эстакада — цепочка кусков), и бордюр одного way
        // не должен перегораживать ни настил соседнего, ни примыкающую
        // дорогу. Поэтому сначала по каждому бордюрному тайлу собираются
        // владельцы (чей бордюр) и примыкания, а блокировка решается ниже
        // щупом «что снаружи»
        let bridge_ways: Vec<(&[Vec2], f32)> = map
            .roads
            .iter()
            .filter(|road| road.bridge)
            .map(|road| {
                (
                    road.points.as_slice(),
                    road.width / 2.0 + bridge_curb_width(road.width),
                )
            })
            .collect();
        let mut curb_tiles: HashMap<usize, CurbTile> = HashMap::new();
        for (index, road) in map.roads.iter().filter(|road| road.bridge).enumerate() {
            let id = index as u32 + 1;
            let curb = bridge_curb_width(road.width);
            let offsets = miter_offsets(&road.points, false, (road.width + curb) / 2.0);
            for side in [-1.0, 1.0] {
                let edge: Vec<Vec2> = road
                    .points
                    .iter()
                    .zip(&offsets)
                    .map(|(&point, &offset)| point + side * offset)
                    .collect();
                self.visit_polyline(&edge, curb, &mut |grid, x, y| {
                    if let Some(index) = grid.index(x, y) {
                        push_id(&mut curb_tiles.entry(index).or_default().owners, id);
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
        for road in map.roads.iter().filter(|road| !road.bridge) {
            let joins = bridge_ways.iter().any(|&(way, _)| {
                road.points
                    .iter()
                    .any(|&point| distance_to_polyline(point, way) < JOIN_EPSILON)
                    || way
                        .iter()
                        .any(|&point| distance_to_polyline(point, &road.points) < JOIN_EPSILON)
            });
            if !joins {
                continue;
            }
            let width = road.width + self.tile_size * SQRT_2;
            self.visit_polyline_rect(&road.points, width, &mut |grid, x, y| {
                if let Some(index) = grid.index(x, y)
                    && let Some(tile) = curb_tiles.get_mut(&index)
                {
                    tile.road = true;
                }
            });
        }
        let (grid_height, tile_size) = (self.grid_size.y, self.tile_size);
        let tile_center = move |index: usize| -> Vec2 {
            let (x, y) = (index as i32 / grid_height, index as i32 % grid_height);
            (Vec2::new(x as f32, y as f32) + 0.5) * tile_size
        };
        // блокировка — щупом «что снаружи»: тайл держит бордюр, если на шаг
        // наружу от осевой владельца НЕ лежит лента другого bridge-way. Так
        // пара «мост + тротуар» запирается по внешнему краю ленты, которая
        // оказалась крайней, — даже когда номинальная ширина проезжей части
        // (primary 16 м) заглатывает свой тротуар целиком и правило «чужая
        // лента накрыла — открыто» оставило бы пару вовсе без барьера.
        // Внутренние швы при этом открыты: щуп из шва попадает в соседнюю
        // ленту. Блокировка ничего не открывает сама по себе: тайл, который
        // щуп оставил открытым, может всё ещё лежать в воде
        for (&index, tile) in &curb_tiles {
            if tile.road {
                continue;
            }
            let center = tile_center(index);
            let holds = tile.owners.iter().filter(|&&id| id != 0).any(|&id| {
                let owner = id as usize - 1;
                let closest = closest_point_on_polyline(center, bridge_ways[owner].0);
                let outward = (center - closest).normalize_or(Vec2::X);
                let probe = center + outward * self.tile_size;
                !bridge_ways.iter().enumerate().any(|(other, &(way, band))| {
                    other != owner && distance_to_polyline(probe, way) <= band
                })
            });
            if holds {
                self.passable[index] = false;
            }
        }
        for road in map.roads.iter().filter(|road| road.bridge) {
            // настил у́же полной ширины на диагональ тайла. Тайлы цепочки
            // бордюра метятся по «осевая бордюра проходит через тайл», и на
            // косом мосту центр такого тайла отклоняется от неё до полудиагонали
            // (√2 м) — то есть залезает внутрь настила. Прорезка полной шириной
            // открывала такие тайлы обратно, и барьер превращался в пунктир.
            // Урезание ровно на этот заход оставляет цепочку бордюра целой при
            // любом угле, а связность настила держит его собственная цепочка по
            // осевой — так же, как у тонких рек в set_polyline.
            let curb = bridge_curb_width(road.width);
            let deck = (road.width + curb - self.tile_size * SQRT_2).max(0.0);
            self.set_polyline(&road.points, deck, true);
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
                    let outer = if distance_to_polyline(tile_center(side), way)
                        >= distance_to_polyline(tile_center(vertical), way)
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
            self.set_polyline(&wall.points, wall.width, false);
        }
        for road in map.roads.iter().filter(|road| road.passage) {
            self.set_polyline(&road.points, road.width.min(PASSAGE_MAX_WIDTH), true);
        }
    }

    /// Тайлы, чей центр внутри полигона (с учётом дырок) — построчной
    /// заливкой (см. `row_spans`).
    fn set_area(&mut self, area: &PolyArea, value: bool) {
        let (min, max) = ring_bounds(&area.outer);
        let min_tile = world_to_tile(min);
        let max_tile = world_to_tile(max);
        let mut scratch = RowScratch::default();
        for y in min_tile.y.max(0)..=max_tile.y.min(self.grid_size.y - 1) {
            row_spans(&area.outer, &area.holes, y, &mut scratch);
            for &(from, to) in &scratch.spans {
                for x in from.max(0)..=to.min(self.grid_size.x - 1) {
                    self.set_passable(x, y, value);
                }
            }
        }
    }

    /// Тайлы, недостижимые из `start`, становятся непроходимыми: замкнутые
    /// дворы и острова иначе порождают заведомо безуспешные A*-поиски,
    /// обходящие всю карту (десятки мс каждый). 4-связность совпадает с
    /// достижимостью A*: диагональ требует обоих смежных прямых тайлов.
    pub fn prune_unreachable(&mut self, start: IVec2) -> usize {
        let Some(start_index) = self.index(start.x, start.y) else {
            return 0;
        };
        if !self.passable[start_index] {
            return 0;
        }

        let mut reachable = vec![false; self.passable.len()];
        let mut queue = std::collections::VecDeque::new();
        reachable[start_index] = true;
        queue.push_back(start);
        while let Some(tile) = queue.pop_front() {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (tile.x + dx, tile.y + dy);
                if let Some(index) = self.index(nx, ny)
                    && self.passable[index]
                    && !reachable[index]
                {
                    reachable[index] = true;
                    queue.push_back(IVec2::new(nx, ny));
                }
            }
        }

        let mut pruned = 0;
        for (index, is_reachable) in reachable.iter().enumerate() {
            if self.passable[index] && !is_reachable {
                self.passable[index] = false;
                pruned += 1;
            }
        }
        pruned
    }

    /// Тайлы в пределах полуширины от осевой полилинии — **плюс** все тайлы,
    /// через которые осевая проходит ([`Self::visit_segment_tiles`]).
    ///
    /// Одной полуширины мало. Тайлы метятся по «центр ближе полуширины», и
    /// лента у́же `tile_size · √2` (2.83 м при тайле 2 м) на косой линии вырождается в
    /// цепочку тайлов, соприкасающихся **углами**: ручей в 2.5 м рисуется в
    /// навмеш-оверлее шахматкой. Своим A* её не перейти — он не срезает углы, —
    /// но `OrdinalGrid` из `bevy_northstar` (HPA*, Theta*) собирается без
    /// такого фильтра и шагает по диагонали прямо между двумя
    /// заблокированными тайлами, а `line_of_sight` сэмплирует точки и
    /// проскакивает через место касания. На Туле это и вышло: на HPA* люди
    /// ходили через ручей.
    ///
    /// Поднимать ширину до минимума — лечение симптома: порог зависит от угла и
    /// от сдвига линии относительно сетки, и даже 3 м оставляли щель. Проход по
    /// осевой даёт четырёхсвязную цепочку **по построению**, при любой ширине,
    /// угле и сдвиге, и при этом не раздувает канаву в 1.5 м до трёх метров.
    fn set_polyline(&mut self, points: &[Vec2], width: f32, value: bool) {
        self.visit_polyline(points, width, &mut |grid, x, y| {
            grid.set_passable(x, y, value)
        });
    }

    /// Обход тех же тайлов без записи в сетку: `visit` решает сам — так
    /// собирается маска бордюров и режутся проёмы «только там, где бордюр».
    fn visit_polyline(
        &mut self,
        points: &[Vec2],
        width: f32,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        let (grid_size, tile_size) = (self.grid_size, self.tile_size);
        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let min_tile = world_to_tile(from.min(to) - width);
            let max_tile = world_to_tile(from.max(to) + width);
            for x in min_tile.x.max(0)..=max_tile.x.min(grid_size.x - 1) {
                for y in min_tile.y.max(0)..=max_tile.y.min(grid_size.y - 1) {
                    let center = (Vec2::new(x as f32, y as f32) + 0.5) * tile_size;
                    if distance_to_segment(center, from, to) <= width / 2.0 {
                        visit(self, x, y);
                    }
                }
            }
            self.visit_segment_tiles(from, to, visit);
        }
    }

    /// Тайлы в прямоугольнике вокруг каждого сегмента: как
    /// [`Self::visit_polyline`], но без капсульных продлений за концы (та же
    /// разница, что `Butt` против `Round` у торцов ленты в отрисовке) и без
    /// цепочки по осевой — покрытию бордюров связность не нужна.
    fn visit_polyline_rect(
        &mut self,
        points: &[Vec2],
        width: f32,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        for segment in points.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            let delta = to - from;
            let length = delta.length();
            let Some(direction) = delta.try_normalize() else {
                continue;
            };
            let min_tile = world_to_tile(from.min(to) - width);
            let max_tile = world_to_tile(from.max(to) + width);
            for x in min_tile.x.max(0)..=max_tile.x.min(self.grid_size.x - 1) {
                for y in min_tile.y.max(0)..=max_tile.y.min(self.grid_size.y - 1) {
                    let center = (Vec2::new(x as f32, y as f32) + 0.5) * self.tile_size;
                    let along = (center - from).dot(direction);
                    let lateral = (center - from).perp_dot(direction).abs();
                    if (0.0..=length).contains(&along) && lateral <= width / 2.0 {
                        visit(self, x, y);
                    }
                }
            }
        }
    }

    /// Тайлы, через которые проходит отрезок, — обход сетки по Amanatides–Woo:
    /// на каждом шаге пересекается ближайшая граница, по x либо по y, поэтому
    /// соседние тайлы цепочки всегда смежны **по стороне**, а не по углу.
    /// Именно эта четырёхсвязность и делает преграду непроходимой для всех
    /// потребителей сетки (см. [`Self::set_polyline`]).
    fn visit_segment_tiles(
        &mut self,
        from: Vec2,
        to: Vec2,
        visit: &mut impl FnMut(&mut Self, i32, i32),
    ) {
        let mut tile = world_to_tile(from);
        let end = world_to_tile(to);
        let delta = to - from;

        // t — доля отрезка; t_max — до следующей границы по оси, t_delta — шаг
        // между границами. Нулевая проекция даёт бесконечность: по этой оси
        // граница не пересекается никогда.
        let tile_size = self.tile_size;
        let axis = move |d: f32, origin: f32, tile: i32| {
            if d == 0.0 {
                return (f32::INFINITY, f32::INFINITY, 0);
            }
            let step = if d > 0.0 { 1 } else { -1 };
            let boundary = (tile + step.max(0)) as f32 * tile_size;
            ((boundary - origin) / d, tile_size / d.abs(), step)
        };
        let (mut t_max_x, t_delta_x, step_x) = axis(delta.x, from.x, tile.x);
        let (mut t_max_y, t_delta_y, step_y) = axis(delta.y, from.y, tile.y);

        visit(self, tile.x, tile.y);
        // потолок шагов — страховка от вырожденного отрезка: ходов не больше,
        // чем тайлов по обеим осям вместе
        let limit = (end.x - tile.x).abs() + (end.y - tile.y).abs();
        for _ in 0..limit {
            if t_max_x < t_max_y {
                tile.x += step_x;
                t_max_x += t_delta_x;
            } else {
                tile.y += step_y;
                t_max_y += t_delta_y;
            }
            visit(self, tile.x, tile.y);
        }
    }
}

/// Насколько близко точка одной ломаной должна лежать к другой ломаной,
/// чтобы дороги считались примыкающими. Развязка в OSM — это общий узел,
/// то есть буквально одна и та же точка в обеих ways; допуск покрывает лишь
/// потерю точности проекции.
const JOIN_EPSILON: f32 = 0.5;

/// Бордюрный тайл на этапе заливки. Два слота владельцев хватает: физически
/// на одном тайле встречаются бордюры максимум двух соседних лент (проезжая
/// часть и её тротуар).
#[derive(Default)]
struct CurbTile {
    /// Bridge-ways, чей бордюр проходит через тайл.
    owners: [u32; 2],
    /// Накрыт панелью примыкающей обычной дороги.
    road: bool,
}

/// Минимальное расстояние от точки до ломаной — по всем её сегментам.
fn distance_to_polyline(point: Vec2, points: &[Vec2]) -> f32 {
    points
        .windows(2)
        .map(|segment| distance_to_segment(point, segment[0], segment[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Ближайшая к `point` точка ломаной.
fn closest_point_on_polyline(point: Vec2, points: &[Vec2]) -> Vec2 {
    let mut best = points[0];
    let mut best_distance = f32::INFINITY;
    for segment in points.windows(2) {
        let (from, to) = (segment[0], segment[1]);
        let delta = to - from;
        let length_squared = delta.length_squared();
        let candidate = if length_squared == 0.0 {
            from
        } else {
            from + delta * ((point - from).dot(delta) / length_squared).clamp(0.0, 1.0)
        };
        let distance = point.distance_squared(candidate);
        if distance < best_distance {
            best_distance = distance;
            best = candidate;
        }
    }
    best
}

/// Дописывает id в первый свободный слот; дубликаты и переполнение — no-op
/// (третий пересекающийся way не меняет исхода блокировки).
fn push_id(slots: &mut [u32; 2], id: u32) {
    if slots.contains(&id) {
        return;
    }
    if slots[0] == 0 {
        slots[0] = id;
    } else if slots[1] == 0 {
        slots[1] = id;
    }
}

/// x-координаты пересечений кольца с горизонталью `scan_y`. Условие
/// пересечения — ровно то же, что в `point_in_polygon`, включая строгие
/// сравнения: иначе построчная заливка разошлась бы с точечной проверкой на
/// кромке полигона.
fn ring_crossings(ring: &[Vec2], scan_y: f32, out: &mut Vec<f32>) {
    if ring.len() < 2 {
        return;
    }
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > scan_y) != (b.y > scan_y) {
            out.push((b.x - a.x) * (scan_y - a.y) / (b.y - a.y) + a.x);
        }
        j = i;
    }
}

/// Отрезки тайлов `[from, to]` строки `y`, чьи центры лежат внутри кольца.
///
/// Это и есть замена перебору «каждый тайл AABB × всё кольцо»: кольцо
/// проходится один раз на строку, а не один раз на тайл. На доме разницы
/// нет, на Темзе — три порядка (её AABB тянется через полкарты).
fn ring_spans(ring: &[Vec2], scan_y: f32, crossings: &mut Vec<f32>, out: &mut Vec<(i32, i32)>) {
    out.clear();
    crossings.clear();
    ring_crossings(ring, scan_y, crossings);
    if crossings.is_empty() {
        return;
    }
    crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // `point_in_polygon` переключает флаг на каждом пересечении справа от
    // точки, значит внутренние отрезки — это пары [c0, c1), [c2, c3), …
    // Нечётный хвост (вырожденное кольцо) отбрасывается вместе с `chunks_exact`.
    let tile_size = navtile_size();
    for pair in crossings.chunks_exact(2) {
        // центр тайла x — это (x + 0.5) * tile_size; ищем x с
        // pair[0] <= центр < pair[1]
        let from = (pair[0] / tile_size - 0.5).ceil() as i32;
        let to = (pair[1] / tile_size - 0.5).ceil() as i32 - 1;
        if from <= to {
            out.push((from, to));
        }
    }
}

/// Переиспользуемые буферы построчной заливки: на реке строк тысячи, и
/// аллокация на каждую съела бы часть выигрыша.
#[derive(Default)]
struct RowScratch {
    crossings: Vec<f32>,
    /// Результат строки — отрезки внешнего кольца за вычетом дырок.
    spans: Vec<(i32, i32)>,
    holes: Vec<(i32, i32)>,
}

/// Отрезки строки `y` для полигона с дырками.
///
/// Дырки вычитаются отрезками, а не сваливаются в общий even-odd список:
/// even-odd совпал бы с прежней поточечной проверкой только для дырок строго
/// внутри внешнего кольца, а кусок дырки, вылезший наружу (кривая
/// OSM-мультиполигональная связка), он бы, наоборот, залил.
fn row_spans(outer: &[Vec2], holes: &[Vec<Vec2>], y: i32, scratch: &mut RowScratch) {
    let scan_y = (y as f32 + 0.5) * navtile_size();
    let RowScratch {
        crossings,
        spans,
        holes: hole_spans,
    } = scratch;
    ring_spans(outer, scan_y, crossings, spans);

    for hole in holes {
        if spans.is_empty() {
            return;
        }
        ring_spans(hole, scan_y, crossings, hole_spans);
        for &(cut_from, cut_to) in hole_spans.iter() {
            let mut index = 0;
            while index < spans.len() {
                let (from, to) = spans[index];
                if cut_to < from || cut_from > to {
                    index += 1;
                    continue;
                }
                spans.remove(index);
                if cut_to < to {
                    spans.insert(index, (cut_to + 1, to));
                }
                if from < cut_from {
                    spans.insert(index, (from, cut_from - 1));
                    index += 1;
                }
            }
        }
    }
}

/// Navmesh под `Arc<RwLock>` — его читают async-задачи поиска пути.
#[derive(Resource)]
pub struct ArcNavmesh(pub Arc<RwLock<Navmesh>>);

impl Default for ArcNavmesh {
    fn default() -> Self {
        // пустой (всё проходимо); заполняется системой `fill_navmesh`,
        // когда `MapData` загружена
        Self(Arc::new(RwLock::new(Navmesh::default())))
    }
}

impl ArcNavmesh {
    pub fn read(&self) -> RwLockReadGuard<'_, Navmesh> {
        self.0.read().unwrap()
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Navmesh> {
        self.0.write().unwrap()
    }
}

#[cfg(test)]
mod tests;
