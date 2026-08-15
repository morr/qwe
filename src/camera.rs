use bevy::app::AppExit;
use bevy::camera_controller::pan_camera::{MousePanSettings, PanCamera, PanCameraPlugin};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SaveSettingsSync, SettingsGroup};
use bevy::window::PrimaryWindow;

use crate::city::City;
use crate::loading::{AppState, WorldInitSet};
use crate::portal::PortalPos;
use crate::prefs::TrackPrefExt;
use crate::restart::RestartEvent;

/// Зум = масштаб трансформа камеры: мировых метров на экранный пиксель.
/// 0.0625 (= 1/16) — «крупный план», нативный пиксель ассетов (16 px = 1 м);
/// ~4.4 — вся карта (5600 м) в кадре при окне 1280.
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 4.5;
/// Публичный: от него считается стартовая ступень зум-LOD трамвая
/// (`map::tram::TramZoomBucket`).
pub const START_ZOOM: f32 = 0.4;
/// Множитель зума на один щелчок колеса.
const ZOOM_STEP: f32 = 1.12;
/// Скорость WASD-пана в *экранных* логических пикселях в секунду — как у
/// `drag_pan`, поэтому на любом масштабе карта уезжает одинаково быстро.
/// (`pan_speed` у PanCamera задаётся в мировых метрах и на крупном плане
/// швыряет камеру, а на общем — еле тащит.)
const PAN_SPEED: f32 = 1125.0;

/// Дебаунс записи вида камеры, секунды реального времени: пан и зум идут
/// пачками по многу кадров подряд, и писать файл на каждый кадр движения
/// нельзя — ждём, пока камера постоит.
const VIEW_SAVE_DEBOUNCE: f32 = 1.0;
/// Троттл к тому же дебаунсу: пока камеру тащат без остановки, «постоит» не
/// наступает никогда, — пишем не реже раза в эти секунды.
const VIEW_SAVE_THROTTLE: f32 = 10.0;

/// Окно двойного нажатия R, секунды реального времени: второй рестарт внутри
/// него ставит камеру на портал независимо от [`CameraPositionMode`].
/// Полсекунды — обычное окно двойного клика; на паузе и на 30x оно должно быть
/// одинаковым, отсюда `Time<Real>`.
const RESTART_DOUBLE_PRESS: f32 = 0.5;

/// Откуда камера начинает — кнопка `position` в ряду тумблеров
/// (`ui/debug.rs`), выбор запоминается между запусками (`prefs.rs`).
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "camera", key = "position_mode")]
pub enum CameraPositionMode {
    /// Старт приложения и R — портал в `START_ZOOM`.
    Reset,
    /// Старт приложения и R — точка и зум, на которых вышли из игры: чаще
    /// работают с одним и тем же участком карты, и искать его заново после
    /// каждого запуска дороже, чем нажать R.
    #[default]
    Save,
}

impl CameraPositionMode {
    /// Подпись на кнопке.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Save => "save",
        }
    }

    /// Следующий режим по кругу — кнопка листает одним кликом.
    pub fn next(self) -> Self {
        match self {
            Self::Reset => Self::Save,
            Self::Save => Self::Reset,
        }
    }
}

/// Вид камеры, записанный при выходе из игры в режиме [`CameraPositionMode::Save`].
/// `None` — сохранять ещё не приходилось, тогда старт идёт от портала.
///
/// Та же группа настроек `camera`, что и у режима: `bevy_settings` сливает
/// одноимённые группы при записи и игнорирует чужие ключи при чтении, а
/// `position_mode` с полями `position` / `zoom` не пересекается.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "camera")]
pub struct SavedCameraView {
    position: Option<Vec2>,
    zoom: Option<f32>,
}

/// Куда и с каким зумом ставится камера при входе в мир.
struct CameraView {
    position: Vec2,
    zoom: f32,
}

