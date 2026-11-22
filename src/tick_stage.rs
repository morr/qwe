//! Порядок тика одним перечнем — [`TickStage`].
//!
//! Симуляция идёт в `FixedUpdate` стадиями, и порядок стадий — часть контракта
//! воспроизведения: ответы о пути применяются до поведения, спавн демонов — до
//! поведения демонов (и до того, как они станут видны в сетке), диспетчер — в
//! самом конце тика. Раньше этот порядок держали рёбра `.before`/`.after`
//! между плагинами и цепочка `SimSet` в `spatial.rs`, а граничные пробы
//! разреза тика (`sim_time.rs`) копировали его руками. Теперь перечень стадий
//! один ([`TickStage::ORDER`]), и цепочка сетов, пробы и тест расписания
//! выводятся из него.

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::*;

/// Стадии одного тика симуляции в `FixedUpdate`, в порядке исполнения.
///
/// Стадии — только порядок, не гейт: ни одна не несёт `run_if`. Система,
/// живущая внутри мира, объявляет свою ветку [`SimPipeline`](crate::determinism::SimPipeline)
/// отдельно — гейт на мир приезжает с ней.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TickStage {
    /// Предохранитель бюджета кадра (`sim_time.rs`): решение «оборвать прогон»
    /// принимается до того, как тик начнёт тратить время.
    Guard,
    /// Счётчик тиков ([`SimTick`](crate::determinism::SimTick)): всё, что
    /// решает этот шаг, ссылается на один и тот же номер.
    Tick,
    /// Приём ответов поиска пути: путь, приземлившийся на этом тике, на нём
    /// же и идётся.
    Answers,
    /// Спавнер демонов: родившийся на тике демон входит в сетку и действует
    /// на том же тике. Точку синхронизации, на которой применяются команды
    /// спавна (и observer сетки кладёт демона в ячейку), держит ребро к
    /// [`TickStage::DemonBehavior`].
    Spawner,
    /// Щель порядка между рождением демона и его поведением. С фазы 6 плана
    /// ×10 сетка демонов инкрементальна (`spatial.rs`), пересобирать здесь
    /// нечего, — стадия осталась, чтобы шов «спавн-команды применены до
    /// поведения» не уехал из перечня и проб разреза тика.
    DemonGrid,
    /// Поведение демонов: выбор целей, погоня, бросок, пожирание.
    DemonBehavior,
    /// Поведение людей: паника, бегство, `escape`. Демоны раньше людей, чтобы
    /// убийство применилось до `escape` и человек не был засчитан и убитым,
    /// и спасшимся в один тик.
    HumanBehavior,
    /// Шаг движения по путям.
    Move,
    /// Расталкивание — строго после шага движения: только там позиции тика
    /// финальны, а толчок доедет до экрана интерполяцией.
    Separation,
    /// Заявки на путь и диспетчер — в самом конце тика: заявки, поданные
    /// поведением этого шага, уезжают в тот же тик.
    Dispatch,
}

impl TickStage {
    /// Порядок стадий — единственный источник истины. Цепочка сетов
    /// ([`TickStagePlugin`]), граничные пробы разреза тика (`sim_time.rs`) и
    /// тест расписания выводятся из этого перечня.
    pub const ORDER: &'static [TickStage] = &[
        TickStage::Guard,
        TickStage::Tick,
        TickStage::Answers,
        TickStage::Spawner,
        TickStage::DemonGrid,
        TickStage::DemonBehavior,
        TickStage::HumanBehavior,
        TickStage::Move,
        TickStage::Separation,
        TickStage::Dispatch,
    ];

    /// Имя стадии в разрезе спайка (`TickSegments`, `sim_time.rs`).
    pub const fn probe_name(self) -> &'static str {
        match self {
            TickStage::Guard => "guard",
            TickStage::Tick => "tick",
            TickStage::Answers => "answers",
            TickStage::Spawner => "spawner",
            TickStage::DemonGrid => "demon_grid",
            TickStage::DemonBehavior => "demon_behavior",
            TickStage::HumanBehavior => "human_behavior",
            TickStage::Move => "move",
            TickStage::Separation => "separation",
            TickStage::Dispatch => "dispatch",
        }
    }
}

/// Цепочка стадий в `FixedUpdate` — рёбра между соседними по перечню.
///
/// Поднимает `SpatialPlugin`: сборки, которым нужен порядок тика, поднимают
/// и его (`main.rs`, `replay_app`, `crowd_demo`, тесты движения). Стенды без
/// `SpatialPlugin` (лестницы поведения в тестах) цепочку не настраивают — им
/// порядок между плагинами не нужен, свои цепочки они держат сами.
///
/// Конфиг сетов в Bevy 0.20 берёт кортежи до 20, но не `Vec`; кортеж рядом с
/// перечнем держал бы порядок дважды — та же теневая копия, которую этот
/// модуль убирает. Попарные рёбра дают тот же транзитивный порядок, что
/// `.chain()` на кортеже.
pub struct TickStagePlugin;

impl Plugin for TickStagePlugin {
    fn build(&self, app: &mut App) {
        for pair in TickStage::ORDER.windows(2) {
            app.configure_sets(FixedUpdate, pair[1].after(pair[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::schedule::{NodeId, Schedules};
    use bevy::prelude::*;

    use super::*;

    /// Заглушка ради членства: каждая стадия должна нести систему, иначе пустые
    /// сеты в графе расписания не о чём проверять.
    fn noop() {}

    /// Расписание исполняет стадии в порядке перечня: между соседними стадиями
    /// стоит ребро, поставленное [`TickStagePlugin`]. Ручное ребро из чужого
    /// плагина порядок не заменяет — перечень и расписание обязаны совпадать.
    #[test]
    fn the_schedule_chains_the_stages_in_the_listed_order() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(TickStagePlugin);
        for stage in TickStage::ORDER {
            app.add_systems(FixedUpdate, noop.in_set(*stage));
        }

        let world = app.world_mut();
        let mut schedule = world
            .resource_mut::<Schedules>()
            .remove(FixedUpdate)
            .expect("расписание заведено плагином");
        schedule.initialize(world).expect("расписание строится");

        let graph = schedule.graph();
        let keys: Vec<_> = TickStage::ORDER
            .iter()
            .map(|stage| {
                graph
                    .system_sets
                    .get_key(stage.intern())
                    .expect("стадия заведена в расписании")
            })
            .collect();
        for (index, pair) in keys.windows(2).enumerate() {
            let (prev, next) = (NodeId::Set(pair[0]), NodeId::Set(pair[1]));
            assert!(
                graph.dependency().graph().contains_edge(prev, next),
                "стадия {:?} обязана идти сразу после {:?}: перечень и расписание разошлись",
                TickStage::ORDER[index + 1],
                TickStage::ORDER[index],
            );
        }
    }
}
