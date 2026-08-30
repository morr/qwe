//! Игровой UI: два слоя поверх карты.
//!
//! **HUD** — то, на что смотрят, не отрываясь от города: счётчики прогона
//! (`ui/stats.rs`), телеметрия и кнопка скорости (`ui/speed.rs`), выбор города
//! (`ui/city.rs`), справка по хоткеям (`ui/hotkeys.rs`), метка агентского
//! запуска (`ui/brp.rs`).
//!
//! **Панель настроек** — одна, со вкладками Map / Nav / Sim / Debug
//! (`ui/shell.rs`); каждая панель кладёт в свою вкладку секцию и наполняет её
//! строками из китов (`ui/knob.rs`, `ui/rows.rs`, `ui/slider.rs`). Панель
//! сворачивается по `Tab`, и что открыто — помнится между запусками.
//!
//! Виджеты первопартийные, из `bevy_feathers`. Всё, что виджетом не является
//! (плашки, заголовки, подписи строк), красится **токенами** той же темы, а не
//! своими цветами: иначе смена темы перекрасила бы половину экрана и оставила
//! вторую. Тема — `ui/theme.rs`, и плашки в ней полупрозрачные: панель лежит на
//! карте, а не вместо неё.

mod brp;
mod buildings;
mod city;
mod debug;
mod hotkeys;
mod knob;
mod navigation;
mod noise;
mod roads;
mod rows;
mod shell;
// `pub` ради демо-сцены расталкивания (`examples/demos/crowd_demo.rs`): ей
// нужна та же строка-ползунок, что и панелям игры, а весь `UiPlugin` она
// поднять не может — он тянет панели, карту и настройки
pub mod slider;
mod speed;
mod stats;
mod theme;
mod tree_rows;
mod trees;

use bevy::ecs::system::IntoObserverSystem;
use bevy::feathers::constants::{fonts, size};
use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::{ThemeBackgroundColor, ThemeTextColor, ThemedText};
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui_widgets::Activate;

pub use self::brp::AgentBrpSession;
pub use self::debug::{DebugConiferNoise, DebugDoors, DebugGrid, DebugNavmesh};
// `pub` по той же причине, что и `slider`: демо расталкивания зовёт киты
// панелей и потому обязано поднять их виджеты само
pub use self::theme::PanelWidgetsPlugin;
use crate::loading::{AppState, PlayPhase};
use crate::map::osm::MapData;
use crate::map::trees::visible_count;
use crate::map::{TreeRowStyle, TreeStyle};

pub const UI_SCREEN_EDGE_PX_OFFSET: f32 = 8.0;

/// Ширина панели настроек. Была 210 px, пока строка-ползунок занимала два
/// этажа: подпись над полосой; 240 — пока подписи были 12-пиксельными. На
/// [`PANEL_FONT`] в 14 px «Conifer share» рядом с полосой перестало помещаться.
pub const PANEL_WIDTH_PX: f32 = 260.0;

/// Плашка панели — тело редакторской панели темы. Токен, а не цвет: тема
/// перекрашивает виджеты панели, и подложка под ними обязана ехать вместе с
/// ними.
pub fn panel_background() -> ThemeBackgroundColor {
    ThemeBackgroundColor(tokens::PANE_BODY_BG)
}

/// Подложка блока внутри панели — строка-счётчик, блок ползунка, шапка группы.
pub fn panel_block_background() -> ThemeBackgroundColor {
    ThemeBackgroundColor(tokens::GROUP_BODY_BG)
}

/// Тип объектов мира, чьё число стоит в заголовке панели (см.
/// [`panel_header`]); компонент висит на тексте счётчика.
#[derive(Component, Clone, Copy)]
pub enum PanelCount {
    Trees,
    TreeRows,
    Buildings,
    Roads,
}

/// Заголовок панели: название и у правого края блока мелким шрифтом число
/// объектов этого типа в мире. Число пустое до загрузки карты — его заполняет
/// и дальше ведёт [`sync_panel_counts`].
pub fn panel_header(title: &str, count: PanelCount) -> impl Bundle {
    (
        ui_node(Node {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            // по базовой линии, а не по центру: у названия и у числа разный
            // кегль, и центрированные они «плавают» друг относительно друга
            align_items: AlignItems::Baseline,
            column_gap: px(6.),
            ..default()
        }),
        children![
            (
                Text::new(title),
                ThemeTextColor(tokens::PANE_HEADER_TEXT),
                // распорка: заголовок забирает всю ширину, число прижимается
                // к правому краю блока
                Node {
                    flex_grow: 1.,
                    ..default()
                },
            ),
            (count, Text::new(""), ThemeTextColor(tokens::TEXT_DIM)),
        ],
    )
}

