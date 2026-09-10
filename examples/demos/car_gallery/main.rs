//! Витрина припаркованных машин: восемь форм улицы, на которых ряд ломался, и
//! девятой клеткой — стенд кузовов, — разом на одном экране, с ручкой
//! занятости, ступенью подробности и поворотом сцены.
//!
//! Ряд вдоль улицы устроен просто, а ломается сложно — и ломается не на
//! прямой, а на форме: на ломаной, чьи звенья короче отступа; на изломе, где
//! внутренняя сторона сжимается; на перекрёстке, который OSM размечает то
//! общей нодой, то торцами двух way. Тесты пиннят каждый такой случай числом,
//! но число не показывает, как это выглядит, — а выглядит ряд машин ровно тем,
//! ради чего он на карте есть.
//!
//! Поэтому клетки витрины — **не** красивые улицы, а перечень случаев: под
//! каждой написано, что именно на ней надо увидеть ([`scene`]). Девятая клетка
//! отвечает на другой вопрос — не «где стоит ряд», а «что в нём стоит»: пять
//! типов кузова на три ступени подробности ([`stand`]).
//!
//! **Геометрия здесь та же, что в игре, а не её копия.** Машины кладёт
//! [`cars_mesh`] — одна дверь наружу, за которой остались и шаг, и палитра, и
//! разрывы на перекрёстках, и правило излома. Витрина не знает о них ничего:
//! она даёт улицы и занятость, а обратно получает готовый меш. Улицы она
//! описывает **той же фикстурой** (`map::osm::fixture`), которой их описывают
//! тесты разбора, так что клетка и тест говорят об одном и том же объекте.
//!
//! Своей у витрины остаётся только подложка — сама проезжая часть под рядами.
//! Она нарисована игровой лентой (`MeshBuilder::push_ribbon`) в игровом
//! `ROAD_COLOR`: ступень яркости между кузовом и асфальтом — половина того, как
//! ряд читается, и на своём сером она была бы не та.
//!
//! Пример не трогает конфиг игры: ни `PrefsPlugin`, ни `MapPlugin`, ни
//! `CameraPlugin` — читать и писать `settings.toml` тут нечему. Колесо крутит
//! игровая `camera::zoom_to_cursor` под своим гейтом `not(hovering_ui)`.
//!
//! ```text
//! cargo run --example car_gallery
//! ```
//!
//! | клавиша | что делает |
//! |---|---|
//! | колесо | зум к точке под курсором |
//! | ЛКМ-перетаскивание, `WASD` | панорама |
//! | `G` | подложка: земля карты → нейтральный серый → тёмный |
//! | `L` | подписи вкл/выкл |
//!
//! `CAR_GALLERY_SHOT=путь.png` — поднять окно, снять витрину и выйти. Нужно
//! затем, что у примера нет BRP: без этого проверить внешний вид из сессии
//! нечем. Окно поднимается само — перекрытое чужим окном macOS снимает чёрным.

mod panel;
mod params;
mod scene;
#[path = "../gallery_shot.rs"]
mod shot;
mod stand;

use bevy::camera_controller::pan_camera::{PanCamera, PanCameraPlugin};
use bevy::feathers::constants::fonts;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::sprite_render::AlphaMode2d;
use bevy::window::PrimaryWindow;
use qwe::camera::{hovering_ui, zoom_to_cursor};
use qwe::map::cars::cars_mesh;
use qwe::map::osm::RoadLine;
use qwe::map::{
    CarStyle, GROUND_COLOR, MeshBuilder, ROAD_COLOR, RibbonCap, RibbonJoin, RoadSmoothing,
    smooth_path,
};
use qwe::ui::{PANEL_WIDTH_PX, UI_SCREEN_EDGE_PX_OFFSET};

use crate::panel::{
    spawn_panel, spawn_readout, sync_param_rows, sync_reset_button, update_readout,
};
use crate::params::Tuning;
use crate::shot::{ShotRequest, auto_shot, request_shot};

const WINDOW_WIDTH: f32 = 1500.0;
const WINDOW_HEIGHT: f32 = 900.0;

/// Мировой размер пикселя шрифта: `Text2d` меряет кегль в пикселях, а сцена —
/// в метрах. Клетки здесь в сотни метров, а не в десятки, как дома у витрины
/// кровель, — и подпись должна быть им под стать.
const TEXT_SCALE: f32 = 0.3;
const CAPTION_FONT: f32 = 24.0;
const HEADER_FONT: f32 = 30.0;
/// Поля вокруг сетки при стартовом зуме, доля её размера.
const VIEW_MARGIN: f32 = 1.06;

