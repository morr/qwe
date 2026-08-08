//! Демо-сцена расталкивания пешек: толпа на пустой карте, крупным планом, с
//! честными числами перекрытия на экране.
//!
//! Зачем отдельная сцена, а не игра. Расталкивание
//! (`movement/separation/`) работает только во вьюпорте, только при зуме
//! ниже [`SEPARATION_MAX_ZOOM`] и не чаще раза в кадр — то есть проверяется
//! ровно в тех условиях, которые в игре надо *дождаться*: воронка у портала,
//! встречные потоки в коридоре, толпа на 30×. Здесь любой из этих случаев
//! ставится клавишей, и рядом висит счётчик перекрывшихся пар.
//!
//! **Числа считаются только по тому, что в кадре.** Расталкивание работает по
//! прямоугольнику вокруг камеры и за кадром намеренно не работает, так что
//! пешки за краем экрана в счёт не идут — иначе метрика мерила бы систему там,
//! где её нет (первая версия этой сцены так и сделала: «пары» набирались из
//! забредших за кадр, и вкл/выкл расталкивания не отличались).
//!
//! **Что здесь настоящее.** Всё, что считает: `MovementPlugin` со своим
//! `separate_pawns`, настоящие `SpatialGrid`, настоящие константы из
//! `settings.rs`. Своя в примере только раскладка толпы и маршруты — то есть
//! то, что в игре даёт поведение. Переписанная копия расталкивания ничего бы
//! не доказала.
//!
//! **Навигация — полигональная, как в игре.** `PolymeshDebug::enabled` включён
//! по умолчанию, и от него же зависит само расталкивание
//! (`movement::separation_runs`): на сеточной навигации его нет вовсе, и мерить
//! в этой сцене было бы нечего. Отсюда два следствия для раскладки:
//! - отрезки маршрутов прокладывает `find_path_polymesh` — waypoint'ы
//!   метрические, углами препятствий, а не центрами навтайлов. Пока меш
//!   строится (первые доли секунды), путь идёт прямой по центрам тайлов, как
//!   раньше: та же подмена, которой в игре сетка обслуживает запросы до
//!   готовности меша;
//! - стены «коридора» кладутся не только в сетку, но и в `MapData::walls` —
//!   иначе полигональный поиск о них не знает и водит пешек сквозь стену.
//!   Смена сценария поэтому пересобирает меш (`PolyNavmesh::clear`).
//!
//! Переключателя на сеточную навигацию здесь нет намеренно: на ней
//! расталкивания не бывает, а эта сцена ровно про него.
//!
//! **Почему толпа выглядит слипшейся даже когда всё правильно.** Радиус тела
//! [`HUMAN_BODY_RADIUS`] = 0.45 м, то есть разведённая пара стоит в 0.9 м, а
//! спрайт — [`HUMAN_SIZE`] = 1.0 м. Спрайты в покое перекрываются на 10%
//! стороны. Поэтому поверх спрайта рисуется круг настоящего радиуса тела:
//! зелёный — дистанция выдержана, красный — реальное перекрытие. Глазом эти
//! два случая на спрайтах неразличимы.
//!
//! ```text
//! cargo run --example crowd_demo
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | `1`–`5` | сценарий: куча / воронка / встречные колонны / коридор / блуждание |
//! | `R` | пересобрать сценарий |
//! | `S` | расталкивание вкл/выкл |
//! | `Space` | пауза |
//! | `-` `=` | скорость симуляции по лестнице 1…30× |
//! | колесо | зум (за 0.75 расталкивание отключается — это видно по счётчику прогонов) |
//!
//! Сцену можно не только смотреть, но и опрашивать: раз в две секунды та же
//! строка метрик уходит в stdout, а BRP поднят на своём порту (`BRP_PORT`,
//! по умолчанию 15704 — не 15702 игры), так что сценарий и скорость
//! переключаются снаружи:
//!
//! ```text
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res set Scenario . '"Funnel"'
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res set DemoSpeed .0 20
//! BRP_PORT=15704 .claude/skills/live-app/scripts/brp res get Overlaps
//! ```
//!
//! **Пример не трогает конфиг игры.** Ни `PrefsPlugin`, ни `SettingsPlugin`,
//! ни `CameraPlugin`, ни `DevPlugin`, ни `MapPlugin` — то есть читать и писать
//! `settings.toml` здесь физически нечему (единственные записи на диск в
//! проекте: `prefs.rs`, `camera.rs`, `dev.rs::screenshot`, кеш Overpass). Свои
//! настройки демо держит в [`DemoConfig`] в этом же файле и никуда не
//! сохраняет.

use std::collections::VecDeque;

use bevy::app::AppExit;
use bevy::diagnostic::{Diagnostic, DiagnosticsStore, RegisterDiagnostic};
use bevy::input::common_conditions::input_just_pressed;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::text::FontSize;

use bevy::ui_widgets::{SliderValue, ValueChange};
use qwe::diagnostics::SIM_SEPARATION_MS;
use qwe::grid::{tile_center, world_to_tile};
use qwe::human::{
    Human, HumanFirstWanderTag, HumanStyle, HumanWanderTag, Pace, WanderHeading, WanderPause,
    pick_wander_targets,
};
use qwe::loading::{AppState, PlayPhase};
use qwe::map::osm::{MapData, WallLine};
use qwe::movement::{
    DestinationClaim, DestinationClaims, Movable, MovableReachedDestinationEvent,
    MovableStateMovingTag, SeparationHolds, SeparationLab, SeparationStats, SeparationSteer,
    SeparationStyle, SimPosition, SlotLab, SlotMatching, SlotSearch, separation_allowed_by_mode,
    separation_cell, slot_side, slot_target,
};
use qwe::navigation::{
    ArcNavmesh, Pathfinder, PathfindingAlgorithm, PolyNavmesh, PolymeshDebug, find_path_polymesh,
};
use qwe::rng::{PawnId, RngDomain, WanderIndex, WorldSeed, decision_stream, stream};
use qwe::settings::{
    HUMAN_SIZE, HUMAN_SPEED_SPREAD, HUMAN_WALK_SPEED, MAP_CENTER_PORTAL_POS, SEPARATION_MAX_ZOOM,
    navtile_size, unit_z,
};
use qwe::ui::slider::{SliderRow, quantize, spawn_slider_row};
use rand::Rng;

// ------------------------------------------------------------ командная строка

/// Разобранная командная строка. Всё до единого — необязательное: без
/// аргументов сцена запускается ровно так же, как до их появления.
///
/// Зачем аргументы, когда есть BRP. Замер — это серия из десятков прогонов, где
/// от прогона к прогону меняется одна константа, и каждый обязан стартовать в
/// одинаковых условиях. Через BRP сценарий и ручки ставятся ПОСЛЕ старта, то
/// есть первые секунды толпа успевает пожить не в том режиме, который меряют, —
/// а именно эти секунды и решают, расслоится поток или слипнется. Аргумент
/// действует с нулевого кадра.
#[derive(Clone, Debug, Default)]
struct Args {
    scenario: Option<Scenario>,
    speed: Option<f32>,
    /// Длина окна замера в РЕАЛЬНЫХ секундах. По истечении сцена печатает
    /// строку `RESULT` и выходит сама — держать её живой нечем и незачем.
    seconds: Option<f32>,
    /// Пешек на сторону (`columns`/`corridor`) или всего (остальные).
    pawns: Option<usize>,
    /// Поперечный разброс колонн, м. 0 — обе колонны в одну линию, как было.
    width: Option<f32>,
    /// Шаг вдоль колонны, м — он же плотность стартовой раскладки.
    spacing: Option<f32>,
    zoom: Option<f32>,
    seed: Option<u64>,
    separation: Option<bool>,
    /// Подпись прогона в строке `RESULT` — по ней отчёт и собирается.
    label: Option<String>,
    /// Снимать экран в начале, середине и конце окна: артефакты (телепорт,
    /// проход насквозь) числами не ловятся до конца.
    shots: bool,
    radius: Option<f32>,
    /// Радиус поиска свободного слота, м ([`SlotSearch`]). Ручка стенда наравне
    /// с радиусом тела: у неё нет правильного значения, есть компромисс между
    /// «хвост толпы остался без слотов» и «цель уехала слишком далеко».
    search: Option<f32>,
    /// Как пачка пешек, идущих в одну точку, разбирает слоты ([`SlotMatching`]).
    matching: Option<SlotMatching>,
    /// Лишние навтайлы к шагу решётки слотов ([`SlotLab::slack`]).
    slot_slack: Option<i32>,
    /// Ближе какого расстояния до цели выдаётся слот ([`SlotLab::claim_at`]).
    claim_at: Option<f32>,
    /// На сколько метров можно столкнуть осевшую пешку, прежде чем она пойдёт
    /// обратно на свой слот ([`SlotLab::regroup`]).
    regroup: Option<f32>,
    hold: Option<f32>,
    sidestep: Option<f32>,
    backstep: Option<f32>,
    lab: Vec<(String, f32)>,
}

