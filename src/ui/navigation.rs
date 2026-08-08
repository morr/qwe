//! Панель Navigation — один UI на обе подсистемы поиска пути: сеточный
//! **Navmesh** (`navigation/navmesh.rs`, заливка непроходимых тайлов) и
//! полигональный **Polymesh** (`navigation/polymesh.rs`, меш polyanya).
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
//! выбрана** (`Display::None`, левую колонку перестыкует
//! `ui::stack_bottom_columns`): ползунок радиуса агента ничего не значит, пока
//! ходят по сетке, а алгоритм поиска по ней — пока ходят по мешу.
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

use std::collections::HashSet;

use bevy::color::Mix;
use bevy::ecs::system::{IntoObserverSystem, SystemParam};
use bevy::picking::hover::Hovered;
use bevy::sprite_render::AlphaMode2d;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, SliderValue, ValueChange};

use bevy::prelude::*;

use crate::determinism::Determinism;
use crate::human::HumanStyle;
use crate::loading::{AppState, WorldInitSet};
use crate::map::MeshBuilder;
use crate::movement::{
    SeparationLab, SeparationStyle, SlotLab, SlotSearch, separation_allowed_by_mode,
};
use crate::navigation::{PathfindingAlgorithm, PolyNavmesh, PolymeshDebug};
use crate::settings::{
    CLAIM_SEARCH_MAX, CLAIM_SEARCH_MIN, CLAIM_SEARCH_STEP, HUMAN_BODY_RADIUS_MAX,
    HUMAN_BODY_RADIUS_MIN, HUMAN_BODY_RADIUS_STEP, MAP_SIZE, POLYMESH_AGENT_RADIUS_MAX,
    POLYMESH_AGENT_RADIUS_MIN, POLYMESH_AGENT_RADIUS_STEP, SEPARATION_LEFT_SHARE_MAX,
    SEPARATION_LEFT_SHARE_MIN, SEPARATION_LEFT_SHARE_STEP, SEPARATION_PASS_SQUEEZE_MAX,
    SEPARATION_PASS_SQUEEZE_MIN, SEPARATION_PASS_SQUEEZE_STEP, SLOT_REGROUP_MAX, SLOT_REGROUP_MIN,
    SLOT_REGROUP_STEP,
};
use crate::ui::slider::{SliderRow, quantize, spawn_slider_row};
use crate::ui::{
    DebugNavmesh, GameUiRoot, UI_SCREEN_EDGE_PX_OFFSET, UI_TEXT_SHADOW, UiLeftColumnSlot,
    UiOpacity, UiPanelGapBelow, ui_color,
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
/// Приглушённая подпись — тем же способом, каким панели показывают неактивное:
/// цветом, а не отдельной иконкой.
const DIMMED_VALUE: Color = Color::srgb(0.45, 0.45, 0.45);
/// Отступ слева у строк-настроек: настройка принадлежит подсистеме над ней, и
/// лесенка говорит это раньше, чем читается подпись.
const NESTED_ROW_INDENT_PX: f32 = 18.;

fn row_color(lighten: f32) -> Color {
    ui_color(UiOpacity::Heavy).mix(&Color::WHITE, lighten)
}

/// Любая строка-кнопка панели — по ней система подсветки находит их все.
#[derive(Component)]
struct NavPanelRow;

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

/// Группа ручек толпы — только для того, чтобы разложить их по заголовкам.
/// Ни ресурса, ни компонента: прятать группы незачем (см. док модуля).
#[derive(Clone, Copy, PartialEq, Eq)]
enum KnobGroup {
    Separation,
    Slots,
}

/// Строка-тумблер расталкивания: собственная ветка в подсветке, потому что под
/// детерминизмом и на сеточной навигации она не откликается вовсе.
#[derive(Component)]
struct SeparationToggleRow;

/// Ползунок группы Separation — прячется, когда расталкивание не работает.
#[derive(Component)]
struct SeparationKnobRow;

/// Числовая ручка толпы. Одним enum'ом на обе подвкладки, а не маркером на
/// строку: спавн, синхронизация подписи и синхронизация бегунка — по разу на
/// все пять, и новая ручка добавляется одной веткой.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Knob {
    PassSqueeze,
    LeftShare,
    BodyRadius,
    SlotSearch,
    Regroup,
}

