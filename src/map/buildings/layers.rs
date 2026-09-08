//! Сборка мешей зданиевых слоёв: полоса фасада с крышами, длинные тени,
//! 2.5D-экструзия. Каждый билдер отдаёт готовый [`MeshBuilder`], а какие из
//! них спавнить в текущем режиме — решает `spawn_buildings` в родителе.

use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;

use super::arches::{arch_openings, arches_by_building, push_arches, push_wall_with_openings};
use super::clutter::{flat_roof_items, push_items, ridge_chimney};
use super::material::{RoofKind, RoofLook, building_seed, roof_look};
use super::roofs::{HipRoof, Roofing, roofing};
use super::{
    BuildingHeightMode, Lean, RoofDetail, building_center, extrusion_lift, facade_color,
    height_or_default, shade_by_light,
};
use crate::map::meshing::{MeshBuilder, Roof};
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, PolyArea, RoadLine};
use crate::map::{SHADOW_COLOR, shadow_dir, shadow_length_scale};

/// Доля реальной высоты, уходящая в полосу фасада. Рисовать все 60 м башни —
/// значит закрасить полквартала: карта сверху, а не изометрия. При 0.2
/// пятиэтажка (15 м) даёт прежние 3 м, и разница этажности всё равно читается.
const FACADE_SCALE: f32 = 0.2;
/// Границы полосы, м: сарай не должен потерять кромку, небоскрёб — накрыть
/// соседний квартал.
const FACADE_HEIGHT_RANGE: RangeInclusive<f32> = 1.5..=12.0;

/// Границы длины тени, м: у сарая тень обязана остаться заметной, у башни —
/// не накрыть полкарты. Сама длина считается из высоты солнца
/// (`map::shadow_length_scale`): пятиэтажка (15 м) отбрасывает 9 м — тень
/// перечёркивает типичную улицу (8–16 м), но не глотает соседний квартал.
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
const PENUMBRA_WIDTH: f32 = 1.0;

/// Ширина парапета, м: у плоской кровли по контуру идёт бортик, и с воздуха
/// он читается светлой каймой на солнечных гранях и тёмной на теневых.
const PARAPET_WIDTH: f32 = 0.7;
/// Насколько бортик светлее кровли на солнечной грани и темнее на теневой.
/// Жёстче ската (`SLOPE_*_MIX`) и мягче стены: это узкая полоска бетона,
/// её задача — очертить кровлю, а не спорить со стеной.
const PARAPET_LIT_MIX: f32 = 0.24;
const PARAPET_SHADED_MIX: f32 = 0.20;

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

/// Парапет по контуру плоской кровли: кайма внутрь контура, светлая на
/// солнечных гранях и тёмная на теневых. Для дырки (двора) — наружу от её
/// кольца, то есть тоже вглубь кровли, и нормаль там смотрит во двор.
///
/// Фактуру бортик не несёт (`set_roof(None)`): шов рулона поперёк парапета
/// читался бы как трещина, а бетонный отлив на снимке и правда ровный.
fn push_parapet(builder: &mut MeshBuilder, ring: &[Vec2], hole: bool, base: Srgba) {
    let orientation = signed_ring_area(ring).signum() * if hole { -1.0 } else { 1.0 };
    let inner = base.into();
    builder.set_roof(None);
    builder.push_inset_band_with(ring, PARAPET_WIDTH, hole, |a, b| {
        let edge = b - a;
        let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero() * orientation;
        let cap = shade_by_light(base, outward, PARAPET_LIT_MIX, PARAPET_SHADED_MIX);
        (cap.into(), inner)
    });
}

