//! Перепись населения — счётчики состояний мира, посчитанные **один раз за
//! кадр**.
//!
//! Потребители читают готовый [`Census`], а не считают своим запросом: раньше
//! HUD (`ui/stats.rs`) ходил двумя запросами каждый кадр, а репортёр
//! тик-спайков (`sim_time`) держал семь видовых `Option<Query>` ради одной
//! строки лога. Считать за кадр, а не за тик — принципиально: тиков в кадре
//! до `64 × скорость`, и перепись двадцати тысяч сущностей на каждом там не
//! место. Сам снимок дешевле, чем кажется: при чисто архетипном фильтре
//! (`With<_>`) `iter().len()` — `ExactSizeIterator`, то есть сумма размеров
//! архетипов, без прохода по сущностям.
//!
//! Репортёр спайков живёт в `AfterFixedMainLoop` — раньше `Update` этого же
//! кадра — и читает снимок прошлого; для контекстной строки лога достаточно.
//! HUD стоит после переписи в том же `Update` и видит текущий кадр.
//!
//! Исходы (`killed`/`escaped`) здесь не живут: они накопительные и растут
//! инкрементально по событиям — `telemetry::Telemetry`. Инвариант переписи
//! «живые + убитые + спасшиеся == население» проверяется раз в кадр
//! ([`assert_population_invariant`]) в `PlayPhase::Live` — единственной фазе,
//! где телеметрия заведомо описывает текущий прогон: на прогреве после
//! рестарта в ней ещё счёт предыдущего.

use bevy::prelude::*;

use crate::demon::{Demon, DemonChaseTag, HordeSchedule};
use crate::human::{
    CorpseTag, Human, HumanFleeTag, HumanWanderTag, PendingPopulation, PopulationSize, SoulMote,
};
use crate::loading::PlayPhase;
use crate::movement::MovableStateMovingTag;
use crate::telemetry::Telemetry;

/// Состояния мира на снимке этого кадра. Все — производные: ресурс
/// пересчитывается с нуля каждый кадр, и сброс при рестарте ему не нужен.
///
/// BRP-читаемый: `res get Census` отвечает тем же, что раньше приходилось
/// собирать из `count Human` и `res get Telemetry`.
#[derive(Resource, Reflect, Default, Debug, Clone, Copy)]
#[reflect(Resource)]
pub struct Census {
    /// Живые люди: `With<Human>` — трупу компонент снимает `to_corpse`.
    pub humans: usize,
    pub demons: usize,
    pub corpses: usize,
    /// Души в полёте — рождаются на смерти и самоликвидируются через пару
    /// секунд.
    pub souls: usize,
    pub fleeing: usize,
    pub wandering: usize,
    /// Пешки в `MovableState::Moving` — по дешёвому тегу состояния, а не по
    /// разбору самой `MovableState`.
    pub moving: usize,
    pub chasing: usize,
    /// Волн орды вышло с начала прогона (`HordeSchedule::wave_index`) — номер
    /// идущей волны в строке спайк-отчёта и `res get Census`.
    pub wave: u64,
}

pub struct CensusPlugin;

impl Plugin for CensusPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Census>()
            .init_resource::<Census>()
            .add_systems(
                Update,
                (
                    count_census,
                    assert_population_invariant.run_if(in_state(PlayPhase::Live)),
                )
                    .chain(),
            );
    }
}

/// Перепись этого кадра. Порядок с потребителями держат они сами: HUD встаёт
/// `.after` этой системы (`ui/stats.rs`), репортёр спайков живёт в другом
/// расписании и читает прошлый кадр.
#[allow(clippy::too_many_arguments)]
pub(crate) fn count_census(
    humans: Query<(), With<Human>>,
    demons: Query<(), With<Demon>>,
    corpses: Query<(), With<CorpseTag>>,
    souls: Query<(), With<SoulMote>>,
    fleeing: Query<(), With<HumanFleeTag>>,
    wandering: Query<(), With<HumanWanderTag>>,
    moving: Query<(), With<MovableStateMovingTag>>,
    chasing: Query<(), With<DemonChaseTag>>,
    horde: Res<HordeSchedule>,
    mut census: ResMut<Census>,
) {
    census.humans = humans.iter().len();
    census.demons = demons.iter().len();
    census.corpses = corpses.iter().len();
    census.souls = souls.iter().len();
    census.fleeing = fleeing.iter().len();
    census.wandering = wandering.iter().len();
    census.moving = moving.iter().len();
    census.chasing = chasing.iter().len();
    census.wave = horde.wave_index;
}

