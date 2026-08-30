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
use crate::loading::{PlayPhase, WorldStarted};
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
    /// Длительность прогона `FixedUpdate` в последнем кадре, мс — сырая, без
    /// сглаживания и без деления на шаги. Потребитель ровно один:
    /// [`frame_overrun`] решает по ней, симуляция ли растянула кадр.
    pub frame_sim_ms: f32,
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
            // новый прогон — часы и долг тиков с нуля (и по R, и по смене
            // города: оба пути триггерят `WorldStarted`)
            .add_observer(on_world_started)
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

/// Новый прогон начинается с чистых часов и чистого регулятора.
///
/// Долг тиков принадлежит прошлому миру: вливать его тики в свежепостроенный —
/// значит начать новый мир рывком. Замер нагрузки и бэкофф конвейера — тоже
/// его: после смены города унаследованный потолок насчитан вовсе про другой
/// мир, и новый стартовал бы придушенным без единого своего замера. Взять
/// старт слишком высоко ничем не грозит — вниз регулятор ходит мгновенно, а
/// кадр держит предохранитель бюджета ([`guard_frame_budget`]).
fn on_world_started(
    _event: On<WorldStarted>,
    mut clock: ResMut<SimClock>,
    mut debt: ResMut<TickDebt>,
    mut speed: ResMut<SimSpeed>,
    mut load: ResMut<SimLoad>,
    mut time: ResMut<Time<Virtual>>,
) {
    clock.restart(time.elapsed_secs_f64());
    debt.owed = 0.0;
    *load = SimLoad::default();
    speed.pipeline = MAX_SIM_SPEED;
    speed.affordable = MAX_SIM_SPEED;
    speed.effective = speed.requested;
    time.set_relative_speed(speed.effective);
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
    // до `observe`: тот выходит на кадре без единого шага, а кадр без шагов —
    // как раз тот случай, ради которого замер и заведён (см. `frame_overrun`)
    load.frame_sim_ms = sim_ms;
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
    speed.effective = advance_speed(speed.effective, target, dt, load.frame_sim_ms);

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
/// ответ снимается через `PATHFINDING_RETIRE_TICKS` **тиков** после
/// диспетчеризации заявки, готов он или нет; не готов — его дожидаются
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
/// расчёта, пока это длится, но **в меру своей вины**: поправка берётся в долю
/// от того, сколько кадра заняла сама симуляция.
///
/// Это не запас «на всякий случай», а разрыв усиления. Число шагов в кадре
/// задаёт длительность **предыдущего** кадра: просел один — он несёт больше
/// виртуального времени, значит больше шагов, значит следующий будет ещё
/// длиннее, и яма кормит сама себя (упираясь только в [`MAX_FRAME_DELTA`]).
/// Поправка держится ровно пока держатся длинные кадры и на равновесие не
/// влияет: в цель уложились — она равна единице.
///
/// Но петля эта существует ровно настолько, насколько кадр растянула
/// симуляция, и усиление у неё в точности равно её доле кадра: шаги следующего
/// кадра стоят `доля × длительность предыдущего`. Поэтому мерить перебор по
/// **длине кадра** — значит вменять симуляции чужую работу. Кадр, длинный не по
/// её вине, короче от замедления не станет, а цена ошибки замерена: на старте
/// мира первые кадры уходят в 100–270 мс на разовой работе (спавн 20 000 людей,
/// загрузка объединённых мешей города, первая компиляция GPU-пайплайнов), шаги
/// занимают в них проценты. Прежняя, безусловная поправка сажала скорость на пол
/// `MIN_SIM_SPEED` при `affordable` в 8–18x, и мир потом три секунды полз обратно
/// удвоениями — та самая просадка «0.1 → 0.5 → 1.0» сразу после старта.
///
/// Поэтому кадр пересобирается из того, за что симуляция отвечает: её
/// собственный прогон плюс отведённый не-симуляции [`SIM_RENDER_BUDGET`]. Если
/// такой кадр в цель укладывается, поправки нет, сколько бы ни занял настоящий;
/// если нет — она ровно та же, что дала бы длина кадра, целиком занятого
/// симуляцией. Заодно из ответа уходит квантование vsync: он больше не зависит
/// от того, какой кадр случился, — как и расчётная ветка регулятора.
///
/// Срабатывает эта поправка теперь редко, и так и задумано:
/// [`guard_frame_budget`] обрывает прогон на [`SIM_FRAME_BUDGET_MS`], то есть
/// разгон петли пресечён структурно, ещё до регулятора. Поправка остаётся
/// второй линией — на кадры, где прогон перевалил за бюджет вместе с
/// последним шагом.
fn frame_overrun(sim_ms: f32) -> f32 {
    let target_ms = 1000.0 / crate::settings::MIN_SIM_FPS;
    ((crate::settings::SIM_RENDER_BUDGET * 1000.0 + sim_ms.max(0.0)) / target_ms).max(1.0)
}

/// Шаг регулятора к целевой скорости: вниз сразу, вверх — с удвоением за
/// [`SPEED_CLIMB_DOUBLE_TIME`].
///
/// Мёртвая зона — только вниз, и это важнее, чем кажется. Ход регулятора
/// несимметричен: вниз мгновенно, вверх ползком, — поэтому шум замера тянул бы
/// скорость вниз храповиком, и полоса нужна ровно на этой стороне. Вверху она
/// не нужна: подъём и так ограничен темпом удвоения, шум через него не
/// пролезает. Симметричная же полоса стоила ровно того, что обещала: `target`
/// в 1.0 останавливал подъём на 0.98, и мир навсегда оставался на 2 % медленнее
/// запрошенного — до цели регулятор не доходил никогда.
fn advance_speed(current: f32, target: f32, dt: f32, sim_ms: f32) -> f32 {
    let target = (target / frame_overrun(sim_ms)).clamp(MIN_SIM_SPEED, MAX_SIM_SPEED);
    if target < current {
        if current - target <= current * SPEED_DEADBAND {
            return current;
        }
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
mod tests;