/// Слои витрины по z: асфальт под машинами, машины над ним, подписи поверх.
const Z_ROAD: f32 = 0.0;
const Z_CARS: f32 = 1.0;
const Z_CAPTION: f32 = 2.0;

/// Подложка витрины. Земля карты — то, на чём улицы лежат в игре; серый —
/// нейтральный фон; тёмный показывает, насколько светлы кузова.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Ground {
    #[default]
    Map,
    Grey,
    Dark,
}

impl Ground {
    fn color(self) -> Color {
        match self {
            Self::Map => GROUND_COLOR,
            Self::Grey => Color::srgb(0.5, 0.5, 0.5),
            Self::Dark => Color::srgb(0.16, 0.16, 0.17),
        }
    }

    /// Чернила подписей: на светлой подложке тёмные, на тёмной светлые.
    fn ink(self) -> Color {
        match self {
            Self::Map | Self::Grey => Color::srgb(0.14, 0.16, 0.20),
            Self::Dark => Color::srgb(0.86, 0.87, 0.90),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Map => Self::Grey,
            Self::Grey => Self::Dark,
            Self::Dark => Self::Map,
        }
    }
}

/// Что видно на витрине помимо улиц: подложка и подписи. Одним ресурсом, а не
/// двумя, потому что чернила подписи выбирает подложка.
#[derive(Resource)]
struct View {
    ground: Ground,
    captions: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            ground: Ground::default(),
            captions: true,
        }
    }
}

impl View {
    fn visibility(&self) -> Visibility {
        if self.captions {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    }
}

/// Слои витрины — один слитый меш на асфальт и один на машины, как в игре.
#[derive(Component)]
struct GalleryLayer;

/// Подписи клеток. Живут отдельно от мешей: от ручек они не зависят.
#[derive(Component)]
struct Caption;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "qwe car gallery — формы улицы, на которых ломался ряд".to_string(),
                        resolution: (WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    level: bevy::log::Level::WARN,
                    filter: "warn,qwe=warn".to_string(),
                    ..default()
                }),
        )
        .add_plugins(PanCameraPlugin)
        // киты панели игры — кнопки, ползунки, тема. Заодно и шрифт: во
        // встроенном `default_font` кириллицы нет
        .add_plugins(qwe::ui::PanelWidgetsPlugin)
        .init_resource::<View>()
        .init_resource::<Tuning>()
        .insert_resource(ClearColor(Ground::default().color()))
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_labels,
                spawn_panel,
                spawn_readout,
                request_shot("CAR_GALLERY_SHOT"),
            ),
        )
        .add_systems(
            Update,
            (
                cycle_ground.run_if(input_just_pressed(KeyCode::KeyG)),
                toggle_captions.run_if(input_just_pressed(KeyCode::KeyL)),
                // колесо над панелью витрине панели и принадлежит: тот же
                // гейт, что у игры, — ввод, адресованный UI, в мир не идёт
                zoom_to_cursor.run_if(not(hovering_ui)),
                update_readout,
                // сетка строится здесь же, а не в `Startup`: на первом кадре
                // ресурс считается только что добавленным, и условие пускает
                // ту же сборку, что потом идёт на каждую правку ручки
                (rebuild_gallery, sync_param_rows, sync_reset_button)
                    .run_if(resource_changed::<Tuning>),
                apply_ground.run_if(resource_changed::<View>),
                auto_shot.run_if(resource_exists::<ShotRequest>),
            )
                .chain(),
        )
        .run();
}

/// Улицы всех клеток в мировых координатах: локальная геометрия клетки,
/// повёрнутая ручкой и сдвинутая в свою ячейку сетки.
///
/// Одним срезом на всю витрину, а не по клетке: перекрёстки восстанавливаются
/// по общим нодам, и клетки, разнесённые на сотни метров, друг другу ничего не
/// добавляют — зато вызов остаётся ровно тем же, каким его делает игра.
fn gallery_roads(tuning: &Tuning) -> Vec<RoadLine> {
    let rotation = Rot2::degrees(tuning.rotation_deg);
    let mut roads = Vec::new();
    for (index, cell) in scene::cells().into_iter().enumerate() {
        let centre = scene::centre(index);
        for road in cell.roads {
            roads.push(RoadLine {
                points: road
                    .points
                    .iter()
                    .map(|point| centre + rotation * *point)
                    .collect(),
                ..road
            });
        }
    }
    roads
}