/// Инвариант переписи: живые + убитые + спасшиеся — всё население прогона.
/// Debug-проверка: в release ничего не стоит, в debug ловит рассинхрон
/// переписи с телеметрией — человек, который ни жив, ни убит, ни спасся,
/// означает дыру в учёте исходов.
///
/// Пока стоит очередь насоса расселения ([`PendingPopulation`] на месте),
/// проверка молчит. Часы несовместимы: курсор насоса двигается `ResMut`-ом
/// немедленно, а спавны его партии доезжают до мира на ближайшей точке
/// синхронизации — перепись, вставшая между насосом и его flush, видит
/// «живых на партию меньше, не расселено на партию больше», и сумма расходится
/// ровно на партию (звено №46: рестарт на 200k уронил приложение на этом
/// ассерте; первый вход не ловил его только потому, что в `Warmup` проверка
/// и так стоит). Ресурс снимается той же командной пачкой, что доносит
/// последнюю партию, — «ресурса нет» значит и очередь исчерпана, и мир уже
/// видит всё население. Накопительные ошибки расселения (задвоенная партия,
/// потерянная пешка) инвариант всё равно ловит: сумма живых и исходов после
/// дренажа обязана сойтись к населению.
fn assert_population_invariant(
    census: Res<Census>,
    telemetry: Res<Telemetry>,
    population: Res<PopulationSize>,
    pending: Option<Res<PendingPopulation>>,
) {
    if pending.is_some() {
        return;
    }
    debug_assert_eq!(
        census.humans + telemetry.killed + telemetry.escaped,
        population.0,
        "перепись разошлась с телеметрией: живых {}, убитых {}, спасшихся {}, \
         население {}",
        census.humans,
        telemetry.killed,
        telemetry.escaped,
        population.0
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loading::AppState;
    use crate::telemetry::TelemetryPlugin;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::state::app::StatesPlugin,
            crate::loading::SimBootPlugin,
            TelemetryPlugin,
            CensusPlugin,
        ));
        // снимок орды читает расписание демонов; в игре его заводит
        // `DemonPlugin`, стенду переписи хватает нулевого
        app.insert_resource(crate::demon::HordeSchedule::default());
        app.insert_resource(PopulationSize(5));
        app
    }

    /// Мир в `Playing`, но ещё в прогреве: перепись считается, проверка
    /// инварианта стоит.
    fn warmed_up(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();
    }

    /// Вход в живой прогон и/или один кадр после него. `set_if_different`, а
    /// не `set`: повторное `set` того же значения в 0.20 переигрывает
    /// `OnEnter(Live)`, и `announce_world_start` обнулял бы телеметрию на
    /// каждом шаге.
    fn step(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set_if_different(PlayPhase::Live);
        app.world_mut().flush();
        app.update();
    }

    fn census_of(app: &mut App) -> Census {
        *app.world().resource::<Census>()
    }

    #[test]
    fn census_counts_states_and_holds_the_invariant_through_outcomes() {
        let mut app = app();
        warmed_up(&mut app);

        // население расселено в прогреве — как в игре: `spawn_population`
        // стоит на входе в `Playing`, задолго до `Live`
        let mut commands = app.world_mut().commands();
        let crowd: Vec<Entity> = (0..5).map(|_| commands.spawn(Human).id()).collect();
        commands.entity(crowd[0]).insert(HumanFleeTag);
        commands.entity(crowd[1]).insert(MovableStateMovingTag);
        commands.spawn((Demon, DemonChaseTag));
        commands.spawn(SoulMote {
            age: Timer::from_seconds(1.5, TimerMode::Once),
        });
        step(&mut app);

        let census = census_of(&mut app);
        assert_eq!(census.humans, 5);
        assert_eq!(census.fleeing, 1);
        assert_eq!(census.moving, 1);
        assert_eq!(census.demons, 1);
        assert_eq!(census.chasing, 1);
        assert_eq!(census.souls, 1);
        assert_eq!(census.corpses, 0);

        // убитый: человек становится трупом — как это делает `to_corpse`,
        // минуя рисование
        app.world_mut()
            .commands()
            .entity(crowd[2])
            .remove::<Human>()
            .insert(CorpseTag);
        app.world_mut().resource_mut::<Telemetry>().killed += 1;
        step(&mut app);

        let census = census_of(&mut app);
        assert_eq!(census.humans, 4);
        assert_eq!(census.corpses, 1);
        // 4 + 1 + 0 == 5 — проверка инварианта отработала и не сработала

        // спасшийся: десполна и счёт исхода — как в `escape`
        app.world_mut().commands().entity(crowd[3]).despawn();
        app.world_mut().resource_mut::<Telemetry>().escaped += 1;
        step(&mut app);

        let census = census_of(&mut app);
        assert_eq!(census.humans, 3);
        // 3 + 1 + 1 == 5 — инвариант жив
    }

    #[test]
    fn the_invariant_is_checked_only_in_live() {
        let mut app = app();
        // телеметрия прошлого прогона: после рестарта, на прогреве, счёт ещё
        // не сброшен (`WorldStarted` звенит только на входе в `Live`)
        app.world_mut().resource_mut::<Telemetry>().killed = 3;
        warmed_up(&mut app);

        let mut commands = app.world_mut().commands();
        for _ in 0..5 {
            commands.spawn(Human);
        }
        app.world_mut().flush();
        app.update();

        // 5 живых + 3 убитых != 5 — в `Warmup` проверка молчит; будь она
        // без фазы, debug-сборка упала бы на этом обновлении
        assert_eq!(census_of(&mut app).humans, 5);
    }

    /// Пока стоит очередь насоса расселения, проверка молчит: курсор насоса
    /// уже двигается, а его партия до мира ещё не доехала (звено №46 — на
    /// рестарте 200k приложение падало на этом расхождении часов). Ресурс
    /// снялся — проверка вернулась и снова ловит реальное расхождение.
    #[test]
    fn the_invariant_stands_down_while_the_spawn_queue_lasts() {
        let mut app = app();
        warmed_up(&mut app);

        // 3 живых при населении 5: пока очередь на месте, проверка стоит —
        // будь иначе, debug-ассерт упал бы на 3 != 5 прямо здесь
        app.world_mut().insert_resource(PendingPopulation::new(5));
        let mut commands = app.world_mut().commands();
        for _ in 0..3 {
            commands.spawn(Human);
        }
        app.world_mut().flush();
        step(&mut app);
        assert_eq!(census_of(&mut app).humans, 3);

        // очередь снялась — проверка вернулась: 3 живых при населении 5
        // обязаны упасть
        app.world_mut().remove_resource::<PendingPopulation>();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(&mut app))).is_err(),
            "инвариант обязан вернуться после снятия очереди: 3 живых != население 5"
        );
    }
}
