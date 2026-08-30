mod behavior;
mod components;
mod decide;
mod systems;

use bevy::prelude::*;

use self::behavior::{escape, flee, panic};
pub use self::components::{
    CorpseTag, FleeRepath, Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag,
    Pace, PanicRecoil, PopulationSize, WanderHeading, WanderPause, to_corpse,
};
// `pick_wander_targets` наружу — им пользуется демо-сцена расталкивания
// (`examples/demos/crowd_demo.rs`), чтобы гонять толпу настоящим блужданием, а
// не своей выдумкой; `HumanPlugin` целиком ей не подходит (его `spawn_humans`
// расселил бы 20 000 пешек по всей карте)
pub use self::systems::{pick_wander_targets, spawn_population};
use self::systems::{spawn_humans, spread_changed, sync_human_pace};
use crate::determinism::{DeterminismPlugin, SimPipeline};
use crate::loading::{AppState, WorldInitSet};
use crate::prefs::TrackPrefExt;
use crate::spatial::SimSet;

pub struct HumanPlugin;

impl Plugin for HumanPlugin {
    fn build(&self, app: &mut App) {
        // ветки конвейера гейтит `DeterminismPlugin` — без него обе работали
        // бы разом; тот же довод, что и в `MovementPlugin`
        if !app.is_plugin_added::<DeterminismPlugin>() {
            app.add_plugins(DeterminismPlugin);
        }

        app.register_type::<Human>()
            .register_type::<HumanWanderTag>()
            .register_type::<HumanFirstWanderTag>()
            .register_type::<HumanFleeTag>()
            .register_type::<CorpseTag>()
            .register_type::<FleeRepath>()
            .register_type::<WanderPause>()
            .register_type::<WanderHeading>()
            .register_type::<PanicRecoil>()
            .register_type::<Pace>()
            .register_type::<HumanStyle>()
            .init_resource::<HumanStyle>()
            .track_pref::<HumanStyle>()
            .init_resource::<PopulationSize>()
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_humans.in_set(WorldInitSet::Spawn),
            )
            // Выбор целей и разброс скоростей зарегистрированы дважды, и это
            // осознанно: одна и та же работа в двух ветках конвейера
            // (`SimPipeline`), а какая из них поедет — решает не эта пара
            // строк, а гейт в `DeterminismPlugin`.
            //
            // В детерминированном режиме обе обязаны идти по тикам: в `Update`
            // и число прогонов за секунду, и дельта, которой заводится
            // `WanderPause`, зависят от fps, а правка ползунка разброса легла
            // бы между разными тиками при разном fps.
            //
            // В обычном режиме обе остаются в `Update`: там на выборе целей
            // держится прогрев (`loading.rs::poll_warmup` ждёт заявок, а
            // `FixedUpdate` на паузе прогрева не идёт вовсе) и покадровый ритм,
            // к которому подогнаны гейт видимости диспетчера и лимит заявок, а
            // ползунок обязан отзываться и на паузе симуляции.
            .add_systems(
                FixedUpdate,
                (
                    pick_wander_targets.in_set(SimPipeline::Deterministic),
                    // ползунок разброса — на тике, а не в кадре: скорости
                    // пешек входят в состояние прогона, и правка, применённая
                    // в кадре, легла бы между разными тиками при разном fps
                    sync_human_pace
                        .run_if(spread_changed)
                        .in_set(SimPipeline::Deterministic),
                    // ветка объявлена явно: `flee` берёт `Res<Backend>`, а
                    // гейт на мир приезжает только с множеством конвейера.
                    // `SimSet::HumanBehavior` держит порядок (люди после
                    // демонов) и настраивается чужим плагином — на него этот
                    // гейт не переложишь: без `SpatialPlugin` множество не
                    // гейтит ничего
                    (panic, flee, escape).chain().in_set(SimPipeline::BothModes),
                )
                    .chain()
                    .in_set(SimSet::HumanBehavior),
            )
            .add_systems(
                Update,
                (
                    pick_wander_targets,
                    // ползунок разброса уже вышедшим людям — только на смену
                    // самого разброса: `HumanStyle` несёт ещё и радиус тела,
                    // и по `resource_changed` его ползунок гонял бы проход по
                    // всей популяции на каждый шаг протяжки
                    sync_human_pace.run_if(spread_changed),
                )
                    // тот же `.chain()`, что и в ветке `FixedUpdate`: обе
                    // системы берут `&mut Movable` на пересекающихся людях
                    // (`With<Human>` — надмножество фильтра выбора целей), и
                    // без цепочки их взаимный порядок в кадре не определён.
                    // Сегодня поля дизъюнктны — цели пишут `state`/`path`,
                    // разброс `speed`, — но это свойство тел систем, а не
                    // расписания, и держаться конвейеру на нём нечем
                    .chain()
                    // гейт на мир приезжает с множеством: `pick_wander_targets`
                    // берёт `Res<Backend>`, которого вне `Playing` нет
                    .in_set(SimPipeline::Live),
            );
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::schedule::ScheduleLabel;
    use bevy::prelude::*;
    use bevy::state::app::StatesPlugin;

