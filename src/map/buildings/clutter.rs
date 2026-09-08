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

use super::layers::{SHADOW_LENGTH_SCALE, silhouette_edges, wall_colors};
use super::material::{RoofKind, RoofLook};
use super::ridge_lift;
use crate::map::SHADOW_DIR;
use crate::map::meshing::MeshBuilder;
use crate::map::osm::model::{point_in_area, signed_ring_area};
use crate::map::osm::{BuildingUse, PolyArea};

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
/// настоящая высота над ней и чем красить.
pub(super) struct RoofItem {
    base: [Vec2; 4],
    height: f32,
    top: Color,
    wall: Color,
}

/// ГПСЧ Лемера (Park–Miller) — тот же, что раскладывает кроны деревьев.
struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self((seed % 0x7FFF_FFFF).max(1))
    }

    fn next_f32(&mut self) -> f32 {
        self.0 = ((u64::from(self.0) * 48271) % 0x7FFF_FFFF) as u32;
        self.0 as f32 / 2_147_483_647.0
    }

    /// Число в `[from, to)`.
    fn range(&mut self, from: f32, to: f32) -> f32 {
        from + self.next_f32() * (to - from)
    }
}

/// Оборудование на плоской кровле дома. `lift` — сдвиг нарисованной кровли
/// над контуром: основания коробок приходят уже сдвинутыми, а попадание в
/// контур проверяется до сдвига, по настоящему пятну.
pub(super) fn flat_roof_items(building: &PolyArea, look: &RoofLook, lift: Vec2) -> Vec<RoofItem> {
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
/// координатах.
pub(super) fn ridge_chimney(look: &RoofLook, ridge: (Vec2, Vec2)) -> Option<RoofItem> {
    let along = (ridge.1 - ridge.0).try_normalize()?;
    let length = (ridge.1 - ridge.0).length();
    if length < 2.0 * CHIMNEY_SIZE.x {
        return None;
    }
    let mut rng = Lcg::new(look.frame.seed.to_bits() ^ 0x85EB_CA6B);
    let at = ridge.0 + along * rng.range(0.25, 0.75) * length;
    let across = Vec2::new(-along.y, along.x);
    Some(RoofItem {
        base: rect(at, CHIMNEY_SIZE, along, across),
        height: CHIMNEY_HEIGHT,
        top: CHIMNEY_TOP,
        wall: CHIMNEY_WALL,
    })
}

/// Коробки в меш: сначала непрозрачная тень каждой, потом сама коробка —
/// видимые стены и верх. В плоских режимах (`lift_dir` не задан) остаётся
/// тень и верх, то есть коробка сверху.
pub(super) fn push_items(
    builder: &mut MeshBuilder,
    items: &[RoofItem],
    lift_dir: Option<Vec2>,
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
        let offset = SHADOW_DIR * item.height * SHADOW_LENGTH_SCALE;
        for (a, b) in silhouette_edges(&item.base, SHADOW_DIR) {
            builder.push_quad([a, b, b + offset, a + offset], shadow);
        }
        builder.push_quad(item.base.map(|point| point + offset), shadow);

        let lift = lift_dir.map_or(Vec2::ZERO, |_| ridge_lift(item.height));
        if let Some(lift_dir) = lift_dir {
            for (a, b) in silhouette_edges(&item.base, -lift_dir) {
                let (bottom, top) = wall_colors(item.wall, a, b, lift_dir);
                builder.push_quad_gradient([a, b, b + lift, a + lift], [bottom, bottom, top, top]);
            }
        }
        builder.push_quad(item.base.map(|point| point + lift), item.top.to_linear());
    }
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

    #[test]
    fn a_block_gets_equipment_and_a_shed_does_not() {
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
        let slab = building(block(16.0, 60.0), BuildingUse::Apartments);
        let look = super::super::material::roof_look(&slab);
        let first = flat_roof_items(&slab, &look, Vec2::ZERO);
        let second = flat_roof_items(&slab, &look, Vec2::ZERO);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.base, b.base);
        }
    }
}