/// Ручки [`SeparationLab`], доступные с командной строки. Список здесь, а не
/// `match` по строке в трёх местах: подпись в `RESULT`, разбор и справка обязаны
/// перечислять одно и то же.
const LAB_KNOBS: [&str; 21] = [
    "rate",
    "max-step",
    "max-speed",
    "horizon",
    "anticipation",
    "margin",
    "lane-bias",
    "compress",
    "compress-at",
    "steer",
    "steer-release",
    "idle-mobility",
    "arrive-slack",
    "slide",
    "pass-squeeze",
    "left-share",
    "stuck-compress",
    "stuck-after",
    "stuck-ramp",
    "hard-core",
    "slide-release",
];

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || {
            argv.next()
                .unwrap_or_else(|| panic!("{flag} expects a value"))
        };
        match flag.trim_start_matches("--") {
            "scenario" => args.scenario = Some(parse_scenario(&value())),
            "speed" => args.speed = Some(parse_number(&value())),
            "seconds" => args.seconds = Some(parse_number(&value())),
            "pawns" => args.pawns = Some(parse_number::<f32>(&value()) as usize),
            "width" => args.width = Some(parse_number(&value())),
            "spacing" => args.spacing = Some(parse_number(&value())),
            "zoom" => args.zoom = Some(parse_number(&value())),
            "seed" => args.seed = Some(parse_number::<f32>(&value()) as u64),
            "sep" => args.separation = Some(matches!(value().as_str(), "on" | "1" | "true")),
            "label" => args.label = Some(value()),
            "shots" => args.shots = true,
            "radius" => args.radius = Some(parse_number(&value())),
            "search" => args.search = Some(parse_number(&value())),
            "matching" => args.matching = Some(parse_matching(&value())),
            "slot-slack" => args.slot_slack = Some(parse_number::<f32>(&value()) as i32),
            "claim-at" => args.claim_at = Some(parse_number(&value())),
            "regroup" => args.regroup = Some(parse_number(&value())),
            "hold" => args.hold = Some(parse_number(&value())),
            "sidestep" => args.sidestep = Some(parse_number(&value())),
            "backstep" => args.backstep = Some(parse_number(&value())),
            "crowd-sidestep" => args
                .lab
                .push(("crowd-sidestep".into(), parse_number(&value()))),
            knob if LAB_KNOBS.contains(&knob) => {
                let knob = knob.to_string();
                args.lab.push((knob, parse_number(&value())));
            }
            other => panic!(
                "unknown flag --{other}; known: {LAB_KNOBS:?} and the flags in the module header"
            ),
        }
    }
    args
}

fn parse_number<T: std::str::FromStr>(raw: &str) -> T {
    raw.parse()
        .unwrap_or_else(|_| panic!("{raw} is not a number"))
}

fn parse_matching(raw: &str) -> SlotMatching {
    match raw {
        "greedy" | "0" => SlotMatching::Greedy,
        "batch" | "1" => SlotMatching::Batch,
        other => panic!("unknown matching {other}; use greedy or batch"),
    }
}

fn parse_scenario(raw: &str) -> Scenario {
    match raw {
        "1" | "pile" => Scenario::Pile,
        "2" | "funnel" => Scenario::Funnel,
        "3" | "columns" => Scenario::Columns,
        "4" | "corridor" => Scenario::Corridor,
        "5" | "wander" => Scenario::Wander,
        other => panic!("unknown scenario {other}; use 1-5 or pile/funnel/columns/corridor/wander"),
    }
}

/// Разложить `--rate 8 --horizon 1.5 …` по полям стенда. Отдельной функцией,
/// потому что имена ручек приходят строками и обязаны совпадать с [`LAB_KNOBS`].
fn apply_lab(lab: &mut SeparationLab, knobs: &[(String, f32)]) {
    for (knob, value) in knobs {
        match knob.as_str() {
            "rate" => lab.rate = *value,
            "max-step" => lab.max_step = *value,
            "max-speed" => lab.max_speed = *value,
            "horizon" => lab.horizon = *value,
            "anticipation" => lab.anticipation = *value,
            "margin" => lab.anticipate_margin = *value,
            "lane-bias" => lab.lane_bias = *value,
            "compress" => lab.compress = *value,
            "compress-at" => lab.compress_at = *value,
            "steer" => lab.steer = *value,
            "steer-release" => lab.steer_release = *value,
            "idle-mobility" => lab.idle_mobility = *value,
            "arrive-slack" => lab.arrive_slack = *value,
            "slide" => lab.slide = *value,
            "pass-squeeze" => lab.pass_squeeze = *value,
            "left-share" => lab.left_share = *value,
            "stuck-compress" => lab.stuck_compress = *value,
            "stuck-after" => lab.stuck_after = *value,
            "stuck-ramp" => lab.stuck_ramp = *value,
            "hard-core" => lab.hard_core = *value,
            "slide-release" => lab.slide_release = *value,
            "crowd-sidestep" => lab.crowd_sidestep = *value,
            other => panic!("unknown lab knob {other}"),
        }
    }
}

// ---------------------------------------------------------------- конфиг демо

/// Настройки сцены. Живут здесь и только здесь: демо ничего не читает с диска
/// и ничего туда не пишет (см. шапку модуля). Доменные величины — радиус тела,
/// скорость ходьбы, размер спрайта — наоборот, берутся из `settings.rs`: демо
/// обязано мерить числа игры, а не свою копию.
#[derive(Resource, Clone)]
struct DemoConfig {
    /// Центр арены. Середина карты — подальше от краёв, где навтайлы кончаются.
    centre: Vec2,
    start_zoom: f32,
    min_zoom: f32,
    max_zoom: f32,
    /// Лестница скоростей на `-`/`=`; 30× — потолок и в игре.
    speeds: [f32; 6],
    /// Сколько пешек в каждом сценарии и на каком масштабе они расставлены.
    pile: usize,
    pile_radius: f32,
    funnel: usize,
    funnel_radius: f32,
    column: usize,
    column_length: f32,
    /// Шаг между пешками в колонне. Больше дистанции покоя намеренно:
    /// стартовая раскладка обязана быть законной, иначе непонятно, кто создал
    /// перекрытие — поток или спавн.
    ///
    /// Держать его в согласии с [`HUMAN_BODY_RADIUS`] обязательно и вручную:
    /// шаг 1.2 м пережил здесь смену радиуса тела с 0.45 на 0.9 (дистанция
    /// покоя 0.9 → 1.8 м) и молча превратил «законную колонну» в стартовую
    /// давку — 95% пешко-времени в перекрытии с нулевого кадра, то есть весь
    /// замер мерил не поток, а разгребание спавна.
    column_spacing: f32,
    /// Поперечная ширина колонны, м: пешки раскладываются по полосам внутри
    /// неё. 0 — обе колонны в одну линию `y = centre.y`, самый злой лобовой
    /// случай и исторический дефолт этой сцены. Больше нуля — «улица»: у потока
    /// есть куда расслоиться ещё до первого касания, и видно, пользуется ли
    /// механизм этой свободой или всё равно сводит всех в колонну.
    column_width: f32,
    corridor: usize,
    corridor_gap: f32,
    corridor_length: f32,
    wander: usize,
    wander_box: f32,
    /// Сид раскладки: одна и та же куча от запуска к запуску.
    seed: u64,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            centre: MAP_CENTER_PORTAL_POS,
            start_zoom: 0.08,
            min_zoom: 0.02,
            max_zoom: 1.5,
            speeds: [1.0, 2.0, 5.0, 10.0, 20.0, 30.0],
            pile: 80,
            pile_radius: 2.0,
            funnel: 200,
            funnel_radius: 45.0,
            column: 40,
            column_length: 40.0,
            // 2 × HUMAN_BODY_RADIUS = 1.8 м дистанция покоя, плюс запас
            column_spacing: 2.0,
            column_width: 0.0,
            corridor: 120,
            corridor_gap: 4.0,
            corridor_length: 60.0,
            wander: 300,
            wander_box: 60.0,
            seed: 1,
        }
    }
}

// ------------------------------------------------------------------ сценарии

#[derive(Resource, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource)]
enum Scenario {
    /// Куча в одной точке, никто никуда не идёт: чистая сходимость.
    #[default]
    Pile,
    /// Все идут с обода в одну точку и там остаются — случай «толпа у портала».
    Funnel,
    /// Две колонны навстречу по ОДНОЙ линии waypoint'ов: проверка того, не
    /// стирает ли постановка на waypoint (`move_moving_entities`) боковой
    /// сдвиг, который дало расталкивание.
    Columns,
    /// То же, но в коридоре между двумя стенами: толчок в непроходимый тайл
    /// расталкивание отбрасывает целиком, без скольжения вдоль стены.
    Corridor,
    /// Настоящее блуждание игры: `pick_wander_targets` + настоящий A*.
    /// Контрольный случай — слипает ли толпу сама игровая связка.
    Wander,
}

impl Scenario {
    fn label(self) -> &'static str {
        match self {
            Self::Pile => "1 pile",
            Self::Funnel => "2 funnel",
            Self::Columns => "3 columns",
            Self::Corridor => "4 corridor",
            Self::Wander => "5 wander (real AI)",
        }
    }
}

/// Всё, что принадлежит сценарию и умирает при его смене.
#[derive(Component)]
struct DemoPawn;

