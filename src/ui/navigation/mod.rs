//! Панель Navigation — один UI на обе подсистемы поиска пути: сеточный
//! **Navmesh** (`navigation/navmesh.rs`, заливка непроходимых тайлов) и
//! полигональный **Polymesh** (`navigation/polymesh/`, меш polyanya).
//!
//! Две верхние строки — переключатель бэкенда, а не два независимых тумблера:
//! пешки всегда ходят по чему-то одному, и состояние «обе выключены» было бы
//! неправдой — сетка обслуживала бы поиск молча. Поэтому клик по любой из них
//! перебрасывает выбор: строка со значением `On` гаснет, вторая загорается.
//! Единственный источник истины — `PolymeshDebug::enabled`; строка `Navmesh`
//! показывает его отрицание. По умолчанию выбран `Polymesh`.
//!
//! У этого выбора есть последствие за пределами поиска пути: на сеточном
//! бэкенде не работает расталкивание (`movement::separation_runs` — waypoint'ы
//! стоят в центрах навтайлов, разводить пешки некуда), и строка `Separation`
//! ниже гаснет так же, как под детерминизмом.
//!
//! Настройки каждой подсистемы живут под её строкой и **видны только пока она
//! выбрана** (`Display::None`; строки над и под ней сдвигает обычный флекс
//! вкладки): ползунок радиуса агента ничего не значит, пока ходят по сетке, а
//! алгоритм поиска по ней — пока ходят по мешу.
//!
//! - `Navmesh` → `Pathfind` (алгоритм поиска), `Show` (сеточный оверлей, он же
//!   `DebugNavmesh`);
//! - `Polymesh` → `Show` (оверлей меша), `Chunks` (иерархия чанков: и постройка
//!   слоями, и их границы на оверлее), `Agent radius` (инфляция препятствий).
//!
//! Размера навтайла здесь нет, хотя он и похож на настройку сетки: в тайлах
//! этого размера мир строится при любом бэкенде (см. `ui/debug.rs`), поэтому
//! кнопка `navtile:` стоит в ряду дебаг-кнопок и не гаснет вместе с этой
//! секцией.
//!
//! # Группы Separation и Slots
//!
//! Нижняя половина панели — две группы ([`KnobGroup`]) про то, как пешки
//! расходятся по дороге и как делят конечные точки: **Separation**
//! (`movement/separation/`) и **Slots** (`movement/destination.rs`). Обе — про
//! перемещение, поэтому живут здесь, а не в World: World отвечает за прогон
//! целиком (seed, детерминизм, счётчики), и ручки толпы в нём стояли только
//! потому, что механизм видо-независимый.
//!
//! Группы, а не подвкладки: ручки толпы подбираются вместе, и спрятанная
//! половина заставляла бы щёлкать туда-сюда посреди подбора. Прятать имеет
//! смысл не «вторую половину», а то, что при нынешних настройках ни на что не
//! влияет, — так уходят настройки невыбранного бэкенда выше и ползунки
//! выключенного расталкивания ниже.
//!
//! Заголовок `Separation` — сам тумблер, ровно как строка `Algo` выше:
//! `on`/`off` справа, приглушённое и неоткликающееся под детерминизмом и на
//! сетке. У слотов тумблера нет — они работают всегда и в обоих режимах, — так
//! что `Slots` просто подпись группы, а не кнопка.
//!
//! Ползунки Separation прячутся, когда расталкивание не работает (детерминизм,
//! сеточный бэкенд, свой `off`): настраивать нечего, пока механизм не
//! запускается вовсе — та же логика, по которой уходят настройки невыбранного
//! бэкенда. Заголовок при этом остаётся: он и есть тумблер, которым
//! расталкивание возвращают.
//!
//! Радиус тела — в группе Slots, хотя ресурс у него человеческий
//! (`HumanStyle::body_radius`): он задаёт и дистанцию покоя, и сторону слота,
//! то есть при подборе толпы нужен рядом с остальными ручками, а не в панели
//! Human через полэкрана.
//!
//! Оверлей polymesh рисует **все** рёбра полигонов построенного меша одним
//! merged-мешем — по нему видно и контуры препятствий, и как polyanya разбила
//! проходимое пространство.

use bevy::ecs::system::{IntoObserverSystem, SystemParam};
use bevy::feathers::theme::{ThemeToken, UiTheme};
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::determinism::Determinism;
use crate::human::HumanStyle;
use crate::loading::{AppState, WorldInitSet};
use crate::movement::{SeparationLab, SeparationStyle, SlotSearch, separation_allowed_by_mode};
use crate::navigation::{PathfindingAlgorithm, PolyNavmesh, PolymeshDebug};
use crate::settings::{
    POLYMESH_AGENT_RADIUS_MAX, POLYMESH_AGENT_RADIUS_MIN, POLYMESH_AGENT_RADIUS_STEP,
};
use crate::ui::knob::{AddKnobsExt, SliderBinding, spawn_knob};
use crate::ui::rows::{ROW_LEFT_PX, on_off, spawn_value_row};
use crate::ui::shell::{SectionSlot, SettingsPanes, SettingsTab, spawn_block};
use crate::ui::{DebugNavmesh, UiBuildSet, panel_block_background, row_label};

