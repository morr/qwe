//! Оборудование на кровле: машинные помещения лифтов, вентшахты, блоки
//! кондиционеров, зенитные фонари промышленных корпусов, трубы на коньках.
//!
//! Ровная крыша с воздуха не бывает пустой, и именно эта мелочь — коробки со
//! своими короткими тенями — делает снимок снимком, а не заливкой. Ставится
//! всё детерминированно: ГПСЧ засеян тем же посевом от первой вершины
//! контура, что выбирает материал кровли ([`super::material`]), так что
//! переключение режима высот или зума не переставляет оборудование.
//!
//! Что где стоит — решает материал кровли, а не назначение: мягкая плоская
//! (битум, гравий, мембрана) несёт машинное помещение и вентшахты, профлист
//! большого корпуса — ленты зенитных фонарей, скатная — трубу на коньке.
//! Разметка идёт в раме дома (длинная ось контура), с отступом от края, и
//! каждая коробка проверяется на попадание в контур — у Г-образного дома
//! прямоугольник рамы торчит наружу.
//!
//! Видно всё это только вблизи: [`super::BuildingZoomBucket`] снимает
//! оборудование целиком, когда метр кровли становится мельче пары пикселей —
//! иначе субпиксельные коробки мерцают при панораме.

use bevy::color::Mix;
use bevy::prelude::*;

use super::Lean;
use super::layers::{silhouette_edges, wall_colors};
use super::material::{RoofKind, RoofLook};
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{point_in_area, signed_ring_area};
use crate::map::osm::{BuildingUse, PolyArea};
use crate::map::seed::Lcg;
use crate::map::{shadow_dir, shadow_length_scale};

/// Сколько мест перебрать, прежде чем отказаться от коробки. Одна попытка
/// на узком корпусе почти всегда промахивалась: машинное помещение 5 × 3.5 м
/// целиком укладывается в двенадцатиметровый дом лишь в узкой полосе.
const PLACE_TRIES: usize = 6;

/// Отступ оборудования от края кровли, м, и доля меньшей стороны дома —
/// на узком корпусе метровый отступ съедает всю крышу.
const EDGE_MARGIN: f32 = 1.6;
const EDGE_MARGIN_SHARE: f32 = 0.18;

/// Машинное помещение лифта и выход на кровлю: одна коробка на дом крупнее
/// этого пятна, вторая — на дом вчетверо крупнее.
const PENTHOUSE_MIN_AREA: f32 = 400.0;
const PENTHOUSE_SECOND_AREA: f32 = 1600.0;
const PENTHOUSE_SIZE: Vec2 = Vec2::new(5.0, 3.5);
const PENTHOUSE_HEIGHT: f32 = 3.0;

/// Вентшахты и дефлекторы: одна на столько квадратов кровли.
const VENT_AREA_PER: f32 = 300.0;
const VENT_MAX: usize = 10;
const VENT_SIZE: Vec2 = Vec2::new(1.1, 1.1);
const VENT_HEIGHT: f32 = 1.2;

/// Блоки кондиционеров — у торговых и казённых зданий, рядком.
const UNIT_AREA_PER: f32 = 500.0;
const UNIT_MAX: usize = 6;
const UNIT_SIZE: Vec2 = Vec2::new(1.8, 1.0);
const UNIT_HEIGHT: f32 = 0.9;

/// Зенитные фонари: ленты вдоль длинной оси большого корпуса под профлистом.
const SKYLIGHT_MIN_AREA: f32 = 700.0;
const SKYLIGHT_WIDTH: f32 = 2.2;
const SKYLIGHT_LENGTH_SHARE: f32 = 0.62;
const SKYLIGHT_HEIGHT: f32 = 0.5;
/// Вторая лента — если поперёк дома есть куда её положить.
const SKYLIGHT_SECOND_WIDTH: f32 = 22.0;

/// Труба на коньке скатной крыши.
const CHIMNEY_SIZE: Vec2 = Vec2::new(0.8, 0.8);
const CHIMNEY_HEIGHT: f32 = 1.4;

