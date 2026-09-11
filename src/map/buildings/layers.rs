//! Сборка мешей зданиевых слоёв: полоса фасада с крышами, длинные тени,
//! 2.5D-экструзия. Каждый билдер отдаёт готовый [`MeshBuilder`], а какие из
//! них спавнить в текущем режиме — решает `spawn_buildings` в родителе.

use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;

use super::arches::{
    ArchOpening, WallCells, arch_openings, arches_by_building, push_arches, push_wall_with_openings,
};
use super::clutter::{flat_roof_items, push_items, ridge_chimney};
use super::garages::{BAY, FACADE_COS, GarageRect, GarageRun, garage_runs, point_to_segment};
use super::material::{
    DOOR_CODE, RoofKind, RoofLook, WallKind, WallLook, building_seed, roof_look, run_kind,
    run_look, run_wall_look, wall_look,
};
use super::order::{draw_order, wall_order};
use super::roofs::{HipRoof, RoofShape, Roofing, min_area_rect, roofing, roofing_of};
use super::{
    BuildingHeightMode, Lean, RoofDetail, extrusion_lift, height_or_default, shade_by_light,
};
use crate::map::meshing::{MeshBuilder, PARAPET_CELLS, Roof, WallFrame, WallMark};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, BuildingUse, PolyArea, RoadLine};
use crate::map::seed::seed_from_point;
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale, sun_stretch};
use crate::settings::STOREY_HEIGHT;

/// Доля реальной высоты, уходящая в полосу фасада. Рисовать все 60 м башни —
/// значит закрасить полквартала: карта сверху, а не изометрия. При 0.2
/// пятиэтажка (15 м) даёт прежние 3 м, и разница этажности всё равно читается.
const FACADE_SCALE: f32 = 0.2;
/// Границы полосы, м: сарай не должен потерять кромку, небоскрёб — накрыть
/// соседний квартал.
const FACADE_HEIGHT_RANGE: RangeInclusive<f32> = 1.5..=12.0;

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
const SHADOW_LENGTH_RANGE: RangeInclusive<f32> = 3.0..=45.0;

/// Высота, на которой рампа тона крыш выходит в максимум: Тула почти вся
/// ниже 75 м, и sqrt в формуле отдаёт разрешение диапазону 5–30 м.
const ROOF_TINT_MAX_HEIGHT: f32 = 60.0;
/// Цвет крыши «в пределе»: темнее и глуше материала — так высотка читается с
/// общего плана. Цель обязана быть **темнее любого цвета любой палитры по
/// светлоте**, иначе рампа переворачивается и высокий дом выходит светлее
/// низкого — а палитры теперь не серые: у красной черепицы (0.72, 0.22, 0.18)
/// сумма каналов 1.12, и нейтральный серый 0.46 её бы *осветлял*. Отсюда
/// почти чёрная цель и короткая смесь вместо прежней пары «0.34 на 0.7»:
/// глубина затемнения та же по порядку, но для насыщенного цвета смесь к
/// тёмному нейтральному — почти масштабирование, тон сохраняется. История
/// цели — 0.71 при почти белой крыше по назначению, 0.34 при первом тёмном
/// битуме; держать 0.34 при поднятом до 0.55 битуме (`material.rs`) нельзя —
/// девятиэтажка (t ≈ 0.67, смесь 0.47) уезжала бы обратно к 0.45, к тем
/// самым грязно-тёмным коробкам на общем плане, ради которых палитры и
/// поднимали. Сейчас она уходит на 0.55 → 0.48.
const ROOF_TALL_COLOR: Color = Color::srgb(0.20, 0.20, 0.21);
/// Насколько рампа может увести крышу к `ROOF_TALL_COLOR` в пределе.
const ROOF_TINT_MAX_MIX: f32 = 0.3;

/// Ширина мягкого края тени, м. Не физическая полутень (угловой размер
/// солнца дал бы сантиметры), а то, чем край тени размыт на снимке:
/// разрешением кадра и светом неба. Метр — это 2–10 экранных пикселей на тех
/// зумах, где тени вообще видны.
pub(super) const PENUMBRA_WIDTH: f32 = 1.0;

/// Осветление верхних вершин стены — дешёвый вертикальный градиент.
const WALL_TOP_LIGHTEN: f32 = 0.15;
/// Насколько стена, повёрнутая прямо к свету, светлее базового тона фасада,
/// и насколько отвёрнутая — темнее. Свет тот же, что даёт тени
/// (`map::sun_light`): при косом подъёме западная стена на нём, южная в тени,
/// и без этой разницы две видимые стены сливались бы в один угол.
const WALL_LIT_MIX: f32 = 0.18;
const WALL_SHADED_MIX: f32 = 0.22;

/// Цвет крыши: базовый — из палитры её материала ([`roof_look`]), при
/// `tinted` поверх идёт рампа по высоте (Кремль и здания без высоты рампу
/// пропускают). Вариации тона внутри квартала даёт уже сам материал — выбор
/// цвета из палитры и разброс по посеву, — поэтому прежнего «минус 3 % на
/// каждый четвёртый дом по индексу» здесь больше нет.
pub(super) fn roof_color(building: &PolyArea, look: &RoofLook, tinted: bool) -> Srgba {
    match building.height {
        Some(height) if tinted && building.kind != AreaKind::Kremlin => {
            let t = (height / ROOF_TINT_MAX_HEIGHT).clamp(0.0, 1.0).sqrt();
            look.base
                .mix(&ROOF_TALL_COLOR.to_srgba(), t * ROOF_TINT_MAX_MIX)
        }
        _ => look.base,
    }
}

/// Ширина панели, м — по основанию стены. Не делитель, а **цель**: сколько
/// панелей встанет на эту стену, решает округление, поэтому на своей стене
/// панель шире или уже трёх метров с небольшим, зато их целое число.
const PANEL_WIDTH: f32 = 3.2;

/// Ширина ячейки стены, м, по облицовке. У всех она панельная, у гаражного
/// ряда — **бокс** ([`garages::BAY`]): ворота стоят по одной на бокс, и мерить
/// их стену панелями значило бы разойтись с гребёнкой боксов на её же кровле.
fn cell_width(kind: WallKind) -> f32 {
    match kind {
        WallKind::GarageDoors => BAY,
        _ => PANEL_WIDTH,
    }
}

/// Сколько ячеек встанет на стену такой длины — целое число, и оно же делитель
/// её собственной сетки: ячейка это `length / wall_columns(length, kind)`.
/// Ширина ячейки — по облицовке ([`cell_width`]), поэтому у гаражного ряда
/// счёт идёт боксами, и тот же счёт уезжает в [`WallCells::panel`]: заплаты
/// вокруг арок и дверей кроятся ровно тем же шагом, что и ворота.
///
/// Наружу — ради проверки: тест меряет клетку тем же делителем, что и стена
/// (`WallSpan::columns`), и убеждается, что ни один проём не разрезал её
/// пополам. Самим заплатам вокруг проёмов функция не нужна — им размер клетки
/// приезжает готовым в [`WallCells`].
pub(super) fn wall_columns(length: f32, kind: WallKind) -> f32 {
    (length / cell_width(kind)).round().max(1.0)
}

