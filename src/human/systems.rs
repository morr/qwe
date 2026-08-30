use bevy::prelude::*;
use rand::Rng;

use crate::grid::tile_center;
use crate::human::components::{
    Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, PanicRecoil,
    WanderHeading, WanderPause,
};
use crate::loading::AppState;
use crate::map::osm::{MapData, PolyArea};
use crate::movement::{
    Movable, NeedsWanderTarget, SimPosition, heading_towards, point_in_cone, ready_to_pick,
    request_wander_path,
};
use crate::navigation::{ArcNavmesh, Backend};
use crate::rng::{
    PawnId, RngDomain, SimRng, Species, WanderIndex, WorldSeed, decision_stream, stream,
};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_SIZE, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, HUMAN_WANDER_PAUSE_SHARE,
    HUMAN_WANDER_RANGE, HUMAN_WANDER_TO_BUILDING_SHARE, RECOIL_CONE, RECOIL_MIN_ERRAND,
    WANDER_CONE, unit_z,
};

/// Сколько зданий перебирается в поисках цели «по делам» в конусе курса;
/// если ни одно не попало — берётся ближайшее по направлению из выборки.
const WANDER_BUILDING_TRIES: usize = 8;
/// Сколько случайных выборок отводится на размещение одного человека, прежде
/// чем тайл добирается сканом сетки.
///
/// Потолок высокий намеренно. На городе проходима примерно половина тайлов —
/// хватает одной-двух выборок. Самый разреженный штатный мир — двор реплея
/// (`map/osm/fixture.rs::crowded_yard`): 3472 проходимых тайла из 5 180 000,
/// то есть в среднем ~1500 выборок на человека; вероятность, что кому-то из
/// населения не хватит ста тысяч, — порядка e^-67. Значит скан включается
/// только на навмеше, который сломан, а не просто тесен, и жребий рабочих
/// миров этот шаг не двигает.
const PLACEMENT_TRIES: usize = 100_000;

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
    size: Res<crate::human::PopulationSize>,
) {
    spawn_population(
        &mut commands,
        &arc_navmesh.read(),
        style.spread,
        seed.0,
        size.0,
    );
}

/// Спавн населения; вызывается на старте и при рестарте сцены.
///
/// Два потока жребия, а не один: размещение идёт общим потоком `Population`
/// (цикл отбора тянет переменное число выборок на человека, и это нормально
/// внутри одного последовательного обхода), а всё личное — цвет, темп, курс —
/// уже из потока самой пешки по её [`PawnId`]. Так внешность и повадки
/// человека номер N не зависят от того, сколько раз отбор промахнулся по
/// непроходимым тайлам у его соседей.
///
/// Отбор ограничен сверху дважды: сетка без единого проходимого тайла
/// отбрасывается целиком до цикла, а на человека отводится
/// [`PLACEMENT_TRIES`] выборок, после чего тайл берётся сканом
/// ([`Navmesh::passable_from`]). Без этого пустой или сплошь заблокированный
/// навмеш — упавший OSM-экстракт, регрессия заливки, смена размера
/// навтайла — вешал бы `OnEnter(AppState::Playing)` без единой строки в логе.
pub fn spawn_population(
    commands: &mut Commands,
    navmesh: &crate::navigation::Navmesh,
    spread: f32,
    world_seed: u64,
    count: usize,
) {
    let mut placement = stream(world_seed, RngDomain::Population, 0);
    // ни одного проходимого тайла — расселять некуда, и отбор ниже крутился
    // бы вечно; заодно это единственный случай, когда `grid_size` нулевой и
    // `random_range(0..0)` паникует
    if navmesh.passable_from(IVec2::ZERO).is_none() {
        error!(
            "population: navmesh {}x{} has no passable tile, {count} humans not placed",
            navmesh.grid_size.x, navmesh.grid_size.y
        );
        return;
    }
    // сколько человек пришлось доставить сканом вместо жребия — одной
    // строкой после цикла, а не двадцатью тысячами строк внутри него
    let mut scanned = 0usize;

    for index in 0..count {
        let pawn_id = index as u32;
        let mut rng = decision_stream(world_seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);

        let mut candidate = IVec2::ZERO;
        let mut drawn = None;
        for _ in 0..PLACEMENT_TRIES {
            candidate = IVec2::new(
                placement.random_range(0..navmesh.grid_size.x),
                placement.random_range(0..navmesh.grid_size.y),
            );
            if navmesh.is_passable(candidate.x, candidate.y) {
                drawn = Some(candidate);
                break;
            }
        }
        // `passable_from` не может вернуть `None`: пустую сетку отсекла
        // проверка выше, и с тех пор навмеш никто не менял
        let tile = match drawn {
            Some(tile) => tile,
            None => {
                scanned += 1;
                navmesh.passable_from(candidate).unwrap_or(candidate)
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
            // номер уникален только внутри вида, поэтому вид едет рядом с ним
            Species::Human,
            WanderIndex::ready(),
            DespawnOnExit(AppState::Playing),
            Name::new("human"),
        ));
    }
    if scanned > 0 {
        warn!(
            "population: {scanned} of {count} humans placed by grid scan — navmesh is nearly fully blocked"
        );
    }
}

