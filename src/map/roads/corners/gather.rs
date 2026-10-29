use std::f32::consts::PI;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::map::along::{arclengths, place_on_path};
use crate::map::meshing::{butt_direction, ribbon_merge_distance, ribbon_vertices};
use crate::map::osm::RoadClass;
use crate::map::osm::model::polyline_length;
use crate::map::spawn::GRASS_COLOR;

use super::{
    ALLEY_COLOR, Arm, Drawn, FarTaper, KERB_PAD_RUN, KerbLayers, MAX_ANGLE, MIN_ANGLE, MIN_ARM,
    Meadows, PROBE_STEP, ROAD_COLOR, SIDEWALK_COLOR, STRAIGHT_TOLERANCE, Scraps,
    UNPAVED_ROAD_COLOR, VERGE_PAVED_MAX, VERGE_YARD_COLOR, YardBounds, bend, fillet, is_nose,
    kerb_pad, kerb_radius, node_key, nose, nose_radius, obtuse_corner, outer_corner, rounded,
    tile_overlaps,
};

/// Слой, в который ложится нос ([`KerbReturns::noses`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Road(RoadClass),
    Unpaved,
    Sidewalk,
}

/// Асфальт всех узлов. Укладка — методом модуля ([`Self::lay`]): порядок
/// слоёв — инвариант `corners`, а не вызывающего. Читатели геометрии берут
/// представления ([`Self::paving`], [`Self::streets`]), счётчики —
/// [`Self::counts`], клинья развилок — [`Self::fork_gores`]; поля закрыты
/// внутрь модуля и его тестов.
#[derive(Default)]
pub struct KerbReturns {
    /// Носы острых развилок ([`NOSE_SHARE`]) и слой каждого. Нос не выпукл и
    /// веером не кладётся: его стороны идут по кромкам лент, а они у острой
    /// развилки гнутые, — контур триангулируется целиком.
    pub(super) noses: Vec<(Fill, Vec<Vec2>)>,
    /// Изогнутые скругления ([`bend`]) и слой каждого: у кольца и гнутого
    /// подхода кромка кривая, контур не выпукл — триангулируется целиком.
    pub(super) bends: Vec<(Fill, Vec<Vec2>)>,
    /// Штрихуемые клинья перед носами острых развилок улиц ([`fork_gore`]):
    /// контур от острия клина до дуги носа — слою островков
    /// (`roads/gores.rs`).
    pub(super) fork_gores: Vec<Vec<Vec2>>,
    /// Контур и класс дорог, в чей слой заливки он ляжет: скругления и
    /// наружные углы. Каждый — веер из первой вершины.
    pub(super) roads: Vec<(RoadClass, Vec<Vec2>)>,
    /// Скругления между двумя грунтовыми улицами — в их слое, так же.
    pub(super) unpaved: Vec<Vec<Vec2>>,
    /// Контуры в слое тротуаров, так же: углы полос тротуара и скругления
    /// узлов с мощёной дорожкой.
    pub(super) sidewalks: Vec<Vec<Vec2>>,
    /// Наезды плитки на дворе у слияний мощёных дорожек ([`tile_overlaps`]):
    /// полоса вдоль кромки луча, за которой двор. Рисуются в слое тротуаров
    /// рядом с `sidewalks`, но лоскуты газона (`roads/scraps.rs`) ими не
    /// кормятся: наезд — сам починка, а не мощение, замыкающее лоскут.
    pub(super) overlaps: Vec<Vec<Vec2>>,
    /// Контуры в слое обочин (`road_verges`), так же: углы, где с одной
    /// стороны или с обеих вместо полосы тротуара обочина.
    pub(super) verges: Vec<Vec<Vec2>>,
    /// Те же углы между двумя **широкими** обочинами — газоном
    /// (`road_verge_lawns` или `road_verge_yards`), как и сами обочины: плиткой угол во все
    /// пятнадцать метров до дорожек был площадью посреди двора.
    pub(super) verge_lawns: Vec<Vec<Vec2>>,
    /// Сколько из `roads` и `sidewalks` — наружные углы, а не скругления.
    pub(super) outer: [usize; 2],
    /// Торцы, кончающиеся в узле, по дорогам: `[начало, конец]` — ленты с
    /// таким торцом кладутся с прямым, а не круглым.
    pub(super) butt: Vec<[bool; 2]>,
    /// На сколько метров не доходит до узла заливка торца `[начало, конец]`:
    /// у асфальтовой улицы, упёршейся в грунтовку, — полуширина грунтовки.
    pub(super) setback: Vec<[f32; 2]>,
}

impl KerbReturns {
    /// Торцы дороги `road`; вне узлов (или без скруглений вовсе) — круглые.
    pub fn butt(&self, road: usize) -> [bool; 2] {
        self.butt.get(road).copied().unwrap_or_default()
    }

    /// Недоход заливки дороги `road` до узла у каждого торца, м.
    pub fn setback(&self, road: usize) -> [f32; 2] {
        self.setback.get(road).copied().unwrap_or_default()
    }

