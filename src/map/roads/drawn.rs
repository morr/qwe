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
use super::network::pairs::Pairs;
use super::network::{self, RoadNodes, Stitches};
use super::pockets::KerbLots;
use super::rings::Rings;
use super::rings::{leg_sections, ring_arcs};
use super::shape::RoadShape;
use super::tapers::{Taper, Tapers};
use super::verges::VERGE_MID_MAX;
use super::{RoadStyle, VergeZoomBucket};
use crate::map::osm::{MapData, PolyArea, RoadClass, RoadLine};
use crate::map::shapes::is_ring;
use crate::map::smooth::{Smoothing, smooth_pinned};

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
    /// Ступень зума обочин (`roads::VergeLods`): на средней и дальней широкая
    /// обочина не рисуется ([`Self::verges_drawn`]).
    verge_step: VergeZoomBucket,
    /// Дорога — дуга кольца (`roads/rings.rs`), по индексу.
    on_ring: Vec<bool>,
    /// Стоянки, перед которыми карман не нужен (`roads/pockets.rs`).
    lots: KerbLots<'m>,
    /// Дворы кварталов (`MapData::landuse`) — проба «двор вплотную к кромке»
    /// для наезда плитки на слияниях мощёных дорожек (`roads/corners.rs`).
    yards: &'m [PolyArea],
    /// Карманы разбора (`MapData::pockets`) — рисуются двором и газоном
    /// (`spawn::mesh_surfaces`), и наезд спрашивает их вместе с дворами.
    pockets: &'m [PolyArea],
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
    /// тротуара, если он рисуется. `verge_step` — ступень зума обочин:
    /// «какие обочины рисуются» — вопрос той же подготовки.
    pub fn new(
        map: &'m MapData,
        style: &RoadStyle,
        shape: &RoadShape,
        verge_step: VergeZoomBucket,
    ) -> Self {
        let nodal = Self::nodal(map, shape);
        let roads = nodal.roads();
        let (paths, nodes, pairs) = (&nodal.axes.paths, &nodal.nodes, &nodal.axes.pairs);
        // тротуар, который рисуется: по карте, если ручка его не прячет
        let stitches = network::stitches(&roads, map, nodes, |road| {
            road.sidewalk().any().filter(|_| style.sidewalks)
        });
        let across_median = paths
            .iter()
            .enumerate()
            .map(|(index, path)| pairs.across_median(index, path, nodes))
            .collect();
        // разделённая улица, сходящаяся в обычную: узел не перекрёсток
        let merges = merges::merges(&roads, paths, nodes, pairs, &map.network);
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
            verge_step,
            ..nodal
        }
    }

    /// Облегчённый каркас — узлы, оси, подмены, клинья, стоянки: то, по чему
    /// стоит ряд машин (`map::cars`). Без стежков и слияний: у ряда нет ни
    /// ленты, ни краски, которые они правят, а считать их — половина цены
    /// подготовки. [`Axis::Ribbon`] здесь совпадает с [`Axis::Nodal`].
    pub fn nodal(map: &'m MapData, shape: &RoadShape) -> Self {
        let osm = map.roads.as_slice();
        let mut nodes = RoadNodes::new(osm);
        // ось по улице целиком, не по way (`roads/axis.rs`); у переезда та же
        // ось, что у его дороги, — он отличается шириной и классом
        let axes = axis::street_axes(osm, &map.rails, &map.network, &mut nodes, shape);
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
            .chain(ring_arcs(osm, &axes.rings))
            .chain(leg_sections(osm, &axes.rings, map.knobs.lane_width));
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
            // каркас для ряда машин: обочин он не спрашивает, ступень —
            // ближняя, как у нарисованной улицы
            verge_step: VergeZoomBucket::for_zoom(0.0),
            lots: KerbLots::new(&map.parking),
            yards: &map.landuse,
            pockets: &map.pockets,
        }
    }

    /// Каркас теста: оси по точкам OSM (без сглаживания), тротуары
    /// рисуются, ступень обочин — ближняя. Без сети карты клиньев и
    /// слияний нет — их кладёт сам тест
    /// ([`Self::with_taper`], [`Self::with_pairs`], [`Self::with_ring`]).
    #[cfg(test)]
    pub fn for_test(map: &'m MapData) -> Self {
        let shape = RoadShape {
            curve_tolerance: 0.0,
            ..default()
        };
        Self::new(
            map,
            &RoadStyle::default(),
            &shape,
            VergeZoomBucket::for_zoom(0.0),
        )
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
        self.axes.pairs.set_runs(road, runs);
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

    /// Общие узлы дорог — по точкам OSM и по месту, куда их сдвинула разводка
    /// пар ([`RoadNodes::alias`]).
    pub fn nodes(&self) -> &RoadNodes {
        &self.nodes
    }

    /// Дворы кварталов (`MapData::landuse`) и карманы разбора
    /// (`MapData::pockets`) — рисуются двором (`spawn::mesh_surfaces`);
    /// проба «двор вплотную к кромке» у наезда плитки (`corners.rs`) спрашивает
    /// их вместе.
    pub fn yards(&self) -> impl Iterator<Item = &PolyArea> + '_ {
        self.yards.iter().chain(self.pockets)
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

    /// Стоянки карты, перед которыми карман не нужен.
    pub fn lots(&self) -> &KerbLots<'m> {
        &self.lots
    }

    /// Тротуар, который у дороги **рисуется**: по карте и при этой ручке
    /// ([`SidewalkProfile::any`](crate::map::osm::model::SidewalkProfile::any)
    /// при [`RoadStyle::sidewalks`]), минус кусок в проёме пары. Один ответ и для
    /// ленты, и для скругления в узле, и для кармана.
    pub fn sidewalk_drawn(&self, index: usize) -> Option<f32> {
        self.by_style(self.roads[index].sidewalk().any())
            .filter(|_| !self.across_median[index])
    }

    /// Тротуар, который рисуется **с этой стороны** `[слева, справа]` по ходу
    /// точек: [`Self::sidewalk_drawn`] там, где его ставит профиль
    /// ([`SidewalkProfile::sides`](crate::map::osm::model::SidewalkProfile::sides)).
    /// Карта, ручка, проём пары и сторона — одним ответом карману, скруглению
    /// в узле и кромке слияния. Сторону пары он не снимает: там её снимает
    /// сам потребитель по [`Pairs::beside`] — у скругления со слаком, у ленты
    /// кусками ([`Pairs::band_pieces`]).
    pub fn sidewalk_on(&self, index: usize, side: usize) -> Option<f32> {
        self.sidewalk_drawn(index)
            .filter(|_| self.roads[index].sidewalk().sides()[side])
    }

    /// Обочины дороги до отдельных тротуаров (`RoadLine::verges`), если
    /// тротуары рисуются; кусок в проёме пары — без них. На средней и
    /// дальней ступени зума ([`VergeZoomBucket`]) широкая обочина
    /// (шире `VERGE_MID_MAX`) не рисуется вовсе — ни лентой, ни углом,
    /// ни лоскутами: остаётся газон или земля, лежащие под ней. Один
    /// ответ и для ленты (`push_verges`), и для скругления в узле
    /// (`corners::kerb_returns`), поэтому ступень живёт здесь.
    pub fn verges_drawn(&self, index: usize) -> [f32; 2] {
        if !self.sidewalks || self.across_median[index] {
            return [0.0; 2];
        }
        let mut verges = self.roads[index].verges;
        if !self.verge_step.draws_wide_verges() {
            for (side, verge) in verges.iter_mut().enumerate() {
                if self.roads[index].verge_widest(side) > VERGE_MID_MAX {
                    *verge = 0.0;
                }
            }
        }
        verges
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
    /// Проём пары не смотрит, в отличие от [`Self::sidewalk_on`]: клин лежит
    /// на теле улицы, а не на куске между половинами.
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

/// Осевая дороги вне улицы: моста, арки, дорожки (улицы идут по
/// [`axis::street_axes`]). Без сглаживания — прямо точки OSM, без
/// копирования. Арки (`passage`) не сглаживаются никогда: их концы приколоты к
/// вершинам контура здания, по ним `arches::arch_openings` ищет проём в стене.
///
/// **Общие узлы с другими дорогами тоже не сглаживаются** ([`RoadNodes`]): на
/// узле кончается поперечная улица и сходятся лучи скругления бордюра
/// (`roads/corners.rs`). Сдвинь хорда сквозную дорогу с узла — торец
/// поперечной повис бы в метре от её асфальта или вылез за дальний край.
///
/// **Замкнутый way сглаживается по циклу**, так что нарисованная ось кольца
/// остаётся кольцом: её потом и рисуют замкнутой лентой ([`super::push_ribbon`],
/// [`super::push_street_fill`]).
pub(super) fn centerline<'a>(
    road: &'a RoadLine,
    smoothing: Smoothing,
    nodes: &RoadNodes,
) -> Cow<'a, [Vec2]> {
    if road.passage {
        return Cow::Borrowed(&road.points);
    }
    smooth_pinned(
        &road.points,
        road.width,
        smoothing,
        is_ring(&road.points),
        |point| nodes.is_shared(point),
    )
}

#[cfg(test)]
mod tests;