/// Состояние дебаунса записи вида (см. [`track_camera_view`]). Время реальное:
/// пауза и скорость симуляции к записи настроек отношения не имеют.
#[derive(Default)]
struct ViewSaveDebounce {
    /// Когда вид разошёлся с записанным на диске — `None`, если не расходился.
    dirty_since: Option<f32>,
    /// Когда камера двигалась в последний раз.
    moved_at: f32,
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PanCameraPlugin)
            .init_resource::<CameraPositionMode>()
            .init_resource::<SavedCameraView>()
            .register_type::<CameraPositionMode>()
            .register_type::<SavedCameraView>()
            // `SavedCameraView` не отслеживается: он меняется каждый кадр
            // протяжки, и запись по изменению перезаписывала бы файл на кадр —
            // у него свой дебаунс (`track_camera_view`)
            .track_pref::<CameraPositionMode>()
            .add_systems(Startup, spawn_camera)
            // карта нового города лежит в тех же координатах, но портал
            // переезжает — камеру возвращаем к нему на каждой загрузке
            .add_systems(
                OnEnter(AppState::Playing),
                place_camera_on_world_ready.in_set(WorldInitSet::Spawn),
            )
            // R сносит сцену (`restart.rs`) — камеру по тому же событию ставит
            // сюда же, в свой модуль, а не в чужой обсервер
            .add_observer(on_restart_place_camera)
            // Мышь и WASD ведём сами (см. ниже); у PanCamera остаётся только
            // применение zoom_factor к масштабу трансформа.
            .add_systems(
                Update,
                (
                    zoom_to_cursor,
                    drag_pan,
                    // WASD в поле ввода — буквы: набирать seed, уезжая
                    // камерой через полгорода, невозможно
                    key_pan.run_if(not(crate::ui::typing_in_text_input)),
                ),
            )
            .add_systems(
                Update,
                track_camera_view
                    .after(zoom_to_cursor)
                    .after(drag_pan)
                    .after(key_pan)
                    .run_if(|mode: Res<CameraPositionMode>| *mode == CameraPositionMode::Save),
            )
            // `after`: закрытие окна пишет `AppExit` тоже в `Last`
            // (`exit_on_all_closed` в наборе `ExitSystems`), и без порядка наша
            // система успевала пройти раньше — вид не сохранялся
            .add_systems(
                Last,
                save_camera_view_on_exit.after(bevy::window::ExitSystems),
            );
    }
}

fn spawn_camera(
    mut commands: Commands,
    city: Res<City>,
    mode: Res<CameraPositionMode>,
    saved: Res<SavedCameraView>,
) {
    // портал ещё не снапнут по navmesh — до входа в `Playing` известен только
    // хинт города; `place_camera_on_world_ready` поправит на точный
    let view = start_view(*mode, &saved, city.portal_hint());
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(view.position.extend(0.0)).with_scale(Vec3::splat(view.zoom)),
        Msaa::Off,
        PanCamera {
            zoom_factor: view.zoom,
            min_zoom: MIN_ZOOM,
            max_zoom: MAX_ZOOM,
            // колесо обрабатывает `zoom_to_cursor`, а не PanCamera
            zoom_speed: 0.0,
            // `=`/`-` отданы скорости симуляции (sim_time)
            key_zoom_in: None,
            key_zoom_out: None,
            // WASD ведёт `key_pan`: шаг PanCamera задан в мировых метрах и
            // потому зависит от масштаба
            pan_speed: 0.0,
            key_up: None,
            key_down: None,
            key_left: None,
            key_right: None,
            // без поворота камеры
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            // drag ведёт `drag_pan`: якорит мир к курсору (1:1, как
            // bevy_pancam в zxc) и не зависит от масштаба ретины
            mouse_pan_settings: MousePanSettings {
                enabled: false,
                button: MouseButton::Left,
            },
            ..default()
        },
        Name::new("main_camera"),
    ));
}

/// Вид, с которого начинается мир: в режиме `Save` — сохранённый при выходе,
/// иначе портал в стартовом зуме.
///
/// Зум идёт вместе с позицией: разглядывать новый город с того приближения, на
/// котором бросили предыдущий, незачем — да и на общем плане (4.5) отъезд после
/// смены города читается как «карта не загрузилась». Сохранённый зум клампится:
/// в файле настроек лежит что угодно, а вне `MIN_ZOOM..MAX_ZOOM` камера
/// показывает пустоту.
fn start_view(mode: CameraPositionMode, saved: &SavedCameraView, portal: Vec2) -> CameraView {
    match (mode, saved.position, saved.zoom) {
        (CameraPositionMode::Save, Some(position), Some(zoom)) => CameraView {
            position,
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
        },
        _ => CameraView {
            position: portal,
            zoom: START_ZOOM,
        },
    }
}

