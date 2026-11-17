//! Панель инспектора точки под телеметрией (правый верхний угол): строка
//! камеры `zoom/x/y cx/cy` — переехала сюда из телеметрии, под ней — список
//! того, что нарисовано под курсором, по семантическому индексу карты
//! (`map/inspect.rs`). В игре панель — второй ребёнок колонки правого края
//! (`ui/speed.rs`); витрина roads состояний не имеет и колонки не ставит —
//! панель стоит сама у края экрана.
//!
//! Панель — инструмент агента, правящего баги OSM: скриншот с ней несёт
//! собственные координаты (строка камеры читается `tools/shot_coords`), а
//! список находок отвечает «что здесь» без BRP-запроса. Точный ответ «какой
//! слой рисует этот пиксель» даёт `brp probe` — он же печатает список находок
//! индекса разделом отчёта.
//!
//! Моноширинный шрифт — как у телеметрии: колонки не пляшут, и OCR скрипта
//! читает строку камеры.

use bevy::camera_controller::pan_camera::PanCamera;
use bevy::ecs::system::SystemParam;
use bevy::feathers::constants::{fonts, size};
use bevy::feathers::controls::FeathersCheckbox;
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::{ThemeTextColor, ThemedText};
use bevy::feathers::tokens;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};
use bevy::text::FontWeight;
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;
use bevy::window::PrimaryWindow;

use super::speed::{PANEL_WIDTH_PX, RightHudColumn};
use crate::camera::{cursor_world, pointer_over_ui};
use crate::dev::{VirtualCursor, cursor_position};
use crate::map::inspect::{
    FeatureRef, Hit, InspectIndex, InspectOrigin, InspectViewport, PinnedFill, feature_shape,
    hit_line, mesh_feature, mesh_pins, pin_stripe, pinned_line,
};
use crate::map::osm::MapData;
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, swap_layers};
use crate::map::{RoadStyle, TreeStyle};
use crate::prefs::PrefsExt;
use crate::settings::Z_INSPECTOR_HIGHLIGHT;
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UiBuildSet, panel_background, ui_column};

/// Больше строк находок панель не показывает: дальше хвост `+N more`.
const INSPECTOR_MAX_LINES: usize = 12;
/// Пересчитывать находки при сдвиге точки больше этого, м: сам запрос — одна
/// ячейка сетки, но форматирование строк на каждый кадр незачем.
const INSPECTOR_QUERY_STEP: f32 = 0.5;

#[derive(Component, Default)]
struct InspectorCameraText;

/// Контейнер списка находок — строки спавнит и убирает [`sync_hits_rows`].
#[derive(Component, Default)]
struct InspectorHitsList;

/// Одна строка находки; все такие убираются при пересчёте списка.
#[derive(Component)]
struct InspectorHitRow;

/// Контейнер блока пинов — черту и строки пинов спавнит и убирает
/// [`sync_pinned_rows`]; видимость всего блока ведёт
/// [`sync_pinned_visibility`].
#[derive(Component, Default)]
struct InspectorPinnedList;

/// Черта-разделитель и строка пина; всё это убирается при пересборке
/// блока.
#[derive(Component)]
struct InspectorPinnedRow;

/// Слой запиненного элемента на карте — помеченные сущности убирает
/// [`sync_pinned_highlights`]. Живёт всегда рядом с
/// [`InspectorHighlightMarker`] (тот держит контракт витрины `roads`:
/// `place_new` слой не сдвигает), различает их для своих систем.
#[derive(Component, Clone, Copy)]
struct InspectorPinnedMarker;

/// Галочка highlight — по ней [`sync_highlight_checked`] ставит и снимает
/// `Checked`.
#[derive(Component, Default)]
struct InspectorHighlightCheckbox;

/// Слой подсветки элемента под курсором — помеченные сущности убирает
/// [`sync_inspector_highlight`]. Публичен ради витрины `roads`: её пакетный
/// сдвиг свежих мешей (`place_new`) пропускает слой по этому маркеру — слой
/// стоит в мире сам, через `InspectOrigin` (`LayerMesh::offset`).
#[derive(Component, Clone, Copy)]
pub struct InspectorHighlightMarker;

/// Находки под курсором и сама точка — то, что насчитало
/// [`update_inspector_hits`]. Читатели — строки списка ([`sync_hits_rows`])
/// и слой подсветки ([`sync_inspector_highlight`]): расчёт один, читателей
/// двое. `point` в `INFINITY` — курсора нет или он вне окна просмотра
/// витрины; следующая точка всегда дальше шага, так что список пересчитается,
/// как только курсор вернётся. `selected` — какая находка выбрана (листают
/// скобки: `]` вперёд, `[` назад — [`cycle_inspector_selection_next`] и
/// [`cycle_inspector_selection_prev`]): жёлтым в списке и на карте стоит
/// она, не обязательно верхняя; пересчёт находок сбрасывает выбор на верхнюю.
#[derive(Resource, Default)]
struct InspectorHitsState {
    point: Vec2,
    hits: Vec<Hit>,
    selected: usize,
}

