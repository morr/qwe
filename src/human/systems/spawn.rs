use bevy::prelude::*;
use rand::RngExt;

use crate::human::components::{
    Human, HumanFirstWanderTag, HumanStyle, HumanWanderTag, Pace, PopulationSize, WanderHeading,
    WanderPause,
};
use crate::human::look::{human_body, roll_attire};
use crate::loading::AppState;
use crate::map::z::unit_z;
use crate::movement::Movable;
use crate::navigation::ArcNavmesh;
use crate::rng::{PawnId, RngDomain, Species, WanderIndex, WorldSeed, decision_stream, stream};
use crate::settings::HUMAN_WALK_SPEED;
use crate::silhouette::Silhouettes;

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

pub fn spawn_humans(mut commands: Commands, population: PopulationSpawn) {
    population.spawn(&mut commands);
}

/// Партионный спавн: сколько пешек насос расселяет за один вызов.
///
/// Планка готовности Фазы 1 (`docs/population-x10-plan.md`) — расселение 200k
/// без рамки длиннее ~50 мс. Живые замеры звена №46 (Париж, 200k): партия
/// 10 000 — рамки ~100–130 мс, партия 4 000 — ~50–64 мс, то есть почти вся
/// цена рамки масштабируется от партии (~7–8 мкс на пешку: выбор цели свежей
/// партии, проводка трансформов, загрузка спрайтов в GPU), и только ~20 мс —
/// постоянные расходы кадра прогрева. Партия 2 000 — расчётные ~30–40 мс,
/// с запасом под планку; мельче — расселение тянется лишние кадры прогрева,
/// которому вдобавок маршрутизировать прилетающих.
pub const SPAWN_BATCH: usize = 2_000;

/// Недосселённое население — очередь насоса ([`PopulationSpawn::pump`]).
///
/// Вставляется [`PopulationSpawn::spawn`] вместо немедленного спавна; насос
/// снимает ресурс, когда очередь исчерпана. Рестарт и переключение города
/// идут через тот же заказ: деспаун сцены случается раньше, остаток очереди
/// перезаписывается целиком.
///
/// BRP-читаемая: `res get PendingPopulation` показывает `count`/`cursor`
/// расселения; `placements` (позиции всего населения) наружу не отдаётся.
#[derive(Resource, Reflect, Debug)]
#[reflect(Resource)]
pub struct PendingPopulation {
    count: usize,
    cursor: usize,
    /// Центры тайлов всех пешек. Считается на первом вызове насоса: размещение
    /// (rejection-sampling) на порядок дешевле флеша сущностей, а единый поток
    /// размещения жил и умирал внутри одного вызова — хранить генератор в
    /// ресурсе между вызовами контракт детерминизма запрещает, поэтому позиции
    /// всего населения готовы заранее и партия лишь спавнит по готовым.
    #[reflect(ignore)]
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
pub(super) fn pump_batch(
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