/// Цвета оборудования: оцинковка и бетон, всё в холодном сером. Фонарь
/// светлее и голубее — это стекло; труба кирпичная.
const EQUIPMENT_TOP: Color = Color::srgb(0.60, 0.60, 0.59);
const EQUIPMENT_WALL: Color = Color::srgb(0.50, 0.50, 0.50);
const PENTHOUSE_TOP: Color = Color::srgb(0.55, 0.54, 0.52);
const PENTHOUSE_WALL: Color = Color::srgb(0.62, 0.61, 0.58);
const SKYLIGHT_TOP: Color = Color::srgb(0.74, 0.77, 0.79);
const SKYLIGHT_WALL: Color = Color::srgb(0.60, 0.62, 0.64);
const CHIMNEY_TOP: Color = Color::srgb(0.38, 0.31, 0.28);
const CHIMNEY_WALL: Color = Color::srgb(0.46, 0.36, 0.31);
/// Насколько тень оборудования темнее самой кровли. Тень непрозрачная —
/// слой зданий рисуется без блендинга, и полупрозрачная тут не смешалась бы.
const CLUTTER_SHADOW_MIX: f32 = 0.30;

/// Коробка на крыше: основание (CCW, уже в координатах нарисованной кровли),
/// настоящая высота над ней, чем красить и докуда пускать тень.
///
/// `reach` считается **в момент раскладки**, а не отрисовки, и это не
/// оптимизация, а развязка: он зависит от контура дома и от сдвига кровли, а
/// у того, кто рисует, на руках только сам предмет. Пока вылет был аргументом
/// [`push_items`], здание и сдвиг ехали туда третьим и четвёртым слагаемым
/// одной тройки с `items`, и несогласованная пара давала не панику, а тихо
/// неверную обрезку тени.
pub(super) struct RoofItem {
    base: [Vec2; 4],
    height: f32,
    top: Color,
    wall: Color,
    reach: f32,
}

/// Оборудование на плоской кровле дома. `lift` — сдвиг нарисованной кровли
/// над контуром: основания коробок приходят уже сдвинутыми, а попадание в
/// контур проверяется до сдвига, по настоящему пятну.
pub(super) fn flat_roof_items(building: &PolyArea, look: &RoofLook, lift: Vec2) -> Vec<RoofItem> {
    // на гараже нет ни машинного помещения, ни вентшахты — там нечего
    // вентилировать, и коробка на боксе сразу выдаёт генератор
    if matches!(look.kind, RoofKind::GarageRow | RoofKind::GarageBlock) {
        return Vec::new();
    }
    let axis = look.frame.axis;
    let perp = Vec2::new(-axis.y, axis.x);
    let Some(frame) = Frame::of(building, axis, perp) else {
        return Vec::new();
    };

    let mut rng = Lcg::new(look.frame.seed.to_bits() ^ 0x9E37_79B9);
    let mut items = Vec::new();
    let area = frame.footprint;
    let soft = matches!(
        look.kind,
        RoofKind::Bitumen | RoofKind::Gravel | RoofKind::Membrane
    );

    // машинное помещение — у мягкой кровли крупного дома
    if soft && area >= PENTHOUSE_MIN_AREA {
        let count = if area >= PENTHOUSE_SECOND_AREA { 2 } else { 1 };
        for _ in 0..count {
            place(
                &mut items,
                &frame,
                building,
                &mut rng,
                lift,
                PENTHOUSE_SIZE,
                PENTHOUSE_HEIGHT,
                PENTHOUSE_TOP,
                PENTHOUSE_WALL,
            );
        }
    }

    // ленты зенитных фонарей — большой корпус под профлистом
    if look.kind == RoofKind::Corrugated && area >= SKYLIGHT_MIN_AREA {
        let length = frame.length * SKYLIGHT_LENGTH_SHARE;
        let lanes = if frame.width >= SKYLIGHT_SECOND_WIDTH {
            vec![-0.22, 0.22]
        } else {
            vec![0.0]
        };
        for share in lanes {
            let center = frame.center() + perp * (share * frame.width);
            let size = Vec2::new(length, SKYLIGHT_WIDTH);
            if let Some(base) = fit(building, center, size, axis, perp, lift) {
                items.push(RoofItem {
                    base,
                    height: SKYLIGHT_HEIGHT,
                    top: SKYLIGHT_TOP,
                    wall: SKYLIGHT_WALL,
                    reach: shadow_reach(building, lift, &base),
                });
            }
        }
    }

    // блоки кондиционеров — торговля и казённые здания
    if matches!(
        building.building_use,
        BuildingUse::Commercial | BuildingUse::Public
    ) {
        let count = ((area / UNIT_AREA_PER) as usize).clamp(1, UNIT_MAX);
        for _ in 0..count {
            place(
                &mut items,
                &frame,
                building,
                &mut rng,
                lift,
                UNIT_SIZE,
                UNIT_HEIGHT,
                EQUIPMENT_TOP,
                EQUIPMENT_WALL,
            );
        }
    }

    // вентшахты — на любой плоской кровле
    let vents = ((area / VENT_AREA_PER) as usize).clamp(1, VENT_MAX);
    for _ in 0..vents {
        place(
            &mut items,
            &frame,
            building,
            &mut rng,
            lift,
            VENT_SIZE,
            VENT_HEIGHT,
            EQUIPMENT_TOP,
            EQUIPMENT_WALL,
        );
    }
    items
}