/// Галочка «highlight» панели инспектора: подсвечивать элемент под курсором
/// на карте прозрачно-жёлтым. По умолчанию включена — и в игре, и в витрине:
/// галочка стоит у края экрана, вне всяких viewport примеров витрины, где
/// панель спрятана, — кликнуть её в витрине неоткуда, а в игре панель без
/// подсветки теряет половину смысла. Выбор запоминается между запусками
/// (группа настроек `inspector_highlight`); витрины `SettingsPlugin`'a не
/// поднимают, так что там галочка каждый раз стартует с умолчания.
///
/// `Reflect` нужен, чтобы верификация ставила её по BRP до появления UI.
#[derive(Resource, Debug, Reflect, SettingsGroup)]
#[reflect(Resource, SettingsGroup, Default)]
pub struct InspectorHighlight(pub bool);

impl Default for InspectorHighlight {
    fn default() -> Self {
        Self(true)
    }
}

/// Прозрачно-жёлтый подсветки — элемент читается сквозь слой.
const HIGHLIGHT_COLOR: Color = Color::srgba(1.0, 1.0, 0.0, 0.3);
/// Фон выделенной строки списка — тот же жёлтый, чуть гуще слоя.
const HIGHLIGHT_ROW_BG: Color = Color::srgba(1.0, 1.0, 0.0, 0.18);

/// Запиненная находка: находка и её цвет. Цвет закрепляется за пином на всю
/// его жизнь — снятие другого пина перекраски не трогает (см. [`free_color`]).
struct PinnedHit {
    hit: Hit,
    color: usize,
}

/// Запиненные находки (клавиша `\`). Сессионные — в отличие от самой галочки
/// подсветки, которая теперь сохраняется: пин — инструмент присеста.
/// Стираются сменой `MapData` (см. [`sync_pinned_highlights`]) и Backspace.
#[derive(Resource, Default)]
struct InspectorPinned(Vec<PinnedHit>);

/// Палитра пинов: каждый запиненный элемент красится своим цветом — слой
/// на карте и фон его строки в панели одним цветом, — чтобы по скриншоту было
/// видно, какой подсвеченный элемент какой строке принадлежит. Цвет
/// закрепляется за пином на всю его жизнь: снятие пина не перекрашивает
/// остальные, а освободившийся цвет достаётся следующему пину
/// ([`free_color`]). Жёлтого в палитре нет: он у подсветки под курсором.
/// Пинов больше палитры — цвета замыкаются по модулю, два пина могут
/// совпасть.
const PIN_COLORS: [Color; 6] = [
    Color::srgba(1.0, 0.5, 0.0, 0.3),    // orange
    Color::srgba(0.15, 0.85, 0.15, 0.3), // green
    Color::srgba(0.1, 0.9, 1.0, 0.3),    // cyan
    Color::srgba(0.25, 0.45, 1.0, 0.3),  // blue
    Color::srgba(1.0, 0.15, 0.9, 0.3),   // magenta
    Color::srgba(1.0, 0.12, 0.12, 0.3),  // red
];

/// Цвет пина по его закреплённому индексу.
fn pinned_color(color_index: usize) -> Color {
    PIN_COLORS[color_index % PIN_COLORS.len()]
}

/// Наименьший свободный цвет: снятый пин отдаёт свой цвет следующему,
/// живые пины цветов не меняют.
fn free_color(pinned: &[PinnedHit]) -> usize {
    let mut color = 0;
    while pinned.iter().any(|p| p.color == color) {
        color += 1;
    }
    color
}

/// Фон строки пина в панели — цвет её слоя, гуще, как
/// [`HIGHLIGHT_ROW_BG`] против [`HIGHLIGHT_COLOR`].
fn pinned_row_bg(color: Color) -> Color {
    let color = color.to_srgba();
    Color::srgba(color.red, color.green, color.blue, 0.18)
}

/// Отступ между списком находок и блоком пинов, px.
const PIN_GAP_PX: f32 = 8.0;
/// Черта между списком находок и пинами — нейтральная, чтобы не спорить ни
/// с жёлтым выбором, ни с цветами пинов.
const PIN_RULE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.22);

/// Корень панели — по нему [`sync_inspector_visibility`] ведёт видимость в
/// витрине: панель видна, пока курсор в окне примера.
#[derive(Component, Default)]
struct InspectorRoot;

/// Панель инспектора точки. В игре ([`UiInspectorPlugin::game`]) панель —
/// `GameUiRoot`: появляется с прочим HUD на `PlayPhase::Live` и прячется при
/// смене города. Витрина `roads` ([`UiInspectorPlugin::standalone`]) состояний
/// не имеет — панель видна сразу.
#[derive(Default)]
pub struct UiInspectorPlugin {
    standalone: bool,
}

impl UiInspectorPlugin {
    /// Игровой режим: панель прячется до `PlayPhase::Live`.
    pub fn game() -> Self {
        Self { standalone: false }
    }

    /// Режим витрины: панель видна с первого кадра.
    pub fn standalone() -> Self {
        Self { standalone: true }
    }
}

