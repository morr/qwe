//! Какая вокруг застройка — и сколько от этого во дворе машин.
//!
//! До сих пор слой машин знал про место только одно: какой ширины улица и
//! какого размера стоянка. На снимке города это неверно вдвойне. Квартал
//! частных домов запаркован **редко** — у каждого дома свой двор и свой гараж,
//! а вдоль улицы стоят единицы; микрорайон девятиэтажек, наоборот, заставлен
//! сплошь, потому что ставить машину больше негде. По кешу Тулы в
//! низкоэтажных кварталах лежит **больше половины** всей парковочной улицы
//! (98.7 км из 188) и 46 размеченных стоянок из 159 — то есть половина города
//! была запаркована по нормам микрорайона.
//!
//! Мера квартала здесь одна — **средневзвешенная по пятну высота** застройки
//! в радиусе [`REACH`] от точки, в этажах. Взвешивание по пятну, а не по числу
//! домов, — потому что иначе два десятка гаражей рядом с девятиэтажкой
//! пересилили бы её числом, хотя машины стоят у неё, а не у них; и наоборот,
//! в частном секторе сотня мелких домов даёт ту же высоту, что и десяток, —
//! квартал не становится выше от того, что он гуще. Медиана читалась бы здесь
//! хуже: граница между кварталами обязана быть мягкой, а средневзвешенная даёт
//! плавный переход — одна секция на краю частного сектора уже немного
//! поднимает его этажность, целый ряд секций поднимает до конца.
//!
//! Считается **по всем** контурам `MapData::buildings`, включая храмы и
//! кремль: мера — «какой высоты то, что здесь стоит», ровно то, что читает
//! глаз на снимке, и башня кремля действительно делает место вокруг себя не
//! частным сектором. Отдельных исключений нет нарочно — каждое из них пришлось
//! бы обосновывать, а вклад их по Туле в доли метра.
//!
//! Пустая окрестность (стоянка у шоссе, промзона на краю карты) даёт `None` и
//! множитель 1 — там остаётся прежнее правило по размеру. По той же причине
//! пустой индекс нейтрален, и витрина `examples/demos/car_gallery`, у которой
//! домов нет вовсе, строит ряды ровно как раньше.

use bevy::math::Vec2;

use crate::map::buildings::height_or_default;
use crate::map::grid::Grid;
use crate::map::osm::PolyArea;
use crate::map::osm::model::{ring_vertex_mean, signed_ring_area};
use crate::settings::STOREY_HEIGHT;

/// Радиус, в котором читается застройка, м: примерно квартал. Меньше — и ряд
/// вдоль улицы менялся бы от дома к дому; больше — и частный сектор рядом с
/// микрорайоном перестал бы от него отличаться.
const REACH: f32 = 120.0;
/// Клетка индекса, м. Равна радиусу, и дом кладётся во **все** клетки, до
/// которых достаёт его радиус, — тогда запрос читает ровно одну клетку, как
/// веер вагонов (`map::wagons::Fan`).
const CELL: f32 = REACH;

/// Этажность, ниже которой квартал читается как частный сектор, и выше
/// которой — как микрорайон. Два этажа — это дом с мансардой и сарай рядом
/// (`buildings::heights` даёт частному дому в основном один этаж); пять —
/// панельная секция, самая населённая ветка вывода этажей.
const LOW_STOREYS: f32 = 2.0;
const HIGH_STOREYS: f32 = 5.0;

/// Во сколько раз плотнее обычного стоят машины в этих двух кварталах.
/// Низкоэтажный — «намного реже»: четверть, то есть у бордюра машина
/// примерно раз в полсотни метров, по машине на два-три участка. Многоэтажный
/// — «немного плотнее»: прибавка в 15 %, не больше, потому что дальше ряд
/// смыкается в сплошную ленту и снова читается автосалоном — тем самым, от
/// чего уходит [`super::lot_occupancy`].
const LOW_FILL: f32 = 0.25;
const HIGH_FILL: f32 = 1.15;

/// Дом в индексе: центр пятна, само пятно как вес и высота отрисовки.
struct Block {
    at: Vec2,
    weight: f32,
    height: f32,
}

/// Застройка города, разложенная по клеткам: по точке отдаёт множитель
/// занятости мест вокруг неё.
///
/// Строится заново на каждую пересборку слоя машин, а не кешируется на
/// загрузку мира, — ровно по той же причине, по которой не кешируются разрывы
/// на перекрёстках: по Туле это единицы миллисекунд на 7.6 тысячи домов, а
/// весь слой машин — проценты от зданиевого.
pub(crate) struct Districts {
    blocks: Vec<Block>,
    /// Номера домов из [`Self::blocks`] по ячейкам, до которых достаёт их
    /// радиус — имя `blocks` занято самим вектором, в который они индексируют.
    blocks_by_cell: Grid<u32>,
}