/// Подпись значения такой строки.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct KnobValueLabel(Knob);

/// Её ползунок.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct KnobSlider(Knob);

impl Knob {
    const ALL: [Self; 5] = [
        Self::PassSqueeze,
        Self::LeftShare,
        Self::BodyRadius,
        Self::SlotSearch,
        Self::Regroup,
    ];

    fn group(self) -> KnobGroup {
        match self {
            Self::PassSqueeze | Self::LeftShare => KnobGroup::Separation,
            Self::BodyRadius | Self::SlotSearch | Self::Regroup => KnobGroup::Slots,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PassSqueeze => "Pass squeeze",
            Self::LeftShare => "Left share",
            Self::BodyRadius => "Body radius",
            Self::SlotSearch => "Slot search",
            Self::Regroup => "Regroup",
        }
    }

    /// `(min, max, шаг)` — из `settings.rs`, как у остальных ползунков.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::PassSqueeze => (
                SEPARATION_PASS_SQUEEZE_MIN,
                SEPARATION_PASS_SQUEEZE_MAX,
                SEPARATION_PASS_SQUEEZE_STEP,
            ),
            Self::LeftShare => (
                SEPARATION_LEFT_SHARE_MIN,
                SEPARATION_LEFT_SHARE_MAX,
                SEPARATION_LEFT_SHARE_STEP,
            ),
            Self::BodyRadius => (
                HUMAN_BODY_RADIUS_MIN,
                HUMAN_BODY_RADIUS_MAX,
                HUMAN_BODY_RADIUS_STEP,
            ),
            Self::SlotSearch => (CLAIM_SEARCH_MIN, CLAIM_SEARCH_MAX, CLAIM_SEARCH_STEP),
            Self::Regroup => (SLOT_REGROUP_MIN, SLOT_REGROUP_MAX, SLOT_REGROUP_STEP),
        }
    }

    /// Значение ручки. По ссылкам на сами ресурсы, а не по `NavPanelValues`:
    /// то же чтение нужно наблюдателю строки, а у него они `ResMut`.
    fn get(
        self,
        lab: &SeparationLab,
        slots: &SlotLab,
        human: &HumanStyle,
        search: &SlotSearch,
    ) -> f32 {
        match self {
            Self::PassSqueeze => lab.pass_squeeze,
            Self::LeftShare => lab.left_share,
            Self::BodyRadius => human.body_radius,
            Self::SlotSearch => search.0,
            Self::Regroup => slots.regroup,
        }
    }

    fn set(self, knobs: &mut KnobResources, value: f32) {
        match self {
            Self::PassSqueeze => knobs.separation_lab.pass_squeeze = value,
            Self::LeftShare => knobs.separation_lab.left_share = value,
            Self::BodyRadius => knobs.human.body_radius = value,
            Self::SlotSearch => knobs.search.0 = value,
            Self::Regroup => knobs.slot_lab.regroup = value,
        }
    }

    /// Единица измерения в подписи: у радиусов и возврата это метры, у двух
    /// долей — голое число.
    fn value_text(self, value: f32) -> String {
        match self {
            Self::PassSqueeze | Self::LeftShare => format!("{value:.2}"),
            Self::BodyRadius | Self::Regroup => format!("{value:.2} m"),
            Self::SlotSearch => format!("{value:.0} m"),
        }
    }
}

/// Ресурсы, которые ползунки толпы правят. Отдельным `SystemParam`, чтобы
/// наблюдатель строки брал их одним аргументом, а не четырьмя.
#[derive(SystemParam)]
struct KnobResources<'w> {
    separation_lab: ResMut<'w, SeparationLab>,
    slot_lab: ResMut<'w, SlotLab>,
    human: ResMut<'w, HumanStyle>,
    search: ResMut<'w, SlotSearch>,
}