/// Труба на коньке: `ridge` — оба конца конька уже в нарисованных
/// координатах. `building` и `lift` — те же, по которым строилась крыша: по
/// ним трубе считается [`RoofItem::reach`], как и всякой коробке на плоской
/// кровле.
pub(super) fn ridge_chimney(
    building: &PolyArea,
    look: &RoofLook,
    ridge: (Vec2, Vec2),
    lift: Vec2,
) -> Option<RoofItem> {
    // печную трубу на гаражном ряду не ставят — там не топят
    if matches!(look.kind, RoofKind::GarageRow | RoofKind::GarageBlock) {
        return None;
    }
    let along = (ridge.1 - ridge.0).try_normalize()?;
    let length = (ridge.1 - ridge.0).length();
    if length < 2.0 * CHIMNEY_SIZE.x {
        return None;
    }
    let mut rng = Lcg::new(look.frame.seed.to_bits() ^ 0x85EB_CA6B);
    let at = ridge.0 + along * rng.range(0.25, 0.75) * length;
    let across = Vec2::new(-along.y, along.x);
    let base = rect(at, CHIMNEY_SIZE, along, across);
    Some(RoofItem {
        base,
        height: CHIMNEY_HEIGHT,
        top: CHIMNEY_TOP,
        wall: CHIMNEY_WALL,
        reach: shadow_reach(building, lift, &base),
    })
}

/// Коробки в меш: сначала непрозрачная тень каждой, потом сама коробка —
/// видимые стены и верх. В плоских режимах (`lean` не задан) остаётся
/// тень и верх, то есть коробка сверху.
pub(super) fn push_items(
    builder: &mut MeshBuilder,
    items: &[RoofItem],
    lean: Option<Lean>,
    roof: Srgba,
) {
    if items.is_empty() {
        return;
    }
    let shadow: LinearRgba = roof.mix(&Srgba::BLACK, CLUTTER_SHADOW_MIX).into();
    builder.set_roof(None);
    for item in items {
        // тень — свип основания по свету: два ребра силуэта плюс сдвинутое
        // основание. Ровно та же конструкция, что у теней самих домов, и по
        // той же причине: у выпуклого прямоугольника свип двух теневых рёбер
        // и есть недостающая часть объединения, а выпуклую оболочку строить
        // не приходится
        // тень обрезана краем кровли ([`RoofItem::reach`], посчитанным в
        // момент раскладки). Физически она бы через край перевалила — и на
        // снимке переваливает, — но рисуется она цветом этой крыши и
        // непрозрачной, в общем меше зданий: на дефолтных 59° метровая
        // вентшахта укладывается в отступ от края (`EDGE_MARGIN`), а на 15°
        // машинное помещение даёт одиннадцать метров и тёмная полоса уехала бы
        // с крыши на соседний дом и на землю. То есть портится это ровно на том
        // конце ползунка, ради которого высота солнца и стала ручкой
        let length = (item.height * shadow_length_scale()).min(item.reach);
        let offset = shadow_dir() * length;
        for (a, b) in silhouette_edges(&item.base, shadow_dir()) {
            builder.push_quad([a, b, b + offset, a + offset], shadow);
        }
        builder.push_quad(item.base.map(|point| point + offset), shadow);

        let lift = lean.map_or(Vec2::ZERO, |lean| lean.ridge(item.height));
        if let Some(lean) = lean {
            for (a, b) in silhouette_edges(&item.base, -lean.dir()) {
                let (bottom, top) = wall_colors(item.wall.to_srgba(), a, b, lean.dir());
                builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
            }
        }
        builder.push_quad(item.base.map(|point| point + lift), item.top.to_linear());
    }
}

