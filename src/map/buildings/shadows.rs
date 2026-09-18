//! Тени зданий: развёртки силуэтов, наземный слой и тени, падающие на
//! кровли соседей.
//!
//! Развёртки считаются **один раз** на сборку слоя ([`ShadowSweeps`]) и
//! достаются обоим сборщикам — наземному ([`shadow_builder`]) и кровельному
//! ([`roof_shadow_builder`]), — потому что геометрия у них одна и та же, а
//! нужна в разной форме. Всё, что здесь лежит, рисуется в двух слоях и
//! больше нигде: `layers.rs` собирает фасады, кровли и экструзию, и общего у
//! них с тенями — только дом, от которого считают.
//!
//! Силуэт здесь — **цепочки** ([`silhouette_chains`]), а не отдельные рёбра:
//! квады на ребро у ступенчатого фасада перекрываются вдоль тени, и
//! полупрозрачность складывается в полосы двойной темноты. Рёбра силуэта по
//! одному (`layers::silhouette_edges`) — это про стены экструзии, не про
//! тень, и потому остались там.

use std::ops::RangeInclusive;

use bevy::prelude::*;

use super::arches::{arch_openings, arches_by_building};
use super::roofs::is_pitched;
use super::temples::Sanctuary;
use super::{BuildingHeightMode, Lean, extrusion_lift, height_or_default};
use crate::map::grid::Grid;
use crate::map::meshing::{MeshBuilder, sweep_convex};
use crate::map::osm::model::{ring_bounds, signed_ring_area};
use crate::map::osm::{BuildingUse, PolyArea, RoadLine};
use crate::map::shadow;
use crate::map::{SHADOW_COLOR, shadow_dir, sun_stretch};

/// Границы длины тени, м, **при дефолтном солнце**: у сарая тень обязана
/// остаться заметной, у башни — не накрыть полкарты. Сама длина считается из
/// высоты солнца (`map::shadow_length_scale`): пятиэтажка (15 м) отбрасывает
/// 9 м — тень перечёркивает типичную улицу (8–16 м), но не глотает соседний
/// квартал.
///
/// Границы едут за солнцем (`map::sun_stretch`), и это не украшение: числа
/// подобраны под `cot 59° = 0.6`, а на 15° масштаб 3.73, и неподвижный
/// потолок в 45 м уравнял бы по длине тени всё выше 12 м — верхние 10–15 %
/// домов Тулы, то есть ровно те кварталы, ради которых ползунок и уводят
/// вниз. На 80° неподвижный пол в 3 м так же съел бы разницу у всего ниже
/// 17 м.
pub(crate) const SHADOW_LENGTH_RANGE: RangeInclusive<f32> = 3.0..=45.0;

/// Ширина мягкого края тени, м. Не физическая полутень (угловой размер
/// солнца дал бы сантиметры), а то, чем край тени размыт на снимке:
/// разрешением кадра и светом неба. Метр — это 2–10 экранных пикселей на тех
/// зумах, где тени вообще видны.
const PENUMBRA_WIDTH: f32 = 1.0;
/// Насколько сосед обязан быть выше, чтобы его тень легла на кровлю, м. Ниже
/// этого тень попадает разве что на карниз, а считать её пришлось бы для
/// каждой пары домов одной этажности — то есть почти для всех.
const SHADOW_MIN_DROP: f32 = 3.0;
/// Ячейка сетки, по которой ищутся отбрасывающие соседи, м: чуть шире самой
/// длинной тени, какую даёт `SHADOW_LENGTH_RANGE` **при дефолтном солнце**.
/// Низкое солнце растягивает развёртку за пределы ячейки
/// (`map::sun_stretch`), и это стоит избирательности, а не правильности:
/// коробка развёртки лежит в каждой ячейке, которую задевает.
const SHADOW_CELL: f32 = 48.0;

