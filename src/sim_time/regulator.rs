//! Регулятор скорости: замер прогона `FixedUpdate` (`begin_sim_load` /
//! `end_sim_load`), предохранитель бюджета кадра, потолок по конвейеру поиска
//! пути и шаг к целевой скорости. Числа, которыми он параметризован, остаются
//! в родителе — блок констант в шапке `sim_time.rs` с задокументированным
//! порядком объявления.

use bevy::prelude::*;

use crate::census::Census;
use crate::determinism::SimTick;
use crate::telemetry::Telemetry;

use super::{
    ACTUAL_SPEED_WINDOW, MAX_SIM_SPEED, MIN_SIM_FPS, MIN_SIM_SPEED, SIM_FRAME_BUDGET_MS,
    SIM_FRAME_SHARE, SIM_RENDER_BUDGET, SIM_TICK_DEBT_CAP, SPEED_BACKOFF_TIME,
    SPEED_CLIMB_DOUBLE_TIME, SPEED_DEADBAND, SPEED_PROBE_TIME, SimLoad, SimSpeed, TickDebt,
    TickSegments,
};

/// Замер прогона `FixedUpdate`: время и число шагов за кадр. Здесь же
/// возвращается долг предохранителя — до того, как `run_fixed_main_schedule`
/// накопит дельту этого кадра, чтобы перенесённые тики встали в общий
/// накопитель и прошли обычным порядком.
pub(crate) fn begin_sim_load(
    mut load: ResMut<SimLoad>,
    mut segments: ResMut<TickSegments>,
    tick: Res<SimTick>,
    mut fixed: ResMut<Time<Fixed>>,
    mut debt: ResMut<TickDebt>,
    virtual_time: Res<Time<Virtual>>,
) {
    segments.reset();
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
pub(crate) fn guard_frame_budget(
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

pub(crate) fn end_sim_load(
    mut load: ResMut<SimLoad>,
    tick: Res<SimTick>,
    mut diagnostics: bevy::diagnostic::Diagnostics,
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
    load.frame_ticks = ticks;
    load.observe(sim_ms, ticks, real.delta_secs());
    let (tick_ms, wait_ms, peak_ms) = (load.tick_ms, load.wait_ms, load.wait_peak_ms);
    diagnostics.add_measurement(&crate::diagnostics::SIM_TICK_MS, || f64::from(tick_ms));
    diagnostics.add_measurement(&crate::diagnostics::SIM_WAIT_MS, || f64::from(wait_ms));
    diagnostics.add_measurement(&crate::diagnostics::SIM_WAIT_PEAK_MS, || f64::from(peak_ms));
}

/// Порог «тик-спайка», мс на шаг — из `QWE_PERF_FRAME_SPIKES=<ms>` (тот же
/// детектор, что у длины кадра в `dev.rs`): шаг дороже порога печатает строку
/// `tick spike: 41.1 ms for 96 ticks` ([`report_tick_spike`]). Секундные
/// средние `sim/tick_ms` всплеск размазывают, а он — самый честный след тех
/// самых «медленных тиков», которые регулятор потом разрезает на скорости;
/// порог держится именно на цене шага, потому что суммарный прогон кадра на
/// высокой скорости штатно уходит за любые разумные пороги.
///
/// Ресурс, а не переменная-однажды: порог виден и правится по BRP на живом
/// прогоне. Ноль (переменной нет) — лог выключен.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
pub(crate) struct TickSpikeThreshold(pub f32);

/// Репортёр тик-спайков: шаг дороже порога — строка в логе с состоянием мира
/// и разрезом по подсистемам. Отдельная система после [`end_sim_load`]: замер
/// не знает, как выглядит отчёт о нём. Счётчики состояния берёт из
/// [`Census`] — снимок прошлого `Update`, на кадр позже вспышки; для
/// контекстной строки достаточно. «Заявки, поданные в сам спайк-тик» — своим
/// запросом: это счёт тика, а не состояния мира.
#[allow(clippy::too_many_arguments)]
pub(crate) fn report_tick_spike(
    load: Res<SimLoad>,
    threshold: Res<TickSpikeThreshold>,
    tick: Res<SimTick>,
    census: Res<Census>,
    telemetry: Res<Telemetry>,
    segments: Res<TickSegments>,
    store: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    requests: Query<&crate::movement::RequestedAt>,
) {
    // порог нулевой — лог выключен (переменной `QWE_PERF_FRAME_SPIKES` нет);
    // порог — на **тик**, не на кадр: на высокой скорости кадр легитимно
    // заполнен шагами до бюджета (регулятор так и устроен), и «кадр дороже
    // порога» писал бы строку каждый кадр. Аномалия — дорогой шаг
    if threshold.0 <= 0.0 || load.frame_ticks == 0 {
        return;
    }
    if load.frame_sim_ms / load.frame_ticks as f32 <= threshold.0 {
        return;
    }
    let (sim_ms, ticks) = (load.frame_sim_ms, load.frame_ticks);
    // контекст волны: подсистемы тика на спайке остаются маленькими, и без
    // счётчиков состояние мира в момент вспышки не восстановить. Демоны в
    // строку не входят (их число всегда известно) — при переписи приходится
    // назвать поле, чтобы не оставить мёртвым весь срез
    let Census {
        humans,
        demons: _,
        corpses,
        souls,
        fleeing,
        wandering,
        moving,
        chasing,
        wave,
    } = *census;
    let killed = telemetry.killed;
    // заявки, поданные в сам спайк-тик: волна перепрокладок видна числом,
    // а не догадкой (panic/flee/демонский выбор цели/filed repath)
    let filed = requests.iter().filter(|r| r.0 == tick.0).count();
    info!(
        "tick spike: {sim_ms:.1} ms for {ticks} ticks (tick {}, humans {humans}, souls {souls}, corpses {corpses}, killed {killed}, fleeing {fleeing}, wandering {wandering}, moving {moving}, chasing {chasing}, wave {wave}, filed {filed})",
        tick.0
    );
    // разрез спайк-тика: последнее значение каждой подсистемы — это её
    // прогон на последнем (спайковом) тике, замеры через deferred-буфер
    // успевают доехать к AfterFixedMainLoop. Что не покрыто системами —
    // «rest»: синхронизации цепочки, применение команд, накладные
    // исполнителя
    let Some(store) = store.as_deref() else {
        return;
    };
    let mut covered = 0.0;
    let mut parts = Vec::new();
    for (name, path) in crate::diagnostics::SPIKE_PARTS {
        if let Some(v) = store.get(path).and_then(|d| d.values().last().copied()) {
            covered += v;
            parts.push(format!("{name} {v:.3}"));
        }
    }
    parts.push(format!("rest {:.3}", sim_ms - covered as f32));
    info!("tick spike dissect: {} ms total", parts.join(" | "));
    // сегменты цепочки: каждая проба держит стену от предыдущей до
    // себя, включая применения команд на стыках — всплеск применения
    // виден в сегменте того среза, где исполнитель его реально применил
    let segments = segments
        .parts
        .iter()
        .map(|(name, ms)| format!("{name} {ms:.1}"))
        .collect::<Vec<_>>()
        .join(" | ");
    info!("tick spike segments: {segments}");
}

/// Безопасное значение `requested`: запись по BRP и старый `settings.toml`
/// могут принести что угодно, а вход в мир копирует его в `Time<Virtual>`
/// напрямую — `set_relative_speed` паникует на отрицательном и нечисловом.
/// Отрицательное и NaN — ноль, «стоять»; плюс-бесконечность кламп съест в
/// потолок — «как можно быстрее» читается по-прежнему. Регулятор всё равно
/// приведёт эффективную скорость к полу [`MIN_SIM_SPEED`].
pub(crate) fn sane_requested(speed: f32) -> f32 {
    if speed.is_nan() {
        return 0.0;
    }
    speed.clamp(0.0, MAX_SIM_SPEED)
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
/// разобрано у [`MIN_SIM_FPS`] и
/// [`SIM_RENDER_BUDGET`]: под vsync
/// замер fps квантован и о запасе не говорит ничего, а `кадр − симуляция`
/// содержит ещё и сон. Обе величины годятся, чтобы показать пользователю, и не
/// годятся, чтобы на них замыкаться.
///
/// Применяется расчёт несимметрично: вниз — сразу, вверх — ползком
/// ([`SPEED_CLIMB_DOUBLE_TIME`]). Асимметрия не про «страховку»: вниз ведёт
/// сглаженный замер, то есть рывка там всё равно нет, а вверх нужен запас на
/// то, чего замер ещё не видит, — цена тика отстаёт от скорости.
pub(crate) fn throttle_speed_to_frame_budget(
    mut time: ResMut<Time<Virtual>>,
    mut speed: ResMut<SimSpeed>,
    load: Res<SimLoad>,
    fixed: Res<Time<Fixed>>,
    real: Res<Time<Real>>,
) {
    // Лесенка выше `MAX_SIM_SPEED` не поднимается, но `requested` пишут и
    // напрямую (по BRP) — режем здесь, чтобы потолок был один на все входы и
    // панель не показывала запрошенное число, которого не бывает.
    speed.requested = sane_requested(speed.requested);

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
pub(crate) fn affordable_speed(tick_ms: f32, hz: f32) -> f32 {
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
pub(crate) fn pipeline_limit(
    current: f32,
    load: &SimLoad,
    effective: f32,
    hz: f32,
    dt: f32,
) -> f32 {
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
/// длиннее, и яма кормит сама себя (упираясь только в
/// [`MAX_FRAME_DELTA`](super::MAX_FRAME_DELTA)).
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
pub(crate) fn frame_overrun(sim_ms: f32) -> f32 {
    let target_ms = 1000.0 / MIN_SIM_FPS;
    ((SIM_RENDER_BUDGET * 1000.0 + sim_ms.max(0.0)) / target_ms).max(1.0)
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
pub(crate) fn advance_speed(current: f32, target: f32, dt: f32, sim_ms: f32) -> f32 {
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
pub(crate) fn measure_actual_speed(
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
pub(crate) struct SpeedWindow {
    real: f32,
    virtual_elapsed: f32,
}
