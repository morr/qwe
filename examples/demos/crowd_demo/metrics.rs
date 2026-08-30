//! Замер: перекрытия, счётчики прогона, окно прохода пешки — и системы,
//! которые их считают, снимают и печатают итог.

use bevy::diagnostic::DiagnosticsStore;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use qwe::diagnostics::SIM_SEPARATION_MS;
use qwe::grid::tile_center;
use qwe::human::HumanStyle;
use qwe::movement::{
    DestinationClaim, Movable, MovableStateMovingTag, SeparationHolds, SeparationStats,
    SeparationSteer, SeparationStyle, SimPosition, SlotLab, SlotSearch, separation_cell,
    slot_side_with_slack, slot_target,
};
use qwe::navigation::PolyNavmesh;
use qwe::settings::{HUMAN_SIZE, navtile_size};

use crate::scenario::{DemoPawn, Route, Scenario};
use crate::{DemoConfig, DemoSpeed};

/// Перекрытие меньше этого считается сошедшимся: мягкий решатель оставляет
/// асимптотический хвост в единицы миллиметров, и без порога «пар» всегда
/// оказывались бы десятки — при перекрытии, которого нет ни на экране, ни по
/// смыслу.
pub(crate) const OVERLAP_EPSILON: f32 = 0.02;

/// Перекрытия текущего кадра — то, ради чего сцена и написана.
///
/// **Считается только то, что в кадре.** Расталкивание работает по
/// прямоугольнику вокруг камеры (`separation/`), и пешки за кадром не
/// разводятся намеренно — считать их значило бы мерить систему там, где она по
/// построению не работает. Прямоугольник здесь взят без запаса `VIEW_MARGIN`,
/// то есть строго внутри рабочего: всё, что попало в счёт, расталкивание
/// точно видело.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub(crate) struct Overlaps {
    /// Пешек в кадре и всего в сцене.
    pub(crate) pawns: usize,
    pub(crate) total: usize,
    /// Радиус, по которому посчитано это перекрытие — им же рисуются круги.
    pub(crate) radius: f32,
    pub(crate) pairs: usize,
    /// Максимальная нехватка расстояния до суммы радиусов, м.
    pub(crate) worst: f32,
    pub(crate) mean: f32,
    /// Сколько пешек участвует хотя бы в одной перекрывшейся паре.
    pub(crate) involved: usize,
    /// Пары, сошедшиеся ближе ОДНОГО радиуса тела: люди «плечом к плечу».
    /// В живой толпе это норма, а не артефакт, — спрайты (1.0 м) при таком
    /// расстоянии ещё только касаются.
    pub(crate) deep: usize,
    /// Пары, у которых центры ближе ПОЛОВИНЫ спрайта: тела наложились наполовину,
    /// и на экране это уже проход насквозь. Вот это артефакт, и мерить его надо
    /// отдельно от давки — первая версия считала артефактом любое `deep`, то есть
    /// заодно и всякое законное прижатие в потоке.
    pub(crate) through: usize,
    /// Позиции в кадре и признак перекрытия — чтобы гизмо рисовало ровно то,
    /// что посчитано, а не считало во второй раз.
    #[reflect(ignore)]
    pub(crate) bodies: Vec<(Vec2, bool)>,
    #[reflect(ignore)]
    pub(crate) links: Vec<(Vec2, Vec2)>,
    /// Кто именно перекрыт — не только сколько. Нужен четвёртому критерию
    /// (`sep_share`): «пешка в состоянии расталкивания» это объединение трёх
    /// множеств — придержанные, рулящие и перекрытые, — а сложить их можно
    /// только по сущностям.
    #[reflect(ignore)]
    pub(crate) involved_set: bevy::ecs::entity::EntityHashSet,
}

/// Сколько тиков движения приходится на один прогон расталкивания. Прогоны
/// считаются не по своей копии гейта, а по настоящему замеру
/// `sim/separation_ms`: система пишет его ровно раз за прогон.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub(crate) struct RunCounters {
    pub(crate) runs: u64,
    #[reflect(ignore)]
    pub(crate) last_measurement: Option<std::time::Instant>,
    pub(crate) ticks_per_run: f32,
    pub(crate) window_ticks: u64,
    pub(crate) window_runs: u64,
}