/// Теневые развёртки всех домов карты — **считанные один раз** на сборку
/// слоя и поделённые между обоими теневыми сборщиками.
///
/// Развёртка дома — по свип-полигону на каждую непрерывную цепочку
/// рёбер-силуэта внешнего кольца: `[цепочка, цепочка + сдвиг в обратном
/// порядке]`, где сдвиг — высота дома через `shadow_length_scale()`, зажатая в
/// [`SHADOW_LENGTH_RANGE`] (обе границы едут за `sun_stretch()`). Не квады на
/// ребро: у ступенчатого фасада квады соседних ступеней перекрываются вдоль
/// тени, и полупрозрачность складывалась в полосы двойной темноты. Свип
/// цепочки самопересечься не может: перп-шаг ребра силуэта равен
/// `outward·shadow_dir() > 0`, то есть цепочка монотонна вдоль перпендикуляра
/// тени.
///
/// Это ровно то же вычисление, которое [`shadow_builder`] и
/// [`roof_shadow_builder`] делали каждый у себя, — два прохода по семи с
/// половиной тысячам домов ради одной и той же геометрии. Цена прохода на
/// Туле — 2 мс (`examples/bench/map_meshing`, ряд `sweeps`), и ровно на эти
/// 2 мс похудел каждый из двух теневых рядов.
///
/// Нужны они в **разной форме**: наземному слою — плоским списком на весь
/// город (он идёт одним `simplify_shape` в общее объединение), кровельному —
/// по домам (у каждого своя рамка, свои отбрасывающие соседи и своё
/// пересечение с контуром цели). Поэтому хранится плоский список, а
/// группировку несёт [`ShadowSweeps::spans`] — так наземный слой получает
/// готовый срез без склейки, а кровельный не платит за неё вовсе.
///
/// Порядок контуров в плоском списке — по домам и внутри дома по цепочкам,
/// тот же, что складывался раньше; обход каждого нормализован к CCW в
/// [`push_contour`].
pub(super) struct ShadowSweeps {
    contours: Vec<Vec<[f32; 2]>>,
    /// На дом — полуинтервал его контуров в `contours`.
    spans: Vec<(usize, usize)>,
}

impl ShadowSweeps {
    pub(super) fn of(buildings: &[PolyArea]) -> Self {
        let stretch = sun_stretch();
        let (min_length, max_length) = (
            *SHADOW_LENGTH_RANGE.start() * stretch,
            *SHADOW_LENGTH_RANGE.end() * stretch,
        );
        let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
        let mut spans = Vec::with_capacity(buildings.len());
        let sanctuary = Sanctuary::of(buildings);
        for (index, building) in buildings.iter().enumerate() {
            let start = contours.len();
            let length = shadow::length(height_or_default(building)).clamp(min_length, max_length);
            let offset = shadow_dir() * length;
            // у части на крыше храма и у отдельной колокольни коробки нет, и
            // тени коробки тоже: их тень — тень венца, она ниже
            let chains = match sanctuary.boxless(index, building) {
                true => Vec::new(),
                false => silhouette_chains(&building.outer, shadow_dir()),
            };
            for chain in chains {
                let mut sweep: Vec<Vec2> = chain.clone();
                sweep.extend(chain.iter().rev().map(|&point| point + offset));
                push_contour(&mut contours, sweep);
            }
            // глава и шпиль выше карниза, и тень храма обязана дотянуться до
            // маковки — иначе на земле он тот же коробок, что и сосед
            for (outline, top) in sanctuary.shadow_casters(index, building) {
                let length = shadow::length(top).clamp(min_length, max_length);
                push_contour(&mut contours, sweep_convex(&outline, shadow_dir() * length));
            }
            spans.push((start, contours.len()));
        }
        Self { contours, spans }
    }

    /// Все развёртки карты подряд — то, что уходит в объединение наземного
    /// слоя.
    fn all(&self) -> &[Vec<[f32; 2]>] {
        &self.contours
    }

    /// Развёртка одного дома.
    fn of_building(&self, index: usize) -> &[Vec<[f32; 2]>] {
        let (start, end) = self.spans[index];
        &self.contours[start..end]
    }
}

