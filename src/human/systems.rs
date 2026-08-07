use bevy::prelude::*;
use rand::Rng;

use crate::grid::{tile_center, world_to_tile};
use crate::human::components::{
    Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, PanicRecoil,
    WanderHeading, WanderPause,
};
use crate::loading::AppState;
use crate::map::osm::{MapData, PolyArea};
use crate::movement::{Movable, MovableState, NeedsWanderTarget, SimPosition};
use crate::navigation::{ArcNavmesh, Pathfinder, find_passable_tile_near};
use crate::rng::{PawnId, RngDomain, WanderIndex, WorldSeed, decision_stream, stream};
use crate::settings::{
    HUMAN_COUNT, HUMAN_FLEE_SPEED, HUMAN_PANIC_RADIUS, HUMAN_SIZE, HUMAN_WALK_SPEED,
    HUMAN_WANDER_PAUSE, HUMAN_WANDER_PAUSE_SHARE, HUMAN_WANDER_RANGE, MAP_SIZE, RADIUS_HYSTERESIS,
    unit_z,
};

/// Отступ целей блуждания от края карты, м.
const MAP_MARGIN: f32 = 4.0;
/// Доля пеших целей «к случайному зданию» (длинные маршруты через город);
/// остальные гуляют поблизости.
const WANDER_TO_BUILDING_SHARE: f32 = 0.8;
/// Полураствор конуса вокруг текущего курса, в котором выбирается следующая
/// цель прогулки, рад (60°). Без него пешка на каждом шаге разворачивалась в
/// случайную сторону и топталась на месте.
const WANDER_CONE: f32 = std::f32::consts::FRAC_PI_3;
/// Сколько зданий перебирается в поисках цели «по делам» в конусе курса;
/// если ни одно не попало — берётся ближайшее по направлению из выборки.
const WANDER_BUILDING_TRIES: usize = 8;
/// Полураствор запретного конуса «назад к демону» после паники, рад
/// (45°, полный раствор — 90°).
const RECOIL_CONE: f32 = std::f32::consts::FRAC_PI_4;
/// Минимальная дальность первой цели после паники, м: человек должен хотя бы
/// выйти из зоны гистерезиса паники. Без порога здание за пределами конуса, но
/// в пятнадцати метрах, даёт ровно ту короткую прогулку, ради ухода от которой
/// цель и форсируется дальней.
const RECOIL_MIN_ERRAND: f32 = HUMAN_PANIC_RADIUS * RADIUS_HYSTERESIS;

/// Лежит ли направление в запретном конусе вокруг `ban`. Косинус растёт с
/// уменьшением угла, поэтому «внутри» — это строгое `>`: точно на границе
/// конуса цель разрешена.
fn in_recoil_cone(direction: Vec2, ban: Vec2) -> bool {
    direction.dot(ban) > RECOIL_CONE.cos()
}

pub fn spawn_humans(
    mut commands: Commands,
    arc_navmesh: Res<ArcNavmesh>,
    style: Res<HumanStyle>,
    seed: Res<WorldSeed>,
) {
    spawn_population(&mut commands, &arc_navmesh.read(), style.spread, seed.0);
}

