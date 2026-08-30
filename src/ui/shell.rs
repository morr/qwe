//! Панель настроек: полоска вкладок и тело под ней — одна на весь UI.
//!
//! До неё настроек было восемь панелей в двух колонках у нижних углов экрана.
//! Колонки складывались из абсолютных панелей, поэтому их приходилось стыковать
//! вручную по замеренным высотам (`stack_bottom_columns`), а правая при
//! 1920×1080 всё равно не помещалась и уезжала за верх экрана. Здесь настройки
//! стоят в **одной** панели: вкладка выбирает, какой набор секций показан, и
//! стыкует их обычный флекс.
//!
//! # Что здесь решено
//!
//! - **Вкладок ровно столько, сколько вариантов у [`SettingsTab`], и в порядке
//!   объявления.** Приём тот же, что был у колонок: слот, прописанный числом в
//!   файле своей панели, дважды сводил две панели в одно место.
//! - **Переключение — мышью, не табуляцией.** `TabNavigationPlugin` в проекте
//!   сознательно не поднят (см. `ui/theme.rs`): с ним пробел и «нажимает»
//!   сфокусированную кнопку, и ставит симуляцию на паузу. Клавиша `Tab` поэтому
//!   свободна, и её берёт **сворачивание** панели — карта под ней становится
//!   чистой, что нужно и для скриншотов, и просто чтобы посмотреть на город.
//! - **Выбранная вкладка и свёрнутость запоминаются** ([`UiShellState`] — группа
//!   настроек): настройки крутят подолгу и возвращаются к ним между запусками,
//!   и панель, каждый раз открывающаяся на первой вкладке, заставляла бы
//!   доходить до нужной заново.
//! - **Колонка не ловит мышь** (`Pickable::IGNORE`): она растянута на всю высоту
//!   экрана, чтобы тело панели знало, где ему кончиться, — а невидимая полоса
//!   шириной в панель, отбирающая у карты клики и протяжки, была бы ровно тем,
//!   что запрещает закон «UI-ввод не доходит до мира» (`CLAUDE.md`).

use bevy::feathers::controls::ButtonVariant;
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::input::common_conditions::input_just_pressed;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::ui_widgets::Activate;

use super::{
    GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiBuildSet, button_variant, panel_background,
    panel_block_background, panel_button_label, spawn_panel_button_with, ui_column, ui_node,
};
use crate::prefs::TrackPrefExt;

/// Вкладка панели настроек. **Порядок объявления — порядок в полоске.**
#[derive(Component, Reflect, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Default)]
pub(super) enum SettingsTab {
    /// Всё, что видно на карте: деревья, ряды деревьев, здания, дороги, поле хвои.
    #[default]
    Map,
    /// Навигация: бэкенд, его настройки, расталкивание и слоты.
    Nav,
    /// Прогон: seed и детерминизм, ручки демонов и людей.
    Sim,
    /// Отладка: слои поверх карты, сборка мира, сброс настроек.
    Debug,
}

impl SettingsTab {
    pub(super) const ALL: [Self; 4] = [Self::Map, Self::Nav, Self::Sim, Self::Debug];

    /// Подпись на кнопке вкладки. Короткая: четыре кнопки делят ширину панели.
    fn label(self) -> &'static str {
        match self {
            Self::Map => "Map",
            Self::Nav => "Nav",
            Self::Sim => "Sim",
            Self::Debug => "Debug",
        }
    }
}

/// Состояние панели настроек — что открыто и открыто ли вообще.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "ui")]
pub(super) struct UiShellState {
    pub(super) tab: SettingsTab,
    /// Свёрнута ли панель: видна одна полоска вкладок, тело скрыто.
    pub(super) collapsed: bool,
}

/// Клик по вкладке: выбрать её и **развернуть** панель; клик по уже выбранной —
/// свернуть.
///
/// Отдельной чистой функцией от наблюдателя: это единственное место, где у
/// панели есть логика, и проверять её через целое приложение с живой мышью
/// пришлось бы ради двух сравнений.
fn on_tab_click(state: UiShellState, tab: SettingsTab) -> UiShellState {
    if state.tab == tab && !state.collapsed {
        return UiShellState {
            collapsed: true,
            ..state
        };
    }
    UiShellState {
        tab,
        collapsed: false,
    }
}

/// Кнопка вкладки в полоске.
#[derive(Component)]
struct TabButton(SettingsTab);

/// Контейнер секций одной вкладки: показан, пока эта вкладка выбрана.
#[derive(Component)]
struct TabPane(SettingsTab);

/// Тело панели — то, что прячется при сворачивании и прокручивается, когда
/// вкладка не влезает по высоте.
#[derive(Component)]
struct ShellBody;

/// Подпись кнопки сворачивания: `-` у развёрнутой панели, `+` у свёрнутой.
#[derive(Component)]
struct CollapseCaption;