impl KnobResources<'_> {
    fn value(&self, knob: Knob) -> f32 {
        knob.get(
            &self.separation_lab,
            &self.slot_lab,
            &self.human,
            &self.search,
        )
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
    PolymeshRadius,
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
            Self::PolymeshRadius => radius_text(poly.radius()),
            // под детерминизмом и на сеточной навигации расталкивания нет
            // вовсе (`movement::separation_runs`), каким бы ни был
            // `SeparationStyle`, — панель обязана показывать положение дел, а
            // не намерение. Собственное значение стиля при этом сохраняется:
            // возврат режима вернёт панель к нему
            Self::Separation => enabled_text(values.separation_enabled()),
        }
    }

    /// Цвет подписи, если он у строки не белый по умолчанию.
    fn color(self, values: &NavPanelValues) -> Option<Color> {
        match self {
            Self::Separation if !values.separation_allowed() => Some(DIMMED_VALUE),
            Self::Separation => Some(Color::WHITE),
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
    slot_lab: Res<'w, SlotLab>,
    human: Res<'w, HumanStyle>,
    search: Res<'w, SlotSearch>,
}

impl NavPanelValues<'_> {
    fn knob(&self, knob: Knob) -> f32 {
        knob.get(
            &self.separation_lab,
            &self.slot_lab,
            &self.human,
            &self.search,
        )
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

pub struct UiNavigationPlugin;

impl Plugin for UiNavigationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, render_navigation_panel)
            // после смены города: оверлей умер с DespawnOnExit, ресурсы живы
            .add_systems(
                OnEnter(AppState::Playing),
                sync_polymesh_overlay.in_set(WorldInitSet::Spawn),
            )
            .add_systems(
                Update,
                (
                    highlight_rows,
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
                    sync_knob_values.run_if(
                        resource_changed::<SeparationLab>
                            .or_else(resource_changed::<SlotLab>)
                            .or_else(resource_changed::<HumanStyle>)
                            .or_else(resource_changed::<SlotSearch>),
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

fn render_navigation_panel(mut commands: Commands, values: NavPanelValues) {
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
            // ряд кнопок под панелью — другой род UI, вплотную он читался как
            // её первая строка
            UiPanelGapBelow,
            GameUiRoot,
            Visibility::Hidden,
            Name::new("navigation_panel"),
            children![(
                Text::new("Navigation"),
                TextFont {
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::WHITE),
                UI_TEXT_SHADOW,
            )],
        ))
        .id();

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

    let radius = values.polymesh.radius();
    let radius_row = spawn_slider_row(
        &mut commands,
        panel,
        SliderRow {
            label: "Agent radius",
            value: radius,
            value_text: radius_text(radius),
            range: (
                POLYMESH_AGENT_RADIUS_MIN,
                POLYMESH_AGENT_RADIUS_MAX,
                POLYMESH_AGENT_RADIUS_STEP,
            ),
        },
        NavValueLabel::PolymeshRadius,
        PolymeshRadiusSlider,
        on_radius_change,
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
            Node {
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
            },
            BackgroundColor(row_color(ROW_LIGHTEN)),
            children![(
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.78, 0.75)),
            )],
        ))
        .id();
    commands.entity(panel).add_child(row);
}