/// Рамка **стены** для шейдера: ось — сама стена, поэтому «поперёк» в
/// шейдере оказывается направлением вверх по ней, и межэтажные швы ложатся
/// параллельно карнизу. Начало отсчёта общее для всей карты, а не своё у
/// каждой стены: фаза шва от этого произвольна, но одинакова у смежных стен
/// одного дома — на угле шов не разрывается, а это единственное, что видно.
fn wall_frame(a: Vec2, b: Vec2, seed: u32) -> Option<Roof> {
    Some(Roof {
        axis: (b - a).try_normalize()?,
        material: RoofKind::Wall.code(),
        seed: (seed >> 12 & 0xff) as f32 / 255.0,
    })
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

/// Плоская кровля: заливка контура (с дворами-дырками) под фактуру своего
/// материала и парапет по краю, если материал мягкий. Ровно это игра кладёт
/// на всякий дом без двускатной крыши — и в плоских режимах, где `outer` это
/// сам контур, и в 2.5D, где он уже поднят на высоту стен.
///
/// Публично, потому что тем же вызовом рисует свои дома витрина
/// `roof_gallery`: материал и цвет она перебирает сама ([`RoofLook::new`]), а
/// геометрия обязана остаться игровой.
pub fn push_flat_roof(
    builder: &mut MeshBuilder,
    look: &RoofLook,
    outer: &[Vec2],
    holes: &[Vec<Vec2>],
    color: Srgba,
) {
    builder.set_roof(Some(look.frame));
    builder.push_polygon(outer, holes, color.into());
    if look.kind.has_parapet() {
        push_parapet(builder, outer, false, color);
        for hole in holes {
            push_parapet(builder, hole, true, color);
        }
    }
}

/// Тон стены `a→b` по её повороту к свету: (низ, верх). Стена видима,
/// значит её настоящая нормаль смотрит против подъёма — это и выбирает
/// сторону перпендикуляра, обход кольца тут ни при чём.
pub(super) fn wall_colors(
    facade: Color,
    a: Vec2,
    b: Vec2,
    lift_dir: Vec2,
) -> (LinearRgba, LinearRgba) {
    let edge = b - a;
    let mut normal = Vec2::new(edge.y, -edge.x).normalize_or_zero();
    if normal.dot(lift_dir) > 0.0 {
        normal = -normal;
    }
    let bottom = shade_by_light(facade.to_srgba(), normal, WALL_LIT_MIX, WALL_SHADED_MIX);
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
    let mut facades = MeshBuilder::default();
    // крыши рисует `RoofMaterial`, и рамку кровли ему даёт этот атрибут
    let mut roofs = MeshBuilder::with_roof_coords();
    for (index, building) in buildings.iter().enumerate() {
        let facade_color = facade_color(building);

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
        facades.push_polygon(&facade_outer, &facade_holes, facade_color.to_linear());
        // крыши — отдельный слой поверх фасадов, так что вырезать проём из
        // полосы достаточно: над аркой крыша останется целой сама собой
        if let Some(passages) = arches.get(&index) {
            push_arches(&mut facades, building, passages, offset);
        }
        // двускатная крыша в плоском режиме — два ската разного тона в
        // одной плоскости: конёк не поднят, но дом уже не коробка
        let look = roof_look(building);
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
                    items.extend(ridge_chimney(&look, ridge_of(&roof)));
                }
            }
            Roofing::Hip(roof) => {
                roofs.set_roof(Some(look.frame));
                push_hip(&mut roofs, &roof);
                if detail.clutter {
                    items.extend(ridge_chimney(&look, hip_ridge_ends(&roof)));
                }
            }
            Roofing::Flat => {
                push_flat_roof(&mut roofs, &look, &building.outer, &building.holes, color);
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
    for building in buildings {
        let length = (height_or_default(building) * shadow_length_scale())
            .clamp(*SHADOW_LENGTH_RANGE.start(), *SHADOW_LENGTH_RANGE.end());
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
        // внутрь просвета от каждой дырки: `push_inset_band` выбирает сторону
        // по площади самого кольца, так что просвету нужно `outside: false`,
        // иначе кайма ляжет на уже залитое тело и обведёт дырку двойной
        // темнотой вместо растушёвки. Каймы соседних фигур могут наложиться,
        // но обе сходят в ноль, и удвоение выходит слабее самой тени
        builder.push_inset_band(&outer, PENUMBRA_WIDTH, true, color, fade);
        for hole in &holes {
            builder.push_inset_band(hole, PENUMBRA_WIDTH, false, color, fade);
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
/// потом крыша. Фасадной полосы в этом режиме нет — её заменяют настоящие
/// стены; `tinted` включает рампу тона крыш, как в `ShadowsTint`.
pub(super) fn extrusion_builder(
    buildings: &[PolyArea],
    passages: &[RoadLine],
    detail: RoofDetail,
) -> MeshBuilder {
    let arches = arches_by_building(buildings, passages);
    let mut order: Vec<usize> = (0..buildings.len()).collect();
    order.sort_by(|&a, &b| {
        let depth = |building: &PolyArea| Lean::depth(building_center(building));
        depth(&buildings[b]).total_cmp(&depth(&buildings[a]))
    });

    // стены и крыши едут одним мешем (painter's порядок общий), так что
    // рамку кровли несёт и он: у стен она нулевая, у крыш своя
    let mut builder = MeshBuilder::with_roof_coords();
    for index in order {
        let building = &buildings[index];
        let facade_color = facade_color(building);
        let lean = Lean::of();
        let lift_dir = lean.dir();
        // через тот же хелпер, что и оверлей дверей, — иначе они разъедутся
        let lift = extrusion_lift(building, BuildingHeightMode::Extrusion);
        builder.set_roof(None);

        // арки вырезаются из стен по-настоящему: сквозь проём видны нижние
        // слои — дорога, идущая сквозь дом, и всё, что движок рисует под ней
        let openings = arches
            .get(&index)
            .map(|passages| arch_openings(building, passages, lift, -lift_dir))
            .unwrap_or_default();
        // видимы стены рёбер, смотрящих против подъёма: при сдвиге
        // вверх-вправо — южные и западные
        let seed = building_seed(building);
        for (a, b) in silhouette_edges(&building.outer, -lift_dir) {
            let (bottom, top) = wall_colors(facade_color, a, b, lift_dir);
            builder.set_roof(wall_frame(a, b, seed));
            push_wall_with_openings(&mut builder, a, b, lift, &openings, bottom, top);
        }
        // двор: видима внутренняя стена его дальней стороны — та, чья
        // наружная (для кольца дыры) нормаль смотрит по подъёму
        for hole in &building.holes {
            for (a, b) in silhouette_edges(hole, lift_dir) {
                let (bottom, top) = wall_colors(facade_color, a, b, lift_dir);
                builder.set_roof(wall_frame(a, b, seed));
                push_wall_with_openings(&mut builder, a, b, lift, &openings, bottom, top);
            }
        }
        builder.set_roof(None);

        let look = roof_look(building);
        let color = roof_color(building, &look, detail.tinted);
        let chimney_on = |builder: &mut MeshBuilder, ridge| {
            if detail.clutter {
                let chimney: Vec<_> = ridge_chimney(&look, ridge).into_iter().collect();
                push_items(builder, &chimney, Some(lean), color);
            }
        };
        match roofing(
            building,
            lift,
            |rise| lean.ridge(rise),
            color,
            building_seed(building),
        ) {
            Roofing::Gable(roof) => {
                // фронтон — верх торцевой стены, видим по тому же правилу, что
                // и стена под ним: наружная нормаль торца смотрит против подъёма
                for ((a, b), apex) in roof.gables {
                    let edge = b - a;
                    if Vec2::new(edge.y, -edge.x).dot(-lift_dir) <= 0.0 {
                        continue;
                    }
                    let (_, top) = wall_colors(facade_color, a, b, lift_dir);
                    builder.push_polygon(&[a, b, apex], &[], top);
                }
                builder.set_roof(Some(look.frame));
                for (slope, slope_color) in roof.slopes {
                    builder.push_quad(slope, slope_color);
                }
                chimney_on(&mut builder, ridge_of(&roof));
                continue;
            }
            Roofing::Hip(roof) => {
                // у вальмы фронтонов нет — скаты сходятся со всех сторон, и
                // торцевая стена кончается на карнизе, как и боковая
                builder.set_roof(Some(look.frame));
                let ridge = hip_ridge_ends(&roof);
                push_hip(&mut builder, &roof);
                chimney_on(&mut builder, ridge);
                continue;
            }
            Roofing::Flat => {}
        }

        let roof_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + lift).collect();
        let roof_holes: Vec<Vec<Vec2>> = building
            .holes
            .iter()
            .map(|hole| hole.iter().map(|p| *p + lift).collect())
            .collect();
        push_flat_roof(&mut builder, &look, &roof_outer, &roof_holes, color);
        if detail.clutter {
            let items = flat_roof_items(building, &look, lift);
            push_items(&mut builder, &items, Some(lean), color);
        }
    }
    builder
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