/// Масштаб трансформа PanCamera держит сам, но пишет его только в кадре, где
/// сам же менял `zoom_factor`, — при постановке вида извне ставим оба.
fn apply_view(transform: &mut Transform, controller: &mut PanCamera, view: CameraView) {
    transform.translation = view.position.extend(transform.translation.z);
    transform.scale = Vec3::splat(view.zoom);
    controller.zoom_factor = view.zoom;
}

/// Постановка камеры по загруженному миру: позиция портала снапится по navmesh
/// уже загруженного города, так что известна только к входу в `Playing`.
///
/// Сохранённый вид уважается лишь на первом входе, то есть на старте
/// приложения: следующий вход с **другим** городом — это его смена, а
/// сохранённая точка принадлежит прошлой карте, и в новом городе камеру ждут
/// у его портала. Перезагрузка **того же** города (смена размера навтайла)
/// камеру не трогает вовсе: пользователь смотрит на тот же участок карты, и
/// увозить его к порталу — значит терять место, которое он разглядывал.
fn place_camera_on_world_ready(
    mut last_city: Local<Option<City>>,
    city: Res<City>,
    mode: Res<CameraPositionMode>,
    saved: Res<SavedCameraView>,
    portal: Res<PortalPos>,
    mut camera: Single<(&mut Transform, &mut PanCamera), With<Camera2d>>,
) {
    let first_load = last_city.is_none();
    let same_city = *last_city == Some(*city);
    *last_city = Some(*city);
    if same_city {
        return;
    }

    let mode = if first_load {
        *mode
    } else {
        CameraPositionMode::Reset
    };
    let (transform, controller) = &mut *camera;
    apply_view(transform, controller, start_view(mode, &saved, portal.0));
}

/// R — рестарт сцены (`restart.rs`) вместе с камерой: она возвращается туда же,
/// куда встаёт на старте приложения, то есть по настройке `position`.
fn on_restart_place_camera(
    event: On<RestartEvent>,
    time: Res<Time<Real>>,
    mut previous_restart: Local<Option<f32>>,
    mode: Res<CameraPositionMode>,
    saved: Res<SavedCameraView>,
    portal: Res<PortalPos>,
    mut camera: Single<(&mut Transform, &mut PanCamera), With<Camera2d>>,
) {
    let now = time.elapsed_secs();
    let double_press =
        previous_restart.is_some_and(|previous| now - previous < RESTART_DOUBLE_PRESS);
    *previous_restart = Some(now);

    // второе R подряд — всегда портал, каким бы ни был режим: это жест
    // «потерялся на карте, верни меня к началу», а не смена настройки.
    // Того же просит рестарт по смене настройки мира (`RestartEvent::to_portal`)
    let mode = if double_press || event.to_portal {
        CameraPositionMode::Reset
    } else {
        *mode
    };
    let (transform, controller) = &mut *camera;
    apply_view(transform, controller, start_view(mode, &saved, portal.0));
}

/// Запись вида камеры прямо по ходу игры, чтобы он переживал и те выходы, до
/// которых [`save_camera_view_on_exit`] не дотягивается (Cmd-Q, `brp quit`,
/// падение).
///
/// Дебаунс + троттл, оба на реальном времени: движение камеры — это десятки
/// кадров подряд, и запись на каждый кадр означала бы сотню перезаписей
/// `settings.toml` на одну протяжку. Файл уходит на диск через
/// [`VIEW_SAVE_DEBOUNCE`] после того, как камера встала, а если её тащат без
/// остановки — не реже раза в [`VIEW_SAVE_THROTTLE`], иначе «встала» не
/// наступит вовсе.
///
/// Пишем сами, а не первопартийным `SaveSettingsDeferred`: его таймер тикает
/// от `Res<Time>`, то есть от виртуальных часов, — на паузе отложенная запись
/// не случилась бы никогда, а на 30x пришла бы в тридцать раз раньше срока.
fn track_camera_view(
    time: Res<Time<Real>>,
    mut debounce: Local<ViewSaveDebounce>,
    mut saved: ResMut<SavedCameraView>,
    camera: Single<(&Transform, &PanCamera), With<Camera2d>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let (transform, controller) = *camera;
    let view = SavedCameraView {
        position: Some(transform.translation.truncate()),
        zoom: Some(controller.zoom_factor),
    };

    // `set_if_neq` не только пишет, но и метит ресурс изменённым — по этой
    // метке `SaveSettingsSync::IfChanged` и решает, что файл пора трогать
    if saved.set_if_neq(view) {
        debounce.moved_at = now;
        debounce.dirty_since.get_or_insert(now);
    }

    let Some(dirty_since) = debounce.dirty_since else {
        return;
    };
    if now - debounce.moved_at < VIEW_SAVE_DEBOUNCE && now - dirty_since < VIEW_SAVE_THROTTLE {
        return;
    }
    debounce.dirty_since = None;
    commands.queue(SaveSettingsSync::IfChanged);
}