/// Куда панели кладут свои секции. Вкладки спавнятся раз и живут всё
/// приложение, поэтому сущность, а не поиск по маркеру на каждый спавн.
#[derive(Resource)]
pub(super) struct SettingsPanes {
    /// Колонка левого края целиком: в неё же попадает HUD-блок счётчиков, и по
    /// ней метка BRP двигает всё, что под ней.
    column: Entity,
    panes: [Entity; SettingsTab::ALL.len()],
}

impl SettingsPanes {
    /// Контейнер вкладки — родитель для секций панели.
    pub(super) fn pane(&self, tab: SettingsTab) -> Entity {
        self.panes[tab as usize]
    }

    /// Колонка левого края — для HUD-блоков, которые стоят НАД панелью настроек.
    pub(super) fn column(&self) -> Entity {
        self.column
    }
}

/// Место секции в своей вкладке — **порядок объявления сверху вниз**.
///
/// Тот же приём, что был у колонок панелей: слот, прописанный числом в файле
/// своей панели, дважды сводил две панели в одно место. Здесь порядок читается
/// целиком в одном месте, а спавнят секции восемь систем в своих плагинах —
/// порядок их запуска внутри `UiBuildSet::Sections` не определён, и без этого
/// перечисления вкладка Map собиралась бы каждый раз по-новому.
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum SectionSlot {
    // Map
    Trees,
    TreeRows,
    Buildings,
    Roads,
    Noise,
    // Nav — одним блоком
    Navigation,
    // Sim
    World,
    Demon,
    Human,
    // Debug
    Overlays,
    WorldBuild,
    Actions,
}

/// Секции по местам: `Startup` спавнит их в порядке запуска систем, то есть в
/// произвольном, — здесь дети каждой вкладки переставляются по [`SectionSlot`].
fn sort_sections(
    panes: Res<SettingsPanes>,
    children: Query<&Children>,
    slots: Query<&SectionSlot>,
    mut commands: Commands,
) {
    for pane in panes.panes {
        let Ok(sections) = children.get(pane) else {
            continue;
        };
        let mut sorted: Vec<Entity> = sections.iter().collect();
        sorted.sort_by_key(|section| slots.get(*section).copied().ok());
        commands.entity(pane).replace_children(&sorted);
    }
}

/// Блок строк во вкладке — секция без заголовка. Вкладка Navigation вся такая:
/// её «заголовки» — сами строки (`Algo`, тумблер `Separation`, подпись `Slots`),
/// и второй заголовок над ними был бы этикеткой на этикетке.
pub(super) fn spawn_block(
    commands: &mut Commands,
    pane: Entity,
    slot: SectionSlot,
    name: &'static str,
) -> Entity {
    let block = commands
        .spawn((ui_column(SECTION_ROW_GAP_PX), slot, Name::new(name)))
        .id();
    commands.entity(pane).add_child(block);
    block
}

/// Секция вкладки: заголовок и колонка строк под ним. Возвращает колонку —
/// именно её панель передаёт китам как родителя строк.
///
/// Панели от переезда во вкладки не изменились ничем, кроме этой строчки:
/// раньше они спавнили себе плашку с заголовком, теперь просят секцию.
pub(super) fn spawn_section(
    commands: &mut Commands,
    pane: Entity,
    slot: SectionSlot,
    header: impl Bundle,
    name: &'static str,
) -> Entity {
    let section = spawn_block(commands, pane, slot, name);
    let title = commands
        .spawn((
            ui_node(Node {
                padding: UiRect {
                    left: px(SECTION_HEADER_PAD_PX),
                    right: px(SECTION_HEADER_PAD_PX),
                    top: px(2.),
                    bottom: px(2.),
                },
                ..default()
            }),
            panel_block_background(),
            children![header],
        ))
        .id();
    // заголовок первым ребёнком: строки секции панель довешивает после
    commands.entity(section).add_child(title);
    section
}

/// Зазор между строками внутри секции и между секциями вкладки.
const SECTION_ROW_GAP_PX: f32 = 4.0;

/// Отступ заголовка секции — по отступу строки-значения (`rows::ROW_LEFT_PX`),
/// чтобы подписи стояли в одну вертикаль.
const SECTION_HEADER_PAD_PX: f32 = 8.0;

/// Зазор между кнопками в полоске вкладок.
const TAB_GAP_PX: f32 = 2.0;

/// Одна «строка» прокрутки колесом в логических пикселях: строка панели ростом
/// `size::ROW_HEIGHT`, за щелчок проезжаем полторы.
const SCROLL_LINE_PX: f32 = 36.0;

pub(super) struct UiShellPlugin;