impl Plugin for UiInspectorPlugin {
    fn build(&self, app: &mut App) {
        let standalone = self.standalone;
        app.init_resource::<InspectorHitsState>()
            .init_resource::<InspectorPinned>()
            .init_resource::<VirtualCursor>();
        // сохранение галочки — только в игре: в витрине `SettingsPlugin`'a нет
        // (примеры конфиг игры не трогают), `track_pref` там предупреждал бы
        // об отсутствующем реестре файлов на каждый клик
        if standalone {
            app.init_resource::<InspectorHighlight>()
                .register_type::<InspectorHighlight>();
        } else {
            app.register_pref::<InspectorHighlight>();
        }
        app.add_systems(
            Startup,
            (
                move |commands: Commands,
                      assets: Res<AssetServer>,
                      column: Option<Res<RightHudColumn>>| {
                    render_inspector_ui(commands, assets, standalone, column);
                },
            )
                .in_set(UiBuildSet::Sections),
        )
        .add_systems(
            Update,
            (
                update_camera_text,
                update_inspector_hits,
                sync_inspector_visibility,
                sync_hits_rows,
                // клавиши — как все хоткеи: пока курсор в текстовом поле
                // (сид), Backspace правит поле, а не стирает пины
                cycle_inspector_selection_next
                    .run_if(bracket_forward_pressed)
                    .run_if(not(super::typing_in_text_input)),
                cycle_inspector_selection_prev
                    .run_if(bracket_backward_pressed)
                    .run_if(not(super::typing_in_text_input)),
                pin_inspector_selection
                    .run_if(backslash_pressed)
                    .run_if(not(super::typing_in_text_input)),
                clear_inspector_pins
                    .run_if(backspace_pressed)
                    .run_if(not(super::typing_in_text_input)),
                sync_highlight_checked,
                sync_pinned_rows,
                sync_pinned_visibility,
                sync_pinned_highlights,
                sync_inspector_highlight,
            ),
        );
    }
}

/// Место панели решается здесь же. Витрина `roads` телеметрии не ставит и
/// состояний не имеет — панель сама стоит у края экрана и видна с первого
/// кадра. В игре панель — второй ребёнок колонки правого края
/// (`ui/speed.rs`): телеметрия, зазор, панель — раскладку ведёт флекс колонки,
/// а видимость её `GameUiRoot` (скрыта до `PlayPhase::Live`, прячется на
/// смене города).
fn render_inspector_ui(
    mut commands: Commands,
    assets: Res<AssetServer>,
    standalone: bool,
    column: Option<Res<RightHudColumn>>,
) {
    let mono: Handle<Font> = assets.load(fonts::MONO);
    let root = commands
        .spawn((
            Node {
                position_type: if standalone {
                    PositionType::Absolute
                } else {
                    PositionType::Relative
                },
                top: if standalone {
                    px(UI_SCREEN_EDGE_PX_OFFSET)
                } else {
                    Val::Auto
                },
                right: if standalone {
                    px(UI_SCREEN_EDGE_PX_OFFSET)
                } else {
                    Val::Auto
                },
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(3.),
                // ширина телеметрии: две панели одной ширины читаются как одна связка
                width: px(PANEL_WIDTH_PX),
                padding: UiRect {
                    top: px(10.),
                    right: px(16.),
                    bottom: px(10.),
                    left: px(16.),
                },
                ..default()
            },
            panel_background(),
            InspectorRoot,
            Name::new("inspector_ui"),
            // моноширинный: колонки строки камеры и находок не пляшут
            InheritableFont {
                font: mono.clone(),
                font_size: size::SMALL_FONT,
                weight: FontWeight::NORMAL,
            },
            children![
                (
                    Text::default(),
                    ThemeTextColor(tokens::TEXT_MAIN),
                    InspectorCameraText,
                ),
                // строки находок спавнит sync_hits_rows; контейнер несёт
                // ThemedText — без него цепочка моно-шрифта до строк не доходит
                (
                    ui_column(1.0),
                    ThemeTextColor(tokens::TEXT_DIM),
                    InspectorHitsList,
                ),
                // блок пинов (`\`): отступ, черта, строки пинов — их
                // спавнит sync_pinned_rows, видимость ведёт
                // sync_pinned_visibility; пока пинов нет, блок скрыт
                (
                    Node {
                        display: Display::None,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2.),
                        margin: UiRect {
                            top: px(PIN_GAP_PX),
                            ..default()
                        },
                        ..default()
                    },
                    ThemedText,
                    ThemeTextColor(tokens::TEXT_DIM),
                    InspectorPinnedList,
                ),
            ],
        ))
        .id();
    // чекбокс подсветки: клик пишет ресурс, Checked ведёт
    // sync_highlight_checked — и клик, и BRP-запись идут через одно место.
    // Шрифт патчится на сцену виджета, как в китах (rows.rs): перо панели —
    // тот же моно, размер — размер панели.
    let checkbox = commands
        .spawn_scene(bsn! {
            @FeathersCheckbox {
                @caption: bsn! { Text("highlight") ThemedText }
            }
            InheritableFont {
                font: {mono},
                font_size: {size::SMALL_FONT},
                weight: FontWeight::NORMAL,
            }
        })
        .observe(
            |change: On<ValueChange<bool>>, mut highlight: ResMut<InspectorHighlight>| {
                highlight.0 = change.value;
            },
        )
        .insert(InspectorHighlightCheckbox)
        .id();
    commands.entity(root).add_child(checkbox);

    // витрина стоит сама; в игре панель прячется и показывается вместе с
    // колонкой — `GameUiRoot` на корне панели теперь не нужен, видимость
    // наследуется от колонки
    if standalone {
        commands.entity(root).insert(GameUiRoot);
    } else {
        let column = column.expect("right hud column spawns in UiBuildSet::Shell, before this");
        commands.entity(column.0).add_child(root);
    }
}