/// Заголовок секции без счётчика — там, где считать в мире нечего (Demon,
/// Human, слои отладки).
pub fn panel_title(title: &str) -> impl Bundle {
    (Text::new(title), ThemeTextColor(tokens::PANE_HEADER_TEXT))
}

/// Счётчики в заголовках панелей: сколько объектов этого типа сейчас в мире.
/// Деревья считаются по собранному набору с учётом ползунка плотности и
/// тумблеров источников — то есть ровно столько крон и стоит на карте.
fn sync_panel_counts(
    map: Res<MapData>,
    style: Res<TreeStyle>,
    mut labels: Query<(&PanelCount, &mut Text)>,
) {
    for (count, mut text) in &mut labels {
        let total = match count {
            PanelCount::Trees => visible_count(&map.tree_appears_at, style.density),
            PanelCount::TreeRows => map.tree_rows.len(),
            PanelCount::Buildings => map.buildings.len(),
            PanelCount::Roads => map.roads.len(),
        };
        text.0 = total.to_string();
    }
}

/// Тень под текстом панелей. Осталась там, где текст лежит **на карте**, а не
/// на плашке панели (значок BRP, справка по хоткеям): на светлой карте буквы
/// без тени с ней сливаются. Смещение в пиксель — дефолтные четыре на 11–20 px
/// шрифте читаются как вторая строка текста.
pub const UI_TEXT_SHADOW: TextShadow = TextShadow {
    offset: Vec2::splat(1.0),
    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
};

/// Вариант кнопки по признаку «активна». «Активно» у каждой кнопки своё —
/// тумблер включён, листалка стоит на умолчании, город выбран, время на паузе, —
/// но выглядит это одинаково, и feathers называет такую кнопку `Primary`.
pub fn button_variant(is_active: bool) -> ButtonVariant {
    if is_active {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Normal
    }
}

/// Кнопка панели — первопартийный виджет feathers целиком, без правок узла:
/// её рост, скругление, отступы, курсор, наведение и нажатие приходят из сцены
/// и темы. `marker` — компонент, по которому система панели находит эту кнопку,
/// чтобы вести её [`ButtonVariant`]; `caption` — её содержимое одним ребёнком.
///
/// Подпись спавнится ребёнком, а не приходит через `@caption` в сцену:
/// у листалок её две, и обе строятся из строк времени выполнения. Ребёнку
/// хватает `ThemedText` — шрифт и цвет к нему спускает `InheritableFont` и
/// `InheritableThemeTextColor` с самой кнопки.
pub fn spawn_panel_button_with<M>(
    commands: &mut Commands,
    parent: Entity,
    marker: impl Bundle,
    caption: impl Bundle,
    is_active: bool,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    let variant = button_variant(is_active);
    let button = commands
        .spawn_scene(bsn! {
            @FeathersButton { @variant: {variant} }
            // виджет несёт свой `InheritableFont` и перекрыл бы им шрифт
            // панели — патчим только кегль, шрифт и вес остаются его
            InheritableFont { font_size: {PANEL_FONT} }
        })
        .insert(marker)
        .observe(on_activate)
        .with_child(caption)
        .id();
    commands.entity(parent).add_child(button);
    button
}

/// Подпись кнопки панели: шрифт и цвет спускает сама кнопка.
pub fn panel_button_label(label: &str) -> impl Bundle {
    (Text::new(label), ThemedText)
}

/// Узел-контейнер игрового UI: любой узел между источником шрифта и текстом
/// под ним.
///
/// **Единственный способ завести такой контейнер в `src/ui/`** — и это не стиль, а
/// защита. Распространение `InheritableFont` идёт только по цепочке сущностей с
/// `ThemedText` (`HierarchyPropagatePlugin::<TextFont, With<ThemedText>>`): на
/// первом же узле без метки обход прекращается, и все подписи ниже остаются с
/// дефолтным шрифтом в 20 px. У feathers подписи — прямые дети виджета, и она с
/// этим не сталкивается; у панелей между корнем и текстом стоят строки-обёртки.
/// Метка, которую надо было помнить на каждой из них, стоила панели World
/// двадцатипиксельных счётчиков — поэтому её больше не ставят руками, её
/// приносит конструктор. Забытую всё же метку ловит `warn_broken_font_chain`.
///
/// Сам источник — не контейнер: `InheritableFont` объявлен
/// `#[require(ThemedText, PropagateOver::<TextFont>)]`, поэтому корень со своим
/// шрифтом (телеметрия) или получивший его от `apply_panel_font` (любой
/// `GameUiRoot`) уже помечен и спавнится голой `Node` — он цепочку начинает, а
/// не стоит в ней.
pub fn ui_node(node: Node) -> (Node, ThemedText) {
    (node, ThemedText)
}

