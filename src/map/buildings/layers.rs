//! Сборка мешей зданиевых слоёв: полоса фасада с крышами и 2.5D-экструзия.
//! Каждый билдер отдаёт готовый [`MeshBuilder`], а какие из них строить в
//! текущем режиме — решает `mesh_buildings` в родителе. Тени — не здесь: оба
//! теневых слоя и развёртки силуэтов живут в [`super::shadows`].
//!
//! Субъекты — в подмодулях: [`walls`] — стены, двери, балконы и фриз вывески,
//! [`garages`] — стены и кровля гаражного прогона, [`roofing`] — кровельная
//! сборка и вид дома целиком. Здесь — оба билдера слоя, сборка дома целиком
//! и метрика ячейки стены.

use std::ops::RangeInclusive;

use bevy::color::Mix;
use bevy::prelude::*;

use super::arches::{
    ArchOpening, TunnelWall, arch_openings, push_arches, push_tunnel_walls,
    push_wall_with_openings, tunnel_walls,
};
use super::classify::classify;
use super::garages::BAY;
use super::material::{RoofLook, WallKind, WallLook, wall_look};
use super::order::wall_order;
use super::roofs::{RoofMix, RoofShape};
use super::temples::{Crown, Sanctuary, push_crowns};
use super::{
    CityContext, HouseFootprint, Lean, RoofDetail, building_center, height_of, lift_of_footprint,
};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::signed_ring_area;
use crate::map::osm::{AreaKind, PolyArea};

mod garages;
mod roofing;
mod walls;

use self::garages::{garage_cells, garage_walls};
use self::roofing::{HouseLook, house_look, push_roofing};
use self::walls::{balcony_house, plan_long_axis, push_brand_band, push_doors, wall_cells};
#[cfg(test)]
pub(super) use walls::door_size;
pub(super) use walls::{BALCONY_STOREYS_MIN, WallSpan, wall_colors};

/// Доля реальной высоты, уходящая в полосу фасада. Рисовать все 60 м башни —
/// значит закрасить полквартала: карта сверху, а не изометрия. При 0.2
/// пятиэтажка (15 м) даёт прежние 3 м, и разница этажности всё равно читается.
const FACADE_SCALE: f32 = 0.2;
/// Границы полосы, м: сарай не должен потерять кромку, небоскрёб — накрыть
/// соседний квартал.
const FACADE_HEIGHT_RANGE: RangeInclusive<f32> = 1.5..=12.0;

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

/// Цвет крыши: базовый — из палитры её материала ([`super::material::roof_look`]), при
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
/// ряда — **бокс** ([`super::garages::BAY`]): ворота стоят по одной на бокс, и мерить
/// их стену панелями значило бы разойтись с гребёнкой боксов на её же кровле.
fn cell_width(kind: WallKind) -> f32 {
    match kind {
        WallKind::GarageDoors => BAY,
        WallKind::Sacred => SACRED_BAY,
        WallKind::BigBox => BIG_BOX_BAY,
        _ => PANEL_WIDTH,
    }
}

/// Кассетный пролёт стены гипермаркета, м — вместо жилой панели. Фасад
/// коробки собран из крупных композитных кассет по колоннам каркаса, и шаг
/// колонн у торгового зала это шесть-двенадцать метров, а не три. С
/// панельной ячейкой стодвадцатиметровая стена «Магнита» делилась на сорок
/// столбцов, и ленточное окно в каждом читалось рядом квартирных.
const BIG_BOX_BAY: f32 = 7.0;

/// Ячейка храмовой стены, м: простенок с одним высоким окном шире жилой
/// панели.
const SACRED_BAY: f32 = 4.0;

/// Сколько ячеек встанет на стену такой длины — целое число, и оно же делитель
/// её собственной сетки: ячейка это `length / wall_columns(length, kind)`.
/// Ширина ячейки — по облицовке ([`cell_width`]), поэтому у гаражного ряда
/// счёт идёт боксами, и тот же счёт уезжает в
/// [`super::arches::WallCells::panel`]: заплаты вокруг арок и дверей кроятся
/// ровно тем же шагом, что и ворота.
///
/// Наружу — ради проверки: тест меряет клетку тем же делителем, что и стена
/// ([`WallSpan::columns`]), и убеждается, что ни один проём не разрезал её
/// пополам. Самим заплатам вокруг проёмов функция не нужна — им размер клетки
/// приезжает готовым в [`super::arches::WallCells`].
pub(super) fn wall_columns(length: f32, kind: WallKind) -> f32 {
    (length / cell_width(kind)).round().max(1.0)
}

