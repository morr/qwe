//! Управление скоростью симуляции (порт лесенки скоростей из
//! `zxc/src/story_time`): Space — пауза, `=`/`-` — быстрее/медленнее.
//!
//! Запрошенная скорость и фактическая — разные величины: машина не всегда
//! тянет запрошенную, и тогда время автоматически замедляется до посильного
//! (см. `throttle_speed_to_frame_budget`). Сверху запрошенная упирается в
//! `MAX_SIM_SPEED`.

use bevy::app::RunFixedMainLoopSystems;
use bevy::prelude::*;

use crate::determinism::SimTick;
use crate::loading::PlayPhase;
use crate::restart::RestartEvent;
use crate::settings::{
    ACTUAL_SPEED_WINDOW, MAX_FRAME_DELTA, MAX_SIM_SPEED, MIN_SIM_SPEED, SIM_FRAME_BUDGET_MS,
    SIM_FRAME_SHARE, SIM_LOAD_PEAK_DECAY, SIM_LOAD_SMOOTHING, SIM_TICK_DEBT_CAP,
    SPEED_BACKOFF_TIME, SPEED_CLIMB_DOUBLE_TIME, SPEED_DEADBAND, SPEED_LADDER, SPEED_PROBE_TIME,
};

/// Скорость симуляции: `requested` крутит пользователь лесенкой, `affordable`
/// насчитал регулятор по замеру нагрузки, `effective` выставляется в
/// `Time<Virtual>` (это `min` первых двух, доведённый ограничителем разгона),
/// `actual` — то, что в итоге получилось (замер виртуального времени против
/// реального).
///
/// Расходятся все пять, и разводить их стоит именно так: `affordable`
/// отвечает на «почему стоим на 4x» замером, а не догадкой, а `effective` —
/// команда регулятора, а не факт. Bevy режет виртуальную дельту кадра по
/// `max_delta`, поэтому фриз или затык (например, пока фоново строится сетка
/// northstar) отнимает у симуляции время помимо регулятора — видно это только
/// в `actual`.
#[derive(Resource, Reflect, Debug)]
#[reflect(Resource)]
pub struct SimSpeed {
    pub requested: f32,
    /// Потолок по конвейеру поиска пути — единственная величина регулятора с
    /// памятью: она не считается заново каждый кадр, а отступает и пробует
    /// (см. [`pipeline_limit`]).
    pub pipeline: f32,
    pub affordable: f32,
    pub effective: f32,
    pub actual: f32,
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self {
            requested: 1.0,
            pipeline: MAX_SIM_SPEED,
            affordable: MAX_SIM_SPEED,
            effective: 1.0,
            actual: 1.0,
        }
    }
}

impl SimSpeed {
    /// Замедлено ли время против запрошенного (с запасом на дребезг замера).
    pub fn is_throttled(&self) -> bool {
        self.actual < self.requested * 0.95
    }
}

/// Часы симуляции: сколько виртуального времени прожил текущий мир.
///
/// Отсчёт идёт от входа в `PlayPhase::Live`, а не от старта приложения:
/// загрузка карты и прогрев проходят в реальном времени и к моменту симуляции
/// отношения не имеют. Смена города перезапускает мир, значит и часы.
///
/// Время виртуальное — стоит на паузе и бежит быстрее на ускорении. Это
/// «сколько прожил мир», а не сколько просидел за ним игрок.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
pub struct SimClock {
    /// `Time<Virtual>::elapsed` на момент входа в `Live`.
    started_at: f64,
    /// Прошедшее время симуляции, сек.
    pub elapsed: f64,
}

impl SimClock {
    /// Часы нового мира с нуля: за точку отсчёта берём текущее виртуальное
    /// время, а не обнуляем `Time<Virtual>` — тот общий, и его сброс сдвинул бы
    /// всем таймерам их дедлайны.
    pub fn restart(&mut self, now: f64) {
        self.started_at = now;
        self.elapsed = 0.0;
    }
}

/// Замер нагрузки симуляции: во сколько обходится один шаг `FixedUpdate`.
///
/// Цена тика — свойство мира (сколько в нём пешек, насколько они заняты), а
/// **не** скорости: скорость меняет число шагов за кадр, а не стоимость
/// каждого. Именно поэтому по ней можно решать уравнение вперёд, вместо того
/// чтобы искать посильную скорость на ощупь обратной связью по fps.
#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct SimLoad {
    /// Начало прогона `FixedUpdate` в текущем кадре.
    #[reflect(ignore)]
    started: Option<std::time::Instant>,
    /// `SimTick` на начало прогона — разница и есть число шагов за кадр.
    tick_at_start: u64,
    /// Работа и простой этого кадра, которые к **шагу** отношения не имеют:
    /// ожидание в `block_on` над поиском пути и системы, гейтящиеся «не чаще
    /// раза в кадр». Копится по ходу прогона, мс.
    frame_extra_ms: f32,
    /// Сглаженная цена одного шага **по процессору**, мс — без покадрового.
    pub tick_ms: f32,
    /// То же покадровое, разнесённое на шаг, мс.
    pub wait_ms: f32,
    /// Пиковое ожидание на шаг, мс: вверх — мгновенно, вниз — спадом за
    /// [`SIM_LOAD_PEAK_DECAY`] к среднему. Потолок по конвейеру держится по
    /// этой величине, а не по [`Self::wait_ms`]: всплески приходят пачками,
    /// среднее размывает их раньше, чем регулятор успевает ответить.
    pub wait_peak_ms: f32,
}