/// Одна стена дома до того, как из неё сделали [`WallFrame`]: отрезок
/// основания, подъём и число этажей. Четвёрка ездит вместе через [`wall_frame`],
/// [`push_doors`] и оба цикла [`push_house_with_arches`], и метрика стены —
/// сколько на ней панелей и каким посевом она разыграна — считается по ней же,
/// а не собирается заново у каждого, кому понадобилась.
///
/// Основание с подъёмом ездят и к `arches::push_wall_with_openings` — стена,
/// в которой он режет проёмы, это ровно эта тройка, и разбирать её по
/// одноимённым параметрам значило бы дать перепутать их местами.
pub(super) struct WallSpan {
    /// Начало грани; от неё же и посев ([`WallSpan::seed`]).
    pub(super) a: Vec2,
    pub(super) b: Vec2,
    /// Подъём: вектор от основания стены до карниза (`extrusion_lift`).
    pub(super) lift: Vec2,
    /// Этажей в доме — целое число ([`storeys_of`]).
    storeys: f32,
    /// Длинная ось плана этого дома — единственное, что стена знает про
    /// **остальные** стены того же дома, и по чему только и отличается
    /// длинный фасад от торца ([`WallSpan::gable_end`]). `None` — либо у
    /// плана нет длинной стороны ([`plan_long_axis`]), либо балконов этому
    /// дому не полагается и ось считать не стали.
    long_axis: Option<Vec2>,
}

impl WallSpan {
    pub(super) fn new(a: Vec2, b: Vec2, lift: Vec2, storeys: f32, long_axis: Option<Vec2>) -> Self {
        Self {
            a,
            b,
            lift,
            storeys,
            long_axis,
        }
    }

    /// Длина основания, м.
    fn length(&self) -> f32 {
        (self.b - self.a).length()
    }

    /// **Торец** ли эта стена — стоит ли она поперёк длинной оси плана.
    ///
    /// Это и есть противопоставление, которого в коде не было: у панельной
    /// секции балконы идут по длинному фасаду, а торец — глухая плита, и
    /// отличить одно от другого по ширине самой стены нельзя. Торец настоящей
    /// секции — её глубина, 12–14 м, то есть четыре панели, и порог ширины
    /// ([`BALCONY_COLUMNS_MIN`]) его пропускает; он отсекает не торец, а
    /// ступеньку контура.
    ///
    /// Поперёк — с запасом в [`GABLE_END_COS_MAX`]: на непрямоугольном плане
    /// «короткая сторона» смысла не имеет, и косая стена остаётся фасадом,
    /// то есть при сомнении всё остаётся как было.
    fn gable_end(&self) -> bool {
        let (Some(axis), Some(along)) = (self.long_axis, (self.b - self.a).try_normalize()) else {
            return false;
        };
        along.dot(axis).abs() <= GABLE_END_COS_MAX
    }

    /// Сколько ячеек встанет на эту стену — целое число ([`wall_frame`]);
    /// ширина ячейки у гаражного ряда своя ([`cell_width`]).
    fn columns(&self, kind: WallKind) -> f32 {
        wall_columns(self.length(), kind)
    }

    /// Посев этой стены, `[0, 1)` — от её начала ([`seed_from_point`]). Одним
    /// местом, потому что дверь обязана взять **тот же** посев, что и стена под
    /// ней ([`push_doors`]): разойдись они, и полотно перестало бы держаться
    /// того же разброса, что рисунок вокруг него.
    fn seed(&self) -> f32 {
        (seed_from_point(self.a) & 0xff) as f32 / 255.0
    }
}

/// Рама **стены** для шейдера: её собственные координаты — номер панели вдоль
/// основания и этаж вверх по подъёму ([`WallFrame`]).
///
/// Считать стену глобальной сеткой нельзя, и это не про точность, а про то,
/// что видно: сетка, не знающая, где стена кончается, режет у её края панель
/// и балкон пополам, а на угле дома швы двух стен сходятся на разной высоте.
/// Поэтому счёт идёт **в ячейках самой стены**: число панелей и этажей целое,
/// так что крайняя панель всегда целая, верхний этаж упирается ровно в
/// карниз, а на угле оба соседа заканчиваются целой панелью и целым этажом.
/// Шейдеру после этого не нужны ни ширина панели, ни высота этажа, ни косина
/// подъёма — только дробная часть координаты.
///
/// Посев — свой у каждой стены (`seed_from_point` от её начала, тот же
/// генератор, что у кровель, дверей и машин), а не общий на дом: столбцы
/// балконов и разброс окон на соседних стенах не должны начинаться одинаково.
///
/// **Материал** же, наоборот, общий на дом ([`WallLook`]): у одного здания не
/// бывает панельного торца и кирпичного фасада, и его код едет в тот же слот
/// атрибута, где у кровли стоит её материал.
fn wall_frame(building: &PolyArea, look: &WallLook, span: &WallSpan) -> Option<WallFrame> {
    let columns = span.columns(look.kind);
    let frame = WallFrame::new(
        span.a,
        span.b,
        span.lift,
        columns,
        span.storeys,
        look.kind.code(),
        span.seed(),
    )?;
    let balconies = balconies_fit(building, look.kind, columns, span);
    Some(match balconies {
        true => frame,
        false => frame.marked(WallMark::Blank),
    })
}

/// Клетки этой стены — всё, что нужно, чтобы выкроить в ней проём: рама,
/// заплата и размер клетки по обеим осям.
///
/// Арке этого хватает целиком ([`push_wall_with_openings`]), и арифметика
/// панелей с этажами остаётся здесь: [`WallSpan`] знает и то и другое, а
/// `arches` — только грань, в которой режет.
///
/// Ячейка — по облицовке ([`cell_width`]), а не всегда панель: у гаражного ряда
/// это бокс прогона, и заплата вокруг проёма кроится тем же счётом, каким на
/// этой стене стоят ворота.
fn wall_cells(building: &PolyArea, look: &WallLook, span: &WallSpan) -> WallCells {
    let frame = wall_frame(building, look, span);
    WallCells {
        frame,
        patch: frame.map(|frame| frame.marked(WallMark::Solid)),
        panel: span.length() / span.columns(look.kind),
        storey: span.lift / (span.storeys + PARAPET_CELLS),
    }
}

/// Видимые стены гаражного контура — **по куску за раз**, а не по целому
/// кольцу.
///
/// Разрез идёт хордой, и она рассекает ребро контура: западная стена буквы Г
/// длиной в 52 м лежит на двух кусках сразу, по 8 и 44 м. Целой ей достаётся
/// сетка одного из них, и на второй половине ворота уезжают от гребёнки над
/// ними — та самая ошибка, ради которой стена и села на сетку куска. Кусок
/// же уже несёт своё кольцо, разрезанное там, где надо: его силуэт и есть
/// список стен, каждая со своим куском.
///
/// Хорда стеной не считается: она внутри дома, и снаружи её нет. Отличается
/// она ровно тем, что не лежит на исходном контуре.
fn garage_walls(run: &GarageRun, outer: &[Vec2], facing: Vec2) -> Vec<(Vec2, Vec2)> {
    run.rects
        .iter()
        .flat_map(|rect| {
            silhouette_edges(&rect.ring, facing)
                .into_iter()
                .filter(|(a, b)| on_ring(outer, (*a + *b) / 2.0))
        })
        .collect()
}