/// Тени зданий на земле. Развёртки приходят готовыми ([`ShadowSweeps`] — там
/// же, почему свип на цепочку силуэта, а не квад на ребро), и **все** свипы
/// карты объединяются булевым union (`i_overlay`) в набор
/// непересекающихся фигур с дырками: тени смежных корпусов и соседних зданий
/// перекрываются на земле, а любое наложение внутри одного полупрозрачного
/// слоя читается как пятно двойной темноты. После union альфа везде ровно
/// одна, и по контурам каждой фигуры идёт мягкий край (`PENUMBRA_WIDTH`).
/// Часть тени под зданиями закрывают их непрозрачные слои. Дыры футпринта
/// (дворы) в объединение не идут: их тень падает внутрь того же футпринта.
/// `extruded` — арки в 2.5D прорезаны по-настоящему, и сквозь дыру видна
/// голая дорога: без заплатки тени проём светится, хотя физически он затенён
/// перемычкой. Заплатка кладётся сюда, в теневой слой: он ниже зданий и
/// просвечивает ровно сквозь вырез.
pub(super) fn shadow_builder(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    sweeps: &ShadowSweeps,
    extruded: bool,
) -> MeshBuilder {
    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    shadow::push_union(&mut builder, sweeps.all(), PENUMBRA_WIDTH);

    if extruded {
        // по возрастанию номера дома, а не в порядке обхода `HashMap`: тот у
        // `std` перемешан случайным `RandomState`, и порядок вершин в
        // объединённом меше менялся бы от запуска к запуску. Два других
        // вызова `arches_by_building` берут дома по ключу и такой правки не
        // требуют
        let mut by_building: Vec<_> = arches_by_building(buildings, passages)
            .into_iter()
            .collect();
        by_building.sort_unstable_by_key(|&(index, _)| index);
        for (index, passages) in by_building {
            let building = &buildings[index];
            let lean = Lean::of();
            let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
            for opening in arch_openings(building, &passages, lift, -lean.dir()) {
                let Some(along) = (opening.b - opening.a).try_normalize() else {
                    continue;
                };
                let (p0, p1) = (
                    opening.a + along * opening.low,
                    opening.a + along * opening.high,
                );
                builder.push_quad([p0, p1, p1 + opening.sill, p0 + opening.sill], color);
            }
        }
    }
    builder
}