    /// Укладка асфальта узлов в слои `layers` — порядок здесь, в модуле:
    /// сначала углы газонов уходят лоскутам (`scraps`) — мощение вокруг
    /// режет их потом, — затем скругления ложатся веером в свои слои (улицы,
    /// грунтовые улицы, тротуары с наездами плитки, обочины, газоны широких
    /// обочин), и последними носы и дуги, триангуляцией целиком. Ленты дорог
    /// кладутся поверх скруглений (`build_roads`), и этот порядок — половина
    /// той картинки. `meadows` решает, каким газоном ляжет угол широкой
    /// обочины: лугом у замапленного газона, травой двора — везде ещё.
    pub(in crate::map::roads) fn lay(
        &self,
        layers: KerbLayers<'_>,
        meadows: &Meadows,
        scraps: &mut Scraps,
    ) {
        let KerbLayers {
            streets,
            alleys,
            unpaved,
            sidewalks,
            verges,
            lawns,
            yards,
        } = layers;
        // углы газонов — лоскутам до всего мощения: лоскут режется по мощению
        for outline in &self.verge_lawns {
            scraps.push_lawn(outline.clone());
        }
        for (class, outline) in &self.roads {
            let (builder, color) = match class {
                RoadClass::Street => (&mut *streets, ROAD_COLOR),
                RoadClass::Alley => (&mut *alleys, ALLEY_COLOR),
            };
            // Скругление не выпукло, но веер из его первой вершины — угла краёв —
            // верен: дуга между точками касания и есть та часть окружности, что
            // видна из угла. `earcutr` на восьми тысячах таких фигур стоил бы
            // больше самой укладки.
            builder.push_convex(outline, color.to_linear());
        }
        for outline in &self.unpaved {
            unpaved.push_convex(outline, UNPAVED_ROAD_COLOR.to_linear());
        }
        // и тот же угол в слое тротуаров: полоса поворачивает за бордюром
        for outline in &self.sidewalks {
            sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        // и наезды плитки на дворе у слияний мощёных дорожек — тем же слоем
        for outline in &self.overlaps {
            sidewalks.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        // и в слое обочин — где вместо полосы обочина до отдельной дорожки
        for outline in &self.verges {
            verges.push_convex(outline, SIDEWALK_COLOR.to_linear());
        }
        for outline in &self.verge_lawns {
            if meadows.under(outline) {
                lawns.push_convex(outline, GRASS_COLOR.to_linear());
            } else {
                yards.push_convex(outline, VERGE_YARD_COLOR.to_linear());
            }
        }
        // носы острых развилок идут по гнутым кромкам лент, и веер из острия
        // их не покрыл бы — триангуляция целиком; носов в городе сотни
        for (fill, outline) in self.noses.iter().chain(&self.bends) {
            let (builder, color) = match fill {
                Fill::Road(RoadClass::Street) => (&mut *streets, ROAD_COLOR),
                Fill::Road(RoadClass::Alley) => (&mut *alleys, ALLEY_COLOR),
                Fill::Unpaved => (&mut *unpaved, UNPAVED_ROAD_COLOR),
                Fill::Sidewalk => (&mut *sidewalks, SIDEWALK_COLOR),
            };
            builder.push_polygon(outline, &[], color.to_linear());
        }
    }

    /// Асфальт узлов, которым режутся лоскуты газона (`roads/scraps.rs`):
    /// улицы, тротуары, обочины и мощёные носы с дугами. Наружу — из
    /// [`Self::paving`]. Наезды плитки ([`Self::overlaps`]) сюда не входят:
    /// наезд — сам починка, а не мощение, замыкающее лоскут.
    pub(in crate::map::roads) fn paving(&self) -> KerbPaving<'_> {
        KerbPaving {
            roads: &self.roads,
            sidewalks: &self.sidewalks,
            verges: &self.verges,
            noses: &self.noses,
            bends: &self.bends,
        }
    }

    /// Асфальт улиц узлов — скругления и носы в слое улиц: пустота, которую
    /// он уже замостил, — просто асфальт перекрёстка, а не пустое устье
    /// (горла штриховки, `gores::mouths`). Наружу — из [`Self::streets`].
    pub(in crate::map::roads) fn streets(&self) -> KerbStreets<'_> {
        KerbStreets {
            roads: &self.roads,
            noses: &self.noses,
            bends: &self.bends,
        }
    }

    /// Что скругления насчитали — для отчёта сборки (`roads::RoadReport`):
    /// группы в счётчиках, наружные углы — отдельно от скруглений.
    pub(in crate::map::roads) fn counts(&self) -> KerbCounts {
        KerbCounts {
            kerb_returns: self.roads.len() + self.unpaved.len() - self.outer[0],
            sidewalk_returns: self.sidewalks.len() - self.outer[1],
            tile_overlaps: self.overlaps.len(),
            bends: self.bends.len(),
            outer_corners: self.outer,
            noses: self.noses.len(),
        }
    }

    /// Штрихуемые клинья перед носами острых развилок улиц ([`fork_gore`])
    /// — `gores::Gores::add_forks`: асфальт под клином уже лежит (лента и
    /// нос развилки), штрихуется только контур.
    pub(in crate::map::roads) fn fork_gores(&self) -> &[Vec<Vec2>] {
        &self.fork_gores
    }

    /// Что скругления насчитали вокруг точки — диагностике тестов
    /// (`roads/tests.rs`, `QWE_DIAG`): группы как есть, по строке на контур.
    #[cfg(test)]
    pub(in crate::map::roads) fn dump_near(&self, at: Vec2, reach: f32) {
        let near = |outline: &Vec<Vec2>| outline.iter().any(|p| p.distance(at) < reach);
        for (class, outline) in self.roads.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr roads {class:?} {outline:?}");
        }
        for (fill, outline) in self.bends.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr bend {fill:?} {outline:?}");
        }
        for (fill, outline) in self.noses.iter().filter(|(_, outline)| near(outline)) {
            eprintln!("DIAG kr nose {fill:?} {outline:?}");
        }
        for outline in self.fork_gores.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr gore {outline:?}");
        }
        for outline in self.sidewalks.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr sidewalk {outline:?}");
        }
        for outline in self.verges.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr verge {outline:?}");
        }
        for outline in self.verge_lawns.iter().filter(|outline| near(outline)) {
            eprintln!("DIAG kr lawn {outline:?}");
        }
    }
}