/// Лежит ли точка на кольце — этим настоящая стена и отличается от хорды
/// разреза.
fn on_ring(ring: &[Vec2], point: Vec2) -> bool {
    let count = ring.len();
    (0..count).any(|at| point_to_segment(point, ring[at], ring[(at + 1) % count]) < 1e-3)
}

/// Клетки гаражной стены — **шагом своего куска**, а не общей меркой [`BAY`].
///
/// Ворота обязаны стоять под своим же швом кровли, и до сих пор это держалось
/// на совпадении: кровля считается шагом куска (`bay`), стена — шагом
/// `длина / round(длина / BAY)`. Две сетки сходятся только там, где длина
/// стены равна длине куска; у разрезанного контура стена вдвое короче, и
/// гребёнка на кровле разъезжалась с воротами под ней тем сильнее, чем дальше
/// от начала.
///
/// Шаг берётся у куска, **фаза — нет**: клеток на стене целое число и
/// кончаются они ровно на её углах, как у всякой другой стены ([`wall_frame`]).
/// Посаженная на фазу куска стена начинается и кончается посреди клетки, и
/// крайние ворота выходят обрезанными — по половине створки на каждом зубе
/// гребёнки. Длинному фасаду фаза и не нужна: его длина и есть длина куска,
/// поэтому тот же шаг от того же угла даёт те же границы.
///
/// Поперечная стена — **торец**: ворот на ней нет ([`WallMark::Solid`]), и
/// меряется она рядами, а не боксами. У ленты ряд один на всю ширину, так что
/// торец — одна клетка без единого шва внутри; у кооператива на нём видны те
/// же границы рядов, что идут по кровле.
///
/// `None` — стена не гаражная, или кусок к ней не повёрнут (косая грань
/// контура): тогда считает [`wall_cells`], как считал.
fn garage_cells(
    building: &PolyArea,
    look: &RoofLook,
    wall: &WallLook,
    span: &WallSpan,
) -> Option<WallCells> {
    if wall.kind != WallKind::GarageDoors {
        return None;
    }
    let run = look.run.as_ref()?;
    let along = (span.b - span.a).try_normalize()?;
    let rect = run.holder((span.a + span.b) / 2.0);
    // кривой обрезок: ворот на нём не рисуется вовсе — ни целых, ни половинок
    if rect.plain {
        let cells = wall_cells(building, wall, span);
        return Some(WallCells {
            frame: cells.patch,
            ..cells
        });
    }
    let facade = rect.faces_the_drive(along);
    let (direction, pitch) = match facade {
        true => (rect.axis, rect.bay),
        false => (Vec2::new(-rect.axis.y, rect.axis.x), rect.row),
    };
    // косая грань не лежит ни вдоль, ни поперёк — её сетке неоткуда взяться
    let turn = along.dot(direction);
    if turn.abs() < FACADE_COS || pitch <= 0.0 {
        return None;
    }
    // Ячеек на стене — целое число, и кончаются они ровно на её углах. Фаза
    // куска сюда не едет намеренно: посаженная на неё стена начинается и
    // кончается посреди ячейки, и крайние ворота выходили обрезанными с обоих
    // концов — по половинке створки на каждом зубе гребёнки. Целое число, как
    // у всякой другой стены (`wall_frame`); от куска берётся **шаг**, и этого
    // достаточно: у длинного фасада его длина и есть длина куска, так что
    // гребёнка над ним идёт тем же шагом от того же угла.
    let columns = (span.length() / pitch).round().max(1.0);
    let frame = WallFrame::new(
        span.a,
        span.b,
        span.lift,
        columns,
        span.storeys,
        wall.kind.code(),
        span.seed(),
    )?;
    // балконов на гараже не бывает ни на фасаде, ни на торце; торцу вдобавок
    // не полагается ни одного проёма
    let patch = frame.marked(WallMark::Solid);
    Some(WallCells {
        frame: Some(match facade {
            true => frame.marked(WallMark::Blank),
            false => patch,
        }),
        patch: Some(patch),
        panel: span.length() / columns,
        storey: span.lift / (span.storeys + PARAPET_CELLS),
    })
}

/// Сколько этажей в этой стене — от **настоящей** высоты дома, а не от
/// нарисованной: `EXTRUDE_SCALE` сжимает стену вместе с этажами, и считать их
/// по сжатой значило бы получить полтора этажа у пятиэтажки.
///
/// Число целое, и на нём держится вся рама ([`wall_frame`]): верхний этаж
/// упирается ровно в карниз, а на углу дома обе стены кончаются одинаково.
fn storeys_of(building: &PolyArea) -> f32 {
    (height_or_default(building) / STOREY_HEIGHT)
        .round()
        .max(1.0)
}

/// Кому балконы полагаются. Это не про геометрию, а про то, что бывает на
/// фотографии: балкон — примета **жилого дома в несколько этажей**, и рисунок
/// стены без него встречается сплошь, а он без него нет.
///
/// Решает в первую очередь **материал стены**, и это не перекладывание
/// условия: назначение уже разобрано один раз, когда дому выбирали облицовку
/// ([`super::material::wall_look`]), и там же учтён рост. Штукатурка достаётся
/// частному сектору и малоэтажке, витраж — торговому центру, профлист —
/// складу; балконов нет ни у кого из них по самому смыслу материала. Остаются
/// панель и кирпич — ровно те две стены, на которых балкон и бывает.
///
/// Четыре ограничения сверх материала:
///
/// * **частный дом** — никогда, каким бы ни вышел материал: `building=house`
///   это отдельный дом с участком, и балкона у него не бывает. Порог
///   этажности отсекает почти все такие дома и сам, но «почти» тут мало —
///   пятиэтажный `house` в выгрузке встречается, и балконы на нём читались бы
///   как ошибка разбора, чем и были бы;
/// * ниже [`BALCONY_STOREYS_MIN`] — двухэтажка с рядом балконов во всю стену
///   читается как ошибка, и на карте это видно первым;
/// * простенок уже [`BALCONY_COLUMNS_MIN`] панелей — **ступенька контура**,
///   глухая стенка уступа: ряд выступов на трёхметровой полоске не бывает
///   ничем, кроме узора. Порог ширины отсекает только её и торцем не
///   притворяется — четыре панели глубокой секции он пропускает;
/// * **торец** ([`WallSpan::gable_end`]) — стена поперёк длинной оси плана.
///   Это и есть «глухие торцы»: балконы идут по длинному фасаду, а торец их
///   не несёт, оставаясь при этом стеной с окнами ([`WallMark::Blank`], а не
///   `Solid`).
fn balconies_fit(building: &PolyArea, kind: WallKind, columns: f32, span: &WallSpan) -> bool {
    balcony_house(building, kind, span.storeys)
        && columns >= BALCONY_COLUMNS_MIN
        && !span.gable_end()
}