/// Фасадная полоса + крыши (режимы Facade / Shadows / ShadowsTint).
pub(super) fn facade_and_roof_builders(
    buildings: &[PolyArea],
    detail: RoofDetail,
    ctx: &CityContext<'_>,
    footprints: &[Option<HouseFootprint>],
) -> (MeshBuilder, MeshBuilder) {
    let mut facades = MeshBuilder::default();
    // крыши рисует `RoofMaterial`, и рамку кровли ему даёт этот атрибут
    let mut roofs = MeshBuilder::with_roof_coords();
    let mut placed = Vec::new();
    for (index, building) in buildings.iter().enumerate() {
        let fp = footprints[index];
        let house = house_look(building, ctx.runs.get(&index), fp);
        let class = classify(building, fp);
        // фактуры у плоской полосы нет — она идёт одним earcut-полигоном, — но
        // цвет у неё тот же, что был бы у настоящей стены в 2.5D
        let facade_color = house.facade;
        let color = roof_color(building, &house.roof, detail.tinted);
        placed.extend(
            ctx.sanctuary
                .crowns(index, building, facade_color, color, Vec2::ZERO, fp),
        );
        // часть на крыше храма — это барабан с главой, отдельная колокольня —
        // столп ярусами: коробки у них нет
        if ctx.sanctuary.boxless(index, building) {
            continue;
        }

        // фасад — тот же контур, сдвинутый вниз: тёмная кромка видна
        // только вдоль южных граней любого полигона. Сдвиг — по высоте из
        // OSM, так что этажность города видна прямо на карте
        let facade_height = (height_of(building, fp) * FACADE_SCALE)
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
        if let Some(passages) = ctx.arches.get(&index) {
            push_arches(&mut facades, building, passages, offset);
        }
        // Плоский режим — вырожденный случай общей сборки крыши ([`push_roofing`]):
        // подъёма нет, конёк не поднимается, фронтонов и слуховых окон нет —
        // но скатной дом всё равно не коробка: два ската разного тона в одной
        // плоскости
        push_roofing(
            &mut roofs,
            building,
            &house,
            color,
            RoofShape::Auto,
            detail.clutter,
            &class,
            Vec2::ZERO,
            None,
            fp,
        );
    }
    // венцы — после всех кровель: см. `temples::push_crowns`
    push_crowns(&mut roofs, &placed, None);
    (facades, roofs)
}

/// 2.5D-экструзия: painter's algorithm внутри одного меша — треугольники
/// растеризуются в порядке index-буфера, поэтому здания пишутся от дальнего
/// конца вектора подъёма к ближнему (при косом подъёме вверх-вправо —
/// с северо-востока на юго-запад, ближнее поверх), на здание сначала стены,
/// потом крыша. Кто кого кроет, решается по парам соседей — [`super::order::draw_order`],
/// там же и почему одного ключа на дом для этого мало.
///
/// `order` — вход, а не своё вычисление: тот же список нужен теням на кровлях,
/// чтобы вычесть из них тела соседей, нарисованных после цели
/// ([`DrawnBodies`]), и строит его один раз на сборку слоя вызывающий.
///
/// Фасадной полосы в этом режиме нет — её заменяют настоящие стены; `tinted`
/// включает рампу тона крыш, как в `ShadowsTint`.
/// Один дом, собранный в потоке ([`extrusion_builder`]): клетка своего
/// центра, собственный меш и венцы, форма крыши для [`RoofMix`]. `None` —
/// коробки у дома нет (часть храма): в скатную когорту такие не попадали и
/// теперь не попадут.
struct HousePart {
    cell: IVec2,
    builder: MeshBuilder,
    crowns: Vec<(Crown, Vec2)>,
    drawn: Option<RoofShape>,
}