impl Plugin for UiShellPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiShellState>()
            .register_type::<UiShellState>()
            .register_type::<SettingsTab>()
            .track_pref::<UiShellState>()
            .add_systems(Startup, spawn_shell.in_set(UiBuildSet::Shell))
            .add_systems(Startup, sort_sections.in_set(UiBuildSet::Sort))
            .add_systems(
                Update,
                (
                    sync_shell.run_if(resource_changed::<UiShellState>),
                    toggle_shell.run_if(
                        input_just_pressed(KeyCode::Tab).and_then(not(super::typing_in_text_input)),
                    ),
                    send_scroll_events,
                ),
            )
            .add_observer(on_scroll);
    }
}

/// Панель целиком: колонка левого края, полоска вкладок, тело со вкладками.
fn spawn_shell(mut commands: Commands, state: Res<UiShellState>) {
    let column = commands
        .spawn((
            super::TopLeftColumn,
            ui_node(Node {
                position_type: PositionType::Absolute,
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                // до низа экрана: телу панели нужен потолок высоты, иначе
                // длинная вкладка растёт за край и прокручивать нечему
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                width: px(super::PANEL_WIDTH_PX),
                ..default()
            }),
            // колонка выше своего содержимого — и невидимая её часть не должна
            // отбирать у карты клики и протяжки
            Pickable::IGNORE,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("ui_left_column"),
        ))
        .id();

    let panel = commands
        .spawn((
            ui_node(Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                // панель ужимается первой, когда колонке не хватает высоты;
                // `min_height: 0` разрешает флексу это сделать
                flex_shrink: 1.,
                min_height: px(0.),
                width: percent(100),
                ..default()
            }),
            Name::new("settings_panel"),
        ))
        .id();
    commands.entity(column).add_child(panel);

    let strip = commands
        .spawn((
            ui_node(Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(TAB_GAP_PX),
                padding: UiRect::all(px(4.)),
                flex_shrink: 0.,
                ..default()
            }),
            ThemeBackgroundColor(tokens::PANE_HEADER_BG),
            Name::new("settings_tabs"),
        ))
        .id();
    commands.entity(panel).add_child(strip);

    for tab in SettingsTab::ALL {
        let button = spawn_panel_button_with(
            &mut commands,
            strip,
            TabButton(tab),
            panel_button_label(tab.label()),
            state.tab == tab && !state.collapsed,
            on_tab_activated,
        );
        // кнопки делят ширину полоски поровну: у сцены feathers ширина по
        // содержимому, и «Debug» была бы вдвое шире «Nav»
        commands
            .entity(button)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.flex_grow = 1.;
                node.flex_basis = px(0.);
                node.padding = UiRect::horizontal(px(4.));
            });
    }

    // кнопка сворачивания — в той же полоске, справа: свернуть панель можно и
    // кликом по активной вкладке, но по кнопке это видно, а не угадывается
    let collapse = spawn_panel_button_with(
        &mut commands,
        strip,
        (),
        (CollapseCaption, panel_button_label(collapse_label(&state))),
        false,
        |_activate: On<Activate>, mut state: ResMut<UiShellState>| {
            state.collapsed = !state.collapsed;
        },
    );
    commands
        .entity(collapse)
        .entry::<Node>()
        .and_modify(|mut node| node.padding = UiRect::horizontal(px(6.)));

    let body = commands
        .spawn((
            ShellBody,
            ui_node(Node {
                display: display_of(!state.collapsed),
                flex_direction: FlexDirection::Column,
                row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                padding: UiRect::all(px(6.)),
                // прокрутка вместо роста за край: вкладка Map длиннее экрана на
                // маленьком окне, и обрезать её молча — потерять строки
                overflow: Overflow::scroll_y(),
                flex_shrink: 1.,
                min_height: px(0.),
                ..default()
            }),
            panel_background(),
            Name::new("settings_body"),
        ))
        .id();
    commands.entity(panel).add_child(body);

    let panes = SettingsTab::ALL.map(|tab| {
        let pane = commands
            .spawn((
                TabPane(tab),
                ui_node(Node {
                    display: display_of(state.tab == tab),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(UI_SCREEN_EDGE_PX_OFFSET),
                    ..default()
                }),
                Name::new(tab.label()),
            ))
            .id();
        commands.entity(body).add_child(pane);
        pane
    });

    commands.insert_resource(SettingsPanes { column, panes });
}

fn collapse_label(state: &UiShellState) -> &'static str {
    if state.collapsed { "+" } else { "-" }
}

fn display_of(visible: bool) -> Display {
    if visible {
        Display::Flex
    } else {
        Display::None
    }
}

fn on_tab_activated(
    activate: On<Activate>,
    tabs: Query<&TabButton>,
    mut state: ResMut<UiShellState>,
) {
    let Ok(tab) = tabs.get(activate.entity) else {
        return;
    };
    state.set_if_neq(on_tab_click(*state, tab.0));
}