mod knobs;
mod overlay;

// Приватные реэкспорты: снаружи модуль виден тем же набором имён, что и до
// разрезания.
use self::knobs::{
    KnobGroup, SeparationToggleRow, spawn_knob_rows, sync_separation_knob_visibility,
    sync_separation_row_inert,
};
use self::overlay::sync_polymesh_overlay;

/// Отступ слева у строк-настроек: настройка принадлежит подсистеме над ней, и
/// лесенка говорит это раньше, чем читается подпись.
const NESTED_ROW_INDENT_PX: f32 = 18.;

/// Какой подсистеме принадлежит строка-настройка. Строка `Algo` компонент не
/// несёт: она видна всегда, она и выбирает подсистему.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum NavSection {
    Navmesh,
    Polymesh,
}

impl NavSection {
    fn label(self) -> &'static str {
        match self {
            Self::Navmesh => "Navmesh",
            Self::Polymesh => "Polymesh",
        }
    }
}

/// Текст значения справа и то, что именно он показывает. Один компонент на
/// все строки, а не маркер на каждую: отдельные `&mut Text`-запросы пришлось
/// бы разводить `Without` каждый с каждым, и новая строка ломала бы все
/// прежние.
#[derive(Component, Clone, Copy)]
enum NavValueLabel {
    Backend,
    Pathfind,
    NavmeshShow,
    PolymeshShow,
    PolymeshChunks,
    Separation,
}

impl NavValueLabel {
    fn text(self, values: &NavPanelValues) -> String {
        let poly = &values.polymesh;
        match self {
            Self::Backend => active_section(poly).label().to_string(),
            Self::Pathfind => values.algorithm.label().to_string(),
            Self::NavmeshShow => enabled_text(values.navmesh_show.0),
            Self::PolymeshShow => enabled_text(poly.show),
            Self::PolymeshChunks => enabled_text(poly.chunks),
            // под детерминизмом и на сеточной навигации расталкивания нет
            // вовсе (`movement::separation_runs`), каким бы ни был
            // `SeparationStyle`, — панель обязана показывать положение дел, а
            // не намерение. Собственное значение стиля при этом сохраняется:
            // возврат режима вернёт панель к нему
            Self::Separation => enabled_text(values.separation_enabled()),
        }
    }

    /// Токен цвета подписи, если он у строки не основной по умолчанию.
    fn color(self, values: &NavPanelValues) -> Option<ThemeToken> {
        match self {
            Self::Separation if !values.separation_allowed() => Some(tokens::TEXT_DIM),
            Self::Separation => Some(tokens::TEXT_MAIN),
            _ => None,
        }
    }
}

/// Всё, что читают подписи панели. Отдельным `SystemParam`, чтобы система
/// синхронизации и спавн панели брали одно и то же, а не расходились списком
/// аргументов.
#[derive(SystemParam)]
struct NavPanelValues<'w> {
    polymesh: Res<'w, PolymeshDebug>,
    navmesh_show: Res<'w, DebugNavmesh>,
    algorithm: Res<'w, PathfindingAlgorithm>,
    separation: Res<'w, SeparationStyle>,
    determinism: Res<'w, Determinism>,
    separation_lab: Res<'w, SeparationLab>,
    human: Res<'w, HumanStyle>,
    search: Res<'w, SlotSearch>,
}

impl NavPanelValues<'_> {
    // ручки читают свои ресурсы поодиночке: привязка каждой знает только тот
    // ресурс, который правит (`knobs.rs`)
    pub(super) fn separation_lab(&self) -> &SeparationLab {
        &self.separation_lab
    }

    pub(super) fn human(&self) -> &HumanStyle {
        &self.human
    }

    pub(super) fn search(&self) -> &SlotSearch {
        &self.search
    }

    /// Работает ли расталкивание при нынешнем режиме — см.
    /// `movement::separation_allowed_by_mode`.
    fn separation_allowed(&self) -> bool {
        separation_allowed_by_mode(self.determinism.0, self.polymesh.enabled)
    }

    /// Работает ли оно на самом деле: и разрешено режимом, и включено.
    fn separation_enabled(&self) -> bool {
        self.separation_allowed() && self.separation.enabled
    }
}

/// Выбранный бэкенд. Единственный источник истины — `PolymeshDebug::enabled`:
/// пешки всегда ходят по чему-то одному, и второй флаг мог бы с ним разойтись.
fn active_section(poly: &PolymeshDebug) -> NavSection {
    if poly.enabled {
        NavSection::Polymesh
    } else {
        NavSection::Navmesh
    }
}