pub(super) fn extrusion_builder(
    buildings: &[PolyArea],
    detail: RoofDetail,
    order: &[usize],
    ctx: &CityContext<'_>,
    footprints: &[Option<HouseFootprint>],
) -> (Vec<MeshBuilder>, RoofMix) {
    use bevy::platform::collections::HashMap;

    use crate::map::parallel::in_parallel;
    use crate::map::surface::MAP_CHUNK;

    let lean = Lean::of();

    // стены и крыши едут одним мешем (painter's порядок общий), так что
    // рамку кровли несёт и он: у стен она нулевая, у крыш своя.
    //
    // Меш режется **домами по клеткам карты** ([`MAP_CHUNK`]), чтобы кадр
    // ближнего зума не рисовал весь город: разрез собранного меша по
    // треугольникам разъял бы дом через границу клетки — его собственный
    // порядок (крыша поверх своих стен) погиб первым же, а порядок соседей
    // держится на сортировке домов, которую разрез тоже ломает. Дом идёт в
    // клетку своего центра целиком, внутри клетки порядок домов — исходный
    // (по `order`), а **между** клетками порядок задаёт глубина их центров:
    // у пары «дом кроет соседа» кроющий стоит ниже-левее кроемого — вся его
    // геометрия лежит между контуром и контуром, сдвинутым вверх-вправо
    // ([`Lean`]), до чужого контура она дотягивается только снизу, — и клетка
    // ниже-левее рисуется позже, как и положено ближнему.
    //
    // Дом от дома не зависит: каждый собирается в своём потоке, и
    // `in_parallel` возвращает результаты в порядке `order` — painter's
    // порядок живёт в порядке частей, клетки раскладывает слияние ниже.
    let parts: Vec<HousePart> = in_parallel(order, |&index| {
        let building = &buildings[index];
        let cell = (building_center(building) / MAP_CHUNK).floor().as_ivec2();
        let fp = footprints[index];
        let house = house_look(building, ctx.runs.get(&index), fp);
        let color = roof_color(building, &house.roof, detail.tinted);
        let lift = lift_of_footprint(building, fp);
        let crowns = ctx
            .sanctuary
            .crowns(index, building, house.wall.base, color, lift, fp);
        // часть на крыше храма — барабан с главой, отдельная колокольня — столп
        // ярусами: коробки от земли у них нет
        if ctx.sanctuary.boxless(index, building) {
            return HousePart {
                cell,
                builder: MeshBuilder::with_roof_coords(),
                crowns,
                drawn: None,
            };
        }
        // арки вырезаются из стен по-настоящему: сквозь проём видны нижние
        // слои — дорога, идущая сквозь дом, и всё, что движок рисует под ней
        let (openings, tunnel) = ctx
            .arches
            .get(&index)
            .map(|passages| {
                let openings = arch_openings(building, passages, lift, -lean.dir());
                // стенки проезда видны только сквозь проём
                let tunnel = match openings.is_empty() {
                    true => Vec::new(),
                    false => tunnel_walls(building, passages, lift, -lean.dir()),
                };
                (openings, tunnel)
            })
            .unwrap_or_default();
        let mut builder = MeshBuilder::with_roof_coords();
        let drawn = push_house_with_arches(
            &mut builder,
            building,
            &house,
            color,
            RoofShape::Auto,
            detail.clutter,
            &openings,
            &tunnel,
            fp,
        );
        HousePart {
            cell,
            builder,
            crowns,
            drawn: Some(drawn),
        }
    });

    let mut mix = RoofMix::default();
    let mut by_cell: HashMap<IVec2, Vec<&HousePart>> = HashMap::new();
    for (place, part) in parts.iter().enumerate() {
        if let Some(drawn) = part.drawn {
            let class = classify(&buildings[order[place]], footprints[order[place]]);
            mix.add(&class, drawn);
        }
        by_cell.entry(part.cell).or_default().push(part);
    }
    // группировка по ключу клетки — детерминированный вход потокам слияния;
    // итоговый порядок клеток всё равно ставит глубина
    let mut cells: Vec<(IVec2, Vec<&HousePart>)> = by_cell.into_iter().collect();
    cells.sort_unstable_by_key(|&(cell, _)| (cell.x, cell.y));
    let mut merged: Vec<(IVec2, MeshBuilder)> = in_parallel(&cells, |(cell, parts)| {
        let mut builder = MeshBuilder::with_roof_coords();
        let mut crowns = Vec::new();
        for part in parts {
            builder.append(&part.builder);
            crowns.extend(part.crowns.iter().copied());
        }
        // венцы — после домов своей клетки: венец выше любой кровли вокруг, и
        // соседям из ближних клеток до него не дотянуться — их подъём кончается
        // на карнизе, до венца, стоящего над серединой чужой коробки, они
        // дотягиваются, только стоя к ней вплотную, а такое редкость (храм —
        // большая коробка). Сортировку внутри венцов делает `push_crowns`.
        push_crowns(&mut builder, &crowns, Some(lean));
        (*cell, builder)
    });
    // painter's порядок между клетками: глубина центра клетки, убывание —
    // дальние раньше, ближние поверх. Один и тот же ключ ставит и дома в
    // клетке, и сами клетки (`order` отсортирован по глубине домов).
    let depth = |cell: IVec2| lean.depth((cell.as_vec2() + 0.5) * MAP_CHUNK);
    merged.sort_by(|a, b| depth(b.0).total_cmp(&depth(a.0)));
    // кусок 0 — самый дальний: на этом держится лестница
    // `building_chunk_z` (модуль `buildings`), и перевёрнутая сортировка
    // молча перепутала бы порядок клеток между чанками
    debug_assert!(
        merged.len() < 2 || depth(merged[0].0) >= depth(merged[merged.len() - 1].0),
        "куски экструзии обязаны идти от дальней клетки к ближней"
    );
    (
        merged
            .into_iter()
            .map(|(_, builder)| builder)
            .collect::<Vec<_>>(),
        mix,
    )
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
    // витрина зовёт дом сама, без сборщика слоя с его готовыми пятнами:
    // пятно считается здесь, один раз на дом
    let fp = HouseFootprint::of(building);
    let house = HouseLook {
        roof: look.clone(),
        wall: *wall,
        facade: wall.base,
    };
    let built = push_house_with_arches(
        builder,
        building,
        &house,
        color,
        shape,
        clutter,
        &[],
        &[],
        fp,
    );
    // дом витрины стоит один — частей храма вокруг нет, и венец у него свой
    let crowns = Sanctuary::of(std::slice::from_ref(building), std::slice::from_ref(&fp)).crowns(
        0,
        building,
        wall.base,
        color,
        lift_of_footprint(building, fp),
        fp,
    );
    push_crowns(builder, &crowns, Some(Lean::of()));
    built
}