/// Тени, падающие **на кровли**: единственное место, где прежняя модель теней
/// прямо врала. Теневой слой лежит под всеми зданиевыми, поэтому
/// девятиэтажка не темнила крышу пятиэтажки под собой, и в плотном квартале
/// это видно сразу.
///
/// Считается ровно то, чего не хватало: пересечение теневой развёртки дома с
/// **контуром соседа, который ниже**. Ниже — потому что тень на крышу
/// **выше** отбрасывающего не попадает, а равные по высоте затеняют друг
/// друга разве что карнизом.
///
/// Порядок — по индексу дома, поэтому меш детерминирован.
///
/// Из готового пятна вычитаются **нарисованные тела** соседей, которых слой
/// экструзии рисует после цели ([`DrawnBodies`]). Слой лежит над всеми
/// зданиевыми, а painter's порядок 2.5D живёт **внутри одного меша**: без
/// вычитания тень, посчитанная для дальней кровли, легла бы тёмным пятном на
/// стену ближнего дома, который эту кровлю визуально закрывает.
///
/// `order` — тот самый список, которым [`super::layers::extrusion_builder`]
/// кладёт дома в меш, и он же признак 2.5D: строит его вызывающий, один раз
/// на сборку слоя, и отдаёт обоим. `None` — плоский режим: подъёма нет, дом
/// рисуется на своём контуре, и вычитать нечего.
///
/// Заливка — жёсткий `push_polygon`, без каймы `PENUMBRA_WIDTH`, которую несёт
/// наземная тень. Не забыто: часть контура пересечения — это не край тени, а
/// линия обреза по контуру кровли (`Intersect` с footprint), и растушёвка там
/// нарисовала бы светлый ободок по периметру каждой крыши.
pub(super) fn roof_shadow_builder(
    buildings: &[PolyArea],
    sweeps: &ShadowSweeps,
    order: Option<&[usize]>,
) -> MeshBuilder {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;

    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    let heights: Vec<f32> = buildings.iter().map(height_or_default).collect();
    let extruded = order.is_some();
    let boxes: Vec<(Vec2, Vec2)> = buildings.iter().map(|b| ring_bounds(&b.outer)).collect();
    let sweep_boxes: Vec<(Vec2, Vec2)> = (0..buildings.len())
        .map(|index| {
            let points: Vec<Vec2> = sweeps
                .of_building(index)
                .iter()
                .flatten()
                .map(|point| Vec2::from_array(*point))
                .collect();
            ring_bounds(&points)
        })
        .collect();

    // сетка по развёрткам: тень длиной до 45 м, домов семь с половиной тысяч,
    // и перебор пар был бы пятьюдесятью миллионами проверок. Детерминизму меша
    // сетка не мешает: `Grid::near` отвечает отсортированным списком без
    // повторов, так что обход её `HashMap` наружу не протекает
    let mut cells: Grid<usize> = Grid::new(SHADOW_CELL);
    for (index, &(min, max)) in sweep_boxes.iter().enumerate() {
        cells.insert(min, max, index);
    }
    // тела соседей по той же сетке: они не отбрасывают тень, а съедают её
    let bodies = DrawnBodies::of(buildings, &boxes, order);
    let sanctuary = Sanctuary::of(buildings);

    for (target, building) in buildings.iter().enumerate() {
        // у части на крыше храма и у отдельной колокольни кровли нет —
        // нарисован только венец, — и тень, посчитанная на поднятый контур
        // части, висела над собором клином
        if sanctuary.boxless(target, building) {
            continue;
        }
        // скатная кровля — не плоскость на высоте карниза, на которую этот слой
        // кладёт тень: в 2.5D скаты поднимаются к коньку, и плоская заплата
        // съезжала с них тёмным прямоугольником поперёк ската. Частный дом
        // стал одноэтажным, двухэтажный сосед теперь выше него на те самые
        // `SHADOW_MIN_DROP`, и заплата легла на половину частного сектора
        if is_pitched(building) {
            continue;
        }
        let (min, max) = boxes[target];
        let lift = if extruded {
            extrusion_lift(building, BuildingHeightMode::Extrusion)
        } else {
            Vec2::ZERO
        };
        // двор в контур дома не входит: без обратного обхода дырки NonZero
        // насчитал бы внутри неё обмотку ±2 и залил бы двор тенью
        let mut footprint: Vec<Vec<[f32; 2]>> = Vec::new();
        push_contour(&mut footprint, building.outer.clone());
        for hole in &building.holes {
            push_hole(&mut footprint, hole.clone());
        }

        let mut casters = cells.near(min, max);
        casters.retain(|&caster| {
            caster != target
                && heights[caster] - heights[target] >= SHADOW_MIN_DROP
                && boxes_overlap((min, max), sweep_boxes[caster])
                && !same_church(&buildings[caster], building)
        });
        if casters.is_empty() {
            continue;
        }

        let cast: Vec<Vec<[f32; 2]>> = casters
            .into_iter()
            .flat_map(|caster| sweeps.of_building(caster).iter().cloned())
            .collect();
        // объединение развёрток и пересечение с контуром — за один вызов:
        // NonZero склеивает перекрывающиеся тени, а Intersect обрезает их по
        // дому. Без склейки две тени на одной крыше дали бы двойную темноту
        let mut shapes = cast.overlay(&footprint, OverlayRule::Intersect, FillRule::NonZero);
        if shapes.is_empty() {
            continue;
        }
        // в нарисованное пространство: тень ложится туда, где кровля
        // нарисована, а не туда, где лежит её футпринт
        for shape in &mut shapes {
            for contour in shape.iter_mut() {
                for point in contour.iter_mut() {
                    *point = [point[0] + lift.x, point[1] + lift.y];
                }
            }
        }

        // и вычесть тела соседей, которые рисуются после цели: слой лежит над
        // всеми зданиевыми, а painter's порядок 2.5D живёт внутри одного меша
        let covers = bodies.covering(buildings, target, (min + lift, max + lift));
        if !covers.is_empty() {
            let flat: Vec<Vec<[f32; 2]>> = shapes.into_iter().flatten().collect();
            shapes = flat.overlay(&covers, OverlayRule::Difference, FillRule::NonZero);
        }

        for shape in shapes {
            let mut rings = shape.into_iter().map(|contour| {
                contour
                    .into_iter()
                    .map(Vec2::from_array)
                    .collect::<Vec<Vec2>>()
            });
            let Some(outer) = rings.next() else {
                continue;
            };
            let holes: Vec<Vec<Vec2>> = rings.collect();
            builder.push_polygon(&outer, &holes, color);
        }
    }
    builder
}

