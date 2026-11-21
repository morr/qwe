//! Орда: волны демонов поверх фонового спавна.
//!
//! Фон ([`tick_spawner`](crate::demon::systems::tick_spawner)) разливает
//! демонов из портала по таймеру; волны — его вторая половина: раз в затишье
//! на карте открывается очаг, из которого за несколько тиков выходит
//! [`HordeStyle::wave_size`] демонов. Общий потолок живых демонов у источников
//! один — [`DemonStyle::cap`](crate::demon::DemonStyle::cap): волна выпускает
//! `min(wave_size, свободное место)`, частичная волна легальна, при полном
//! капе волны вырождаются в пустые затишья (трейд принят владельцем при ревью
//! плана ×10).
//!
//! Детерминизм: расписание — функция `(seed, SimTick)`, и каждый параметр
//! волны — очаг, разброс позиций выпуска, затишье — разыгрывается из **чистого
//! потока** её номера [`stream`](crate::rng::stream)`(seed, Demon,
//! HORDE_WAVE_KEY + wave_index)`. Поток заводится и умирает внутри одного
//! вызова — как поток размещения населения; никаких «живых» ГПСЧ-ресурсов
//! план не добавляет. Ключи волн — верхняя половина пространства ключей
//! (от 2⁶³): у ключей-решений верхние 32 бита — номер пешки, и у настоящих
//! прогонов он далеко ниже 2³¹.

use std::time::Duration;

use bevy::prelude::*;
use rand::RngExt;

use crate::demon::components::{DemonSpawner, DemonStyle, HordeStyle};
use crate::demon::systems::{DemonArrival, DemonBirth, spawn_demon};
use crate::determinism::SimTick;
use crate::diagnostics::{SIM_HORDE_MS, SimMeasure};
use crate::loading::WorldStarted;
use crate::navigation::{ArcNavmesh, Navmesh};
use crate::portal::PortalPos;
use crate::rng::{RngDomain, WorldSeed, stream};
use crate::silhouette::Silhouettes;

/// Ключ первой волны в пространстве ключей [`stream`](crate::rng::stream):
/// половина от 2⁶⁴. У решений пешек ключ — `(pawn_id << 32) | решение`, и
/// пока номер пешки меньше 2³¹ (людей 200 000, демонов до 5000 — запас в сто
/// тысяч раз), столкновение невозможно.
pub const HORDE_WAVE_KEY: u64 = 0x8000_0000_0000_0000;

/// Радиус разброса выпуска вокруг очага, м. Теснее — стая давится
/// расталкиванием на первом же тике, шире — волна перестаёт читаться как
/// событие в одном районе.
const WAVE_SCATTER: f32 = 50.0;
/// Попыток найти проходимую точку внутри разброса, после чего демон ставится
/// на очаг (он проходим по построению). Лимит — та же страховка, что у
/// расселения населения (`PLACEMENT_TRIES`): сплошь закрытый район не вешает
/// выпуск.
const WAVE_SCATTER_TRIES: u32 = 16;
/// Попыток разыграть проходимый тайл очага по всей карте, после чего тайл
/// берётся сканом ([`Navmesh::passable_from`]) — как у расселения населения.
const WAVE_HEARTH_TRIES: u32 = 64;
/// Разброс затишья: волна выходит через `wave_every × (1 ± jitter)` — ритм
/// не метроном, очередная волна не предсказуема до секунды.
const WAVE_LULL_JITTER: f32 = 0.25;

/// Состояние расписания орды: состояние мира, а не настройка — `WorldStarted`
/// пересобирает его целиком ([`on_world_started`]). В реестре типов — ради
/// живого осмотра по BRP (`res get HordeSchedule`), рядом с `DemonSpawner`.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub struct HordeSchedule {
    /// Тик, на котором выходит следующая волна. Ноль — первая волна выходит
    /// вместе со стартовым залпом портала.
    pub next_wave_tick: u64,
    /// Номер следующей волны — ключ её потока и счётчик вышедших волн.
    pub wave_index: u64,
    /// Точки выпуска идущей волны, FIFO: за тик вычитывается
    /// [`HordeStyle::wave_len`] — всплеск команд размазан по тикам.
    pub release: Vec<Vec2>,
    /// Очаг идущей волны — происхождение ([`WarpOrigin`](crate::demon::WarpOrigin))
    /// её демонов.
    pub release_origin: Vec2,
}

