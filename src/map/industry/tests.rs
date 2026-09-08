use super::*;

fn structure(kind: StructureKind, radius: f32, height: f32) -> Structure {
    Structure {
        at: Vec2::ZERO,
        radius,
        height,
        kind,
    }
}

/// Тень цилиндра — оболочка круга и сдвинутого круга: она обязана накрывать
/// оба и не быть шире круга поперёк сдвига. Сдвинутый круг раньше рисовали бы
/// просто кругом, и тогда полоса между основанием и тенью оставалась бы
/// пустой — у шестидесятиметровой трубы это тридцать метров пропущенной тени.
#[test]
fn the_sweep_covers_both_ends_and_stays_a_stadium() {
    let (radius, offset) = (5.0, Vec2::new(30.0, 0.0));
    let ring = sweep(Vec2::ZERO, radius, offset);

    let far = ring.iter().map(|point| point.x).fold(f32::MIN, f32::max);
    let near = ring.iter().map(|point| point.x).fold(f32::MAX, f32::min);
    assert!((far - (offset.x + radius)).abs() < 0.3, "far end at {far}");
    assert!((near + radius).abs() < 0.3, "near end at {near}");

    // поперёк сдвига стадион ровно шириной круга
    let across = ring.iter().map(|point| point.y.abs()).fold(0.0, f32::max);
    assert!((across - radius).abs() < 0.3, "half width {across}");
}

/// Труба в три метра — это кружок, который на общем плане теряется; видно её
/// только по тени, и тень обязана быть много длиннее самого кружка.
#[test]
fn a_chimney_is_mostly_its_own_shadow() {
    let chimney = structure(StructureKind::Chimney, 2.5, 60.0);
    let mut builder = MeshBuilder::default();
    push_shadow(&mut builder, &chimney);

    let reach = builder
        .positions_for_test()
        .iter()
        .map(|position| Vec2::new(position[0], position[1]).length())
        .fold(0.0_f32, f32::max);
    assert!(
        reach > 8.0 * chimney.radius,
        "the shadow reaches only {reach} m"
    );
}

/// Стеной становится половина, обращённая **против** крена: с этой стороны
/// стоит камера, и виден ближний бок цилиндра. Она же одна закрывает весь
/// силуэт ниже верхнего круга — дна не видно никогда.
#[test]
fn the_near_half_becomes_the_wall_and_leaves_no_bottom() {
    let tank = structure(StructureKind::Tank, 8.0, 12.0);
    let lift = Vec2::new(0.0, 20.0);
    let mut leaning = MeshBuilder::default();
    push_wall(&mut leaning, &tank, lift);
    // квад на грань, четыре вершины на квад — на половину граней
    assert_eq!(leaning.vertex_count(), 4 * CIRCLE_SIDES / 2);

    let corners = leaning.positions_for_test();
    // вниз стенка доходит ровно до нижней точки круга основания — это и есть
    // ближний торец силуэта
    let lowest = corners
        .iter()
        .map(|position| position[1])
        .fold(f32::MAX, f32::min);
    assert!(
        (lowest - (tank.at.y - tank.radius)).abs() < 1e-3,
        "the near cap stops at {lowest}"
    );
    // а вверх — до поднятого «экватора» круга, то есть **выше** нижней точки
    // верхнего круга: стенка заходит под верх с запасом, и щели между ними
    // не остаётся ни при какой длине крена
    let highest = corners
        .iter()
        .map(|position| position[1])
        .fold(f32::MIN, f32::max);
    assert!(
        (highest - (tank.at.y + lift.y)).abs() < 1e-3,
        "the wall stops at {highest}"
    );
    assert!(
        highest > tank.at.y + lift.y - tank.radius,
        "a gap at the top"
    );

    // без крена цилиндр стоит отвесно, и стены не видно вовсе
    let mut upright = MeshBuilder::default();
    push_wall(&mut upright, &tank, Vec2::ZERO);
    assert!(upright.is_empty());
}
