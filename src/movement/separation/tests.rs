use super::pairs::side_of;
use super::*;

fn entity(index: u32) -> Entity {
    Entity::from_raw_u32(index).unwrap()
}

/// Стоящая пешка: без курса обход встречного не включается, и тесты
/// продольной коррекции меряют её одну.
fn pawn(index: u32, position: Vec2, radius: f32, mobility: f32) -> Pawn {
    Pawn {
        entity: entity(index),
        pawn_id: index,
        position,
        radius,
        mobility,
        heading: Vec2::ZERO,
        speed: 2.8,
        human: true,
        stuck: 0.0,
    }
}

fn walking(index: u32, position: Vec2, heading: Vec2) -> Pawn {
    Pawn {
        heading,
        ..pawn(index, position, 0.45, 1.0)
    }
}

/// Ручки по умолчанию, но с ОДНОЙ поправкой: доля левшей обнулена.
///
/// Сторона обхода в дефолте игры личная ([`SEPARATION_LEFT_SHARE`]), то есть
/// зависит от `PawnId` конкретной пешки. Тест, который проверяет «уходит
/// вправо», после этого проверял бы не механику, а то, каким вышел хэш
/// номера, — и ломался бы от переименования пешек в соседнем тесте. Долю
/// левшей меряют два теста, и оба ставят её явно.
fn tuning(fraction: f32) -> Tuning {
    Tuning {
        fraction,
        dt: fraction / SEPARATION_RATE,
        sidestep: SEPARATION_SIDESTEP,
        cell: SEPARATION_CELL,
        lab: SeparationLab {
            left_share: 0.0,
            ..SeparationLab::default()
        },
    }
}

fn tuning_with(fraction: f32, lab: SeparationLab) -> Tuning {
    Tuning {
        lab,
        ..tuning(fraction)
    }
}

/// Упреждение, настроенное так, чтобы пара за несколько метров друг от
/// друга уже попадала в горизонт.
fn anticipating() -> SeparationLab {
    SeparationLab {
        experiments: SeparationExperiments {
            horizon: 1.5,
            anticipation: 2.0,
            lane_bias: 0.5,
            ..Default::default()
        },
        // сторона обхода фиксируется правой по той же причине, что и в
        // [`tuning`]
        left_share: 0.0,
        ..Default::default()
    }
}

fn state_with(pawns: Vec<Pawn>) -> SeparationState {
    SeparationState {
        pawns,
        ..Default::default()
    }
}

/// Перекрытая пара расходится в противоположные стороны вдоль своей оси,
/// ровно на долю перекрытия.
#[test]
fn an_overlapping_pair_is_pushed_apart() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    // перекрытие 0.4 м делится пополам: каждому по 0.2 вдоль оси пары
    assert!((state.pushes[0] - Vec2::new(-0.2, 0.0)).length() < 1e-4);
    assert!((state.pushes[1] - Vec2::new(0.2, 0.0)).length() < 1e-4);
}

/// Пара на дистанции суммы радиусов — уже не перекрытие: толчков нет,
/// разведённая толпа не продолжает расползаться.
#[test]
fn a_settled_pair_is_left_alone() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.95, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert_eq!(state.pushes[0], Vec2::ZERO);
    assert_eq!(state.pushes[1], Vec2::ZERO);
}

/// Точно совпавшие позиции разводятся по детерминированной оси, а не
/// остаются на месте с нулевым направлением.
#[test]
fn coincident_pawns_get_a_deterministic_axis() {
    let position = Vec2::new(10.0, 10.0);
    let mut state = state_with(vec![
        pawn(1, position, 0.45, 1.0),
        pawn(2, position, 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.pushes[0].length() > 1e-4);
    assert!((state.pushes[0] + state.pushes[1]).length() < 1e-4);

    // тот же набор — та же ось: направление не дрожит от прогона к прогону
    let first = state.pushes[0];
    resolve_pushes(&mut state, tuning(1.0));
    assert!((state.pushes[0] - first).length() < 1e-6);
}

/// Неподвижный участник (пожирающий демон) толкает, но не двигается:
/// вся коррекция достаётся подвижному.
#[test]
fn an_immovable_pawn_pushes_without_moving() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.9, 0.0),
        pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert_eq!(state.pushes[0], Vec2::ZERO);
    let expected = 0.45 + 0.9 - 0.5;
    assert!((state.pushes[1] - Vec2::new(expected, 0.0)).length() < 1e-4);
}