impl HordeSchedule {
    /// Обязательства перед капом: вышедшая очередь волны место уже заняла —
    /// координаты посчитаны под свободное место до её постановки, и фон, не
    /// считающий её, переполнял бы кап на время выпуска.
    pub fn pending(&self) -> usize {
        self.release.len()
    }
}

/// Разыгранная волна: всё её жребий одним куском, из потока одного номера.
/// Чистая функция мира — свободного места и счётчиков в ней нет, усечение до
/// капа делает система; поэтому «то же зерно — то же расписание» проверяется
/// без приложения ([`tests::the_same_seed_plays_the_same_wave`]).
struct WavePlan {
    /// Очаг — центр проходимого тайла.
    origin: Vec2,
    /// Точки выпуска; длиной всегда `wave_size` — усечение до свободного
    /// места на стороне системы.
    positions: Vec<Vec2>,
    /// Затишье до следующей волны, сек.
    lull_secs: f32,
}

/// Разыграть волну `wave_index` мира `world_seed` по её чистому потоку.
///
/// Обход — фиксированный счётчик (`for _ in 0..n`), как у размещения
/// населения: потребление потока не зависит ни от мира, ни от порядка запросов.
fn plan_wave(world_seed: u64, wave_index: u64, style: &HordeStyle, navmesh: &Navmesh) -> WavePlan {
    let mut rng = stream(world_seed, RngDomain::Demon, HORDE_WAVE_KEY + wave_index);

    // ни одного проходимого тайла — очаг ставить некуда (упавший OSM-экстракт,
    // регрессия заливки); та же отсекающая проверка, что у расселения
    if navmesh.passable_from(IVec2::ZERO).is_none() {
        warn!(
            "horde: navmesh {}x{} has no passable tile, wave {wave_index} is empty",
            navmesh.grid_size.x, navmesh.grid_size.y
        );
        return WavePlan {
            origin: Vec2::ZERO,
            positions: Vec::new(),
            lull_secs: style.wave_every(),
        };
    }

    let mut hearth_tile = IVec2::ZERO;
    let mut drawn = None;
    for _ in 0..WAVE_HEARTH_TRIES {
        hearth_tile = IVec2::new(
            rng.random_range(0..navmesh.grid_size.x),
            rng.random_range(0..navmesh.grid_size.y),
        );
        if navmesh.is_passable(hearth_tile.x, hearth_tile.y) {
            drawn = Some(hearth_tile);
            break;
        }
    }
    // `passable_from` не может вернуть `None`: пустую сетку отсекла проверка
    // выше, и с тех пор навмеш никто не менял
    let hearth_tile =
        drawn.unwrap_or_else(|| navmesh.passable_from(hearth_tile).unwrap_or(hearth_tile));
    let hearth = navmesh.tile_center(hearth_tile);

    let mut positions = Vec::with_capacity(style.wave_size);
    for _ in 0..style.wave_size {
        let mut position = hearth;
        for _ in 0..WAVE_SCATTER_TRIES {
            let angle = rng.random_range(0.0..std::f32::consts::TAU);
            let radius = rng.random_range(0.0..WAVE_SCATTER);
            let candidate = hearth + Vec2::from_angle(angle) * radius;
            let tile = navmesh.to_tile(candidate);
            if navmesh.is_passable(tile.x, tile.y) {
                position = candidate;
                break;
            }
        }
        positions.push(position);
    }

    let lull_secs = style.wave_every()
        * (1.0 - WAVE_LULL_JITTER + 2.0 * WAVE_LULL_JITTER * rng.random_range(0.0..1.0));

    WavePlan {
        origin: hearth,
        positions,
        lull_secs,
    }
}

