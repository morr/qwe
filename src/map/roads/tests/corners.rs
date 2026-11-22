use super::super::fixtures::{a_tee, with_network};
use super::super::*;
use crate::map::osm::fixture;

/// Стороны наружного угла лежат на **торцевых кромках лент**: прямой торец
/// идёт поперёк исходного крайнего звена оси ([`meshing::butt_direction`]),
/// а направление луча смотрит на первую вершину ленты после слияния, — у
/// улицы, разрезанной в узле с лёгким изломом, те расходятся, и веер по
/// направлению луча оставлял между собой и торцами клиновидную щель поперёк
/// асфальта (Тула, Фрунзе у съезда, N1).
#[test]
fn an_outer_corner_sides_lie_on_the_ribbon_butt_edges() {
    use crate::map::meshing::butt_direction;
    // улица ЮЮВ—ССВ, разрезанная в узле; у северного way первая вершина
    // через 1.5 м — общая с коротким проездом (пин оси), лента же сливает
    // её (четверть ширины 2 м), и направление луча — уже на следующую
    // вершину; съезд примыкает с ССЗ, узел — перекрёсток
    let node = Vec2::new(300.0, 100.0);
    let near = node + Vec2::from_angle(72.6_f32.to_radians()) * 1.5;
    let far = near + Vec2::from_angle(75.0_f32.to_radians()) * 20.0;
    let across = near + Vec2::new(-6.0, 6.0);
    let map = with_network(vec![
        fixture::street(
            vec![node + Vec2::from_angle(251.8_f32.to_radians()) * 20.0, node],
            8.0,
        ),
        fixture::street(vec![node, near, far], 8.0),
        fixture::street(
            vec![node + Vec2::from_angle(160.0_f32.to_radians()) * 15.0, node],
            8.0,
        ),
        fixture::street(vec![across, near], 5.0),
    ]);
    let prepared = Drawn::new(&map, &RoadStyle::default(), &RoadShape::default());
    let returns = corners::kerb_returns(&prepared, 1.0, VergeZoomBucket::for_zoom(0.0));
    // торец северного way: кромка веера на нём — вторая сторона веера
    // (справа по ходу луча), узел, полуширина и нахлёст вдоль торцевого звена
    let axis = prepared.axis(1, Axis::Ribbon);
    let butt = butt_direction(axis, false).expect("у оси есть крайнее звено");
    let expected = node - butt.perp() * 4.0 + butt * corners::OUTER_OVERLAP;
    let streets: Vec<&[Vec2]> = returns.streets().outlines().collect();
    let found = streets
        .iter()
        .flat_map(|outline| outline.iter())
        .any(|point| point.distance(expected) < 0.02);
    assert!(
        found,
        "стороны веера — по торцевым кромкам: вершины {expected} нет ни в \
         одном {streets:?}"
    );
}

/// Мощение углов для покрытия карманов (`osm/parse/pockets.rs`): на
/// перекрёстке оно есть — скругления не выводятся из полос покрытия, и без
/// этого прохода щель карманов ложилась бы поверх нарисованного вентиля
/// (Свободы × Первомайская: карман засевался на асфальте горла).
#[test]
fn corner_paving_contours_carry_the_drawn_fans() {
    let contours = corner_paving_contours(&a_tee());
    assert!(
        !contours.is_empty(),
        "на перекрёстке нет контуров скруглений"
    );
    // узел тройника (350, 100): скругление ищется рядом с ним, а не где-то
    let near_node = contours.iter().any(|outline| {
        outline
            .iter()
            .any(|point| point.distance(Vec2::new(350.0, 100.0)) < 30.0)
    });
    assert!(near_node, "контуры скруглений не у узла перекрёстка");
}