/// Контейнер-строка: дети в ряд, по центру, с зазором `gap`.
pub fn ui_row(gap: f32) -> (Node, ThemedText) {
    ui_node(Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(gap),
        ..default()
    })
}

/// Контейнер-колонка: дети друг под другом с зазором `gap`.
pub fn ui_column(gap: f32) -> (Node, ThemedText) {
    ui_node(Node {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        row_gap: px(gap),
        ..default()
    })
}

/// Подпись слева в строке панели — приглушённая, как `label_dim` в галерее
/// feathers.
///
/// `ThemeTextColor`, а не `TextColor`: он тянет за собой `ThemedText` (шрифт
/// спустится сверху) и `PropagateOver<TextColor>` — то есть цвет строки-кнопки
/// на подпись не пойдёт, и она останется приглушённой на любом варианте кнопки.
pub fn row_label(label: &str) -> impl Bundle {
    (Text::new(label), ThemeTextColor(tokens::TEXT_DIM))
}

/// Значение справа в строке панели — основным цветом темы.
pub fn row_value(value: impl Into<String>) -> impl Bundle {
    (Text::new(value.into()), ThemeTextColor(tokens::TEXT_MAIN))
}

/// Кегль подписей панелей — редакторские 14 px feathers.
///
/// Двенадцать здесь стояли, пока панели занимали обе нижние колонки от края до
/// края экрана: на 14 px левая колонка переставала помещаться по высоте. Колонок
/// больше нет, настройки лежат в одной панели со вкладками, и мельчить незачем —
/// кнопка, подпись которой приходится разглядывать, хуже кнопки, которая
/// занимает на два пикселя больше.
pub const PANEL_FONT: FontSize = size::MEDIUM_FONT;

/// Шрифт всех подписей панели: одна вставка на корень, а не `TextFont` на
/// каждый текст.
///
/// `InheritableFont` спускает шрифт всем потомкам с `ThemedText`. Виджеты
/// внутри несут свой такой же компонент и перекрыли бы его для своего
/// поддерева — поэтому киты (`spawn_panel_button_with`, `spawn_value_row`)
/// патчат его тем же [`PANEL_FONT`].
///
/// Системой по `Added`, а не параметром в десяти функциях спавна: шрифт —
/// свойство всего игрового UI, и панели про `AssetServer` знать не должны.
///
/// `Without<InheritableFont>` — корень, принёсший **свой** шрифт, оставляем ему.
/// Панель телеметрии спавнится с моноширинным FiraMono (её числа выровнены по
/// колонкам форматом `{:>5.2}`), и безусловная вставка перетирала его FiraSans'ом
/// в первом же кадре: колонки дрожали, а причина была в чужом файле.
fn apply_panel_font(
    roots: Query<Entity, (Added<GameUiRoot>, Without<InheritableFont>)>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    for root in &roots {
        commands.entity(root).insert(InheritableFont {
            font: assets.load(fonts::REGULAR),
            font_size: PANEL_FONT,
            weight: FontWeight::NORMAL,
        });
    }
}

/// «Между этим текстом и его шрифтом стоит узел без `ThemedText`» — в лог, на
/// первом же кадре, с именем виноватого узла.
///
/// Единственный способ заметить обрыв иначе — увидеть на экране подпись не того
/// размера и опознать в ней дефолтные 20 px; счётчики панели World простояли так
/// не один день. Проверка ходит по `Added<Text>`, то есть по одному разу на
/// текст, и только в отладочной сборке — в релизе панели уже собраны верно.
///
/// Молчит, когда источника шрифта над текстом нет вовсе (экран загрузки, метка
/// BRP): там дефолтный шрифт — не поломка, а решение.
#[cfg(debug_assertions)]
fn warn_broken_font_chain(texts: Query<Entity, Added<Text>>, chain: FontChain) {
    for text in &texts {
        if let Some(warning) = broken_font_chain(&chain, text) {
            warn!("{warning}");
        }
    }
}