/// Прямоугольник всей витрины в мировых единицах — по нему ставится камера.
/// Считается по шагу сетки, а не по геометрии: клетки разного размера, и
/// кадрировать по самой длинной улице значило бы прижать соседние к краю.
fn grid_rect() -> Rect {
    // последняя ячейка сетки — стенд кузовов, он идёт следом за клетками-улицами
    let last = scene::cells().len();
    let half = scene::CELL_PITCH / 2.0;
    // верх сетки — первая клетка с местом под её заголовок, низ — последняя с
    // местом под её подпись; ряды идут вниз, поэтому y у них разного знака
    let top = scene::centre(0).y + half.y + scene::HEADER_RISE;
    let bottom = scene::centre(last).y - half.y - scene::CAPTION_DROP;
    Rect::from_corners(
        Vec2::new(scene::centre(0).x - half.x, bottom),
        Vec2::new(scene::centre(scene::COLUMNS - 1).x + half.x, top),
    )
}

/// Полоса окна, занятая панелью: отступ от края экрана, сама панель и такой же
/// зазор справа от неё.
fn panel_span() -> f32 {
    PANEL_WIDTH_PX + 2.0 * UI_SCREEN_EDGE_PX_OFFSET
}

fn spawn_camera(mut commands: Commands, window: Single<&Window, With<PrimaryWindow>>) {
    let rect = grid_rect();
    let viewport_width = window.width() - panel_span();
    let zoom = (rect.width() * VIEW_MARGIN / viewport_width)
        .max(rect.height() * VIEW_MARGIN / window.height());
    // свободная часть окна лежит правее центра экрана ровно на полширины
    // занятой панелью полосы — камера едет влево на столько же
    let centre = rect.center() - Vec2::new(panel_span() / 2.0 * zoom, 0.0);
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            near: -1000.0,
            far: 1000.0,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(centre.extend(0.0)).with_scale(Vec3::splat(zoom)),
        Msaa::Off,
        PanCamera {
            zoom_factor: zoom,
            min_zoom: zoom / 40.0,
            max_zoom: zoom * 4.0,
            // колесо ведёт `camera::zoom_to_cursor`: линейный зум самого
            // PanCamera на крупном плане неуправляем
            zoom_speed: 0.0,
            key_zoom_in: None,
            key_zoom_out: None,
            rotation_speed: 0.0,
            key_rotate_ccw: None,
            key_rotate_cw: None,
            pan_speed: 40.0,
            ..default()
        },
    ));
}

/// Заголовок над клеткой и то, что на ней надо увидеть, — под ней. От ручек не
/// зависят, поэтому спавнятся один раз.
fn spawn_labels(mut commands: Commands, assets: Res<AssetServer>, view: Res<View>) {
    let font: Handle<Font> = assets.load(fonts::REGULAR);
    for (index, cell) in scene::cells().iter().enumerate() {
        let centre = scene::centre(index);
        let label = |text: String, at: Vec2, size: f32| {
            (
                Caption,
                Text2d::new(text),
                label_font(&font, size),
                TextColor(view.ground.ink()),
                Anchor::CENTER,
                view.visibility(),
                Transform::from_translation(at.extend(Z_CAPTION))
                    .with_scale(Vec3::splat(TEXT_SCALE)),
            )
        };
        commands.spawn(label(
            format!("{}. {}", index + 1, cell.title),
            centre + Vec2::new(0.0, scene::HEADER_RISE),
            HEADER_FONT,
        ));
        commands.spawn(label(
            cell.note.to_string(),
            centre - Vec2::new(0.0, scene::CAPTION_DROP),
            CAPTION_FONT,
        ));
    }
    spawn_stand_labels(&mut commands, &font, &view);
}