/// Докуда тень коробки дотянется, не съехав с нарисованной кровли: ближайшее
/// пересечение луча тени с контуром здания — из каждого угла основания, по
/// внешнему кольцу и по каждому двору.
///
/// Меряется по **ненадвинутому** контуру, как и попадание коробки в него
/// (`fit`): сдвиг 2.5D переносит кровлю целиком. У скатной крыши контур чуть
/// уже нарисованного ската (настоящая крыша свисает), так что труба на коньке
/// обрезается с запасом в свою пользу — лучше, чем наоборот.
fn shadow_reach(building: &PolyArea, lift: Vec2, base: &[Vec2; 4]) -> f32 {
    let dir = shadow_dir();
    // лучи идут из всех четырёх углов base, сдвинутых на -lift — ребро можно
    // пропустить, только если оно позади каждого из этих истинных начал луча
    let min_origin = base
        .iter()
        .map(|c| (*c - lift).dot(dir))
        .fold(f32::MAX, f32::min);
    let mut reach = f32::MAX;
    for ring in std::iter::once(&building.outer).chain(building.holes.iter()) {
        for (index, &a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            // ребро целиком позади всех лучей — ни один их не догонит
            if a.dot(dir) < min_origin && b.dot(dir) < min_origin {
                continue;
            }
            for corner in base {
                if let Some(hit) = ray_hits_segment(*corner - lift, dir, a, b) {
                    reach = reach.min(hit);
                }
            }
        }
    }
    reach
}

/// Длина по лучу до пересечения с отрезком, если луч его пересекает.
fn ray_hits_segment(from: Vec2, dir: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let edge = b - a;
    let denom = dir.perp_dot(edge);
    if denom.abs() < 1e-6 {
        return None;
    }
    let to_a = a - from;
    let along_ray = to_a.perp_dot(edge) / denom;
    let along_edge = to_a.perp_dot(dir) / denom;
    (along_ray >= 0.0 && (0.0..=1.0).contains(&along_edge)).then_some(along_ray)
}

/// Рама дома: длинная ось, её длина и ширина, начало — угол описанного по
/// этой оси прямоугольника, уже с отступом от края.
struct Frame {
    origin: Vec2,
    axis: Vec2,
    perp: Vec2,
    length: f32,
    width: f32,
    footprint: f32,
}

impl Frame {
    /// Рама с отступом от края; `None` — после отступа не осталось места.
    fn of(building: &PolyArea, axis: Vec2, perp: Vec2) -> Option<Self> {
        let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
        let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
        for point in &building.outer {
            min_u = min_u.min(point.dot(axis));
            max_u = max_u.max(point.dot(axis));
            min_v = min_v.min(point.dot(perp));
            max_v = max_v.max(point.dot(perp));
        }
        let (length, width) = (max_u - min_u, max_v - min_v);
        let margin = EDGE_MARGIN.min(width.min(length) * EDGE_MARGIN_SHARE);
        let (length, width) = (length - 2.0 * margin, width - 2.0 * margin);
        if length <= 0.0 || width <= 0.0 {
            return None;
        }
        Some(Self {
            origin: axis * (min_u + margin) + perp * (min_v + margin),
            axis,
            perp,
            length,
            width,
            footprint: signed_ring_area(&building.outer).abs(),
        })
    }