/// `Tab` — свернуть/развернуть. Через `typing_in_text_input`, как все хоткеи:
/// в поле seed'а табуляция принадлежит полю.
fn toggle_shell(mut state: ResMut<UiShellState>) {
    state.collapsed = !state.collapsed;
}

/// Панель вслед за состоянием: вариант кнопок, видимость вкладок и тела,
/// подпись сворачивания.
fn sync_shell(
    state: Res<UiShellState>,
    mut tabs: Query<(&TabButton, &mut ButtonVariant)>,
    mut panes: Query<(&TabPane, &mut Node), Without<ShellBody>>,
    body: Option<Single<&mut Node, With<ShellBody>>>,
    caption: Option<Single<&mut Text, With<CollapseCaption>>>,
) {
    for (tab, mut variant) in &mut tabs {
        variant.set_if_neq(button_variant(state.tab == tab.0 && !state.collapsed));
    }
    for (pane, mut node) in &mut panes {
        let display = display_of(state.tab == pane.0);
        if node.display != display {
            node.display = display;
        }
    }
    if let Some(mut body) = body {
        let display = display_of(!state.collapsed);
        if body.display != display {
            body.display = display;
        }
    }
    if let Some(mut caption) = caption {
        caption.set_if_neq(Text::new(collapse_label(&state)));
    }
}

/// Колесо над панелью — прокрутка панели, а не зум карты. Событие адресуется
/// узлу под курсором и всплывает вверх по иерархии, пока не найдёт того, кто
/// умеет прокручиваться: щёлкать колесом ровно по телу панели, а не по строке в
/// нём, никто не станет.
#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    /// Дельта в логических пикселях.
    delta: f32,
}

fn send_scroll_events(
    mut wheel: MessageReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    mut commands: Commands,
) {
    for event in wheel.read() {
        let delta = match event.unit {
            MouseScrollUnit::Line => -event.y * SCROLL_LINE_PX,
            MouseScrollUnit::Pixel => -event.y,
        };
        if delta == 0. {
            continue;
        }
        for pointer in hover_map.values() {
            for entity in pointer.keys().copied() {
                commands.trigger(Scroll { entity, delta });
            }
        }
    }
}

fn on_scroll(mut scroll: On<Scroll>, mut scrollables: Query<(&mut ScrollPosition, &ComputedNode)>) {
    let Ok((mut position, computed)) = scrollables.get_mut(scroll.entity) else {
        return;
    };
    let max = (computed.content_size().y - computed.size().y) * computed.inverse_scale_factor();
    if max <= 0. {
        return;
    }
    let next = clamp_scroll(position.y, scroll.delta, max);
    if next != position.y {
        position.y = next;
        // дельта израсходована — выше по иерархии её никто не увидит
        scroll.propagate(false);
    }
}

/// Новое положение прокрутки: сдвиг на дельту, но не за пределы содержимого.
fn clamp_scroll(position: f32, delta: f32, max: f32) -> f32 {
    (position + delta).clamp(0., max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Порядок вкладок — порядок объявления; подписи не повторяются, иначе две
    /// кнопки полоски выглядели бы одной.
    #[test]
    fn the_tabs_are_declared_once_each() {
        let labels: Vec<&str> = SettingsTab::ALL.iter().map(|tab| tab.label()).collect();
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(labels.len(), unique.len());
        assert_eq!(labels[0], SettingsTab::Map.label());
    }

    /// Клик по чужой вкладке разворачивает панель: иначе кнопка отвечала бы
    /// только сменой подсветки, а показанного не менялось бы ничего.
    #[test]
    fn a_click_on_another_tab_opens_the_panel() {
        let collapsed = UiShellState {
            tab: SettingsTab::Map,
            collapsed: true,
        };
        assert_eq!(
            on_tab_click(collapsed, SettingsTab::Nav),
            UiShellState {
                tab: SettingsTab::Nav,
                collapsed: false,
            }
        );
    }

    /// Клик по своей вкладке — сворачивание, второй клик — обратно.
    #[test]
    fn a_click_on_the_open_tab_collapses_and_back() {
        let open = UiShellState {
            tab: SettingsTab::Sim,
            collapsed: false,
        };
        let collapsed = on_tab_click(open, SettingsTab::Sim);
        assert!(collapsed.collapsed);
        assert_eq!(collapsed.tab, SettingsTab::Sim);
        assert_eq!(on_tab_click(collapsed, SettingsTab::Sim), open);
    }

    #[test]
    fn scrolling_stays_inside_the_content() {
        assert_eq!(clamp_scroll(0., -50., 100.), 0.);
        assert_eq!(clamp_scroll(80., 50., 100.), 100.);
        assert_eq!(clamp_scroll(10., 20., 100.), 30.);
    }
}