/// Что скругления насчитали — поля `RoadReport` о них (`roads.rs`):
/// скругления без наружных углов, наезды плитки, носы, дуги, наружные углы.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::map::roads) struct KerbCounts {
    /// Скругления асфальта улиц и грунтовых — без наружных углов.
    pub(in crate::map::roads) kerb_returns: usize,
    /// Скругления тротуаров — без наружных углов.
    pub(in crate::map::roads) sidewalk_returns: usize,
    /// Наезды плитки на дворе у слияний мощёных дорожек.
    pub(in crate::map::roads) tile_overlaps: usize,
    /// Изогнутые скругления — у колец и гнутых подходов.
    pub(in crate::map::roads) bends: usize,
    /// Наружные углы — асфальт улиц и тротуар.
    pub(in crate::map::roads) outer_corners: [usize; 2],
    /// Носы острых развилок, всех слоёв.
    pub(in crate::map::roads) noses: usize,
}

/// Мощение вокруг лоскутов газона — представление [`KerbReturns`] для
/// `roads/scraps.rs`: контуры, коими лоскут режется. Плоские группы и носы с
/// дугами раздельны — между ними вызывающий кладёт своё мощение (горки), и
/// порядок вклада в `Scraps` сохраняется.
#[derive(Clone, Copy)]
pub(in crate::map::roads) struct KerbPaving<'a> {
    /// Скругления улиц и аллей; мощение — только улицы.
    roads: &'a [(RoadClass, Vec<Vec2>)],
    sidewalks: &'a [Vec<Vec2>],
    verges: &'a [Vec<Vec2>],
    noses: &'a [(Fill, Vec<Vec2>)],
    bends: &'a [(Fill, Vec<Vec2>)],
}

impl<'a> KerbPaving<'a> {
    /// Плоское мощение: скругления улиц, тротуары, обочины — веером.
    pub(in crate::map::roads) fn flat(self) -> impl Iterator<Item = &'a [Vec2]> {
        self.roads
            .iter()
            .filter(|(class, _)| *class == RoadClass::Street)
            .map(|(_, outline)| outline.as_slice())
            .chain(self.sidewalks.iter().map(Vec::as_slice))
            .chain(self.verges.iter().map(Vec::as_slice))
    }

    /// Мощёные носы и дуги — улиц и тротуаров: грунт и аллеи лоскут не режут.
    pub(in crate::map::roads) fn curved(self) -> impl Iterator<Item = &'a [Vec2]> {
        [self.noses, self.bends]
            .into_iter()
            .flatten()
            .filter(|(fill, _)| matches!(fill, Fill::Road(RoadClass::Street) | Fill::Sidewalk))
            .map(|(_, outline)| outline.as_slice())
    }
}

/// Асфальт улиц узлов — представление [`KerbReturns`] для горл штриховки
/// (`gores::mouths`): только слой улиц, без тротуаров и грунтовых.
#[derive(Clone, Copy)]
pub(in crate::map::roads) struct KerbStreets<'a> {
    roads: &'a [(RoadClass, Vec<Vec2>)],
    noses: &'a [(Fill, Vec<Vec2>)],
    bends: &'a [(Fill, Vec<Vec2>)],
}

impl<'a> KerbStreets<'a> {
    /// Контуры асфальта улиц: скругления, носы и дуги.
    pub(in crate::map::roads) fn outlines(self) -> impl Iterator<Item = &'a [Vec2]> {
        self.roads
            .iter()
            .filter(|(class, _)| *class == RoadClass::Street)
            .map(|(_, outline)| outline.as_slice())
            .chain(
                [self.noses, self.bends]
                    .into_iter()
                    .flatten()
                    .filter(|(fill, _)| matches!(fill, Fill::Road(RoadClass::Street)))
                    .map(|(_, outline)| outline.as_slice()),
            )
    }
}