/// Где стоит камера и насколько приближена. Строка нужна не игроку, а чтению
/// скриншота со стороны: по ней видно, какой кусок карты в кадре, без запроса
/// к живому миру по BRP (переехала из телеметрии `ui/speed.rs`).
///
/// Формат `0.410/2374/2703 2510/2880` — сначала камера как `zoom/x/y` (порядок
/// пермалинка slippy-карт), через пробел — точка под курсором как `x/y`.
///
/// Координаты — метры карты от юго-западного угла: мир минус [`InspectOrigin`].
/// У игры сдвига нет; витрина `roads` ставит сдвиг текущего примера (чей
/// viewport под курсором, иначе показанного), и панель говорит тем же языком,
/// что `tools/osm_near`. Зум — метры на экранный
/// пиксель, как их держит `PanCamera::zoom_factor`: меньше — ближе. Три знака,
/// а не два: по зуму `tools/shot_coords` переводит пиксели скриншота в метры,
/// и на близком зуме (0.05) второй знак давал ошибку в 10 %.
///
/// Курсор вне окна координат не даёт — вместо чисел прочерк, чтобы строка не
/// теряла хвост и не выглядела обрезанной. Над UI-панелью — тоже прочерк:
/// точка под панелью миру не адресуется, и читать её координаты — то же, что
/// панорамировать карту сквозь панель.
fn update_camera_text(
    text: Single<&mut Text, With<InspectorCameraText>>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    virtual_cursor: Res<VirtualCursor>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    origin: Res<InspectOrigin>,
) {
    let (transform, controller) = camera.into_inner();
    let world_center = transform.translation.truncate();
    let zoom = controller.zoom_factor;

    let cursor = match cursor_position(&window, &virtual_cursor) {
        Some(cursor) if !pointer_over_ui(&hover_map, &ui_nodes) => {
            let world = cursor_world(&window, cursor, transform, zoom) - origin.0;
            format!("{:.0}/{:.0}", world.x, world.y)
        }
        _ => "-/-".to_string(),
    };

    text.into_inner().set_if_neq(Text(format!(
        "{zoom:.3}/{:.0}/{:.0} {cursor}",
        world_center.x - origin.0.x,
        world_center.y - origin.0.y
    )));
}

/// Входы списка находок одним параметром — идиома `LayerMaterials`: у системы
/// их восемь, и clippy считает их за лишние. Карта — `Option`: `MapData`
/// появляется только с разобранным миром, и голый `Res` внутри составного
/// параметра до неё паникует (см. комментарий у `sync_panel_counts` в
/// `ui/mod.rs`).
#[derive(SystemParam)]
struct InspectorInputs<'w> {
    map: Option<Res<'w, MapData>>,
    index: Res<'w, InspectIndex>,
    origin: Res<'w, InspectOrigin>,
    viewport: Res<'w, InspectViewport>,
    trees: Res<'w, TreeStyle>,
    roads: Res<'w, RoadStyle>,
}

