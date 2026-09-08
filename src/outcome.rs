//! Исход прогона: сердце осквернено — победа; демонов нет и не на что
//! призвать — поражение (тупик). Состояние прогона: сброс на `WorldStarted`,
//! в отпечатке. На переходе мир встаёт на паузу; плашка — `ui/outcome.rs`.
//! Модель — скилл `city-siege`.

use bevy::prelude::*;

use crate::corruption::{Corruption, spread_corruption};
use crate::demon::{Demon, DemonKind};
use crate::determinism::{SimPipeline, SimTick};
use crate::district::Districts;
use crate::loading::{PlayPhase, WorldStarted};
use crate::souls::{Souls, summon_cost};
use crate::spatial::SimSet;

/// Чем кончился прогон. Тик — `SimTick`: исходы сравниваются по тику, не по
/// часам (`determinism`).
#[derive(Resource, Reflect, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[reflect(Resource)]
pub enum Outcome {
    #[default]
    Running,
    Won {
        tick: u64,
    },
    Lost {
        tick: u64,
        reason: LossReason,
    },
}

impl Outcome {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Почему проиграно. В M1 один тупик: живых демонов нет и душ меньше, чем
/// стоит самый дешёвый — призвать некого и не на что.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LossReason {
    Stalemate,
}

/// Чистое правило исхода на тике.
pub fn judge(
    heart_corrupted: bool,
    demons_alive: usize,
    souls_available: u32,
    tick: u64,
) -> Outcome {
    if heart_corrupted {
        Outcome::Won { tick }
    } else if demons_alive == 0 && souls_available < summon_cost(DemonKind::Imp, 0) {
        Outcome::Lost {
            tick,
            reason: LossReason::Stalemate,
        }
    } else {
        Outcome::Running
    }
}

/// Исход на тике — после шага скверны того же тика, чтобы победа была
/// объявлена на тике осквернения сердца, а не следующим. На переходе мир
/// встаёт на паузу: дальше смотреть не на что, а R умеет всё нужное
/// (решение 6 `ROADMAP.md`).
fn judge_outcome(
    tick: Res<SimTick>,
    districts: Res<Districts>,
    corruption: Res<Corruption>,
    souls: Res<Souls>,
    demons: Query<(), With<Demon>>,
    mut outcome: ResMut<Outcome>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !outcome.is_running() {
        return;
    }
    let heart_corrupted = districts
        .heart
        .is_some_and(|heart| corruption.is_corrupted(heart));
    let verdict = judge(
        heart_corrupted,
        demons.iter().len(),
        souls.available(),
        tick.0,
    );
    if verdict.is_running() {
        return;
    }
    *outcome = verdict;
    info!("outcome: {verdict:?}");
    time.pause();
}

/// Новый прогон — исход снова открыт. Пауза снимается только если она была
/// исходом: `sim_time::on_world_started` выставляет скорость, но паузы не
/// трогает, а рестарт по R минует `Warmup`, где `resume_world` снимает её при
/// первом входе. Паузу игрока пробелом (исход `Running`) — не трогать.
fn on_world_started(
    _event: On<WorldStarted>,
    mut outcome: ResMut<Outcome>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !outcome.is_running() {
        time.unpause();
    }
    *outcome = Outcome::Running;
}

pub struct OutcomePlugin;

impl Plugin for OutcomePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Outcome>()
            .init_resource::<Outcome>()
            .add_observer(on_world_started)
            .add_systems(
                FixedUpdate,
                judge_outcome
                    .after(spread_corruption)
                    .in_set(SimSet::Territory)
                    .in_set(SimPipeline::BothModes)
                    .run_if(in_state(PlayPhase::Live)),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heart_wins_and_an_empty_purse_without_demons_loses() {
        assert_eq!(judge(true, 0, 0, 7), Outcome::Won { tick: 7 });
        assert_eq!(
            judge(false, 0, 0, 7),
            Outcome::Lost {
                tick: 7,
                reason: LossReason::Stalemate
            }
        );
        // демоны есть — прогон идёт, сколько бы душ ни было
        assert_eq!(judge(false, 1, 0, 7), Outcome::Running);
        // демонов нет, но на Беса хватает — ещё не тупик
        assert_eq!(
            judge(false, 0, summon_cost(DemonKind::Imp, 0), 7),
            Outcome::Running
        );
        // осквернённое сердце важнее пустого кошелька
        assert_eq!(judge(true, 0, 0, 7), Outcome::Won { tick: 7 });
    }

    /// Пауза исхода снимается новым прогоном; пауза игрока — нет.
    #[test]
    fn a_new_run_lifts_only_the_outcomes_pause() {
        let mut app = App::new();
        app.init_resource::<Time<Virtual>>();
        app.add_plugins(OutcomePlugin);

        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        *app.world_mut().resource_mut::<Outcome>() = Outcome::Won { tick: 1 };
        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(*app.world().resource::<Outcome>(), Outcome::Running);

        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.world_mut().trigger(WorldStarted);
        app.world_mut().flush();
        assert!(
            app.world().resource::<Time<Virtual>>().is_paused(),
            "a player's pause survives a restart"
        );
    }
}