    use super::*;

    /// Пары систем, чьи доступы пересекаются, а взаимный порядок расписание не
    /// задаёт. Считает их сама постройка графа, поэтому расписание достаточно
    /// **проинициализировать** — гонять системы не нужно (а и нечем: они просят
    /// ресурсы мира). Расписание при этом уезжает из `Schedules` насовсем: на
    /// каждую метку вызов один, приложение после проверки не запускается.
    fn unordered_conflicts(app: &mut App, label: impl ScheduleLabel) -> Vec<String> {
        let world = app.world_mut();
        let mut schedule = world
            .resource_mut::<Schedules>()
            .remove(label)
            .expect("расписание заведено плагином");
        schedule.initialize(world).expect("расписание строится");
        schedule
            .graph()
            .conflicting_systems()
            .to_string(schedule.graph(), world.components())
            .map(|(a, b, on)| format!("{a} ↔ {b} (по {on:?})"))
            .collect()
    }

    /// Обе ветки конвейера упорядочены изнутри.
    #[test]
    fn neither_branch_leaves_the_wander_pair_unordered() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, HumanPlugin));

        assert!(
            unordered_conflicts(&mut app, Update).is_empty(),
            "Update branch: pick_wander_targets and sync_human_pace must be ordered"
        );
        assert!(
            unordered_conflicts(&mut app, FixedUpdate).is_empty(),
            "FixedUpdate branch: pick_wander_targets and sync_human_pace must be ordered"
        );
    }

    /// Сколько систем расписания состоит в множестве. Страховка от зелёного
    /// впустую: «ничего не упало» стоит чего-то лишь тогда, когда падать было
    /// чему. Требует уже построенного расписания — отсюда порядок вызовов.
    fn systems_in_set(app: &mut App, label: impl ScheduleLabel, set: impl SystemSet) -> usize {
        let world = app.world_mut();
        let mut schedule = world
            .resource_mut::<Schedules>()
            .remove(label)
            .expect("расписание заведено плагином");
        schedule.initialize(world).expect("расписание строится");
        schedule
            .graph()
            .systems_in_set(set.intern())
            .expect("множество объявлено этим расписанием")
            .len()
    }

    /// Инвариант `CONTEXT.md` («Backend / Walkable»): система, берущая
    /// `Res<Backend>`, состоит в множестве `SimPipeline` — только оно приносит
    /// гейт на мир (`in_world`). Цепочке `panic`/`flee`/`escape` его давал
    /// чужой `SimSet::HumanBehavior`, а тот настраивает `SpatialPlugin`:
    /// без этого плагина множество не гейтит **ничего**.
    ///
    /// Двор поэтому и собран без него. Мир не поднимался, `AppState` остался
    /// `Loading`, ресурсов мира нет вовсе — и система, которую не удержал гейт,
    /// валится на валидации параметров. Наблюдаемое — что расписание прошло.
    #[test]
    fn human_behavior_does_not_run_outside_the_world() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, HumanPlugin))
            .init_state::<AppState>();

        // расписание крутится прямое: находка про `FixedUpdate`, а полный кадр
        // притащил бы сюда сохранение настроек и рестарт по смене seed'а
        app.world_mut().run_schedule(FixedUpdate);

        assert_eq!(
            systems_in_set(&mut app, FixedUpdate, SimPipeline::BothModes),
            3,
            "цепочка panic/flee/escape обязана состоять в SimPipeline::BothModes"
        );
    }
}