/// Запись вида камеры при выходе из игры — в `Last`, **после**
/// `bevy::window::ExitSystems`: `AppExit` пишут и Esc
/// (`main.rs::close_on_esc`, `Update`), и закрытие окна крестиком
/// (`exit_on_all_closed` — тоже `Last`, и без `after` наша система успевала
/// пройти раньше, чем сообщение появлялось). После `Last` расписаний уже нет,
/// но команда записи применяется в его же конце, до того как раннер увидит
/// выход.
///
/// Пишем синхронно, как `prefs::save_prefs`: отложенная запись до выхода уже не
/// успевает. Мимо этой системы уходят два выхода — Cmd-Q на macOS (минует цикл
/// кадров вовсе, та же оговорка стоит в доках `bevy_settings`) и `brp quit`
/// (пишет `AppExit` из `RemoteLast`, то есть уже после `Last`); там вид
/// сохраняет [`track_camera_view`], с точностью до своего дебаунса.
fn save_camera_view_on_exit(
    mut exit: MessageReader<AppExit>,
    mode: Res<CameraPositionMode>,
    mut saved: ResMut<SavedCameraView>,
    camera: Option<Single<(&Transform, &PanCamera), With<Camera2d>>>,
    mut commands: Commands,
) {
    if exit.read().next().is_none() || *mode != CameraPositionMode::Save {
        return;
    }
    let Some(camera) = camera else {
        return;
    };
    let (transform, controller) = *camera;

    saved.set_if_neq(SavedCameraView {
        position: Some(transform.translation.truncate()),
        zoom: Some(controller.zoom_factor),
    });
    commands.queue(SaveSettingsSync::IfChanged);
}

/// Смещение курсора от центра окна в мировых осях (экранный y — вниз).
pub fn cursor_offset(window: &Window, cursor: Vec2) -> Vec2 {
    (cursor - window.size() / 2.0) * Vec2::new(1.0, -1.0)
}

/// Кусок мира в кадре — то, чем всякий гейт видимости отвечает на «стоит ли
/// этим заниматься»: диспетчер заявок, расталкивание, прогрев и обе отсечки
/// гизмо. Значение, а не пара `Single<&Transform, With<Camera2d>>` +
/// `Single<&Window>`: правило «окно пополам, умножить на зум» писалось на
/// каждом из этих мест заново, и ни одно из них не проверялось тестом, потому
/// что для проверки требовалась живая камера с окном.
///
/// Не `Camera::viewport` из bevy — тот в пикселях.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    /// Центр кадра, мировые метры.
    pub centre: Vec2,
    /// Полуразмер кадра с уже учтённым запасом.
    pub half_extent: Vec2,
    /// Мировых метров на логический пиксель — масштаб трансформа камеры.
    pub zoom: f32,
}

impl Viewport {
    /// `screens` — сколько экранов в стороны считать «в кадре»: `1.0` — ровно
    /// то, что видно, больше — запас за кромкой. Запас у каждого гейта свой и
    /// живёт рядом с ним, потому что вопрос у каждого свой: у прогрева
    /// запаса нет («видит ли пешку игрок»), у диспетчера и расталкивания —
    /// [`VIEW_MARGIN`](crate::movement::VIEW_MARGIN), у гизмо — свои экраны.
    pub fn of(window: &Window, camera: &Transform, screens: f32) -> Self {
        let zoom = camera.scale.x;
        Self {
            centre: camera.translation.truncate(),
            half_extent: window.size() / 2.0 * zoom * screens,
            zoom,
        }
    }

