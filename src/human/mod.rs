mod behavior;
mod components;
mod corpse;
mod decide;
mod lod;
mod look;
mod soul;
mod systems;

use bevy::prelude::*;

use self::behavior::{escape, flee, panic};
pub use self::components::{
    Attire, CorpseTag, FleeRepath, HUMAN_BODY_RADIUS, HUMAN_BODY_RADIUS_MAX, HUMAN_BODY_RADIUS_MIN,
    HUMAN_BODY_RADIUS_STEP, HUMAN_SPEED_SPREAD, HUMAN_SPEED_SPREAD_MAX, HUMAN_SPEED_SPREAD_MIN,
    HUMAN_SPEED_SPREAD_STEP, Human, HumanFirstWanderTag, HumanFleeTag, HumanStyle, HumanWanderTag,
    Pace, PanicRecoil, PopulationSize, WanderHeading, WanderPause,
};
// `blood_pool` / `blood_spatter` / `spread_blood` наружу — их берёт витрина
// крови (`examples/demos/blood_gallery`), чтобы показывать те же сущности, что
// игра вешает на труп, а не свою копию; `HumanPlugin` целиком ей не подходит
// (его `spawn_humans` заказал бы ей расселение всего населения)
pub use self::corpse::{
    BakeQueue, BloodLook, BloodPool, BloodSpatter, BloodSpread, CORPSE_SPAN, CorpsePose,
    blood_look, blood_pool, blood_spatter, corpse_pose, spread_blood, to_corpse,
};
// `corpse_quads` / `reset_bake_queue` — внутри: их зовёт пекарь запечённых
// трупов (`silhouette::bake`)
pub(crate) use self::corpse::{corpse_quads, reset_bake_queue};
use self::look::{on_calm_tint, on_panic_tint};
// камерный LOD: `FarTag`/`FAR_SLICE_TICKS` наружу — их спрашивают поштучные
// проходы движения (`interpolate`, шаг тика) и регистрация срезных систем
pub use self::lod::{FAR_SLICE_TICKS, FarTag, LodDwell};
use self::lod::{assign_lod_tiers, far_onset, far_slice};
use self::soul::rise_souls;
pub use self::soul::{SoulMote, release_soul};
// `pick_wander_targets` наружу — им пользуется демо-сцена расталкивания
// (`examples/demos/crowd_demo.rs`), чтобы гонять толпу настоящим блужданием, а
// не своей выдумкой; `HumanPlugin` целиком ей не подходит (его `spawn_humans`
// расселил бы всё население по всей карте)
pub use self::systems::{
    PendingPopulation, PopulationBirth, PopulationSpawn, pick_wander_targets, spawn_population,
};
use self::systems::{pump_pending_population, spawn_humans, spread_changed, sync_human_pace};
use crate::determinism::{DeterminismPlugin, SimPipeline};
use crate::loading::{AppState, WorldInitSet};
use crate::prefs::PrefsExt;
use crate::silhouette::{bake, far};
use crate::tick_stage::TickStage;

pub struct HumanPlugin;

