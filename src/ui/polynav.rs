//! Панель Polymesh — прототип полигонального navmesh (`navigation/polymesh.rs`):
//! три тумблера и ползунок радиуса агента. Отдельный блок над дебаг-рядом, а
//! не тумблер в нём: у панели есть собственные ручки, и строка со значением
//! справа читается так же, как строки панелей Roads и Trees.
//!
//! Тумблеры разведены по тому, что они меняют:
//! `Enabled` — строить меш и ходить по нему (бэкенд поиска пути),
//! `Show` — рисовать оверлей (ничего не перестраивает),
//! `Chunks` — иерархия чанков: и постройка слоями, и их границы на оверлее.
//! Оверлей рисует **все** рёбра полигонов построенного меша одним merged-мешем
//! — по нему видно и контуры препятствий, и как polyanya разбила проходимое
//! пространство.

use std::collections::HashSet;

use bevy::color::Mix;
use bevy::ecs::system::IntoObserverSystem;
use bevy::picking::hover::Hovered;
use bevy::sprite_render::AlphaMode2d;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, SliderValue, ValueChange};

use bevy::prelude::*;

use crate::loading::{AppState, WorldInitSet};
use crate::map::MeshBuilder;
use crate::navigation::{PolyNavmesh, PolymeshDebug};
use crate::settings::{
    MAP_SIZE, POLYMESH_AGENT_RADIUS_MAX, POLYMESH_AGENT_RADIUS_MIN, POLYMESH_AGENT_RADIUS_STEP,
};
use crate::ui::slider::{SliderRow, quantize, spawn_slider_row};
use crate::ui::{
    DebugNavmesh, GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiLeftColumnSlot,
    UiOpacity, ui_color,
};

/// Над заливкой сеточного navmesh-оверлея (5.2), под юнитами.
const POLYMESH_OVERLAY_Z: f32 = 5.3;
/// Толщина ребра, метры мира: видна на городском зуме, не заливает экран.
const POLYMESH_EDGE_WIDTH: f32 = 0.4;
const POLYMESH_EDGE_COLOR: Color = Color::srgba(0.2, 0.85, 0.95, 0.6);
/// Заливка непроходимого — **тот же** красный, что у сеточного оверлея
/// (`debug.rs::sync_navmesh_overlay`): два слоя показывают одно и то же, и
/// одинаковый цвет — единственное, что делает их точность сравнимой на глаз.
const POLYMESH_BLOCKED_COLOR: Color = Color::srgba(0.9, 0.15, 0.15, 0.35);

/// Границы чанков — верхний уровень иерархии, по которому выбирается коридор
/// (`polymesh::find_path_polymesh`). Тёмные и полупрозрачные: они не часть
/// геометрии, а разбиение поверх неё, и читаться должны как сетка на карте, а
/// не как ещё один слой мира. Жёлтый пробовался и сливался с песком и дорогами.
/// Штрих той же толщины, что и рёбра меша: сетка чанков рисуется всегда, и
/// жирная линия перечёркивала бы геометрию, которую оверлей и показывает.
const POLYMESH_CHUNK_COLOR: Color = Color::srgba(0.05, 0.05, 0.08, 0.7);
const POLYMESH_CHUNK_WIDTH: f32 = 0.4;

/// Строки — как у панелей Roads и Trees: плотный фон поверх полупрозрачной
/// панели, осветление под курсором и при нажатии.
const ROW_LIGHTEN: f32 = 0.0;
const HOVER_LIGHTEN: f32 = 0.12;
const PRESSED_LIGHTEN: f32 = 0.24;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Строка-тумблер `Enabled` — по ней система подсветки находит её, а
/// [`PolymeshEnabledLabel`] адресует текст значения.
#[derive(Component)]
struct PolymeshEnabledRow;

/// Текст значения тумблера (`On` / `Off`) и то, какой именно тумблер он
/// подписывает. Один компонент на все три строки, а не маркер на каждую:
/// три отдельных `&mut Text`-запроса пришлось бы разводить `Without` каждый
/// с каждым, и добавление четвёртой строки ломало бы все предыдущие.
#[derive(Component, Clone, Copy)]
enum PolymeshToggleLabel {
    Enabled,
    Show,
    Chunks,
}