/// Стена сценария: и спрайт на экране, и запись о заглушенных навтайлах.
#[derive(Component)]
struct DemoWall;

/// Маршрут пешки. Заменяет собой всё поведение — демо гоняет толпу по заранее
/// известным линиям, чтобы мерить расталкивание, а не блуждание.
///
/// `cycle` решает, что происходит после последней точки: замкнутый маршрут
/// начинает круг заново, незамкнутый кончается — [`drive_routes`] снимает
/// компонент, и пешка остаётся стоять там, куда пришла. Без этого дошедшая до
/// цели пешка немедленно получала бы отрезок назад и ходила бы туда-сюда,
/// неотличимо от блуждания.
#[derive(Component)]
struct Route {
    legs: Vec<Vec2>,
    next: usize,
    cycle: bool,
}

// ------------------------------------------------------------------- метрики

/// Скорость симуляции — отдельным ресурсом, а не прямой записью в
/// `Time<Virtual>`: так её видно и можно поставить снаружи по BRP, а сравнение
/// «пар на 1× против пар на 30×» делается без рук на клавиатуре.
#[derive(Resource, Reflect, Clone, Copy, Debug)]
#[reflect(Resource)]
struct DemoSpeed(f32);

impl Default for DemoSpeed {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Перекрытие меньше этого считается сошедшимся: мягкий решатель оставляет
/// асимптотический хвост в единицы миллиметров, и без порога «пар» всегда
/// оказывались бы десятки — при перекрытии, которого нет ни на экране, ни по
/// смыслу.
const OVERLAP_EPSILON: f32 = 0.02;

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
struct Overlaps {
    /// Пешек в кадре и всего в сцене.
    pawns: usize,
    total: usize,
    /// Радиус, по которому посчитано это перекрытие — им же рисуются круги.
    radius: f32,
    pairs: usize,
    /// Максимальная нехватка расстояния до суммы радиусов, м.
    worst: f32,
    mean: f32,
    /// Сколько пешек участвует хотя бы в одной перекрывшейся паре.
    involved: usize,
    /// Пары, сошедшиеся ближе ОДНОГО радиуса тела: люди «плечом к плечу».
    /// В живой толпе это норма, а не артефакт, — спрайты (1.0 м) при таком
    /// расстоянии ещё только касаются.
    deep: usize,
    /// Пары, у которых центры ближе ПОЛОВИНЫ спрайта: тела наложились наполовину,
    /// и на экране это уже проход насквозь. Вот это артефакт, и мерить его надо
    /// отдельно от давки — первая версия считала артефактом любое `deep`, то есть
    /// заодно и всякое законное прижатие в потоке.
    through: usize,
    /// Позиции в кадре и признак перекрытия — чтобы гизмо рисовало ровно то,
    /// что посчитано, а не считало во второй раз.
    #[reflect(ignore)]
    bodies: Vec<(Vec2, bool)>,
    #[reflect(ignore)]
    links: Vec<(Vec2, Vec2)>,
    /// Кто именно перекрыт — не только сколько. Нужен четвёртому критерию
    /// (`sep_share`): «пешка в состоянии расталкивания» это объединение трёх
    /// множеств — придержанные, рулящие и перекрытые, — а сложить их можно
    /// только по сущностям.
    #[reflect(ignore)]
    involved_set: bevy::ecs::entity::EntityHashSet,
}

/// Сколько тиков движения приходится на один прогон расталкивания. Прогоны
/// считаются не по своей копии гейта, а по настоящему замеру
/// `sim/separation_ms`: система пишет его ровно раз за прогон.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct RunCounters {
    runs: u64,
    #[reflect(ignore)]
    last_measurement: Option<std::time::Instant>,
    ticks_per_run: f32,
    window_ticks: u64,
    window_runs: u64,
}

/// Отрезки маршрута, которые полигональный поиск не смог проложить.
///
/// Не молчаливый откат на прямую: цель, выбранная по проходимости тайла, может
/// лежать внутри раздутого на радиус агента контура — в игре это
/// `PathfindingError` (см. `movement/systems.rs::apply_result`), и здесь пешка
/// так же остаётся стоять до следующего тика. Прямая «в обход меша» провела бы
/// её сквозь стену коридора и выглядела бы как работающий сценарий.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct PathMisses(u64);

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
struct Trial {
    label: String,
    /// Длина окна в реальных секундах; 0 — интерактивный запуск без замера.
    window: f32,
    shots: bool,
    /// Реальное время открытия окна. `None`, пока сцена не готова: считать до
    /// постройки полигонального меша значит мерить сеточную ходьбу
    started: Option<f32>,
    real: f32,
    virtual_secs: f64,
    frames: u64,

    travel: f64,
    progress: f64,
    arrivals: u64,

    held_secs: f64,
    overlap_secs: f64,
    /// Знаменатель для обоих: сколько «пешко-секунд» в кадре всего прожито.
    pawn_secs: f64,

    worst_overlap: f32,
    deep_events: u64,
    /// Настоящий артефакт: тела наложились наполовину (см. `Overlaps::through`).
    through_events: u64,
    /// Самое длинное смещение одной пешки за ОДИН ТИК, м — детектор
    /// телепорта: потолок известен точно, см. [`sample_travel`].
    worst_tick_step: f32,

    spread: f64,
    spread_samples: f64,
    lane_order: f64,
    lane_samples: f64,

    /// Пешко-секунды в СОСТОЯНИИ РАСТАЛКИВАНИЯ — четвёртый критерий. Состояние
    /// это объединение трёх множеств, а не одно из них: придержанная
    /// ([`SeparationHolds`]) идёт ослабленным шагом, рулящая
    /// ([`SeparationSteer`]) идёт не туда, куда хотела, перекрытая платит и тем
    /// и другим позже. Считать только придержку значило бы объявить победителем
    /// вариант, который придержку отключил (`--hold 1`), ничего не починив.
    sep_secs: f64,
    /// Раздельно, чтобы было видно, из чего сложилось объединение.
    steer_secs: f64,
    /// Виртуальная секунда, на которой ВПЕРВЫЕ выполнились оба первых критерия
    /// (все пешки на своих слотах, никто не идёт). `None` — за окно так и не
    /// сошлось.
    settled_at: Option<f64>,
    /// Путь, намотанный пешками ВНЕ состояния «иду» — чистая дрожь осевшей
    /// толпы. Второй критерий числом: «никто не двигается» это не только
    /// «никто не идёт», но и «никого не колышет толчками».
    idle_drift: f64,

    sep_ms: f64,
    sep_ms_samples: f64,
    /// Замер, по которому уже прибавили — тот же трюк, что у `RunCounters`.
    last_sep_ms: Option<std::time::Instant>,

    shots_taken: u32,
}

/// Позиция пешки на прошлом кадре — база для `travel`. Компонентом, а не
/// картой в ресурсе: спавн и деспавн ведёт ECS, а не отдельная уборка.
#[derive(Component)]
struct LastSample(Vec2);

/// Личный счёт пешки за окно замера — то, из чего складываются МЕДИАННЫЕ
/// критерии. Суммы по толпе (`Trial::travel`, `Trial::sep_secs`) прячут
/// распределение: десяток застрявших в давке пешек тонет в двух сотнях
/// дошедших, а медиана показывает судьбу ТИПИЧНОЙ пешки. Компонентом по той же
/// причине, что [`LastSample`].
#[derive(Component, Default)]
struct PawnWindow {
    /// Метры, намотанные этой пешкой за окно (потиково, как `Trial::travel`).
    travel: f32,
    /// Её секунды в состоянии расталкивания — то же объединение трёх множеств,
    /// что у `Trial::sep_secs` (придержана, рулит или перекрыта).
    sep_secs: f32,
    /// Сколько метров она СБЛИЗИЛАСЬ со своими целями (то же, что `Trial::
    /// progress`, но лично).
    ///
    /// Нужен потому, что медианный `travel` во встречном потоке обманывает в
    /// СВОЮ сторону: он растёт и от дуг обхода, и от дрожи в толчее, так что
    /// «прошла больше» и «продвинулась дальше» — разные вещи. На стенде это
    /// видно прямо: вариант с `med_travel` 284 м доходил до цели втрое реже
    /// варианта с `med_travel` 207 м.
    progress: f32,
}

/// Где пешка стояла в момент ОТКРЫТИЯ окна замера. База для нижней границы
/// пути: сумма прямых «старт → куда в итоге встал» — это тот путь, который
/// пешки прошли бы, если бы шли к своим слотам по прямой и никого не
/// встретили. Отношение `travel` к ней (`detour`) и есть третий критерий в
/// виде, не зависящем от того, насколько плотно упакованы слоты: чем плотнее
/// толпа садится, тем БОЛЬШЕ ей идти, и голый `travel` за это наказывал бы.
#[derive(Component)]
struct WindowOrigin(Vec2);

#[derive(Component)]
struct OverlayText;

/// Пределы ползунка радиуса: от «меньше половины спрайта» (как было до правки)
/// до заведомо избыточного личного пространства.
const RADIUS_MIN: f32 = 0.3;
const RADIUS_MAX: f32 = 1.2;
const RADIUS_STEP: f32 = 0.01;

#[derive(Component)]
struct RadiusSlider;