/// Затишье в тиках: расписание живёт в `SimTick`, а настройка — в секундах.
/// Шаг фиксированный, пересчёт — одно деление; NaN и бесконечность отсечены
/// [`HordeStyle::wave_every`].
fn secs_to_ticks(secs: f32, timestep: Duration) -> u64 {
    (secs / timestep.as_secs_f32()).round() as u64
}

/// Расписание и выпуск волн; в `TickStage::Spawner` после спавнера, той же
/// цепочкой — залп, фон и волна — один класс порядка: родившийся на тике
/// демон входит в сетку демонов на нём же.
#[allow(clippy::too_many_arguments)]
pub fn play_waves(
    fixed: Res<Time<Fixed>>,
    tick: Res<SimTick>,
    mut commands: Commands,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    mut schedule: ResMut<HordeSchedule>,
    mut spawner: ResMut<DemonSpawner>,
    style: Res<HordeStyle>,
    demon_style: Res<DemonStyle>,
    portal_pos: Res<PortalPos>,
    seed: Res<WorldSeed>,
    silhouettes: Res<Silhouettes>,
    navmesh: Res<ArcNavmesh>,
) {
    let _measure = SimMeasure::new(&mut diagnostics, &SIM_HORDE_MS);

    // волны выключены — фон-only, сегодняшний мир: и расписание стоит, и
    // недовыпущенная очередь снимается следом
    if style.wave_size == 0 {
        schedule.release.clear();
        return;
    }

    let birth = DemonBirth::new(&seed, &portal_pos, &demon_style, &silhouettes);

    // выпуск идущей волны: порция за тик. `wave_len` 0 (BRP-мусор) ведёт себя
    // как 1 — иначе очередь никогда не кончилась бы
    if !schedule.release.is_empty() {
        let portion = style.wave_len.max(1).min(schedule.release.len());
        let origin = schedule.release_origin;
        for position in schedule.release.drain(..portion) {
            spawn_demon(
                &mut commands,
                &mut spawner,
                &birth,
                DemonArrival::Wave { position, origin },
            );
        }
        return;
    }

    if tick.0 < schedule.next_wave_tick {
        return;
    }

    // жребий волны — целиком из её потока, независимо от состояния мира
    let guard = navmesh.read();
    let plan = plan_wave(seed.0, schedule.wave_index, &style, &guard);
    drop(guard);

    // свободное место: общий кап минус вышедшие минус обязательства очереди
    let free = demon_style
        .cap()
        .saturating_sub(spawner.spawned + schedule.pending());
    schedule.release_origin = plan.origin;
    schedule.release = plan.positions.into_iter().take(free).collect();
    schedule.wave_index += 1;
    schedule.next_wave_tick = tick.0 + secs_to_ticks(plan.lull_secs, fixed.timestep());
}