/// Список находок под курсором — из семантического индекса
/// (`map/inspect.rs::InspectIndex`), сверху самый верхний слой. Пересчёт —
/// при сдвиге точки больше [`INSPECTOR_QUERY_STEP`], при пересборке индекса
/// (новая карта, новый пример витрины), при сдвиге витрины и при смене окна
/// просмотра. Курсор вне окна просмотра (витрина, [`InspectViewport::Outside`])
/// или над UI-панелью находок не даёт — мир под панелью не ищется, как и не
/// панорамируется сквозь неё (`camera.rs::pointer_over_ui`); в витрине панель
/// в это время спрятана ([`sync_inspector_visibility`]). Результат пишется в
/// `InspectorHitsState`, откуда его берут строки списка ([`sync_hits_rows`]) и
/// слой подсветки ([`sync_inspector_highlight`]).
fn update_inspector_hits(
    mut state: ResMut<InspectorHitsState>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    virtual_cursor: Res<VirtualCursor>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    inputs: InspectorInputs,
) {
    let InspectorInputs {
        map,
        index,
        origin,
        viewport,
        trees,
        roads,
    } = inputs;
    let Some(map) = map.as_deref() else {
        return;
    };
    let over_ui = pointer_over_ui(&hover_map, &ui_nodes);
    let Some(cursor) = cursor_position(&window, &virtual_cursor).filter(|_| !over_ui) else {
        // прочерк вместо чисел: курсора нет (или он над панелью — это не
        // точка карты), следующая точка — всегда дальше шага (INFINITY), так
        // что список пересчитается на её возврат
        state.point = Vec2::INFINITY;
        state.hits.clear();
        state.selected = 0;
        return;
    };
    let (transform, controller) = camera.into_inner();
    let point = cursor_world(&window, cursor, transform, controller.zoom_factor) - origin.0;
    if !viewport.contains(point) {
        state.point = Vec2::INFINITY;
        state.hits.clear();
        state.selected = 0;
        return;
    }
    // ручка Sidewalks тоже вход: её переворот меняет список без движения
    // курсора и без пересборки индекса
    let inputs_fresh =
        index.is_changed() || origin.is_changed() || viewport.is_changed() || roads.is_changed();
    if !inputs_fresh && state.point.distance(point) < INSPECTOR_QUERY_STEP {
        return;
    }
    state.point = point;
    state.selected = 0;
    state.hits = if index.is_built() {
        index.hits(map, trees.density, &roads, point)
    } else {
        Vec::new()
    };
}

/// Скобки листают выбор находок: `]` — вперёд, `[` — назад.
fn bracket_forward_pressed(keys: Res<ButtonInput<KeyCode>>) -> bool {
    keys.just_pressed(KeyCode::BracketRight)
}

/// См. [`bracket_forward_pressed`].
fn bracket_backward_pressed(keys: Res<ButtonInput<KeyCode>>) -> bool {
    keys.just_pressed(KeyCode::BracketLeft)
}

/// `]` листает выбор по находкам под курсором вперёд: верхняя → следующая → … →
/// снова верхняя. Выбрана одна находка и для строки списка, и для слоя
/// подсветки ([`sync_inspector_highlight`]) — жёлтым стоит то, что цикл
/// выбрал, а не обязательно верхнее. Пересчёт находок (курсор ушёл дальше
/// шага, список сменился) сбрасывает выбор, и цикл ведёт по новому списку.
/// Назад, с тем же заворотом, листает [`cycle_inspector_selection_prev`].
fn cycle_inspector_selection_next(mut state: ResMut<InspectorHitsState>) {
    if state.hits.is_empty() {
        return;
    }
    state.selected = (state.selected + 1) % state.hits.len();
}

/// `[` листает выбор назад — зеркально [`cycle_inspector_selection_next`]:
/// верхняя → последняя.
fn cycle_inspector_selection_prev(mut state: ResMut<InspectorHitsState>) {
    if state.hits.is_empty() {
        return;
    }
    state.selected = (state.selected + state.hits.len() - 1) % state.hits.len();
}

/// `\` — клавиша пина ([`pin_inspector_selection`]).
fn backslash_pressed(keys: Res<ButtonInput<KeyCode>>) -> bool {
    keys.just_pressed(KeyCode::Backslash)
}

/// Backspace — клавиша сброса пинов ([`clear_inspector_pins`]).
fn backspace_pressed(keys: Res<ButtonInput<KeyCode>>) -> bool {
    keys.just_pressed(KeyCode::Backspace)
}

/// `\` запинивает выбранную находку и снимает пин с неё же: тоггл по
/// адресу элемента ([`FeatureRef`]) — один и тот же элемент в списке и в
/// пинах совпадает. Работает только с включённой галочкой highlight:
/// пин — часть подсветки, без неё панель пины не показывает и клавиша
/// молчит. Пока пинов нет и элемент не выбран, нажатие ничего не делает.
fn pin_inspector_selection(
    enabled: Res<InspectorHighlight>,
    state: Res<InspectorHitsState>,
    mut pinned: ResMut<InspectorPinned>,
) {
    if !enabled.0 {
        return;
    }
    let Some(hit) = state.hits.get(state.selected) else {
        return;
    };
    if let Some(at) = pinned.0.iter().position(|c| c.hit.feature == hit.feature) {
        pinned.0.remove(at);
    } else {
        let color = free_color(&pinned.0);
        pinned.0.push(PinnedHit {
            hit: hit.clone(),
            color,
        });
    }
}

/// Backspace стирает все пины разом — сброс не про элемент, выбор не нужен.
/// Тоже только с включённой галочкой highlight: без неё пины не видны, и
/// сброс «невидимого» только удивил бы при включении.
fn clear_inspector_pins(enabled: Res<InspectorHighlight>, mut pinned: ResMut<InspectorPinned>) {
    if !enabled.0 || pinned.0.is_empty() {
        return;
    }
    pinned.0.clear();
}