/// Дом, у которого балкон бывает вообще: материал, назначение, рост — то, что
/// не зависит от отдельной стены. Спрашивается это дважды, и второй раз до
/// стен: длинную ось плана ([`plan_long_axis`]) считать незачем, если балконов
/// не будет ни на одной из них ([`push_house_with_arches`]).
fn balcony_house(building: &PolyArea, kind: WallKind, storeys: f32) -> bool {
    matches!(kind, WallKind::Panel | WallKind::Brick)
        && building.building_use != BuildingUse::House
        && storeys >= BALCONY_STOREYS_MIN
}

/// Длинная ось плана — та самая, вдоль которой стоит длинный фасад, и `None`,
/// если у плана длинной стороны нет.
///
/// Ось берётся у **минимального описанного прямоугольника** (`min_area_rect`,
/// первое ребро вдоль длинной оси) — того же, по которому идёт конёк
/// двускатной крыши и фактура кровли, так что второго понятия «как этот дом
/// повёрнут» в проекте не заводится.
///
/// Отношение сторон — оговорка, без которой правило врёт: у башни в плане
/// квадрат, короткой стороны у неё нет, и объявить две её стены торцами
/// значило бы снять балконы с половины дома по броску округления. Порог
/// [`GABLE_PLAN_RATIO_MIN`] проходит панельная секция (12–14 м на 35 и
/// длиннее — это 2.5 и выше) и не проходит башня
/// (`heights::TOWER_MAX_RATIO` — 1.7).
fn plan_long_axis(ring: &[Vec2]) -> Option<Vec2> {
    let rect = min_area_rect(ring)?;
    let long = rect[1] - rect[0];
    let short = (rect[2] - rect[1]).length();
    let axis = long.try_normalize()?;
    (short > 0.0 && long.length() / short >= GABLE_PLAN_RATIO_MIN).then_some(axis)
}

/// С какого этажа дом носит балконы и с какой ширины стены они на ней
/// помещаются — в этажах и панелях самой стены.
///
/// Порог этажности — **та же** граница, по которой стене выбирают облицовку
/// ([`super::material`] читает эту константу как `LOW_RISE_STOREYS`): ниже неё
/// дом малоэтажный, а малоэтажному не достаётся ни панель, ни витраж, то есть
/// и балконам не с чего взяться. Одно число на оба правила, а не два
/// совпадающих: правка одного молча развела бы их. Объявлено оно здесь, потому
/// что витрина стен вычитывает пороги балконов прямо из этого файла
/// (`examples/demos/wall_gallery/constants.rs`).
pub(super) const BALCONY_STOREYS_MIN: f32 = 4.0;
const BALCONY_COLUMNS_MIN: f32 = 3.0;

/// Во сколько раз план должен быть длиннее, чем шире, чтобы у него **был**
/// торец ([`plan_long_axis`]). Ниже порога дом в плане квадратный, длинного
/// фасада у него нет, и все его стены — фасады.
const GABLE_PLAN_RATIO_MIN: f32 = 1.5;
/// Насколько стена считается стоящей поперёк длинной оси: косинус угла между
/// ними ([`WallSpan::gable_end`]). 0.5 — это 60°, то есть торцем становится
/// стена, отклонившаяся от поперечника не больше чем на 30°; всё, что косее,
/// остаётся фасадом с балконами, как было до правила торцов.
const GABLE_END_COS_MAX: f32 = 0.5;

/// Насколько далеко от грани контура может лежать вход, чтобы считаться
/// стоящим на ней, м. Сгенерированные двери (`osm::entrances`) лежат на грани
/// точно, размеченные в OSM — в вершине кольца, и полметра тут только на
/// разнобой координат.
const DOOR_ON_WALL: f32 = 0.5;

/// Проём входа в метрах: ширина по основанию стены и высота вверх по подъёму,
/// вместе с обрамлением и козырьком (само полотно шейдер рисует внутри).
/// Разные они не для разнообразия, а потому что дверь — это масштабная линейка
/// дома: в подъезд входят по двое, в частный дом по одному, у витрины створки
/// стеклянные во весь рост, у склада не дверь, а ворота.
pub(super) fn door_size(kind: WallKind) -> Vec2 {
    match kind {
        WallKind::Panel | WallKind::Brick => Vec2::new(1.9, 2.8),
        WallKind::Plaster => Vec2::new(1.3, 2.4),
        WallKind::Shopfront => Vec2::new(2.4, 3.0),
        WallKind::Shed => Vec2::new(3.2, 2.9),
        // ворота рисует сама облицовка, по створке на бокс; сюда эта стена не
        // доходит ([`push_doors`]), и размер тут только чтобы `match` остался
        // исчерпывающим
        WallKind::GarageDoors => Vec2::new(3.0, 2.5),
    }
}

/// Где вход стоит на грани `a→b`, в метрах от её начала, — или `None`, если он
/// не на ней. Конец грани не её: это начало следующей, и вход в общей вершине
/// (а размеченный в OSM вход стоит именно в вершине) достаётся одной из двух.
fn door_on_edge(door: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let length = (b - a).length();
    let along = (b - a).try_normalize()?;
    let offset = door - a;
    let at = offset.dot(along);
    match offset.perp_dot(along).abs() <= DOOR_ON_WALL && (0.0..length).contains(&at) {
        true => Some(at),
        false => None,
    }
}

/// Чья это дверь: номер грани кольца, которой вход принадлежит.
///
/// Одной, а не всякой, что оказалась в допуске. [`DOOR_ON_WALL`] — полметра, а
/// ступенька контура в OSM бывает и в двадцать сантиметров: вход тогда попадал
/// сразу на две грани, каждая вдвигала полотно внутрь себя ([`push_doors`] —
/// дверь у края грани сдвигается, чтобы влезть целиком), и на стене выходило
/// два полотна рядом при одной двери в данных.
///
/// Побеждает **ближайшая** грань, при равенстве — та, что раньше в кольце.
/// Ничью надо разрешать явно: у вырожденного нулевого уступа расстояние до
/// обеих граней одинаково с точностью до бита, и «строго ближе» тогда не
/// отсекает ни одну.
fn door_edge(ring: &[Vec2], door: Vec2) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for index in 0..ring.len() {
        let from = ring[index];
        let to = ring[(index + 1) % ring.len()];
        if door_on_edge(door, from, to).is_none() {
            continue;
        }
        let Some(along) = (to - from).try_normalize() else {
            continue;
        };
        let across = (door - from).perp_dot(along).abs();
        if best.is_none_or(|(nearest, _)| across < nearest) {
            best = Some((across, index));
        }
    }
    best.map(|(_, index)| index)
}