/// Подписи стенда кузовов: заголовок, тип под каждым столбцом и ступень
/// подробности слева от каждой строки. Стенд увеличен трансформом, а подписи
/// — нет, поэтому их места считаются из тех же координат стенда, умноженных
/// на его масштаб.
fn spawn_stand_labels(commands: &mut Commands, font: &Handle<Font>, view: &View) {
    let index = scene::cells().len();
    let centre = scene::centre(index);
    let half = stand::half_size() * stand::SCALE;
    let label = |text: String, at: Vec2, size: f32, anchor: Anchor| {
        (
            Caption,
            Text2d::new(text),
            label_font(font, size),
            TextColor(view.ground.ink()),
            anchor,
            view.visibility(),
            Transform::from_translation(at.extend(Z_CAPTION)).with_scale(Vec3::splat(TEXT_SCALE)),
        )
    };
    commands.spawn(label(
        format!("{}. Кузова (×{:.0})", index + 1, stand::SCALE),
        centre + Vec2::new(0.0, scene::HEADER_RISE),
        HEADER_FONT,
        Anchor::CENTER,
    ));
    commands.spawn(label(
        "тип кузова выпадает ГПСЧ; здесь он заказан, чтобы увидеть все пять".to_string(),
        centre - Vec2::new(0.0, scene::CAPTION_DROP),
        CAPTION_FONT,
        Anchor::CENTER,
    ));
    let gap = CAPTION_FONT * TEXT_SCALE;
    // ступени — заголовками над столбцами, типы — словом слева от строки,
    // прижатым к стенду правым краем.
    //
    // Правый край подписи типа стоит на 126 м от центра ячейки (полустенд
    // 118.8 плюс отступ), а граница ячейки — на 150: под слово остаётся 24 м, и
    // самые длинные («Универсал», «Кроссовер») в них не помещаются — заходят в
    // поле соседней ячейки метров на десять. Так и оставлено: поле там пустое
    // (клетка-улица кончается в 100 м от своего центра, а её подписи — заголовок
    // и примечание — стоят по центру и не дотягиваются), а обе альтернативы
    // портят сам стенд: мельче он не нужен (`stand::SCALE` — лупа над машиной в
    // 4.5 м рядом с улицами в сотни), а подпись типа над каждой машиной левого
    // столбца легла бы на асфальт стенда и сломала сравнение трёх ступеней
    // подряд, ради которого стенд и заведён.
    for (column, detail) in stand::DETAILS.iter().enumerate() {
        let at = stand::at(column, 0) * stand::SCALE;
        commands.spawn(label(
            format!("{detail:?}"),
            centre + Vec2::new(at.x, half.y + gap),
            CAPTION_FONT,
            Anchor::CENTER,
        ));
    }
    for (row, (_, name)) in stand::SHAPES.iter().enumerate() {
        let at = stand::at(0, row) * stand::SCALE;
        commands.spawn(label(
            name.to_string(),
            centre + Vec2::new(-half.x - gap, at.y),
            CAPTION_FONT,
            Anchor::CENTER_RIGHT,
        ));
    }
}

/// Пересборка витрины под текущие ручки: асфальт своим мешем, машины —
/// игровым вызовом.
fn rebuild_gallery(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    tuning: Res<Tuning>,
    existing: Query<Entity, With<GalleryLayer>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let roads = gallery_roads(&tuning);

    let mut asphalt = MeshBuilder::default();
    let road_color = ROAD_COLOR.to_linear();
    // город кладёт ленту по сглаженной осевой; витрина обязана класть её так
    // же, иначе показывает не игровую геометрию, а свою
    let smoothing = RoadSmoothing::default();
    for road in &roads {
        asphalt.push_ribbon(
            &smooth_path(&road.points, road.width, smoothing),
            false,
            road.width,
            road_color,
            RibbonJoin::Round,
            RibbonCap::Round,
        );
    }

    let cars = cars_mesh(
        &roads,
        CarStyle {
            visible: true,
            occupancy: tuning.occupancy,
        },
        smoothing,
        tuning.car_detail(),
    );

    // асфальт непрозрачен, у машин тень полупрозрачна — как в игре, два
    // материала и два слоя по z
    let flat = materials.add(Color::WHITE);
    let blended = materials.add(ColorMaterial {
        alpha_mode: AlphaMode2d::Blend,
        ..default()
    });
    for (builder, z, material, name) in [
        (asphalt, Z_ROAD, flat, "gallery_roads"),
        (cars, Z_CARS, blended.clone(), "gallery_cars"),
    ] {
        if builder.is_empty() {
            continue;
        }
        commands.spawn((
            GalleryLayer,
            Mesh2d(meshes.add(builder.build())),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, 0.0, z),
            Name::new(name),
        ));
    }

    // стенд кузовов — та же геометрия, увеличенная трансформом: ручка
    // подробности его не касается, на нём все три ступени сразу
    let centre = scene::centre(scene::cells().len());
    commands.spawn((
        GalleryLayer,
        Mesh2d(meshes.add(stand::mesh().build())),
        MeshMaterial2d(blended),
        Transform::from_translation(centre.extend(Z_CARS)).with_scale(Vec3::splat(stand::SCALE)),
        Name::new("gallery_stand"),
    ));
}

/// Подпись шрифтом панелей игры: во встроенном шрифте bevy кириллицы нет.
fn label_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

fn cycle_ground(mut view: ResMut<View>) {
    view.ground = view.ground.next();
}

fn toggle_captions(mut view: ResMut<View>) {
    view.captions = !view.captions;
}

/// Подложка и подписи одной системой: чернила зависят от подложки, а видимость
/// — от тумблера, и обе живут на одних и тех же сущностях.
fn apply_ground(
    view: Res<View>,
    mut clear: ResMut<ClearColor>,
    mut captions: Query<(&mut Visibility, &mut TextColor), With<Caption>>,
) {
    clear.0 = view.ground.color();
    let visibility = view.visibility();
    for (mut value, mut color) in &mut captions {
        *value = visibility;
        color.0 = view.ground.ink();
    }
}