/// Идущие ЛОБ В ЛОБ получают боковую составляющую — иначе их толкает
/// строго назад по собственному пути, и разойтись они не могут вообще.
///
/// Обходит РОВНО ОДИН (здесь — старший `PawnId`): вправо от себя у
/// встречных — противоположные стороны мира, и две одинаковые добавки дали
/// бы паре сил вместо обхода (см. [`yields`]).
#[test]
fn one_of_a_head_on_pair_steps_aside() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    // продольная часть прежняя: каждого назад по своему курсу
    assert!(state.pushes[0].x < 0.0);
    assert!(state.pushes[1].x > 0.0);
    // в сторону уходит только уступающий, и вправо ОТ СЕБЯ: идущий на −X — в +Y
    assert_eq!(state.pushes[0].y, 0.0, "{:?}", state.pushes[0]);
    assert!(state.pushes[1].y > 0.0, "{:?}", state.pushes[1]);
}

/// Обход — только паре, которая одна: у встречных в куче он выключен.
///
/// Одинаковый разворот вправо у всех перекрывшихся складывается в общее
/// вращение, и затор начинает крутиться вместо того, чтобы рассасываться.
#[test]
fn a_head_on_pair_with_a_third_pawn_nearby_does_not_step_aside() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
        // третий висит на встречном сзади — пара больше не одна
        walking(3, Vec2::new(11.2, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    // толкает по-прежнему всех, но строго вдоль оси: боковой добавки нет
    assert!(state.pushes.iter().all(|push| push.x != 0.0));
    assert!(state.pushes.iter().all(|push| push.y == 0.0));
}

/// Идущие одним курсом: всю коррекцию забирает ЗАДНИЙ, переднего не
/// трогает никто — его не за что толкать в спину.
#[test]
fn in_a_queue_only_the_pawn_behind_gives_way() {
    let mut state = state_with(vec![
        // догоняющий — первый: сосед у него спереди
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert_eq!(state.pushes[1], Vec2::ZERO, "переднего толкать нельзя");
    assert!(state.pushes[0].length() > 0.0);
    // и обход в сторону догоняющему тоже не полагается — он не встречный
    assert!(state.pushes[0].y.abs() < 1e-6, "{:?}", state.pushes[0]);
}

/// Сходящаяся пара — не очередь. Курсы расходятся всего на 80°, то есть
/// «примерно один курс» по порогу, но сосед впереди У ОБОИХ: никто никого не
/// догнал, и коррекция делится по подвижности, а не достаётся целиком тому,
/// кто в паре оказался первым.
#[test]
fn a_converging_pair_is_not_a_queue() {
    let heading_b = Vec2::from_angle(-80f32.to_radians());
    let offset = Vec2::from_angle(60f32.to_radians()) * 0.5;
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.0, 10.0) + offset, heading_b),
    ]);
    // обход выключен: тест меряет доли, а не боковую добавку
    let quiet = Tuning {
        sidestep: 0.0,
        ..tuning(1.0)
    };
    resolve_pushes(&mut state, quiet);

    let direction = offset.normalize();
    assert!(
        state.pushes[0].dot(direction) < 0.0,
        "{:?}",
        state.pushes[0]
    );
    assert!(
        state.pushes[1].dot(direction) > 0.0,
        "{:?}",
        state.pushes[1]
    );
    assert!(
        (state.pushes[0].length() - state.pushes[1].length()).abs() < 1e-6,
        "{:?} / {:?}",
        state.pushes[0],
        state.pushes[1]
    );
}