impl SimLoad {
    /// Прибавить время, потраченное в кадре, но не в шаге: простой в
    /// `block_on` над поиском пути (`movement::apply_pathfinding_results`) и
    /// покадровые системы внутри `FixedUpdate` (`movement::separation`).
    ///
    /// Делить такое на число шагов нельзя, и это не мелочь учёта: шагов в кадре
    /// тем больше, чем длиннее кадр, так что покадровая работа, размазанная по
    /// шагам, выглядела бы **дешевеющей** при просадке — регулятор разрешал бы
    /// больше, кадр становился бы длиннее, и так по кругу.
    pub fn add_frame_cost(&mut self, spent: std::time::Duration) {
        self.frame_extra_ms += spent.as_secs_f32() * 1000.0;
    }

    /// Учесть кадр: `sim_ms` — время его прогона `FixedUpdate`, `ticks` —
    /// сколько шагов в нём прошло, `dt` — реальная длительность кадра
    /// (она же шаг сглаживания).
    ///
    /// Ожидание вычитается, и это не мелочь учёта. Цена по процессору —
    /// свойство мира и от скорости не зависит; ожидание зависит от неё прямо:
    /// срок ответа отмерен в **тиках** (`PATHFINDING_RETIRE_TICKS`), так что
    /// чем быстрее идут тики, тем меньше у пула реального времени на ту же
    /// работу и тем дольше главный поток стоит. Смешать их в одно число —
    /// значит замкнуть регулятор на величину, которой он сам управляет: замер
    /// живьём показал ход цены тика 2.9…7.6 мс с периодом 2–4 с и скорость,
    /// ходившую за ним 1.3…3.4x.
    fn observe(&mut self, sim_ms: f32, ticks: u64, dt: f32) {
        let wait_ms = std::mem::take(&mut self.frame_extra_ms);
        if ticks == 0 {
            return;
        }
        let cpu = (sim_ms - wait_ms).max(0.0) / ticks as f32;
        let wait = wait_ms / ticks as f32;
        // первый замер кладётся целиком, а не подмешивается к нулю: ноль
        // означает «нагрузка неизвестна», а неизвестная нагрузка снимает
        // ограничение со скорости — пока фильтр полз бы от нуля, регулятор
        // успевал бы разогнаться мимо посильного и словить провал сразу после
        if self.tick_ms <= 0.0 {
            self.tick_ms = cpu;
            self.wait_ms = wait;
            self.wait_peak_ms = wait;
            return;
        }
        // сглаживание по реальному времени, а не по кадрам: иначе постоянная
        // фильтра меняется вместе с частотой кадров — то есть ровно тогда,
        // когда регулятор нужен больше всего
        let alpha = 1.0 - (-dt / SIM_LOAD_SMOOTHING).exp();
        self.tick_ms += (cpu - self.tick_ms) * alpha;
        self.wait_ms += (wait - self.wait_ms) * alpha;
        // пик несимметричен нарочно: сырой замер поднимает его без сглаживания,
        // а спадает он к среднему за свою, много более длинную постоянную
        let decay = 1.0 - (-dt / SIM_LOAD_PEAK_DECAY).exp();
        self.wait_peak_ms =
            (self.wait_peak_ms + (self.wait_ms - self.wait_peak_ms) * decay).max(wait);
    }
}

/// Долг тиков предохранителя кадра: виртуальное время, снятое с накопителя
/// `Time<Fixed>` посреди прогона, чтобы кадр не ушёл за бюджет
/// ([`SIM_FRAME_BUDGET_MS`]). Возвращается в накопитель в начале следующего
/// кадра — тики не теряются, а переезжают; логика мира этого не видит,
/// раскладка тиков по кадрам и так плавает вместе с длиной кадра.
///
/// `deferred` — счётчик всего перенесённого за сессию, только для телеметрии:
/// живой проверке нужно видеть, что предохранитель вообще срабатывает.
#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct TickDebt {
    owed: f32,
    pub deferred: f64,
}

pub struct SimTimePlugin;

impl Plugin for SimTimePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SimSpeed>()
            .register_type::<SimClock>()
            .register_type::<SimLoad>()
            .register_type::<TickDebt>()
            .init_resource::<SimSpeed>()
            .init_resource::<SimClock>()
            .init_resource::<SimLoad>()
            .init_resource::<TickDebt>()
            .add_systems(
                RunFixedMainLoop,
                (
                    begin_sim_load.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
                    end_sim_load.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
                ),
            )
            // предохранитель — первым в тике: решение «оборвать прогон»
            // принимается до того, как очередной тик начнёт тратить время
            .add_systems(
                FixedUpdate,
                guard_frame_budget.before(crate::spatial::SimSet::SpatialRebuild),
            )
            .add_systems(Startup, pin_max_delta)
            // рестарт по R отстраивает мир заново — часам тоже начинать с нуля
            .add_observer(restart_sim_clock)
            // прогрев идёт на паузе: мир уже собран, но за экраном загрузки
            // ему двигаться незачем — пусть пешки сначала получат пути
            .add_systems(OnEnter(PlayPhase::Warmup), pause_simulation)
            .add_systems(
                OnEnter(PlayPhase::Live),
                (resume_simulation, start_sim_clock),
            )
            .add_systems(
                Update,
                (
                    // пробел, `-` и `=` — символы, пока курсор в поле ввода
                    modify_time.run_if(not(crate::ui::typing_in_text_input)),
                    throttle_speed_to_frame_budget,
                    measure_actual_speed,
                    tick_sim_clock.run_if(in_state(PlayPhase::Live)),
                )
                    .chain(),
            );
    }
}