    fn center(&self) -> Vec2 {
        self.origin + self.axis * (self.length / 2.0) + self.perp * (self.width / 2.0)
    }
}

/// Одна попытка поставить коробку: точка в раме по ГПСЧ, проверка на попадание
/// в контур. Промах просто теряется — дырявый ряд вентшахт на Г-образном доме
/// выглядит естественнее, чем шахта, висящая над двором.
#[allow(clippy::too_many_arguments)]
fn place(
    items: &mut Vec<RoofItem>,
    frame: &Frame,
    building: &PolyArea,
    rng: &mut Lcg,
    lift: Vec2,
    size: Vec2,
    height: f32,
    top: Color,
    wall: Color,
) {
    for _ in 0..PLACE_TRIES {
        let center = frame.origin
            + frame.axis * rng.range(0.0, frame.length)
            + frame.perp * rng.range(0.0, frame.width);
        if let Some(base) = fit(building, center, size, frame.axis, frame.perp, lift) {
            items.push(RoofItem {
                base,
                height,
                top,
                wall,
                reach: shadow_reach(building, lift, &base),
            });
            return;
        }
    }
}

/// Основание коробки, если все четыре угла легли внутрь контура. Проверка по
/// **ненадвинутому** пятну: сдвиг 2.5D переносит кровлю целиком, и то, что
/// внутри контура, останется внутри нарисованной крыши.
fn fit(
    building: &PolyArea,
    center: Vec2,
    size: Vec2,
    axis: Vec2,
    perp: Vec2,
    lift: Vec2,
) -> Option<[Vec2; 4]> {
    let base = rect(center, size, axis, perp);
    base.iter()
        .all(|corner| point_in_area(*corner, building))
        .then(|| base.map(|corner| corner + lift))
}