/// Цепочка «узел → родитель» глазами проверки шрифта: метка `ThemedText`,
/// источник `InheritableFont` и имя узла для лога.
#[cfg(debug_assertions)]
type FontChain<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static ChildOf>,
        Has<ThemedText>,
        Has<InheritableFont>,
        Option<&'static Name>,
    ),
>;

/// Текст предупреждения об обрыве над `text` — или `None`, если шрифт дошёл
/// (либо идти ему неоткуда).
///
/// Имя виноватого узла берётся там же, где он найден: на итерации с источником
/// шрифта в руках уже чужой `Name`, и печатать его как нарушителя — значит
/// показывать на панель вместо обёртки внутри неё. Источник в сообщении
/// остаётся, но своей строкой: контейнеры из `ui_node` обычно безымянны, и имя
/// панели — единственное, за что цепляется взгляд в логе.
#[cfg(debug_assertions)]
fn broken_font_chain(chain: &FontChain, text: Entity) -> Option<String> {
    let mut entity = text;
    let mut broken: Option<(Entity, String)> = None;
    while let Ok((parent, themed, is_source, name)) = chain.get(entity) {
        if is_source {
            // источник шрифта найден; если по дороге к нему был узел без
            // метки — до текста шрифт не дошёл
            let (node, culprit) = broken?;
            let source = node_name(name);
            return Some(format!(
                "UI font chain broken: text {text} under {source} — node {culprit} ({node}) \
                 has no ThemedText, so the font stops there (use ui_node/ui_row/ui_column)"
            ));
        }
        if !themed && broken.is_none() {
            broken = Some((entity, node_name(name)));
        }
        let Some(parent) = parent else { break };
        entity = parent.parent();
    }
    None
}

/// Имя узла для лога: собственный `Name`, если он есть.
#[cfg(debug_assertions)]
fn node_name(name: Option<&Name>) -> String {
    name.map_or_else(|| "<unnamed>".to_owned(), Name::to_string)
}

/// Кнопка панели с одной подписью — обычный случай.
pub fn spawn_panel_button<M>(
    commands: &mut Commands,
    parent: Entity,
    marker: impl Bundle,
    label: &str,
    is_active: bool,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) -> Entity {
    spawn_panel_button_with(
        commands,
        parent,
        marker,
        panel_button_label(label),
        is_active,
        on_activate,
    )
}

/// «Курсор стоит в текстовом поле» — условие расписания для КАЖДОГО хоткея.
///
/// Без него набор seed'а разговаривает с игрой: пробел ставит симуляцию на
/// паузу, `-` и `=` крутят скорость, R перезапускает мир прямо посреди ввода.
/// Это тот же закон, что и «клик по панели не доходит до мира»
/// (`camera.rs::pointer_over_ui`), только для клавиатуры: событие, адресованное
/// UI, миру не принадлежит.
///
/// `Option` у фокуса: `InputFocus` ставит `bevy_ui_widgets`, и в тестовых
/// сборках без него условие должно отвечать «не печатаем», а не валить
/// валидацию параметров.
pub fn typing_in_text_input(
    focus: Option<Res<bevy::input_focus::InputFocus>>,
    fields: Query<(), With<bevy::text::EditableText>>,
) -> bool {
    focus
        .and_then(|focus| focus.get())
        .is_some_and(|entity| fields.contains(entity))
}

/// Корень игровой панели. Панели спавнятся в `Startup`, но поверх экрана
/// загрузки им делать нечего — до конца прогрева (`PlayPhase::Live`) они
/// скрыты (спавнятся с `Visibility::Hidden`).
#[derive(Component)]
pub struct GameUiRoot;

/// Колонка левого края: HUD-счётчики, под ними панель настроек. По ней метка
/// BRP (`ui/brp.rs`) двигает вниз всё разом — маркер поэтому здесь, а не в
/// файле того, кто колонку спавнит (`ui/shell.rs`) или того, кто её двигает
/// (`ui/stats.rs`).
#[derive(Component)]
pub(super) struct TopLeftColumn;