/// Части одного храма ([`crate::map::osm::Sacred::complex`]) — одно здание.
///
/// Слой теней на кровлях лежит **над** слоем зданий, а собор в OSM — это
/// перекрывающиеся контуры: колокольня, барабаны и пристройки отбрасывали тень
/// на кровлю своего же собора, и она ложилась поверх его глав и барабанов
/// полупрозрачными клиньями. Внутри одного храма кровельной тени нет.
fn same_church(a: &PolyArea, b: &PolyArea) -> bool {
    matches!(
        (a.building_use, b.building_use),
        (BuildingUse::Church(a), BuildingUse::Church(b)) if a.complex != 0 && a.complex == b.complex
    )
}

/// Пересекаются ли два AABB (`ring_bounds`): касание считается пересечением —
/// префильтр обязан ошибаться в сторону «да».
fn boxes_overlap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
    a.0.x <= b.1.x && b.0.x <= a.1.x && a.0.y <= b.1.y && b.0.y <= a.1.y
}

/// Нарисованные тела домов в 2.5D — то, чем сосед закрывает чужую кровлю.
///
/// Тело дома — сумма Минковского его контура с отрезком подъёма `[0, lift]`:
/// снизу настоящий контур, сверху поднятый, между ними свипы силуэтных
/// цепочек по направлению подъёма. Ровно то пятно, в котором
/// [`super::layers::extrusion_builder`] рисует стены и крышу этого дома.
///
/// В плоских режимах пусто: подъёма нет, дом рисуется на своём контуре, и
/// накрыть кровлю соседа ему нечем.
struct DrawnBodies {
    lifts: Vec<Vec2>,
    /// Место дома в [`super::order::draw_order`]: больше — рисуется позже, поверх. Не
    /// `Lean::depth` центра: одним числом на дом отношение «кто кого кроет»
    /// не выражается (см. модуль [`super::order`]), а спрашивается здесь
    /// ровно оно.
    rank: Vec<usize>,
    boxes: Vec<(Vec2, Vec2)>,
    /// Номера тел по ячейкам их рамок — те же номера, которыми индексируются
    /// [`Self::lifts`], [`Self::rank`] и [`Self::boxes`].
    bodies_by_cell: Grid<usize>,
}

impl DrawnBodies {
    /// Пусто — в плоских режимах. Не `Default`: у сетки нет осмысленного
    /// значения по умолчанию, размер ячейки обязателен (`map/grid.rs`), и
    /// ровно это она и стережёт.
    fn empty() -> Self {
        Self {
            lifts: Vec::new(),
            rank: Vec::new(),
            boxes: Vec::new(),
            bodies_by_cell: Grid::new(SHADOW_CELL),
        }
    }

    fn of(buildings: &[PolyArea], boxes: &[(Vec2, Vec2)], order: Option<&[usize]>) -> Self {
        // порядок есть ровно в 2.5D: в плоских режимах его никто не строит, и
        // накрывать кровлю соседа там нечем
        let Some(order) = order else {
            return Self::empty();
        };
        let lifts: Vec<Vec2> = buildings
            .iter()
            .map(|building| extrusion_lift(building, BuildingHeightMode::Extrusion))
            .collect();
        // тот самый список, которым `extrusion_builder` кладёт дома в меш, —
        // он строится один раз на сборку слоя и достаётся обоим
        let mut rank = vec![0usize; buildings.len()];
        for (place, &index) in order.iter().enumerate() {
            rank[index] = place;
        }
        let boxes: Vec<(Vec2, Vec2)> = boxes
            .iter()
            .zip(&lifts)
            .map(|(&(min, max), &lift)| (min.min(min + lift), max.max(max + lift)))
            .collect();
        let mut bodies_by_cell: Grid<usize> = Grid::new(SHADOW_CELL);
        for (index, &(min, max)) in boxes.iter().enumerate() {
            bodies_by_cell.insert(min, max, index);
        }
        Self {
            lifts,
            rank,
            boxes,
            bodies_by_cell,
        }
    }