#[derive(Component)]
struct RadiusValueLabel;

/// Пределы ползунка радиуса поиска слота. Снизу — меньше, чем нужно даже
/// десятку пешек, чтобы было видно, как хвост толпы остаётся без слотов и
/// сваливается в общую точку; сверху — вчетверо больше дефолта: 40 м на шаге
/// 2 м это 21 × 21 слот, с запасом на всю «воронку».
const SEARCH_MIN: f32 = 2.0;
const SEARCH_MAX: f32 = 40.0;
const SEARCH_STEP: f32 = 1.0;

#[derive(Component)]
struct SearchSlider;

#[derive(Component)]
struct SearchValueLabel;

fn main() {
    let args = parse_args();
    let mut app = App::new();
    // часть систем игры просит параметры, которых в этой сцене нет (`Gizmos`
    // до появления камеры, `MapData` у полимеша). В игре такое — ошибка, здесь
    // ожидаемое состояние, и ему место в предупреждении, а не в панике
    app.set_error_handler(bevy::ecs::error::warn);

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "qwe crowd demo — separation".to_string(),
                    resolution: (1400, 900).into(),
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                level: bevy::log::Level::WARN,
                filter: "warn,qwe=warn".to_string(),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::srgb(0.72, 0.71, 0.68)))
    // ровно те плагины игры, которые считают движение и расталкивание.
    // НЕ добавлять сюда `PrefsPlugin`/`CameraPlugin`/`DevPlugin`/`MapPlugin`:
    // они читают и пишут конфиг игры и её кеши (см. шапку модуля)
    .add_plugins((
        qwe::rng::RngPlugin,
        qwe::navigation::NavigationPlugin,
        qwe::spatial::SpatialPlugin,
        qwe::movement::MovementPlugin,
    ))
    .init_state::<AppState>()
    .add_sub_state::<PlayPhase>()
    // `Determinism` не вставляем сознательно: `separate_pawns` идёт под
    // `run_if(not(deterministic))` и в детерминированном режиме выключен
    .init_resource::<MapData>()
    .init_resource::<HumanStyle>()
    .init_resource::<DemoConfig>()
    .init_resource::<Scenario>()
    .init_resource::<DemoSpeed>()
    .init_resource::<Overlaps>()
    .init_resource::<RunCounters>()
    .init_resource::<PathMisses>()
    .init_resource::<Trial>()
    .register_type::<Scenario>()
    .register_type::<DemoSpeed>()
    .register_type::<Overlaps>()
    .register_type::<RunCounters>()
    .register_type::<PathMisses>()
    // свой порт: 15702 занимает игра пользователя, 15703 — её же
    // инспекция из `live-app`
    .add_plugins((
        bevy::remote::RemotePlugin::default(),
        bevy::remote::http::RemoteHttpPlugin::default().with_port(brp_port()),
    ))
    // плоский A*: не зависит от фоновой постройки иерархии HPA и готов с
    // первого кадра. Пути на пустой сетке короткие, цена неважна
    .insert_resource(PathfindingAlgorithm::Astar)
    .insert_resource(WorldSeed(1))
    // `separate_pawns` пишет этот замер, но регистрирует его
    // `GameDiagnosticsPlugin`, которого здесь нет: без регистрации замер
    // выбрасывается, и считать прогоны было бы нечем
    .register_diagnostic(
        Diagnostic::new(SIM_SEPARATION_MS)
            .with_suffix(" ms")
            .with_max_history_length(1),
    )
    .add_systems(
        Startup,
        (
            spawn_camera,
            spawn_overlay,
            spawn_sliders,
            spawn_mechanism_panel,
        ),
    )
    .add_systems(
        Update,
        (
            enter_live.run_if(in_state(PlayPhase::Warmup)),
            // блуждание — та же регистрация, что в `HumanPlugin` для
            // недетерминированного режима
            pick_wander_targets,
            respawn_scenario
                .run_if(resource_changed::<Scenario>.or_else(input_just_pressed(KeyCode::KeyR))),
            pick_scenario,
            toggle_separation.run_if(input_just_pressed(KeyCode::KeyS)),
            toggle_pause.run_if(input_just_pressed(KeyCode::Space)),
            zoom_camera,
            // бегунок ведёт та же система, что и в игре; `sync_knob_rows`
            // подтягивает панель механизмов после нажатия пресета
            (qwe::ui::slider::sync_slider_thumbs, sync_knob_rows),
            apply_speed.run_if(resource_changed::<DemoSpeed>),
            (
                count_separation_runs,
                measure_overlaps,
                draw_bodies,
                update_overlay,
                report_to_stdout,
                // замер — последним в кадре и строго после `measure_overlaps`:
                // он складывает ровно те числа, что тот посчитал
                sample_trial,
                take_shots,
                finish_trial,
            )
                .chain(),
        ),
    )
    // порядок явный: прерывание подошедших — до выдачи отрезков, иначе слот
    // они получат тиком позже (`interrupt_for_slot_claim` снимает тег
    // движения командой, а её применяет точка синхронизации цепочки)
    .add_systems(
        FixedUpdate,
        (
            count_ticks,
            interrupt_for_slot_claim,
            drive_routes,
            regroup_to_slot,
            sample_travel,
        )
            .chain(),
    )
    .add_observer(
        |_arrival: On<MovableReachedDestinationEvent>, mut trial: ResMut<Trial>| {
            trial.arrivals += 1;
        },
    );

    apply_args(&mut app, args);

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Playing);

    app.run();
}

/// Разложить командную строку по ресурсам — ДО первого кадра, чтобы толпа с
/// самого начала жила в том режиме, который меряют (см. [`Args`]).
fn apply_args(app: &mut App, args: Args) {
    let world = app.world_mut();
    if let Some(seconds) = args.seconds {
        // окно замера включает и печать `RESULT`, и выход: снаружи процесс
        // убивать не нужно, а значит и гадать, успел ли он дописать строку
        world.resource_mut::<Trial>().window = seconds;
    }
    {
        let mut trial = world.resource_mut::<Trial>();
        trial.shots = args.shots;
        trial.label = args.label.clone().unwrap_or_else(|| "baseline".to_string());
    }
    if let Some(scenario) = args.scenario {
        *world.resource_mut::<Scenario>() = scenario;
    }
    if let Some(speed) = args.speed {
        world.resource_mut::<DemoSpeed>().0 = speed;
    }
    {
        let mut config = world.resource_mut::<DemoConfig>();
        if let Some(pawns) = args.pawns {
            config.pile = pawns;
            config.funnel = pawns;
            config.column = pawns;
            config.corridor = pawns;
            config.wander = pawns;
        }
        if let Some(width) = args.width {
            config.column_width = width;
        }
        if let Some(spacing) = args.spacing {
            config.column_spacing = spacing;
        }
        if let Some(zoom) = args.zoom {
            config.start_zoom = zoom;
        }
        if let Some(seed) = args.seed {
            config.seed = seed;
        }
    }
    {
        let mut style = world.resource_mut::<SeparationStyle>();
        if let Some(enabled) = args.separation {
            style.enabled = enabled;
        }
        if let Some(hold) = args.hold {
            style.hold = hold;
        }
        if let Some(sidestep) = args.sidestep {
            style.sidestep = sidestep;
        }
        if let Some(backstep) = args.backstep {
            style.backstep = backstep;
        }
    }
    if let Some(radius) = args.radius {
        world.resource_mut::<HumanStyle>().body_radius = radius;
    }
    if let Some(search) = args.search {
        world.resource_mut::<SlotSearch>().0 = search;
    }
    if let Some(matching) = args.matching {
        world.resource_mut::<SlotLab>().matching = matching;
    }
    if let Some(slack) = args.slot_slack {
        world.resource_mut::<SlotLab>().slack = slack;
    }
    if let Some(claim_at) = args.claim_at {
        world.resource_mut::<SlotLab>().claim_at = claim_at;
    }
    if let Some(regroup) = args.regroup {
        world.resource_mut::<SlotLab>().regroup = regroup;
    }
    apply_lab(&mut world.resource_mut::<SeparationLab>(), &args.lab);
}

/// Порт BRP этой сцены. По умолчанию 15704: 15702 держит игра пользователя,
/// 15703 — её инспекция из скилла `live-app`, и ответ с чужого порта ничем не
/// отличается от своего.
fn brp_port() -> u16 {
    std::env::var("BRP_PORT")
        .map(|value| value.parse().expect("BRP_PORT is not a port number"))
        .unwrap_or(15704)
}

/// `PlayPhase` в игре переключает загрузчик, которого здесь нет: без `Live`
/// часть систем навигации молча не работает.
fn enter_live(mut next: ResMut<NextState<PlayPhase>>) {
    next.set(PlayPhase::Live);
}

// ------------------------------------------------- панель механизмов и пресеты

/// Все ручки стенда, которые есть смысл крутить глазами, — одним списком.
///
/// Список, а не пятнадцать почти одинаковых функций: у каждой ручки одно и то же
/// поведение (подпись, диапазон, чтение из ресурса, запись в ресурс), и
/// расходиться этим копиям незачем. Тот же список кормит и панель, и её
/// обновление после нажатия пресета.
struct Knob {
    label: &'static str,
    /// `(min, max, шаг)`, как у ползунков игры.
    range: (f32, f32, f32),
    /// Сколько знаков после запятой в подписи значения.
    digits: usize,
    get: fn(&Tuning) -> f32,
    set: fn(&mut Tuning, f32),
}

