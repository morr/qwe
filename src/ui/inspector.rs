//! Панель инспектора точки под телеметрией (правый верхний угол): строка
//! камеры `zoom/x/y cx/cy` — переехала сюда из телеметрии, под ней — список
//! того, что нарисовано под курсором, по семантическому индексу карты
//! (`map/inspect.rs`).
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
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::ui::Checked;
use bevy::ui_widgets::ValueChange;
use bevy::window::PrimaryWindow;

use super::speed::{PANEL_WIDTH_PX, TelemetryPanel};
use crate::camera::cursor_offset;
use crate::map::inspect::{
    FeatureRef, Hit, InspectIndex, InspectOrigin, InspectViewport, feature_shape, hit_line,
    mesh_feature,
};
use crate::map::osm::MapData;
use crate::map::surface::{LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::{RoadStyle, TreeStyle};
use crate::settings::Z_INSPECTOR_HIGHLIGHT;
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, panel_background, ui_column};

/// Зазор между телеметрией и этой панелью, px.
const INSPECTOR_GAP_PX: f32 = 6.0;
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
/// как только курсор вернётся. `selected` — какая находка выбрана (Cmd
/// листает, [`cycle_inspector_selection`]): жёлтым в списке и на карте стоит
/// она, не обязательно верхняя; пересчёт находок сбрасывает выбор на верхнюю.
#[derive(Resource, Default)]
struct InspectorHitsState {
    point: Vec2,
    hits: Vec<Hit>,
    selected: usize,
}

/// Галочка «highlight» панели инспектора: подсвечивать элемент под курсором
/// на карте прозрачно-жёлтым.
///
/// Сессионная, и это осознанное исключение из «каждый UI-ресурс —
/// `SettingsGroup`»: подсветка — инструмент отладки на один присест, а не
/// предпочтение игрока, — в `settings.toml` ей места нет, поэтому ни группы,
/// ни `track_pref`. `Reflect` нужен, чтобы верификация ставила её по BRP до
/// появления UI.
#[derive(Resource, Debug, Default, Reflect)]
#[reflect(Resource, Default)]
pub struct InspectorHighlight(pub bool);

/// Прозрачно-жёлтый подсветки — элемент читается сквозь слой.
const HIGHLIGHT_COLOR: Color = Color::srgba(1.0, 1.0, 0.0, 0.3);
/// Фон выделенной строки списка — тот же жёлтый, чуть гуще слоя.
const HIGHLIGHT_ROW_BG: Color = Color::srgba(1.0, 1.0, 0.0, 0.18);

/// Корень панели — по нему [`position_below_telemetry`] ставит панель вплотную
/// под телеметрию.
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
        app.init_resource::<InspectorHighlight>()
            .init_resource::<InspectorHitsState>()
            .register_type::<InspectorHighlight>();
        if standalone {
            // витрина рождается с включённой подсветкой: галочка стоит у края
            // экрана — вне всяких viewport примеров, где панель спрятана, — и
            // кликнуть её в витрине неоткуда
            app.insert_resource(InspectorHighlight(true));
        }
        app.add_systems(
                Startup,
                move |commands: Commands, assets: Res<AssetServer>| {
                    render_inspector_ui(commands, assets, standalone);
                },
            )
            .add_systems(
                Update,
                (
                    position_below_telemetry,
                    update_camera_text,
                    update_inspector_hits,
                    sync_inspector_visibility,
                    sync_hits_rows,
                    cycle_inspector_selection.run_if(cmd_just_pressed),
                    sync_highlight_checked,
                    sync_inspector_highlight,
                ),
            );
    }
}

