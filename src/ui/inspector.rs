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
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::ThemeTextColor;
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::text::FontWeight;
use bevy::window::PrimaryWindow;

use super::speed::{PANEL_WIDTH_PX, TelemetryPanel};
use crate::camera::cursor_offset;
use crate::map::inspect::{InspectIndex, InspectOrigin, format_hits};
use crate::map::osm::MapData;
use crate::map::{RoadStyle, TreeStyle};
use crate::ui::{GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, panel_background};

/// Зазор между телеметрией и этой панелью, px.
const INSPECTOR_GAP_PX: f32 = 6.0;
/// Больше строк находок панель не показывает: дальше хвост `+N more`.
const INSPECTOR_MAX_LINES: usize = 12;
/// Пересчитывать находки при сдвиге точки больше этого, м: сам запрос — одна
/// ячейка сетки, но форматирование строк на каждый кадр незачем.
const INSPECTOR_QUERY_STEP: f32 = 0.5;

#[derive(Component, Default)]
struct InspectorCameraText;

#[derive(Component, Default)]
struct InspectorHitsText;

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
            ),
        );
    }
}

fn render_inspector_ui(mut commands: Commands, assets: Res<AssetServer>, standalone: bool) {
    let mono: Handle<Font> = assets.load(fonts::MONO);
    commands.spawn((
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
            font: mono,
            font_size: size::SMALL_FONT,
            weight: FontWeight::NORMAL,
        },
        children![
            (
                Text::default(),
                ThemeTextColor(tokens::TEXT_MAIN),
                InspectorCameraText,
            ),
            (
                Text::default(),
                ThemeTextColor(tokens::TEXT_DIM),
                InspectorHitsText,
            ),
        ],
    ));
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
    trees: Res<'w, TreeStyle>,
    roads: Res<'w, RoadStyle>,
}

/// Список находок под курсором — из семантического индекса
/// (`map/inspect.rs::InspectIndex`), сверху самый верхний слой. Пересчёт —
/// при сдвиге точки больше [`INSPECTOR_QUERY_STEP`], при пересборке индекса
/// (новая карта, новый пример витрины) и при сдвиге витрины.
fn update_inspector_hits(
    text: Single<&mut Text, With<InspectorHitsText>>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    inputs: InspectorInputs,
    mut last: Local<Vec2>,
) {
    let InspectorInputs {
        map,
        index,
        origin,
        trees,
        roads,
    } = inputs;
    let Some(map) = map.as_deref() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        text.into_inner().set_if_neq(Text("-".to_string()));
        return;
    };
    let (transform, controller) = camera.into_inner();
    let point = transform.translation.truncate()
        + cursor_offset(&window, cursor) * controller.zoom_factor
        - origin.0;
    // ручка Sidewalks тоже вход: её переворот меняет список без движения
    // курсора и без пересборки индекса
    let inputs_fresh = index.is_changed() || origin.is_changed() || roads.is_changed();
    if !inputs_fresh && last.distance(point) < INSPECTOR_QUERY_STEP {
        return;
    }
    *last = point;

    let hits = if index.is_built() {
        index.hits(map, trees.density, &roads, point)
    } else {
        Vec::new()
    };
    let body = if hits.is_empty() {
        "-".to_string()
    } else {
        format_hits(&hits, INSPECTOR_MAX_LINES)
    };
    text.into_inner().set_if_neq(Text(body));
}