/// Панель видна, только пока курсор внутри окна просмотра — гейт уже посчитан
/// в `InspectorHitsState::point` (`INFINITY` — вне). Игра окон не знает
/// ([`InspectViewport::Whole`]) — система молчит, панелью ведёт видимость
/// колонки правого края (её `GameUiRoot` и `show_game_ui` в `ui/mod.rs`).
fn sync_inspector_visibility(
    viewport: Res<InspectViewport>,
    state: Res<InspectorHitsState>,
    mut root: Single<&mut Visibility, With<InspectorRoot>>,
) {
    if matches!(*viewport, InspectViewport::Whole) {
        return;
    }
    let shown = state.point.is_finite();
    root.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}

/// Строки находок — построчно, до [`INSPECTOR_MAX_LINES`] узлов и хвост
/// `+N more`; выбранная строка (листают скобки `]`/`[`)
/// несёт жёлтый фон — тот элемент, чью форму рисует слой подсветки. Пусто
/// (курсора нет, карта не разобрана) — прочерк. Строки пересоздаются на
/// каждую смену `InspectorHitsState`: их ≤13, спавн узлов дешевле сравнения
/// содержимого.
fn sync_hits_rows(
    mut commands: Commands,
    state: Res<InspectorHitsState>,
    list: Single<Entity, With<InspectorHitsList>>,
    rows: Query<Entity, With<InspectorHitRow>>,
) {
    if !state.is_changed() {
        return;
    }
    for row in rows.iter() {
        commands.entity(row).despawn();
    }
    let list = list.into_inner();
    if state.hits.is_empty() {
        spawn_hit_row(&mut commands, list, "-".to_string(), false);
        return;
    }
    for (ordinal, hit) in state.hits.iter().enumerate().take(INSPECTOR_MAX_LINES) {
        let highlighted = ordinal == state.selected;
        spawn_hit_row(&mut commands, list, hit_line(hit), highlighted);
    }
    if state.hits.len() > INSPECTOR_MAX_LINES {
        spawn_hit_row(
            &mut commands,
            list,
            format!("+{} more", state.hits.len() - INSPECTOR_MAX_LINES),
            false,
        );
    }
}

/// Строка списка: свои `Node` + `Text` на одном узле — контейнер списка уже
/// несёт `ThemedText`, так что цепочка моно-шрифта не рвётся.
fn spawn_hit_row(commands: &mut Commands, parent: Entity, text: String, highlighted: bool) {
    let mut row = commands.spawn((
        Node {
            padding: UiRect {
                left: px(4.),
                right: px(4.),
                ..default()
            },
            ..default()
        },
        Text::new(text),
        ThemedText,
        ThemeTextColor(tokens::TEXT_DIM),
        InspectorHitRow,
    ));
    if highlighted {
        row.insert(BackgroundColor(HIGHLIGHT_ROW_BG));
    }
    let row = row.id();
    commands.entity(parent).add_child(row);
}

/// Блок пинов: черта-разделитель, затем по строке на пин — той же
/// геометрии, что строки находок, с фоном своего цвета ([`PIN_COLORS`]).
/// Строки ведут `p` вместо `z` ([`pinned_line`]): `tools/shot_coords`
/// собирает только `z …` строки, так что пин не смешается с находками под
/// курсором и не станет «выбранной строкой» его отчёта.
fn sync_pinned_rows(
    mut commands: Commands,
    pinned: Res<InspectorPinned>,
    list: Single<Entity, With<InspectorPinnedList>>,
    rows: Query<Entity, With<InspectorPinnedRow>>,
) {
    if !pinned.is_changed() {
        return;
    }
    for row in rows.iter() {
        commands.entity(row).despawn();
    }
    let list = list.into_inner();
    if pinned.0.is_empty() {
        return;
    }
    let rule = commands
        .spawn((
            Node {
                width: percent(100.),
                height: px(1.),
                ..default()
            },
            BackgroundColor(PIN_RULE_COLOR),
            InspectorPinnedRow,
        ))
        .id();
    commands.entity(list).add_child(rule);
    for entry in pinned.0.iter() {
        let row = commands
            .spawn((
                Node {
                    padding: UiRect {
                        left: px(4.),
                        right: px(4.),
                        ..default()
                    },
                    ..default()
                },
                Text::new(pinned_line(&entry.hit)),
                ThemedText,
                ThemeTextColor(tokens::TEXT_DIM),
                BackgroundColor(pinned_row_bg(pinned_color(entry.color))),
                InspectorPinnedRow,
            ))
            .id();
        commands.entity(list).add_child(row);
    }
}

/// Блок пинов виден, только когда есть что показывать и стоит галочка
/// highlight: пин — часть подсветки, без неё панель и клавиша молчат.
fn sync_pinned_visibility(
    enabled: Res<InspectorHighlight>,
    pinned: Res<InspectorPinned>,
    mut node: Single<&mut Node, With<InspectorPinnedList>>,
) {
    if !pinned.is_changed() && !enabled.is_changed() {
        return;
    }
    let shown = enabled.0 && !pinned.0.is_empty();
    let display = if shown { Display::Flex } else { Display::None };
    if node.display != display {
        node.display = display;
    }
}