/// Ползунки одной группы, в порядке [`Knob::ALL`].
fn spawn_knob_rows(
    commands: &mut Commands,
    panel: Entity,
    values: &NavPanelValues,
    group: KnobGroup,
) {
    for knob in Knob::ALL.into_iter().filter(|knob| knob.group() == group) {
        let value = values.knob(knob);
        let row = spawn_slider_row(
            commands,
            panel,
            SliderRow {
                label: knob.label(),
                value,
                value_text: knob.value_text(value),
                range: knob.range(),
            },
            KnobValueLabel(knob),
            KnobSlider(knob),
            move |change: On<ValueChange<f32>>,
                  mut commands: Commands,
                  mut knobs: KnobResources| {
                let (min, max, step) = knob.range();
                let stepped = quantize(change.value, min, max, step);
                commands.entity(change.source).insert(SliderValue(stepped));
                // ресурс правится только на реальной смене шага: иначе каждый
                // пиксель протяжки метил бы его изменённым
                if (knobs.value(knob) - stepped).abs() > f32::EPSILON {
                    knob.set(&mut knobs, stepped);
                }
            },
        );
        indent_slider_row(commands, row);
        if group == KnobGroup::Separation {
            // начальная видимость ставится здесь, а не оставляется системе:
            // она ходит под `resource_changed`, а на первом кадре ресурсы уже
            // не «изменённые» — при выключенном расталкивании ползунки так и
            // висели бы до первого клика по чему-нибудь
            let visible = values.separation_enabled();
            commands
                .entity(row)
                .insert(SeparationKnobRow)
                .entry::<Node>()
                .and_modify(move |mut node| node.display = display_of(visible));
        }
    }
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
        RowStyle::Backend => px(8.),
        RowStyle::Setting(_) => px(8. + NESTED_ROW_INDENT_PX),
    };
    let row = commands
        .spawn((
            Button,
            NavPanelRow,
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
                    left,
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
                    Text::new(value),
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
    if let RowStyle::Setting(section) = style {
        commands.entity(row).insert(section);
    }
    commands.entity(panel).add_child(row);
    row
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

/// Осветление строки под курсором и при нажатии (как у панели Roads), плюс два
/// исключения этой панели. Одной системой, а не тремя: фон у строки один, и
/// три системы, пишущие его по разным правилам, затирали бы друг друга в
/// зависимости от порядка в расписании.
fn highlight_rows(
    determinism: Res<Determinism>,
    polymesh: Res<PolymeshDebug>,
    mut rows: Query<
        (
            &Hovered,
            Has<Pressed>,
            Has<SeparationToggleRow>,
            &mut BackgroundColor,
        ),
        With<NavPanelRow>,
    >,
) {
    let separation_allowed = separation_allowed_by_mode(determinism.0, polymesh.enabled);
    for (hovered, pressed, separation_toggle, mut background) in &mut rows {
        let lighten = if separation_toggle && !separation_allowed {
            // тумблер не откликается вовсе — подсветка обещала бы, что клик
            // что-то сделает
            ROW_LIGHTEN
        } else if pressed {
            PRESSED_LIGHTEN
        } else if hovered.get() {
            HOVER_LIGHTEN
        } else {
            ROW_LIGHTEN
        };
        background.set_if_neq(BackgroundColor(row_color(lighten)));
    }
}

/// Подписи и бегунки ручек толпы вслед за ресурсами: их правят не только эти
/// ползунки (BRP, панель демо-сцены, пресеты стенда), а расходиться показанному
/// и настоящему нельзя.
fn sync_knob_values(
    values: NavPanelValues,
    mut commands: Commands,
    mut labels: Query<(&KnobValueLabel, &mut Text)>,
    sliders: Query<(Entity, &KnobSlider, &SliderValue)>,
) {
    for (label, mut text) in &mut labels {
        let next = label.0.value_text(values.knob(label.0));
        if text.0 != next {
            text.0 = next;
        }
    }
    for (entity, slider, value) in &sliders {
        let next = values.knob(slider.0);
        if (value.0 - next).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(next));
        }
    }
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

/// Ползунки расталкивания — только пока оно работает. Их прячет то же, что
/// гасит подпись группы: детерминизм, сеточный бэкенд и собственный тумблер.
/// Настраивать нечего, пока механизм не запускается вовсе, — та же логика, по
/// которой уходят настройки невыбранного бэкенда.
///
/// Строка-заголовок остаётся: она и есть тумблер, которым расталкивание
/// возвращают, — спрятать её значило бы запереть себя снаружи.
fn sync_separation_knob_visibility(
    values: NavPanelValues,
    mut rows: Query<&mut Node, With<SeparationKnobRow>>,
) {
    let display = display_of(values.separation_enabled());
    for mut node in &mut rows {
        if node.display != display {
            node.display = display;
        }
    }
}

/// Актуализация подписей и бегунка после правки ресурса извне (BRP,
/// восстановленные настройки, хоткей N) — паттерн `sync_noise_values`.
fn sync_nav_values(
    values: NavPanelValues,
    mut labels: Query<(&mut Text, &mut TextColor, &NavValueLabel)>,
    sliders: Query<(Entity, &SliderValue), With<PolymeshRadiusSlider>>,
    mut commands: Commands,
) {
    for (mut text, mut color, label) in &mut labels {
        text.0 = label.text(&values);
        if let Some(next) = label.color(&values) {
            color.0 = next;
        }
    }
    let radius = values.polymesh.radius();
    for (slider, value) in &sliders {
        if (value.0 - radius).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(radius));
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