/// …и поэтому обход у неё не умножается на ноль. Уступающего выбирает
/// [`yields`] по `PawnId`; пока сходящаяся пара считалась очередью, доля
/// уступающего в половине пар была нулевой, и боковой добавки не возникало
/// вовсе — при том, что для сходящихся она единственный способ разойтись.
#[test]
fn a_converging_pair_still_steps_aside() {
    let heading_b = Vec2::from_angle(-80f32.to_radians());
    let offset = Vec2::from_angle(60f32.to_radians()) * 0.5;
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.0, 10.0) + offset, heading_b),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    // всё, что поперёк оси пары, — это обход: продольная коррекция и твёрдое
    // ядро идут строго вдоль `direction`
    let direction = offset.normalize();
    assert!(
        state.pushes[1].perp_dot(direction).abs() > 1e-4,
        "{:?}",
        state.pushes[1]
    );
}

/// Придержан упёршийся в СТОЯЩЕГО: давить в того, кто не сдвинется с
/// места сам, бесполезно. Стоящего не придерживают — у него нет курса.
#[test]
fn a_pawn_walking_into_a_standing_neighbour_is_held() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.held[0], "упёршийся в стоящего придержан");
    assert!(!state.held[1], "стоящий не придержан");
}

/// Попутчиков не придерживают: очередь, идущая в одну сторону, проходится
/// полным шагом — иначе поток целиком ползёт на доле скорости от любого
/// касания. Догоняющего осаживает не придержка, а доля коррекции
/// ([`shares`]: задний забирает её всю).
#[test]
fn a_queue_walking_the_same_way_is_not_held() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.held.iter().all(|held| !held));
}

/// Лоб в лоб упираются оба — придержаны оба: без этого чей-то шаг
/// продолжает гасить коррекцию пары, и равновесие лишь сдвигается.
#[test]
fn both_of_a_head_on_pair_are_held() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.held[0] && state.held[1]);
}

/// Стоящих не придерживают (курса нет), разошедшихся — тоже: придержка
/// живёт ровно столько же, сколько само перекрытие.
#[test]
fn standing_and_settled_pawns_are_not_held() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        walking(3, Vec2::new(20.0, 10.0), Vec2::X),
        walking(4, Vec2::new(20.95, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.held.iter().all(|held| !held));
}

/// Демона не придерживают никогда: погоня обязана смыкаться, а «толпа
/// обтекает демона» уже выражено подвижностью. Человек навстречу демону
/// придержан как обычно.
#[test]
fn a_demon_is_never_held() {
    let mut state = state_with(vec![
        Pawn {
            human: false,
            ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
        },
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(!state.held[0], "демон прёт сквозь толпу");
    assert!(state.held[1], "человек навстречу демону придержан");
}

/// …но упор ему засчитывают: придержка человеческая (`SeparationHolds`), а
/// залипание — общее, иначе запрет скольжения с демона не снимется никогда.
#[test]
fn a_demon_braces_even_though_it_is_never_held() {
    let mut state = state_with(vec![
        Pawn {
            human: false,
            ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
        },
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(!state.held[0], "демон не придержан");
    assert!(state.braced[0], "но в чужое тело он упёрся");
    assert!(
        state.braced[1] && state.held[1],
        "человек и упёрся, и придержан"
    );
}

/// По умолчанию упреждения нет: пара в шести метрах друг от друга — не
/// пара вообще, и лишней работы прогон не делает. Это тот самый гарант, что
/// стенд не меняет поведение игры, пока его ручки не тронуты.
#[test]
fn without_a_horizon_a_distant_pair_is_not_touched() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(16.0, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.pairs.is_empty());
    assert_eq!(state.pushes[0], Vec2::ZERO);
    assert_eq!(state.pushes[1], Vec2::ZERO);
}

/// С горизонтом та же пара расходится ЗАРАНЕЕ — каждый вправо от себя, то
/// есть в мировых координатах в разные стороны, и к моменту встречи зазор
/// уже набран. Толчок строго поперечный: тормозить сдвигом позиции нельзя.
#[test]
fn an_approaching_head_on_pair_steps_aside_before_touching() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(16.0, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning_with(1.0, anticipating()));

    assert_eq!(state.pairs.len(), 1, "пара попала в горизонт");
    assert!(!state.pairs[0].2, "но перекрытием не считается");
    // идущий на +X уходит вправо от себя, то есть в −Y; встречный — в +Y
    assert!(state.pushes[0].y < 0.0, "{:?}", state.pushes[0]);
    assert!(state.pushes[1].y > 0.0, "{:?}", state.pushes[1]);
    assert!(
        state.pushes[0].x.abs() < 1e-6,
        "толчок только поперёк курса"
    );
    assert!(
        state.pushes[1].x.abs() < 1e-6,
        "толчок только поперёк курса"
    );
    // и никаких придержек: тела ещё не соприкоснулись
    assert!(state.held.iter().all(|held| !held));
}

/// Расходящихся упреждение не трогает: сосед впереди, но уходит быстрее —
/// уклоняться не от чего, и самый частый случай не стоит ничего.
#[test]
fn a_pair_walking_away_is_not_anticipated() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::NEG_X),
        walking(2, Vec2::new(16.0, 10.0), Vec2::X),
    ]);
    resolve_pushes(&mut state, tuning_with(1.0, anticipating()));

    assert_eq!(state.pushes[0], Vec2::ZERO);
    assert_eq!(state.pushes[1], Vec2::ZERO);
}

