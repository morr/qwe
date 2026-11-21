use bevy::prelude::*;
use rand::RngExt;

use crate::human::components::{
    Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag, Pace, PanicRecoil,
    PopulationSize, WanderHeading, WanderPause,
};
use crate::human::look::{human_body, roll_attire};
use crate::loading::AppState;
use crate::map::osm::{MapData, PolyArea};
use crate::map::z::unit_z;
use crate::movement::{
    Movable, NeedsWanderTarget, SimPosition, assert_ready_to_pick, heading_towards, point_in_cone,
    request_wander_path,
};
use crate::navigation::{ArcNavmesh, Backend};
use crate::rng::{
    PawnId, RngDomain, SimRng, Species, WanderIndex, WorldSeed, decision_stream, stream,
};
use crate::settings::{
    HUMAN_FLEE_SPEED, HUMAN_WALK_SPEED, HUMAN_WANDER_PAUSE, HUMAN_WANDER_PAUSE_SHARE,
    HUMAN_WANDER_RANGE, HUMAN_WANDER_TO_BUILDING_SHARE, RECOIL_CONE, RECOIL_MIN_ERRAND,
    WANDER_CONE,
};
use crate::silhouette::Silhouettes;

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

pub fn spawn_humans(mut commands: Commands, population: PopulationSpawn) {
    population.spawn(&mut commands);
}

/// Партионный спавн: сколько пешек насос расселяет за один вызов.
///
/// Планка готовности Фазы 1 (`docs/population-x10-plan.md`) — расселение 200k
/// без рамки длиннее ~50 мс. Живой замер звена №46 (Париж, 200k): партия в
/// 10 000 стоит рамке ~100–130 мс — примерно 7–10 мкс на пешку поверх
/// постоянных ~20–30 мс кадра прогрева (выбор цели свежей партии, проводка
/// трансформов, загрузка спрайтов в GPU), поэтому партия — 4 000: расчётные
/// ~45–60 мс на рамке партии. Мельче — расселение тянется лишние кадры
/// прогрева, которому вдобавок маршрутизировать прилетающих.
pub const SPAWN_BATCH: usize = 4_000;

/// Недосселённое население — очередь насоса ([`PopulationSpawn::pump`]).
///
/// Вставляется [`PopulationSpawn::spawn`] вместо немедленного спавна; насос
/// снимает ресурс, когда очередь исчерпана. Рестарт и переключение города
/// идут через тот же заказ: деспаун сцены случается раньше, остаток очереди
/// перезаписывается целиком.
#[derive(Resource, Debug)]
pub struct PendingPopulation {
    count: usize,
    cursor: usize,
    /// Центры тайлов всех пешек. Считается на первом вызове насоса: размещение
    /// (rejection-sampling) на порядок дешевле флеша сущностей, а единый поток
    /// размещения жил и умирал внутри одного вызова — хранить генератор в
    /// ресурсе между вызовами контракт детерминизма запрещает, поэтому позиции
    /// всего населения готовы заранее и партия лишь спавнит по готовым.
    placements: Option<Vec<Vec2>>,
}

impl PendingPopulation {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            cursor: 0,
            placements: None,
        }
    }
}

/// Всё, чем расселяется население, одним `SystemParam`.
///
/// Пятёрку ресурсов читают одинаково все заказчики расселения — `spawn_humans`
/// (`WorldInitSet::Spawn`), `restart::on_restart` и насос
/// ([`pump_pending_population`]), — и держаться в шаг они обязаны: рестарт
/// расселяет то же население, иначе сравнивать два прогона нечем. Врозь это
/// держалось дисциплиной, а пятый ресурс (атлас силуэтов) вдобавок упёр
/// `on_restart` в `clippy::too_many_arguments`.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PopulationSpawn<'w> {
    navmesh: Res<'w, ArcNavmesh>,
    style: Res<'w, HumanStyle>,
    seed: Res<'w, WorldSeed>,
    size: Res<'w, PopulationSize>,
    silhouettes: Res<'w, Silhouettes>,
}

impl PopulationSpawn<'_> {
    /// Заказать расселение населения по навмешу, каким он будет на первом
    /// вызове насоса: вместо немедленного спавна вставляется
    /// [`PendingPopulation`], и партиями по [`SPAWN_BATCH`] пешек его
    /// вычитывает [`pump_pending_population`].
    pub fn spawn(&self, commands: &mut Commands) {
        commands.insert_resource(PendingPopulation::new(self.size.0));
    }

    /// Один вызов насоса: расселить до `batch` пешек из очереди.
    /// Вернувшееся «true» — очередь исчерпана (или расселять нечего),
    /// ресурс можно снимать.
    pub fn pump(
        &self,
        commands: &mut Commands,
        pending: &mut PendingPopulation,
        batch: usize,
    ) -> bool {
        pump_batch(
            commands,
            &self.navmesh.read(),
            self.style.spread,
            self.seed.0,
            &self.silhouettes,
            pending,
            batch,
        )
    }
}