/// Отрезки маршрута, которые полигональный поиск не смог проложить.
///
/// Не молчаливый откат на прямую: цель, выбранная по проходимости тайла, может
/// лежать внутри раздутого на радиус агента контура — в игре это
/// `PathfindingError` (см. `movement/pathfinding.rs::apply_result`), и здесь пешка
/// так же остаётся стоять до следующего тика. Прямая «в обход меша» провела бы
/// её сквозь стену коридора и выглядела бы как работающий сценарий.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub(crate) struct PathMisses(pub(crate) u64);

/// Окно замера и всё, что в нём накоплено.
///
/// Два критерия, ради которых сцена и меряется:
/// 1. **пройденное расстояние** — чем меньше пешки толкаются, тем дальше
///    уезжают за то же время. Меряется тремя способами сразу, потому что каждый
///    по отдельности обманывается: `travel` (сумма модулей смещения) растёт и от
///    дрожи на месте; `progress` (сближение с текущей целью) не видит обхода,
///    который окупится через секунду; `arrivals` (сколько раз цель достигнута)
///    честнее всех, но грубее — на коротком окне их единицы. Врать одновременно
///    всем трём нечем;
/// 2. **время в расталкивании** — `held_secs` (пешка идёт ослабленным шагом,
///    [`SeparationHolds`]) и `overlap_secs` (пешка внутри чужого тела). Первое —
///    буквально «состояние расталкивания», второе — его причина.
///
/// Плюс детекторы артефактов, которые глазом ловятся не сразу: `worst_push` —
/// самый длинный одиночный толчок (телепорт), `deep_events` — пары, сошедшиеся
/// ближе ОДНОГО радиуса, то есть прошедшие сквозь друг друга на экране.
///
/// И две числовые характеристики «поток или колонна»: `spread` (насколько
/// толпа разъехалась поперёк) и `lane_order` (сложился ли правосторонний
/// порядок — встречные по разные стороны оси).
#[derive(Resource, Default)]
pub(crate) struct Trial {
    pub(crate) label: String,
    /// Длина окна в реальных секундах; 0 — интерактивный запуск без замера.
    pub(crate) window: f32,
    pub(crate) shots: bool,
    /// Реальное время открытия окна. `None`, пока сцена не готова: считать до
    /// постройки полигонального меша значит мерить сеточную ходьбу
    pub(crate) started: Option<f32>,
    pub(crate) real: f32,
    pub(crate) virtual_secs: f64,
    pub(crate) frames: u64,

    pub(crate) travel: f64,
    pub(crate) progress: f64,
    pub(crate) arrivals: u64,

    pub(crate) held_secs: f64,
    pub(crate) overlap_secs: f64,
    /// Знаменатель для обоих: сколько «пешко-секунд» в кадре всего прожито.
    pub(crate) pawn_secs: f64,

    pub(crate) worst_overlap: f32,
    pub(crate) deep_events: u64,
    /// Настоящий артефакт: тела наложились наполовину (см. `Overlaps::through`).
    pub(crate) through_events: u64,
    /// Самое длинное смещение одной пешки за ОДИН ТИК, м — детектор
    /// телепорта: потолок известен точно, см. [`sample_travel`].
    pub(crate) worst_tick_step: f32,

    pub(crate) spread: f64,
    pub(crate) spread_samples: f64,
    pub(crate) lane_order: f64,
    pub(crate) lane_samples: f64,

    /// Пешко-секунды в СОСТОЯНИИ РАСТАЛКИВАНИЯ — четвёртый критерий. Состояние
    /// это объединение трёх множеств, а не одно из них: придержанная
    /// ([`SeparationHolds`]) идёт ослабленным шагом, рулящая
    /// ([`SeparationSteer`]) идёт не туда, куда хотела, перекрытая платит и тем
    /// и другим позже. Считать только придержку значило бы объявить победителем
    /// вариант, который придержку отключил (`--hold 1`), ничего не починив.
    pub(crate) sep_secs: f64,
    /// Раздельно, чтобы было видно, из чего сложилось объединение.
    pub(crate) steer_secs: f64,
    /// Виртуальная секунда, на которой ВПЕРВЫЕ выполнились оба первых критерия
    /// (все пешки на своих слотах, никто не идёт). `None` — за окно так и не
    /// сошлось.
    pub(crate) settled_at: Option<f64>,
    /// Путь, намотанный пешками ВНЕ состояния «иду» — чистая дрожь осевшей
    /// толпы. Второй критерий числом: «никто не двигается» это не только
    /// «никто не идёт», но и «никого не колышет толчками».
    pub(crate) idle_drift: f64,