    /// Контуры тел, которые рисуются **после** `target` и задевают `bounds`
    /// (рамку уже поднятой тени).
    ///
    /// «После» — дальше по [`super::order::draw_order`], тому самому списку, которым
    /// [`super::layers::extrusion_builder`] кладёт дома в меш. У равных ключей
    /// и у пары, которую отношение не связало, это по-прежнему база того же
    /// порядка — глубина центра, — но пару, которую база расставляет неверно
    /// (крыло Г-образного дома перед соседом, а центр за ним), `draw_order`
    /// уже перевернул, и вычитание обязано идти за ним, иначе тень остаётся
    /// на нарисованной стене ровно там, где порядок и чинили.
    ///
    /// Двор соседа в тело входит целиком: у двора есть свои стены, и вычесть
    /// лишнее (тень, которую было бы видно сквозь просвет) дешевле, чем
    /// оставить тёмное пятно на нарисованной стене.
    fn covering(
        &self,
        buildings: &[PolyArea],
        target: usize,
        bounds: (Vec2, Vec2),
    ) -> Vec<Vec<[f32; 2]>> {
        let direction = Lean::of().dir();
        let mut covers: Vec<Vec<[f32; 2]>> = Vec::new();
        for cover in self.bodies_by_cell.near(bounds.0, bounds.1) {
            // сама цель отсеивается тем же правилом: место в порядке у неё
            // одно, а строго дальше себя она не стоит
            let later = self.rank[cover] > self.rank[target];
            if !later || !boxes_overlap(bounds, self.boxes[cover]) {
                continue;
            }
            let (outer, lift) = (&buildings[cover].outer, self.lifts[cover]);
            for chain in silhouette_chains(outer, direction) {
                let mut sweep: Vec<Vec2> = chain.clone();
                sweep.extend(chain.iter().rev().map(|&point| point + lift));
                push_contour(&mut covers, sweep);
            }
            push_contour(&mut covers, outer.clone());
            push_contour(
                &mut covers,
                outer.iter().map(|&point| point + lift).collect(),
            );
        }
        covers
    }
}

/// Контур в список для объединения, обходом против часовой стрелки — тем, что
/// NonZero считает заливкой. Обход свипа зависит от того, с какой стороны дома
/// идёт цепочка силуэта, поэтому нормализуется здесь и только здесь.
fn push_contour(contours: &mut Vec<Vec<[f32; 2]>>, mut ring: Vec<Vec2>) {
    if ring.len() < 3 {
        return;
    }
    if signed_ring_area(&ring) < 0.0 {
        ring.reverse();
    }
    contours.push(ring.into_iter().map(|point| [point.x, point.y]).collect());
}

/// Дыра контура — тот же контур обратным обходом: при NonZero он гасит заливку
/// внутреннего кармана (двора), в который тень попасть не может. Тот же приём
/// и тот же довод, что у `navigation::polymesh::build::push_hole`.
fn push_hole(contours: &mut Vec<Vec<[f32; 2]>>, ring: Vec<Vec2>) {
    let count = contours.len();
    push_contour(contours, ring);
    // вырожденное кольцо `push_contour` отбрасывает — разворачивать нечего
    if let Some(hole) = contours.get_mut(count) {
        hole.reverse();
    }
}

/// Непрерывные (циклически) цепочки рёбер-силуэта кольца — рёбер, чья
/// наружная нормаль смотрит по `direction`. Обход начинается после
/// освещённого ребра, чтобы цепочка не рвалась на шве кольца.
fn silhouette_chains(ring: &[Vec2], direction: Vec2) -> Vec<Vec<Vec2>> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_ring_area(ring).signum();
    let count = ring.len();
    let is_silhouette = |index: usize| {
        let edge = ring[(index + 1) % count] - ring[index];
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        outward.dot(direction) > 0.0
    };
    let Some(lit) = (0..count).find(|&index| !is_silhouette(index)) else {
        // у простого кольца все рёбра силуэтными быть не могут — кривой
        // контур OSM остаётся без тени, а не роняет карту
        return Vec::new();
    };

    let mut chains: Vec<Vec<Vec2>> = Vec::new();
    let mut current: Vec<Vec2> = Vec::new();
    for step in 1..=count {
        let index = (lit + step) % count;
        if is_silhouette(index) {
            if current.is_empty() {
                current.push(ring[index]);
            }
            current.push(ring[(index + 1) % count]);
        } else if !current.is_empty() {
            chains.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chains.push(current);
    }
    chains
}

#[cfg(test)]
mod tests;