/// Порядок сборки UI в `Startup`: сначала оболочка со вкладками, потом секции
/// панелей в них.
///
/// Множеством, а не одной цепочкой в `UiPlugin`: секций восемь, они живут в
/// своих плагинах, и цепочка из восьми имён здесь была бы вторым списком
/// панелей — ровно тем, что этот файл уже однажды завёл слотами колонок.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum UiBuildSet {
    /// Колонка, полоска вкладок и пустые контейнеры вкладок.
    Shell,
    /// Секции панелей внутри вкладок.
    Sections,
    /// Расстановка секций по местам: спавнят их разные плагины, порядок
    /// запуска систем внутри набора не определён.
    Sort,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Startup,
            (UiBuildSet::Shell, UiBuildSet::Sections, UiBuildSet::Sort).chain(),
        )
        .add_plugins((
            // первопартийные виджеты панелей и тема под облик qwe
            theme::PanelWidgetsPlugin,
            shell::UiShellPlugin,
            speed::UiSpeedPlugin,
            stats::UiStatsPlugin,
            debug::UiDebugTogglesPlugin,
            trees::UiTreeStylePlugin,
            tree_rows::UiTreeRowStylePlugin,
            noise::UiConiferNoisePlugin,
            navigation::UiNavigationPlugin,
            buildings::UiBuildingStylePlugin,
            roads::UiRoadStylePlugin,
            city::UiCityPlugin,
            hotkeys::UiHotkeysPlugin,
            brp::UiBrpBadgePlugin,
        ))
        .add_systems(Update, apply_panel_font)
        .add_systems(
            Update,
            // `resource_changed` без `resource_exists` паникует до загрузки
            // карты, а `and_then` не вычисляет правую часть зря
            sync_panel_counts.run_if(
                resource_exists::<MapData>.and_then(
                    resource_changed::<MapData>
                        .or_else(resource_changed::<TreeStyle>)
                        .or_else(resource_changed::<TreeRowStyle>),
                ),
            ),
        )
        .add_systems(OnEnter(PlayPhase::Live), show_game_ui)
        // смена города возвращает приложение на экран загрузки — панели
        // прячутся до конца следующего прогрева
        .add_systems(OnExit(AppState::Playing), hide_game_ui);

        // строго после вставки шрифта: до неё у корня ещё нет источника, и
        // проверка сочла бы, что шрифту неоткуда идти
        #[cfg(debug_assertions)]
        app.add_systems(Update, warn_broken_font_chain.after(apply_panel_font));
    }
}

fn show_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Inherited;
    }
}

fn hide_game_ui(mut roots: Query<&mut Visibility, With<GameUiRoot>>) {
    for mut visibility in &mut roots {
        *visibility = Visibility::Hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Активна» — это один вариант кнопки, а не свой цвет у каждой панели:
    /// цвета обоих вариантов держит тема feathers (`ui/theme.rs`).
    #[test]
    fn the_active_button_is_the_primary_variant() {
        assert_eq!(button_variant(true), ButtonVariant::Primary);
        assert_eq!(button_variant(false), ButtonVariant::Normal);
    }

    /// Виноват узел без `ThemedText`, а не источник шрифта над ним: проверка
    /// сперва печатала имя источника, а нарушителя — голым `Entity`, и
    /// предупреждение показывало на панель целиком.
    #[cfg(debug_assertions)]
    #[test]
    fn the_broken_font_chain_warning_names_the_node_without_themed_text() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        let panel = app
            .world_mut()
            .spawn((
                Name::new("settings_panel"),
                ThemedText,
                InheritableFont {
                    font: Handle::default(),
                    font_size: PANEL_FONT,
                    weight: FontWeight::NORMAL,
                },
            ))
            .id();
        let wrapper = app
            .world_mut()
            .spawn((
                Name::new("row_without_marker"),
                Node::default(),
                ChildOf(panel),
            ))
            .id();
        app.world_mut()
            .spawn((Text::new("42"), ThemedText, ChildOf(wrapper)));

        fn probe(chain: FontChain, texts: Query<Entity, With<Text>>) -> Option<String> {
            texts
                .iter()
                .find_map(|text| broken_font_chain(&chain, text))
        }
        let warning = app
            .world_mut()
            .run_system_once(probe)
            .expect("проверка цепочки прогоняется")
            .expect("цепочка порвана — предупреждение есть");

        assert!(
            warning.contains("node row_without_marker"),
            "предупреждение не назвало виноватый узел: {warning}"
        );
        assert!(
            warning.contains("under settings_panel"),
            "предупреждение потеряло источник шрифта: {warning}"
        );
    }
}
