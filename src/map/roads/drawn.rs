//! **Drawn** — дороги карты так, как они рисуются: переезды через тротуар
//! асфальтом проезда и дуги колец сечением всего кольца, общие узлы, оси улиц,
//! стежки, клинья, слияния. Строится один раз на [`mesh_roads`](super::mesh_roads)
//! и по облегчённому рецепту ([`Drawn::nodal`]) — на слой машин (`map::cars`).
//!
//! Все векторы — по индексу в `map.roads`, длины равны (`assert` в сборке).
//! Ни одна правка `RoadLine::points` не трогает: навмеш, двери и деревья видят
//! OSM как есть. Поля закрыты: что потребителю нужно, он спрашивает — и в
//! первую очередь **какую ось** он берёт ([`Axis`]).

use std::borrow::Cow;

use bevy::prelude::*;

use super::axis::{self, Axes};
use super::merges::{self, Merges};
use super::network::pairs::{PROBE_STEP, Pairs, Partner};
use super::network::{self, RoadNodes, Stitches};
use super::pockets::KerbLots;
use super::rings::Rings;
use super::shape::RoadShape;
use super::tapers::{Taper, Tapers};
use super::{MEDIAN_CROSSING_MAX, RoadStyle, drawn_sidewalk, ring_arcs};
use crate::map::osm::model::polyline_length;
use crate::map::osm::{MapData, RoadClass, RoadLine};

/// Какую ось берёт потребитель — решение вынесено из порядка `let` в тип.
///
/// У дороги три оси. Точки OSM (`RoadLine::points`) — для всего, что ищет
/// узел по ключу (`junctions::node_key`): базовые разрывы, клинья, разрывы
/// ряда, сид кармана; их `Drawn` не подменяет и не отдаёт — они у самой
/// `RoadLine`. Две другие — здесь.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// После сглаживания, до стежков — **узловое**: скругления, слияния,
    /// карманы, станции штрихов, полотно трамвая, фронтаж стоянок, ряд машин.
    /// Торцы стоят в точках OSM, и узел по ключу на них ещё находится.
    Nodal,
    /// Со стежками — **лента**: асфальт, краска, траектории, острова, нос
    /// слияния. Стежок довёл торец до чужой оси, и ключа узла у него нет.
    Ribbon,
}

/// Подготовленные дороги — одно значение вместо локалов в начале
/// `mesh_roads`. Поля закрыты: подмодули берут [`Drawn::roads`],
/// [`Drawn::axes`] и запросы ниже.
pub struct Drawn<'m> {
    /// Дороги как рисуются, по индексу `map.roads`: подмены на своих местах
    /// (переезд через тротуар — улицей шириной у́жего проезда,
    /// `network::driveway_crossings`; дуга кольца — сечением всего кольца,
    /// [`ring_arcs`]), прочие — заимствованы из карты.
    roads: Vec<Cow<'m, RoadLine>>,
    /// Сколько подмен легло (переезд и дуга на одной дороге — две).
    crossings: usize,
    /// Общие узлы дорог любого класса — по точкам OSM.
    nodes: RoadNodes,
    /// Оси улиц (`roads/axis.rs`), пары половин и кольца.
    axes: Axes<'m>,
    /// Стежки висячих торцов (`network::stitches`) — по дорогам как рисуются.
    stitches: Stitches,
    /// Ось со стежками — только у дорог, которых стежок коснулся; прочие
    /// рисуются по своей оси ([`Axis::Ribbon`]).
    stitched: Vec<Option<Vec<Vec2>>>,
    /// Клинья между сечениями улиц (`roads/tapers.rs`) — **единственный**
    /// расчёт на слой: карманы и ряд машин берут его же.
    tapers: Tapers,
    /// Слияния разделённой улицы в обычную (`roads/merges.rs`).
    merges: Merges,
    /// Кусок поперечной улицы в проёме разделительной — между половинами
    /// одной пары: тротуара он не несёт, его полоса светлым пятном лежала
    /// посреди перекрёстка.
    across_median: Vec<bool>,
    /// Ручка «Sidewalks»: рисуется ли тротуар вовсе ([`Self::sidewalk_drawn`]).
    sidewalks: bool,
    /// Дорога — дуга кольца (`roads/rings.rs`), по индексу.
    on_ring: Vec<bool>,
    /// Стоянки, перед которыми карман не нужен (`roads/pockets.rs`).
    lots: KerbLots<'m>,
}