/// Ресурсы, которые крутит панель, — одним параметром: в системе их иначе
/// набирается столько, что не остаётся места на запросы.
#[derive(bevy::ecs::system::SystemParam)]
struct Tuning<'w> {
    lab: ResMut<'w, SeparationLab>,
    slots: ResMut<'w, SlotLab>,
    style: ResMut<'w, SeparationStyle>,
}

const KNOBS: &[Knob] = &[
    Knob {
        label: "pass squeeze",
        range: (0.3, 1.0, 0.05),
        digits: 2,
        get: |t| t.lab.pass_squeeze,
        set: |t, value| t.lab.pass_squeeze = value,
    },
    Knob {
        label: "left share",
        range: (0.0, 0.5, 0.05),
        digits: 2,
        get: |t| t.lab.left_share,
        set: |t, value| t.lab.left_share = value,
    },
    Knob {
        label: "regroup",
        range: (0.0, 4.0, 0.25),
        digits: 2,
        get: |t| t.slots.regroup,
        set: |t, value| t.slots.regroup = value,
    },
    Knob {
        label: "stuck compress",
        range: (0.0, 0.8, 0.05),
        digits: 2,
        get: |t| t.lab.stuck_compress,
        set: |t, value| t.lab.stuck_compress = value,
    },
    Knob {
        label: "steer",
        range: (0.0, 2.0, 0.1),
        digits: 1,
        get: |t| t.lab.steer,
        set: |t, value| t.lab.steer = value,
    },
    Knob {
        label: "hold",
        range: (0.0, 1.0, 0.05),
        digits: 2,
        get: |t| t.style.hold,
        set: |t, value| t.style.hold = value,
    },
    Knob {
        label: "rate",
        range: (1.0, 16.0, 1.0),
        digits: 0,
        get: |t| t.lab.rate,
        set: |t, value| t.lab.rate = value,
    },
    Knob {
        label: "max speed",
        range: (0.0, 4.0, 0.1),
        digits: 1,
        get: |t| t.lab.max_speed,
        set: |t, value| t.lab.max_speed = value,
    },
    Knob {
        label: "slide",
        range: (0.0, 1.0, 0.1),
        digits: 1,
        get: |t| t.lab.slide,
        set: |t, value| t.lab.slide = value,
    },
    Knob {
        label: "slide release",
        range: (0.0, 3.0, 0.25),
        digits: 2,
        get: |t| t.lab.slide_release,
        set: |t, value| t.lab.slide_release = value,
    },
    Knob {
        label: "hard core",
        range: (0.0, 0.7, 0.05),
        digits: 2,
        get: |t| t.lab.hard_core,
        set: |t, value| t.lab.hard_core = value,
    },
    Knob {
        label: "compress",
        range: (0.0, 0.6, 0.05),
        digits: 2,
        get: |t| t.lab.compress,
        set: |t, value| t.lab.compress = value,
    },
    Knob {
        label: "claim at",
        range: (0.0, 40.0, 2.0),
        digits: 0,
        get: |t| t.slots.claim_at,
        set: |t, value| t.slots.claim_at = value,
    },
];

/// Набор настроек целиком — кнопка в панели. Пресеты названы по тому, ЧТО они
/// показывают, а не по номеру эксперимента: их смотрят глазами, переключая
/// туда-сюда на живой толпе.
struct Preset {
    label: &'static str,
    apply: fn(&mut Tuning),
}

const PRESETS: &[Preset] = &[
    Preset {
        label: "game",
        apply: |t| {
            *t.lab = SeparationLab::default();
            *t.slots = SlotLab::default();
            *t.style = SeparationStyle::default();
        },
    },
    // прошлый визуальный фаворит — тот, с которого начался этот заход:
    // постоянное сжатие радиуса, из-за которого толпа садится слипшейся
    Preset {
        label: "old",
        apply: |t| {
            *t.lab = SeparationLab {
                rate: 4.0,
                max_speed: 1.4,
                steer: 1.0,
                compress: 0.2,
                ..SeparationLab::default()
            };
            *t.slots = SlotLab {
                matching: SlotMatching::Batch,
                ..SlotLab::default()
            };
            t.style.hold = 1.0;
        },
    },
    // победитель воронки: протискивание мимо стоящих плюс возврат на свой слот
    Preset {
        label: "funnel",
        apply: |t| {
            *t.lab = SeparationLab {
                rate: 4.0,
                max_speed: 1.4,
                steer: 1.0,
                pass_squeeze: 0.6,
                left_share: 0.2,
                ..SeparationLab::default()
            };
            *t.slots = SlotLab {
                matching: SlotMatching::Batch,
                regroup: 1.0,
                ..SlotLab::default()
            };
            t.style.hold = 1.0;
        },
    },
    // победитель улицы: то же самое, но возврат на слот там не при делах
    Preset {
        label: "street",
        apply: |t| {
            *t.lab = SeparationLab {
                rate: 4.0,
                max_speed: 1.4,
                steer: 1.0,
                pass_squeeze: 0.6,
                left_share: 0.2,
                ..SeparationLab::default()
            };
            *t.slots = SlotLab {
                matching: SlotMatching::Batch,
                ..SlotLab::default()
            };
            t.style.hold = 1.0;
        },
    },
];

/// Ползунок ручки под этим номером в [`KNOBS`].
#[derive(Component)]
struct KnobSlider(usize);

/// Подпись значения той же ручки.
#[derive(Component)]
struct KnobValueLabel(usize);

/// Панель механизмов справа: кнопки пресетов сверху, под ними ползунок на
/// каждую ручку.
fn spawn_mechanism_panel(mut commands: Commands, tuning: Tuning) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(10.),
                right: px(12.),
                width: px(220.),
                flex_direction: FlexDirection::Column,
                row_gap: px(2.),
                ..default()
            },
            BackgroundColor(Color::srgba(0., 0., 0., 0.25)),
        ))
        .id();

    let presets = commands
        .spawn((
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                column_gap: px(4.),
                padding: UiRect::all(px(6.)),
                ..default()
            },
            ChildOf(panel),
        ))
        .id();
    for (index, preset) in PRESETS.iter().enumerate() {
        qwe::ui::spawn_panel_button(
            &mut commands,
            presets,
            PresetButton,
            preset.label,
            move |_activate: On<bevy::ui_widgets::Activate>, mut tuning: Tuning| {
                (PRESETS[index].apply)(&mut tuning);
            },
        );
    }

    for (index, knob) in KNOBS.iter().enumerate() {
        let value = (knob.get)(&tuning);
        spawn_slider_row(
            &mut commands,
            panel,
            SliderRow {
                label: knob.label,
                value,
                value_text: format!("{value:.*}", knob.digits),
                range: knob.range,
            },
            KnobValueLabel(index),
            KnobSlider(index),
            move |change: On<ValueChange<f32>>, mut commands: Commands, mut tuning: Tuning| {
                let (min, max, step) = KNOBS[index].range;
                let stepped = quantize(change.value, min, max, step);
                commands.entity(change.source).insert(SliderValue(stepped));
                (KNOBS[index].set)(&mut tuning, stepped);
            },
        );
    }
}

/// Кнопка пресета — маркер, только чтобы её было по чему найти.
#[derive(Component)]
struct PresetButton;

/// Подтянуть ползунки и подписи к ресурсам: пресет меняет по десятку ручек
/// разом, и без этого панель показывала бы то, чего в ресурсах уже нет.
fn sync_knob_rows(
    mut commands: Commands,
    tuning: Tuning,
    sliders: Query<(Entity, &KnobSlider, &SliderValue)>,
    mut labels: Query<(&KnobValueLabel, &mut Text)>,
) {
    if !tuning.lab.is_changed() && !tuning.slots.is_changed() && !tuning.style.is_changed() {
        return;
    }
    // `SliderValue` неизменяемый компонент — ставится вставкой, как и в
    // наблюдателях самих ползунков
    for (entity, slider, current) in &sliders {
        let next = (KNOBS[slider.0].get)(&tuning);
        if current.0 != next {
            commands.entity(entity).insert(SliderValue(next));
        }
    }
    for (label, mut text) in &mut labels {
        let knob = &KNOBS[label.0];
        let next = format!("{:.*}", knob.digits, (knob.get)(&tuning));
        if text.0 != next {
            text.0 = next;
        }
    }
}

fn spawn_camera(mut commands: Commands, config: Res<DemoConfig>) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(config.centre.extend(0.0))
            .with_scale(Vec3::splat(config.start_zoom)),
        Msaa::Off,
    ));
}