/// Самый длинный честный кадр — константа, и от скорости симуляции она не
/// зависит (см. [`MAX_FRAME_DELTA`]). Прибита явно, чтобы молчаливая смена
/// дефолта Bevy не поменяла поведение на фризах.
fn pin_max_delta(mut time: ResMut<Time<Virtual>>) {
    time.set_max_delta(std::time::Duration::from_secs_f32(MAX_FRAME_DELTA));
}

/// Пауза на время прогрева. Заявки на путь при этом идут: их подача и
/// диспетчеризация живут в `Update`, а стоит только `FixedUpdate`.
///
/// В **детерминированном** режиме заявки на паузе не идут: там весь конвейер —
/// и `pick_wander_targets`, и диспетчер, и приёмка — живёт в `FixedUpdate`.
/// Пешечного прогрева в этом режиме поэтому нет вовсе, см.
/// `loading::poll_warmup`; паузу это не отменяет — мир за экраном загрузки не
/// двигается ни в одном из режимов.
fn pause_simulation(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

fn resume_simulation(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

/// Долг тиков сбрасывается вместе с часами: он принадлежит прошлому миру,
/// и вливать его тики в свежепостроенный — значит начать новый мир рывком.
fn start_sim_clock(
    mut clock: ResMut<SimClock>,
    mut debt: ResMut<TickDebt>,
    time: Res<Time<Virtual>>,
) {
    clock.restart(time.elapsed_secs_f64());
    debt.owed = 0.0;
}

fn restart_sim_clock(
    _event: On<RestartEvent>,
    mut clock: ResMut<SimClock>,
    mut debt: ResMut<TickDebt>,
    time: Res<Time<Virtual>>,
) {
    clock.restart(time.elapsed_secs_f64());
    debt.owed = 0.0;
}

/// На паузе виртуальная дельта нулевая, поэтому часы сами стоят.
fn tick_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time<Virtual>>) {
    clock.elapsed = time.elapsed_secs_f64() - clock.started_at;
}

fn modify_time(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        toggle_pause(&mut time);
    }
    if keys.just_pressed(KeyCode::Equal) {
        speed.requested = next_time_scale(speed.requested);
    }
    if keys.just_pressed(KeyCode::Minus) {
        speed.requested = previous_time_scale(speed.requested);
    }
}

/// Замер прогона `FixedUpdate`: время и число шагов за кадр. Здесь же
/// возвращается долг предохранителя — до того, как `run_fixed_main_schedule`
/// накопит дельту этого кадра, чтобы перенесённые тики встали в общий
/// накопитель и прошли обычным порядком.
fn begin_sim_load(
    mut load: ResMut<SimLoad>,
    tick: Res<SimTick>,
    mut fixed: ResMut<Time<Fixed>>,
    mut debt: ResMut<TickDebt>,
    virtual_time: Res<Time<Virtual>>,
) {
    // на паузе долг придерживается: вернуть его в накопитель — значит дать
    // тикам идти под паузой, у стоящего мира долгов нет
    let owed = if virtual_time.is_paused() {
        0.0
    } else {
        std::mem::take(&mut debt.owed)
    };
    if owed > 0.0 {
        // всё сверх потолка выбрасывается: мир, не тянущий бюджет даже на
        // минимальной скорости, копил бы долг бесконечно и навсегда отстал бы
        // от виртуальных часов — а `max_delta` уже решил, что безнадёжно
        // отставшее не досчитывают
        let returned = owed.min(SIM_TICK_DEBT_CAP);
        fixed.accumulate_overstep(std::time::Duration::from_secs_f32(returned));
    }
    load.started = Some(std::time::Instant::now());
    load.tick_at_start = tick.0;
}

/// Предохранитель кадра: как только прогон `FixedUpdate` выел бюджет
/// ([`SIM_FRAME_BUDGET_MS`]), оставшийся накопитель снимается и переезжает в
/// долг следующему кадру ([`TickDebt`]).
///
/// Регулятор целит скорость в этот же бюджет, но по **сглаженному** замеру, а
/// всплеск (пачка заявок на путь, вставшая в `block_on`) приходит раньше, чем
/// замер о нём узнаёт. Без предохранителя такой кадр обязан дотянуть все
/// накопленные тики — и растягивается на глубину всплеска; с ним — обрывается
/// на бюджете, и всплеск виден как временная просадка `actual`, а не как рывок
/// картинки. Проверка стоит один `Instant::elapsed` на тик.
fn guard_frame_budget(
    load: Res<SimLoad>,
    mut fixed: ResMut<Time<Fixed>>,
    mut debt: ResMut<TickDebt>,
) {
    let Some(started) = load.started else {
        return;
    };
    if started.elapsed().as_secs_f32() * 1000.0 < SIM_FRAME_BUDGET_MS {
        return;
    }
    let rest = fixed.overstep();
    if rest.is_zero() {
        return;
    }
    fixed.discard_overstep(rest);
    debt.owed += rest.as_secs_f32();
    debt.deferred += rest.as_secs_f64();
}