/// Что подготовка нашла — вложенное поле `RoadReport::drawn`; строку лога
/// печатает `RoadReport`, как и прежде.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct DrawnStats {
    /// Подмены: переезды через тротуар и дуги колец.
    pub crossings: usize,
    pub stitches: usize,
    /// Швы ways, пройденные осью улицы одной кривой (`roads/axis.rs`).
    pub seams: usize,
    /// Изломы, на которые звеньев не хватило для радиуса в полуширину.
    pub tight: usize,
    /// Клинья между сечениями улиц (`roads/tapers.rs`).
    pub tapers: usize,
    /// Слияния разделённой улицы в обычную (`roads/merges.rs`).
    pub merges: usize,
    /// Разделительные парных половин (`roads/network/pairs.rs`): асфальтом,
    /// газоном и из асфальтовых — трамвайных полотен.
    pub medians: [usize; 3],
    /// Кольца, нарисованные гладкой фигурой (`roads/rings.rs`), и щели
    /// между подходом и кольцом, залитые асфальтом.
    pub rings: [usize; 2],
}

impl<'m> Drawn<'m> {
    /// Дороги для ленты: узловой каркас плюс стежки, слияния и куски в
    /// проёме пары. `style` — ручка тротуаров: стежок меряется до края
    /// тротуара, если он рисуется.
    pub fn new(map: &'m MapData, style: &RoadStyle, shape: &RoadShape) -> Self {
        let nodal = Self::nodal(map, shape);
        let roads = nodal.roads();
        let (paths, nodes, runs) = (&nodal.axes.paths, &nodal.nodes, &nodal.axes.pairs.runs);
        let stitches = network::stitches(&roads, map, nodes, |road| drawn_sidewalk(style, road));
        // Торцы узлов — точки OSM, и ось их не двигает.
        let across_median = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
                    return false;
                };
                polyline_length(path) < MEDIAN_CROSSING_MAX
                    && nodes.roads_at(start).iter().any(|&half| {
                        half != index
                            && runs[half].iter().any(|run| {
                                run.partner != index && nodes.roads_at(end).contains(&run.partner)
                            })
                    })
            })
            .collect();
        // разделённая улица, сходящаяся в обычную: узел не перекрёсток
        let merges = merges::merges(&roads, paths, nodes, runs, &map.network);
        // стежок до дороги, до которой OSM торец не довёл
        // (`roads/network/mod.rs`)
        let stitched = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                stitches.touches(index).then(|| {
                    let mut points = path.to_vec();
                    stitches.apply(index, &mut points);
                    points
                })
            })
            .collect();
        Self {
            stitches,
            stitched,
            merges,
            across_median,
            sidewalks: style.sidewalks,
            ..nodal
        }
    }

    /// Облегчённый каркас — узлы, оси, подмены, клинья, стоянки: то, по чему
    /// стоит ряд машин (`map::cars`). Без стежков и слияний: у ряда нет ни
    /// ленты, ни краски, которые они правят, а считать их — половина цены
    /// подготовки. [`Axis::Ribbon`] здесь совпадает с [`Axis::Nodal`].
    pub fn nodal(map: &'m MapData, shape: &RoadShape) -> Self {
        let osm = map.roads.as_slice();
        let nodes = RoadNodes::new(osm);
        // ось по улице целиком, не по way (`roads/axis.rs`); у переезда та же
        // ось, что у его дороги, — он отличается шириной и классом
        let axes = axis::street_axes(osm, &map.rails, &map.network, &nodes, shape);
        let mut roads: Vec<Cow<'m, RoadLine>> = osm.iter().map(Cow::Borrowed).collect();
        let mut crossings = 0;
        // порядок — порядок подмены: поздняя побеждает
        let substitutes = network::driveway_crossings(osm, &nodes)
            .into_iter()
            .map(|(index, width)| {
                let crossing = RoadLine {
                    class: RoadClass::Street,
                    width,
                    ..osm[index].clone()
                };
                (index, crossing)
            })
            .chain(ring_arcs(osm, &axes.rings));
        for (index, road) in substitutes {
            roads[index] = Cow::Owned(road);
            crossings += 1;
        }
        let count = roads.len();
        let drawn: Vec<&RoadLine> = roads.iter().map(Cow::as_ref).collect();
        let tapers = Tapers::new(&drawn, &map.network, &nodes, shape.taper());
        assert_eq!(axes.paths.len(), count, "ось на каждую дорогу карты");
        let on_ring = (0..count)
            .map(|road| axes.rings.of(road).is_some())
            .collect();
        Self {
            roads,
            crossings,
            nodes,
            axes,
            on_ring,
            stitches: Stitches {
                ends: vec![[None; 2]; count],
                targets: vec![[None; 2]; count],
                count: 0,
            },
            stitched: vec![None; count],
            tapers,
            merges: Merges::default(),
            across_median: vec![false; count],
            sidewalks: false,
            lots: KerbLots::new(&map.parking),
        }
    }

    /// Каркас теста: оси по точкам OSM (без сглаживания), тротуары
    /// рисуются. Без сети карты клиньев и слияний нет — их кладёт сам тест
    /// ([`Self::with_taper`], [`Self::with_pairs`], [`Self::with_ring`]).
    #[cfg(test)]
    pub fn for_test(map: &'m MapData) -> Self {
        let shape = RoadShape {
            curve_tolerance: 0.0,
            ..default()
        };
        Self::new(map, &RoadStyle::default(), &shape)
    }

    /// Ручка тротуаров теста.
    #[cfg(test)]
    pub fn with_sidewalks(mut self, sidewalks: bool) -> Self {
        self.sidewalks = sidewalks;
        self
    }

    /// Тот же узел как перекрёсток: найденные слияния забыты.
    #[cfg(test)]
    pub fn without_merges(mut self) -> Self {
        self.merges = Merges::default();
        self
    }

    /// Клин у торца `end` дороги `road` — как если бы его нашёл `Tapers`.
    #[cfg(test)]
    pub fn with_taper(mut self, road: usize, end: usize, taper: Taper) -> Self {
        self.tapers.set(road, end, taper);
        self
    }

    /// Куски пары на дороге `road` — вместо найденных `Pairs`.
    #[cfg(test)]
    pub fn with_pairs(mut self, road: usize, runs: Vec<super::network::pairs::PairRun>) -> Self {
        self.axes.pairs.runs[road] = runs;
        self
    }

    /// Дорога `road` — дуга кольца, каким бы ни был её контур.
    #[cfg(test)]
    pub fn with_ring(mut self, road: usize) -> Self {
        self.on_ring[road] = true;
        self
    }

    /// Дорог на карте — и в каждом векторе здесь.
    pub fn len(&self) -> usize {
        self.roads.len()
    }

    /// Дорога как рисуется: подмена, если легла, иначе дорога карты.
    pub fn road(&self, index: usize) -> &RoadLine {
        &self.roads[index]
    }

    /// Дороги как рисуются, по индексу `map.roads` — срез для подмодулей.
    pub fn roads(&self) -> Vec<&RoadLine> {
        self.roads.iter().map(Cow::as_ref).collect()
    }

    /// Общие узлы дорог — по точкам OSM.
    pub fn nodes(&self) -> &RoadNodes {
        &self.nodes
    }

    /// Ось дороги — какую, говорит потребитель.
    pub fn axis(&self, index: usize, which: Axis) -> &[Vec2] {
        match which {
            Axis::Nodal => &self.axes.paths[index],
            Axis::Ribbon => self.stitched[index]
                .as_deref()
                .unwrap_or(&self.axes.paths[index]),
        }
    }

    /// Оси всех дорог, по индексу `map.roads`, — срез для подмодулей.
    /// Заимствования: ось со стежком есть только у тронутых им дорог, у
    /// прочих лента идёт по узловой оси.
    pub fn axes(&self, which: Axis) -> Vec<Cow<'_, [Vec2]>> {
        (0..self.len())
            .map(|index| Cow::Borrowed(self.axis(index, which)))
            .collect()
    }

    /// Парные половины разделённых улиц и их разделительные.
    pub fn pairs(&self) -> &Pairs {
        &self.axes.pairs
    }

    /// Кольца, нарисованные гладкой фигурой.
    pub fn rings(&self) -> &Rings {
        &self.axes.rings
    }

    /// Дорога — дуга кольца (`roads/rings.rs`).
    pub fn on_ring(&self, index: usize) -> bool {
        self.on_ring[index]
    }

    /// Стежки — цели их узлы краски (`junctions::marking_breaks`).
    pub fn stitches(&self) -> &Stitches {
        &self.stitches
    }

    /// Пришит ли торец `[начало, конец]` дороги стежком.
    pub fn stitched_end(&self, index: usize) -> [bool; 2] {
        self.stitches.ends[index].map(|end| end.is_some())
    }

    /// Длина стежка перед началом узловой оси, м: куски пары меряны по оси
    /// без стежка, а лента идёт со стежком.
    pub fn stitch_offset(&self, index: usize) -> f32 {
        self.stitches.ends[index][0].map_or(0.0, |start| start.distance(self.axes.paths[index][0]))
    }

    /// Клинья между сечениями улиц — один расчёт на слой.
    pub fn tapers(&self) -> &Tapers {
        &self.tapers
    }

    /// Клинья у торцов дороги `[начало, конец]`.
    pub fn taper_ends(&self, index: usize) -> [Option<Taper>; 2] {
        self.tapers.at(index)
    }

    /// Слияния разделённой улицы в обычную.
    pub fn merges(&self) -> &Merges {
        &self.merges
    }

    /// Торец `end` дороги — плечо слияния (`roads/merges.rs`).
    pub fn is_merged(&self, index: usize, end: usize) -> bool {
        self.merges.is_merged(index, end)
    }

    /// Лежит ли на длине `at` по узловой оси рядом вторая половина
    /// разделённой улицы и слева ли она (`roads/network/pairs.rs`): с её
    /// стороны тротуара нет. Кусок пары может кончиться на пробу раньше
    /// узла — две пробы слака.
    pub fn paired(&self, index: usize, at: f32) -> Option<bool> {
        let slack = 2.0 * PROBE_STEP;
        self.axes.pairs.runs[index]
            .iter()
            .find(|run| run.from - slack <= at && at <= run.to + slack)
            .map(|run| run.left)
    }

    /// Вторые половины разделённой улицы у дороги — по её кускам пары.
    pub fn partners(&self, index: usize) -> impl Iterator<Item = Partner> + '_ {
        self.axes.pairs.runs[index].iter().map(|run| Partner {
            road: run.partner,
            paved: run.paved,
        })
    }

    /// Стоянки карты, перед которыми карман не нужен.
    pub fn lots(&self) -> &KerbLots<'m> {
        &self.lots
    }

    /// Тротуар, который у дороги **рисуется**: по карте и при этой ручке
    /// ([`drawn_sidewalk`]), минус кусок в проёме пары. Один ответ и для
    /// ленты, и для скругления в узле, и для кармана.
    pub fn sidewalk_drawn(&self, index: usize) -> Option<f32> {
        self.by_style(self.roads[index].sidewalk().any())
            .filter(|_| !self.across_median[index])
    }

    /// Тротуар, который у дороги есть **на карте**
    /// ([`SidewalkProfile::any`](crate::map::osm::model::SidewalkProfile::any)),
    /// ручка не смотрит: зебра по правилу — вопрос модели, как карман, а
    /// ручка только прячет ленту. Кусок в проёме пары — нет.
    pub fn sidewalk_mapped(&self, index: usize) -> Option<f32> {
        self.roads[index]
            .sidewalk()
            .any()
            .filter(|_| !self.across_median[index])
    }

    /// Полуширина полосы с тротуаром по стороне `[слева, справа]` по ходу
    /// точек: `width / 2` плюс тротуар, если он рисуется и стоит с этой
    /// стороны по тегу (`sidewalk=*`); иначе голая кромка. Считалась в
    /// шести местах — клин тротуара, скругления, кромки слияний, карманы.
    pub fn band_half(&self, index: usize, side: usize) -> f32 {
        let road = &self.roads[index];
        road.width / 2.0 + self.by_style(road.sidewalk().on(side)).unwrap_or(0.0)
    }

    /// Тротуар при ручке — без оглядки на проём пары.
    fn by_style(&self, sidewalk: Option<f32>) -> Option<f32> {
        sidewalk.filter(|_| self.sidewalks)
    }

    /// Счётчики подготовки для строки `road meshing:`.
    pub fn stats(&self) -> DrawnStats {
        DrawnStats {
            crossings: self.crossings,
            stitches: self.stitches.count,
            seams: self.axes.seams,
            tight: self.axes.tight,
            tapers: self.tapers.count,
            merges: self.merges.list.len(),
            medians: self.axes.pairs.count(),
            rings: [self.axes.rings.list.len(), self.axes.rings.webs.len()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::{SidewalkSide, fixture};
    use crate::map::roads::network::RoadNetwork;

    fn with_network(roads: Vec<RoadLine>) -> MapData {
        MapData {
            network: RoadNetwork::new(&roads),
            roads,
            ..default()
        }
    }

    #[test]
    fn every_vector_is_indexed_by_the_map_roads() {
        let map = with_network(vec![
            fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
            fixture::street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, 80.0)], 8.0),
        ]);
        let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
        assert_eq!(drawn.len(), 2);
        assert_eq!(drawn.roads().len(), 2);
        assert_eq!(drawn.axes(Axis::Nodal).len(), 2);
        assert_eq!(drawn.axes(Axis::Ribbon).len(), 2);
        assert_eq!(drawn.stats().crossings, 0);
    }

    #[test]
    fn a_driveway_crossing_is_drawn_as_a_street_of_the_narrower_width() {
        // дорожка 3.5 м пересекает тротуар улицы между двумя кусками
        // проезда 5 м: рисуется улицей в 5 м, а на карте остаётся собой
        let footway = RoadLine {
            class: RoadClass::Alley,
            highway: crate::map::osm::Highway::Path,
            ..fixture::street(vec![Vec2::new(50.0, -10.0), Vec2::new(50.0, -18.0)], 3.5)
        };
        let map = with_network(vec![
            fixture::street(
                vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(100.0, 0.0)],
                12.0,
            ),
            fixture::street(vec![Vec2::new(50.0, 0.0), Vec2::new(50.0, -10.0)], 5.0),
            footway,
            fixture::street(vec![Vec2::new(50.0, -18.0), Vec2::new(50.0, -60.0)], 5.0),
        ]);
        let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
        assert_eq!(drawn.stats().crossings, 1);
        assert_eq!(drawn.road(2).class, RoadClass::Street);
        assert_eq!(drawn.road(2).width, 5.0);
        assert_eq!(map.roads[2].class, RoadClass::Alley);
        assert_eq!(map.roads[2].width, 3.5);
        let roads = drawn.roads();
        assert!(
            std::ptr::eq(roads[0], &map.roads[0]),
            "без подмены — дорога карты"
        );
    }

    #[test]
    fn stitched_axis_differs_from_nodal_only_at_stitched_ends() {
        // проезд кончается в трёх метрах за кромкой тротуара улицы
        // (`tests.rs::a_dangling_end_short_of_a_street_is_stitched`)
        let map = with_network(vec![
            fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 12.0),
            fixture::street(vec![Vec2::new(50.0, -60.0), Vec2::new(50.0, -9.0)], 5.0),
        ]);
        let drawn = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
        assert_eq!(drawn.stats().stitches, 1);
        assert_eq!(drawn.stitched_end(0), [false; 2]);
        assert_eq!(drawn.stitched_end(1), [false, true]);
        assert_eq!(drawn.axis(0, Axis::Nodal), drawn.axis(0, Axis::Ribbon));
        let (nodal, ribbon) = (drawn.axis(1, Axis::Nodal), drawn.axis(1, Axis::Ribbon));
        assert_eq!(ribbon.len(), nodal.len() + 1);
        assert_eq!(&ribbon[..nodal.len()], nodal);
        assert!(ribbon[ribbon.len() - 1].distance(Vec2::new(50.0, 0.0)) < 1e-3);
        assert_eq!(drawn.stitch_offset(1), 0.0, "стежок у конца, не у начала");
        // облегчённый каркас стежков не ищет
        let nodal = Drawn::nodal(&map, &RoadShape::default());
        assert_eq!(nodal.stats().stitches, 0);
        assert_eq!(nodal.axis(1, Axis::Nodal), nodal.axis(1, Axis::Ribbon));
    }

    #[test]
    fn band_half_takes_the_sidewalk_only_where_the_tag_puts_it() {
        let mut street = fixture::street(vec![Vec2::ZERO, Vec2::new(100.0, 0.0)], 8.0);
        street.sidewalks = [SidewalkSide::Tagged, SidewalkSide::None];
        let map = with_network(vec![street]);
        let drawn = Drawn::for_test(&map);
        let band = crate::map::osm::model::sidewalk_band(8.0);
        assert_eq!(drawn.sidewalk_drawn(0), Some(band));
        assert_eq!(drawn.sidewalk_mapped(0), Some(band));
        assert_eq!(drawn.band_half(0, 0), 4.0 + band);
        assert_eq!(
            drawn.band_half(0, 1),
            4.0,
            "справа тротуара нет — голая кромка"
        );
        // ручка прячет ленту, но не тротуар по карте
        let hidden = Drawn::for_test(&map).with_sidewalks(false);
        assert_eq!(hidden.sidewalk_drawn(0), None);
        assert_eq!(hidden.sidewalk_mapped(0), Some(band));
        assert_eq!(hidden.band_half(0, 0), 4.0);
    }
}
