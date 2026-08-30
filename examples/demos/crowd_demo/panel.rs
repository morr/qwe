//! Панель механизмов: ручки расталкивания ползунками, пресеты кнопками и
//! плашка с числами прогона.
//!
//! **Оформление — игровое, целиком.** Плашки красит `panel_background()` (тот
//! же полупрозрачный токен темы, что и панели игры), подписи — `row_label` /
//! `row_value`, кнопки и ползунки приходят китами из `qwe::ui`. Своих цветов
//! сцена не заводит: панель, покрашенная мимо темы, врёт про то, как это
//! выглядит в игре, а смотрят на неё ровно за этим.
//!
//! **Шрифт плашки ставят себе сами.** В игре его вешает `apply_panel_font` по
//! `Added<GameUiRoot>`, но эта система живёт в `UiPlugin`, которого здесь нет —
//! а без `InheritableFont` подписи достаются дефолтному шрифту bevy в 20 px, и
//! строки-ползунки вылезают за край плашки (ровно этим сцена и выглядела после
//! переезда на feathers).

use bevy::feathers::constants::{fonts, size};
use bevy::feathers::font_styles::InheritableFont;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::{SliderValue, ValueChange};
use qwe::human::HumanStyle;
use qwe::movement::{
    SeparationHolds, SeparationLab, SeparationStyle, SlotLab, SlotMatching, SlotSearch,
    separation_allowed_by_mode,
};
use qwe::navigation::{MeshMode, NavMode, Pathfinder};
use qwe::settings::{HUMAN_SIZE, SEPARATION_MAX_ZOOM};
use qwe::ui::slider::{SliderRow, quantize, spawn_slider_row};
use qwe::ui::{
    PANEL_FONT, PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET, panel_background, panel_block_background,
    panel_title, row_label, row_value, ui_node, ui_row,
};

use crate::DemoSpeed;
use crate::metrics::{Overlaps, PathMisses, RunCounters};
use crate::scenario::Scenario;

#[derive(Component)]
pub(crate) struct OverlayText;

/// Пределы ползунка радиуса: от «меньше половины спрайта» (как было до правки)
/// до заведомо избыточного личного пространства.
pub(crate) const RADIUS_MIN: f32 = 0.3;
pub(crate) const RADIUS_MAX: f32 = 1.2;
pub(crate) const RADIUS_STEP: f32 = 0.01;

#[derive(Component)]
pub(crate) struct RadiusSlider;

#[derive(Component)]
pub(crate) struct RadiusValueLabel;

/// Пределы ползунка радиуса поиска слота. Снизу — меньше, чем нужно даже
/// десятку пешек, чтобы было видно, как хвост толпы остаётся без слотов и
/// сваливается в общую точку; сверху — вчетверо больше дефолта: 40 м на шаге
/// 2 м это 21 × 21 слот, с запасом на всю «воронку».
pub(crate) const SEARCH_MIN: f32 = 2.0;
pub(crate) const SEARCH_MAX: f32 = 40.0;
pub(crate) const SEARCH_STEP: f32 = 1.0;

#[derive(Component)]
pub(crate) struct SearchSlider;

#[derive(Component)]
pub(crate) struct SearchValueLabel;

/// Все ручки стенда, которые есть смысл крутить глазами, — одним списком.
///
/// Список, а не пятнадцать почти одинаковых функций: у каждой ручки одно и то же
/// поведение (подпись, диапазон, чтение из ресурса, запись в ресурс), и
/// расходиться этим копиям незачем. Тот же список кормит и панель, и её
/// обновление после нажатия пресета.
pub(crate) struct Knob {
    pub(crate) label: &'static str,
    /// `(min, max, шаг)`, как у ползунков игры.
    pub(crate) range: (f32, f32, f32),
    /// Сколько знаков после запятой в подписи значения.
    pub(crate) digits: usize,
    pub(crate) get: fn(&Tuning) -> f32,
    pub(crate) set: fn(&mut Tuning, f32),
}

/// Ресурсы, которые крутит панель, — одним параметром: в системе их иначе
/// набирается столько, что не остаётся места на запросы.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Tuning<'w> {
    pub(crate) lab: ResMut<'w, SeparationLab>,
    pub(crate) slots: ResMut<'w, SlotLab>,
    pub(crate) style: ResMut<'w, SeparationStyle>,
}

