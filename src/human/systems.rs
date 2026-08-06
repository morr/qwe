use bevy::prelude::*;
use rand::Rng;

use crate::grid::{tile_center, world_to_tile};
use crate::human::components::{
    Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, WanderHeading,
    WanderPause,
};
use crate::loading::AppState;
use crate::map::osm::{MapData, PolyArea};
use crate::movement::{Movable, MovableState, SimPosition};
use crate::navigation::{ArcNavmesh, Pathfinder, find_passable_tile_near};
use crate::settings::{
    HUMAN_COUNT, HUMAN_FLEE_SPEED, HUMAN_SIZE, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE,
    HUMAN_WANDER_PAUSE_SHARE, HUMAN_WANDER_RANGE, MAP_SIZE, unit_z,
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

pub fn spawn_humans(mut commands: Commands, arc_navmesh: Res<ArcNavmesh>, style: Res<HumanStyle>) {
    spawn_population(&mut commands, &arc_navmesh.read(), style.spread);
}

/// Спавн населения; вызывается на старте и при рестарте сцены.
pub fn spawn_population(
    commands: &mut Commands,
    navmesh: &crate::navigation::Navmesh,
    spread: f32,
) {
    let mut rng = rand::rng();

    for _ in 0..HUMAN_COUNT {
        let tile = loop {
            let candidate = IVec2::new(
                rng.random_range(0..navmesh.grid_size.x),
                rng.random_range(0..navmesh.grid_size.y),
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
            WanderHeading(Vec2::from_angle(
                rng.random_range(0.0..std::f32::consts::TAU),
            )),
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
fn pick_building_ahead(map: &MapData, rng: &mut impl Rng, position: Vec2, heading: Vec2) -> Vec2 {
    let cone_cos = WANDER_CONE.cos();
    let mut best: Option<(f32, Vec2)> = None;

    for _ in 0..WANDER_BUILDING_TRIES {
        let building = &map.buildings[rng.random_range(0..map.buildings.len())];
        let point = building_target(building, rng);
        let Some(direction) = (point - position).try_normalize() else {
            continue;
        };
        let alignment = direction.dot(heading);
        if alignment >= cone_cos {
            return point;
        }
        if best.is_none_or(|(best_alignment, _)| alignment > best_alignment) {
            best = Some((alignment, point));
        }
    }

    best.map(|(_, point)| point).unwrap_or(position)
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
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    pathfinder: Pathfinder,
    map: Res<MapData>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            &mut WanderPause,
            &mut WanderHeading,
            Has<HumanFirstWanderTag>,
        ),
        (With<Human>, With<HumanWanderTag>),
    >,
) {
    let mut rng = rand::rng();
    let navmesh = pathfinder.navmesh.read();

    for (entity, sim_position, mut movable, mut pause, mut heading, is_first_wander) in &mut query {
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

        let to_building = !is_first_wander
            && rng.random_range(0.0..1.0) < WANDER_TO_BUILDING_SHARE
            && !map.buildings.is_empty();
        let target = if to_building {
            // «по делам»: вершина контура здания, лежащего по курсу — иначе
            // маршрут через весь город разворачивает пешку назад
            pick_building_ahead(&map, &mut rng, sim_position.0, heading.0)
        } else {
            // прогулка поблизости — в конусе вокруг курса
            let turn = rng.random_range(-WANDER_CONE..WANDER_CONE);
            let direction = Vec2::from_angle(turn).rotate(heading.0);
            let distance = rng.random_range(HUMAN_WANDER_RANGE.0..HUMAN_WANDER_RANGE.1);
            (sim_position.0 + direction * distance)
                .clamp(Vec2::splat(MAP_MARGIN), MAP_SIZE - MAP_MARGIN)
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

        // следующая пауза — уже после прибытия
        pause.0.set_duration(roll_wander_pause(&mut rng));
        pause.0.reset();
    }
}