/// Ползунки сцены — тот же кит строки-ползунка, что у панелей игры
/// (`qwe::ui::slider`), чтобы обе величины подбирались глазом на живой толпе, а
/// не пересборкой. Пишут прямо в ресурсы движения, откуда их берут и сама
/// механика, и гизмо этой сцены: `HumanStyle::body_radius` — «личное
/// пространство», `SlotSearch` — докуда искать свободный слот назначения.
/// Обе ручки есть и в панелях игры (`ui/stats.rs`) — эта сцена не заводит своих.
fn spawn_sliders(mut commands: Commands, style: Res<HumanStyle>, search: Res<SlotSearch>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(150.),
                left: px(12.),
                width: px(240.),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0., 0., 0., 0.25)),
        ))
        .id();

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Body radius",
            value: style.body_radius,
            value_text: format!("{:.2} m", style.body_radius),
            range: (RADIUS_MIN, RADIUS_MAX, RADIUS_STEP),
        },
        RadiusValueLabel,
        RadiusSlider,
        |change: On<ValueChange<f32>>,
         mut commands: Commands,
         mut style: ResMut<HumanStyle>,
         mut label: Query<&mut Text, With<RadiusValueLabel>>| {
            let stepped = quantize(change.value, RADIUS_MIN, RADIUS_MAX, RADIUS_STEP);
            // ползунок «управляемый»: он только сообщает о правке, а своё
            // `SliderValue` не трогает — без этой строки бегунок остаётся на
            // месте, хотя значение уже изменилось (и следующая протяжка
            // считается от старого)
            commands.entity(change.source).insert(SliderValue(stepped));
            style.body_radius = stepped;
            for mut text in &mut label {
                text.0 = format!("{stepped:.2} m");
            }
        },
    );

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Slot search",
            value: search.0,
            value_text: format!("{:.0} m", search.0),
            range: (SEARCH_MIN, SEARCH_MAX, SEARCH_STEP),
        },
        SearchValueLabel,
        SearchSlider,
        |change: On<ValueChange<f32>>,
         mut commands: Commands,
         mut search: ResMut<SlotSearch>,
         mut label: Query<&mut Text, With<SearchValueLabel>>| {
            let stepped = quantize(change.value, SEARCH_MIN, SEARCH_MAX, SEARCH_STEP);
            commands.entity(change.source).insert(SliderValue(stepped));
            search.0 = stepped;
            for mut text in &mut label {
                text.0 = format!("{stepped:.0} m");
            }
        },
    );
}

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        OverlayText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.),
            ..default()
        },
        TextColor(Color::srgb(0.1, 0.1, 0.12)),
        Node {
            position_type: PositionType::Absolute,
            top: px(10.),
            left: px(12.),
            ..default()
        },
    ));
}

// --------------------------------------------------------------------- спавн

/// Раскладка сценария: снести прошлый, вернуть навмешу проходимость, поставить
/// новый. Одна система на все пять — раскладки различаются только позициями и
/// маршрутами.
///
/// Стены живут в двух местах сразу: тайлы сетки и `MapData::walls` для
/// полигонального меша. Второе — не дубль ради дубля: меш строится из
/// векторной геометрии карты и о правках сетки не знает вовсе, так что без
/// записи в `MapData` пешки ходили бы сквозь стены коридора.
#[allow(clippy::too_many_arguments)]
fn respawn_scenario(
    mut commands: Commands,
    config: Res<DemoConfig>,
    scenario: Res<Scenario>,
    navmesh: Res<ArcNavmesh>,
    mut map: ResMut<MapData>,
    mut poly: ResMut<PolyNavmesh>,
    mut polymesh: ResMut<PolymeshDebug>,
    mut misses: ResMut<PathMisses>,
    old: Query<Entity, Or<(With<DemoPawn>, With<DemoWall>)>>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    clear_arena(&navmesh, config.centre);
    // геометрия прошлого сценария — из карты тоже; пересобрать меш придётся,
    // если стены были или появятся
    let rebuild_polymesh = !map.walls.is_empty() || matches!(*scenario, Scenario::Corridor);
    map.walls.clear();
    misses.0 = 0;

    let mut rng = stream(config.seed, RngDomain::Population, 0);
    let centre = config.centre;

    match *scenario {
        Scenario::Pile => {
            for index in 0..config.pile {
                let angle = rng.random_range(0.0..std::f32::consts::TAU);
                let radius = config.pile_radius * rng.random_range(0.0f32..1.0).sqrt();
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    centre + Vec2::from_angle(angle) * radius,
                    None,
                    false,
                );
            }
        }
        Scenario::Funnel => {
            for index in 0..config.funnel {
                let angle = std::f32::consts::TAU * index as f32 / config.funnel as f32;
                let rim = centre + Vec2::from_angle(angle) * config.funnel_radius;
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    rim,
                    // с обода в точку и всё: дошедшая пешка остаётся в центре.
                    // Обратного отрезка нет намеренно — с ним воронка после
                    // первого прохода превращалась в хождение туда-сюда, и
                    // сорвавшуюся с пути пешку было не отличить от блуждающей
                    Some(Route {
                        legs: vec![centre],
                        next: 0,
                        cycle: false,
                    }),
                    false,
                );
            }
        }
        Scenario::Columns => {
            let half = config.column_length / 2.0;
            // полос ровно столько, сколько влезает по шагу колонны: при ширине
            // 0 полоса одна и раскладка та же, что была
            let lanes = (config.column_width / config.column_spacing)
                .floor()
                .max(0.0) as usize
                + 1;
            for index in 0..config.column {
                let lane = index % lanes;
                let rank = index / lanes;
                let across = if lanes > 1 {
                    -config.column_width / 2.0
                        + config.column_width * lane as f32 / (lanes - 1) as f32
                } else {
                    0.0
                };
                let offset = rank as f32 * config.column_spacing;
                let left = centre + Vec2::new(-half - offset, across);
                let right = centre + Vec2::new(half + offset, across);
                // при нулевой ширине обе колонны идут по одной линии
                // y = centre.y, то есть по одним и тем же центрам навтайлов
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    left,
                    Some(Route {
                        legs: vec![right, left],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    (config.column + index) as u32,
                    right,
                    Some(Route {
                        legs: vec![left, right],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
            }
        }
        Scenario::Corridor => {
            let half = config.corridor_length / 2.0;
            let gap = config.corridor_gap / 2.0;
            spawn_wall(&mut commands, &navmesh, &mut map, centre, half, gap, 1.0);
            spawn_wall(&mut commands, &navmesh, &mut map, centre, half, gap, -1.0);

            for index in 0..config.corridor {
                let side = if index % 2 == 0 { -1.0 } else { 1.0 };
                let along = index as f32 / config.corridor as f32 * half;
                let lane = rng.random_range(-gap + 0.5..gap - 0.5);
                let start = centre + Vec2::new(side * (half + along), lane);
                let end = centre + Vec2::new(-side * (half + along), lane);
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    start,
                    Some(Route {
                        legs: vec![end, start],
                        next: 0,
                        cycle: true,
                    }),
                    false,
                );
            }
        }
        Scenario::Wander => {
            let half = config.wander_box / 2.0;
            for index in 0..config.wander {
                let position = centre
                    + Vec2::new(rng.random_range(-half..half), rng.random_range(-half..half));
                spawn_pawn(
                    &mut commands,
                    config.seed,
                    index as u32,
                    position,
                    None,
                    true,
                );
            }
        }
    }

    // меш описывает геометрию прошлого сценария — под новую его надо
    // построить заново. В игре ту же работу делает вход в `Playing` при смене
    // города; здесь состояние не меняется, поэтому постройку будит правка
    // тумблера, на которую подписан `sync_polymesh_build`
    if rebuild_polymesh {
        poly.clear();
        polymesh.set_changed();
    }
}

/// Пешка сценария. Бандл — тот же, что у `human::spawn_population`, иначе
/// расталкивание её не увидит: оно берёт кандидатов из `SpatialGrid<Human>`
/// (наполняется обсервером по `Transform`) и требует `PawnId` не-опционально.
fn spawn_pawn(
    commands: &mut Commands,
    seed: u64,
    pawn_id: u32,
    position: Vec2,
    route: Option<Route>,
    wandering: bool,
) {
    let mut rng = decision_stream(seed, RngDomain::Human, pawn_id, WanderIndex::SPAWN);
    let color = Color::hsl(
        rng.random_range(0.0..360.0),
        rng.random_range(0.35..0.75),
        rng.random_range(0.35..0.65),
    );
    let pace = Pace(rng.random_range(-1.0..=1.0));
    let heading = WanderHeading(Vec2::from_angle(
        rng.random_range(0.0..std::f32::consts::TAU),
    ));

    let mut entity = commands.spawn((
        DemoPawn,
        Sprite {
            color,
            custom_size: Some(Vec2::splat(HUMAN_SIZE)),
            ..default()
        },
        Transform::from_translation(position.extend(unit_z(position.y))),
        Human,
        Movable::new(pace.speed(HUMAN_WALK_SPEED, HUMAN_SPEED_SPREAD)),
        pace,
        PawnId(pawn_id),
        WanderIndex::ready(),
        LastSample(position),
        WindowOrigin(position),
        ProgressSample::default(),
        PawnWindow::default(),
        Name::new("demo pawn"),
    ));
    if let Some(route) = route {
        entity.insert(route);
    }
    if wandering {
        // то, что спрашивает `pick_wander_targets`
        entity.insert((
            HumanWanderTag,
            HumanFirstWanderTag,
            WanderPause(Timer::from_seconds(0.0, TimerMode::Once)),
            heading,
        ));
    }
}

/// Полоса стены вдоль коридора: спрайт для глаза, заглушенные навтайлы для
/// расталкивания и линия в `MapData::walls` для полигонального меша.
///
/// Записей две, потому что бэкендов два и читают они разное: расталкивание
/// проверяет проходимость тайла (`separation/`), а меш строится из
/// векторных контуров карты. Стена, забытая во втором, — дыра ровно в том
/// сценарии, ради которого она поставлена.
fn spawn_wall(
    commands: &mut Commands,
    navmesh: &ArcNavmesh,
    map: &mut MapData,
    centre: Vec2,
    half_length: f32,
    gap: f32,
    side: f32,
) {
    let thickness = 6.0;
    let band = centre + Vec2::new(0.0, side * (gap + thickness / 2.0));
    let size = Vec2::new(half_length * 2.0 + thickness, thickness);

    // осевая ленты — то же, чем стена задана в OSM: `ribbon_outline` раздует
    // её обратно до `size` при постройке меша
    map.walls.push(WallLine {
        points: vec![
            band - Vec2::new(size.x / 2.0, 0.0),
            band + Vec2::new(size.x / 2.0, 0.0),
        ],
        width: thickness,
    });

    {
        let mut navmesh = navmesh.write();
        let lo = world_to_tile(band - size / 2.0);
        let hi = world_to_tile(band + size / 2.0);
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                navmesh.set_passable(x, y, false);
            }
        }
    }

    commands.spawn((
        DemoWall,
        Sprite {
            color: Color::srgb(0.42, 0.40, 0.38),
            custom_size: Some(size),
            ..default()
        },
        Transform::from_translation(band.extend(unit_z(band.y) - 1.0)),
    ));
}