/// `Checked` галочки ведёт ресурс: клик пишет его обсёрвером, BRP — напрямую,
/// система приводит виджет к ресурсу в обе стороны.
fn sync_highlight_checked(
    enabled: Res<InspectorHighlight>,
    checkbox: Query<Entity, With<InspectorHighlightCheckbox>>,
    mut commands: Commands,
) {
    if !enabled.is_changed() {
        return;
    }
    for entity in checkbox.iter() {
        if enabled.0 {
            commands.entity(entity).insert_if_new(Checked);
        } else {
            commands.entity(entity).remove::<Checked>();
        }
    }
}

/// Входы слоя подсветки одним параметром — идиома `LayerMaterials`: у системы
/// их девять, и clippy считает их за лишние. Слоя двое — под курсором и
/// запиненный, — запросы разводят их маркерами: у подсветки запрос без
/// маркера пина, у пинов только с ним.
#[derive(SystemParam)]
struct HighlightLayer<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: LayerMaterials<'w>,
    index: Res<'w, InspectIndex>,
    origin: Res<'w, InspectOrigin>,
    viewport: Res<'w, InspectViewport>,
    camera: Single<'w, 's, &'static PanCamera, With<Camera2d>>,
    overlay: Query<
        'w,
        's,
        Entity,
        (
            With<InspectorHighlightMarker>,
            Without<InspectorPinnedMarker>,
        ),
    >,
    pinned_overlay: Query<'w, 's, Entity, With<InspectorPinnedMarker>>,
}

/// Слой подсветки элемента под курсором (галочка `InspectorHighlight`): форма
/// выбранной находки (листают скобки `]`/`[`), а если она запинена —
/// штриховкой «жёлтый + цвет пина»: своей подсветкой запиненный элемент уже
/// обладает, и жёлтая заливка поверх неё не читалась бы. Штриховка — тот же
/// [`mesh_pins`] с двумя одинаковыми заливками: обе целиком уходят в
/// «перекрытие» и режутся попарно. Положенная формой оверлеев
/// (`map::surface::swap_layers`): прежние сущности маркера убрать, новый
/// слой — если есть что показывать.
/// `spawn_layers` ставит слою `DespawnOnExit`. Despawn из `Update` через
/// команды здесь законен, как и у оверлеев: убираются только свои меченые
/// сущности.
///
/// Находки приходят готовые из `InspectorHitsState` — расчёт один на панель.
/// Рисуется **выбранная** находка (листают скобки `]`/`[`), не обязательно
/// верхняя. Пересборка — при смене выбора, при смене самих пинов (снятие
/// пина возвращает жёлтую заливку), при пересборке индекса (перекройка дорог
/// меняет формы кусков) и когда зум сдвинул ширину полос штриховки
/// (`drawn` хранит её вместе с выбором): пока курсор ходит по одному зданию,
/// слой не трогается. Смена сдвига или окна просмотра (`InspectOrigin`,
/// `InspectViewport` — витрина показала другой пример или увела курсор) —
/// тоже пересборка: слой стоит в мире на «карта + сдвиг», и на новом сдвиге
/// старый стоит не там; в окно другого примера слой не должен вылезти и
/// потому режется им ([`InspectViewport::clip`]), даже если сам объект больше
/// окна.
fn sync_inspector_highlight(
    enabled: Res<InspectorHighlight>,
    state: Res<InspectorHitsState>,
    pinned: Res<InspectorPinned>,
    map: Option<Res<MapData>>,
    roads: Res<RoadStyle>,
    layer: HighlightLayer,
    mut drawn: Local<(Option<FeatureRef>, f32)>,
) {
    let HighlightLayer {
        mut commands,
        mut meshes,
        materials,
        index,
        origin,
        viewport,
        camera,
        overlay,
        pinned_overlay: _,
    } = layer;
    // типовой кадр: галочка выключена и слоя нет — работы ноль
    if !enabled.0 && drawn.0.is_none() {
        return;
    }
    let selected = if enabled.0 {
        state.hits.get(state.selected).map(|hit| hit.feature)
    } else {
        None
    };
    // выбор запинен — его закреплённый цвет (None — не запинен)
    let pinned_color_index = selected.and_then(|feature| {
        pinned
            .0
            .iter()
            .find_map(|pinned| (pinned.hit.feature == feature).then_some(pinned.color))
    });
    let stripe = pin_stripe(camera.zoom_factor);
    if *drawn == (selected, stripe)
        && !origin.is_changed()
        && !viewport.is_changed()
        && !index.is_changed()
        && !pinned.is_changed()
    {
        return;
    }
    *drawn = (selected, stripe);
    let layer = selected.and_then(|feature| {
        let map = map.as_ref()?;
        let shapes = feature_shape(&index, map, &roads, feature)?;
        let mut builder = match pinned_color_index {
            // запинен: штриховка «жёлтый + цвет пина» на всей форме
            Some(color_index) => mesh_pins(
                &[
                    PinnedFill {
                        shapes: shapes.clone(),
                        color: HIGHLIGHT_COLOR.to_linear(),
                    },
                    PinnedFill {
                        shapes,
                        color: pinned_color(color_index).to_linear(),
                    },
                ],
                stripe,
                0.0,
            ),
            None => mesh_feature(&shapes, HIGHLIGHT_COLOR.to_linear()),
        };
        if let Some(window) = viewport.clip() {
            builder.clip_to_rect(window.min, window.max);
        }
        Some(
            LayerMesh::new(
                builder,
                Z_INSPECTOR_HIGHLIGHT,
                "inspector_highlight",
                MaterialSpec::Blend,
            )
            .offset(origin.0),
        )
    });
    swap_layers(
        &mut commands,
        &mut meshes,
        &materials,
        layer,
        InspectorHighlightMarker,
        &overlay,
    );
}