/// Насос расселения: до [`SPAWN_BATCH`] пешек за вызов, исчерпанную очередь
/// снимает.
///
/// Зарегистрирован дважды, как и весь конвейер:
///
/// - **`FixedUpdate`, `SimPipeline::BothModes`, до `TickStage::Tick`** —
///   партионность привязана к тику, не к кадру: сколько сущностей существует
///   на данном тике, не зависит от fps, иначе реплей расходится на разном
///   железе. Ребро к стадии держит точку синхронизации, на которой партии
///   применяются до поведения этого же тика;
/// - **`Update`, `SimPipeline::Live`** — живой прогон. Во время прогрева
///   `FixedUpdate` стоит на паузе, а `poll_warmup` (`loading.rs`) ждёт заявок
///   от пешек в кадре — население обязано существовать и подаваться по мере
///   прилёта партий, пока мир стоит. Симуляции в прогреве нет, поэтому
///   кадрозависимость спавна траекторию не меняет; после `Live` живая половина
///   насоса продолжается здесь же — живой режим реплеем не заявлен.
pub fn pump_pending_population(
    mut commands: Commands,
    mut pending: ResMut<PendingPopulation>,
    population: PopulationSpawn,
) {
    if population.pump(&mut commands, &mut pending, SPAWN_BATCH) {
        commands.remove_resource::<PendingPopulation>();
    }
}

/// Всё, что население получает при расселении помимо навмеша: четыре
/// значения ездят одним, а не четвёркой позиционных аргументов (тот же приём,
/// что у `demon::systems::DemonBirth`).
///
/// Отдельно от [`PopulationSpawn`]: стенды и тесты строят набор из голых
/// значений, ECS-ресурсов там нет вовсе.
pub struct PopulationBirth<'a> {
    /// Полуширина разброса личной скорости — `HumanStyle::spread`.
    pub spread: f32,
    pub world_seed: u64,
    pub count: usize,
    pub silhouettes: &'a Silhouettes,
}

/// Разместить всё население: по центру проходимого тайла на пешку.
///
/// Единый поток размещения (`RngDomain::Population`) — то самое исключение
/// из правила «у каждой пешки свой поток» (`src/rng.rs`), и законно оно ровно
/// пока обход — `for index in 0..count` внутри одного вызова этой функции:
/// цикл отбора тянет переменное число выборок на человека, но порядок
/// потребления — счётчик, а не обход запроса. Насос поэтому считает позиции
/// **всего** населения на первом вызове (`PendingPopulation::placements`) —
/// разрезание спавна на партии превратило бы иначе исключение в хранение
/// генератора в ресурсе между вызовами, то есть ровно то, что контракт
/// запрещает.
///
/// Отбор ограничен сверху дважды: сетка без единого проходимого тайла
/// отбрасывается целиком до цикла, а на человека отводится
/// [`PLACEMENT_TRIES`] выборок, после чего тайл берётся сканом
/// ([`Navmesh::passable_from`]). Без этого пустой или сплошь заблокированный
/// навмеш — упавший OSM-экстракт, регрессия заливки, смена размера
/// навтайла — вешал бы расселение без единой строки в логе.
fn place_population(
    navmesh: &crate::navigation::Navmesh,
    world_seed: u64,
    count: usize,
) -> Vec<Vec2> {
    let mut placement = stream(world_seed, RngDomain::Population, 0);
    // ни одного проходимого тайла — расселять некуда, и отбор ниже крутился
    // бы вечно; заодно это единственный случай, когда `grid_size` нулевой и
    // `random_range(0..0)` паникует
    if navmesh.passable_from(IVec2::ZERO).is_none() {
        error!(
            "population: navmesh {}x{} has no passable tile, {count} humans not placed",
            navmesh.grid_size.x, navmesh.grid_size.y
        );
        return Vec::new();
    }
    // сколько человек пришлось доставить сканом вместо жребия — одной
    // строкой после цикла, а не двадцатью тысячами строк внутри него
    let mut scanned = 0usize;

    let mut placements = Vec::with_capacity(count);
    for _ in 0..count {
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
        // тайл взяли снапшотом навмеша — и в мир переводим его его же
        // `tile_center`: глобальный считает по атомику навтайла, что верно
        // только пока снапшот и атомик совпадают (см. `navigation/navmesh.rs`)
        placements.push(navmesh.tile_center(tile));
    }
    if scanned > 0 {
        warn!(
            "population: {scanned} of {count} humans placed by grid scan — navmesh is nearly fully blocked"
        );
    }
    placements
}