/// Спавн населения; вызывается на старте и при рестарте сцены.
///
/// Два потока жребия, а не один: размещение идёт общим потоком `Population`
/// (цикл отбора тянет переменное число выборок на человека, и это нормально
/// внутри одного последовательного обхода), а всё личное — цвет, темп, курс —
/// уже из потока самой пешки по её [`PawnId`]. Так внешность и повадки
/// человека номер N не зависят от того, сколько раз отбор промахнулся по
/// непроходимым тайлам у его соседей.
pub fn spawn_population(
    commands: &mut Commands,
    navmesh: &crate::navigation::Navmesh,
    spread: f32,
    world_seed: u64,
) {
    let mut placement = stream(world_seed, RngDomain::Population, 0);

    for index in 0..HUMAN_COUNT {
        let pawn_id = index as u32;
        let mut rng = decision_stream(world_seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);

        let tile = loop {
            let candidate = IVec2::new(
                placement.random_range(0..navmesh.grid_size.x),
                placement.random_range(0..navmesh.grid_size.y),
            );
            if navmesh.is_passable(candidate.x, candidate.y) {
                break candidate;
            }
        };
        let position = tile_center(tile);

        // пастельная «одежда» со случайным тоном
        let color = Color::hsl(
            rng.random_range(0.0..360.0),
            rng.random_range(0.35..0.75),
            rng.random_range(0.35..0.65),
        );
        // без стартовой паузы: все идут с первого кадра. Залп из 20 000 целей
        // разруливают гейт видимости диспетчера (мирные вне экрана путь не
        // получают) и дешёвый HPA* — рассинхронизация тут только заставляла
        // пешек в кадре стоять первые секунды
        let pause = Timer::from_seconds(0.0, TimerMode::Once);
        // жребий двусторонний: минус — человек медленнее базы, плюс — быстрее
        let pace = Pace(rng.random_range(-1.0..=1.0));
        let heading = WanderHeading(Vec2::from_angle(
            rng.random_range(0.0..std::f32::consts::TAU),
        ));

        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(HUMAN_SIZE)),
                ..default()
            },
            Transform::from_translation(position.extend(unit_z(position.y))),
            Human,
            HumanWanderTag,
            HumanFirstWanderTag,
            Movable::new(pace.speed(HUMAN_WALK_SPEED, spread)),
            pace,
            WanderPause(pause),
            heading,
            PawnId(pawn_id),
            WanderIndex::ready(),
            DespawnOnExit(AppState::Playing),
            Name::new("human"),
        ));
    }
}

/// Ползунок разброса — людям, уже гуляющим по городу; аналог
/// `sync_demon_speed`, и так же по `resource_changed`, а не каждый кадр.
///
/// База берётся по тегу состояния: пересчитать бегущего от `HUMAN_WALK_SPEED`
/// значило бы посадить его на шаг до самого конца паники — `flee` вернёт
/// беговую скорость только на выходе из состояния, а не на входе.
pub fn sync_human_pace(
    style: Res<HumanStyle>,
    mut humans: Query<(&mut Movable, &Pace, Has<HumanFleeTag>), With<Human>>,
) {
    for (mut movable, pace, fleeing) in &mut humans {
        let base = if fleeing {
            HUMAN_FLEE_SPEED
        } else {
            HUMAN_WALK_SPEED
        };
        movable.speed = pace.speed(base, style.spread);
    }
}

/// Точка, к которой идут «в этот дом»: вход из OSM, если он у дома размечен,
/// иначе случайная вершина контура. Выбор идёт от здания, а не от общего
/// списка входов, именно поэтому: входов на город сотни (Тула — 431 на 6946
/// домов), и адресуйся пешки прямо к ним, двадцать тысяч человек ходили бы по
/// одним и тем же дверям.
fn building_target(building: &PolyArea, rng: &mut impl Rng) -> Vec2 {
    let points = if building.entrances.is_empty() {
        &building.outer
    } else {
        &building.entrances
    };
    points[rng.random_range(0..points.len())]
}

/// Здание «по курсу»: из `WANDER_BUILDING_TRIES` случайных зданий
/// берётся первое, попавшее в конус вокруг `heading`; если ни одно не попало —
/// лучшее по направлению из выборки. Полный перебор 7500 зданий тут не нужен:
/// цель и так случайная, важно лишь не отправить пешку назад.
///
/// `ban` — запретный конус после паники (`PanicRecoil`). Кандидат в конусе или
/// ближе `RECOIL_MIN_ERRAND` отсеивается до всякого сравнения, то есть не
/// участвует и в запасном «лучшем по направлению»: именно этот запасной путь
/// раньше и мог вернуть здание почти строго назад, к демону. Вся выборка
/// отсеялась — `None`, вызывающий перебросит на следующем кадре.
fn pick_building_ahead(
    map: &MapData,
    rng: &mut impl Rng,
    position: Vec2,
    heading: Vec2,
    ban: Option<Vec2>,
) -> Option<Vec2> {
    let cone_cos = WANDER_CONE.cos();
    let mut best: Option<(f32, Vec2)> = None;

    for _ in 0..WANDER_BUILDING_TRIES {
        let building = &map.buildings[rng.random_range(0..map.buildings.len())];
        let point = building_target(building, rng);
        if ban.is_some() && point.distance_squared(position) < RECOIL_MIN_ERRAND * RECOIL_MIN_ERRAND
        {
            continue;
        }
        let Some(direction) = (point - position).try_normalize() else {
            continue;
        };
        if ban.is_some_and(|ban| in_recoil_cone(direction, ban)) {
            continue;
        }
        let alignment = direction.dot(heading);
        if alignment >= cone_cos {
            return Some(point);
        }
        if best.is_none_or(|(best_alignment, _)| alignment > best_alignment) {
            best = Some((alignment, point));
        }
    }

    best.map(|(_, point)| point)
}