    pub(crate) sep_ms: f64,
    pub(crate) sep_ms_samples: f64,
    /// Замер, по которому уже прибавили — тот же трюк, что у `RunCounters`.
    pub(crate) last_sep_ms: Option<std::time::Instant>,

    pub(crate) shots_taken: u32,
}

/// Позиция пешки на прошлом кадре — база для `travel`. Компонентом, а не
/// картой в ресурсе: спавн и деспавн ведёт ECS, а не отдельная уборка.
#[derive(Component)]
pub(crate) struct LastSample(pub(crate) Vec2);

/// Личный счёт пешки за окно замера — то, из чего складываются МЕДИАННЫЕ
/// критерии. Суммы по толпе (`Trial::travel`, `Trial::sep_secs`) прячут
/// распределение: десяток застрявших в давке пешек тонет в двух сотнях
/// дошедших, а медиана показывает судьбу ТИПИЧНОЙ пешки. Компонентом по той же
/// причине, что [`LastSample`].
#[derive(Component, Default)]
pub(crate) struct PawnWindow {
    /// Метры, намотанные этой пешкой за окно (потиково, как `Trial::travel`).
    pub(crate) travel: f32,
    /// Её секунды в состоянии расталкивания — то же объединение трёх множеств,
    /// что у `Trial::sep_secs` (придержана, рулит или перекрыта).
    pub(crate) sep_secs: f32,
    /// Сколько метров она СБЛИЗИЛАСЬ со своими целями (то же, что `Trial::
    /// progress`, но лично).
    ///
    /// Нужен потому, что медианный `travel` во встречном потоке обманывает в
    /// СВОЮ сторону: он растёт и от дуг обхода, и от дрожи в толчее, так что
    /// «прошла больше» и «продвинулась дальше» — разные вещи. На стенде это
    /// видно прямо: вариант с `med_travel` 284 м доходил до цели втрое реже
    /// варианта с `med_travel` 207 м.
    pub(crate) progress: f32,
}

/// Где пешка стояла в момент ОТКРЫТИЯ окна замера. База для нижней границы
/// пути: сумма прямых «старт → куда в итоге встал» — это тот путь, который
/// пешки прошли бы, если бы шли к своим слотам по прямой и никого не
/// встретили. Отношение `travel` к ней (`detour`) и есть третий критерий в
/// виде, не зависящем от того, насколько плотно упакованы слоты: чем плотнее
/// толпа садится, тем БОЛЬШЕ ей идти, и голый `travel` за это наказывал бы.
#[derive(Component)]
pub(crate) struct WindowOrigin(pub(crate) Vec2);

pub(crate) fn count_ticks(mut counters: ResMut<RunCounters>) {
    counters.window_ticks += 1;
}

/// Прогон расталкивания виден по свежему замеру `sim/separation_ms`: система
/// пишет его последним действием, ровно раз за прогон.
pub(crate) fn count_separation_runs(
    diagnostics: Res<DiagnosticsStore>,
    mut counters: ResMut<RunCounters>,
) {
    let time = diagnostics
        .get(&SIM_SEPARATION_MS)
        .and_then(|diagnostic| diagnostic.measurement())
        .map(|measurement| measurement.time);
    if time.is_some() && time != counters.last_measurement {
        counters.last_measurement = time;
        counters.runs += 1;
        counters.window_runs += 1;
    }
    // окно в пару секунд: показывать среднее за весь запуск бессмысленно —
    // скорость и зум по ходу меняют картину
    if counters.window_ticks >= 128 {
        counters.ticks_per_run = if counters.window_runs > 0 {
            counters.window_ticks as f32 / counters.window_runs as f32
        } else {
            f32::INFINITY
        };
        counters.window_ticks = 0;
        counters.window_runs = 0;
    }
}