/// Новый прогон мира — расписание с нуля. Предусловие то же, что у сброса
/// спавнера: живых демонов быть не должно, и жалоба о них уже прозвучала из
/// `demon::on_world_started` — своего голоса здесь не нужно.
pub(super) fn on_world_started(_event: On<WorldStarted>, mut schedule: ResMut<HordeSchedule>) {
    *schedule = HordeSchedule::default();
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::demon::systems::pick_wander_targets;
    use crate::demon::{Demon, DemonWanderTag, WarpOrigin};
    use crate::loading::{AppState, PlayPhase};
    use crate::navigation::Backend;
    use crate::portal::PortalPos;
    use crate::rng::{PawnId, WanderIndex};
    use crate::settings::DEMON_INITIAL_BURST;

    /// Пин расписания: то же зерно и номер волны — тот же жребий целиком.
    /// Расписание — чистая функция `(seed, номер)`, поэтому проверяется без
    /// приложения: мир не входит в её входы. Навмеш — пустой, всюду
    /// проходимый, на полный размер карты: в дворе фикстуры проходимых тайлов
    /// мало, и скан-фолбэк сводит очаг соседних волн к одному тайлу, тогда как
    /// здесь разница жребия видна прямо в очаге.
    #[test]
    fn the_same_seed_plays_the_same_wave() {
        let navmesh = Navmesh::default();
        let style = HordeStyle::default();

        let a = plan_wave(1, 0, &style, &navmesh);
        let b = plan_wave(1, 0, &style, &navmesh);
        assert_eq!(a.origin, b.origin);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.lull_secs, b.lull_secs);

        // соседняя волна и чужое зерно — разные жребии
        let c = plan_wave(1, 1, &style, &navmesh);
        let d = plan_wave(2, 0, &style, &navmesh);
        assert_ne!(a.origin, c.origin, "волна 1 легла на очаг волны 0");
        assert_ne!(a.origin, d.origin, "чужое зерно сыграло тот же очаг");
        assert_ne!(a.positions, c.positions);
        assert_ne!(a.lull_secs, c.lull_secs);
    }

    /// Пин: позиции выпуска не ставят демонов в стены. Навмеш с настоящими
    /// препятствиями — двор фикстуры, тот же рецепт, что у прогона повтора
    /// (`tests/determinism.rs`): заливка по `MapData`, прунинг от портала.
    #[test]
    fn wave_positions_land_on_passable_tiles() {
        let yard = crate::map::osm::fixture::crowded_yard();
        let mut navmesh = Navmesh::default();
        navmesh.fill_from_mapdata(&yard.map);
        navmesh.prune_unreachable(yard.portal);
        let style = HordeStyle::default();

        let plan = plan_wave(7, 3, &style, &navmesh);
        assert!(!plan.positions.is_empty());
        for position in &plan.positions {
            let tile = navmesh.to_tile(*position);
            assert!(
                navmesh.is_passable(tile.x, tile.y),
                "точка выпуска {position} легла в непроходимый тайл {tile}"
            );
        }
    }

    /// Двор демонов: тот же состав, что у пинов спавнера в `demon/mod.rs` —
    /// здесь проверяется орда, которой у того двора нет.
    fn app(horde: HordeStyle) -> App {
        let mut app = App::new();
        app.set_error_handler(bevy::ecs::error::warn);
        let navmesh = std::sync::Arc::new(std::sync::RwLock::new(Navmesh::default()));
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            crate::demon::DemonPlugin,
            crate::spatial::SpatialPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .insert_resource(Backend::from_grid(navmesh.clone()))
        .insert_resource(ArcNavmesh(navmesh))
        .insert_resource(PortalPos(Vec2::splat(100.0)))
        .insert_resource(crate::rng::WorldSeed(1))
        .insert_resource(horde)
        .init_resource::<bevy::diagnostic::DiagnosticsStore>()
        .init_resource::<crate::telemetry::Telemetry>()
        .add_plugins(crate::loading::SimBootPlugin);
        app
    }

    fn tick_through_a_moving_warmup(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        for _ in 0..4 {
            app.update();
        }
    }

    fn go_live(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<PlayPhase>>()
            .set(PlayPhase::Live);
        for _ in 0..4 {
            app.update();
        }
    }

    /// Нулевой размер волны — сегодняшний мир: расписание молчит и очередь
    /// выпуска снимает, недовыпущенная волна не доигрывается.
    #[test]
    fn zero_wave_size_silences_the_horde_and_drops_the_release() {
        let app = &mut app(HordeStyle {
            wave_size: 0,
            ..Default::default()
        });
        tick_through_a_moving_warmup(app);
        app.world_mut().insert_resource(HordeSchedule {
            next_wave_tick: 0,
            wave_index: 0,
            release: vec![Vec2::new(500.0, 500.0), Vec2::new(600.0, 600.0)],
            release_origin: Vec2::splat(550.0),
        });
        go_live(app);

        let schedule = app.world().resource::<HordeSchedule>();
        assert!(
            schedule.release.is_empty(),
            "выключенная орда доиграла очередь: {:?}",
            schedule.release
        );
        // демонов фон тоже выпускает (интервал 0.03 с), поэтому проверяется
        // не общий счёт, а отсутствие именно волновых: у каждого живого
        // демона происхождение — портал
        let portal = app.world().resource::<PortalPos>().0;
        let mut query = app.world_mut().query_filtered::<&WarpOrigin, With<Demon>>();
        for origin in query.iter(app.world()) {
            assert_eq!(
                origin.0, portal,
                "выключенная орда выпустила своего демона: очаг {:?}",
                origin.0
            );
        }
    }

    /// Частичная волна легальна: свободного места меньше волны — выходят
    /// ровно свободные, кап не переполняется ни на одного.
    #[test]
    fn a_wave_releases_only_the_free_room() {
        let app = &mut app(HordeStyle {
            wave_size: 500,
            wave_every: 600.0,
            wave_len: 200,
        });
        tick_through_a_moving_warmup(app);
        // кап у входа в мир: залп 8 + свободного 4 — волна 500 выпускает 4
        app.world_mut().insert_resource(crate::demon::DemonStyle {
            cap: DEMON_INITIAL_BURST + 4,
            ..Default::default()
        });
        go_live(app);

        assert_eq!(
            app.world().resource::<DemonSpawner>().spawned,
            DEMON_INITIAL_BURST + 4,
            "волна переполнила общий кап"
        );
        // очередь исчерпана: всё свободное место вышло
        assert!(
            app.world().resource::<HordeSchedule>().release.is_empty(),
            "частичная волна оставила очередь недовыпущенной"
        );
    }

    /// Пин эквивалентности: у рождённого порталом демона происхождение — сам
    /// портал. Именно «прочь от портала» и было видовой политикой до орды;
    /// вектор `away` строится тем же выражением от той же точки
    /// (`pick_wander_targets`), число бросков не меняется.
    #[test]
    fn a_portal_demon_carries_the_portal_as_its_warp_origin() {
        let app = &mut app(HordeStyle {
            wave_size: 0,
            ..Default::default()
        });
        tick_through_a_moving_warmup(app);
        go_live(app);

        let portal = app.world().resource::<PortalPos>().0;
        let mut query = app.world_mut().query_filtered::<&WarpOrigin, With<Demon>>();
        for origin in query.iter(app.world()) {
            assert_eq!(
                origin.0, portal,
                "порталовый демон блуждает не от портала: {:?} против {:?}",
                origin.0, portal
            );
        }
    }

    /// Пин эквивалентности, другая половина: происхождение уводит блуждание
    /// от очага, а не от портала — один и тот же жребий одного и того же
    /// демона в одном и том же месте даёт разные цели при разных
    /// происхождениях. Система зовётся напрямую, без плагина: проверяется
    /// формула выбора цели, а не цепочка спавна.
    #[test]
    fn the_warp_origin_stears_the_first_wander_target() {
        let navmesh = Navmesh::default();
        let first_target = |origin: Vec2| {
            let mut app = App::new();
            app.insert_resource(Backend::from_grid(std::sync::Arc::new(
                std::sync::RwLock::new(navmesh.clone()),
            )))
            .insert_resource(WorldSeed(1))
            .add_systems(Update, pick_wander_targets);

            app.world_mut().spawn((
                Demon,
                WarpOrigin(origin),
                DemonWanderTag,
                crate::movement::Movable::new(10.0),
                PawnId(0),
                WanderIndex::ready(),
                // умолчание `#[require]` у `Movable` — нулевая точка; демону
                // назначено место на карте, откуда оба происхождения смотрят
                // в разные стороны
                crate::movement::SimPosition(Vec2::splat(1000.0)),
            ));
            app.update();

            let request = app
                .world_mut()
                .query_filtered::<&crate::movement::PathfindingRequest, With<Demon>>()
                .single(app.world())
                .expect("первая цель не подана");
            request.end_tile
        };

        // демон в (1000, 1000): «прочь» от портала (100, 100) — на северо-восток,
        // от очага (100, 1900) — на юго-восток
        let from_portal = first_target(Vec2::splat(100.0));
        let from_hearth = first_target(Vec2::new(100.0, 1900.0));
        assert_ne!(
            from_portal, from_hearth,
            "происхождение не влияет на первую цель: политика «от происхождения» мертва"
        );
    }
}