fn render_inspector_ui(mut commands: Commands, assets: Res<AssetServer>, standalone: bool) {
    let mono: Handle<Font> = assets.load(fonts::MONO);
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                // под телеметрией: точную высоту ставит position_below_telemetry,
                // здесь стартовое значение до первого измерения
                top: px(UI_SCREEN_EDGE_PX_OFFSET),
                right: px(UI_SCREEN_EDGE_PX_OFFSET),
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
            GameUiRoot,
            InspectorRoot,
            // в игре панель показывает show_game_ui (`ui/mod.rs`), у витрины
            // состояний нет — видна сразу
            if standalone {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
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
}

/// Панель встаёт вплотную под телеметрию: её высота меняется вместе с
/// содержимым, а `ComputedNode::size` — в физических пикселях, отсюда
/// `inverse_scale_factor` (идиома `offset_below_brp_badge`). У витрины `roads`
/// телеметрии нет — `Single` молча пропускает систему, панель стоит у края.
fn position_below_telemetry(
    mut root: Single<&mut Node, With<InspectorRoot>>,
    telemetry: Single<&ComputedNode, With<TelemetryPanel>>,
) {
    let top = px(UI_SCREEN_EDGE_PX_OFFSET
        + telemetry.size.y * telemetry.inverse_scale_factor
        + INSPECTOR_GAP_PX);
    if root.top != top {
        root.top = top;
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
/// У игры сдвига нет; витрина `roads` ставит сдвиг показанного примера, и
/// панель говорит тем же языком, что `tools/osm_near`. Зум — метры на экранный
/// пиксель, как их держит `PanCamera::zoom_factor`: меньше — ближе. Три знака,
/// а не два: по зуму `tools/shot_coords` переводит пиксели скриншота в метры,
/// и на близком зуме (0.05) второй знак давал ошибку в 10 %.
///
/// Курсор вне окна координат не даёт — вместо чисел прочерк, чтобы строка не
/// теряла хвост и не выглядела обрезанной.
fn update_camera_text(
    text: Single<&mut Text, With<InspectorCameraText>>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    origin: Res<InspectOrigin>,
) {
    let (transform, controller) = camera.into_inner();
    let world_center = transform.translation.truncate();
    let zoom = controller.zoom_factor;

    let cursor = match window.cursor_position() {
        Some(cursor) => {
            let world = world_center + cursor_offset(&window, cursor) * zoom - origin.0;
            format!("{:.0}/{:.0}", world.x, world.y)
        }
        None => "-/-".to_string(),
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
/// находок не даёт: панель в это время спрятана ([`sync_inspector_visibility`]).
/// Результат пишется в `InspectorHitsState`, откуда его берут строки списка
/// ([`sync_hits_rows`]) и слой подсветки ([`sync_inspector_highlight`]).
fn update_inspector_hits(
    mut state: ResMut<InspectorHitsState>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
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
    let Some(cursor) = window.cursor_position() else {
        // прочерк вместо чисел: курсора нет, следующая точка — всегда дальше
        // шага (INFINITY), так что список пересчитается на её возврат
        state.point = Vec2::INFINITY;
        state.hits.clear();
        state.selected = 0;
        return;
    };
    let (transform, controller) = camera.into_inner();
    let point = transform.translation.truncate()
        + cursor_offset(&window, cursor) * controller.zoom_factor
        - origin.0;
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

/// Cmd нажат — листать выбор находок. Обе клавиши Command.
fn cmd_just_pressed(keys: Res<ButtonInput<KeyCode>>) -> bool {
    keys.just_pressed(KeyCode::SuperLeft) || keys.just_pressed(KeyCode::SuperRight)
}

/// Cmd листает выбор по находкам под курсором: верхняя → следующая → … →
/// снова верхняя. Выбрана одна находка и для строки списка, и для слоя
/// подсветки ([`sync_inspector_highlight`]) — жёлтым стоит то, что цикл
/// выбрал, а не обязательно верхнее. Пересчёт находок (курсор ушёл дальше
/// шага, список сменился) сбрасывает выбор, и цикл ведёт по новому списку.
fn cycle_inspector_selection(mut state: ResMut<InspectorHitsState>) {
    if state.hits.is_empty() {
        return;
    }
    state.selected = (state.selected + 1) % state.hits.len();
}

/// Панель видна, только пока курсор внутри окна просмотра — гейт уже посчитан
/// в `InspectorHitsState::point` (`INFINITY` — вне). Игра окон не знает
/// ([`InspectViewport::Whole`]) — система молчит, панелью ведут `GameUiRoot`
/// и `show_game_ui`.
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
/// `+N more`; выбранная строка (Cmd листает, [`cycle_inspector_selection`])
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
/// их девять, и clippy считает их за лишние.
#[derive(SystemParam)]
struct HighlightLayer<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: LayerMaterials<'w>,
    origin: Res<'w, InspectOrigin>,
    viewport: Res<'w, InspectViewport>,
    overlay: Query<'w, 's, Entity, With<InspectorHighlightMarker>>,
}

/// Слой подсветки элемента под курсором (галочка `InspectorHighlight`): форма
/// верхней не-ground находки — [`feature_shape`] + [`mesh_feature`], положенные
/// паттерном оверлеев (`ui/debug/overlays.rs::respawn_overlay`): прежние
/// сущности маркера убрать, новый слой — если есть что показывать.
/// `spawn_layers` ставит слою `DespawnOnExit`. Despawn из `Update` через
/// команды здесь законен, как и у оверлеев: убираются только свои меченые
/// сущности.
///
/// Находки приходят готовые из `InspectorHitsState` — расчёт один на панель.
/// Рисуется **выбранная** находка (Cmd листает, [`cycle_inspector_selection`]),
/// не обязательно верхняя. Пересборка — при смене выбора: пока курсор ходит
/// по одному зданию, слой не трогается. Смена сдвига или окна просмотра
/// (`InspectOrigin`, `InspectViewport` — витрина показала другой пример или
/// увела курсор) — тоже пересборка: слой стоит в мире на «карта + сдвиг», и
/// на новом сдвиге старый стоит не там; в окно другого примера слой не должен
/// вылезти и потому режется им ([`InspectViewport::clip`]), даже если сам
/// объект больше окна.
fn sync_inspector_highlight(
    enabled: Res<InspectorHighlight>,
    state: Res<InspectorHitsState>,
    map: Option<Res<MapData>>,
    roads: Res<RoadStyle>,
    layer: HighlightLayer,
    mut drawn: Local<Option<FeatureRef>>,
) {
    let HighlightLayer {
        mut commands,
        mut meshes,
        materials,
        origin,
        viewport,
        overlay,
    } = layer;
    // типовой кадр: галочка выключена и слоя нет — работы ноль
    if !enabled.0 && drawn.is_none() {
        return;
    }
    let selected = if enabled.0 {
        state.hits.get(state.selected).map(|hit| hit.feature)
    } else {
        None
    };
    if *drawn == selected && !origin.is_changed() && !viewport.is_changed() {
        return;
    }
    for entity in overlay.iter() {
        commands.entity(entity).despawn();
    }
    *drawn = selected;
    let Some(feature) = selected else {
        return;
    };
    let Some(map) = map else {
        return;
    };
    let Some(shapes) = feature_shape(&map, &roads, feature) else {
        return;
    };
    let mut builder = mesh_feature(&shapes, HIGHLIGHT_COLOR.to_linear());
    if let Some(window) = viewport.clip() {
        builder.clip_to_rect(window.min, window.max);
    }
    if builder.is_empty() {
        return;
    }
    spawn_layers(
        &mut commands,
        &mut meshes,
        &materials,
        [LayerMesh::new(
            builder,
            Z_INSPECTOR_HIGHLIGHT,
            "inspector_highlight",
            MaterialSpec::Blend,
        )
        .offset(origin.0)],
        InspectorHighlightMarker,
    );
}