/// Пересчитать перекрытия. Своя мелкая сетка — та же, что у самого
/// расталкивания ([`SEPARATION_CELL`]), но считаем в лоб: пешек здесь сотни, и
/// понятность важнее скорости.
pub(crate) fn measure_overlaps(
    pawns: Query<(Entity, &SimPosition), With<DemoPawn>>,
    camera: Query<&Transform, With<Camera2d>>,
    window: Query<&Window>,
    style: Res<HumanStyle>,
    mut overlaps: ResMut<Overlaps>,
) {
    let (Ok(camera), Ok(window)) = (camera.single(), window.single()) else {
        return;
    };
    let cell = separation_cell(style.body_radius);
    let half_view = Vec2::new(window.width(), window.height()) / 2.0 * camera.scale.x;
    let min = camera.translation.truncate() - half_view;
    let max = camera.translation.truncate() + half_view;

    let total = pawns.iter().len();
    let bodies: Vec<(Entity, Vec2)> = pawns
        .iter()
        .map(|(entity, position)| (entity, position.0))
        .filter(|(_, position)| position.cmpge(min).all() && position.cmple(max).all())
        .collect();
    let positions: Vec<Vec2> = bodies.iter().map(|(_, position)| *position).collect();

    let mut cells: HashMap<IVec2, Vec<usize>> = HashMap::new();
    for (index, position) in positions.iter().enumerate() {
        cells
            .entry((*position / cell).floor().as_ivec2())
            .or_default()
            .push(index);
    }

    let min_distance = style.body_radius * 2.0;
    let mut pairs = 0usize;
    let mut deep = 0usize;
    let mut through = 0usize;
    let mut worst = 0.0f32;
    let mut sum = 0.0f32;
    let mut involved = vec![false; positions.len()];
    overlaps.links.clear();

    for (index, position) in positions.iter().enumerate() {
        let cell = (*position / cell).floor().as_ivec2();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(neighbours) = cells.get(&(cell + IVec2::new(dx, dy))) else {
                    continue;
                };
                for &other in neighbours {
                    if other <= index {
                        continue;
                    }
                    let overlap = min_distance - position.distance(positions[other]);
                    if overlap > OVERLAP_EPSILON {
                        pairs += 1;
                        if overlap > style.body_radius {
                            deep += 1;
                        }
                        if overlap > min_distance - HUMAN_SIZE / 2.0 {
                            through += 1;
                        }
                        sum += overlap;
                        worst = worst.max(overlap);
                        involved[index] = true;
                        involved[other] = true;
                        overlaps.links.push((*position, positions[other]));
                    }
                }
            }
        }
    }

    overlaps.pawns = positions.len();
    overlaps.total = total;
    overlaps.radius = style.body_radius;
    overlaps.pairs = pairs;
    overlaps.deep = deep;
    overlaps.through = through;
    overlaps.worst = worst;
    overlaps.mean = if pairs > 0 { sum / pairs as f32 } else { 0.0 };
    overlaps.involved = involved.iter().filter(|flag| **flag).count();
    overlaps.involved_set.clear();
    overlaps.involved_set.extend(
        bodies
            .iter()
            .zip(&involved)
            .filter_map(|((entity, _), flag)| flag.then_some(*entity)),
    );
    overlaps.bodies = positions
        .iter()
        .zip(involved)
        .map(|(position, flag)| (*position, flag))
        .collect();
}

/// Круг настоящего радиуса тела поверх спрайта: красный — перекрытие, зелёный
/// — дистанция выдержана. Без него «вплотную» и «друг на друге» на спрайтах
/// 1.0 м при дистанции покоя 0.9 м неразличимы.
pub(crate) fn draw_bodies(mut gizmos: Gizmos, overlaps: Res<Overlaps>) {
    const RED: Color = Color::srgb(0.9, 0.1, 0.1);
    for (position, overlapping) in &overlaps.bodies {
        let color = if *overlapping {
            RED
        } else {
            Color::srgb(0.15, 0.55, 0.2)
        };
        gizmos.circle_2d(*position, overlaps.radius, color);
    }
    for (from, to) in &overlaps.links {
        gizmos.line_2d(*from, *to, RED);
    }
}