/// Прямоугольник `size` вокруг точки, повёрнутый в раму (CCW при
/// правой паре `axis`/`perp`).
fn rect(center: Vec2, size: Vec2, axis: Vec2, perp: Vec2) -> [Vec2; 4] {
    let half_axis = axis * (size.x / 2.0);
    let half_perp = perp * (size.y / 2.0);
    [
        center - half_axis - half_perp,
        center + half_axis - half_perp,
        center + half_axis + half_perp,
        center - half_axis + half_perp,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::AreaKind;
    use crate::settings::SUN_ELEVATION_MIN;

    fn building(outer: Vec<Vec2>, building_use: BuildingUse) -> PolyArea {
        PolyArea {
            outer,
            holes: Vec::new(),
            kind: AreaKind::Building,
            building_use,
            height: Some(15.0),
            entrances: Vec::new(),
        }
    }

    fn block(width: f32, length: f32) -> Vec<Vec2> {
        vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(length, 0.0),
            Vec2::new(length, width),
            Vec2::new(0.0, width),
        ]
    }

    /// Луч тени останавливается на краю кровли: азимут 270° кладёт тень строго
    /// на восток, и от коробки в `x = 4..6` до стены `x = 20` остаётся 14 м —
    /// по ближайшему из углов, а не по дальнему.
    #[test]
    fn the_shadow_reach_stops_at_the_roof_edge() {
        let _sun = crate::map::sun_at(270.0, 45.0);
        assert!(
            (shadow_dir() - Vec2::X).length() < 1e-5,
            "{:?}",
            shadow_dir()
        );
        let slab = building(block(20.0, 20.0), BuildingUse::Other);
        let base = rect(Vec2::new(5.0, 10.0), Vec2::splat(2.0), Vec2::X, Vec2::Y);
        let reach = shadow_reach(&slab, Vec2::ZERO, &base);
        assert!((reach - 14.0).abs() < 1e-3, "{reach}");
    }

    /// На низком солнце тень оборудования обязана остаться на крыше: рисуется
    /// она непрозрачной и цветом этой кровли, так что съехавшая полоса легла бы
    /// тёмной чертой на соседний дом и на землю.
    #[test]
    fn a_low_sun_keeps_the_equipment_shadow_on_the_roof() {
        let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let mut items = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert!(!items.is_empty());
        // и отдельно — машинное помещение у самого края с наветренной стороны:
        // разложенное оборудование до края может и не достать, а зажим нужен
        // именно ему
        let base = rect(Vec2::new(50.0, 8.0), PENTHOUSE_SIZE, Vec2::X, Vec2::Y);
        items.push(RoofItem {
            base,
            height: PENTHOUSE_HEIGHT,
            top: PENTHOUSE_TOP,
            wall: PENTHOUSE_WALL,
            reach: shadow_reach(&slab, Vec2::ZERO, &base),
        });
        let mut clamped = 0;
        for item in &items {
            let wanted = item.height * shadow_length_scale();
            // тот самый вылет, который увидит `push_items`, — из предмета, а
            // не пересчитанный тестом заново
            let reach = item.reach;
            if reach < wanted {
                clamped += 1;
            }
            let offset = shadow_dir() * wanted.min(reach);
            for corner in item.base {
                // зажатая тень кончается ровно на стене, и точка на самом
                // контуре в `point_in_area` уже наружу — отступаем на промилле
                let far = corner + offset * 0.999;
                assert!(point_in_area(far, &slab), "{far:?} is off the roof");
            }
        }
        // без зажима хоть одна тень с этой крыши уходит: машинное помещение в
        // три метра даёт на 15° одиннадцать метров при шестнадцатиметровой
        // ширине корпуса
        assert!(clamped > 0, "nothing was clamped, the case is not covered");
    }

    #[test]
    fn a_block_gets_equipment_and_a_shed_does_not() {
        let _sun = crate::map::default_sun();
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let items = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert!(!items.is_empty(), "a 960 m2 block carries equipment");

        // сарай 3 × 3: после отступа от края ставить некуда
        let shed = building(block(3.0, 3.0), BuildingUse::Other);
        let look = super::super::material::roof_look(&shed);
        assert!(flat_roof_items(&shed, &look, Vec2::ZERO).len() <= 1);
    }

    #[test]
    fn equipment_stays_inside_the_footprint() {
        let _sun = crate::map::default_sun();
        // Г-образный контур: рама прямоугольна, дом — нет
        let ell = building(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(60.0, 0.0),
                Vec2::new(60.0, 14.0),
                Vec2::new(20.0, 14.0),
                Vec2::new(20.0, 50.0),
                Vec2::new(0.0, 50.0),
            ],
            BuildingUse::Apartments,
        );
        let look = super::super::material::roof_look(&ell);
        for item in flat_roof_items(&ell, &look, Vec2::ZERO) {
            for corner in item.base {
                assert!(point_in_area(corner, &ell), "{corner:?} is off the roof");
            }
        }
    }

    #[test]
    fn the_placement_is_stable() {
        let _sun = crate::map::default_sun();
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let first = flat_roof_items(&slab, &look, Vec2::ZERO);
        let second = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.base, b.base);
        }
    }

    /// Тень трубы на коньке обязана остаться на крыше. Наклон конька сдвигает
    /// базовую точку вверх; передача сдвига в shadow_reach обязана быть полной
    /// (lift + ridge_offset).
    #[test]
    fn ridge_chimney_shadow_stays_on_roof_with_low_sun() {
        let _sun = crate::map::sun_at(300.0, SUN_ELEVATION_MIN);
        let house = building(block(8.0, 10.0), BuildingUse::Other);
        let look = super::super::material::roof_look(&house);
        // конёк двускатной крыши проходит по средней линии короткой стороны
        let ridge = (Vec2::new(4.0, 0.0), Vec2::new(4.0, 10.0));
        // ridge_offset на типовой крыше — несколько дециметров; тест ищет баг,
        // когда сдвиг конька не передан, поэтому lift=Vec2::ZERO даст краткие
        // тени. С правильным ridge_offset эта область покрывается.
        let lift = Vec2::new(0.5, 0.5);
        if let Some(item) = ridge_chimney(&house, &look, ridge, lift) {
            let offset = shadow_dir() * (item.height * shadow_length_scale()).min(item.reach);
            for corner in item.base {
                let far = corner + offset * 0.999;
                assert!(
                    point_in_area(far, &house),
                    "{far:?} is off the house boundary"
                );
            }
        }
    }
}