pub(crate) const KNOBS: &[Knob] = &[
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
pub(crate) struct Preset {
    pub(crate) label: &'static str,
    pub(crate) apply: fn(&mut Tuning),
}

pub(crate) const PRESETS: &[Preset] = &[
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
pub(crate) struct KnobSlider(pub(crate) usize);

/// Подпись значения той же ручки.
#[derive(Component)]
pub(crate) struct KnobValueLabel(pub(crate) usize);

/// Отступ содержимого от края плашки и зазор между строками — как в теле
/// панели настроек игры (`ui/shell.rs`).
const PLAQUE_PAD_PX: f32 = 6.0;
const ROW_GAP_PX: f32 = 4.0;

/// Отступ заголовка группы — по отступу строки-значения, чтобы подписи
/// заголовка и строк стояли в одну вертикаль.
const GROUP_HEADER_PAD_PX: f32 = 8.0;

/// Шрифт плашки — игровой [`PANEL_FONT`]; ставится вручную, см. шапку модуля.
fn panel_font(assets: &AssetServer) -> InheritableFont {
    InheritableFont {
        font: assets.load(fonts::REGULAR),
        font_size: PANEL_FONT,
        weight: FontWeight::NORMAL,
    }
}

/// Прозрачная колонка у края экрана — форма панели игры: тянется до низа, чтобы
/// плашке в ней было куда упереться и от чего ужиматься, но сама ничего не
/// красит и кликов не ловит (она выше своего содержимого, и невидимая её часть
/// не должна отбирать у карты протяжки).
fn column_node(node: Node) -> impl Bundle {
    (
        ui_node(Node {
            position_type: PositionType::Absolute,
            top: px(UI_SCREEN_EDGE_PX_OFFSET),
            bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
            ..node
        }),
        Pickable::IGNORE,
    )
}

/// Плашка в колонке: полупрозрачное тело панели игры, ужимающееся под высоту
/// колонки. `min_height: 0` — разрешение флексу это сделать.
fn plaque_node(node: Node) -> Node {
    Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: px(ROW_GAP_PX),
        padding: UiRect::all(px(PLAQUE_PAD_PX)),
        overflow: Overflow::scroll_y(),
        flex_shrink: 1.,
        min_height: px(0),
        ..node
    }
}

/// Заголовок группы строк — плашка потемнее с названием, как заголовок секции
/// в панели настроек игры.
fn group_header(title: &'static str) -> impl Bundle {
    (
        ui_node(Node {
            padding: UiRect::axes(px(GROUP_HEADER_PAD_PX), px(2.)),
            ..default()
        }),
        panel_block_background(),
        children![panel_title(title)],
    )
}

/// Панель механизмов справа: кнопки пресетов сверху, под ними ползунок на
/// каждую ручку.
pub(crate) fn spawn_mechanism_panel(
    mut commands: Commands,
    assets: Res<AssetServer>,
    tuning: Tuning,
) {
    let column = commands
        .spawn((
            column_node(Node {
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
                align_items: AlignItems::FlexEnd,
                ..default()
            }),
            Name::new("demo_right_column"),
        ))
        .id();

    let panel = commands
        .spawn((
            ui_node(plaque_node(Node {
                width: px(PANEL_WIDTH_PX),
                ..default()
            })),
            panel_background(),
            panel_font(&assets),
            Name::new("mechanism_panel"),
            ChildOf(column),
            children![group_header("Presets")],
        ))
        .id();

    let presets = commands.spawn((ui_row(ROW_GAP_PX), ChildOf(panel))).id();
    for (index, preset) in PRESETS.iter().enumerate() {
        let button = qwe::ui::spawn_panel_button(
            &mut commands,
            presets,
            PresetButton,
            preset.label,
            false,
            move |_activate: On<bevy::ui_widgets::Activate>, mut tuning: Tuning| {
                (PRESETS[index].apply)(&mut tuning);
            },
        );
        // кнопки делят ширину строки поровну — как вкладки в полоске панели
        // игры: по содержимому четыре пресета встают в 260 px впритык, и
        // «funnel» уезжал бы за край плашки
        commands
            .entity(button)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.flex_grow = 1.;
                node.flex_basis = px(0.);
                node.padding = UiRect::horizontal(px(4.));
            });
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
pub(crate) struct PresetButton;

/// Подтянуть ползунки и подписи к ресурсам: пресет меняет по десятку ручек
/// разом, и без этого панель показывала бы то, чего в ресурсах уже нет.
pub(crate) fn sync_knob_rows(
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

/// Клавиши сцены — подпись под числами прогона, приглушённая, как справка по
/// хоткеям в игре.
const HOTKEYS: &str =
    "1-5 scenario   R respawn   S separation   Space pause   -/= speed   wheel zoom";

/// Левая колонка сцены: плашка с числами прогона, под ней две ручки.
///
/// Одной системой и одной колонкой, а не двумя плашками с посчитанным вручную
/// `top`: числа занимают то шесть строк, то семь, и разъезжающиеся плашки — это
/// ровно то, чем сцена выглядела до перехода на игровые стили. Колонка ставит
/// их друг под друга сама.
///
/// Ползунки — тот же кит строки-ползунка, что у панелей игры
/// (`qwe::ui::slider`), чтобы обе величины подбирались глазом на живой толпе, а
/// не пересборкой. Пишут прямо в ресурсы движения, откуда их берут и сама
/// механика, и гизмо этой сцены: `HumanStyle::body_radius` — «личное
/// пространство», `SlotSearch` — докуда искать свободный слот назначения.
/// Обе ручки есть и в панелях игры (`ui/navigation`) — эта сцена не заводит своих.
pub(crate) fn spawn_left_column(
    mut commands: Commands,
    assets: Res<AssetServer>,
    style: Res<HumanStyle>,
    search: Res<SlotSearch>,
) {
    let column = commands
        .spawn((
            column_node(Node {
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                align_items: AlignItems::FlexStart,
                ..default()
            }),
            Name::new("demo_left_column"),
        ))
        .id();

    commands.spawn((
        ui_node(plaque_node(Node::default())),
        panel_background(),
        // моноширинный и помельче — как панель телеметрии игры: числа
        // перекрытий меняются каждый кадр, и на пропорциональном шрифте цифры
        // под собой пляшут
        InheritableFont {
            font: assets.load(fonts::MONO),
            font_size: size::SMALL_FONT,
            weight: FontWeight::NORMAL,
        },
        Name::new("demo_metrics"),
        ChildOf(column),
        children![(OverlayText, row_value("")), row_label(HOTKEYS)],
    ));

    let panel = commands
        .spawn((
            ui_node(plaque_node(Node {
                width: px(PANEL_WIDTH_PX),
                ..default()
            })),
            panel_background(),
            panel_font(&assets),
            Name::new("demo_crowd_knobs"),
            ChildOf(column),
            children![group_header("Crowd")],
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_overlay(
    scenario: Res<Scenario>,
    style: Res<SeparationStyle>,
    speed: Res<DemoSpeed>,
    overlaps: Res<Overlaps>,
    holds: Res<SeparationHolds>,
    counters: Res<RunCounters>,
    misses: Res<PathMisses>,
    pathfinder: Pathfinder,
    time: Res<Time<Virtual>>,
    camera: Query<&Transform, With<Camera2d>>,
    mut overlay: Query<&mut Text, With<OverlayText>>,
) {
    let zoom = camera.single().map(|camera| camera.scale.x).unwrap_or(0.0);
    let gated = zoom >= SEPARATION_MAX_ZOOM;
    // подпись читает то же значение, что и симуляция. Раньше здесь стоял свой
    // `match` по паре сырых ресурсов — потому что `polymesh_build()` отдавал
    // один `None` и на «выключен», и на «ещё строится», а подписи их надо
    // различать. `NavMode` их различает сам
    let mode = pathfinder.mode();
    let navigation = match &mode {
        NavMode::Mesh(MeshMode::Ready(_)) => "polymesh",
        NavMode::Mesh(MeshMode::Pending) => "polymesh (building, walking the grid)",
        NavMode::Grid(_) => "navmesh grid",
    };
    // клавиши переключить бэкенд здесь нет, но по BRP тумблер достижим — а на
    // сеточной навигации расталкивания не бывает вовсе, и подпись обязана это
    // говорить, а не показывать `ON` у выключенной системы.
    // Детерминизма в этой сцене нет по построению (`Determinism` не вставлен).
    // «Непрерывно» — тумблер меша, не готовность: и строящийся меш
    // непрерывен (см. `navigation::ContinuousSpace`)
    let mode_off = !separation_allowed_by_mode(false, matches!(mode, NavMode::Mesh(_)));
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
         navigation {navigation}   path misses {misses}",
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