/// Накопить кадр в окно замера. Считается ТО ЖЕ, что видит расталкивание, — по
/// пешкам в кадре (`Overlaps` уже отфильтровал их прямоугольником камеры).
///
/// Окно открывается не на первом кадре, а когда полигональный меш построен:
/// до этого пешки идут по сетке центрами навтайлов, а там расталкивания нет
/// вовсе (`separation_runs`), и первые доли секунды мерили бы другую систему.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_trial(
    time: Res<Time<Virtual>>,
    real: Res<Time<Real>>,
    poly: Res<PolyNavmesh>,
    overlaps: Res<Overlaps>,
    holds: Res<SeparationHolds>,
    steer: Res<SeparationSteer>,
    diagnostics: Res<DiagnosticsStore>,
    mut trial: ResMut<Trial>,
    pawns: Query<(&SimPosition, &Movable)>,
    census: Query<(Has<MovableStateMovingTag>, Has<Route>), With<DemoPawn>>,
    mut windows: Query<(Entity, &mut PawnWindow)>,
    mut union: Local<bevy::ecs::entity::EntityHashSet>,
) {
    if trial.started.is_none() {
        if poly.build().is_none() {
            return;
        }
        trial.started = Some(real.elapsed_secs());
    }
    let started = trial.started.expect("window is open");
    trial.real = real.elapsed_secs() - started;
    let dt = time.delta_secs_f64();
    trial.virtual_secs += dt;
    trial.frames += 1;

    if let Some(measurement) = diagnostics
        .get(&SIM_SEPARATION_MS)
        .and_then(|diagnostic| diagnostic.measurement())
        && Some(measurement.time) != trial.last_sep_ms
    {
        trial.last_sep_ms = Some(measurement.time);
        trial.sep_ms += measurement.value;
        trial.sep_ms_samples += 1.0;
    }

    trial.pawn_secs += overlaps.pawns as f64 * dt;
    trial.held_secs += holds.0.len() as f64 * dt;
    trial.overlap_secs += overlaps.involved as f64 * dt;
    trial.steer_secs += steer.0.len() as f64 * dt;
    // объединение трёх множеств, а не сумма трёх счётчиков: пешка бывает
    // придержана, зарулена и перекрыта одновременно, и втрое считать её нельзя.
    // Буфер в `Local` — множество строится каждый кадр, и своя аллокация на
    // кадр была бы платой на ровном месте
    union.clear();
    union.extend(overlaps.involved_set.iter().copied());
    union.extend(holds.0.iter().copied());
    union.extend(steer.0.keys().copied());
    trial.sep_secs += union.len() as f64 * dt;
    // то же множество — в личный счёт каждой пешки: медианное время в
    // расталкивании собирается из этих секунд в `finish_trial`
    if !union.is_empty() {
        for (entity, mut window) in &mut windows {
            if union.contains(&entity) {
                window.sep_secs += dt as f32;
            }
        }
    }
    trial.worst_overlap = trial.worst_overlap.max(overlaps.worst);
    trial.deep_events += overlaps.deep as u64;
    trial.through_events += overlaps.through as u64;

    // Ось потока — СРЕДНЕЕ положение толпы, а не центр карты. По центру карты
    // обе величины выходили константами: цели пешек стоят в центрах навтайлов,
    // вся колонна висит на одном и том же смещении от `centre`, и «разброс»
    // читался как ровно 1.00 м во всех до единого прогонах — включая
    // выключенное расталкивание. Расслоение — это разлёт ОТНОСИТЕЛЬНО СЕБЯ.
    let mut axis = 0.0f64;
    let mut population = 0.0f64;
    for (position, _) in &pawns {
        axis += position.0.y as f64;
        population += 1.0;
    }
    let axis = (axis / population.max(1.0)) as f32;

    let mut spread = 0.0f64;
    let mut spread_counted = 0.0f64;
    let mut order = 0.0f64;
    let mut counted = 0.0f64;
    for (position, movable) in &pawns {
        let across = position.0.y - axis;
        spread += across.abs() as f64;
        spread_counted += 1.0;
        // правосторонний порядок: идущий на +x обязан быть НИЖЕ оси, идущий на
        // −x — выше. +1 — полосы сложились, 0 — перемешаны, −1 — левостороннее
        let along = movable.last_direction.x;
        if along.abs() > 0.5 && across.abs() > 0.05 {
            order += -(along.signum() * across.signum()) as f64;
            counted += 1.0;
        }
    }
    trial.spread += spread;
    trial.spread_samples += spread_counted;
    trial.lane_order += order;
    trial.lane_samples += counted;

    // Критерии 1 и 2 одним числом: момент, когда толпа СОШЛАСЬ. «Сошлась» —
    // это ни одной идущей пешки И ни одного невыданного отрезка маршрута:
    // пешка, у которой `Route` ещё висит, не дошла, а стоит и каждый тик
    // безуспешно просит путь (`PathMisses`), и по одному лишь «не идёт» она
    // читалась бы как осевшая.
    if trial.settled_at.is_none()
        && census.iter().len() > 0
        && census.iter().all(|(moving, pending)| !moving && !pending)
    {
        trial.settled_at = Some(trial.virtual_secs);
    }
}