fn end_sim_load(
    mut load: ResMut<SimLoad>,
    mut diagnostics: bevy::diagnostic::Diagnostics,
    tick: Res<SimTick>,
    real: Res<Time<Real>>,
) {
    let Some(started) = load.started.take() else {
        return;
    };
    // `saturating_sub` — не педантизм: `SimTick` обнуляется рестартом и сменой
    // города, и без него уехавший назад счётчик дал бы кадр с абсурдным числом
    // шагов, а тот — абсурдную цену тика
    let ticks = tick.0.saturating_sub(load.tick_at_start);
    let sim_ms = started.elapsed().as_secs_f32() * 1000.0;
    load.observe(sim_ms, ticks, real.delta_secs());
    let (tick_ms, wait_ms, peak_ms) = (load.tick_ms, load.wait_ms, load.wait_peak_ms);
    diagnostics.add_measurement(&crate::diagnostics::SIM_TICK_MS, || f64::from(tick_ms));
    diagnostics.add_measurement(&crate::diagnostics::SIM_WAIT_MS, || f64::from(wait_ms));
    diagnostics.add_measurement(&crate::diagnostics::SIM_WAIT_PEAK_MS, || f64::from(peak_ms));
}

/// Авто-замедление: скорость назначается **расчётом** по замеренной цене
/// шага, а не подкруткой по обратной связи с fps.
///
/// Симуляция на скорости `S` требует `S × 64` шагов на реальную секунду, то
/// есть `кадр × S × 64` шагов на кадр. Цена одного шага замерена
/// ([`SimLoad`]) и от скорости не зависит. Разрешив симуляции занимать
/// [`SIM_FRAME_SHARE`] кадра, получаем `S` одним делением — см.
/// [`affordable_speed`].
///
/// Почему не обратная связь по fps, хотя цель формулируется именно в кадрах,
/// разобрано у [`MIN_SIM_FPS`](crate::settings::MIN_SIM_FPS) и
/// [`SIM_RENDER_BUDGET`](crate::settings::SIM_RENDER_BUDGET): под vsync
/// замер fps квантован и о запасе не говорит ничего, а `кадр − симуляция`
/// содержит ещё и сон. Обе величины годятся, чтобы показать пользователю, и не
/// годятся, чтобы на них замыкаться.
///
/// Применяется расчёт несимметрично: вниз — сразу, вверх — ползком
/// ([`SPEED_CLIMB_DOUBLE_TIME`]). Асимметрия не про «страховку»: вниз ведёт
/// сглаженный замер, то есть рывка там всё равно нет, а вверх нужен запас на
/// то, чего замер ещё не видит, — цена тика отстаёт от скорости.
fn throttle_speed_to_frame_budget(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    load: Res<SimLoad>,
    fixed: Res<Time<Fixed>>,
    real: Res<Time<Real>>,
) {
    // Лесенка выше `MAX_SIM_SPEED` не поднимается, но `requested` пишут и
    // напрямую (по BRP) — режем здесь, чтобы потолок был один на все входы и
    // панель не показывала запрошенное число, которого не бывает.
    if speed.requested > MAX_SIM_SPEED {
        speed.requested = MAX_SIM_SPEED;
    }

    let hz = 1.0 / fixed.timestep().as_secs_f32();
    let dt = real.delta_secs();
    speed.pipeline = pipeline_limit(speed.pipeline, &load, speed.effective, hz, dt);
    speed.affordable = affordable_speed(load.tick_ms, hz).min(speed.pipeline);
    let target = speed.affordable.min(speed.requested);
    speed.effective = advance_speed(speed.effective, target, dt);

    if time.relative_speed() != speed.effective {
        time.set_relative_speed(speed.effective);
    }
}

/// Скорость, при которой шаги симуляции занимают [`SIM_FRAME_SHARE`] кадра.
///
/// Ограничений два, независимых, и берётся меньшее.
///
/// **По процессору.** Кадр длительности `d` несёт `d × S × hz` шагов ценой
/// `tick_ms` каждый; требуем `d × S × hz × tick_ms ≤ доля × d × 1000` — `d`
/// сокращается, и от него (а значит и от квантования vsync, и от того, какой
/// кадр случился прошлым) ответ не зависит вовсе. Отсюда же устойчивость: раз
/// симуляция занимает фиксированную долю **любого** кадра, длительность кадра
/// сходится к `остальное / (1 − доля)` — сжатие с коэффициентом `доля < 1`, а
/// не петля с усилением, которую надо гасить гистерезисом.
///
/// **По конвейеру поиска пути** — вторым, отдельным ограничением
/// ([`SimSpeed::pipeline`]), и уже не расчётом. В детерминированном режиме
/// ответ снимается через `PATHFINDING_RETIRE_TICKS` **тиков** после заявки,
/// готов он или нет; не готов — его дожидаются
/// (`movement::apply_pathfinding_results`). Срок отмерен в тиках, а работа
/// делается в реальном времени, так что чем быстрее идут тики, тем меньше
/// пулу достаётся секунд на ту же работу.
///
/// Решать это в лоб — как сделано выше для процессора — нельзя, и это
/// проверено замером: пул поиска пути есть очередь, и вблизи насыщения
/// ожидание растёт как `1/(1 − загрузка)`. Усиление там сколь угодно велико,
/// одношаговая формула перелетает, и живьём это видно как ожидание, ходившее
/// 0.9…8.0 мс на шаг с периодом около секунды при совершенно ровной цене по
/// процессору (1.4…2.3 мс). Поэтому здесь интегральный ход с медленным
/// шагом — тот же приём, которым лечат ровно ту же беду в управлении
/// перегрузкой сети.
fn affordable_speed(tick_ms: f32, hz: f32) -> f32 {
    if tick_ms <= 0.0 {
        return MAX_SIM_SPEED;
    }
    (1000.0 * SIM_FRAME_SHARE / (hz * tick_ms)).clamp(MIN_SIM_SPEED, MAX_SIM_SPEED)
}