/// Асфальт всех узлов: скругления проезжей части и тротуаров, наружные углы и
/// прямые торцы плеч.
///
/// Всё — по подготовленным дорогам `drawn` (`roads/drawn.rs`): **узловые**
/// осевые (`Axis::Nodal` — после сглаживания, до стежков; мост и арка не
/// участвуют, [`rounded`]); тротуар по стороне, если он рисуется
/// (`Drawn::sidewalk_on`); лежит ли рядом вторая половина разделённой
/// улицы и слева ли (`Pairs::beside`, `roads/network/pairs.rs`) — с её
/// стороны тротуара нет, и угол по нему не скругляется; клинья у торцов
/// (`Drawn::taper_ends`, `roads/tapers.rs`) — в клине кромка уже ближе к
/// оси, и прямой пробег луча с другого конца кончается там, где клин
/// начинается, а торец под клином стоит в узле на полуширине узкого соседа с
/// сужаемых сторон — по ней и считаются его углы; торец плечо слияния
/// (`Drawn::is_merged`, `roads/merges.rs`) — плечи одного слияния друг другу
/// не перекрёсток, ни прямых торцов, ни углов между ними. `scale` —
/// множитель радиусов по классам (ручка `Corner radius`).
pub fn kerb_returns(drawn: &Drawn, scale: f32) -> KerbReturns {
    let roads = drawn.roads();
    let nodes = drawn.nodes();
    let mut arms: HashMap<(i32, i32), (Vec2, Vec<Arm>)> = HashMap::new();
    for (index, &road) in roads.iter().enumerate() {
        let Some(path) = rounded(drawn, index) else {
            continue;
        };
        if path.len() < 2 {
            continue;
        }
        let wedges = drawn.taper_ends(index);
        let [head, tail] = wedges.map(|wedge| wedge.map_or(0.0, |wedge| wedge.length));
        let total = polyline_length(path);
        // клинья так, как их нарежет лента
        let fitted = super::tapers::fit(total, [head, tail].map(Some));
        let closed = path[0] == path[path.len() - 1];
        let last = path.len() - 1;
        // вершины, которые оставит лента (`meshing::ribbon_vertices`): край
        // ленты прямой между ними, а не между вершинами оси
        let drawn_vertices = if closed || fitted == [None; 2] {
            ribbon_vertices(path, closed, ribbon_merge_distance(road.width))
        } else {
            let ends = [0, 1].map(|end| Some((fitted[end]?, roads[wedges[end]?.seam].width)));
            wedged_vertices(path, total, ends, road.width)
        };
        let mut along = 0.0;
        for (vertex, &node) in path.iter().enumerate() {
            if vertex > 0 {
                along += node.distance(path[vertex - 1]);
            }
            if closed && vertex == last {
                continue;
            }
            if !nodes.is_shared(node) {
                continue;
            }
            let entry = arms
                .entry(node_key(node))
                .or_insert_with(|| (node, Vec::new()));
            // вперёд по пути и назад; у замкнутого кольца — через шов (его
            // последняя вершина повторяет первую и пропущена выше)
            let ring = last;
            let step = |from: usize, forward: bool| {
                if closed {
                    Some(if forward {
                        (from + 1) % ring
                    } else {
                        (from + ring - 1) % ring
                    })
                } else if forward {
                    (from < last).then_some(from + 1)
                } else {
                    from.checked_sub(1)
                }
            };
            // Луч меряется по вершинам **ленты**: она сливает точки оси ближе
            // четверти своей ширины, и край идёт прямо до следующей
            // оставленной. Направление на слитую вершину расходилось с краем
            // ленты — у широкой улицы на градусы, и сторона скругления
            // отходила от края светлой щелью (Тула, Сойфера × Лейтейзена);
            // прямой край, продлённый через слитую вершину на изгибе оси,
            // выводил дугу за кромку асфальтовым язычком (Халтурина ×
            // Гоголевская). Узел, который лента сама слила, — по вершинам оси.
            let on_ribbon = |index: usize| drawn_vertices[index] || !drawn_vertices[vertex];
            for forward in [true, false] {
                let mut at = vertex;
                let mut next = None;
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    at = index;
                    if on_ribbon(index) && path[index].distance(node) >= MIN_ARM {
                        next = Some(path[index]);
                        break;
                    }
                }
                let Some(next) = next else {
                    continue;
                };
                let direction = (next - node).normalize();
                // край идёт прямо и через вершины, лежащие на той же прямой: OSM
                // ставит их где угодно — узел пересечения с тротуаром в двух
                // метрах от улицы, излом в полградуса, — и дуга, обрезанная по
                // первой из них, не ложилась совсем
                let mut run = next.distance(node);
                while let Some(index) = step(at, forward) {
                    if index == vertex {
                        break;
                    }
                    at = index;
                    if !on_ribbon(index) {
                        continue;
                    }
                    let offset = path[index] - node;
                    let along = offset.dot(direction);
                    if along <= run || direction.perp_dot(offset).abs() > STRAIGHT_TOLERANCE {
                        break;
                    }
                    run = along;
                }
                // первый общий узел по оси, докуда может дотянуться хвост
                // площадки у бордюра: за ним дорогу пересекает дорожка или
                // проезд
                let mut crossed = f32::INFINITY;
                let (mut at, mut reach) = (vertex, 0.0);
                while let Some(index) = step(at, forward) {
                    if index == vertex || reach > run + KERB_PAD_RUN {
                        break;
                    }
                    reach += path[index].distance(path[at]);
                    at = index;
                    if nodes.is_shared(path[index]) {
                        crossed = reach;
                        break;
                    }
                }
                let at_end = !closed && (vertex == 0 || vertex == last);
                // Клин, под которым стоит узел: у торца — свой, даже не
                // легший; дальше — тот, на чьей длине узел лежит. Узел
                // примыкания бывает и внутри клина (Тула, Сойфера у Фёдора
                // Смирнова: в 14 м от шва на клине в 33 м) — там кромка уже
                // не своя и не соседа, а посередине, и скругление, построенное
                // на своей полуширине, стояло за кромкой, открывая клин
                // тротуара в проезжей части. `(торец, клин, длина, от шва)`
                let under = (!closed)
                    .then(|| {
                        [0, 1].into_iter().find_map(|end| {
                            let wedge = wedges[end]?;
                            let from_seam = if end == 0 { along } else { total - along };
                            match fitted[end] {
                                Some(length) if from_seam < length => {
                                    Some((end, wedge, length, from_seam))
                                }
                                None if at_end && vertex == end * last => {
                                    Some((end, wedge, 0.0, 0.0))
                                }
                                _ => None,
                            }
                        })
                    })
                    .flatten();
                // Кромка прямая только до клина: дальше лента сужается, и
                // касательная, заведённая в клин, торчала из-под него шипом
                // асфальта и тротуара (пример 08, улица в 9 м с клином к
                // однополосной). В клине — до его конца или до шва. Длина
                // клина — подогнанная: сырая бывает длиннее самой дороги, и
                // капа по ней обнуляла прямой пробег всех лучей дороги —
                // углам у узлов не хватало кромки, и асфальтовый угол устья
                // торчал шипом (Тула, съезд с Фрунзе в стоянку, N2).
                let [head_fitted, tail_fitted] = fitted.map(|length| length.unwrap_or(0.0));
                if !closed {
                    let straight = match under {
                        Some((end, _, length, from_seam)) if length > 0.0 => {
                            if forward == (end == 0) {
                                length - from_seam
                            } else {
                                from_seam
                            }
                        }
                        _ if forward => total - tail_fitted - along,
                        _ => along - head_fitted,
                    };
                    run = run.min(straight.max(0.0));
                }
                // стороны луча: слева по пути — слева по лучу вперёд и справа
                // по лучу назад; тротуар — по тегу со своей стороны пути
                let mut sides =
                    [0, 1].map(|at| drawn.sidewalk_on(index, usize::from((at == 0) != forward)));
                // под клином: с сужаемых сторон кромка в узле — между узким
                // соседом и своей, по месту на клине (у шва — соседа, и
                // тротуар там тоже его), и идёт наклонно — от шва к своей
                // полуширине, к шву — к соседской; с сохранённой — своя
                let own = road.width / 2.0;
                let (mut half, mut reach, mut slope) = ([own; 2], [own; 2], [0.0; 2]);
                let mut widening = 0.0;
                if let Some((end, wedge, length, from_seam)) = under {
                    let away = forward == (end == 0);
                    // клин не лёг — кромка сразу своя
                    widening = match (length > 0.0, away) {
                        (false, _) => f32::MIN_POSITIVE,
                        (true, true) => length - from_seam,
                        (true, false) => from_seam,
                    };
                    let seam = roads[wedge.seam];
                    let seam_half = seam.width / 2.0;
                    let share = if length > 0.0 {
                        from_seam / length
                    } else {
                        0.0
                    };
                    for (side, tapered) in wedge.sides.into_iter().enumerate() {
                        if !tapered {
                            continue;
                        }
                        let at = usize::from((side == 0) != forward);
                        half[at] = seam_half + (own - seam_half) * share;
                        reach[at] = if away { own } else { seam_half };
                        if length > 0.0 && widening > 0.0 {
                            slope[at] = (reach[at] - half[at]) / widening;
                        }
                        if from_seam == 0.0 {
                            sides[at] = drawn.sidewalk_on(wedge.seam, side);
                        }
                    }
                }
                // кусок пары кончается там, где пробы перестали её находить:
                // до узла — не дальше двух проб
                if let Some(left) = drawn.pairs().beside(index, along, 2.0 * PROBE_STEP) {
                    sides[usize::from(left != forward)] = None;
                }
                // обочины — слева и справа по пути, как тротуар по тегу; у
                // узла — какая она там по месту (`RoadLine::verge_at`)
                let mut verge = [None; 2];
                let scale = polyline_length(&road.points) / total.max(f32::EPSILON);
                for (side, width) in drawn.verges_drawn(index).into_iter().enumerate() {
                    verge[usize::from((side == 0) != forward)] =
                        (width > 0.0).then(|| road.verge_at(side, along * scale));
                }
                // клин дальше по лучу: узел вне его (под узлом — `under`, и
                // клин перед узлом она же), кромка за узлом сужается к
                // дальнему шву way. Стороны клина — по ходу точек way, лучу —
                // по его ходу
                let far = (!closed).then(|| {
                    let end = usize::from(forward);
                    let wedge = wedges[end]?;
                    let length = fitted[end]?;
                    let to_seam = if forward { total - along } else { along };
                    (to_seam > length).then(|| FarTaper {
                        start: to_seam - length,
                        length,
                        half: roads[wedge.seam].width / 2.0,
                        sides: [
                            wedge.sides[usize::from(!forward)],
                            wedge.sides[usize::from(forward)],
                        ],
                    })
                });
                entry.1.push(Arm {
                    class: road.class,
                    highway: road.highway,
                    ring: drawn.on_ring(index),
                    paved: road.is_paved_path(),
                    unpaved: road.is_unpaved_street(),
                    half,
                    sidewalk: sides,
                    verge,
                    direction,
                    run,
                    crossed,
                    end: at_end.then_some((index, usize::from(vertex == last))),
                    butt: at_end
                        .then(|| {
                            // у конца ось кончается в узле: звено смотрит на
                            // узел, лучу нужно от него
                            butt_direction(path, vertex == last)
                                .map(|dir| if vertex == last { -dir } else { dir })
                        })
                        .flatten(),
                    far: far.flatten(),
                    full: own,
                    reach,
                    widening,
                    slope,
                    path,
                    vertex,
                    forward,
                });
            }
        }
    }

    let mut returns = KerbReturns {
        butt: vec![[false; 2]; roads.len()],
        setback: vec![[0.0; 2]; roads.len()],
        ..default()
    };
    let is_merged = |arm: &Arm| {
        arm.end
            .is_some_and(|(road, end)| drawn.is_merged(road, end))
    };
    // между двумя плечами слияния нет ни угла, ни скругления
    let merge_pair = |first: &Arm, second: &Arm| is_merged(first) && is_merged(second);
    // рамки дворов — для проб наезда плитки у каждого узла ниже
    let yards = YardBounds::of(drawn.yards());
    for (node, mut found) in arms.into_values() {
        if found.len() < 2 {
            continue;
        }
        found.sort_by(|a, b| a.direction.to_angle().total_cmp(&b.direction.to_angle()));
        for class in [RoadClass::Street, RoadClass::Alley] {
            let group: Vec<&Arm> = found.iter().filter(|arm| arm.class == class).collect();
            // Асфальт, продолженный грунтовкой, обрывается поперёк: круглый
            // торец ленты лежал на грунте полукругом. Оба торца прямые, а
            // щель с наружной стороны излома закрывает грунт — под асфальтом.
            if let [first, second] = group[..]
                && first.unpaved != second.unpaved
                && !is_junction(&group)
                && let (Some((a, a_end)), Some((b, b_end))) = (first.end, second.end)
                && !is_merged(first)
                && !is_merged(second)
            {
                returns.butt[a][a_end] = true;
                returns.butt[b][b_end] = true;
                for (first, second) in pairs(&group) {
                    let halves = (first.half[0], second.half[1]);
                    if let Some(outline) = outer_corner(node, first, second, halves) {
                        returns.unpaved.push(outline);
                    }
                }
                continue;
            }
            // узел одного слияния — продолжение дороги, а не перекрёсток
            if !is_junction(&group) || group.iter().all(|arm| is_merged(arm)) {
                continue;
            }
            // Прямой срез плеча аллеи оправдан только там, где рядом ляжет
            // уголковый кусок — то есть у самого плеча нет встречного и хоть
            // один сосед тоже кончается (пару проходных лучей кусок не
            // получает, см. пропуск ниже). Иначе срезу ничем не прикрыться, и
            // у слияния дорожек он торчал клином и ступенькой на стыке лент
            // разной ширины (Тула, Фрунзе × Красноармейский). Круглая шапка
            // ложится на соседние ленты — тот же слой, тот же цвет, — и
            // слияние читается пересечением полос. Исключение — широкий
            // торец: полудиск шапки радиусом в полуширину луча ложится на
            // узел поверх чужих лент, и там, где чужая лента уже, он
            // выступает за её кромку — клин на газоне и светлая плита
            // первого штриха зебры на соседней ленте (Тула, витрина 02:
            // зебра шире тротуаров в узле; Первомайская × проспект Ленина,
            // юг: зебра шире изгибаемого тротуара). Срез кладёт торец в
            // плоскость узла — выступать нечему, а узкие соседи закрыты
            // своими лентами.
            let squared = |arm: &Arm| {
                arm.end.is_some()
                    && group.iter().any(|other| {
                        !std::ptr::eq(*other, arm)
                            && -arm.direction.dot(other.direction) > 0.0
                            && other.half[0].min(other.half[1]) < arm.half[0].max(arm.half[1])
                    })
                    || (!continues(&group, arm)
                        && group
                            .iter()
                            .any(|other| !std::ptr::eq(*other, arm) && !continues(&group, other)))
            };
            for arm in &group {
                if class == RoadClass::Alley && !squared(arm) {
                    continue;
                }
                if let Some((road, end)) = arm.end.filter(|_| !is_merged(arm)) {
                    returns.butt[road][end] = true;
                }
            }
            // Асфальт, упёршийся в грунтовку, кончается на её кромке: лента
            // до узла лежала поверх грунта языком до оси грунтовки, с двумя
            // уступами там, где её торец шире скруглений (Тула, 13). Только
            // когда асфальтовое плечо в узле одно — асфальт, пересекающий
            // грунтовку, идёт через неё.
            let paved: Vec<&&Arm> = group.iter().filter(|arm| !arm.unpaved).collect();
            let dirt = group.iter().filter(|arm| arm.unpaved).map(|arm| arm.full);
            if class == RoadClass::Street
                && let [arm] = paved[..]
                && let Some((road, end)) = arm.end.filter(|_| !is_merged(arm))
                && group.len() - paved.len() >= 2
            {
                returns.setback[road][end] = dirt.fold(0.0, f32::max);
            }
            for (first, second) in pairs(&group) {
                if merge_pair(first, second) {
                    continue;
                }
                // Дорожка проходит узел насквозь — у луча есть встречный: крест
                // или устье примыкания, и ленты аллей на узлах не режутся, так
                // что угол и так кроется самим стыком. Кусок был бы заплаткой
                // поверх стыка — ступени, клинья, швы (Тула, Первомайская ×
                // проспект Ленина). Остаётся угол двух лучей, кончающихся в
                // узле, — L-поворот, развилка, — там правда есть вырез.
                if class == RoadClass::Alley
                    && (continues(&group, first) || continues(&group, second))
                {
                    continue;
                }
                let radius = kerb_radius(first, second) * scale;
                let halves = (first.half[0], second.half[1]);
                // угол двух мощёных дорожек — плиткой, в слое тротуаров:
                // песчаное скругление на стыке двух плиточных аллей читалось бы
                // пятном. Одна мощёная, вторая грунтовая — песком под мощёной
                // лентой, как у улиц грунтовка под асфальтом
                let paved = first.paved && second.paved;
                // угол двух грунтовок — грунтом; грунтовки с асфальтом сюда
                // доходит только наружным или носом — тоже грунтом
                let fill = if paved {
                    Fill::Sidewalk
                } else if first.unpaved || second.unpaved {
                    Fill::Unpaved
                } else {
                    Fill::Road(class)
                };
                // Грунтовка входит в асфальт без скругления: асфальт идёт
                // прямо, и его кромка — край грунтовки (Калуга, 07). Асфальтовый
                // веер у её устья читался отводом от асфальтовой улицы. Всё,
                // что между ними всё же закрывается, — грунтом, под асфальтом.
                let mixed = first.unpaved != second.unpaved;
                let round = if mixed {
                    None
                } else {
                    // у обычного перекрёстка кромка дуги — сама линия бордюра
                    // между прямыми краями двух улиц: ленты заканчиваются у узла
                    // прямыми торцами, и кроме куска угол не кроет никто. Общий
                    // шаг читался гранями (Тула, Фёдора Смирнова ×
                    // Красноармейский, 3399 3835)
                    fillet(node, first, second, halves, radius)
                };
                // прямая кромка короче касательной — дуга на гнутые кромки
                if round.is_none()
                    && !mixed
                    && let Some(outline) = bend(first, second, halves, radius)
                {
                    returns.bends.push((fill, outline));
                    continue;
                }
                // тупой угол со сменой ширины — клин широкой кромки
                let round = round.or_else(|| {
                    (!mixed)
                        .then(|| obtuse_corner(node, first, second, halves))
                        .flatten()
                });
                let (outline, outer) = match round {
                    Some(outline) => (outline, false),
                    None if is_nose(first, second) => {
                        let radius = nose_radius(radius, scale);
                        // клин штрихуется только на развилке — узел из трёх
                        // лучей, острый угол, а не нос несложившегося
                        // скругления — и между проезжими частями улиц, а не
                        // у проезда, дорожки или грунтовки. На сложном узле
                        // клинья легли островками посреди его асфальта (Орёл,
                        // витрина 04), у развилки под 34° — клочком в горле
                        // узла (Тула, 06)
                        let gored = fill == Fill::Road(RoadClass::Street)
                            && group.len() == 3
                            && ccw_angle(first, second) < MIN_ANGLE
                            && [first, second]
                                .iter()
                                .all(|arm| arm.highway.is_street() && !arm.ring);
                        if let Some(found) = nose(first, second, (0.0, 0.0), radius, gored) {
                            returns.noses.push((fill, found.outline));
                            returns.fork_gores.extend(found.gore);
                        }
                        continue;
                    }
                    None => match outer_corner(node, first, second, halves) {
                        Some(outline) => (outline, true),
                        None => continue,
                    },
                };
                match fill {
                    Fill::Sidewalk => returns.sidewalks.push(outline),
                    Fill::Unpaved => returns.unpaved.push(outline),
                    Fill::Road(class) => returns.roads.push((class, outline)),
                }
                returns.outer[usize::from(paved)] += usize::from(outer);
            }
            // Асфальт, идущий сквозь грунтовку двумя way с изломом: торцы
            // прямые, скруглений к грунту нет, и щель с наружной стороны
            // излома светилась грунтом тонкой чертой поперёк асфальта (Калуга,
            // 06: шов участков с `lane_markings=no` на переезде гравийки).
            // Закрывает её тот же наружный веер, что у продолжения дороги.
            let paved: Vec<&Arm> = group.iter().copied().filter(|arm| !arm.unpaved).collect();
            if paved.len() < group.len()
                && paved.len() == 2
                && !is_junction(&paved)
                && paved.iter().all(|arm| arm.end.is_some() && !is_merged(arm))
            {
                for (first, second) in pairs(&paved) {
                    let halves = (first.half[0], second.half[1]);
                    if let Some(outline) = outer_corner(node, first, second, halves) {
                        returns.roads.push((class, outline));
                        returns.outer[0] += 1;
                    }
                }
            }
        }
        // Наезд плитки на дворе у слияния мощёных дорожек (R37-01, пример 15):
        // две плиточные полосы (например, переход через улицу и дорожка двора)
        // сходятся в узле, а клин двора квартала входит остриём в щель между
        // их кромками — секторы между лучами закрывают скругления, но полосу
        // двора вдоль кромки снаружи не закрывает ничто. Наезд — полоса плитки
        // вдоль кромки каждого луча, за которой вплотную двор: кромки
        // «наезжают» на него у стыка, и шов смыкается. Сторона, где другой
        // луч ближе [`TILE_OVERLAP_OCCUPIED`], занята угловыми правилами и не
        // трогается; без двора вплотную наезда нет — полосы не раздулись.
        let tiled: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Alley && arm.paved)
            .collect();
        // проходной узел — крест дорожек — наездов не просит: полосы легли бы
        // поверх стыка кромок прямоугольниками, а не поверх щели слияния
        // (Тула, Первомайская × проспект Ленина). У примыкания или слияния
        // хоть один луч в узле кончается — там щель у двора настоящая
        let alleys: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Alley)
            .collect();
        if tiled.len() >= 2
            && !tiled.iter().all(|arm| is_merged(arm))
            && !alleys.iter().all(|arm| continues(&alleys, arm))
        {
            returns.overlaps.extend(tile_overlaps(node, &tiled, &yards));
        }
        // Угол, где хоть с одной стороны вместо полосы тротуара обочина до
        // отдельной дорожки, — той же дугой, но в слой обочин под зеленью:
        // без него между концом полосы (или обочины) одной улицы, обочиной
        // другой и бордюрной дугой оставался клин голой земли (Тула, 15).
        let verged: Vec<&Arm> = found
            .iter()
            .filter(|arm| (0..2).any(|side| arm.sidewalk[side].or(arm.verge[side]).is_some()))
            .collect();
        if is_junction(&verged) && !verged.iter().all(|arm| is_merged(arm)) {
            for (first, second) in pairs(&verged) {
                if merge_pair(first, second)
                    || (first.verge[0].is_none() && second.verge[1].is_none())
                {
                    continue;
                }
                // обочина тянется дальше полосы — угол по ней, где она есть
                let (Some(a), Some(b)) = (
                    first.verge[0].or(first.sidewalk[0]),
                    second.verge[1].or(second.sidewalk[1]),
                ) else {
                    continue;
                };
                // дуга — по узкой из двух: по широкой (как у полос тротуара)
                // она уходила от бордюра, и вдоль узкой оставался клин земли
                let halves = (first.half[0] + a, second.half[1] + b);
                let radius = kerb_radius(first, second) * scale - a.min(b);
                if let Some(outline) = fillet(node, first, second, halves, radius)
                    .or_else(|| obtuse_corner(node, first, second, halves))
                    .or_else(|| outer_corner(node, first, second, halves))
                {
                    if a.min(b) > VERGE_PAVED_MAX {
                        returns.verge_lawns.push(outline);
                    } else {
                        returns.verges.push(outline);
                    }
                }
                // у газона угол, куда выходят зебры, — площадкой плитки вдоль
                // бордюрной дуги, иначе переход кончался на траве серпом
                // между двумя газонами (Тула, 01)
                if a.max(b) > VERGE_PAVED_MAX {
                    let road = (first.half[0], second.half[1]);
                    let radius = kerb_radius(first, second) * scale;
                    returns
                        .verges
                        .extend(kerb_pad(node, first, second, road, radius));
                }
            }
        }
        // Тротуары — свои соседи: проезд без тротуара не рвёт полосу улицы,
        // через которую он выходит, и угол считается между улицами по обе
        // стороны от него.
        let walked: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.sidewalk.iter().any(Option::is_some))
            .collect();
        // две полосы одной улицы сквозь узел, где к ней примыкает улица без
        // тротуара: сами по себе они продолжение, но торцы у них прямые —
        // узел улиц перекрёсток, — и без угла между ними через тротуар шла
        // нить (Орёл, витрина 04, север)
        let streets: Vec<&Arm> = found
            .iter()
            .filter(|arm| arm.class == RoadClass::Street)
            .collect();
        let butted = walked.len() == 2
            && walked.iter().all(|arm| arm.end.is_some())
            && is_junction(&streets);
        if !(is_junction(&walked) || butted) || walked.iter().all(|arm| is_merged(arm)) {
            continue;
        }
        for (first, second) in pairs(&walked) {
            if merge_pair(first, second) {
                continue;
            }
            // угол от левого края первого луча к правому краю второго
            let (Some(a), Some(b)) = (first.sidewalk[0], second.sidewalk[1]) else {
                continue;
            };
            // тот же центр, что у бордюрной дуги: радиус меньше на тротуар,
            // полоса шире на него же. Берётся больший из двух тротуаров — при
            // разной их ширине одной дугой обе полосы не обойти, а меньший
            // радиус оставляет асфальтовый клин внутри тротуарного
            let halves = (first.half[0] + a, second.half[1] + b);
            let radius = kerb_radius(first, second) * scale - a.max(b);
            if let Some(outline) = fillet(node, first, second, halves, radius) {
                returns.sidewalks.push(outline);
            } else if let Some(outline) = bend(first, second, halves, radius) {
                returns.bends.push((Fill::Sidewalk, outline));
            } else if let Some(outline) = obtuse_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
            } else if is_nose(first, second) {
                // У носа концентричная дуга ушла бы в минус — остриё островка
                // всё мощёное, — и нос тротуара свой, того же малого радиуса:
                // иначе между полосами тротуара торчал бы шип земли.
                let radius = nose_radius(kerb_radius(first, second) * scale, scale);
                if let Some(found) = nose(first, second, (a, b), radius, false) {
                    returns.noses.push((Fill::Sidewalk, found.outline));
                }
            } else if let Some(outline) = outer_corner(node, first, second, halves) {
                returns.sidewalks.push(outline);
                returns.outer[1] += 1;
            }
        }
    }
    returns
}