impl PolymeshToggleLabel {
    fn value(self, debug: &PolymeshDebug) -> bool {
        match self {
            Self::Enabled => debug.enabled,
            Self::Show => debug.show,
            Self::Chunks => debug.chunks,
        }
    }
}

/// Текст значения радиуса.
#[derive(Component)]
struct PolymeshRadiusLabel;

/// Ползунок радиуса.
#[derive(Component)]
struct PolymeshRadiusSlider;

/// Что нарисовано: поколение постройки и радиус — пока те же, пересобирать
/// слой незачем (идиома `ConiferNoiseOverlayMarker`). Чанков в ключе нет:
/// их переключение перестраивает меш, то есть двигает поколение.
#[derive(Component)]
struct PolymeshOverlayMarker {
    generation: u32,
    radius_bits: u32,
}

pub struct UiPolynavPlugin;

impl Plugin for UiPolynavPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_polynav_panel)
            // после смены города: оверлей умер с DespawnOnExit, ресурсы живы
            .add_systems(
                OnEnter(AppState::Playing),
                sync_polymesh_overlay.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    highlight_rows,
                    // два слоя рисуют одно и то же поверх одной карты — тумблеры
                    // взаимоисключающие (см. `enforce_overlay_exclusivity`)
                    enforce_overlay_exclusivity.run_if(
                        resource_changed::<PolymeshDebug>.or_else(resource_changed::<DebugNavmesh>),
                    ),
                    sync_polynav_values.run_if(resource_changed::<PolymeshDebug>),
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

fn render_polynav_panel(mut commands: Commands, debug: Res<PolymeshDebug>) {
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(UI_SCREEN_EDGE_PX_OFFSET),
                left: px(UI_SCREEN_EDGE_PX_OFFSET),
                flex_direction: FlexDirection::Column,
                row_gap: px(4.),
                padding: UiRect::all(px(10.)),
                width: px(210.),
                ..default()
            },
            BackgroundColor(ui_color(UiOpacity::Medium)),
            // 0 — дебаг-тумблеры, 1 — Noise; левую колонку перестыкует
            // `ui::stack_bottom_columns`
            UiLeftColumnSlot(2),
            GameUiRoot,
            Visibility::Hidden,
            Name::new("polymesh_panel"),
            children![(
                Text::new("Polymesh"),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            )],
        ))
        .id();

    spawn_toggle_row(
        &mut commands,
        panel,
        "Enabled",
        debug.enabled,
        PolymeshToggleLabel::Enabled,
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.enabled = !debug.enabled;
        },
    );

    spawn_toggle_row(
        &mut commands,
        panel,
        "Show",
        debug.show,
        PolymeshToggleLabel::Show,
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.show = !debug.show;
        },
    );

    spawn_toggle_row(
        &mut commands,
        panel,
        "Chunks",
        debug.chunks,
        PolymeshToggleLabel::Chunks,
        |_activate: On<Activate>, mut debug: ResMut<PolymeshDebug>| {
            debug.chunks = !debug.chunks;
        },
    );

    spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Agent radius",
            value: debug.radius(),
            value_text: radius_text(debug.radius()),
            range: (
                POLYMESH_AGENT_RADIUS_MIN,
                POLYMESH_AGENT_RADIUS_MAX,
                POLYMESH_AGENT_RADIUS_STEP,
            ),
        },
        PolymeshRadiusLabel,
        PolymeshRadiusSlider,
        on_radius_change,
    );
}