pub struct UiNavigationPlugin;

impl Plugin for UiNavigationPlugin {
    fn build(&self, app: &mut App) {
        // ручки этой панели правят три разных ресурса, и радиус агента —
        // четвёртый; `HumanStyle` регистрирует ещё и панель Human, повторный
        // вызов кит отбрасывает сам
        app.add_knobs::<SeparationLab>()
            .add_knobs::<SlotSearch>()
            .add_knobs::<HumanStyle>()
            .add_knobs::<PolymeshDebug>()
            .add_systems(Startup, build_navigation_tab.in_set(UiBuildSet::Sections))
            // после смены города: оверлей умер с DespawnOnExit, ресурсы живы
            .add_systems(
                OnEnter(AppState::Playing),
                sync_polymesh_overlay.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    // именно `resource_changed`: начальное состояние метки
                    // ставит спавн панели, а первый кадр смены не видит
                    sync_separation_row_inert.run_if(
                        resource_changed::<Determinism>.or_else(resource_changed::<PolymeshDebug>),
                    ),
                    (
                        sync_nav_values,
                        sync_section_visibility,
                        sync_separation_knob_visibility,
                    )
                        .run_if(
                            resource_changed::<PolymeshDebug>
                                .or_else(resource_changed::<DebugNavmesh>)
                                .or_else(resource_changed::<PathfindingAlgorithm>)
                                // подпись расталкивания зависит и от режима
                                .or_else(resource_changed::<SeparationStyle>)
                                .or_else(resource_changed::<Determinism>),
                        ),
                    // PolyNavmesh меняется ровно в момент снятия готового
                    // меша с таска — тогда оверлей и появляется
                    sync_polymesh_overlay
                        .run_if(in_state(AppState::Playing))
                        .run_if(
                            resource_changed::<PolymeshDebug>
                                .or_else(resource_changed::<PolyNavmesh>),
                        ),
                ),
            );
    }
}

fn build_navigation_tab(mut commands: Commands, panes: Res<SettingsPanes>, values: NavPanelValues) {
    // одним блоком без заголовка: заголовки этой вкладки — сами строки
    // (`Algo`, тумблер `Separation`, подпись `Slots`), и общая шапка над ними
    // повторяла бы имя вкладки
    let panel = spawn_block(
        &mut commands,
        panes.pane(SettingsTab::Nav),
        SectionSlot::Navigation,
        "navigation_rows",
    );

    // выбор бэкенда — одна строка на оба: клик листает `Navmesh` ⇄ `Polymesh`
    spawn_row(
        &mut commands,
        panel,
        "Algo",
        RowStyle::Backend,
        NavValueLabel::Backend,
        active_section(&values.polymesh).label().to_string(),
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.enabled = !debug.enabled;
        },
    );

    // настройки сеточной навигации
    spawn_row(
        &mut commands,
        panel,
        "Pathfind",
        RowStyle::Setting(NavSection::Navmesh),
        NavValueLabel::Pathfind,
        values.algorithm.label().to_string(),
        |_activate: On<Activate>, mut algorithm: ResMut<PathfindingAlgorithm>| {
            *algorithm = algorithm.next();
        },
    );
    spawn_row(
        &mut commands,
        panel,
        "Show",
        RowStyle::Setting(NavSection::Navmesh),
        NavValueLabel::NavmeshShow,
        enabled_text(values.navmesh_show.0),
        |_activate: On<Activate>, mut show: ResMut<DebugNavmesh>| {
            show.0 = !show.0;
        },
    );
    // настройки полигональной навигации
    spawn_row(
        &mut commands,
        panel,
        "Show",
        RowStyle::Setting(NavSection::Polymesh),
        NavValueLabel::PolymeshShow,
        enabled_text(values.polymesh.show),
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.show = !debug.show;
        },
    );
    spawn_row(
        &mut commands,
        panel,
        "Chunks",
        RowStyle::Setting(NavSection::Polymesh),
        NavValueLabel::PolymeshChunks,
        enabled_text(values.polymesh.chunks),
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.chunks = !debug.chunks;
        },
    );

    let radius_row = spawn_knob(
        &mut commands,
        panel,
        "Agent radius",
        &*values.polymesh,
        SliderBinding::<PolymeshDebug> {
            get: |debug| debug.radius(),
            set: |debug, value| debug.agent_radius = value,
            range: (
                POLYMESH_AGENT_RADIUS_MIN,
                POLYMESH_AGENT_RADIUS_MAX,
                POLYMESH_AGENT_RADIUS_STEP,
            ),
            text: |value| format!("{value:.1} m"),
        },
    );
    commands.entity(radius_row).insert(NavSection::Polymesh);
    indent_slider_row(&mut commands, radius_row);

    // --- группа Separation: её заголовок и есть тумблер, как строка `Algo` ---
    let toggle_row = spawn_row(
        &mut commands,
        panel,
        "Separation",
        RowStyle::Backend,
        NavValueLabel::Separation,
        enabled_text(values.separation_enabled()),
        |_activate: On<Activate>,
         mut style: ResMut<SeparationStyle>,
         determinism: Res<Determinism>,
         polymesh: Res<PolymeshDebug>| {
            // под детерминизмом и на сеточной навигации расталкивания нет
            // вовсе — тумблер не должен молча переключать то, что всё равно
            // не работает
            if !separation_allowed_by_mode(determinism.0, polymesh.enabled) {
                return;
            }
            style.enabled = !style.enabled;
        },
    );
    commands.entity(toggle_row).insert(SeparationToggleRow);
    // начальное состояние неотзывчивости: `sync_separation_row_inert` ходит по
    // `resource_changed`, а на первом кадре ничего ещё не менялось
    if !separation_allowed_by_mode(values.determinism.0, values.polymesh.enabled) {
        commands
            .entity(toggle_row)
            .insert(bevy::ui::InteractionDisabled);
    }
    spawn_knob_rows(&mut commands, panel, &values, KnobGroup::Separation);

    // --- группа Slots: тумблера у них нет, заголовок просто подпись ---
    spawn_group_label(&mut commands, panel, "Slots");
    spawn_knob_rows(&mut commands, panel, &values, KnobGroup::Slots);
}