/// Вершины разомкнутой оси `path` (длиной `total`), которые оставит лента
/// дороги шириной `width` с клиньями `ends` (`[у начала, у конца]`: длина,
/// как её нарежет лента, и ширина узкого соседа). Лента кладётся кусками
/// (`roads.rs`, `tapers::split`): тело сливает точки на
/// [`ribbon_merge_distance`] своей ширины, клин — на половине узкой
/// полуширины (`MeshBuilder::push_taper_sided`) и от шва; каждый кусок — со
/// своих концов. Вершина на стыке кусков остаётся.
fn wedged_vertices(
    path: &[Vec2],
    total: f32,
    ends: [Option<(f32, f32)>; 2],
    width: f32,
) -> Vec<bool> {
    let (along, _) = arclengths(path);
    let mut kept = vec![false; path.len()];
    let [head, tail] = ends.map(|end| end.map_or(0.0, |(length, _)| length));
    let mut pieces = vec![(head, total - tail, false, ribbon_merge_distance(width))];
    if let Some((length, narrow)) = ends[0] {
        pieces.push((0.0, length, false, narrow / 4.0));
    }
    if let Some((length, narrow)) = ends[1] {
        pieces.push((total - length, total, true, narrow / 4.0));
    }
    for (from, to, reversed, distance) in pieces {
        // точки куска и чьи они: вершина оси или точка разреза
        let mut points: Vec<(Vec2, Option<usize>)> = Vec::new();
        let place = |at: f32| place_on_path(path, &along, at).map(|(point, _)| point);
        let vertex_at = |at: f32| along.iter().position(|&value| value == at);
        match vertex_at(from) {
            Some(index) => points.push((path[index], Some(index))),
            None => points.extend(place(from).map(|point| (point, None))),
        }
        for (index, &at) in along.iter().enumerate() {
            if at > from && at < to {
                points.push((path[index], Some(index)));
            }
        }
        match vertex_at(to) {
            Some(index) => points.push((path[index], Some(index))),
            None => points.extend(place(to).map(|point| (point, None))),
        }
        if reversed {
            points.reverse();
        }
        let line: Vec<Vec2> = points.iter().map(|&(point, _)| point).collect();
        for ((_, index), keep) in points.iter().zip(ribbon_vertices(&line, false, distance)) {
            if let Some(index) = *index {
                kept[index] |= keep;
            }
        }
    }
    kept
}