/// Строка-тумблер `Enabled` со значением справа — та же кнопка-строка, что
/// листает значения в панелях Roads и Trees, только значение булево.
fn spawn_toggle_row<L: Component, M>(
    commands: &mut Commands,
    panel: Entity,
    label: &str,
    value: bool,
    value_marker: L,
    on_activate: impl IntoObserverSystem<Activate, (), M>,
) {
    let row = commands
        .spawn((
            Button,
            PolymeshEnabledRow,
            Pickable::default(),
            // `Hovered` кормит UI-picking, `Pressed` ставит виджет — оба нужны
            Hovered::default(),
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(6.),
                padding: UiRect {
                    top: px(4.),
                    right: px(8.),
                    bottom: px(4.),
                    left: px(8.),
                },
                ..default()
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![
                (
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::srgb(0.75, 0.78, 0.75)),
                    Node {
                        flex_grow: 1.,
                        ..default()
                    },
                ),
                (
                    value_marker,
                    Text::new(enabled_text(value)),
                    TextFont {
                        font_size: FontSize::Px(12.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ),
            ],
        ))
        .observe(on_activate)
        .id();
    commands.entity(panel).add_child(row);
}

fn enabled_text(enabled: bool) -> String {
    if enabled { "On" } else { "Off" }.to_string()
}

/// Ползунок дискретный: ресурс правится только на реальной смене шага —
/// каждый шаг перезапускает постройку меша.
fn on_radius_change(
    change: On<ValueChange<f32>>,
    mut commands: Commands,
    mut debug: ResMut<PolymeshDebug>,
) {
    let stepped = quantize(
        change.value,
        POLYMESH_AGENT_RADIUS_MIN,
        POLYMESH_AGENT_RADIUS_MAX,
        POLYMESH_AGENT_RADIUS_STEP,
    );
    commands.entity(change.source).insert(SliderValue(stepped));
    if (debug.agent_radius - stepped).abs() > f32::EPSILON {
        debug.agent_radius = stepped;
    }
}

fn radius_text(radius: f32) -> String {
    format!("{radius:.1} m")
}

/// Осветление строки под курсором и при нажатии (как у панели Roads).
fn highlight_rows(
    mut rows: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<PolymeshEnabledRow>>,
) {
    for (hovered, pressed, mut background) in &mut rows {
        let lighten = if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.set_if_neq(BackgroundColor(row_color(lighten)));
    }
}

/// Полигональный и сеточный слои закрашивают одно и то же — непроходимое —
/// поверх одной карты, и включённые вместе они читаются как один слой с
/// удвоенной альфой: сравнить их точность, ради чего всё и делалось,
/// невозможно. Поэтому включение одного гасит другой. Гасит **только**
/// включение: обратная правка видит выключенный ресурс и ничего не пишет,
/// так что цикла из двух систем, толкающих друг друга, не выходит.
///
/// Спор идёт только о картинке, поэтому гасится `show`, а не `enabled`:
/// включить сеточный оверлей — значит перестать рисовать полигональный, но
/// не свести пешек с меша. Кто по чему ходит, решает строка `Enabled`.
fn enforce_overlay_exclusivity(
    mut polymesh: ResMut<PolymeshDebug>,
    mut navmesh: ResMut<DebugNavmesh>,
) {
    let polymesh_drawn = polymesh.enabled && polymesh.show;
    if polymesh.is_changed() && polymesh_drawn && navmesh.0 {
        navmesh.0 = false;
    } else if navmesh.is_changed() && navmesh.0 && polymesh_drawn {
        polymesh.show = false;
    }
}

/// Актуализация подписей и бегунка после правки ресурса извне (BRP,
/// восстановленные настройки, взаимное гашение слоёв) — паттерн
/// `sync_noise_values`.
fn sync_polynav_values(
    debug: Res<PolymeshDebug>,
    mut toggles: Query<(&mut Text, &PolymeshToggleLabel)>,
    mut labels: Query<&mut Text, (With<PolymeshRadiusLabel>, Without<PolymeshToggleLabel>)>,
    sliders: Query<(Entity, &SliderValue), With<PolymeshRadiusSlider>>,
    mut commands: Commands,
) {
    for (mut text, toggle) in &mut toggles {
        text.0 = enabled_text(toggle.value(&debug));
    }
    for mut text in &mut labels {
        text.0 = radius_text(debug.radius());
    }
    for (slider, value) in &sliders {
        if (value.0 - debug.radius()).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(debug.radius()));
        }
    }
}