/// Сжатие радиуса в давке: та же пара при том же расстоянии перестаёт
/// считаться требующей коррекции, потому что в толчее дистанция покоя
/// падает. Пары как таковой это не отменяет — считаются они по полному
/// радиусу, иначе загрузка зависела бы сама от себя.
#[test]
fn a_squeezed_pair_stops_pushing() {
    let layout = || {
        state_with(vec![
            pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
            pawn(2, Vec2::new(10.6, 10.0), 0.45, 1.0),
        ])
    };

    let mut loose = layout();
    resolve_pushes(&mut loose, tuning(1.0));
    assert!(loose.pushes[0].length() > 0.0, "без сжатия пара расходится");

    let squeezed = SeparationLab {
        experiments: SeparationExperiments {
            compress: 0.5,
            compress_at: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut tight = layout();
    resolve_pushes(&mut tight, tuning_with(1.0, squeezed));
    assert_eq!(tight.pairs.len(), 1, "пара всё ещё пара");
    assert_eq!(tight.pushes[0], Vec2::ZERO);
    assert_eq!(tight.pushes[1], Vec2::ZERO);
}

/// Протискивание мимо стоящего ([`SeparationLab::pass_squeeze`]): пара
/// «идущая + стоящая» расходится на ужатую дистанцию, а пара стоящих рядом
/// — на полную. Ровно в этом смысл ручки: ужимается проход, а не толпа.
#[test]
fn only_a_walker_squeezes_past_a_standing_pawn() {
    let squeeze = SeparationLab {
        pass_squeeze: 0.5,
        ..Default::default()
    };
    // 0.9 м между центрами при радиусе 0.45: полная дистанция покоя, то
    // есть ужатая (0.45 м) уже выдержана
    let mut passing = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        pawn(2, Vec2::new(10.6, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut passing, tuning_with(1.0, squeeze));
    assert_eq!(passing.pushes[0], Vec2::ZERO, "идущая протискивается");
    assert_eq!(passing.pushes[1], Vec2::ZERO, "и стоящую не двигает");

    let mut standing = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.6, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut standing, tuning_with(1.0, squeeze));
    assert!(
        standing.pushes[0].length() > 0.0,
        "двое стоящих держат полную дистанцию"
    );

    let mut walkers = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.6, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut walkers, tuning_with(1.0, squeeze));
    assert!(
        walkers.pushes[0].length() > 0.0,
        "двое идущих — тоже полную: поток не обязан слипаться"
    );
}

/// Кратковременное сжатие ([`SeparationExperiments::stuck_compress`]) достаётся
/// только тому, кто УЖЕ залип: та же пара при нулевом стаже упора
/// расталкивается как обычно.
#[test]
fn only_a_stuck_pawn_squeezes() {
    let lab = SeparationLab {
        experiments: SeparationExperiments {
            stuck_compress: 0.5,
            stuck_after: 0.0,
            stuck_ramp: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let layout = |stuck: f32| {
        state_with(vec![
            Pawn {
                stuck,
                ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
            },
            Pawn {
                stuck,
                ..walking(2, Vec2::new(10.6, 10.0), Vec2::NEG_X)
            },
        ])
    };

    let mut fresh = layout(0.0);
    resolve_pushes(&mut fresh, tuning_with(1.0, lab));
    assert!(fresh.pushes[0].length() > 0.0, "не залипшие расходятся");

    let mut jammed = layout(1.0);
    resolve_pushes(&mut jammed, tuning_with(1.0, lab));
    assert_eq!(jammed.pushes[0], Vec2::ZERO, "залипшие протискиваются");
}

/// Твёрдое ядро ([`SeparationExperiments::hard_core`]) снимает наложение тел
/// ЦЕЛИКОМ, даже когда мягкая часть отдана долей: то, чем тела уже
/// пересеклись, не торгуется.
#[test]
fn the_hard_core_is_resolved_in_full() {
    let lab = SeparationLab {
        experiments: SeparationExperiments {
            hard_core: 0.5,
            ..Default::default()
        },
        ..Default::default()
    };
    // радиусы 0.45 + 0.45: покой 0.9, ядро 0.45, а стоят пешки в 0.3
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.3, 10.0), 0.45, 1.0),
    ]);
    // мягкой части достаётся сотая доля перекрытия — ядру это не помеха
    resolve_pushes(&mut state, tuning_with(0.01, lab));

    // ядро 0.45 против расстояния 0.3: по 0.075 на каждого
    assert!((state.core_pushes[0] - Vec2::new(-0.075, 0.0)).length() < 1e-4);
    assert!((state.core_pushes[1] - Vec2::new(0.075, 0.0)).length() < 1e-4);
    // и мягкая часть при этом осталась мягкой
    assert!(state.pushes[0].length() < 0.01);
}