/// Подпись группы: та же полоса, что у строк-кнопок, но без `Button` и
/// наблюдателя — нажимать в ней нечего.
fn spawn_group_label(commands: &mut Commands, panel: Entity, label: &str) {
    let row = commands
        .spawn((
            crate::ui::ui_node(Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            }),
            panel_block_background(),
            children![row_label(label)],
        ))
        .id();
    commands.entity(panel).add_child(row);
}
/// Отступ вложенной строки-ползунка. `spawn_slider_row` — общий кит панелей
/// правой колонки и про лесенку этой панели не знает, поэтому padding правится
/// здесь: без него ползунок секции стоял левее строк-кнопок той же секции.
fn indent_slider_row(commands: &mut Commands, row: Entity) {
    commands
        .entity(row)
        .entry::<Node>()
        .and_modify(|mut node| node.padding.left = px(8. + NESTED_ROW_INDENT_PX));
}

fn display_of(visible: bool) -> Display {
    if visible {
        Display::Flex
    } else {
        Display::None
    }
}

/// Строка выбора бэкенда или настройка одной из подсистем: настройка несёт
/// метку секции, по которой её прячут вместе с невыбранной подсистемой.
#[derive(Clone, Copy)]
enum RowStyle {
    Backend,
    Setting(NavSection),
}

/// Строка-кнопка со значением справа — та же кнопка-строка, что листает
/// значения в панелях Roads и Trees.
fn spawn_row<M>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    style: RowStyle,
    value_marker: NavValueLabel,
    value: String,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let left = match style {
        RowStyle::Backend => ROW_LEFT_PX,
        RowStyle::Setting(_) => ROW_LEFT_PX + NESTED_ROW_INDENT_PX,
    };
    let row = spawn_value_row(
        commands,
        panel,
        label,
        left,
        value_marker,
        value,
        on_activate,
    );
    if let RowStyle::Setting(section) = style {
        commands.entity(row).insert(section);
    }
    row
}

fn enabled_text(enabled: bool) -> String {
    on_off(enabled).to_string()
}

/// Настройки невыбранной подсистемы уходят из раскладки целиком: они не
/// «недоступны», они ни на что не влияют, пока ходят по другой.
fn sync_section_visibility(debug: Res<PolymeshDebug>, mut rows: Query<(&NavSection, &mut Node)>) {
    let active = active_section(&debug);
    for (section, mut node) in &mut rows {
        let display = display_of(*section == active);
        if node.display != display {
            node.display = display;
        }
    }
}
/// Актуализация подписей и бегунка после правки ресурса извне (BRP,
/// восстановленные настройки, хоткей N) — паттерн `sync_noise_values`.
fn sync_nav_values(
    values: NavPanelValues,
    theme: Res<UiTheme>,
    mut labels: Query<(&mut Text, &mut TextColor, &NavValueLabel)>,
) {
    for (mut text, mut color, label) in &mut labels {
        text.0 = label.text(&values);
        // цвет пишется в `TextColor`, а не в `ThemeTextColor`: тот immutable и
        // менялся бы вставкой на каждый кадр смены режима, а тема здесь — лишь
        // источник двух цветов, и оба берутся из неё же
        if let Some(next) = label.color(&values) {
            color.0 = theme.color(&next);
        }
    }
}