/// Слои запиненных элементов (клавиша `\` при включённой галочке highlight):
/// все пины одним мешем ([`mesh_pins`]) — каждый своим цветом
/// ([`pinned_color`]), в перекрытиях штриховка 45° с чередованием цветов
/// пары, полосы одной толщины на глаз ([`pin_stripe`] от зума). Слой, в
/// отличие от подсветки под курсором, курсора не слушает: пересчёт находок и
/// уход мыши его не трогают — он перерисовывается только при смене самого
/// набора пинов, при пересборке индекса (перекройка дорог меняет формы), при
/// смене галочки и когда зум сдвинул ширину полосы (см. `last_stripe`).
/// Смена `MapData` (новый город, витрина переключила пример, пересборка
/// набора деревьев) стирает пины: индексы находок указывали в чужую карту.
fn sync_pinned_highlights(
    enabled: Res<InspectorHighlight>,
    mut pinned: ResMut<InspectorPinned>,
    map: Option<Res<MapData>>,
    roads: Res<RoadStyle>,
    layer: HighlightLayer,
    mut drawn: Local<usize>,
    mut last_stripe: Local<f32>,
) {
    let HighlightLayer {
        mut commands,
        mut meshes,
        materials,
        index,
        origin,
        viewport,
        camera,
        overlay: _,
        pinned_overlay,
    } = layer;
    // чужая карта — чужие индексы находок: пины стираются
    if map.as_ref().is_some_and(|map| map.is_changed()) {
        pinned.0.clear();
    }
    let stripe = pin_stripe(camera.zoom_factor);
    // зум уехал на заметную долю — полосы другой толщины, меш перестроить
    let stripe_shifted = (*last_stripe - stripe).abs() > stripe * 0.02;
    // типовой кадр: галочка выключена и слоёв нет — работы ноль
    if !enabled.0 && *drawn == 0 {
        return;
    }
    // слой стоит, пока не изменилось ничего из того, чем он рисуется
    if enabled.0
        && !enabled.is_changed()
        && !pinned.is_changed()
        && !index.is_changed()
        && !roads.is_changed()
        && !stripe_shifted
    {
        return;
    }
    *last_stripe = stripe;
    // галочку сняли — слои спрятать; сами пины остаются
    let layer = enabled
        .0
        .then(|| {
            let map = map.as_ref()?;
            let fills: Vec<PinnedFill> = pinned
                .0
                .iter()
                .filter_map(|entry| {
                    feature_shape(&index, map, &roads, entry.hit.feature).map(|shapes| PinnedFill {
                        shapes,
                        color: pinned_color(entry.color).to_linear(),
                    })
                })
                .collect();
            let mut builder = mesh_pins(&fills, stripe, stripe * 0.25);
            if let Some(window) = viewport.clip() {
                builder.clip_to_rect(window.min, window.max);
            }
            Some(
                LayerMesh::new(
                    builder,
                    Z_INSPECTOR_HIGHLIGHT,
                    "inspector_pins",
                    MaterialSpec::Blend,
                )
                .offset(origin.0),
            )
        })
        .flatten();
    let pinned_layer = layer.is_some();
    swap_layers(
        &mut commands,
        &mut meshes,
        &materials,
        layer,
        (InspectorHighlightMarker, InspectorPinnedMarker),
        &pinned_overlay,
    );
    *drawn = usize::from(pinned_layer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::inspect::HitSource;

    fn pinned_hits(colors: &[usize]) -> Vec<PinnedHit> {
        colors
            .iter()
            .map(|&color| PinnedHit {
                hit: Hit {
                    z: 0.0,
                    label: String::new(),
                    src: HitSource::Osm,
                    id: None,
                    feature: FeatureRef::Tree(0),
                },
                color,
            })
            .collect()
    }

    #[test]
    fn a_freed_color_goes_to_the_next_pin_and_survivors_keep_theirs() {
        // цвета 0,1,2 заняты — следующий пин берёт 3
        assert_eq!(free_color(&pinned_hits(&[0, 1, 2])), 3);
        // сняли пин с цветом 1 — следующий берёт именно его,
        // а цвета выживших (0 и 2) не трогаются
        assert_eq!(free_color(&pinned_hits(&[0, 2])), 1);
        // вся палитра занята — цвет уходит за её пределы и замкнётся по модулю
        assert_eq!(free_color(&pinned_hits(&[0, 1, 2, 3, 4, 5])), 6);
    }
}