/// Облицовка, которую игра выбрала бы этому дому. Витринам — чтобы не
/// повторять у себя ни деление высоты на высоту этажа, ни развилки
/// классификатора: оба шага (`wall_look` ведёт их изнутри) остаются его.
/// Пятна у витрины нет — классификатор считает его сам, как и у прочих
/// внешних вызовов.
pub fn wall_of(building: &PolyArea) -> WallLook {
    wall_look(building, None)
}

#[allow(clippy::too_many_arguments)]
fn push_house_with_arches(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    house: &HouseLook,
    color: Srgba,
    shape: RoofShape,
    clutter: bool,
    openings: &[ArchOpening],
    tunnel: &[TunnelWall],
    fp: Option<HouseFootprint>,
) -> RoofShape {
    // этажи считаются от настоящей высоты дома, а не от нарисованной: подъём
    // сжимает стену вместе с ними
    let class = classify(building, fp);
    let storeys = class.storeys();
    let facade_color = house.wall.base;
    let lean = Lean::of();
    let lift_dir = lean.dir();
    // через тот же хелпер, что и оверлей дверей, — иначе они разъедутся
    let lift = lift_of_footprint(building, fp);
    // длинная ось плана — одна на дом, и спрашивается она только там, где
    // может что-то решить: на складе, частном доме и малоэтажке торец от
    // фасада всё равно ничем не отличается
    let long_axis = balcony_house(building, house.wall.kind, storeys)
        .then(|| plan_long_axis(fp))
        .flatten();
    builder.set_roof(None);
    // стенки проезда — позади всех стен дома: сквозь проём видна только их
    // часть, остальное накроют простенки, перемычка и крыша
    push_tunnel_walls(builder, tunnel, facade_color, lift_dir);

    // видимы стены рёбер, смотрящих против подъёма: при сдвиге
    // вверх-вправо — южные и западные; двор добавляет к ним внутреннюю стену
    // своей дальней стороны — ту, чья наружная (для кольца дыры) нормаль
    // смотрит по подъёму
    // у разрезанного гаражного контура стены берутся по куску за раз: хорда
    // разреза рассекает ребро, и целой стене досталась бы сетка одного куска
    // на обе половины
    let mut walls = match house.roof.run.as_ref().filter(|run| run.rects.len() > 1) {
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
        let cells = garage_cells(building, &house.roof, &house.wall, &span)
            .unwrap_or_else(|| wall_cells(building, &house.wall, &span));
        push_wall_with_openings(builder, &span, &cells, openings, bottom, top);
        // фриз вывески — поверх своей стены и до входа
        push_brand_band(builder, building, &house.wall, &span, &cells, lift_dir);
        // вход ложится поверх стены, которой он принадлежит, — порядок кладки
        // внутри дома и есть его глубина
        push_doors(
            builder,
            building,
            &house.wall,
            &span,
            &cells,
            openings,
            bottom,
        );
    }
    builder.set_roof(None);

    push_roofing(
        builder,
        building,
        house,
        color,
        shape,
        clutter,
        &class,
        lift,
        Some(lean),
        fp,
    )
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