/// Пауза, которую человек выстоит на следующей цели: `HUMAN_WANDER_PAUSE_SHARE`
/// останавливаются на 2–10 с, остальные уходят дальше тем же кадром.
///
/// Бросок делается на цель, а не на человека: постоянно спешащая пятая часть
/// населения — это два разных сорта пешеходов, а нужен один, который иногда
/// останавливается. И бросается он заранее, при выборе цели, потому что это тот
/// же кадр, где пауза и так перезаряжается, — прибытие о ней ничего не знает.
///
/// Нулевая пауза срабатывает сразу: `Timer` в `Once` считает себя истёкшим при
/// первом же `tick`, если `elapsed >= duration`.
fn roll_wander_pause(rng: &mut impl Rng) -> std::time::Duration {
    if rng.random_range(0.0..1.0) >= HUMAN_WANDER_PAUSE_SHARE {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f32(rng.random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1))
}

/// Мирное блуждание: пауза 2–10 с на каждом пятом прибытии (см.
/// `roll_wander_pause`), затем цель — 80% идут «по делам» к
/// случайному зданию города (длинные маршруты, настоящая нагрузка на
/// pathfinding), 20% гуляют в 20–40 м от себя. Первая цель после спавна —
/// всегда прогулка поблизости (`HumanFirstWanderTag`).
///
/// Первая цель после паники (`PanicRecoil`) — наоборот, всегда дальняя и не в
/// запретном конусе: человек, отбежавший от демона, не должен ни вернуться
/// туда же, ни остаться в том же квартале.
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    pathfinder: Pathfinder,
    map: Res<MapData>,
    seed: Res<WorldSeed>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            &mut WanderPause,
            &mut WanderHeading,
            &PawnId,
            &mut WanderIndex,
            Option<&PanicRecoil>,
            Has<HumanFirstWanderTag>,
        ),
        (
            With<Human>,
            With<HumanWanderTag>,
            // тег держит ровно `Idle` и `PathfindingError` — те же состояния,
            // что отбирала проверка в теле цикла (она осталась подстраховкой)
            With<NeedsWanderTarget>,
        ),
    >,
) {
    let navmesh = pathfinder.navmesh.read();

    for (
        entity,
        sim_position,
        mut movable,
        mut pause,
        mut heading,
        pawn_id,
        mut wander_index,
        recoil,
        is_first_wander,
    ) in &mut query
    {
        if !matches!(
            movable.state,
            MovableState::Idle | MovableState::PathfindingError(_)
        ) {
            continue;
        }

        pause.0.tick(time.delta());
        if !pause.0.is_finished() {
            continue;
        }

        // Поток заводится здесь, а не в начале итерации: до этой точки решение
        // ещё не принимается, а `next` сдвигает счётчик — тикающая пауза
        // прокручивала бы номера решений вхолостую, и число прокруток зависело
        // бы от частоты кадров.
        //
        // Засев — `(PawnId, номер решения)`, а не общий поток на систему и не
        // живой поток на пешке: выбор цели не должен зависеть ни от порядка
        // обхода запроса, ни от того, сколько выборок съело прошлое решение
        // этой же пешки
        let rng = &mut wander_index.next(seed.0, RngDomain::Human, pawn_id.0);

        // после паники — только «по делам», причём это перебивает и бросок
        // 80/20, и `HumanFirstWanderTag`: тот существует, чтобы 20 000 пешек
        // не подали маршрут через весь город одним кадром, а паника на спавне
        // достаёт лишь толпу в 60 м от портала, и успокаиваются те вразнобой
        let to_building = !map.buildings.is_empty()
            && (recoil.is_some()
                || (!is_first_wander && rng.random_range(0.0..1.0) < WANDER_TO_BUILDING_SHARE));
        let target = if to_building {
            // «по делам»: вершина контура здания, лежащего по курсу — иначе
            // маршрут через весь город разворачивает пешку назад
            let Some(point) =
                pick_building_ahead(&map, rng, sim_position.0, heading.0, recoil.map(|r| r.0))
            else {
                // вся выборка отсеялась запретом — новые восемь зданий
                // следующим кадром; сорваться на прогулку поблизости нельзя,
                // это ровно то, от чего человека и уводят
                continue;
            };
            point
        } else {
            // прогулка поблизости — в конусе вокруг курса
            let turn = rng.random_range(-WANDER_CONE..WANDER_CONE);
            let direction = Vec2::from_angle(turn).rotate(heading.0);
            let distance = rng.random_range(HUMAN_WANDER_RANGE.0..HUMAN_WANDER_RANGE.1);
            let point = (sim_position.0 + direction * distance)
                .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN);
            // под запретом сюда попадают только жители города без зданий:
            // дальнего маршрута там не существует, и прогулка с проверкой
            // конуса — лучшее доступное поведение. Проверять надо после
            // клампа: у самого края карты он и разворачивает направление
            if let Some(ban) = recoil.map(|r| r.0)
                && (point - sim_position.0)
                    .try_normalize()
                    .is_none_or(|direction| in_recoil_cone(direction, ban))
            {
                continue;
            }
            point
        };

        let Some(target_tile) = find_passable_tile_near(&navmesh, world_to_tile(target)) else {
            continue;
        };
        // курс — по фактически выбранной цели, следующая пойдёт от него
        if let Some(direction) = (tile_center(target_tile) - sim_position.0).try_normalize() {
            heading.0 = direction;
        }

        movable.to_pathfinding(
            entity,
            world_to_tile(sim_position.0),
            target_tile,
            &mut commands,
        );
        if is_first_wander {
            commands.entity(entity).remove::<HumanFirstWanderTag>();
        }
        // запрет живёт ровно до первой удачной цели
        if recoil.is_some() {
            commands.entity(entity).remove::<PanicRecoil>();
        }

        // следующая пауза — уже после прибытия
        pause.0.set_duration(roll_wander_pause(rng));
        pause.0.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Запрет — на направление к демону и всё, что ближе 45° к нему.
    #[test]
    fn recoil_cone_catches_the_way_back() {
        let ban = Vec2::X;
        assert!(in_recoil_cone(Vec2::X, ban));
        assert!(in_recoil_cone(Vec2::from_angle(0.7), ban));
        assert!(in_recoil_cone(Vec2::from_angle(-0.7), ban));
    }

    /// Всё, что дальше 45°, — разрешено, включая ровно противоположное.
    #[test]
    fn recoil_cone_lets_the_rest_through() {
        let ban = Vec2::X;
        assert!(!in_recoil_cone(Vec2::from_angle(0.8), ban));
        assert!(!in_recoil_cone(Vec2::Y, ban));
        assert!(!in_recoil_cone(-Vec2::X, ban));
    }

    /// Точно на границе конуса цель разрешена: сравнение строгое.
    #[test]
    fn recoil_cone_boundary_is_allowed() {
        assert!(!in_recoil_cone(Vec2::from_angle(RECOIL_CONE), Vec2::X));
    }

    /// Отпечаток населения: всё, что разыгрывается при спавне, в битах — на
    /// float'ах сравнивать нечего, нужна побайтовая одинаковость.
    fn population_fingerprint(seed: u64) -> Vec<(u32, u32, u32, u32, u32, u32)> {
        let mut world = World::new();
        let navmesh = crate::navigation::Navmesh::default();
        spawn_population(&mut world.commands(), &navmesh, 0.3, seed);
        world.flush();

        let mut query = world.query::<(&PawnId, &Transform, &Pace, &WanderHeading)>();
        let mut rows: Vec<_> = query
            .iter(&world)
            .map(|(pawn_id, transform, pace, heading)| {
                (
                    pawn_id.0,
                    transform.translation.x.to_bits(),
                    transform.translation.y.to_bits(),
                    pace.0.to_bits(),
                    heading.0.x.to_bits(),
                    heading.0.y.to_bits(),
                )
            })
            .collect();
        rows.sort_unstable();
        rows
    }

    /// Население — чистая функция от seed. Это фундамент всего остального:
    /// если спавн разъезжается, повторять симуляцию уже нечему.
    #[test]
    fn population_is_a_function_of_the_seed() {
        assert_eq!(population_fingerprint(7), population_fingerprint(7));
    }

    /// ...и при этом seed что-то значит: разные seed дают разное население.
    #[test]
    fn different_seeds_give_different_populations() {
        assert_ne!(population_fingerprint(7), population_fingerprint(8));
    }
}