/// Входы этой стены — **из данных**, а не по броску шейдера.
///
/// Дверь дома придумана не здесь: `osm::entrances` ставит её на грань, которая
/// смотрит на дорогу, и не ставит на ту, к которой прижат сосед; туда же идёт
/// пешка (`human::systems::building_target`) и туда же смотрит гизмо дверей.
/// Пока шейдер сам разыгрывал вход на 24 % панелей первого этажа, все трое
/// говорили о разных дверях: нарисованная стояла там, где входа нет, а вход —
/// там, где стена глухая. Поэтому дверь **приходит геометрией**, и панельная
/// сетка её не двигает: снести полотно к центру ячейки — те же полтора метра
/// расхождения, ради устранения которых всё и затевалось.
///
/// Два четырёхугольника на вход:
///
/// * **заплата** — ячейки стены, которые задевает полотно, целиком: рама и код
///   у неё те же, что у стены (швы и кладка проходят насквозь), но помечена
///   она [`WallMark::Solid`], то есть без проёмов. Без неё из-за двери
///   выглядывало бы окно этой ячейки: окно стоит по центру ячейки, а дверь —
///   где ей велели данные, и они пересекаются;
/// * **полотно** — ровно проём, со своей рамой в клетку `[0, 1]²`
///   ([`WallFrame::opening`]) и кодом [`DOOR_CODE`].
///
/// Дверь у самого угла (в OSM вход — это вершина кольца, то есть угол дома)
/// вдвигается внутрь стены целиком: половина полотна, уехавшая на соседнюю
/// грань, читалась бы как дыра в углу. Угловой вход достаётся **одной** грани —
/// той, для которой он начало, а не конец.
fn push_doors(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    wall: &WallLook,
    span: &WallSpan,
    cells: &WallCells,
    openings: &[ArchOpening],
    color: LinearRgba,
) {
    // У гаражного ряда вход в каждом боксе, и рисует их сама облицовка —
    // отдельное полотно по входу из OSM встало бы поверх створки соседним
    // прямоугольником другого размера.
    if building.entrances.is_empty() || wall.kind == WallKind::GarageDoors {
        return;
    }
    let length = span.length();
    let Some(along) = (span.b - span.a).try_normalize() else {
        return;
    };
    let size = door_size(wall.kind);
    if length < size.x {
        return;
    }
    // клетки стены у двери и у арки одни и те же: панель вдоль основания,
    // этаж вверх по подъёму, и та же заплата «без проёмов». Выше этажа дверь
    // не поднимается
    let door_up = cells.storey * (size.y / STOREY_HEIGHT).min(1.0);
    let seed = span.seed();
    let patch = cells.patch;

    // номер этой грани в кольце — по нему и решается, чья дверь
    let mine = building
        .outer
        .iter()
        .position(|&from| from == span.a)
        .filter(|index| building.outer[(index + 1) % building.outer.len()] == span.b);

    for &door in &building.entrances {
        // сперва дешёвая проверка «эта дверь вообще на этой грани»: `door_edge`
        // обходит всё кольцо, и звать его на каждый вход каждой стены незачем
        let Some(at) = door_on_edge(door, span.a, span.b) else {
            continue;
        };
        // вход принадлежит **одной** грани — ближайшей: у ступеньки контура
        // мельче полуметра его забирали обе, каждая вдвигала полотно внутрь
        // себя, и на стене выходила двойная дверь
        if door_edge(&building.outer, door) != mine {
            continue;
        }
        let half = size.x / 2.0;
        let center = at.clamp(half, length - half);
        // арка — уже проём во всю стену, второго в нём не бывает
        if openings.iter().any(|opening| {
            opening.a == span.a
                && opening.b == span.b
                && center + half > opening.low
                && center - half < opening.high
        }) {
            continue;
        }

        // клетки, которые задело полотно, достаются ему целиком — тем же
        // счётом, каким кроится заплата арки ([`WallCells::block`])
        let (first, last) = cells.block(center - half, center + half, length);
        let (p0, p1) = (span.a + along * first, span.a + along * last);
        builder.set_wall(patch);
        builder.push_quad([p0, p1, p1 + cells.storey, p0 + cells.storey], color);

        let (d0, d1) = (
            span.a + along * (center - half),
            span.a + along * (center + half),
        );
        builder.set_wall(WallFrame::opening(d0, d1, door_up, DOOR_CODE, seed));
        builder.push_quad([d0, d1, d1 + door_up, d0 + door_up], color);
    }
}

/// Вальма в меш: скаты по контуру, потом площадка конька поверх них.
fn push_hip(builder: &mut MeshBuilder, roof: &HipRoof) {
    for (slope, tone) in &roof.slopes {
        builder.push_quad(*slope, *tone);
    }
    let (ridge, tone) = &roof.ridge;
    builder.push_polygon(ridge, &[], *tone);
}

/// Концы «конька» вальмы — две самые далёкие друг от друга точки её
/// площадки. У настоящей вальмы конёк это отрезок, и на четырёх-двадцати
/// вершинах площадки перебор пар дешевле любой геометрии.
fn hip_ridge_ends(roof: &HipRoof) -> (Vec2, Vec2) {
    let ring = &roof.ridge.0;
    let mut best = (ring[0], ring[0]);
    let mut span = 0.0;
    for (index, &a) in ring.iter().enumerate() {
        for &b in &ring[index + 1..] {
            let distance = a.distance_squared(b);
            if distance > span {
                span = distance;
                best = (a, b);
            }
        }
    }
    best
}

/// Конёк двускатной крыши: у ската `[карниз, карниз, конёк, конёк]` два
/// последних угла и есть его концы. Отдельным полем `GableRoof` их не держит —
/// они уже там, а труба на коньке единственный, кому они понадобились.
fn ridge_of(roof: &super::roofs::GableRoof) -> (Vec2, Vec2) {
    let [_, _, far, near] = roof.slopes[0].0;
    (near, far)
}

/// Кровля этого дома: у бокса, вошедшего в гаражный прогон, она берётся
/// **от прогона**, а не от него самого. В этом весь приём: общий посев и
/// общая ось превращают двадцать домиков в одну ленту, а по-своему посеянный
/// бокс красится и ребрится сам по себе.
fn look_of(building: &PolyArea, run: Option<&GarageRun>) -> RoofLook {
    match run {
        Some(run) => run_look(run),
        None => roof_look(building),
    }
}