/// Вернуть арене проходимость: стены прошлого сценария иначе остались бы в
/// навмеше навсегда — он живёт в ресурсе, а не в сущностях сцены.
fn clear_arena(navmesh: &ArcNavmesh, centre: Vec2) {
    const ARENA_HALF: f32 = 120.0;
    let mut navmesh = navmesh.write();
    let lo = world_to_tile(centre - Vec2::splat(ARENA_HALF));
    let hi = world_to_tile(centre + Vec2::splat(ARENA_HALF));
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            navmesh.set_passable(x, y, true);
        }
    }
}

// ------------------------------------------------------------------ движение

/// Выдать вставшей пешке следующий отрезок маршрута. Заменяет собой очередь и
/// асинхронность настоящего диспетчера, но не сам поиск: путь по готовому мешу
/// считает `find_path_polymesh` — тот же вызов, что делает таск в игре, только
/// синхронно (пешек здесь сотни, и то лишь в момент, когда отрезок кончился).
///
/// Пока меш строится, путь идёт прямой по центрам навтайлов — в такой форме
/// его отдаёт сеточный A*, и это ровно то, чем в игре сетка обслуживает
/// запросы до готовности меша.
///
/// Незамкнутый маршрут после последнего отрезка снимается: пешка выпадает из
/// запроса этой системы и остаётся стоять. Заявку на слот (`DestinationClaim`)
/// при этом не трогаем — пешка на нём стоит, и отпустить его значило бы отдать
/// занятое место следующему.
/// Отрезок идёт через тот же слот назначения, что и цели в игре
/// (`movement::destination`): без этого «воронка» гоняла бы 200 пешек в одну
/// точку — то есть проверяла бы не расталкивание, а очередь к одному тайлу.
#[allow(clippy::too_many_arguments)]
fn drive_routes(
    mut commands: Commands,
    pathfinder: Pathfinder,
    style: Res<HumanStyle>,
    search: Res<SlotSearch>,
    lab: Res<SlotLab>,
    mut claims: ResMut<DestinationClaims>,
    mut misses: ResMut<PathMisses>,
    mut pawns: Query<
        (
            Entity,
            &mut Movable,
            &mut Route,
            &SimPosition,
            Option<&DestinationClaim>,
        ),
        Without<MovableStateMovingTag>,
    >,
    mut batch: Local<Vec<(Entity, Option<IVec2>, IVec2, Vec2)>>,
    mut slots: Local<Vec<Option<(IVec2, IVec2)>>>,
) {
    claims.sync(slot_side(style.body_radius * 2.0) + lab.slack, search.0);
    let polymesh = pathfinder.polymesh_build();
    let navmesh = pathfinder.navmesh.read();

    // заявки — всем пакетом и ДО прокладки путей, как в игре
    // (`assign_destination_slots` идёт раньше диспетчера): пакетное назначение
    // (`SlotMatching::Batch`) без пакета вырождается в жадное
    batch.clear();
    batch.extend(
        pawns
            .iter()
            // отложенная выдача ([`SlotLab::claim_at`]): пока цель далеко,
            // пешка в пакет не входит вовсе — слот занимать рано, и занятым в
            // индексе он числиться не должен
            .filter(|(_, _, route, position, _)| {
                lab.claim_at <= 0.0 || position.0.distance(route.legs[route.next]) <= lab.claim_at
            })
            .map(|(entity, _, route, position, claim)| {
                (
                    entity,
                    claim.map(|claim| claim.0),
                    world_to_tile(route.legs[route.next]),
                    position.0,
                )
            }),
    );
    qwe::movement::claim_batch(
        &mut claims,
        lab.matching,
        &batch,
        |tile| navmesh.is_passable(tile.x, tile.y),
        &mut slots,
    );
    let assigned: HashMap<Entity, Option<(IVec2, IVec2)>> = batch
        .iter()
        .zip(slots.iter())
        .map(|((entity, ..), slot)| (*entity, *slot))
        .collect();

    for (entity, mut movable, mut route, position, _) in &mut pawns {
        let leg = route.legs[route.next];
        let desired = world_to_tile(leg);
        // не в пакете — значит цель ещё далеко и слот не выдан: пешка идёт в
        // саму точку, а отрезок НЕ засчитывается пройденным. Подойдя, она
        // остановится ([`interrupt_for_slot_claim`]) и получит слот здесь же
        let approaching = !assigned.contains_key(&entity);
        let (target_tile, target) = match assigned.get(&entity).copied().flatten() {
            Some((slot, tile)) => {
                commands.entity(entity).insert(DestinationClaim(slot));
                (tile, tile_center(tile))
            }
            None => (desired, leg),
        };

        let path = match polymesh.as_deref() {
            // путь включает стартовую точку — её и отбрасываем, как это
            // делает приёмник ответа в игре
            Some(build) => match find_path_polymesh(build, position.0, target) {
                Some(points) => points.into_iter().skip(1).collect(),
                None => {
                    // цель не села на меш — пешка стоит и пробует снова на
                    // следующем тике, отрезок не считается пройденным
                    misses.0 += 1;
                    continue;
                }
            },
            None => straight_path(position.0, target),
        };

        if !approaching {
            route.next += 1;
            if route.next == route.legs.len() {
                if route.cycle {
                    route.next = 0;
                } else {
                    commands.entity(entity).remove::<Route>();
                }
            }
        }
        // путь из одной точки означает «уже на месте» (тот же контракт, что у
        // `apply_result`): отрезок засчитан, а вести пешку по пустому пути
        // нельзя — `move_moving_entities` докатывал бы её по инерции
        if path.is_empty() {
            continue;
        }
        movable.to_moving(target_tile, path, entity, &mut commands);
    }
}

/// Вернуть на свой слот пешку, которую с него столкнули ([`SlotLab::regroup`]).
///
/// Запрос — ровно тот же, что у переписи `finish_trial`: осевшая (не идёт и
/// маршрута нет) со своей заявкой. Дальше — прямой отрезок к цели слота; путь
/// короткий, поэтому прокладывается той же `find_path_polymesh`, что и всё
/// остальное в этой сцене.
fn regroup_to_slot(
    mut commands: Commands,
    pathfinder: Pathfinder,
    style: Res<HumanStyle>,
    lab: Res<SlotLab>,
    mut pawns: Query<
        (Entity, &mut Movable, &SimPosition, &DestinationClaim),
        (Without<MovableStateMovingTag>, Without<Route>),
    >,
) {
    if lab.regroup <= 0.0 {
        return;
    }
    let side = slot_side(style.body_radius * 2.0) + lab.slack;
    let polymesh = pathfinder.polymesh_build();
    for (entity, mut movable, position, claim) in &mut pawns {
        let home = slot_target(claim.0, side);
        let target = tile_center(home);
        if position.0.distance(target) <= lab.regroup {
            continue;
        }
        let path: VecDeque<Vec2> = match polymesh.as_deref() {
            Some(build) => match find_path_polymesh(build, position.0, target) {
                Some(points) => points.into_iter().skip(1).collect(),
                None => continue,
            },
            None => straight_path(position.0, target),
        };
        if path.is_empty() {
            continue;
        }
        movable.to_moving(home, path, entity, &mut commands);
    }
}

