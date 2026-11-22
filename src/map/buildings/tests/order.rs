//! Порядок painter-а: глубина по подъёму, порядок домов-соседей и порядок
//! стен дома со ступенчатым фасадом.

use super::super::fixtures::{building, detail, extruded_mesh, mesh_points, rect};
use super::super::layers::silhouette_edges;
use super::super::*;
use super::support::{leaning_pair, stepped_facade};
use crate::map::osm::{AreaKind, BuildingUse};

#[test]
fn the_painter_order_puts_the_far_side_first() {
    let _sun = crate::map::default_sun();
    // «дальше» — вдоль отклонения верха: верх дальнего дома уезжает на
    // ближний, и ближний обязан лечь поверх, то есть попасть в буфер позже
    let lean = Lean::of();
    let dir = lean.dir();
    let near = lean.depth(Vec2::ZERO);
    let far = lean.depth(dir * 900.0);
    assert!(far > near);
    // поперёк отклонения глубина не меняется: сортировать там нечего
    assert_eq!(lean.depth(dir.perp() * 700.0), near);
}

#[test]
fn the_order_puts_the_leaning_neighbour_over_the_wing_it_covers() {
    let _sun = crate::map::default_sun();
    let (low, tall) = leaning_pair();
    let lean = Lean::of();
    // случай ловится только тогда, когда прежний ключ на нём и ошибается:
    // центр пятна девятиэтажки дальше по подъёму, значит она писалась первой
    assert!(lean.depth(building_center(&tall)) > lean.depth(building_center(&low)));
    // а кроет она: её кровля поднята на 27 м и ложится на крыло соседа
    let pair = [low.clone(), tall.clone()];
    assert_eq!(
        order::draw_order(&pair, lean, &HouseFootprint::all(&pair)),
        vec![0, 1]
    );
    // порядок входа на это не влияет — решает геометрия, а не индекс
    let swapped = [tall, low];
    assert_eq!(
        order::draw_order(&swapped, lean, &HouseFootprint::all(&swapped)),
        vec![1, 0]
    );
}

#[test]
fn the_order_writes_every_building_once() {
    let _sun = crate::map::default_sun();
    let (low, tall) = leaning_pair();
    let mut list = vec![low, tall];
    // ряд сцепленных домов: пары спорят друг с другом, и порядок обязан
    // остаться полным даже там, где отношение зациклилось
    for step in 1..6 {
        let shift = Vec2::new(step as f32 * 9.0, step as f32 * 7.0);
        let mut next = building(
            rect(shift, shift + Vec2::new(24.0, 18.0)),
            Some(9.0 * step as f32),
            AreaKind::Building,
        );
        next.building_use = BuildingUse::Apartments;
        list.push(next);
    }
    let order = order::draw_order(&list, Lean::of(), &HouseFootprint::all(&list));
    let mut seen = order.clone();
    seen.sort_unstable();
    assert_eq!(seen, (0..list.len()).collect::<Vec<_>>());
}

#[test]
fn the_wall_order_puts_the_stepped_back_section_first() {
    let _sun = crate::map::default_sun();
    let stepped = stepped_facade();
    let lean = Lean::of();
    let walls = silhouette_edges(&stepped.outer, -lean.dir());
    // видимы три стены: два южных фасада секций и западный торец
    let near = walls
        .iter()
        .position(|(a, b)| a.y == 0.0 && b.y == 0.0)
        .expect("near section wall");
    let far = walls
        .iter()
        .position(|(a, b)| a.y == 3.0 && b.y == 3.0)
        .expect("far section wall");
    // обход контура кладёт ближнюю раньше дальней — это и есть та ошибка
    assert!(near < far, "ring order walks the near wall first");

    let lift = lift_of(&stepped);
    let order = order::wall_order(&walls, lean, lift);
    let at = |wall: usize| order.iter().position(|&index| index == wall).unwrap();
    assert!(
        at(far) < at(near),
        "the near wall must cover the far one: {order:?}"
    );
    let mut seen = order.clone();
    seen.sort_unstable();
    assert_eq!(seen, (0..walls.len()).collect::<Vec<_>>());
}

#[test]
fn the_stepped_facade_reaches_the_mesh_near_wall_last() {
    let _sun = crate::map::default_sun();
    let stepped = stepped_facade();
    let mesh = extruded_mesh(&[stepped], &[], detail(false)).build();
    let points = mesh_points(&mesh);
    // угол, который есть только у своей стены: (20, 0) — у ближней,
    // (40, 3) — у дальней
    let first = |corner: Vec2| {
        points
            .iter()
            .position(|p| p.distance(corner) < 0.01)
            .unwrap_or_else(|| panic!("no vertex at {corner:?}"))
    };
    assert!(
        first(Vec2::new(40.0, 3.0)) < first(Vec2::new(20.0, 0.0)),
        "the far section's wall goes into the buffer first"
    );
}