impl Districts {
    pub(crate) fn new(buildings: &[PolyArea]) -> Self {
        let mut blocks = Vec::with_capacity(buildings.len());
        let mut blocks_by_cell = Grid::new(CELL);
        for building in buildings {
            let Some(at) = ring_vertex_mean(&building.outer) else {
                continue;
            };
            let weight = signed_ring_area(&building.outer).abs();
            if weight <= 0.0 {
                continue;
            }
            let index = blocks.len() as u32;
            blocks.push(Block {
                at,
                weight,
                height: height_or_default(building),
            });
            blocks_by_cell.insert(at - REACH, at + REACH, index);
        }
        Self {
            blocks,
            blocks_by_cell,
        }
    }

    /// Множитель занятости мест у `point`: [`LOW_FILL`] в частном секторе,
    /// [`HIGH_FILL`] в микрорайоне, линейно между ними и ровно 1 там, где
    /// домов рядом нет вовсе.
    pub(super) fn fill_at(&self, point: Vec2) -> f32 {
        let Some(storeys) = self.storeys_at(point) else {
            return 1.0;
        };
        let t = ((storeys - LOW_STOREYS) / (HIGH_STOREYS - LOW_STOREYS)).clamp(0.0, 1.0);
        LOW_FILL + (HIGH_FILL - LOW_FILL) * t
    }

    /// Этажность застройки вокруг `point` — средневзвешенная по пятну высота
    /// домов ближе [`REACH`], делённая на высоту этажа. `None`, если рядом не
    /// стоит ничего.
    ///
    /// Её же читает разбор: тротуар у жилой улицы без тега решает та же мера
    /// квартала (`osm::parse::infer_sidewalks`), чтобы машины у бордюра и
    /// полоса вдоль него видели один и тот же частный сектор.
    pub(crate) fn storeys_at(&self, point: Vec2) -> Option<f32> {
        let mut weight = 0.0;
        let mut volume = 0.0;
        for &index in self.blocks_by_cell.at(point) {
            let block = &self.blocks[index as usize];
            if block.at.distance_squared(point) > REACH * REACH {
                continue;
            }
            weight += block.weight;
            volume += block.weight * block.height;
        }
        (weight > 0.0).then(|| volume / weight / STOREY_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{building, square};

    /// Дом со **своей** высотой: этажность квартала меряется тем же
    /// `height_or_default`, так что тег снимает с теста зависимость от вывода.
    fn house(at: Vec2, side: f32, height: f32) -> PolyArea {
        PolyArea {
            height: Some(height),
            ..building(square(at, side / 2.0), Vec::new())
        }
    }

    #[test]
    fn an_empty_neighbourhood_changes_nothing() {
        let districts = Districts::new(&[]);
        assert_eq!(districts.fill_at(Vec2::ZERO), 1.0);
    }

    #[test]
    fn a_private_sector_parks_far_thinner_than_a_microdistrict() {
        let houses: Vec<PolyArea> = (0..20)
            .map(|i| house(Vec2::new(i as f32 * 20.0, 0.0), 10.0, 3.2))
            .collect();
        let slabs: Vec<PolyArea> = (0..6)
            .map(|i| house(Vec2::new(i as f32 * 40.0, 0.0), 28.0, 27.0))
            .collect();
        assert_eq!(
            Districts::new(&houses).fill_at(Vec2::new(200.0, 0.0)),
            LOW_FILL
        );
        assert_eq!(
            Districts::new(&slabs).fill_at(Vec2::new(100.0, 0.0)),
            HIGH_FILL
        );
    }

    #[test]
    fn a_lone_slab_outweighs_the_garages_around_it() {
        // два десятка гаражей числом больше секции, но пятном меньше:
        // взвешивание по пятну заведено ровно ради этого случая
        let mut scene: Vec<PolyArea> = (0..20)
            .map(|i| house(Vec2::new(i as f32 * 8.0, 40.0), 6.0, 3.0))
            .collect();
        scene.push(house(Vec2::new(80.0, 0.0), 30.0, 27.0));
        let storeys = Districts::new(&scene)
            .storeys_at(Vec2::new(80.0, 20.0))
            .expect("дома рядом есть");
        assert!(storeys > HIGH_STOREYS, "{storeys} этажей");
    }

    #[test]
    fn the_reading_is_a_smooth_ramp_between_the_two() {
        let mid: Vec<PolyArea> = (0..4)
            .map(|i| house(Vec2::new(i as f32 * 30.0, 0.0), 20.0, 10.5))
            .collect();
        let fill = Districts::new(&mid).fill_at(Vec2::new(45.0, 0.0));
        assert!(
            fill > LOW_FILL && fill < HIGH_FILL,
            "{fill} — середина шкалы"
        );
    }

    #[test]
    fn a_house_far_outside_the_reach_is_not_counted() {
        let scene = vec![
            house(Vec2::ZERO, 10.0, 3.0),
            house(Vec2::new(REACH * 2.5, 0.0), 40.0, 30.0),
        ];
        let districts = Districts::new(&scene);
        assert_eq!(districts.fill_at(Vec2::ZERO), LOW_FILL);
    }
}