/// Путь и прогресс — ПОТИКОВО, в `FixedUpdate`.
///
/// Почему не в кадре вместе с остальным. Толчок расталкивания приходит раз в
/// кадр, а шагов ходьбы в кадре пять с лишним (5x, 64 Гц): на кадровой выборке
/// они частично гасят друг друга внутри одного замера, и «пройденное
/// расстояние» выходило меньше настоящего, а `push` — больше него, до
/// невозможного отношения 1.67. Тик — тот шаг, на котором обе величины
/// определены, и потолок смещения за тик известен точно (`speed / 64` плюс
/// [`SeparationLab::max_step`]), так что тот же счётчик служит и детектором
/// телепорта.
///
/// `progress` — СО ЗНАКОМ, а не выпрямленный. Выпрямленная сумма приращений
/// растёт от одной дрожи на месте: пешка, которую качает толчками вперёд-назад,
/// набирает «прогресс», никуда не уехав (первая версия так и намерила прогресс
/// БОЛЬШЕ пути). Со знаком сумма телескопируется в честное «на сколько
/// приблизился к цели за окно», а цена обхода честно вычитается.
pub(crate) fn sample_travel(
    mut trial: ResMut<Trial>,
    mut pawns: Query<(
        &SimPosition,
        &Movable,
        &mut LastSample,
        &mut ProgressSample,
        &mut WindowOrigin,
        &mut PawnWindow,
    )>,
) {
    if trial.started.is_none() {
        // окно ещё не открыто: базу всё равно освежаем, иначе первый тик окна
        // получил бы смещение за всю постройку меша разом
        for (position, _, mut last, mut sample, mut origin, _) in &mut pawns {
            last.0 = position.0;
            origin.0 = position.0;
            sample.target = None;
        }
        return;
    }
    for (position, movable, mut last, mut sample, _, mut window) in &mut pawns {
        let step = position.0.distance(last.0);
        last.0 = position.0;
        trial.travel += step as f64;
        window.travel += step;
        trial.worst_tick_step = trial.worst_tick_step.max(step);

        let qwe::movement::MovableState::Moving(target) = movable.state else {
            // не идёт, а сместилась — это её колышет расталкивание: дрожь
            // осевшей толпы, второй критерий
            trial.idle_drift += step as f64;
            sample.target = None;
            continue;
        };
        let distance = position.0.distance(tile_center(target));
        // цель сменилась — прошлое расстояние про другую точку, прогресса нет
        if sample.target == Some(target) {
            trial.progress += (sample.distance - distance) as f64;
            window.progress += sample.distance - distance;
        }
        sample.target = Some(target);
        sample.distance = distance;
    }
}

/// Расстояние до текущей цели на прошлом тике, см. [`sample_travel`].
#[derive(Component, Default)]
pub(crate) struct ProgressSample {
    pub(crate) target: Option<IVec2>,
    pub(crate) distance: f32,
}

