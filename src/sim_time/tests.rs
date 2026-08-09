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

/// Прогон симуляции, который сам растягивает кадр до `dt`: всё, что не
/// отведено отрисовке, занято шагами.
fn sim_filling(dt: f32) -> f32 {
    (dt * 1000.0 - SIM_RENDER_BUDGET * 1000.0).max(0.0)
}

/// Просевший кадр режет скорость сверх расчёта — и ровно на своё
/// отставание. Это разрыв усиления: шагов в кадре тем больше, чем длиннее
/// был предыдущий, так что без поправки одна просадка тянет следующую.
#[test]
fn a_late_frame_cuts_the_speed_beyond_the_solver() {
    let target = 10.0;
    // кадр в цель уложился — поправки нет вовсе, ни на 60, ни ровно на 30
    for dt in [1.0 / 120.0, 1.0 / 60.0, 1.0 / MIN_SIM_FPS] {
        let sim_ms = sim_filling(dt);
        // на самой границе бюджета деление даёт 1.0000001 — сравниваем с
        // допуском, а не побитово
        let overrun = frame_overrun(sim_ms);
        assert!((overrun - 1.0).abs() < 1e-5, "кадр {dt} получил поправку");
        assert_eq!(advance_speed(target, target, dt, sim_ms), target);
    }
    // вдвое длиннее целевого — вдвое ниже скорость, и сразу, а не ползком
    let long = 2.0 / MIN_SIM_FPS;
    let halved = advance_speed(target, target, long, sim_filling(long));
    assert!(
        (halved - target / 2.0).abs() < 1e-3,
        "на вдвое просевшем кадре скорость {halved}"
    );
    // и поправка временная: кадр вернулся в цель — цель снова прежняя
    let short = 1.0 / 60.0;
    assert!(advance_speed(halved, target, short, sim_filling(short)) > halved);
}

/// Регрессия на просадку «0.1 → 0.5 → 1.0» в первые секунды мира: кадр,
/// растянутый **не** симуляцией, скорость резать не имеет права. Числа —
/// замеренные на старте: кадр 270 мс на разовой работе спавна и первой
/// отрисовки города, шаги в нём — 19 мс, семь процентов кадра.
#[test]
fn a_long_frame_the_simulation_did_not_cause_leaves_the_speed_alone() {
    let dt = 0.27;
    let sim_ms = 19.0;
    assert_eq!(
        frame_overrun(sim_ms),
        1.0,
        "чужой кадр урезал скорость в {} раз",
        frame_overrun(sim_ms)
    );
    // а поправка по длине кадра резала бы восьмикратно — вот цена вопроса
    assert!(dt * MIN_SIM_FPS > 8.0);
    // и на самой скорости это не сказывается: 1x остаётся 1x
    assert_eq!(advance_speed(1.0, 1.0, dt, sim_ms), 1.0);

    // но своя вина остаётся своей: тот же кадр, занятый шагами целиком,
    // режется ровно по его длине
    let guilty = frame_overrun(sim_filling(dt));
    assert!(
        (guilty - dt * MIN_SIM_FPS).abs() < 1e-3,
        "кадр по вине симуляции получил поправку {guilty}"
    );

    // граница: пока прогон укладывается в свой бюджет, поправки нет, за ним —
    // появляется. Бюджет и есть та черта, на которой обрывает `guard_frame_budget`
    assert!((frame_overrun(SIM_FRAME_BUDGET_MS) - 1.0).abs() < 1e-5);
    assert!(frame_overrun(SIM_FRAME_BUDGET_MS * 1.5) > 1.0);
}

/// Регулятор обязан **дойти** до запрошенного, а не встать в двух процентах
/// под ним: симметричная мёртвая зона глушила последний шаг подъёма, и мир
/// вечно шёл на 0.98x при запрошенном 1x.
#[test]
fn the_climb_reaches_the_request_exactly() {
    let dt = 1.0 / 60.0;
    let sim_ms = sim_filling(dt);
    let mut speed = MIN_SIM_SPEED;
    for _ in 0..(60.0 * SPEED_CLIMB_DOUBLE_TIME * 6.0) as usize {
        speed = advance_speed(speed, 1.0, dt, sim_ms);
    }
    assert_eq!(speed, 1.0, "подъём встал на {speed} вместо запрошенного 1x");

    // и полоса на месте там, где она нужна: дрожь замера вниз внутри неё
    // скорость не роняет — иначе шум тянул бы её храповиком
    let jitter = 1.0 - SPEED_DEADBAND / 2.0;
    assert_eq!(advance_speed(1.0, jitter, dt, sim_ms), 1.0);
    // а честная просадка проходит сразу
    let real_drop = 1.0 - SPEED_DEADBAND * 2.0;
    assert_eq!(advance_speed(1.0, real_drop, dt, sim_ms), real_drop);
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
            speed = advance_speed(speed, target, previous, sim * 1000.0);
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