    /// Юго-западный угол кадра.
    pub fn min(&self) -> Vec2 {
        self.centre - self.half_extent
    }

    /// Северо-восточный угол кадра.
    pub fn max(&self) -> Vec2 {
        self.centre + self.half_extent
    }

    /// Точка в кадре. Кромка считается своей.
    ///
    /// `#[inline]`: спрашивают из циклов по всем дверям карты (под десять
    /// тысяч за кадр) и по всем идущим пешкам, а профиль `dev` собирает нас на
    /// `opt-level = 1` с раздельными codegen-unit'ами — без атрибута на месте
    /// прежней арифметики оказался бы вызов через границу модуля.
    #[inline]
    pub fn contains(&self, point: Vec2) -> bool {
        let offset = (point - self.centre).abs();
        offset.x <= self.half_extent.x && offset.y <= self.half_extent.y
    }

    /// Квадрат расстояния до центра кадра — ключ «ближе к центру раньше».
    #[inline]
    pub fn distance_from_centre_squared(&self, point: Vec2) -> f32 {
        (point - self.centre).length_squared()
    }
}

/// Зум колесом к точке под курсором: мировая точка под курсором остаётся
/// на месте, а не уезжает к центру экрана.
fn zoom_to_cursor(
    window: Single<&Window, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut query: Query<(&mut Transform, &mut PanCamera), With<Camera>>,
) {
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
    };
    if lines == 0.0 {
        return;
    }
    let Ok((mut transform, mut controller)) = query.single_mut() else {
        return;
    };

    let old_zoom = controller.zoom_factor;
    let new_zoom =
        (old_zoom * ZOOM_STEP.powf(-lines)).clamp(controller.min_zoom, controller.max_zoom);
    if new_zoom == old_zoom {
        return;
    }

    if let Some(cursor) = window.cursor_position() {
        let offset = cursor_offset(&window, cursor);
        let world_under_cursor = transform.translation.truncate() + offset * old_zoom;
        let translation = world_under_cursor - offset * new_zoom;
        transform.translation = translation.extend(transform.translation.z);
    }
    controller.zoom_factor = new_zoom;
    transform.scale = Vec3::splat(new_zoom);
}

/// Пан на WASD в экранной скорости: шаг умножается на `zoom_factor`, поэтому
/// на крупном плане камера проходит меньше метров, а на общем — больше, и на
/// экране карта в обоих случаях едет с одной скоростью.
///
/// Время реальное: пан не должен замирать вместе с паузой симуляции.
fn key_pan(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&mut Transform, &PanCamera), With<Camera>>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    let Some(dir) = dir.try_normalize() else {
        return;
    };
    let Ok((mut transform, controller)) = query.single_mut() else {
        return;
    };

    let delta = dir * PAN_SPEED * controller.zoom_factor * time.delta_secs();
    transform.translation += delta.extend(0.0);
}

/// Состояние протяжки левой кнопкой. Решение «камера или UI» принимается один
/// раз, в кадре нажатия, и держится до отпускания: протяжка ползунка плотности
/// уводит курсор с панели, и покадровая проверка «курсор над UI» отдала бы
/// остаток протяжки камере.
#[derive(Default, Clone, Copy)]
enum DragPan {
    #[default]
    Idle,
    /// Зажатие началось над панелью — камера в нём не участвует.
    OverUi,
    /// Зажатие началось над картой; хранится позиция курсора в прошлом кадре.
    Dragging(Vec2),
}

/// Курсор над каким-нибудь узлом `bevy_ui` (идиома из `zxc/src/input.rs`):
/// `HoverMap` собирает UI-пикинг, мировой ввод под панелью обрабатывать нельзя.
fn pointer_over_ui(hover_map: &HoverMap, ui_nodes: &Query<(), With<Node>>) -> bool {
    hover_map
        .values()
        .flat_map(|pointer| pointer.keys())
        .any(|entity| ui_nodes.contains(*entity))
}