/// Без ручки ядра нет: буфер пуст, и толчок ровно тот же, что был.
#[test]
fn without_the_knob_there_is_no_core_push() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(10.0, 10.0), 0.45, 1.0),
        pawn(2, Vec2::new(10.3, 10.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.core_pushes.iter().all(|push| *push == Vec2::ZERO));
}

/// Скольжение отпускает залипшего ([`SeparationExperiments::slide_release`]):
/// свободной пешке запрет «не лезь в тело» выдаётся, простоявшей в упоре —
/// уже нет, иначе сходящаяся толпа встаёт колом.
#[test]
fn sliding_lets_go_of_a_pawn_that_has_been_stuck() {
    let lab = SeparationLab {
        experiments: SeparationExperiments {
            slide: 1.0,
            slide_release: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let layout = |stuck: f32| {
        state_with(vec![
            Pawn {
                stuck,
                ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
            },
            pawn(2, Vec2::new(10.5, 10.0), 0.45, 1.0),
        ])
    };

    let mut fresh = layout(0.0);
    resolve_pushes(&mut fresh, tuning_with(1.0, lab));
    assert!(fresh.blocks[0] != Vec2::ZERO, "свободной запрет выдан");

    let mut jammed = layout(1.5);
    resolve_pushes(&mut jammed, tuning_with(1.0, lab));
    assert_eq!(jammed.blocks[0], Vec2::ZERO, "залипшую отпустили");
}

/// Счётчик залипания копится и у демона: [`advance_stuck`] считает по упору
/// (`braced`), а не по придержке, — иначе клапан [`SeparationExperiments::slide_release`]
/// у демона не срабатывает никогда.
#[test]
fn the_stuck_clock_counts_a_demons_bracing() {
    let lab = SeparationLab {
        experiments: SeparationExperiments {
            slide: 1.0,
            slide_release: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut state = state_with(vec![
        Pawn {
            human: false,
            ..walking(1, Vec2::new(10.0, 10.0), Vec2::X)
        },
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning_with(1.0, lab));
    advance_stuck(&mut state, 0.25, &lab);

    assert_eq!(state.stuck.get(&entity(1)).copied(), Some(0.25), "демон");
    assert_eq!(state.stuck.get(&entity(2)).copied(), Some(0.25), "человек");
}

/// Пока обе ручки залипания на нуле, карта не собирается вовсе: это тот же
/// гарант «стенд не меняет игру», что и у остальных ручек.
#[test]
fn the_stuck_clock_stays_empty_while_its_knobs_are_untouched() {
    let mut state = state_with(vec![
        walking(1, Vec2::new(10.0, 10.0), Vec2::X),
        walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
    ]);
    resolve_pushes(&mut state, tuning(1.0));
    advance_stuck(&mut state, 0.25, &SeparationLab::default());

    assert!(state.stuck.is_empty());
}

/// Доля левшей ([`SeparationLab::left_share`]): при 1.0 обход зеркалится
/// целиком, при 0 остаётся правым — то есть сторона действительно личная и
/// берётся из `PawnId`, а не жёстко зашита.
#[test]
fn a_share_of_pawns_steps_aside_to_the_left() {
    let layout = || {
        state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
        ])
    };

    let mut righties = layout();
    resolve_pushes(&mut righties, tuning(1.0));
    assert!(righties.pushes[1].y > 0.0, "идущий на −X уходит в +Y");

    let lefties = SeparationLab {
        left_share: 1.0,
        ..Default::default()
    };
    let mut mirrored = layout();
    resolve_pushes(&mut mirrored, tuning_with(1.0, lefties));
    assert!(mirrored.pushes[1].y < 0.0, "у левши — ровно наоборот");
}

/// Стороны обхода не одинаковы у всех: при доле 0.3 в сотне пешек есть и
/// левши, и правши. Это то самое «немного разброса», ради которого ручка и
/// заведена, — и оно обязано быть УСТОЙЧИВЫМ (сторона зависит от `PawnId`,
/// а не от прогона).
#[test]
fn the_side_of_a_pawn_is_personal_and_stable() {
    let side = |pawn_id: u32| {
        let pawn = walking(pawn_id, Vec2::ZERO, Vec2::X);
        side_of(&pawn, 0.3)
    };
    let lefties = (0..100).filter(|id| side(*id).y > 0.0).count();
    assert!(
        (15..45).contains(&lefties),
        "левшей должно быть около трети, а не {lefties}"
    );
    assert_eq!(side(7), side(7), "сторона одной и той же пешки постоянна");
}

/// Обход в куче — под ручкой: по умолчанию его нет (гейт `alone`), с
/// ручкой встречные получают боковую добавку и в толпе тоже.
#[test]
fn a_crowd_sidesteps_only_when_the_knob_is_on() {
    let layout = || {
        state_with(vec![
            walking(1, Vec2::new(10.0, 10.0), Vec2::X),
            walking(2, Vec2::new(10.5, 10.0), Vec2::NEG_X),
            walking(3, Vec2::new(11.2, 10.0), Vec2::NEG_X),
        ])
    };

    let mut gated = layout();
    resolve_pushes(&mut gated, tuning(1.0));
    assert!(gated.pushes.iter().all(|push| push.y == 0.0));

    let crowded = SeparationLab {
        experiments: SeparationExperiments {
            crowd_sidestep: 0.5,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut loose = layout();
    resolve_pushes(&mut loose, tuning_with(1.0, crowded));
    assert!(loose.pushes.iter().any(|push| push.y != 0.0));
}

/// Соседи через границу ячейки мелкой сетки видят друг друга: пара на
/// стыке двух ячеек — всё ещё пара.
#[test]
fn a_pair_across_a_fine_cell_boundary_is_still_resolved() {
    let mut state = state_with(vec![
        pawn(1, Vec2::new(SEPARATION_CELL - 0.1, 1.0), 0.45, 1.0),
        pawn(2, Vec2::new(SEPARATION_CELL + 0.1, 1.0), 0.45, 1.0),
    ]);
    resolve_pushes(&mut state, tuning(1.0));

    assert!(state.pushes[0].x < 0.0);
    assert!(state.pushes[1].x > 0.0);
}

/// Один прогон над стоящей парой: толчки → кламп → применение. Чистый
/// аналог цикла применения из `separate_pawns` (без `damp_along_heading` —
/// у стоящих пешек курса нет, и гашение вдоль курса не участвует).
fn run_pair(positions: [Vec2; 2], radius: f32, dt: f32) -> [Vec2; 2] {
    let lab = SeparationLab {
        left_share: 0.0,
        ..SeparationLab::default()
    };
    let mut state = state_with(vec![
        pawn(1, positions[0], radius, 1.0),
        pawn(2, positions[1], radius, 1.0),
    ]);
    resolve_pushes(
        &mut state,
        Tuning {
            fraction: relaxation_fraction(lab.rate, dt),
            dt,
            sidestep: SEPARATION_SIDESTEP,
            cell: SEPARATION_CELL,
            lab,
        },
    );
    [
        positions[0] + clamped_step(state.pushes[0], state.core_pushes[0], &lab, dt),
        positions[1] + clamped_step(state.pushes[1], state.core_pushes[1], &lab, dt),
    ]
}

/// Главный инвариант: один прогон с большим dt и N прогонов с dt/N дают одну
/// и ту же итоговую дистанцию — расталкивание одинаково на всех скоростях
/// симуляции. T = 0.5 с — кадр на 30× при 60 fps; 32 прогона по T/32 — те же
/// полсекунды кадрами 1×. Допуск 0.01 м покрывает единственный прогон, в
/// котором потолок скорости перестаёт быть связанным (ошибка перехода фаз).
#[test]
fn one_long_run_matches_many_short_ones() {
    // дистанция покоя 0.9 м, стартовый зазор 0.5 м → перекрытие 0.4 м
    let start = [Vec2::new(0.0, 1.0), Vec2::new(0.5, 1.0)];
    let total = 0.5;
    let big = run_pair(start, 0.45, total);
    let mut small = start;
    for _ in 0..32 {
        small = run_pair(small, 0.45, total / 32.0);
    }
    let big_gap = (big[1] - big[0]).length();
    let small_gap = (small[1] - small[0]).length();
    assert!(
        (big_gap - small_gap).abs() < 0.01,
        "one run: {big_gap}, 32 runs: {small_gap}"
    );
}

/// Экспонента не насыщается на игровых dt: на кадре 30× (0.5 с) доля всё ещё
/// меньше единицы — «мягкость» не исчезает (линейная `rate · dt` насыщалась
/// уже при dt > 1/8 с, т.е. с ~7.5×). На малых dt совпадает с линейной.
/// Ассерт нарочно на dt кадра 30×, а не на «любом dt»: при dt >> 1/rate
/// f32-доля легитимно округляется ровно до 1.0 (снять больше перекрытия, чем
/// есть, она всё равно не может).
#[test]
fn the_relaxation_fraction_never_saturates() {
    assert!(relaxation_fraction(SEPARATION_RATE, 0.5) < 1.0);
    let dt = 1.0 / 240.0;
    let linear = SEPARATION_RATE * dt;
    assert!((relaxation_fraction(SEPARATION_RATE, dt) - linear).abs() / linear < 0.02);
}

/// На большом dt потолок — `max_speed · dt`, а не `max_step`: эффективная
/// скорость расталкивания не падает с ростом скорости симуляции (раньше выше
/// ~13× связывался `max_step`, и скорость падала до `max_step / dt`).
#[test]
fn the_ceiling_scales_with_dt_instead_of_max_step() {
    let lab = SeparationLab::default();
    let dt = 0.5; // кадр на 30×
    let step = clamped_step(Vec2::new(5.0, 0.0), Vec2::ZERO, &lab, dt);
    assert!((step.length() - lab.max_speed * dt).abs() < 1e-4);
    assert!(step.length() > lab.max_step);
}

/// На 1× страховка от телепорта работает как раньше: толчок твёрдого ядра, у
/// которого потолка скорости нет, всё равно не длиннее `max_step`.
#[test]
fn max_step_still_guards_the_core_at_normal_speed() {
    let lab = SeparationLab::default();
    let step = clamped_step(Vec2::ZERO, Vec2::new(5.0, 0.0), &lab, 1.0 / 60.0);
    assert!((step.length() - lab.max_step).abs() < 1e-4);
}

/// Что в игре ДЕЙСТВУЕТ, а что лежит выключенным ради стенда, — пин, а не
/// обещание в доке: оба типа разбираются ЦЕЛИКОМ, поэтому новое поле ломает
/// компиляцию теста и заставляет решить, в какую группу оно идёт. Ровно на
/// этом месте разъехались док типа и `references/crowd.md`.
#[test]
fn game_defaults_keep_every_experiment_off() {
    let SeparationLab {
        rate,
        max_step,
        max_speed,
        steer,
        steer_release,
        pass_squeeze,
        left_share,
        experiments,
    } = SeparationLab::default();
    assert_eq!(rate, SEPARATION_RATE);
    assert_eq!(max_step, SEPARATION_MAX_STEP);
    assert_eq!(max_speed, SEPARATION_MAX_SPEED);
    assert_eq!(steer, SEPARATION_STEER);
    assert_eq!(steer_release, 2.0);
    assert_eq!(pass_squeeze, SEPARATION_PASS_SQUEEZE);
    assert_eq!(left_share, SEPARATION_LEFT_SHARE);

    let SeparationExperiments {
        horizon,
        anticipation,
        anticipate_margin,
        lane_bias,
        compress,
        compress_at,
        crowd_sidestep,
        idle_mobility,
        arrive_slack,
        slide,
        stuck_compress,
        stuck_after,
        stuck_ramp,
        hard_core,
        slide_release,
    } = experiments;
    // гейты: ноль — это «ветка решателя не исполняется»
    for (name, gate) in [
        ("horizon", horizon),
        ("anticipation", anticipation),
        ("lane_bias", lane_bias),
        ("compress", compress),
        ("crowd_sidestep", crowd_sidestep),
        ("slide", slide),
        ("stuck_compress", stuck_compress),
        ("hard_core", hard_core),
        ("slide_release", slide_release),
    ] {
        assert_eq!(gate, 0.0, "{name} в игре обязан быть выключен");
    }
    // нейтральные множители: ветка исполняется, но ничего не меняет
    assert_eq!(idle_mobility, 1.0);
    assert_eq!(arrive_slack, 1.0);
    assert_eq!(anticipate_margin, 1.0);
    // вторичные параметры выключенных механизмов — инертны, пока гейт ноль
    assert_eq!(compress_at, 4.0);
    assert_eq!(stuck_after, 0.5);
    assert_eq!(stuck_ramp, 0.5);
}

/// Продольная ручка: 1 (дефолт игры, [`SEPARATION_BACKSTEP`]) оставляет толчок
/// целым, 0 — строго поперечным, промежуточное значение делит продольную часть.
/// У стоящей пешки курса нет, и её толчок не трогают ни при каком `keep`.
#[test]
fn the_backstep_knob_scales_only_the_part_along_the_heading() {
    let push = Vec2::new(1.0, 1.0);

    assert_eq!(damp_along_heading(push, Vec2::X, 1.0), push);
    assert_eq!(damp_along_heading(push, Vec2::X, 0.0), Vec2::new(0.0, 1.0));
    assert_eq!(damp_along_heading(push, Vec2::X, 0.5), Vec2::new(0.5, 1.0));
    assert_eq!(damp_along_heading(push, Vec2::ZERO, 0.0), push);
}