/// Плоская кровля: заливка контура (с дворами-дырками) под фактуру своего
/// материала. Ровно это игра кладёт на всякий дом без скатной крыши — и в
/// плоских режимах, где `outer` это сам контур, и в 2.5D, где он уже поднят
/// на высоту стен.
///
/// **Бортика по краю у мягкой кровли больше нет.** Он был каймой в 0.7 м,
/// затенённой по нормали ребра, — то есть той же самой конструкцией, что и
/// скаты вальмы, только уже; с воздуха он читался не парапетом, а маленькой
/// вальмой на каждом панельном доме, и отличить по нему настоящую вальму от
/// плоской крыши было нельзя. Край кровли и без него держат стены под ней.
pub(super) fn push_flat_roof(
    builder: &mut MeshBuilder,
    look: &RoofLook,
    outer: &[Vec2],
    holes: &[Vec<Vec2>],
    color: Srgba,
    lift: Vec2,
) {
    let Some(run) = look.run.as_ref() else {
        builder.set_roof(Some(look.frame));
        builder.push_polygon(outer, holes, color.into());
        return;
    };
    // гаражный контур считается в ячейках, как стена: рамка у каждой вершины
    // своя, и мировая точка с осью шейдеру не нужны. Ячейки эти — **куска**,
    // а не дома, поэтому и кровля кладётся по куску за раз: куски покрывают
    // контур целиком и без нахлёста
    if run.rects.len() > 1 {
        for rect in &run.rects {
            match rect.plain {
                // кривой обрезок: гребёнки боксов на нём нет вовсе, только
                // профлист того же цвета, что у соседей по контуру
                true => builder.set_roof(Some(plain_garage_roof(look))),
                false => builder.set_wall(garage_frame(rect, look.frame.seed, lift)),
            }
            let ring: Vec<Vec2> = rect.ring.iter().map(|at| *at + lift).collect();
            builder.push_polygon(&ring, &[], color.into());
        }
        return;
    }
    // один кусок — значит резать было нечего, и контур кладётся как пришёл:
    // с дворами, которых у куска не бывает
    builder.set_wall(garage_frame(run.main(), look.frame.seed, lift));
    builder.push_polygon(outer, holes, color.into());
}

/// Кровля куска, на котором гаража **не рисуется**: тот же профлист, каким
/// крыт гараж, в том же цвете и с той же фазой — но обычной рамкой по мировой
/// точке, без единой ячейки бокса. Так и выглядит зуб гребёнки в четыре
/// метра: кусок кровли, а не отдельный гараж поперёк соседей.
fn plain_garage_roof(look: &RoofLook) -> Roof {
    Roof {
        axis: look.frame.axis,
        material: RoofKind::Corrugated.code(),
        seed: look.frame.seed,
    }
}

/// Рама **гаражного куска** для шейдера: номер бокса вдоль него и номер ряда
/// поперёк, оба целые на торцах ([`WallFrame::run`]).
///
/// `lift` — сдвиг, с которым кровля этого дома уже легла в меш (в плоских
/// режимах ноль, в 2.5D подъём стен): рама обязана считаться в той же
/// системе, в какой лежат вершины, иначе швы уедут от дома.
fn garage_frame(rect: &GarageRect, seed: f32, lift: Vec2) -> Option<WallFrame> {
    WallFrame::run(
        rect.origin + lift,
        rect.axis,
        rect.bay,
        rect.row,
        run_kind(rect.block).code(),
        seed,
    )
}

/// Тон стены `a→b` по её повороту к свету: (низ, верх). Стена видима,
/// значит её настоящая нормаль смотрит против подъёма — это и выбирает
/// сторону перпендикуляра, обход кольца тут ни при чём.
pub(super) fn wall_colors(
    facade: Srgba,
    a: Vec2,
    b: Vec2,
    lift_dir: Vec2,
) -> (LinearRgba, LinearRgba) {
    let edge = b - a;
    let mut normal = Vec2::new(edge.y, -edge.x).normalize_or_zero();
    if normal.dot(lift_dir) > 0.0 {
        normal = -normal;
    }
    let bottom = shade_by_light(facade, normal, WALL_LIT_MIX, WALL_SHADED_MIX);
    let top = bottom.mix(&Srgba::WHITE, WALL_TOP_LIGHTEN);
    (bottom.into(), top.into())
}

/// Фасадная полоса + крыши (режимы Facade / Shadows / ShadowsTint).
pub(super) fn facade_and_roof_builders(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    detail: RoofDetail,
) -> (MeshBuilder, MeshBuilder) {
    let arches = arches_by_building(buildings, passages);
    let runs = garage_runs(buildings);
    let mut facades = MeshBuilder::default();
    // крыши рисует `RoofMaterial`, и рамку кровли ему даёт этот атрибут
    let mut roofs = MeshBuilder::with_roof_coords();
    for (index, building) in buildings.iter().enumerate() {
        // фактуры у плоской полосы нет — она идёт одним earcut-полигоном, — но
        // цвет у неё тот же, что был бы у настоящей стены в 2.5D
        let facade_color = wall_look(building, storeys_of(building)).base;

        // фасад — тот же контур, сдвинутый вниз: тёмная кромка видна
        // только вдоль южных граней любого полигона. Сдвиг — по высоте из
        // OSM, так что этажность города видна прямо на карте
        let facade_height = (height_or_default(building) * FACADE_SCALE)
            .clamp(*FACADE_HEIGHT_RANGE.start(), *FACADE_HEIGHT_RANGE.end());
        let offset = Vec2::new(0.0, -facade_height);
        let facade_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + offset).collect();
        let facade_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + offset).collect())
            .collect();
        facades.push_polygon(&facade_outer, &facade_holes, facade_color.into());
        // крыши — отдельный слой поверх фасадов, так что вырезать проём из
        // полосы достаточно: над аркой крыша останется целой сама собой
        if let Some(passages) = arches.get(&index) {
            push_arches(&mut facades, building, passages, offset);
        }
        // двускатная крыша в плоском режиме — два ската разного тона в
        // одной плоскости: конёк не поднят, но дом уже не коробка
        let look = look_of(building, runs.get(&index));
        let color = roof_color(building, &look, detail.tinted);
        let mut items = Vec::new();
        match roofing(
            building,
            Vec2::ZERO,
            |_| Vec2::ZERO,
            color,
            building_seed(building),
        ) {
            Roofing::Gable(roof) => {
                roofs.set_roof(Some(look.frame));
                for (slope, slope_color) in roof.slopes {
                    roofs.push_quad(slope, slope_color);
                }
                if detail.clutter {
                    items.extend(ridge_chimney(
                        building,
                        &look,
                        ridge_of(&roof),
                        roof.ridge_offset,
                    ));
                }
            }
            Roofing::Hip(roof) => {
                roofs.set_roof(Some(look.frame));
                push_hip(&mut roofs, &roof);
                if detail.clutter {
                    items.extend(ridge_chimney(
                        building,
                        &look,
                        hip_ridge_ends(&roof),
                        roof.ridge_offset,
                    ));
                }
            }
            Roofing::Flat => {
                push_flat_roof(
                    &mut roofs,
                    &look,
                    &building.outer,
                    &building.holes,
                    color,
                    Vec2::ZERO,
                );
                if detail.clutter {
                    items = flat_roof_items(building, &look, Vec2::ZERO);
                }
            }
        }
        // в плоском режиме у коробки нет стен — только тень и верх, как у
        // самих домов в этих режимах
        push_items(&mut roofs, &items, None, color);
    }
    (facades, roofs)
}