/// Пан зажатой левой кнопкой: точка мира «схвачена» курсором и движется с
/// ним один в один (по логическим px, поэтому ретина-масштаб не удваивает
/// скорость, как это делал экранный `delta` у PanCamera).
fn drag_pan(
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut drag: Local<DragPan>,
    mut query: Query<(&mut Transform, &PanCamera), With<Camera>>,
) {
    if !buttons.pressed(MouseButton::Left) {
        *drag = DragPan::Idle;
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    if matches!(*drag, DragPan::Idle) {
        *drag = if pointer_over_ui(&hover_map, &ui_nodes) {
            DragPan::OverUi
        } else {
            DragPan::Dragging(cursor)
        };
        return;
    }
    let DragPan::Dragging(last) = *drag else {
        return;
    };
    let Ok((mut transform, controller)) = query.single_mut() else {
        return;
    };

    let delta = (cursor - last) * Vec2::new(1.0, -1.0) * controller.zoom_factor;
    transform.translation -= delta.extend(0.0);
    *drag = DragPan::Dragging(cursor);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Окно 1000×600 с камерой в (100, 100): полуразмер — 500×300 мировых
    /// метров на зуме 1.0.
    fn viewport(zoom: f32, screens: f32) -> Viewport {
        let mut window = Window::default();
        window.resolution.set(1000.0, 600.0);
        let camera = Transform::from_xyz(100.0, 100.0, 0.0).with_scale(Vec3::splat(zoom));
        Viewport::of(&window, &camera, screens)
    }

    #[test]
    fn the_frame_is_half_the_window_in_world_metres() {
        let view = viewport(1.0, 1.0);
        assert_eq!(view.centre, Vec2::new(100.0, 100.0));
        assert_eq!(view.half_extent, Vec2::new(500.0, 300.0));
        assert_eq!(view.zoom, 1.0);
    }

    /// Зум — метров на пиксель, поэтому на общем плане в кадр влезает больше
    /// мира, а не меньше.
    #[test]
    fn zooming_out_widens_the_frame() {
        assert_eq!(viewport(2.0, 1.0).half_extent, Vec2::new(1000.0, 600.0));
        assert_eq!(viewport(0.5, 1.0).half_extent, Vec2::new(250.0, 150.0));
    }

    #[test]
    fn the_margin_multiplies_the_frame() {
        assert_eq!(viewport(1.0, 1.2).half_extent, Vec2::new(600.0, 360.0));
    }

    /// Углы кадра — то, чем расталкивание спрашивает сетку соседей, и они
    /// обязаны согласоваться с [`Viewport::contains`], иначе выборка по
    /// прямоугольнику и отсев по точке разошлись бы на кромке.
    #[test]
    fn the_corners_agree_with_the_frame() {
        let view = viewport(1.0, 1.0);
        assert_eq!(view.min(), Vec2::new(-400.0, -200.0));
        assert_eq!(view.max(), Vec2::new(600.0, 400.0));
        assert!(view.contains(view.min()));
        assert!(view.contains(view.max()));
    }

    /// Кромка — своя: гейты писались через `<=`, и точка ровно на границе
    /// кадра обязана остаться внутри.
    #[test]
    fn the_edge_of_the_frame_counts_as_inside() {
        let view = viewport(1.0, 1.0);
        assert!(view.contains(Vec2::new(600.0, 400.0)));
        assert!(view.contains(Vec2::new(-400.0, -200.0)));
        assert!(!view.contains(Vec2::new(600.1, 100.0)));
        assert!(!view.contains(Vec2::new(100.0, 400.1)));
    }

    /// Кадр прямоугольный, а не круглый: угловой отступ по обеим осям меньше
    /// полуразмера — точка внутри, хотя до центра дальше, чем до кромки по x.
    #[test]
    fn the_frame_is_a_rectangle_not_a_circle() {
        let view = viewport(1.0, 1.0);
        assert!(view.contains(Vec2::new(590.0, 390.0)));
    }

    #[test]
    fn distance_is_measured_from_the_centre_of_the_frame() {
        let view = viewport(1.0, 1.0);
        assert_eq!(
            view.distance_from_centre_squared(Vec2::new(103.0, 104.0)),
            25.0
        );
    }
}