/// Новый потолок по конвейеру: пока главный поток занят больше отведённой доли
/// — отступаем, пока укладывается — пробуем прибавить.
///
/// Занятость считается на один шаг: работа плюс ожидание против
/// `доля × 1000 / (hz × S)` — столько миллисекунд приходится на шаг, если
/// симуляции отдана её доля кадра. Критерий тот же самый, что и у расчётной
/// ветки, просто применённый к тому, что расчёту не поддаётся.
///
/// Шаг **пропорционален перебору**, а не постоянен: на 10 % сверх доли и режем
/// на 10 % в секунду, вдвое сверх — вдвое. Постоянный шаг проверен и отвергнут
/// замером: отступление режет всё время, пока разгребается очередь, то есть
/// заведомо переваливает за нужное, и получается своя пила — скорость ходила
/// 1.1…1.8x с периодом около двух секунд.
///
/// Проба вверх — та же величина, но по своему, много более медленному
/// времени: переполненная очередь разгребается, только если перестать в неё
/// лить, а лишняя осторожность стоит лишь секунд разгона. Отношение зажато
/// вдвое в обе стороны, чтобы одиночный выброс замера не двигал потолок на
/// порядок.
///
/// Занятость берётся по **пиковому** ожиданию ([`SimLoad::wait_peak_ms`]), не
/// по среднему. Средним всплеск размывается раньше, чем интегральный ход
/// успевает ответить, и каждая пачка заявок оставляла свой зуб на графике
/// кадров; по пику скорость стоит ниже — с запасом под всплеск, который уже
/// случался недавно. Сознательный размен скорости на ровность.
fn pipeline_limit(current: f32, load: &SimLoad, effective: f32, hz: f32, dt: f32) -> f32 {
    let busy = load.tick_ms + load.wait_peak_ms;
    let allowed = 1000.0 * SIM_FRAME_SHARE / (hz * effective.max(MIN_SIM_SPEED));
    if busy <= 0.0 {
        return current;
    }
    let ratio = (allowed / busy).clamp(0.5, 2.0);
    let time = if ratio < 1.0 {
        SPEED_BACKOFF_TIME
    } else {
        SPEED_PROBE_TIME
    };
    (current * ratio.powf(dt / time)).clamp(MIN_SIM_SPEED, MAX_SIM_SPEED)
}

/// Насколько кадр вышел длиннее целевого — на столько же режем скорость сверх
/// расчёта, пока это длится.
///
/// Это не запас «на всякий случай», а разрыв усиления. Число шагов в кадре
/// задаёт длительность **предыдущего** кадра: просел один — он несёт больше
/// виртуального времени, значит больше шагов, значит следующий будет ещё
/// длиннее, и яма кормит сама себя (упираясь только в [`MAX_FRAME_DELTA`]).
/// Поправка держится ровно пока держатся длинные кадры и на равновесие не
/// влияет: в цель уложились — она равна единице.
///
/// Замеренную длительность берём как есть, вместе с квантованием vsync: ниже
/// цели квантование только грубит поправку, а грубая поправка на просадке
/// лучше точной.
fn frame_overrun(dt: f32) -> f32 {
    (dt * crate::settings::MIN_SIM_FPS).max(1.0)
}

/// Шаг регулятора к целевой скорости: вниз сразу, вверх — с удвоением за
/// [`SPEED_CLIMB_DOUBLE_TIME`].
///
/// Мёртвая зона симметрична, и это важнее, чем кажется: с ней срабатывает
/// только вниз, шум замера тянул бы скорость вниз храповиком — вниз мгновенно,
/// вверх ползком.
fn advance_speed(current: f32, target: f32, dt: f32) -> f32 {
    let target = (target / frame_overrun(dt)).clamp(MIN_SIM_SPEED, MAX_SIM_SPEED);
    if (target - current).abs() <= current * SPEED_DEADBAND {
        return current;
    }
    if target < current {
        return target;
    }
    (current * (dt / SPEED_CLIMB_DOUBLE_TIME).exp2()).min(target)
}

/// Замер фактической скорости: сколько виртуального времени набежало на
/// секунду реального.
///
/// Считается по окну реального времени (`ACTUAL_SPEED_WINDOW`), а не по
/// кадрам: просадка — это как раз несколько длинных кадров, и в среднем
/// по кадрам они весят столько же, сколько быстрые, то есть теряются.
/// Окно ловит и то, чего не знает регулятор, — обрезку дельты по `max_delta`.
fn measure_actual_speed(
    virtual_time: Res<Time<Virtual>>,
    real_time: Res<Time<Real>>,
    mut speed: ResMut<SimSpeed>,
    mut window: Local<SpeedWindow>,
) {
    // на паузе мерить нечего, и накопленное окно к моменту снятия паузы
    // протухнет — сбрасываем
    if virtual_time.is_paused() {
        *window = SpeedWindow::default();
        return;
    }

    window.real += real_time.delta_secs();
    window.virtual_elapsed += virtual_time.delta_secs();
    if window.real < ACTUAL_SPEED_WINDOW {
        return;
    }

    speed.actual = window.virtual_elapsed / window.real;
    *window = SpeedWindow::default();
}