/// Оверлей построенного меша: заливка непроходимых контуров плюс рёбра
/// полигонов, всё одним merged-мешем. Ключ кеша — на маркере: пересборка
/// только когда постройка сменилась, а не на каждом тычке ресурса.
fn sync_polymesh_overlay(
    mut commands: Commands,
    debug: Res<PolymeshDebug>,
    poly: Res<PolyNavmesh>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    overlay: Query<(Entity, &PolymeshOverlayMarker)>,
) {
    let generation = poly.generation();
    let radius_bits = poly.built_radius().to_bits();
    let visible = debug.enabled && debug.show;
    if visible
        && overlay
            .iter()
            .any(|(_, drawn)| drawn.generation == generation && drawn.radius_bits == radius_bits)
    {
        return;
    }
    for (entity, _) in &overlay {
        commands.entity(entity).despawn();
    }
    if !visible {
        return;
    }
    let Some(built) = poly.build() else {
        return;
    };

    let mut builder = MeshBuilder::default();
    // сначала заливка — внутри одного меша порядок индексов и есть порядок
    // растеризации, так что рёбра лягут поверх неё
    let blocked = POLYMESH_BLOCKED_COLOR.to_linear();
    for obstacle in &built.obstacles {
        builder.push_polygon(obstacle, &[], blocked);
    }
    let color = POLYMESH_EDGE_COLOR.to_linear();
    for layer in &built.mesh.layers {
        // общее ребро соседних полигонов рисуется один раз — иначе на
        // полупрозрачном штрихе каждый внутренний шов был бы вдвое темнее
        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        for polygon in &layer.polygons {
            let count = polygon.vertices.len();
            for index in 0..count {
                let a = polygon.vertices[index];
                let b = polygon.vertices[(index + 1) % count];
                if !seen.insert((a.min(b), a.max(b))) {
                    continue;
                }
                // polyanya живёт на glam 0.30 — конверсия по полям. Координаты
                // вершин локальные для слоя: чанк триангулирован от своего
                // угла, мировая точка — плюс `offset`
                let origin = Vec2::new(layer.offset.x, layer.offset.y);
                let from = &layer.vertices[a as usize].coords;
                let to = &layer.vertices[b as usize].coords;
                builder.push_stroke(
                    &[
                        origin + Vec2::new(from.x, from.y),
                        origin + Vec2::new(to.x, to.y),
                    ],
                    false,
                    POLYMESH_EDGE_WIDTH,
                    color,
                );
            }
        }
    }

    // границы чанков — последними, чтобы легли поверх рёбер меша. Условия нет:
    // сетка берётся из самой постройки, и у плоского меша она 1x1, то есть ни
    // одной внутренней линии. Рисуется ровно то, по чему ходит поиск
    {
        let (grid, chunk_size) = built.chunks();
        let chunk_color = POLYMESH_CHUNK_COLOR.to_linear();
        for column in 1..grid.x {
            let x = column as f32 * chunk_size.x;
            builder.push_stroke(
                &[Vec2::new(x, 0.0), Vec2::new(x, MAP_SIZE.y)],
                false,
                POLYMESH_CHUNK_WIDTH,
                chunk_color,
            );
        }
        for row in 1..grid.y {
            let y = row as f32 * chunk_size.y;
            builder.push_stroke(
                &[Vec2::new(0.0, y), Vec2::new(MAP_SIZE.x, y)],
                false,
                POLYMESH_CHUNK_WIDTH,
                chunk_color,
            );
        }
    }

    if builder.is_empty() {
        return;
    }
    commands.spawn((
        PolymeshOverlayMarker {
            generation,
            radius_bits,
        },
        Mesh2d(meshes.add(builder.build())),
        MeshMaterial2d(materials.add(ColorMaterial {
            alpha_mode: AlphaMode2d::Blend,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, POLYMESH_OVERLAY_Z),
        DespawnOnExit(AppState::Playing),
        Name::new("polymesh_overlay"),
    ));
}