/// Луч проходит узел насквозь: в группе есть встречный ему луч — направления
/// расходятся от прямого не дальше [`MAX_ANGLE`]. Не по [`Arm::end`]: OSM режет
/// дорожку на два way прямо в узле креста, и оба «кончаются», хотя дорога идёт
/// насквозь. Соосный шов двух way одной дорожки — тоже продолжение.
fn continues(group: &[&Arm], arm: &Arm) -> bool {
    group.iter().any(|other| {
        !std::ptr::eq(*other, arm) && arm.direction.angle_to(other.direction).abs() >= MAX_ANGLE
    })
}

/// Узел ли это для группы лучей одного класса: три плеча и больше или два,
/// сходящиеся углом. Два почти соосных плеча — продолжение дороги (другой
/// класс, другой way), и острые — развилка без третьего плеча: там торцы
/// остаются круглыми.
fn is_junction(group: &[&Arm]) -> bool {
    match group {
        [] | [_] => false,
        [first, second] => {
            let angle = ccw_angle(first, second);
            let turn = |angle: f32| (MIN_ANGLE..=MAX_ANGLE).contains(&angle);
            turn(angle) || turn(2.0 * PI - angle)
        }
        _ => true,
    }
}

/// Угол от луча `first` против часовой стрелки до луча `second`, (0, 2π].
pub(super) fn ccw_angle(first: &Arm, second: &Arm) -> f32 {
    ccw_between(first.direction, second.direction)
}

/// Угол от направления `from` против часовой стрелки до `to`, (0, 2π].
pub(super) fn ccw_between(from: Vec2, to: Vec2) -> f32 {
    let angle = to.to_angle() - from.to_angle();
    if angle <= 0.0 {
        angle + 2.0 * PI
    } else {
        angle
    }
}

/// Соседние по углу пары лучей группы — по кругу, чтобы последний луч встретил
/// первый. Меньше двух лучей — пар нет.
fn pairs<'a, 'd>(group: &'a [&'a Arm<'d>]) -> impl Iterator<Item = (&'a Arm<'d>, &'a Arm<'d>)> {
    let count = if group.len() < 2 { 0 } else { group.len() };
    (0..count).map(move |index| (group[index], group[(index + 1) % group.len()]))
}
