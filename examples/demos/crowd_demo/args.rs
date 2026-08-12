//! Разбор командной строки демо-сцены: что можно задать флагом вместо
//! клавиши, и как ручки стенда (`--lab`) доезжают до `SeparationLab`.

use qwe::movement::{SeparationLab, SlotMatching};

use crate::scenario::Scenario;

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
pub(crate) struct Args {
    pub(crate) scenario: Option<Scenario>,
    pub(crate) speed: Option<f32>,
    /// Длина окна замера в РЕАЛЬНЫХ секундах. По истечении сцена печатает
    /// строку `RESULT` и выходит сама — держать её живой нечем и незачем.
    pub(crate) seconds: Option<f32>,
    /// Пешек на сторону (`columns`/`corridor`) или всего (остальные).
    pub(crate) pawns: Option<usize>,
    /// Поперечный разброс колонн, м. 0 — обе колонны в одну линию, как было.
    pub(crate) width: Option<f32>,
    /// Шаг вдоль колонны, м — он же плотность стартовой раскладки.
    pub(crate) spacing: Option<f32>,
    pub(crate) zoom: Option<f32>,
    pub(crate) seed: Option<u64>,
    pub(crate) separation: Option<bool>,
    /// Подпись прогона в строке `RESULT` — по ней отчёт и собирается.
    pub(crate) label: Option<String>,
    /// Снимать экран в начале, середине и конце окна: артефакты (телепорт,
    /// проход насквозь) числами не ловятся до конца.
    pub(crate) shots: bool,
    pub(crate) radius: Option<f32>,
    /// Радиус поиска свободного слота, м ([`SlotSearch`]). Ручка стенда наравне
    /// с радиусом тела: у неё нет правильного значения, есть компромисс между
    /// «хвост толпы остался без слотов» и «цель уехала слишком далеко».
    pub(crate) search: Option<f32>,
    /// Как пачка пешек, идущих в одну точку, разбирает слоты ([`SlotMatching`]).
    pub(crate) matching: Option<SlotMatching>,
    /// Лишние навтайлы к шагу решётки слотов ([`SlotLab::slack`]).
    pub(crate) slot_slack: Option<i32>,
    /// Ближе какого расстояния до цели выдаётся слот ([`SlotLab::claim_at`]).
    pub(crate) claim_at: Option<f32>,
    /// На сколько метров можно столкнуть осевшую пешку, прежде чем она пойдёт
    /// обратно на свой слот ([`SlotLab::regroup`]).
    pub(crate) regroup: Option<f32>,
    pub(crate) hold: Option<f32>,
    pub(crate) sidestep: Option<f32>,
    pub(crate) backstep: Option<f32>,
    pub(crate) lab: Vec<(String, f32)>,
}

/// Ручки [`SeparationLab`], доступные с командной строки. Список здесь, а не
/// `match` по строке в трёх местах: подпись в `RESULT`, разбор и справка обязаны
/// перечислять одно и то же.
pub(crate) const LAB_KNOBS: [&str; 21] = [
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

pub(crate) fn parse_args() -> Args {
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

pub(crate) fn parse_number<T: std::str::FromStr>(raw: &str) -> T {
    raw.parse()
        .unwrap_or_else(|_| panic!("{raw} is not a number"))
}

pub(crate) fn parse_matching(raw: &str) -> SlotMatching {
    match raw {
        "greedy" | "0" => SlotMatching::Greedy,
        "batch" | "1" => SlotMatching::Batch,
        other => panic!("unknown matching {other}; use greedy or batch"),
    }
}

pub(crate) fn parse_scenario(raw: &str) -> Scenario {
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
pub(crate) fn apply_lab(lab: &mut SeparationLab, knobs: &[(String, f32)]) {
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