/// Тени зданий: на каждую непрерывную цепочку рёбер-силуэта внешнего кольца —
/// **один** свип-полигон `[цепочка, цепочка + сдвиг в обратном порядке]`.
/// Не квады на ребро: у ступенчатого фасада квады соседних ступеней
/// перекрываются вдоль тени, и полупрозрачность складывалась в полосы двойной
/// темноты. Свип цепочки самопересечься не может: перп-шаг ребра силуэта
/// равен `outward·shadow_dir() > 0`, то есть цепочка монотонна вдоль
/// перпендикуляра тени.
///
/// Затем **все** свипы карты объединяются булевым union (`i_overlay`) в набор
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
    extruded: bool,
) -> MeshBuilder {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::float::simplify::SimplifyShape;

    let mut sweeps: Vec<Vec<[f32; 2]>> = Vec::new();
    let stretch = sun_stretch();
    let (min_length, max_length) = (
        *SHADOW_LENGTH_RANGE.start() * stretch,
        *SHADOW_LENGTH_RANGE.end() * stretch,
    );
    for building in buildings {
        let length =
            (height_or_default(building) * shadow_length_scale()).clamp(min_length, max_length);
        let offset = shadow_dir() * length;
        for chain in silhouette_chains(&building.outer, shadow_dir()) {
            let mut sweep: Vec<Vec2> = chain.clone();
            sweep.extend(chain.iter().rev().map(|&point| point + offset));
            push_contour(&mut sweeps, sweep);
        }
    }

    let mut builder = MeshBuilder::default();
    let color = SHADOW_COLOR.to_linear();
    // край тени на снимке мягкий, и не из-за углового размера солнца (тот дал
    // бы сантиметры), а из-за разрешения кадра и рассеянного света неба.
    // Поэтому полутень задаётся видом, а не физикой: метр — это 2–10 экранных
    // пикселей на тех зумах, где тени вообще видны
    let fade = LinearRgba {
        alpha: 0.0,
        ..color
    };
    for shape in sweeps.simplify_shape(FillRule::NonZero) {
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
        // кайма от каждого контура объединённой фигуры — наружу от внешнего и
        // внутрь просвета от каждой дырки: кайма выбирает сторону по площади
        // самого кольца, так что просвету нужно `outside: false`, иначе она
        // ляжет на уже залитое тело и обведёт дырку двойной темнотой вместо
        // растушёвки. Каймы соседних фигур могут наложиться, но обе сходят в
        // ноль, и удвоение выходит слабее самой тени. Ширина на каждой
        // вершине — своя, см. [`penumbra`]
        builder.push_inset_band_tapered(&outer, PENUMBRA_WIDTH, true, penumbra, color, fade);
        for hole in &holes {
            builder.push_inset_band_tapered(hole, PENUMBRA_WIDTH, false, penumbra, color, fade);
        }
    }

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

/// Доля [`PENUMBRA_WIDTH`], которую кайма получает на вершине, идущей в
/// сторону `direction`: проекция этого направления на [`shadow_dir`].
///
/// Полутень растёт с расстоянием от того, кто отбрасывает тень, а у самой
/// стены её нет вовсе — тень примыкает к дому жёстко. В объединённой фигуре
/// это различие читается локально: у ребра примыкания «наружу» смотрит
/// **против** света (тело тени лежит по `shadow_dir()` от него), у дальнего
/// края — по свету, у бокового — поперёк. Отсюда и правило: у примыкания
/// ноль, у дальнего края вся ширина, вдоль боковой стороны — рост от нуля на
/// углу дома до полной ширины на дальнем конце, ровно как у настоящей
/// полутени.
///
/// Без него метровая кайма шла и по контуру примыкания: на солнечной стороне
/// каждого выпуклого угла дома оставалось тёмное пятно в метр, и дом выходил
/// обведён мягкой каймой — тем самым «контактным затенением», которое из
/// объединения убрали (`references` в скилле `osm-map`).
fn penumbra(direction: Vec2) -> f32 {
    direction.dot(shadow_dir()).max(0.0)
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

/// Непрерывные (циклически) цепочки рёбер-силуэта кольца — рёбер, чья
/// наружная нормаль смотрит по `direction`. Обход начинается после
/// освещённого ребра, чтобы цепочка не рвалась на шве кольца.
pub(super) fn silhouette_chains(ring: &[Vec2], direction: Vec2) -> Vec<Vec<Vec2>> {
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

/// 2.5D-экструзия: painter's algorithm внутри одного меша — треугольники
/// растеризуются в порядке index-буфера, поэтому здания пишутся от дальнего
/// конца вектора подъёма к ближнему (при косом подъёме вверх-вправо —
/// с северо-востока на юго-запад, ближнее поверх), на здание сначала стены,
/// потом крыша. Кто кого кроет, решается по парам соседей — [`draw_order`],
/// там же и почему одного ключа на дом для этого мало. Фасадной полосы в этом
/// режиме нет — её заменяют настоящие стены; `tinted` включает рампу тона
/// крыш, как в `ShadowsTint`.
pub(super) fn extrusion_builder(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    detail: RoofDetail,
) -> MeshBuilder {
    let arches = arches_by_building(buildings, passages);
    let lean = Lean::of();
    let order = draw_order(buildings, lean);
    let runs = garage_runs(buildings);

    // стены и крыши едут одним мешем (painter's порядок общий), так что
    // рамку кровли несёт и он: у стен она нулевая, у крыш своя
    let mut builder = MeshBuilder::with_roof_coords();
    for index in order {
        let building = &buildings[index];
        let look = look_of(building, runs.get(&index));
        let color = roof_color(building, &look, detail.tinted);
        // арки вырезаются из стен по-настоящему: сквозь проём видны нижние
        // слои — дорога, идущая сквозь дом, и всё, что движок рисует под ней
        let openings = arches
            .get(&index)
            .map(|passages| {
                let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
                arch_openings(building, passages, lift, -lean.dir())
            })
            .unwrap_or_default();
        push_house_with_arches(
            &mut builder,
            building,
            &look,
            &wall_of_run(building, runs.get(&index)),
            color,
            RoofShape::Auto,
            detail.clutter,
            &openings,
        );
    }
    builder
}

/// Один дом в 2.5D: видимые стены, потом крыша заказанной формы и её
/// оборудование. Возвращает форму, которая **на самом деле** легла в меш, —
/// заказанной она равна не всегда: на негодном контуре скатная крыша не
/// строится, и дом остаётся с плоской.
///
/// Публично, потому что тем же вызовом строят свои дома витрины: форму,
/// материал кровли и цвет перебирает `roof_gallery` ([`RoofShape`],
/// [`RoofLook::new`]), облицовку стен — `wall_gallery` ([`WallLook::new`]), а
/// геометрия, скаты и оборудование обязаны остаться игровыми. Арок у витрин
/// нет — их знает только городская ветка ([`push_house_with_arches`]).
///
/// `wall` — вход, а не вывод из дома, по той же причине, по какой входом стала
/// `RoofLook`: посевом до всякого сочетания материала с высотой не добраться
/// (витраж не выпадает частному дому), а витрина обязана показать их все. Что
/// выбрала бы сама игра, отвечает [`wall_of`].
pub fn push_house(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    look: &RoofLook,
    wall: &WallLook,
    color: Srgba,
    shape: RoofShape,
    clutter: bool,
) -> RoofShape {
    push_house_with_arches(builder, building, look, wall, color, shape, clutter, &[])
}

/// Облицовка, которую игра выбрала бы этому дому. Витринам — чтобы не
/// повторять у себя деление высоты на высоту этажа, городу — чтобы не звать
/// `wall_look` мимо [`storeys_of`].
pub fn wall_of(building: &PolyArea) -> WallLook {
    wall_look(building, storeys_of(building))
}

/// То же, но с оглядкой на гаражный прогон: бокс, вошедший в него, одет в
/// **ворота** ([`WallKind::GarageDoors`]) — по створке на ячейку, и ячейка тут
/// его собственная, размером с бокс. Кровля этого дома берётся от прогона
/// ровно так же ([`look_of`]), и обе стороны коробки говорят тогда одно и то
/// же: сверху гребёнка боксов, сбоку ряд ворот.
fn wall_of_run(building: &PolyArea, run: Option<&GarageRun>) -> WallLook {
    match run {
        Some(run) => run_wall_look(run),
        None => wall_of(building),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_house_with_arches(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    look: &RoofLook,
    wall: &WallLook,
    color: Srgba,
    shape: RoofShape,
    clutter: bool,
    openings: &[ArchOpening],
) -> RoofShape {
    // этажи считаются от настоящей высоты дома, а не от нарисованной: подъём
    // сжимает стену вместе с ними
    let storeys = storeys_of(building);
    let facade_color = wall.base;
    let lean = Lean::of();
    let lift_dir = lean.dir();
    // через тот же хелпер, что и оверлей дверей, — иначе они разъедутся
    let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
    // длинная ось плана — одна на дом, и считается она только там, где может
    // что-то решить: перебор рёбер в `min_area_rect` квадратичный, а на складе,
    // частном доме и малоэтажке торец от фасада всё равно ничем не отличается
    let long_axis = balcony_house(building, wall.kind, storeys)
        .then(|| plan_long_axis(&building.outer))
        .flatten();
    builder.set_roof(None);

    // видимы стены рёбер, смотрящих против подъёма: при сдвиге
    // вверх-вправо — южные и западные; двор добавляет к ним внутреннюю стену
    // своей дальней стороны — ту, чья наружная (для кольца дыры) нормаль
    // смотрит по подъёму
    let seed = building_seed(building);
    // у разрезанного гаражного контура стены берутся по куску за раз: хорда
    // разреза рассекает ребро, и целой стене досталась бы сетка одного куска
    // на обе половины
    let mut walls = match look.run.as_ref().filter(|run| run.rects.len() > 1) {
        Some(run) => garage_walls(run, &building.outer, -lift_dir),
        None => silhouette_edges(&building.outer, -lift_dir),
    };
    for hole in &building.holes {
        walls.extend(silhouette_edges(hole, lift_dir));
    }
    // ...и кладутся они по глубине, а не по обходу контура: у дома со
    // ступенчатым фасадом соседние стены перекрываются на экране
    for index in wall_order(&walls, lean, lift) {
        let (a, b) = walls[index];
        let span = WallSpan::new(a, b, lift, storeys, long_axis);
        let (bottom, top) = wall_colors(facade_color, a, b, lift_dir);
        // гаражная стена считается в ячейках своего куска: ворота обязаны
        // встать под собственный шов кровли, а на торце их нет вовсе
        let cells = garage_cells(building, look, wall, &span)
            .unwrap_or_else(|| wall_cells(building, wall, &span));
        push_wall_with_openings(builder, &span, &cells, openings, bottom, top);
        // вход ложится поверх стены, которой он принадлежит, — порядок кладки
        // внутри дома и есть его глубина
        push_doors(builder, building, wall, &span, &cells, openings, bottom);
    }
    builder.set_roof(None);

    let chimney_on = |builder: &mut MeshBuilder, ridge, ridge_offset| {
        if clutter {
            let chimney: Vec<_> = ridge_chimney(building, look, ridge, lift + ridge_offset)
                .into_iter()
                .collect();
            push_items(builder, &chimney, Some(lean), color);
        }
    };
    match roofing_of(shape, building, lift, |rise| lean.ridge(rise), color, seed) {
        Roofing::Gable(roof) => {
            // фронтон — верх торцевой стены, видим по тому же правилу, что
            // и стена под ним: наружная нормаль торца смотрит против подъёма
            for ((a, b), apex) in roof.gables {
                let edge = b - a;
                if Vec2::new(edge.y, -edge.x).dot(-lift_dir) <= 0.0 {
                    continue;
                }
                let (_, top) = wall_colors(facade_color, a, b, lift_dir);
                // фронтон продолжает раму стены под ним — иначе рисунок рвался
                // бы ровно на карнизе, — но помечен как «над карнизом»: проёмов
                // на треугольнике нет, окно на нём резалось бы скатом
                let span = WallSpan::new(a, b, lift, storeys, long_axis);
                builder.set_wall(
                    wall_frame(building, wall, &span).map(|frame| frame.marked(WallMark::Solid)),
                );
                builder.push_polygon(&[a, b, apex], &[], top);
            }
            builder.set_roof(Some(look.frame));
            for (slope, slope_color) in roof.slopes {
                builder.push_quad(slope, slope_color);
            }
            chimney_on(builder, ridge_of(&roof), roof.ridge_offset);
            RoofShape::Gable
        }
        Roofing::Hip(roof) => {
            // у вальмы фронтонов нет — скаты сходятся со всех сторон, и
            // торцевая стена кончается на карнизе, как и боковая
            builder.set_roof(Some(look.frame));
            let ridge = hip_ridge_ends(&roof);
            let ridge_offset = roof.ridge_offset;
            push_hip(builder, &roof);
            chimney_on(builder, ridge, ridge_offset);
            RoofShape::Hip
        }
        Roofing::Flat => {
            let roof_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + lift).collect();
            let roof_holes: Vec<Vec<Vec2>> = building
                .holes
                .iter()
                .map(|hole| hole.iter().map(|p| *p + lift).collect())
                .collect();
            push_flat_roof(builder, look, &roof_outer, &roof_holes, color, lift);
            if clutter {
                let items = flat_roof_items(building, look, lift);
                push_items(builder, &items, Some(lean), color);
            }
            RoofShape::Flat
        }
    }
}

/// Рёбра кольца, чья наружная нормаль смотрит по `direction` — силуэт с
/// подветренной стороны. Обход кольца (CW/CCW) учитывается по знаковой
/// площади, так что результат от него не зависит.
pub(super) fn silhouette_edges(ring: &[Vec2], direction: Vec2) -> Vec<(Vec2, Vec2)> {
    if ring.len() < 3 {
        return Vec::new();
    }
    let orientation = signed_ring_area(ring).signum();
    let mut edges = Vec::new();
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let edge = b - a;
        // для CCW-кольца наружная нормаль ребра — правый перпендикуляр
        let outward = Vec2::new(edge.y, -edge.x) * orientation;
        if outward.dot(direction) > 0.0 {
            edges.push((a, b));
        }
    }
    edges
}