/// Одна пешка: личный жребий — одежда, темп, курс — из её собственного потока
/// по [`PawnId`], затем бандл. Позицию приносит размещение
/// ([`place_population`]); партионность спавна на личный жребий влиять не
/// может — поток выводится из номера пешки, а не из места в очереди.
///
/// Одежда — первые три броска потока; палитра в `look.rs`. Без стартовой
/// паузы: все идут с первого кадра. Залп целей разруливают гейт видимости
/// диспетчера (мирные вне экрана путь не получают) и дешёвый HPA* —
/// рассинхронизация тут только заставляла пешек в кадре стоять первые секунды.
fn spawn_human(
    commands: &mut Commands,
    spread: f32,
    world_seed: u64,
    silhouettes: &Silhouettes,
    pawn_id: u32,
    position: Vec2,
) {
    let mut rng = decision_stream(world_seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);
    let attire = roll_attire(&mut rng);
    let pause = Timer::from_seconds(0.0, TimerMode::Once);
    // жребий двусторонний: минус — человек медленнее базы, плюс — быстрее
    let pace = Pace(rng.random_range(-1.0..=1.0));
    let heading = WanderHeading(Vec2::from_angle(
        rng.random_range(0.0..std::f32::consts::TAU),
    ));

    let (sprite, silhouette) = human_body(silhouettes, &attire);
    commands.spawn((
        // внешность — вложенным бандлом: плоский кортеж упёрся в предел
        // пятнадцати элементов `Bundle`
        (sprite, silhouette, attire),
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

/// Тело одного вызова насоса, отрезанное от ECS-ресурсов: его зовёт и
/// [`PopulationSpawn::pump`], и пин батчинга в тестах — голыми значениями.
/// Вернувшееся «true» — очередь исчерпана (или расселять нечего).
fn pump_batch(
    commands: &mut Commands,
    navmesh: &crate::navigation::Navmesh,
    spread: f32,
    world_seed: u64,
    silhouettes: &Silhouettes,
    pending: &mut PendingPopulation,
    batch: usize,
) -> bool {
    if pending.cursor >= pending.count {
        return true;
    }
    let placements = pending
        .placements
        .get_or_insert_with(|| place_population(navmesh, world_seed, pending.count));
    // пустой навмеш: `place_population` уже отлогировал ошибку; расселённых
    // нет ни одного — ровно как вёл себя немедленный спавн
    if placements.is_empty() {
        return true;
    }
    let from = pending.cursor;
    let to = (from + batch).min(pending.count);
    for (offset, &position) in placements[from..to].iter().enumerate() {
        spawn_human(
            commands,
            spread,
            world_seed,
            silhouettes,
            (from + offset) as u32,
            position,
        );
    }
    pending.cursor = to;
    pending.cursor >= pending.count
}

/// Спавн населения целиком одной командой; вызывается стендами и тестами.
/// Игра расселяется через насос ([`PopulationSpawn::spawn`] +
/// [`pump_pending_population`]) — там партия лишь спавнит пешек по заранее
/// готовым позициям, так что батчинг на жребий повлиять не может (пин
/// «партиями == не партиями» стоит рядом в тестах).
///
/// Два потока жребия, а не один: размещение идёт общим потоком `Population`
/// (см. [`place_population`]), а всё личное — цвет, темп, курс — из потока
/// самой пешки по её [`PawnId`]. Так внешность и повадки человека номер N не
/// зависят от того, сколько раз отбор промахнулся по непроходимым тайлам у
/// его соседей.
pub fn spawn_population(
    commands: &mut Commands,
    navmesh: &crate::navigation::Navmesh,
    birth: &PopulationBirth,
) {
    let placements = place_population(navmesh, birth.world_seed, birth.count);
    for (pawn_id, &position) in placements.iter().enumerate() {
        spawn_human(
            commands,
            birth.spread,
            birth.world_seed,
            birth.silhouettes,
            pawn_id as u32,
            position,
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
/// списка входов, именно поэтому: размеченных в OSM входов на город сотни
/// (Тула — 491 на 7723 дома), и адресуйся пешки прямо к ним, двадцать тысяч
/// человек ходили бы по одним и тем же дверям.
fn building_target(building: &PolyArea, rng: &mut impl RngExt) -> Vec2 {
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
    rng: &mut impl RngExt,
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
fn roll_wander_pause(rng: &mut impl RngExt) -> std::time::Duration {
    if rng.random_range(0.0..1.0) >= HUMAN_WANDER_PAUSE_SHARE {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f32(rng.random_range(HUMAN_WANDER_PAUSE.0..HUMAN_WANDER_PAUSE.1))
}

/// Радиус широкого кольца просеивания первого выхода гуляющего. Обычное
/// просеивание цели смотрит 8 соседей тайла (`Walkable::sift_target`), и на
/// плотном городе кольцо часто стоит целиком на доме — заявка уезжает на
/// следующий кадр. На шторме входа (20 000 первых выходов одним кадром) таких
/// переспрашиваний ~30%, и прогрев (`loading.rs::poll_warmup`) держит экран
/// загрузки, пока trickle этих заявок не иссякнет, — около десятка кадров.
/// Широкое кольцо ставит промахнувшуюся заявку тем же кадром; бросков потока
/// решений не делает (просеивание — геометрия), поэтому на выбор цели не
/// влияет — только на то, уедет ли она в этот кадр.
const FIRST_WANDER_SIFT_TILES: i32 = 4;

/// Мирное блуждание, один шаг скелета `movement/wander.rs`: отсев по состоянию
/// → пауза 2–10 с на каждом пятом прибытии (см. `roll_wander_pause`) → поток
/// решений → [`choose_target`] (вся политика «куда») → заявка на путь, курс,
/// снятие тегов и перезарядка паузы.
///
/// Каждый из четырёх `continue` — своё основание пропустить пешку до следующего
/// кадра, и ни один из них не двигает состояние вперёд.
#[allow(clippy::too_many_arguments)]
pub fn pick_wander_targets(
    mut commands: Commands,
    time: Res<Time>,
    backend: Res<Backend>,
    map: Res<MapData>,
    seed: Res<WorldSeed>,
    determinism: Option<Res<crate::determinism::Determinism>>,
    mut query: Query<
        (
            Entity,
            &SimPosition,
            &mut Movable,
            &mut WanderPause,
            &mut WanderHeading,
            &PawnId,
            &mut WanderIndex,
            &mut PanicRecoil,
            Has<HumanFirstWanderTag>,
        ),
        (
            With<Human>,
            With<HumanWanderTag>,
            // тег держит ровно `Idle` и `PathfindingError` (`needs_path`):
            // ставят его переходы `Movable`, тело цикла только стережёт этот
            // контракт под debug
            With<NeedsWanderTarget>,
        ),
    >,
) {
    // реплей ведёт свои заявки по тикам: широкий кольец — только живому режиму
    let deterministic = determinism.is_some_and(|mode| mode.0);
    let walkable = backend.walkable();

    for (
        entity,
        sim_position,
        mut movable,
        mut pause,
        mut heading,
        pawn_id,
        mut wander_index,
        mut recoil,
        is_first_wander,
    ) in &mut query
    {
        assert_ready_to_pick(&movable.state);

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
            recoil.0,
            is_first_wander,
        ) else {
            continue;
        };

        // первый выход не переспрашивает: на шторме входа переспрос ~30%
        // первых заявок растягивал прогрев на десяток кадров. Просеивание
        // цели для первого выхода — широким кольцом, живому режиму (см.
        // `sift_target_far`); неудача wide-кольца уезжает на следующий кадр,
        // как и раньше
        let target = if is_first_wander
            && !deterministic
            && walkable
                .sift_target(crate::grid::world_to_tile(target))
                .is_none()
        {
            walkable
                .sift_target_far(crate::grid::world_to_tile(target), FIRST_WANDER_SIFT_TILES)
                .map(crate::grid::tile_center)
                .unwrap_or(target)
        } else {
            target
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
        // запрет живёт ровно до первой удачной цели — гасится записью на
        // месте: компонент постоянный (`#[require]` у `Human`), и снятие
        // командой перегоняло бы человека между архетипами на каждом выборе
        recoil.0 = None;

        // следующая пауза — уже после прибытия
        pause.0.set_duration(roll_wander_pause(rng));
        pause.0.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human::{HUMAN_BODY_RADIUS_MAX, HUMAN_SPEED_SPREAD_MAX};
    use crate::map::osm::fixture::{building, rect};
    use crate::settings::MAP_SIZE;

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

    /// Население в пинах спавна. Дефолт мира стал 200 000 (план ×10,
    /// docs/population-x10-plan.md), а пин проверяет функцию от seed, не
    /// масштаб, — 5 000 держат тест быстрым и во флеше, и в flush.
    const PIN_POPULATION: usize = 5_000;

    /// Строки отпечатка населения: всё, что разыгрывается при спавне, в битах —
    /// на float'ах сравнивать нечего, нужна побайтовая одинаковость.
    fn spawn_fingerprint(world: &mut World) -> Vec<(u32, u32, u32, u32, u32, u32)> {
        let mut query = world.query::<(&PawnId, &Transform, &Pace, &WanderHeading)>();
        let mut rows: Vec<_> = query
            .iter(world)
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

    /// Отпечаток населения, расселённого одним флешем (`spawn_population`).
    fn population_fingerprint(seed: u64, count: usize) -> Vec<(u32, u32, u32, u32, u32, u32)> {
        let mut world = World::new();
        let navmesh = crate::navigation::Navmesh::default();
        spawn_population(
            &mut world.commands(),
            &navmesh,
            &PopulationBirth {
                spread: 0.3,
                world_seed: seed,
                count,
                silhouettes: &Silhouettes::default(),
            },
        );
        world.flush();
        spawn_fingerprint(&mut world)
    }

    /// Отпечаток населения, расселённого насосом (`pump_batch`) партиями по
    /// `batch` — до исчерпания очереди.
    fn pumped_population_fingerprint(
        seed: u64,
        count: usize,
        batch: usize,
    ) -> Vec<(u32, u32, u32, u32, u32, u32)> {
        let mut world = World::new();
        let navmesh = crate::navigation::Navmesh::default();
        let mut pending = PendingPopulation::new(count);
        while !pump_batch(
            &mut world.commands(),
            &navmesh,
            0.3,
            seed,
            &Silhouettes::default(),
            &mut pending,
            batch,
        ) {}
        world.flush();
        spawn_fingerprint(&mut world)
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
        assert_eq!(
            population_fingerprint(7, PIN_POPULATION),
            population_fingerprint(7, PIN_POPULATION)
        );
    }

    /// ...и при этом seed что-то значит: разные seed дают разное население.
    #[test]
    fn different_seeds_give_different_populations() {
        assert_ne!(
            population_fingerprint(7, PIN_POPULATION),
            population_fingerprint(8, PIN_POPULATION)
        );
    }

    /// Пин Фазы 1 плана ×10 («партиями == не партиями», бит-в-бит): насос
    /// режет `0..count` на партии, но личный жребий выводится из номера пешки,
    /// а позиции считаются разом до всякого разрезания — батчингу нечем
    /// сместить хоть один бросок. Одна партия (batch = count), десять (50) и
    /// немедленный спавн обязаны дать один и тот же отпечаток.
    #[test]
    fn batching_does_not_change_the_population() {
        let whole = population_fingerprint(7, 500);
        let one_batch = pumped_population_fingerprint(7, 500, 500);
        let ten_batches = pumped_population_fingerprint(7, 500, 50);
        assert_eq!(whole, one_batch, "партия «всё сразу» != немедленный спавн");
        assert_eq!(whole, ten_batches, "десять партий != немедленный спавн");
    }

    /// Пустой навмеш не спавнит никого: приложение остаётся живым с
    /// единственной строкой в логе.
    #[test]
    fn population_refuses_a_navmesh_without_passable_tiles() {
        let mut world = World::new();
        let navmesh = navmesh_blocked_except(vec![]);
        spawn_population(
            &mut world.commands(),
            &navmesh,
            &PopulationBirth {
                spread: 0.3,
                world_seed: 7,
                count: 4,
                silhouettes: &Silhouettes::default(),
            },
        );
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
        let hole_tile_size = crate::grid::navtile_size();
        let hole_rect = rect(hole_center, hole_center + hole_tile_size);
        let navmesh = navmesh_blocked_except(vec![hole_rect]);
        spawn_population(
            &mut world.commands(),
            &navmesh,
            &PopulationBirth {
                spread: 0.3,
                world_seed: 7,
                count: 4,
                silhouettes: &Silhouettes::default(),
            },
        );
        world.flush();

        let count = world.query::<&Human>().iter(&world).count();
        assert_eq!(count, 4, "все люди должны быть спавнены даже со сканом");

        let hole_center_world = crate::grid::tile_center(hole_tile);
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
        building(vec![corner], vec![])
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