/// Накопитель окна замера фактической скорости.
#[derive(Default)]
struct SpeedWindow {
    real: f32,
    virtual_elapsed: f32,
}

pub fn toggle_pause(time: &mut Time<Virtual>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
}

/// Ступень для кнопки Speed: та же лесенка, но по кругу — с верхней ступени
/// возвращаемся к 1x. Кнопка одна, и без замыкания сверху было бы не
/// выбраться иначе как хоткеем.
pub fn cycle_time_scale(speed: f32) -> f32 {
    if speed >= MAX_SIM_SPEED {
        1.0
    } else {
        next_time_scale(speed)
    }
}

/// Следующая ступень лесенки: первая строго выше текущей скорости, с верхней
/// ступени — остаёмся на ней. Произвольное значение (по BRP `requested` пишут
/// любым) прижимается к ближайшей ступени сверху.
pub fn next_time_scale(speed: f32) -> f32 {
    SPEED_LADDER
        .into_iter()
        .find(|&step| step > speed)
        .unwrap_or(MAX_SIM_SPEED)
}

/// Предыдущая ступень лесенки: последняя строго ниже текущей скорости,
/// с нижней ступени — остаёмся на ней.
pub fn previous_time_scale(speed: f32) -> f32 {
    SPEED_LADDER
        .into_iter()
        .rev()
        .find(|&step| step < speed)
        .unwrap_or(SPEED_LADDER[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::settings::{MIN_SIM_FPS, SIM_RENDER_BUDGET};

    /// Шаг фиксированного расписания, как его держит `Time<Fixed>`.
    const HZ: f32 = 64.0;

    /// Нагрузка без простоя: только цена шага по процессору.
    fn cpu_only(tick_ms: f32) -> SimLoad {
        SimLoad {
            tick_ms,
            ..SimLoad::default()
        }
    }

    /// Подстановка ответа обратно в задачу: на посильной скорости шаги
    /// симуляции обязаны занять ровно отведённую им долю кадра.
    #[test]
    fn affordable_speed_solves_the_frame_budget() {
        for tick_ms in [0.05f32, 0.25, 1.0, 4.0] {
            let speed = affordable_speed(tick_ms, HZ);
            if speed >= MAX_SIM_SPEED {
                continue; // упёрлись в потолок, уравнение тут ни при чём
            }
            let frame = 1.0 / MIN_SIM_FPS;
            let sim_ms = frame * speed * HZ * tick_ms;
            assert!(
                (sim_ms - frame * 1000.0 * SIM_FRAME_SHARE).abs() < 1e-3,
                "на {speed}x шаги заняли {sim_ms} мс вместо доли кадра"
            );
            // и остаток кадра — ровно бюджет не-симуляции
            assert!((frame * 1000.0 - sim_ms - SIM_RENDER_BUDGET * 1000.0).abs() < 1e-2);
        }
    }

    /// Пока цена шага не замерена (пауза, загрузка, кадр без единого шага),
    /// ограничивать нечем — регулятор не имеет права выдумывать ограничение.
    #[test]
    fn an_unmeasured_simulation_allows_the_full_request() {
        assert_eq!(affordable_speed(0.0, HZ), MAX_SIM_SPEED);
        assert_eq!(affordable_speed(-1.0, HZ), MAX_SIM_SPEED);
        // дешёвый мир — тоже потолок, а не «сколько получится»
        assert_eq!(affordable_speed(0.001, HZ), MAX_SIM_SPEED);
    }

    /// Ожидание конвейера — второе, независимое ограничение: работы по
    /// процессору на копейку, а скорость всё равно упирается, потому что срок
    /// ответа отмерен в тиках и на быстрых тиках пул не успевает.
    ///
    /// Ход интегральный, поэтому и проверяется как ход: под простоем потолок
    /// обязан идти вниз, без простоя — обратно вверх, и оба конца зажаты.
    #[test]
    fn the_pathfinding_pipeline_caps_the_speed_on_its_own() {
        let dt = 1.0 / 60.0;
        // при 2x на шаг отведено 1000 × доля / (64 × 2) ≈ 4.77 мс
        let allowed = 1000.0 * SIM_FRAME_SHARE / (HZ * 2.0);
        let steps = (60.0 * SPEED_BACKOFF_TIME) as usize;

        // занят ровно вдвое сверх отведённого — за время отработки ровно вдвое
        // и срезали. Потолок держится по пику ожидания, не по среднему
        let waiting = SimLoad {
            tick_ms: 0.1,
            wait_ms: 2.0 * allowed - 0.1,
            wait_peak_ms: 2.0 * allowed - 0.1,
            ..SimLoad::default()
        };
        let mut cap = MAX_SIM_SPEED;
        for _ in 0..steps {
            cap = pipeline_limit(cap, &waiting, 2.0, HZ, dt);
        }
        assert!(
            (cap - MAX_SIM_SPEED / 2.0).abs() < 0.2,
            "на двойном переборе за {SPEED_BACKOFF_TIME} с ушли в {cap}x"
        );

        // а на переборе в 10 % — на те же 10 %. Ради этого шаг и сделан
        // пропорциональным: постоянный резал бы вдвое в обоих случаях, всё
        // время, пока разгребается очередь, — и заводил собственную пилу
        let slight = SimLoad {
            tick_ms: 0.1,
            wait_ms: allowed * 1.1 - 0.1,
            wait_peak_ms: allowed * 1.1 - 0.1,
            ..SimLoad::default()
        };
        let mut gentle = MAX_SIM_SPEED;
        for _ in 0..steps {
            gentle = pipeline_limit(gentle, &slight, 2.0, HZ, dt);
        }
        assert!(
            (gentle - MAX_SIM_SPEED / 1.1).abs() < 0.2,
            "на переборе 10 % срезали до {gentle}x"
        );

        // простой ушёл — потолок возвращается
        let calm = cpu_only(0.1);
        let climbed = pipeline_limit(cap, &calm, 2.0, HZ, dt);
        assert!(climbed > cap, "без простоя потолок не растёт: {climbed}");

        // и оба конца зажаты
        assert_eq!(
            pipeline_limit(MAX_SIM_SPEED, &calm, 2.0, HZ, 10.0),
            MAX_SIM_SPEED
        );
        assert_eq!(
            pipeline_limit(MIN_SIM_SPEED, &waiting, 2.0, HZ, 10.0),
            MIN_SIM_SPEED
        );
    }

    /// Пик ожидания несимметричен: всплеск поднимает его мгновенно и целиком,
    /// а отпускает он медленно — иначе пачка заявок, размытая средним,
    /// проходила бы мимо регулятора, и каждый всплеск оставлял бы свой зуб.
    #[test]
    fn a_wait_burst_raises_the_peak_at_once_and_lets_go_slowly() {
        let dt = 1.0 / 60.0;
        let mut load = SimLoad::default();
        // ровная жизнь: 2 мс работы и 0.5 мс простоя на шаг — пик сидит
        // рядом со средним
        for _ in 0..240 {
            load.frame_extra_ms = 5.0;
            load.observe(25.0, 10, dt);
        }
        let calm_peak = load.wait_peak_ms;
        assert!(calm_peak < 1.0, "на ровной жизни пик {calm_peak}");

        // одиночный всплеск: 8 мс ожидания на шаг — пик берёт его целиком
        load.frame_extra_ms = 80.0;
        load.observe(100.0, 10, dt);
        assert!(
            (load.wait_peak_ms - 8.0).abs() < 1e-3,
            "пик после всплеска {} вместо 8",
            load.wait_peak_ms
        );
        // а среднее — нет: оно и должно отставать
        assert!(load.wait_ms < 2.0, "среднее прыгнуло до {}", load.wait_ms);

        // секунду спустя пик ещё помнит всплеск, хотя жизнь снова ровная
        for _ in 0..60 {
            load.frame_extra_ms = 5.0;
            load.observe(25.0, 10, dt);
        }
        assert!(
            load.wait_peak_ms > calm_peak + 1.0,
            "пик забыл всплеск за секунду: {}",
            load.wait_peak_ms
        );
        // но не вечно: за несколько постоянных спада возвращается к среднему
        for _ in 0..(60.0 * SIM_LOAD_PEAK_DECAY * 5.0) as usize {
            load.frame_extra_ms = 5.0;
            load.observe(25.0, 10, dt);
        }
        assert!(
            (load.wait_peak_ms - load.wait_ms).abs() < 0.1,
            "пик не вернулся к среднему: {} против {}",
            load.wait_peak_ms,
            load.wait_ms
        );
    }

    /// Просевший кадр режет скорость сверх расчёта — и ровно на своё
    /// отставание. Это разрыв усиления: шагов в кадре тем больше, чем длиннее
    /// был предыдущий, так что без поправки одна просадка тянет следующую.
    #[test]
    fn a_late_frame_cuts_the_speed_beyond_the_solver() {
        let target = 10.0;
        // кадр в цель уложился — поправки нет вовсе, ни на 60, ни ровно на 30
        for dt in [1.0 / 120.0, 1.0 / 60.0, 1.0 / MIN_SIM_FPS] {
            assert_eq!(frame_overrun(dt), 1.0, "кадр {dt} получил поправку");
            assert_eq!(advance_speed(target, target, dt), target);
        }
        // вдвое длиннее целевого — вдвое ниже скорость, и сразу, а не ползком
        let halved = advance_speed(target, target, 2.0 / MIN_SIM_FPS);
        assert!(
            (halved - target / 2.0).abs() < 1e-3,
            "на вдвое просевшем кадре скорость {halved}"
        );
        // и поправка временная: кадр вернулся в цель — цель снова прежняя
        assert!(advance_speed(halved, target, 1.0 / 60.0) > halved);
    }

    /// Ответ не зависит от длительности кадра — в этом весь смысл перехода от
    /// обратной связи по fps к расчёту: квантование vsync больше не влияет.
    #[test]
    fn the_answer_does_not_depend_on_the_frame_that_happened() {
        let speed = affordable_speed(2.0, HZ);
        for frame in [1.0 / 120.0, 1.0 / 60.0, 1.0 / 30.0, 1.0 / 7.0] {
            let sim_ms = frame * speed * HZ * 2.0;
            let share = sim_ms / (frame * 1000.0);
            assert!(
                (share - SIM_FRAME_SHARE).abs() < 1e-4,
                "на кадре {frame} доля симуляции {share}"
            );
        }
    }

    /// Регулятор обязан **прийти и встать**, а не бегать пилой. Модель
    /// адверсарная — прошлая была слишком доброй и потому ничего не поймала:
    ///
    /// * длительность кадра квантуется vsync (ступени 60 / 30 / 20 / 15);
    /// * нагрузка **отстаёт** от скорости: разгон порождает заявки на поиск
    ///   пути, их цена приходит через секунду-другую;
    /// * длинный кадр несёт больше виртуального времени, а значит и больше
    ///   шагов, — до клампа по `MAX_FRAME_DELTA`.
    #[test]
    fn the_governor_settles_on_a_ringing_plant() {
        let plant = Plant::default();
        let (tail, peak, sustainable) = plant.run(30.0, 4000);

        let low = tail.iter().copied().fold(f32::MAX, f32::min);
        let high = tail.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            high - low < 1.0,
            "регулятор не встал: кадры ходят {low}…{high}"
        );
        assert!(
            low >= MIN_SIM_FPS,
            "встали на {low} fps, ниже цели {MIN_SIM_FPS}"
        );
        // и это не «замерли на месте»: скорость выросла до посильной
        assert!(
            peak > 1.0 && (peak - sustainable).abs() < sustainable * 0.2,
            "скорость {peak} не похожа на посильные {sustainable}"
        );
    }

    /// Регрессия ровно на жалобу «ускорение оказывается сильнее чем нужно»:
    /// на пути к равновесию скорость не имеет права перелетать посильную —
    /// именно перелёт и ловил новый провал сразу после разгона.
    #[test]
    fn the_climb_never_overshoots_the_sustainable_speed() {
        let plant = Plant::default();
        let (_, peak, sustainable) = plant.run(30.0, 4000);
        assert!(
            peak <= sustainable * 1.1,
            "разогнались до {peak}x при посильных {sustainable}x"
        );
    }

    /// Запрошенное меньше посильного — регулятор просто не вмешивается.
    #[test]
    fn a_modest_request_is_left_alone() {
        let plant = Plant::default();
        let (tail, peak, _) = plant.run(1.0, 1200);
        assert!((peak - 1.0).abs() < 1e-3, "1x поехали как {peak}x");
        let low = tail.iter().copied().fold(f32::MAX, f32::min);
        assert!(low >= 59.9, "на 1x кадры просели до {low}");
    }

    /// Модель машины: цена шага, цена всего остального и запаздывание
    /// нагрузки. Числа подобраны под замеренное поведение — ~1.6x посильных
    /// на запрошенных 10x.
    struct Plant {
        /// Цена шага в ненагруженном мире, мс.
        base_tick_ms: f32,
        /// Во сколько раз дорожает шаг к `MAX_SIM_SPEED`: на скорости пул
        /// поиска пути забивается и отбирает ядра у главного потока.
        load_factor: f32,
        /// Постоянная запаздывания нагрузки, с.
        lag: f32,
        /// Всё, что в кадре не симуляция, с.
        render: f32,
        /// Интервал развёртки, с.
        vsync: f32,
    }

    impl Default for Plant {
        fn default() -> Self {
            Self {
                base_tick_ms: 3.0,
                load_factor: 2.0,
                lag: 1.0,
                render: 0.005,
                vsync: 1.0 / 60.0,
            }
        }
    }

    impl Plant {
        /// Прогнать `steps` кадров на запрошенной скорости. Возвращает хвост
        /// истории fps, максимум скорости за прогон и посильную скорость в
        /// равновесии.
        fn run(&self, requested: f32, steps: usize) -> (Vec<f32>, f32, f32) {
            let mut load = SimLoad::default();
            let mut speed = 1.0f32;
            let mut tick_ms = self.base_tick_ms;
            let mut frame = self.vsync;
            let mut peak = speed;
            let mut tail = Vec::new();

            for step in 0..steps {
                // шагов в кадре — по длительности предыдущего, как их
                // накапливает `Time<Fixed>` из виртуальной дельты
                let ticks = (frame.min(MAX_FRAME_DELTA) * speed * HZ) as u64;
                let sim = ticks as f32 * tick_ms / 1000.0;
                let previous = frame;
                frame = self.quantise(self.render + sim);

                load.observe(sim * 1000.0, ticks, previous);
                let target = affordable_speed(load.tick_ms, HZ).min(requested);
                speed = advance_speed(speed, target, previous);
                peak = peak.max(speed);

                // нагрузка догоняет скорость не сразу
                let settled = self.tick_ms_at(speed);
                tick_ms += (settled - tick_ms) * (1.0 - (-previous / self.lag).exp());

                if step >= steps - 200 {
                    tail.push(1.0 / frame);
                }
            }
            (tail, peak, self.sustainable(requested))
        }

        fn tick_ms_at(&self, speed: f32) -> f32 {
            self.base_tick_ms * (1.0 + (self.load_factor - 1.0) * speed / MAX_SIM_SPEED)
        }

        /// Vsync отдаёт кадр только на границе развёртки.
        fn quantise(&self, cost: f32) -> f32 {
            (cost / self.vsync).ceil().max(1.0) * self.vsync
        }

        /// Скорость, на которой шаги занимают ровно отведённую долю кадра при
        /// установившейся (уже подорожавшей) цене шага.
        fn sustainable(&self, requested: f32) -> f32 {
            let mut speed = requested;
            for _ in 0..200 {
                speed = affordable_speed(self.tick_ms_at(speed), HZ).min(requested);
            }
            speed
        }
    }
}