/// Остановить подошедшую к цели пешку, у которой слота ещё нет, — чтобы
/// [`drive_routes`] выдал ей слот здесь и сейчас ([`SlotLab::claim_at`]).
///
/// Без остановки отложенная выдача не работает вовсе: пешка, идущая в саму
/// точку, доберётся до неё только сквозь всех, кто там уже осел, — то есть
/// ровно тем способом, ради избавления от которого выдача и откладывается.
/// Прерывание — это и есть момент «дошёл до толпы»: дальше пешка выбирает
/// ближайший к цели свободный слот, и все занятые лежат глубже неё.
fn interrupt_for_slot_claim(
    mut commands: Commands,
    lab: Res<SlotLab>,
    mut pawns: Query<
        (Entity, &mut Movable, &Route, &SimPosition),
        (With<MovableStateMovingTag>, Without<DestinationClaim>),
    >,
) {
    if lab.claim_at <= 0.0 {
        return;
    }
    for (entity, mut movable, route, position) in &mut pawns {
        if position.0.distance(route.legs[route.next]) > lab.claim_at {
            continue;
        }
        // не «дошла»: событие прибытия здесь было бы ложью, идти ей ещё в слот
        movable.to_idle(entity, &mut commands, false);
    }
}

/// Центры навтайлов вдоль прямой — форма пути сеточного поиска.
fn straight_path(from: Vec2, to: Vec2) -> VecDeque<Vec2> {
    let steps = ((to - from).length() / navtile_size()).ceil().max(1.0) as i32;
    let mut path = VecDeque::new();
    let mut previous = None;
    for step in 1..=steps {
        let point = from.lerp(to, step as f32 / steps as f32);
        let tile = world_to_tile(point);
        if previous != Some(tile) {
            path.push_back(tile_center(tile));
            previous = Some(tile);
        }
    }
    path
}

// -------------------------------------------------------------------- ввод

/// Выбор сценария цифрами. Один системный обработчик на пять клавиш, а не пять
/// систем с `run_if`: это переключатель, а не пять разных действий.
fn pick_scenario(keys: Res<ButtonInput<KeyCode>>, mut scenario: ResMut<Scenario>) {
    for (key, next) in [
        (KeyCode::Digit1, Scenario::Pile),
        (KeyCode::Digit2, Scenario::Funnel),
        (KeyCode::Digit3, Scenario::Columns),
        (KeyCode::Digit4, Scenario::Corridor),
        (KeyCode::Digit5, Scenario::Wander),
    ] {
        if keys.just_pressed(key) && *scenario != next {
            *scenario = next;
        }
    }
}

fn toggle_separation(mut style: ResMut<SeparationStyle>) {
    // менять `SeparationStyle` из примера безопасно ровно потому, что здесь нет
    // `SettingsPlugin`: это `SettingsGroup`, и с `PrefsPlugin` тумблер уехал бы
    // в конфиг игры
    style.enabled = !style.enabled;
}

fn toggle_pause(mut time: ResMut<Time<Virtual>>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
}

/// Скорость по лестнице — своей, а не через `SimTimePlugin`: губернатор кадров
/// из игры срезал бы запрошенные 30× и смазал бы сравнение «пар на 1× против
/// пар на 30×».
/// Возвращает новую ступень, а не пишет в ресурс: запись через `ResMut`
/// пометила бы `DemoSpeed` изменённым каждый кадр, и гейт `resource_changed` у
/// [`apply_speed`] перестал бы что-либо значить.
fn step_speed(keys: &ButtonInput<KeyCode>, config: &DemoConfig, current: f32) -> Option<f32> {
    let up = keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd);
    let down = keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract);
    if !up && !down {
        return None;
    }
    let index = config
        .speeds
        .iter()
        .position(|step| (step - current).abs() < 0.01)
        .unwrap_or(0);
    let next = if up {
        (index + 1).min(config.speeds.len() - 1)
    } else {
        index.saturating_sub(1)
    };
    Some(config.speeds[next])
}

fn apply_speed(speed: Res<DemoSpeed>, mut time: ResMut<Time<Virtual>>) {
    time.set_relative_speed(speed.0);
}

/// Зум колесом. Панорамирования нет намеренно: панелей в сцене нет, но правило
/// «мышь над UI не двигает мир» проще соблюсти, не заводя перетаскивание.
fn zoom_camera(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<DemoConfig>,
    scroll: Res<AccumulatedMouseScroll>,
    mut speed: ResMut<DemoSpeed>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    // `bypass_change_detection` — чтобы чтение текущей ступени не считалось
    // изменением: иначе `DemoSpeed` менялся бы каждый кадр
    if let Some(next) = step_speed(&keys, &config, speed.bypass_change_detection().0) {
        speed.0 = next;
    }

    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
    };
    if lines == 0.0 {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let zoom = (transform.scale.x * 1.2f32.powf(-lines)).clamp(config.min_zoom, config.max_zoom);
    transform.scale = Vec3::splat(zoom);
}

// ------------------------------------------------------------------ метрики

fn count_ticks(mut counters: ResMut<RunCounters>) {
    counters.window_ticks += 1;
}

/// Прогон расталкивания виден по свежему замеру `sim/separation_ms`: система
/// пишет его последним действием, ровно раз за прогон.
fn count_separation_runs(diagnostics: Res<DiagnosticsStore>, mut counters: ResMut<RunCounters>) {
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
fn measure_overlaps(
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
fn draw_bodies(mut gizmos: Gizmos, overlaps: Res<Overlaps>) {
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
fn sample_trial(
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
fn sample_travel(
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
struct ProgressSample {
    target: Option<IVec2>,
    distance: f32,
}

/// Снимок экрана в начале, середине и конце окна: телепорт и проход насквозь
/// числами ловятся не полностью, и отчёт без картинок не проверить.
fn take_shots(mut commands: Commands, mut trial: ResMut<Trial>) {
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
fn median(values: &mut [f32]) -> f32 {
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

fn slug(label: &str) -> String {
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
fn finish_trial(
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
    let side = slot_side(style.body_radius * 2.0) + lab.slack;
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
fn report_to_stdout(
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

#[allow(clippy::too_many_arguments)]
fn update_overlay(
    scenario: Res<Scenario>,
    style: Res<SeparationStyle>,
    speed: Res<DemoSpeed>,
    overlaps: Res<Overlaps>,
    holds: Res<SeparationHolds>,
    counters: Res<RunCounters>,
    misses: Res<PathMisses>,
    polymesh: Res<PolymeshDebug>,
    poly: Res<PolyNavmesh>,
    time: Res<Time<Virtual>>,
    camera: Query<&Transform, With<Camera2d>>,
    mut overlay: Query<&mut Text, With<OverlayText>>,
) {
    let zoom = camera.single().map(|camera| camera.scale.x).unwrap_or(0.0);
    let gated = zoom >= SEPARATION_MAX_ZOOM;
    // тот же вопрос, что решает `Pathfinder::polymesh_build`: тумблер плюс
    // готовность меша
    let navigation = match (polymesh.enabled, poly.build().is_some()) {
        (true, true) => "polymesh",
        (true, false) => "polymesh (building, walking the grid)",
        (false, _) => "navmesh grid",
    };
    // клавиши переключить бэкенд здесь нет, но по BRP тумблер достижим — а на
    // сеточной навигации расталкивания не бывает вовсе, и подпись обязана это
    // говорить, а не показывать `ON` у выключенной системы.
    // Детерминизма в этой сцене нет по построению (`Determinism` не вставлен)
    let mode_off = !separation_allowed_by_mode(false, polymesh.enabled);
    let share = if overlaps.pawns > 0 {
        overlaps.involved as f32 / overlaps.pawns as f32 * 100.0
    } else {
        0.0
    };

    let text = format!(
        "{scenario}\n\
         pawns in view {pawns} of {total}   overlapping pairs {pairs}   involved {involved} ({share:.0}%)   held {held}\n\
         worst {worst:.3} m   mean {mean:.3} m   (rest distance {rest:.2} m, sprite {sprite:.2} m)\n\
         separation {separation}{gate}   speed {speed:.0}x{paused}   zoom {zoom:.3}\n\
         move ticks per separation run {per_run}   runs {runs}\n\
         navigation {navigation}   path misses {misses}\n\
         \n\
         1-5 scenario   R respawn   S separation   Space pause   -/= speed   wheel zoom",
        scenario = scenario.label(),
        pawns = overlaps.pawns,
        total = overlaps.total,
        pairs = overlaps.pairs,
        involved = overlaps.involved,
        share = share,
        held = holds.0.len(),
        worst = overlaps.worst,
        mean = overlaps.mean,
        rest = overlaps.radius * 2.0,
        sprite = HUMAN_SIZE,
        separation = if style.enabled && !mode_off {
            "ON"
        } else {
            "OFF"
        },
        gate = if mode_off {
            " (grid nav: no separation)"
        } else if gated {
            " (zoomed out: gated)"
        } else {
            ""
        },
        speed = speed.0,
        paused = if time.is_paused() { " PAUSED" } else { "" },
        zoom = zoom,
        per_run = if counters.ticks_per_run.is_finite() {
            format!("{:.1}", counters.ticks_per_run)
        } else {
            "-".to_string()
        },
        runs = counters.runs,
        navigation = navigation,
        misses = misses.0,
    );

    for mut overlay in &mut overlay {
        overlay.0 = text.clone();
    }
}