impl Plugin for HumanPlugin {
    fn build(&self, app: &mut App) {
        // материал запечённых трупов — свой конвейер и свои ассеты
        // (`Assets<PointMaterial>`), как у материалов карты. Гейт по
        // `AssetPlugin`: конвейер везёт системы, которым нужен `AssetServer`,
        // и на голых стендах (тесты, реплей на MinimalPlugins) роняет первый
        // кадр; там пекарь и так молча стоит — трупы остаются сущностями.
        if app.is_plugin_added::<bevy::asset::AssetPlugin>() {
            app.add_plugins(
                bevy::sprite_render::Material2dPlugin::<bake::PointMaterial>::default(),
            );
        }
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
            .register_type::<PopulationSize>()
            .register_type::<Attire>()
            .register_type::<BloodPool>()
            .register_type::<BloodSpatter>()
            .register_type::<BloodSpread>()
            .register_type::<SoulMote>()
            .register_type::<PendingPopulation>()
            .register_type::<LodDwell>()
            .register_type::<FarTag>()
            .register_type::<BakeQueue>()
            .register_type::<bake::CorpseChunk>()
            .register_type::<far::FarCrowdChunk>()
            .init_resource::<BakeQueue>()
            .init_resource::<bake::DespawnQueue>()
            .init_resource::<bake::CorpseField>()
            .init_resource::<far::FarCrowdField>()
            .add_observer(reset_bake_queue)
            .add_observer(bake::on_world_started)
            // дальний слой живых (Фаза 5 ×10): членство заводит переход яруса
            // — demote; обратный переход членство не снимает, его разбирает
            // перестройка (см. far.rs)
            .add_observer(far::on_far_inserted)
            .add_observer(far::on_world_started)
            // смена города: сущности чанков снимает `DespawnOnExit`, поле
            // дальнего слоя — здесь, у владельца (как `clear_map_backends`
            // в navigation): пережившее сцену поле гоняло бы перестройку
            // нового мира по мёртвым клеткам прошлого
            .add_systems(OnExit(AppState::Playing), far::clear_far_field)
            // точечный материал запечённых трупов — на старте, сразу после
            // атласа (он ест то же изображение); без рендера ресурс не
            // вставится, и пекарь со своей перестройкой молча стоят
            .add_systems(
                Startup,
                bake::init_point_material.after(crate::silhouette::build_atlas),
            )
            .register_pref::<HumanStyle>()
            .init_resource::<PopulationSize>()
            // атлас силуэтов собирает `SilhouettePlugin` (в игре); без него
            // ресурс остаётся пустым, и спавн рисует обычные квадраты —
            // приложению без рендера (реплей, тесты) больше и не нужно
            .init_resource::<crate::silhouette::Silhouettes>()
            // тон паники — на переходах, а не покадрово: обсерверы смены
            // `HumanFleeTag` красят спрайт при входе в панику и при выходе
            .add_observer(on_panic_tint)
            .add_observer(on_calm_tint)
            .add_systems(
                OnEnter(AppState::Playing),
                spawn_humans.in_set(WorldInitSet::Spawn),
            )
            // Насос расселения, обе ветки — см. док коммент системы: тиковая
            // (до `TickStage::Tick`) держит реплей, кадровая (в `Update`) —
            // прогрев, на паузе которого `FixedUpdate` не идёт вовсе.
            // `run_if` обязателен вдвойне: без ресурса насос падает на
            // валидации `ResMut`, а после исчерпания перестаёт запускаться
            .add_systems(
                Update,
                pump_pending_population
                    .in_set(SimPipeline::Live)
                    .run_if(resource_exists::<PendingPopulation>)
                    .before(pick_wander_targets),
            )
            .add_systems(
                FixedUpdate,
                pump_pending_population
                    .in_set(SimPipeline::BothModes)
                    .run_if(resource_exists::<PendingPopulation>)
                    .before(TickStage::Tick),
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
                    // `TickStage::HumanBehavior` держит порядок (люди после
                    // демонов), но стадии — только порядок: без ветки
                    // конвейера стадия не гейтит ничего
                    (panic, flee, escape).chain().in_set(SimPipeline::BothModes),
                    // души — косметика на тике, в хвосте поведения: искра,
                    // выпущенная обсервером убийства этого тика, поднимается
                    // со следующего. В `BothModes`, потому что живёт внутри
                    // мира; despawn отжившей — в `FixedUpdate`, где ему и
                    // место (CLAUDE.md)
                    rise_souls.in_set(SimPipeline::BothModes),
                )
                    .chain()
                    .in_set(TickStage::HumanBehavior),
            )
            // Дальний конвейер (onset + срезы) — отдельно от ближних, но с
            // полным порядком: расписание этого проекта не оставляет
            // конфликтующих систем без порядка («кто первый» у исполнителя —
            // скрытое состояние; пин в tests ниже). Общий `chain` из дюжины
            // систем ставил auto-sync-пойнт между каждой парой — до девяти
            // применений буферов за тик; `chain_ignore_deferred` даёт тот же
            // порядок без пойнтов: конфликт доступов и так сериализует
            // копии. Единственный sync-пойнт — после `far_onset` (обычный
            // `chain` наружной пары): срез обязан видеть вставленный этим
            // тиком `FleeRepath` и убегает тем же тиком, зеркально паре
            // `panic` → `flee` ближних. `.after(escape)` — порядок против
            // ближней цепочки, sync-пойнта не ставит: множества пешек
            // непересекающиеся. Только `SimPipeline::Live`: под тумблером
            // детерминизма дальних не существует вовсе — реплей идёт полным
            // AI (план ×10, решение владельца).
            .add_systems(
                FixedUpdate,
                (
                    // onset паники дальних — инвертированный сбор (демоны ×
                    // соседи), зеркально `panic`; только `Live` — см. lod.rs
                    far_onset,
                    (
                        far_slice::<0>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0.is_multiple_of(FAR_SLICE_TICKS)
                        }),
                        far_slice::<1>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 1
                        }),
                        far_slice::<2>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 2
                        }),
                        far_slice::<3>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 3
                        }),
                        far_slice::<4>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 4
                        }),
                        far_slice::<5>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 5
                        }),
                        far_slice::<6>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 6
                        }),
                        far_slice::<7>.run_if(|tick: Res<crate::determinism::SimTick>| {
                            tick.0 % FAR_SLICE_TICKS == 7
                        }),
                    )
                        .chain_ignore_deferred(),
                )
                    .chain()
                    .after(escape)
                    .in_set(TickStage::HumanBehavior)
                    .in_set(SimPipeline::Live),
            )
            // кровь растекается в кадре, а не на тике: это косметика, тика ей
            // не нужно, а роста хочется плавного. `BothModes` — потому что
            // трупы есть в обоих режимах, и только множество приносит гейт на
            // мир. Перед системой силуэтов, чтобы переписанный размер доехал
            // до спрайта в том же кадре, а не в следующем
            .add_systems(
                Update,
                spread_blood
                    .before(crate::silhouette::size_fresh_silhouettes)
                    .in_set(SimPipeline::BothModes),
            )
            // Запекание трупов (Фаза 3 ×10): пекарь после `spread_blood` — та
            // же лужа, что встала в очередь этим кадром, может печься им же.
            // Только `Live`: под тумблером детерминизма и в реплей-дворе
            // рендера нет — материал не инициализирован, трупы остаются
            // сущностями (штатно). Зум в юниформе материала — сам по себе:
            // с пекарем его не связывает ничего, кроме «тот же материал».
            // Дальние чанки живут тем же материалом и потому в той же цепочке
            // (Фаза 5 ×10): их перестройка читает тот же юниформ и тот же
            // `Assets<Mesh>`
            .add_systems(
                Update,
                (
                    bake::bake_queued_corpses,
                    bake::rebuild_dirty_chunks,
                    bake::sync_point_uniform,
                    far::rebuild_far_chunks,
                )
                    .chain()
                    .after(spread_blood)
                    .in_set(SimPipeline::Live),
            )
            // Сдача запечённых: деспавн тел — в `FixedUpdate`, в хвосте
            // `HumanBehavior` рядом с `rise_souls` (правило CLAUDE.md «массовый
            // деспавн не из середины `Update`»); очередь пишет пекарь из кадра
            .add_systems(
                FixedUpdate,
                bake::despawn_baked_corpses
                    .after(rise_souls)
                    .in_set(TickStage::HumanBehavior)
                    .in_set(SimPipeline::Live),
            )
            .add_systems(
                Update,
                (
                    // назначение ярусов — первым в кадре: свежепродвинутый в
                    // кадр обязан успеть подать заявку тем же кадром, иначе
                    // прогрев ждёт его лишний кадр. `SimPipeline::Live`:
                    // расписание дальних — свойство живого вида, реплей ярус
                    // не знает
                    assign_lod_tiers,
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
    /// гейт на мир (`in_world`). Стадии тика (`TickStage`) порядок только
    /// задают: настраивай цепочку или нет, стадия не гейтит **ничего**.
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
            5,
            "цепочка panic/flee/escape, rise_souls и насос расселения обязаны \
             состоять в SimPipeline::BothModes"
        );
    }
}