/// `HumanStyle` несёт два независимых поля: `spread` и `body_radius`. Гейт по
/// `resource_changed::<HumanStyle>` запускал обход всей популяции на каждый
/// шаг протяжки ползунка **Body radius**, хотя сама система читает только
/// `spread` и не трогает радиус вовсе.
///
/// Состояние хранится в `Local`; сравнение по значению заодно гасит запись
/// того же числа (например, `ResetSettings` при уже дефолтных настройках).
/// Смена режима детерминизма на ходу — каждой регистрации свой `Local`, и
/// включённая позже ветка на первом прогоне увидит расхождение и догонит
/// правку, ровно как пропущенная по старому ран-кондишену система.
pub fn spread_changed(style: Res<HumanStyle>, mut applied: Local<Option<f32>>) -> bool {
    let current = style.spread;
    let changed = *applied != Some(current);
    *applied = Some(current);
    changed
}

/// Ползунок разброса — людям, уже гуляющим по городу; аналог
/// `sync_demon_speed`, и так же не каждый кадр: гейт — [`spread_changed`].
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

/// Куда человек хочет попасть на этом шаге — вся видовая политика мирного
/// блуждания; обвязка (заявка на путь, курс, снятие тегов, пауза) остаётся в
/// [`pick_wander_targets`].
///
/// 80% идят «по делам» к случайному зданию города (длинные маршруты, настоящая
/// нагрузка на pathfinding), 20% гуляют в 20–40 м от себя. Первая цель после
/// спавна — всегда прогулка поблизости (`is_first_wander`), первая после паники
/// (`ban`) — наоборот, всегда дальняя и не в запретном конусе.
///
/// `None` — цели на этом кадре нет: либо вся выборка зданий отсеялась запретом,
/// либо прогулка вышла в запретный конус. Вызывающий пропускает пешку целиком —
/// ни заявки, ни снятия тегов, ни перезарядки паузы.
///
/// **Число и порядок бросков — часть потока решений** (`CONTEXT.md`, «Decision
/// stream»): жребий 80/20 делается только там, где делался и раньше — короткое
/// замыкание `&&` не пускает его ни при пустом списке зданий, ни под запретом,
/// ни на первой цели; дальше идут броски одной выбранной ветки.
fn choose_target(
    map: &MapData,
    rng: &mut SimRng,
    position: Vec2,
    heading: Vec2,
    ban: Option<Vec2>,
    is_first_wander: bool,
) -> Option<Vec2> {
    // после паники — только «по делам», причём это перебивает и бросок
    // 80/20, и `HumanFirstWanderTag`: тот существует, чтобы 20 000 пешек
    // не подали маршрут через весь город одним кадром, а паника на спавне
    // достаёт лишь толпу в 60 м от портала, и успокаиваются те вразнобой
    let to_building = !map.buildings.is_empty()
        && (ban.is_some()
            || (!is_first_wander && rng.random_range(0.0..1.0) < HUMAN_WANDER_TO_BUILDING_SHARE));

    if to_building {
        // «по делам»: вершина контура здания, лежащего по курсу — иначе
        // маршрут через весь город разворачивает пешку назад. Вся выборка
        // отсеялась запретом — `None`, новые восемь зданий следующим кадром;
        // сорваться на прогулку поблизости нельзя, это ровно то, от чего
        // человека и уводят
        return pick_building_ahead(map, rng, position, heading, ban);
    }

    // прогулка поблизости — в конусе вокруг курса
    let point = point_in_cone(rng, position, heading, WANDER_CONE, HUMAN_WANDER_RANGE);
    // под запретом сюда попадают только жители города без зданий: дальнего
    // маршрута там не существует, и прогулка с проверкой конуса — лучшее
    // доступное поведение. Проверять надо после клампа: у самого края карты он
    // и разворачивает направление
    if let Some(ban) = ban
        && (point - position)
            .try_normalize()
            .is_none_or(|direction| in_recoil_cone(direction, ban))
    {
        return None;
    }
    Some(point)
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

/// Мирное блуждание, один шаг скелета `movement/wander.rs`: отсев по состоянию
/// → пауза 2–10 с на каждом пятом прибытии (см. `roll_wander_pause`) → поток
/// решений → [`choose_target`] (вся политика «куда») → заявка на путь, курс,
/// снятие тегов и перезарядка паузы.
///
/// Каждый из четырёх `continue` — своё основание пропустить пешку до следующего
/// кадра, и ни один из них не двигает состояние вперёд.
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    backend: Res<Backend>,
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
    let walkable = backend.walkable();

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
        if !ready_to_pick(&movable.state) {
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

        // цель на этом кадре не выбралась — пешка ждёт следующего кадра, ничего
        // не сбрасывая: ни `PanicRecoil`, ни `HumanFirstWanderTag`, ни паузу
        let Some(target) = choose_target(
            &map,
            rng,
            sim_position.0,
            heading.0,
            recoil.map(|r| r.0),
            is_first_wander,
        ) else {
            continue;
        };

        let Some(target_tile) = request_wander_path(
            &mut commands,
            &walkable,
            entity,
            &mut movable,
            sim_position.0,
            target,
        ) else {
            continue;
        };
        // курс — по фактически выбранной цели, следующая пойдёт от него
        if let Some(direction) = heading_towards(sim_position.0, target_tile) {
            heading.0 = direction;
        }

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
    use crate::map::osm::{
        AreaKind,
        fixture::{building, rect},
    };
    use crate::settings::{HUMAN_BODY_RADIUS_MAX, HUMAN_SPEED_SPREAD_MAX, MAP_SIZE};

    /// Мир одного гуляющего: гейту хватает ресурса, системе — `Movable` и `Pace`.
    fn pace_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HumanStyle>()
            .add_systems(Update, sync_human_pace.run_if(spread_changed));

        let entity = app
            .world_mut()
            .spawn((Human, HumanWanderTag, Movable::new(0.0), Pace(1.0)))
            .id();

        // первый прогон: Local = None → система отработает раз
        app.update();

        (app, entity)
    }

    /// После прогрева ползунок **Body radius** не должен менять скорости людей.
    /// Часовой на `Movable::speed` — проверка лучше, чем сравнение скоростей,
    /// потому что при неизменном `spread` система переписала бы ровно то же число,
    /// и «лишний проход» иначе неотличим от его отсутствия.
    #[test]
    fn the_body_radius_slider_leaves_the_crowd_alone() {
        let (mut app, entity) = pace_app();
        {
            // `entity_mut` держится в переменной: `Mut<Movable>` заимствует
            // именно его, и временное значение умерло бы раньше заимствования
            let mut entity_mut = app.world_mut().entity_mut(entity);
            let mut movable = entity_mut.get_mut::<Movable>().unwrap();
            movable.speed = -1.0; // часовой
        }

        app.world_mut().resource_mut::<HumanStyle>().body_radius = HUMAN_BODY_RADIUS_MAX;
        app.update();

        // если система запустилась, -1.0 был бы перезаписан
        assert_eq!(
            app.world().entity(entity).get::<Movable>().unwrap().speed,
            -1.0
        );
    }

    /// Смена `spread` должна гонять систему.
    #[test]
    fn moving_the_spread_slider_retunes_the_crowd() {
        let (mut app, entity) = pace_app();
        let new_spread = HUMAN_SPEED_SPREAD_MAX;
        app.world_mut().resource_mut::<HumanStyle>().spread = new_spread;
        app.update();

        let expected = Pace(1.0).speed(HUMAN_WALK_SPEED, new_spread);
        assert_eq!(
            app.world().entity(entity).get::<Movable>().unwrap().speed,
            expected
        );
    }

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
        spawn_population(
            &mut world.commands(),
            &navmesh,
            0.3,
            seed,
            crate::settings::HUMAN_COUNT,
        );
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

    /// Навмеш, где проходимы ровно перечисленные дырки в сплошной застройке.
    fn navmesh_blocked_except(holes: Vec<Vec<Vec2>>) -> crate::navigation::Navmesh {
        let mut map = MapData::default();
        map.buildings
            .push(building(rect(Vec2::ZERO, crate::settings::MAP_SIZE), holes));
        let mut navmesh = crate::navigation::Navmesh::default();
        navmesh.fill_from_mapdata(&map);
        navmesh
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

    /// Пустой навмеш не спавнит никого: приложение остаётся живым с
    /// единственной строкой в логе.
    #[test]
    fn population_refuses_a_navmesh_without_passable_tiles() {
        let mut world = World::new();
        let navmesh = navmesh_blocked_except(vec![]);
        spawn_population(&mut world.commands(), &navmesh, 0.3, 7, 4);
        world.flush();
        let count = world.query::<&Human>().iter(&world).count();
        assert_eq!(count, 0, "пустой навмеш не должен спавнить никого");
    }

    /// Сплошь заблокированный навмеш (одна дырка в тайл) — все люди в тайле
    /// дырки, но не более чем за максимум попыток.
    #[test]
    fn population_falls_back_to_a_grid_scan() {
        let mut world = World::new();
        let hole_center = crate::settings::MAP_SIZE * 0.25;
        let hole_tile = crate::grid::world_to_tile(hole_center);
        let hole_tile_size = crate::settings::navtile_size();
        let hole_rect = rect(hole_center, hole_center + hole_tile_size);
        let navmesh = navmesh_blocked_except(vec![hole_rect]);
        spawn_population(&mut world.commands(), &navmesh, 0.3, 7, 4);
        world.flush();

        let count = world.query::<&Human>().iter(&world).count();
        assert_eq!(count, 4, "все люди должны быть спавнены даже со сканом");

        let hole_center_world = tile_center(hole_tile);
        let mut query = world.query::<&Transform>();
        for transform in query.iter(&world) {
            let position = transform.translation.truncate();
            assert_eq!(
                position, hole_center_world,
                "все люди должны быть в центре дырки"
            );
        }
    }

    /// Вершина контура — детерминированно, т.к. `building_target` индексирует
    /// `outer` по случайному числу, а в тесте контур из одной вершины.
    fn building_at(corner: Vec2) -> PolyArea {
        PolyArea {
            outer: vec![corner],
            holes: vec![],
            kind: AreaKind::Building,
            height: None,
            entrances: vec![],
        }
    }

    /// На первую цель после спавна: жребий 80/20 не разыгрывается, идёт прогулка
    /// поблизости, независимо от расстояния до зданий.
    #[test]
    fn the_first_target_after_spawn_is_a_nearby_stroll() {
        let mut map = MapData::default();
        let home_far = MAP_SIZE / 2.0 + Vec2::X * 300.0;
        map.buildings.push(building_at(home_far));

        let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
        let position = MAP_SIZE / 2.0;
        let heading = Vec2::X;

        for _ in 0..50 {
            let target = choose_target(&map, &mut rng, position, heading, None, true)
                .expect("прогулка поблизости всегда успешна");
            assert!(
                target.distance(position) <= HUMAN_WANDER_RANGE.1,
                "цель должна быть в пределах диапазона прогулки, а не у дома в 300 м"
            );
        }
    }

    /// Паника перебивает `is_first_wander`: если `ban` установлена, ищем дом,
    /// независимо от `is_first_wander = true`.
    #[test]
    fn panic_forces_an_errand_over_the_first_wander_tag() {
        let mut map = MapData::default();
        let home_ahead = MAP_SIZE / 2.0 + Vec2::X * 300.0;
        map.buildings.push(building_at(home_ahead));

        let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
        let position = MAP_SIZE / 2.0;
        let heading = Vec2::X;
        let ban = Some(-Vec2::X); // демон за спиной

        let target = choose_target(&map, &mut rng, position, heading, ban, true)
            .expect("дом по курсу — всегда успешен");
        assert_eq!(target, home_ahead, "цель должна быть вершиной дома");
    }

    /// Если вся выборка зданий в запретном конусе или ближе `RECOIL_MIN_ERRAND`,
    /// `None` — вызывающий обязан пропустить пешку, сохранив паническое состояние.
    #[test]
    fn a_ban_that_kills_the_whole_sample_gives_no_target() {
        let mut map = MapData::default();
        let home_back = MAP_SIZE / 2.0 - Vec2::X * 300.0; // только назад
        map.buildings.push(building_at(home_back));

        let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
        let position = MAP_SIZE / 2.0;
        let heading = Vec2::X;
        let ban = Some(-Vec2::X);

        let target = choose_target(&map, &mut rng, position, heading, ban, false);
        assert_eq!(
            target, None,
            "единственный дом назад и в конусе — должен быть отсеян"
        );
    }

    /// В городе без зданий прогулка проверяется на запрет: если конус запретит
    /// её, `None` отправляет пешку в холостой цикл, иначе цель разрешена.
    #[test]
    fn a_city_without_buildings_strolls_outside_the_ban_cone() {
        let map = MapData::default(); // нет зданий
        let mut rng = decision_stream(1, RngDomain::Human, 0, 1);
        let position = MAP_SIZE / 2.0;
        let heading = Vec2::X;
        let ban = Vec2::X;

        let mut had_none = false;
        for _ in 0..200 {
            if let Some(target) = choose_target(&map, &mut rng, position, heading, Some(ban), false)
            {
                let direction = (target - position).normalize();
                assert!(
                    !in_recoil_cone(direction, ban),
                    "цель должна быть вне запретного конуса"
                );
            } else {
                had_none = true;
            }
        }
        assert!(had_none, "запрет должен отсечь хотя бы один вызов из 200");
    }

    /// Политика не должна съедать или добавлять броски поверх `point_in_cone`:
    /// жребий 80/20 и сама функция выбора точки в конусе занимают ровно по
    /// одному `random_range`, и тест страхует сдвиг потока решений.
    #[test]
    fn the_stroll_branch_spends_exactly_two_draws() {
        let map = MapData::default(); // нет зданий
        let position = MAP_SIZE / 2.0;
        let heading = Vec2::X;

        // первый поток: вызовем choose_target, потом `random_range` на нём
        let mut rng1 = decision_stream(1, RngDomain::Human, 0, 1);
        let _ = choose_target(&map, &mut rng1, position, heading, None, true);
        let draw1: f32 = rng1.random_range(0.0..1.0);

        // второй поток: `point_in_cone` напрямую, потом тот же `random_range`
        let mut rng2 = decision_stream(1, RngDomain::Human, 0, 1);
        let _ = point_in_cone(
            &mut rng2,
            position,
            heading,
            WANDER_CONE,
            HUMAN_WANDER_RANGE,
        );
        let draw2: f32 = rng2.random_range(0.0..1.0);

        assert_eq!(draw1.to_bits(), draw2.to_bits(), "потоки должны совпадать");
    }
}