/// Снимок экрана в начале, середине и конце окна: телепорт и проход насквозь
/// числами ловятся не полностью, и отчёт без картинок не проверить.
pub(crate) fn take_shots(mut commands: Commands, mut trial: ResMut<Trial>) {
    if !trial.shots || trial.window <= 0.0 || trial.started.is_none() {
        return;
    }
    let due = match trial.shots_taken {
        0 => 0.5,
        1 => trial.window / 2.0,
        2 => trial.window - 0.3,
        _ => return,
    };
    if trial.real < due {
        return;
    }
    // под `target/`: снимки — расходный материал замера, а не результат, и в
    // репозитории им делать нечего (`target` уже в `.gitignore`)
    const SHOTS: &str = "target/crowd-shots";
    std::fs::create_dir_all(SHOTS).expect("cannot create the screenshot directory");
    let path = format!("{SHOTS}/{}-{}.png", slug(&trial.label), trial.shots_taken);
    trial.shots_taken += 1;
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

/// Медиана выборки; сортирует буфер на месте. Чётная длина — среднее двух
/// центральных, пустая выборка — 0 (строка `RESULT` обязана остаться числом).
pub(crate) fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(f32::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

pub(crate) fn slug(label: &str) -> String {
    label
        .chars()
        .map(|symbol| {
            if symbol.is_alphanumeric() {
                symbol
            } else {
                '-'
            }
        })
        .collect()
}

/// Закрыть окно: одна строка `RESULT` в stdout и выход. Строка машинно
/// читаемая (`ключ=значение`), потому что её собирают в таблицу отчёта, а не
/// читают глазами.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_trial(
    trial: Res<Trial>,
    stats: Res<SeparationStats>,
    misses: Res<PathMisses>,
    overlaps: Res<Overlaps>,
    config: Res<DemoConfig>,
    search: Res<SlotSearch>,
    style: Res<HumanStyle>,
    lab: Res<SlotLab>,
    census: Query<
        (
            &SimPosition,
            &WindowOrigin,
            &PawnWindow,
            Option<&DestinationClaim>,
            Has<MovableStateMovingTag>,
            Has<Route>,
        ),
        With<DemoPawn>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    if trial.window <= 0.0 || trial.started.is_none() || trial.real < trial.window {
        return;
    }
    let pawn_secs = trial.pawn_secs.max(1e-9);

    // Финальная перепись — первые два критерия в лоб. «В центре» меряется
    // радиусом поиска слота плюс запас в дистанцию покоя: дальше него слота
    // пешке не выдавали, значит всё, что там стоит, — это либо осевшая толпа,
    // либо застрявший хвост, и различать их надо, а не считать вместе.
    let reach = search.0 + 2.0 * overlaps.radius;
    let (mut settled, mut walking, mut pending, mut stranded) = (0u32, 0u32, 0u32, 0u32);
    // Первый критерий в строгом виде: не «встала где-то в центре», а «стои́т
    // НА СВОЁМ слоте». Разница принципиальна — толпа, осевшая сплошным
    // перекрытием там, куда её вытолкнуло, по счётчику `settled` неотличима от
    // толпы, разошедшейся по решётке, а на экране это две разные картинки.
    // Допуск — половина шага решётки: дальше начинается чужой слот
    let side = slot_side_with_slack(style.body_radius * 2.0, lab.slack);
    let on_slot_tolerance = side as f32 * navtile_size() / 2.0;
    let mut on_slot = 0u32;
    let mut net = 0.0f64;
    // След толпы: докуда от центра она в итоге растеклась. Без него третий
    // критерий читается неверно — толпа, севшая просторнее, проходит МЕНЬШЕ
    // (останавливается раньше), и «выигрыш в пути» оказывается платой площадью
    let mut foot = 0.0f32;
    // Личные счета — в медианы: типичная пешка вместо суммы по толпе
    let mut travels: Vec<f32> = Vec::with_capacity(census.iter().len());
    let mut sep_times: Vec<f32> = Vec::with_capacity(census.iter().len());
    let mut progresses: Vec<f32> = Vec::with_capacity(census.iter().len());
    for (position, origin, window, claim, moving, route) in &census {
        travels.push(window.travel);
        sep_times.push(window.sep_secs);
        progresses.push(window.progress);
        if !moving
            && !route
            && let Some(claim) = claim
            && position.0.distance(tile_center(slot_target(claim.0, side))) <= on_slot_tolerance
        {
            on_slot += 1;
        }
        net += position.0.distance(origin.0) as f64;
        let home = position.0.distance(config.centre) <= reach;
        if !moving && !route {
            foot = foot.max(position.0.distance(config.centre));
        }
        match (moving, route, home) {
            (true, ..) => walking += 1,
            (false, true, _) => pending += 1,
            (false, false, true) => settled += 1,
            (false, false, false) => stranded += 1,
        }
    }

    println!(
        "RESULT label={label} real={real:.2} virtual={virtual_secs:.1} pawns={pawns} \
         settled={settled} on_slot={on_slot} walking={walking} pending={pending} \
         stranded={stranded} \
         settled_at={settled_at:.1} idle_drift={idle_drift:.1} foot={foot:.1} \
         travel={travel:.0} net={net:.0} detour={detour:.3} \
         med_travel={med_travel:.1} med_progress={med_progress:.1} \
         med_sep={med_sep:.2} med_sep_share={med_sep_share:.4} \
         sep_share={sep_share:.4} steer_share={steer_share:.4} \
         progress={progress:.0} arrivals={arrivals} \
         held_share={held:.4} overlap_share={overlap:.4} \
         push={push:.0} push_share={push_share:.4} \
         worst_overlap={worst:.3} deep={deep} through={through} worst_push={worst_push:.3} worst_step={worst_step:.3} \
         spread={spread:.2} lane_order={lane_order:.3} \
         sep_ms={sep_ms:.3} runs={runs} pairs={pairs} anticipated={anticipated} \
         fps={fps:.1} misses={misses}",
        label = trial.label,
        real = trial.real,
        virtual_secs = trial.virtual_secs,
        pawns = overlaps.total,
        settled = settled,
        on_slot = on_slot,
        walking = walking,
        pending = pending,
        stranded = stranded,
        // −1, а не пусто: строку разбирают как `ключ=число`, и «не сошлось»
        // обязано быть числом, отличимым от любой настоящей секунды
        settled_at = trial.settled_at.unwrap_or(-1.0),
        idle_drift = trial.idle_drift,
        foot = foot,
        travel = trial.travel,
        net = net,
        detour = trial.travel / net.max(1e-9),
        med_travel = median(&mut travels),
        med_progress = median(&mut progresses),
        med_sep = median(&mut sep_times),
        med_sep_share = median(&mut sep_times) as f64 / trial.virtual_secs.max(1e-9),
        sep_share = trial.sep_secs / pawn_secs,
        steer_share = trial.steer_secs / pawn_secs,
        progress = trial.progress,
        arrivals = trial.arrivals,
        held = trial.held_secs / pawn_secs,
        overlap = trial.overlap_secs / pawn_secs,
        push = stats.push_metres,
        push_share = stats.push_metres / trial.travel.max(1e-9),
        worst = trial.worst_overlap,
        deep = trial.deep_events,
        through = trial.through_events,
        worst_push = stats.worst_push,
        worst_step = trial.worst_tick_step,
        spread = trial.spread / trial.spread_samples.max(1.0),
        lane_order = trial.lane_order / trial.lane_samples.max(1.0),
        sep_ms = trial.sep_ms / trial.sep_ms_samples.max(1.0),
        runs = stats.runs,
        pairs = stats.overlapping_pairs,
        anticipated = stats.anticipated_pairs,
        fps = trial.frames as f64 / trial.real.max(1e-9) as f64,
        misses = misses.0,
    );
    exit.write(AppExit::Success);
}

/// Та же сводка в stdout раз в две реальные секунды: сцену смотрят глазами, но
/// числа надо ещё и приложить к отчёту, а из окна их не скопировать.
#[allow(clippy::too_many_arguments)]
pub(crate) fn report_to_stdout(
    real: Res<Time<Real>>,
    scenario: Res<Scenario>,
    style: Res<SeparationStyle>,
    speed: Res<DemoSpeed>,
    overlaps: Res<Overlaps>,
    holds: Res<SeparationHolds>,
    counters: Res<RunCounters>,
    misses: Res<PathMisses>,
    mut next_report: Local<f32>,
) {
    let now = real.elapsed_secs();
    if now < *next_report {
        return;
    }
    *next_report = now + 2.0;
    println!(
        "{:<20} sep {:<3} {:>4.0}x  in view {:>4}  pairs {:>4}  involved {:>4}  held {:>4}  worst {:>6.3}  mean {:>6.3}  ticks/run {:>5.1}  path misses {:>4}",
        scenario.label(),
        if style.enabled { "on" } else { "off" },
        speed.0,
        overlaps.pawns,
        overlaps.pairs,
        overlaps.involved,
        holds.0.len(),
        overlaps.worst,
        overlaps.mean,
        counters.ticks_per_run,
        misses.0,
    );
}
